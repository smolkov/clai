//! `run_command` (see `spec/tools.md`, "run_command").

use std::collections::VecDeque;
use std::io;
use std::os::fd::OwnedFd;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use serde::Deserialize;
use tokio::io::AsyncReadExt;
use tokio::net::unix::pipe;
use tokio::process::{Child, Command};
use tokio::time::{Instant, sleep, sleep_until, timeout};
use tokio_util::sync::CancellationToken;

use crate::permissions::command_proposals;

use super::output::{FULL_BYTES, ToolOutput, group};
use super::question::Details;
use super::{Call, Env, Failure, Op as AnyOp, Progress, cancelled, in_range};

/// Added to the environment so that nothing waits for input.
pub const ENV: [(&str, &str); 4] = [
    ("PAGER", "cat"),
    ("GIT_PAGER", "cat"),
    ("GIT_TERMINAL_PROMPT", "0"),
    ("TERM", "dumb"),
];
const DEFAULT_TIMEOUT_S: u64 = 120;
const MAX_TIMEOUT_S: u64 = 600;
/// From `SIGTERM` to `SIGKILL`.
const KILL_AFTER: Duration = Duration::from_secs(2);
/// How long output may still come after the shell exited, from processes
/// it started in the background; then they are stopped.
const AFTER_EXIT: Duration = Duration::from_millis(500);
/// Output kept from the end when the output is too long.
const TAIL_BYTES: usize = 64 << 10;
/// Least time between two progress messages.
const PROGRESS_EVERY: Duration = Duration::from_millis(100);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Args {
    command: String,
    timeout_s: Option<u64>,
}

#[derive(Debug)]
pub struct Op {
    pub command: String,
    cwd: PathBuf,
    timeout_s: u64,
}

pub(super) fn prepare(env: &Env, args: Result<Args, Failure>) -> Result<Call, Failure> {
    let args = args?;
    if args.command.trim().is_empty() {
        return Err("command is empty".to_owned().into());
    }
    let timeout_s = in_range(
        "timeout_s",
        args.timeout_s,
        1,
        MAX_TIMEOUT_S,
        DEFAULT_TIMEOUT_S,
    )?;
    let checked = env.checker().check_command(&args.command);
    let cwd = env.workspace.root().to_path_buf();
    Ok(Call {
        summary: args.command.clone(),
        denial: Some(checked.denial()),
        proposals: command_proposals(&checked),
        details: Details::RunCommand {
            command: args.command.clone(),
            cwd: env.workspace.tilde(&cwd),
            timeout_s,
            env: ENV
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            parts: checked.parts,
            redirects: checked.redirects,
        },
        decision: checked.decision,
        op: AnyOp::Run(Op {
            command: args.command,
            cwd,
            timeout_s,
        }),
    })
}

/// The start and the end of the output.
#[derive(Default)]
struct Capture {
    head: Vec<u8>,
    tail: VecDeque<u8>,
    total: usize,
}

impl Capture {
    fn push(&mut self, bytes: &[u8]) {
        self.total += bytes.len();
        let room = (FULL_BYTES - TAIL_BYTES).saturating_sub(self.head.len());
        let (to_head, rest) = bytes.split_at(room.min(bytes.len()));
        self.head.extend_from_slice(to_head);
        self.tail.extend(rest);
        while self.tail.len() > TAIL_BYTES {
            self.tail.pop_front();
        }
    }

    fn text(&self) -> String {
        let mut out = String::from_utf8_lossy(&self.head).into_owned();
        let kept = self.head.len() + self.tail.len();
        if self.total > kept {
            out.push_str(&format!("\n… {} bytes cut …\n", group(self.total - kept)));
        }
        let tail: Vec<u8> = self.tail.iter().copied().collect();
        out.push_str(&String::from_utf8_lossy(&tail));
        out
    }
}

enum End {
    Exited(ExitStatus),
    Timeout,
    Cancelled,
}

impl Op {
    pub async fn run(
        self,
        _env: &Env,
        cancel: CancellationToken,
        progress: Option<Progress>,
    ) -> ToolOutput {
        let mut progress = progress.map(Sender::new);
        let output = self.execute(cancel, progress.as_mut()).await;
        if let Some(sender) = &mut progress {
            sender.flush();
        }
        match output {
            Ok(output) => output,
            Err(e) => ToolOutput::error(format!("cannot run the command: {e}")),
        }
    }

