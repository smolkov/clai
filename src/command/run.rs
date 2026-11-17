//! `codeagent run` and its short form `codeagent TASK...` (see `spec/command.md`).

use std::io::{self, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use clap::{Args, ValueEnum, ValueHint};
use clap_complete::ArgValueCompleter;
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;

use super::common::{Input, task_text};
use super::{Context, Execute};
use crate::agent::{Agent, Asker, EndReason, EventSink, PolicyAsker, Setup, system_prompt, today};
use crate::complete;
use crate::config;
use crate::error::{Error, Result};
use crate::lsp::pool::Pool;
use crate::output::write_stdout;
use crate::permissions::{SessionRules, Workspace};
use crate::provider;
use crate::report::{self, Collector, Outcome};
use crate::scratch::Scratch;
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
    /// Turn on skill NAME for the task; repeatable. `codeagent skills` lists
    /// them.
    #[arg(long = "skill", value_name = "NAME")]
    pub skills: Vec<String>,
    /// Print a report instead of the last answer: Markdown, or with
    /// `--report=json` one JSON object.
    #[arg(
        long,
        value_name = "FORMAT",
        num_args = 0..=1,
        require_equals = true,
        default_missing_value = "text"
    )]
    pub report: Option<ReportFormat>,
    /// Keep the scratch directory after the run and print its path on
    /// standard error.
    #[arg(long)]
    pub keep_tmp: bool,
    /// Task; `-` reads it from standard input, as does no task when
    /// standard input is not a terminal.
    #[arg(value_name = "TASK")]
    pub task: Vec<String>,
}

/// The format of `--report`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum ReportFormat {
    /// Markdown.
    Text,
    Json,
}

/// The longest task from standard input.
const MAX_TASK: usize = 1024 * 1024;

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
        let task = self.task(ctx)?;
        let answering = answering(
            self.yes,
            self.no_ask,
            !task.from_stdin && ctx.startup.stdin_terminal,
        );
        let prepared = self.setup(ctx, answering)?;
        let report = self.report.map(|format| Reporting {
            format,
            workspace: prepared.root.clone(),
            place: prepared.place.clone(),
            model: prepared.setup.model_name.clone(),
        });
        let asker = asker(answering, prepared.place, prepared.unicode);
        let scratch = prepared.setup.env.workspace.scratch().cloned();
        let agent = Agent::new(prepared.setup);
        if !prepared.skills.is_empty() {
            agent.skills().set(prepared.skills);
        }
        let result = one_shot(agent, task.text, asker, prepared.unicode, report);
        if let Some(scratch) = scratch {
            self.end_scratch(&scratch);
        }
        result
    }
}

impl Run {
    /// Removes the scratch directory, or with `--keep-tmp` says where it is.
    fn end_scratch(&self, scratch: &Scratch) {
        if self.keep_tmp {
            if scratch.exists() {
                crate::output::note(&format!(
                    "scratch directory kept: {}",
                    scratch.path().display()
                ));
            }
        } else if let Err(e) = scratch.remove() {
            crate::output::note(&format!("cannot remove the scratch directory: {e}"));
        }
    }

    /// The task: the words, or standard input for `-` or for no words when
    /// it is not a terminal.
    fn task(&self, ctx: &Context) -> Result<Input> {
        if self.task.is_empty() && ctx.startup.stdin_terminal {
            return Err(Error::Usage(
                "no task: give it as words or on standard input".to_owned(),
            ));
        }
        let dash = ["-".to_owned()];
        let words = if self.task.is_empty() {
            &dash[..]
        } else {
            &self.task[..]
        };
        task_text(words, "task", MAX_TASK)
    }
}

/// What `--report` needs besides the events.
struct Reporting {
    format: ReportFormat,
    /// Absolute, links resolved.
    workspace: PathBuf,
    /// As shown: `~/ws/codeagent`.
    place: String,
    model: String,
}

/// What the agent of a run is made from, with what the terminal output needs.
struct Prepared {
    setup: Setup,
    /// The workspace, absolute.
    root: PathBuf,
    /// The skills of `--skill`, in the order given.
    skills: Vec<crate::skills::Active>,
    /// The workspace as shown: `~/ws/codeagent`.
    place: String,
    unicode: bool,
}

