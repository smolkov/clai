//! `clai run` and its short form `clai TASK...` (see `spec/command.md`).

use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use clap::{Args, ValueHint};
use clap_complete::ArgValueCompleter;
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;

use super::common::{input_text, workspace};
use super::{Context, Execute};
use crate::agent::{Agent, Asker, EndReason, EventSink, PolicyAsker, Setup, system_prompt, today};
use crate::complete;
use crate::config;
use crate::error::{Error, Result};
use crate::lsp::pool::Pool;
use crate::output::write_stdout;
use crate::permissions::{SessionRules, Workspace};
use crate::provider;
use crate::sync::lock;
use crate::text::{LineAsker, Renderer, unicode};
use crate::tools::Env;
use crate::tools::lsp::LspEnv;

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
pub(super) const INTERRUPTED: u8 = 130;

/// The signal that stopped the run: none yet, `SIGINT`, other.
pub(super) const NO_SIGNAL: u8 = 0;
pub(super) const SIGINT: u8 = 1;
pub(super) const OTHER_SIGNAL: u8 = 2;

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

impl Execute for Run {
    fn name(&self) -> &'static str {
        "run"
    }

    fn run(&self, ctx: &Context) -> Result<()> {
        let task = input_text(&self.task, "task")?;
        let answering = answering(
            self.yes,
            self.no_ask,
            !task.from_stdin && io::stdin().is_terminal(),
        );
        let prepared = self.setup(ctx, answering)?;
        let asker = asker(answering, prepared.place, prepared.unicode);
        one_shot(
            Agent::new(prepared.setup),
            task.text,
            asker,
            prepared.unicode,
        )
    }
}

/// What the agent of a run is made from, with what the terminal output needs.
struct Prepared {
    setup: Setup,
    /// The workspace as shown: `~/ws/clai`.
    place: String,
    unicode: bool,
}

impl Run {
    /// The workspace, the configuration, the model and its provider.
    /// `answering`: who answers questions.
    fn setup(&self, ctx: &Context, answering: Answering) -> Result<Prepared> {
        let can_ask = answering == Answering::Terminal;
        let workspace = workspace(self.dir.as_deref())?;
        let options = ctx.load_options(workspace.clone());
        let config = config::load(&options)?;
        let model_ref = match &self.model {
            Some(text) => config.resolve_model(text).map_err(Error::Usage)?,
            None => config.default_model.clone(),
        };
        let Some((provider_config, model)) = config.model(&model_ref) else {
            return Err(Error::Usage(format!("unknown model `{model_ref}`")));
        };
        let provider = provider::from_config(provider_config)
            .map_err(|e| Error::Failed(format!("provider {}: {e}", provider_config.name)))?;
        let workspace =
            Workspace::new(&workspace, &config.permissions.extra_dirs, options.env.home)
                .map_err(Error::io(workspace.display().to_string()))?;
        let place = workspace.tilde(workspace.root());
        let unicode = unicode(config.tui.symbols, |name| std::env::var(name).ok());
        tracing::info!(model = %model_ref, "run");
        Ok(Prepared {
            setup: Setup {
                provider,
                model: model.clone(),
                model_name: model_ref.to_string(),
                limits: config.limits,
                system: system_prompt(&workspace, &today()),
                can_ask,
                env: Env {
                    permissions: Arc::new(config.permissions.clone()),
                    session: Arc::new(SessionRules::default()),
                    workspace: Arc::new(workspace),
                    lsp: Some(Arc::new(LspEnv::new(
                        Pool::new(),
                        &config.lsp,
                        std::env::var_os("PATH").as_deref(),
                        can_ask,
                        answering == Answering::Allow,
                    ))),
                },
            },
            place,
            unicode,
        })
    }
}

/// The asker for who answers: a policy, or the terminal.
fn asker(answering: Answering, place: String, unicode: bool) -> Box<dyn Asker> {
    match answering {
        Answering::Allow => Box::new(PolicyAsker::Allow),
        Answering::Deny => Box::new(PolicyAsker::Deny),
        Answering::Terminal => Box::new(LineAsker::new(
            Box::new(io::BufReader::new(io::stdin())),
            Box::new(io::stderr()),
            place,
            unicode,
        )),
    }
}

/// Runs the one turn here, not in a server, on a runtime of its own.
fn one_shot(agent: Agent, task: String, asker: Box<dyn Asker>, unicode: bool) -> Result<()> {
    let runtime = crate::runtime::multi_thread()?;
    let result = runtime.block_on(turn(agent, task, asker, unicode));
    // A question may still wait for a line on standard input.
    runtime.shutdown_timeout(Duration::from_millis(100));
    result
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
            let text = lock(&renderer).event(&event);
            if !text.is_empty() {
                let mut stderr = io::stderr().lock();
                let _ = stderr.write_all(text.as_bytes());
                let _ = stderr.flush();
            }
        })
    };
    let end = agent.run_turn(task, &sink, asker.as_ref(), cancel).await;
    // The language servers of the run end with it.
    if let Some(lsp) = &agent.env().lsp {
        lsp.pool.stop_all().await;
    }

    match stopped.load(Ordering::SeqCst) {
        SIGINT => return Err(Error::Exit(INTERRUPTED)),
        OTHER_SIGNAL => return Err(Error::Exit(1)),
        _ => {}
    }
    match end.reason {
        EndReason::Done => {
            let answer = lock(&renderer).answer().unwrap_or_default().to_owned();
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

/// `SIGINT`, `SIGTERM` and `SIGHUP` cancel the turn (or the request of
/// `clai commit`). Later signals are
/// ignored while it stops, so no command is left running.
pub(super) fn watch_signals(cancel: CancellationToken, stopped: Arc<AtomicU8>) -> Result<()> {
    let listen = |kind: SignalKind| signal(kind).map_err(Error::io("signal handler"));
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