    async fn execute(
        &self,
        cancel: CancellationToken,
        mut progress: Option<&mut Sender>,
    ) -> io::Result<ToolOutput> {
        let (reader, writer) = io::pipe()?;
        let mut command = Command::new("sh");
        command
            .arg("-c")
            .arg(&self.command)
            .current_dir(&self.cwd)
            .envs(ENV)
            .stdin(Stdio::null())
            .stdout(writer.try_clone()?)
            .stderr(writer)
            .process_group(0);
        let mut child = command.spawn()?;
        // The command holds our copies of the pipe's write end.
        drop(command);
        let group_id = child
            .id()
            .and_then(|id| i32::try_from(id).ok())
            .map(Pid::from_raw);
        let mut output = pipe::Receiver::from_owned_fd(OwnedFd::from(reader))?;
        let mut capture = Capture::default();
        let mut buf = vec![0u8; 16 << 10];
        let mut eof = false;
        let mut status: Option<ExitStatus> = None;
        let deadline = Instant::now() + Duration::from_secs(self.timeout_s);
        let mut after_exit = Instant::now();
        let end = loop {
            if let (Some(s), true) = (status, eof) {
                break End::Exited(s);
            }
            tokio::select! {
                biased;
                () = cancel.cancelled() => break End::Cancelled,
                () = sleep_until(deadline) => break End::Timeout,
                read = output.read(&mut buf), if !eof => match read {
                    Ok(0) | Err(_) => eof = true,
                    Ok(n) => {
                        let bytes = buf.get(..n).unwrap_or(&[]);
                        capture.push(bytes);
                        if let Some(sender) = progress.as_deref_mut() {
                            sender.push(bytes);
                        }
                    }
                },
                () = sleep_until(progress.as_deref().map_or(deadline, Sender::due)),
                    if progress.as_deref().is_some_and(Sender::waiting) =>
                {
                    if let Some(sender) = progress.as_deref_mut() {
                        sender.flush();
                    }
                }
                waited = child.wait(), if status.is_none() => {
                    status = Some(waited?);
                    after_exit = Instant::now() + AFTER_EXIT;
                }
                () = sleep_until(after_exit), if status.is_some() && !eof => {
                    // Background processes still hold the output open.
                    stop(group_id, &mut child).await;
                    drain(&mut output, &mut capture).await;
                    break End::Exited(status.unwrap_or_default());
                }
            }
        };
        let text = capture.text();
        let text = text.trim_end_matches('\n');
        let join = |last: &str| {
            if text.is_empty() {
                last.to_owned()
            } else {
                format!("{text}\n{last}")
            }
        };
        Ok(match end {
            End::Exited(status) => {
                // Stop what the command left running in its group.
                signal(group_id, Signal::SIGKILL);
                match (status.code(), status.signal()) {
                    (Some(0), _) => ToolOutput::ok(join("exit code 0")),
                    (Some(code), _) => ToolOutput::error(join(&format!("exit code {code}"))),
                    (None, Some(sig)) => {
                        ToolOutput::error(join(&format!("killed by signal {sig}")))
                    }
                    (None, None) => ToolOutput::error(join("ended without an exit code")),
                }
            }
            End::Timeout => {
                stop(group_id, &mut child).await;
                drain(&mut output, &mut capture).await;
                let text = capture.text();
                let text = text.trim_end_matches('\n');
                let last = format!("stopped after {} s (timeout)", self.timeout_s);
                ToolOutput::error(if text.is_empty() {
                    last
                } else {
                    format!("{text}\n{last}")
                })
            }
            End::Cancelled => {
                stop(group_id, &mut child).await;
                drain(&mut output, &mut capture).await;
                cancelled(capture.text().trim_end_matches('\n'))
            }
        })
    }
}

/// Sends output to a [`Progress`] channel, at most every 100 ms, in whole
/// UTF-8 characters.
struct Sender {
    channel: Progress,
    pending: Vec<u8>,
    last: Instant,
}

impl Sender {
    fn new(channel: Progress) -> Sender {
        Sender {
            channel,
            pending: Vec::new(),
            last: Instant::now(),
        }
    }

    fn push(&mut self, bytes: &[u8]) {
        self.pending.extend_from_slice(bytes);
        // Kept short: the full output is in the result.
        if self.pending.len() > FULL_BYTES {
            self.pending.clear();
        }
        if self.last.elapsed() >= PROGRESS_EVERY {
            self.flush();
        }
    }

    fn waiting(&self) -> bool {
        !self.pending.is_empty()
    }

    fn due(&self) -> Instant {
        self.last + PROGRESS_EVERY
    }

    /// Sends what is pending, except an unfinished character at the end.
    fn flush(&mut self) {
        let keep = match std::str::from_utf8(&self.pending) {
            Ok(_) => 0,
            Err(e) if e.error_len().is_none() => self.pending.len() - e.valid_up_to(),
            Err(_) => 0,
        };
        let rest = self.pending.split_off(self.pending.len() - keep);
        let text = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending = rest;
        self.last = Instant::now();
        if !text.is_empty() {
            // The receiver may be gone; the output is still in the result.
            let _ = self.channel.send(text);
        }
    }
}

/// `SIGTERM` to the group, `SIGKILL` 2 s later if the shell still runs.
async fn stop(group_id: Option<Pid>, child: &mut Child) {
    signal(group_id, Signal::SIGTERM);
    if timeout(KILL_AFTER, child.wait()).await.is_err() {
        signal(group_id, Signal::SIGKILL);
        let _ = timeout(KILL_AFTER, child.wait()).await;
    } else {
        // The shell is gone; stop what is left of the group.
        sleep(Duration::from_millis(50)).await;
        signal(group_id, Signal::SIGKILL);
    }
}

fn signal(group_id: Option<Pid>, signal: Signal) {
    if let Some(pid) = group_id {
        // ESRCH: the group is already gone.
        let _ = killpg(pid, signal);
    }
}

/// Reads what is left in the pipe, for a short time.
async fn drain(output: &mut pipe::Receiver, capture: &mut Capture) {
    let mut buf = vec![0u8; 16 << 10];
    let until = Instant::now() + Duration::from_millis(200);
    while let Ok(Ok(n)) = tokio::time::timeout_at(until, output.read(&mut buf)).await {
        if n == 0 {
            break;
        }
        capture.push(buf.get(..n).unwrap_or(&[]));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_keeps_start_and_end() {
        let mut c = Capture::default();
        c.push(&vec![b'a'; FULL_BYTES]);
        c.push(&vec![b'b'; 2 * TAIL_BYTES]);
        let text = c.text();
        assert!(text.starts_with('a') && text.ends_with('b'));
        assert!(text.contains("bytes cut"));
        assert!(text.len() <= FULL_BYTES + 100);
    }
}