impl Run {
    /// The workspace, the configuration, the model and its provider.
    /// `answering`: who answers questions.
    fn setup(&self, ctx: &Context, answering: Answering) -> Result<Prepared> {
        let can_ask = answering == Answering::Terminal;
        let workspace = ctx.workspace(self.dir.as_deref())?;
        let options = ctx.load_options(workspace.clone());
        let config = config::load(&options)?;
        for warning in &config.warnings {
            crate::output::note(warning);
        }
        let model_ref = match &self.model {
            Some(text) => config.resolve_model(text).map_err(Error::Usage)?,
            None => config.default_model.clone(),
        };
        let Some((provider_config, model)) = config.model(&model_ref) else {
            return Err(Error::Usage(format!("unknown model `{model_ref}`")));
        };
        let provider = provider::from_config(provider_config, &ctx.startup)
            .map_err(|e| Error::Failed(format!("provider {}: {e}", provider_config.name)))?;
        let mut skills = Vec::new();
        if !self.skills.is_empty() {
            let root = workspace
                .canonicalize()
                .unwrap_or_else(|_| workspace.clone());
            let found = crate::skills::find_for(&config, &root, &options.env);
            for name in &self.skills {
                let skill = found
                    .activate(name)
                    .ok_or_else(|| Error::Usage(found.unknown(name)))?;
                if !skills.contains(&skill) {
                    skills.push(skill);
                }
            }
        }
        let children = crate::child::Children::from_config(
            &config,
            options.env.home.as_deref(),
            ctx.paths.state.as_deref(),
            &ctx.paths.runtime,
        );
        let mut workspace =
            Workspace::new(&workspace, &config.permissions.extra_dirs, options.env.home)
                .map_err(Error::io(workspace.display().to_string()))?;
        if config.scratch {
            let key = crate::scratch::run_key(std::process::id());
            workspace = workspace.with_scratch(Scratch::new(&ctx.paths.runtime, &key));
        }
        // No provider keys for commands; maybe the sandbox (RFC 0035).
        workspace = workspace.with_children(children);
        let place = workspace.tilde(workspace.root());
        let root = workspace.root().to_path_buf();
        let unicode = unicode(config.tui.symbols, ctx.startup.getenv());
        tracing::info!(model = %model_ref, "run");
        Ok(Prepared {
            setup: Setup {
                provider,
                model: model.clone(),
                model_name: model_ref.to_string(),
                limits: config.limits,
                history: config.history,
                system: system_prompt(&workspace, &today()),
                can_ask,
                secrets: crate::secrets::Redactor::from_config(
                    &config.secrets,
                    config.providers.values(),
                    ctx.startup.vars(),
                )
                .map(Arc::new),
                env: Env {
                    permissions: Arc::new(config.permissions.clone()),
                    session: Arc::new(SessionRules::default()),
                    workspace: Arc::new(workspace),
                    lsp: Some(Arc::new(LspEnv::new(
                        Pool::new(),
                        &config.lsp,
                        ctx.startup.var_os("PATH"),
                        "run".into(),
                        can_ask,
                        answering == Answering::Allow,
                    ))),
                },
            },
            root,
            place,
            unicode,
            skills,
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
fn one_shot(
    agent: Agent,
    task: String,
    asker: Box<dyn Asker>,
    unicode: bool,
    report: Option<Reporting>,
) -> Result<()> {
    let runtime = crate::runtime::multi_thread()?;
    let result = runtime.block_on(turn(agent, task, asker, unicode, report));
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

/// Runs the turn: events to standard error, the answer or the report to
/// standard output.
async fn turn(
    mut agent: Agent,
    task: String,
    asker: Box<dyn Asker>,
    unicode: bool,
    report: Option<Reporting>,
) -> Result<()> {
    let cancel = CancellationToken::new();
    let stopped = Arc::new(AtomicU8::new(NO_SIGNAL));
    watch_signals(cancel.clone(), stopped.clone())?;

    let renderer = Arc::new(Mutex::new(Renderer::new(unicode)));
    let collector = Arc::new(Mutex::new(Collector::default()));
    let sink = {
        let renderer = renderer.clone();
        let collector = collector.clone();
        EventSink::call(move |event| {
            lock(&collector).event(&event);
            let text = lock(&renderer).event(&event);
            if !text.is_empty() {
                let mut stderr = io::stderr().lock();
                let _ = stderr.write_all(text.as_bytes());
                let _ = stderr.flush();
            }
        })
    };
    let started = Instant::now();
    let message = match &report {
        Some(_) => {
            agent.offer_report();
            report::work_message(&task)
        }
        None => task,
    };
    let end = agent
        .run_turn(message, &sink, asker.as_ref(), cancel.clone())
        .await;
    let answer = lock(&renderer).answer().unwrap_or_default().to_owned();
    if report.is_some()
        && end.reason == EndReason::Done
        && agent.report().is_none()
        && !cancel.is_cancelled()
    {
        agent
            .run_turn(
                report::ASK_AGAIN.to_owned(),
                &sink,
                asker.as_ref(),
                cancel.clone(),
            )
            .await;
    }
    // The language servers of the run end with it.
    if let Some(lsp) = &agent.env().lsp {
        lsp.pool.stop_all().await;
    }

    match stopped.load(Ordering::SeqCst) {
        SIGINT => return Err(Error::Exit(INTERRUPTED)),
        OTHER_SIGNAL => return Err(Error::Exit(1)),
        _ => {}
    }
    if let Some(reporting) = report {
        let Some(outcome) = Outcome::of(end.reason) else {
            return Err(Error::Exit(1));
        };
        let collector = lock(&collector);
        let reported = agent.report();
        let scratch_path = agent.env().workspace.scratch().map(Scratch::path);
        let fallback = match outcome {
            Outcome::Done => answer,
            Outcome::Limit => String::new(),
            Outcome::Failed => collector.error().unwrap_or("the run failed").to_owned(),
        };
        if reported.is_none() && outcome != Outcome::Failed {
            crate::output::note("the model gave no report; its status is unknown");
        }
        let run = report::Run {
            workspace: &reporting.workspace,
            model: &reporting.model,
            scratch: scratch_path.as_deref(),
            outcome,
            duration: started.elapsed().as_secs(),
        };
        let report = collector.report(run, reported, &fallback);
        let text = match reporting.format {
            ReportFormat::Text => report::render_text(&report, &reporting.place, unicode),
            ReportFormat::Json => report::render_json(&report),
        };
        write_stdout(&text)?;
        return match outcome {
            Outcome::Done => Ok(()),
            Outcome::Limit | Outcome::Failed => Err(Error::Exit(1)),
        };
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
/// `codeagent commit`). Later signals are
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
