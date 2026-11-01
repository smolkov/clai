//! `clai run` and its short form `clai TASK...` (see `spec/command.md`).

use std::io::{self, IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use clap::{Args, ValueHint};
use clap_complete::ArgValueCompleter;
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;

use crate::agent::{Agent, Asker, EndReason, EventSink, PolicyAsker, Setup, system_prompt, today};
use crate::complete;
use crate::config::{self, Environment, LoadOptions};
use crate::error::{Error, Result};
use crate::output::write_stdout;
use crate::permissions::{SessionRules, Workspace};
use crate::provider;
use crate::text::{LineAsker, Renderer, unicode};
use crate::tools::Env;

#[derive(Debug, Args)]
pub struct Run {
    /// Model to use, as PROVIDER/MODEL.
    #[arg(long, value_name = "PROVIDER/MODEL", add = ArgValueCompleter::new(complete::models))]
    pub model: Option<String>,
    /// Allow tool calls that would ask; deny rules still apply.
    #[arg(long)]
    pub yes: bool,
    /// Never ask; deny tool calls that would ask.
    #[arg(long, conflicts_with = "yes")]
    pub no_ask: bool,
    /// Workspace. Default: the current directory.
    #[arg(short = 'C', value_name = "DIR", value_hint = ValueHint::DirPath)]
    pub dir: Option<PathBuf>,
    /// Task; `-` reads it from standard input.
    #[arg(value_name = "TASK", required = true)]
    pub task: Vec<String>,
}

/// Exit code after `Ctrl-C`.
const INTERRUPTED: u8 = 130;

/// The signal that stopped the run: none yet, `SIGINT`, other.
const NO_SIGNAL: u8 = 0;
const SIGINT: u8 = 1;
const OTHER_SIGNAL: u8 = 2;

/// Who answers questions in this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Answering {
    /// `--yes`.
    Allow,
    /// `--no-ask`, or no terminal.
    Deny,
    /// Standard input is a terminal.
    Terminal,
}

impl Run {
    pub fn run(&self, config_file: Option<&Path>) -> Result<()> {
        let from_stdin = self.task == ["-"];
        let task = if from_stdin {
            let mut text = String::new();
            io::stdin()
                .read_to_string(&mut text)
                .map_err(|source| Error::Io {
                    context: "standard input".to_owned(),
                    source,
                })?;
            text
        } else {
            self.task.join(" ")
        };
        let task = task.trim().to_owned();
        if task.is_empty() {
            return Err(Error::Usage("the task is empty".to_owned()));
        }
        let answering = answering(
            self.yes,
            self.no_ask,
            !from_stdin && io::stdin().is_terminal(),
        );

        let workspace = match &self.dir {
            Some(dir) => dir.clone(),
            None => std::env::current_dir().map_err(|source| Error::Io {
                context: "current directory".to_owned(),
                source,
            })?,
        };
        if !workspace.is_dir() {
            return Err(Error::Usage(format!(
                "{}: not a directory",
                workspace.display()
            )));
        }
        let env = Environment::from_process();
        let config = config::load(&LoadOptions {
            config_file: config_file.map(Path::to_path_buf),
            workspace: workspace.clone(),
            env: env.clone(),
        })?;
        let model_ref = match &self.model {
            Some(text) => config.resolve_model(text).map_err(Error::Usage)?,
            None => config.default_model.clone(),
        };
        let Some((provider_config, model)) = config.model(&model_ref) else {
            return Err(Error::Usage(format!("unknown model `{model_ref}`")));
        };
        let provider = provider::from_config(provider_config)
            .map_err(|e| Error::Failed(format!("provider {}: {e}", provider_config.name)))?;
        let workspace = Workspace::new(&workspace, &config.permissions.extra_dirs, env.home)
            .map_err(|source| Error::Io {
                context: workspace.display().to_string(),
                source,
            })?;
        let place = workspace.tilde(workspace.root());
        let unicode = unicode(config.tui.symbols, |name| std::env::var(name).ok());
        let setup = Setup {
            provider,
            model: model.clone(),
            limits: config.limits,
            system: system_prompt(&workspace, &today()),
            env: Env {
                permissions: Arc::new(config.permissions.clone()),
                session: Arc::new(SessionRules::default()),
                workspace: Arc::new(workspace),
            },
        };
        tracing::info!(model = %model_ref, "run");

        let asker: Box<dyn Asker> = match answering {
            Answering::Allow => Box::new(PolicyAsker::Allow),
            Answering::Deny => Box::new(PolicyAsker::Deny),
            Answering::Terminal => Box::new(LineAsker::new(
                Box::new(io::BufReader::new(io::stdin())),
                Box::new(io::stderr()),
                place,
                unicode,
            )),
        };
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|source| Error::Io {
                context: "async runtime".to_owned(),
                source,
            })?;
        let result = runtime.block_on(turn(Agent::new(setup), task, asker, unicode));
        // A question may still wait for a line on standard input.
        runtime.shutdown_timeout(Duration::from_millis(100));
        result
    }
}

