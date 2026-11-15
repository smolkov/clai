//! `codeagent server`, `codeagent server stop`, `codeagent server status` (see `spec/command.md`).

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::PathBuf;
use std::time::Duration;

use clap::{Args, Subcommand};
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;

use super::{Context, Execute, Runtime};
use crate::client::{Client, Connect, unexpected};
use crate::config::LoadOptions;
use crate::error::{Error, Result};
use crate::output::write_stdout;
use crate::protocol::{Payload, Request};
use crate::server::{self, Options};

/// How long `stop` waits for the server to end its sessions.
const STOP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Args)]
pub struct Server {
    #[command(subcommand)]
    pub action: Option<ServerAction>,
    /// Started by a client: leave the terminal's session, ignore `SIGHUP`.
    #[arg(long, hide = true)]
    pub detached: bool,
}

#[derive(Debug, Subcommand)]
pub enum ServerAction {
    /// Stop the running server and end all its sessions.
    Stop {
        /// Do not ask for confirmation.
        #[arg(long)]
        force: bool,
    },
    /// Print whether a server runs, its PID and session count.
    Status,
}

impl Execute for Server {
    fn name(&self) -> &'static str {
        match self.action {
            None => "server",
            Some(ServerAction::Stop { .. }) => "server stop",
            Some(ServerAction::Status) => "server status",
        }
    }

    fn run(&self, ctx: &Context) -> Result<()> {
        match self.action {
            None => foreground(ctx, self.detached),
            Some(ServerAction::Stop { force }) => stop(ctx, force),
            Some(ServerAction::Status) => status(ctx),
        }
    }
}

/// Runs the server until it is stopped, idle, or gets `SIGINT`/`SIGTERM`.
fn foreground(ctx: &Context, detached: bool) -> Result<()> {
    if detached {
        // Fails only if already a group leader; then it stays where it is.
        let _ = nix::unistd::setsid();
    }
    let sessions_dir = ctx.paths.sessions.clone().ok_or_else(|| {
        Error::Usage("cannot save sessions: neither XDG_STATE_HOME nor HOME is set".to_owned())
    })?;
    let options = Options::new(
        ctx.paths.runtime.clone(),
        sessions_dir,
        LoadOptions {
            config_file: ctx.config_file.clone(),
            workspace: PathBuf::new(),
            // `CODEAGENT_MODEL` and `CODEAGENT_THEME` come from each client, not from
            // the environment the server happened to start in.
            env: ctx.startup.for_server(),
        },
    );
    let stop = options.stop.clone();
    let runtime = crate::runtime::multi_thread()?;
    let result = runtime.block_on(async move {
        watch_signals(stop, detached)?;
        server::serve(options).await
    });
    runtime.shutdown_timeout(Duration::from_secs(1));
    result
}

fn watch_signals(stop: CancellationToken, detached: bool) -> Result<()> {
    let listen = |kind: SignalKind| signal(kind).map_err(Error::io("signal handler"));
    let mut int = listen(SignalKind::interrupt())?;
    let mut term = listen(SignalKind::terminate())?;
    let mut hup = listen(SignalKind::hangup())?;
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = int.recv() => {}
                _ = term.recv() => {}
                _ = hup.recv() => {
                    if detached {
                        continue;
                    }
                }
            }
            tracing::info!("signal; stopping the server");
            stop.cancel();
        }
    });
    Ok(())
}

fn status(ctx: &Context) -> Result<()> {
    let result = ctx.block_on(Runtime::Client, async {
        let mut client = Client::connect(&Connect::cli(ctx, false)).await?;
        match client.request(Request::Status).await? {
            Payload::Status(status) => Ok(status),
            other => Err(unexpected(&other)),
        }
    });
    match result {
        Ok(status) => write_stdout(&format!(
            "running, pid {}, {} {} ({} running, {} busy), version {}\n",
            status.pid,
            status.sessions,
            if status.sessions == 1 {
                "session"
            } else {
                "sessions"
            },
            status.running,
            status.busy,
            status.version
        )),
        Err(Error::NoServer) => {
            write_stdout("not running\n")?;
            Err(Error::Exit(1))
        }
        Err(e) => Err(e),
    }
}

fn stop(ctx: &Context, force: bool) -> Result<()> {
    ctx.block_on(Runtime::Client, async {
        let mut client = Client::connect(&Connect::cli(ctx, false)).await?;
        // Sessions stay saved; only work in progress is interrupted.
        let busy = match client.request(Request::Status).await? {
            Payload::Status(status) => status.busy,
            other => return Err(unexpected(&other)),
        };
        if busy > 0 && !force && io::stdin().is_terminal() && !confirm(busy)? {
            return Err(Error::Failed("not stopped".to_owned()));
        }
        client.request(Request::Stop).await?;
        // The server closes the connection when it has stopped its sessions.
        let closed = tokio::time::timeout(STOP_TIMEOUT, async {
            while let Ok(Some(_)) = client.next().await {}
        })
        .await;
        match closed {
            Ok(()) => Ok(()),
            Err(_) => Err(Error::Failed("the server did not stop in time".to_owned())),
        }
    })
}

fn confirm(busy: usize) -> Result<bool> {
    let mut stderr = io::stderr().lock();
    let (plural, verb) = if busy == 1 { ("", "is") } else { ("s", "are") };
    let _ = write!(
        stderr,
        "{busy} session{plural} {verb} working; stop the server and interrupt them? [y/N] "
    );
    let _ = stderr.flush();
    let mut line = String::new();
    io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(Error::io("standard input"))?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}