/// Who answers, from the options and whether there is a terminal.
pub fn answering(yes: bool, no_ask: bool, terminal: bool) -> Answering {
    if yes {
        Answering::Allow
    } else if no_ask || !terminal {
        Answering::Deny
    } else {
        Answering::Terminal
    }
}

/// Runs the turn: events to standard error, the answer to standard output.
async fn turn(mut agent: Agent, task: String, asker: Box<dyn Asker>, unicode: bool) -> Result<()> {
    let cancel = CancellationToken::new();
    let stopped = Arc::new(AtomicU8::new(NO_SIGNAL));
    watch_signals(cancel.clone(), stopped.clone())?;

    let renderer = Arc::new(Mutex::new(Renderer::new(unicode)));
    let sink = {
        let renderer = renderer.clone();
        EventSink::call(move |event| {
            let text = renderer
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .event(&event);
            if !text.is_empty() {
                let mut stderr = io::stderr().lock();
                let _ = stderr.write_all(text.as_bytes());
                let _ = stderr.flush();
            }
        })
    };
    let end = agent.run_turn(task, &sink, asker.as_ref(), cancel).await;

    match stopped.load(Ordering::SeqCst) {
        SIGINT => return Err(Error::Exit(INTERRUPTED)),
        OTHER_SIGNAL => return Err(Error::Exit(1)),
        _ => {}
    }
    match end.reason {
        EndReason::Done => {
            let answer = renderer
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .answer()
                .unwrap_or_default()
                .to_owned();
            if answer.is_empty() {
                Ok(())
            } else if answer.ends_with('\n') {
                write_stdout(&answer)
            } else {
                write_stdout(&format!("{answer}\n"))
            }
        }
        // Already said on standard error.
        EndReason::Cancelled | EndReason::Limit | EndReason::Failed => Err(Error::Exit(1)),
    }
}

/// `SIGINT`, `SIGTERM` and `SIGHUP` cancel the turn. Later signals are
/// ignored while it stops, so no command is left running.
fn watch_signals(cancel: CancellationToken, stopped: Arc<AtomicU8>) -> Result<()> {
    let listen = |kind: SignalKind| {
        signal(kind).map_err(|source| Error::Io {
            context: "signal handler".to_owned(),
            source,
        })
    };
    let mut int = listen(SignalKind::interrupt())?;
    let mut term = listen(SignalKind::terminate())?;
    let mut hup = listen(SignalKind::hangup())?;
    tokio::spawn(async move {
        loop {
            let which = tokio::select! {
                _ = int.recv() => SIGINT,
                _ = term.recv() => OTHER_SIGNAL,
                _ = hup.recv() => OTHER_SIGNAL,
            };
            if stopped
                .compare_exchange(NO_SIGNAL, which, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                tracing::info!("signal; cancelling the turn");
                cancel.cancel();
            }
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn who_answers() {
        assert_eq!(answering(true, false, true), Answering::Allow);
        assert_eq!(answering(true, false, false), Answering::Allow);
        assert_eq!(answering(false, true, true), Answering::Deny);
        assert_eq!(answering(false, false, false), Answering::Deny);
        assert_eq!(answering(false, false, true), Answering::Terminal);
    }
}
