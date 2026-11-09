//! `clai commit` (see `spec/command.md` and `spec/commit.md`): prints a
//! message for the staged changes, or with `-a` all changes of tracked
//! files; commits nothing.

use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Duration;

use clap::{Args, ValueHint};
use clap_complete::ArgValueCompleter;
use tokio_util::sync::CancellationToken;

use super::common::workspace;
use super::run::{INTERRUPTED, NO_SIGNAL, SIGINT, watch_signals};
use super::{Context, Execute};
use crate::commit::{self, Repo, Scope};
use crate::complete;
use crate::config;
use crate::error::{Error, Result};
use crate::output::write_stdout;
use crate::permissions::{Checker, SessionRules, Workspace};
use crate::provider;
use crate::text::unicode;

#[derive(Debug, Args)]
pub struct Commit {
    /// Describe all changes of tracked files, as `git commit -a` commits
    /// them, not only the staged ones.
    #[arg(short = 'a', long)]
    pub all: bool,
    /// Model to use, as PROVIDER/MODEL.
    #[arg(long, value_name = "PROVIDER/MODEL", add = ArgValueCompleter::new(complete::models))]
    pub model: Option<String>,
    /// Extra instruction for the model.
    #[arg(long, value_name = "TEXT")]
    pub hint: Option<String>,
    /// A directory in the repository. Default: the current directory.
    #[arg(value_hint = ValueHint::DirPath)]
    pub dir: Option<PathBuf>,
}

impl Commit {
    fn scope(&self) -> Scope {
        if self.all { Scope::All } else { Scope::Staged }
    }
}

impl Execute for Commit {
    fn name(&self) -> &'static str {
        "commit"
    }

    fn run(&self, ctx: &Context) -> Result<()> {
        let dir = workspace(self.dir.as_deref())?;
        let repo = Repo::open(&dir)?;
        let root = repo.root().to_path_buf();
        let options = ctx.load_options(root.clone());
        let config = config::load(&options)?;
        let model_ref = match &self.model {
            Some(text) => config.resolve_model(text).map_err(Error::Usage)?,
            None => config
                .commit
                .model
                .clone()
                .unwrap_or_else(|| config.default_model.clone()),
        };
        let Some((provider_config, model)) = config.model(&model_ref) else {
            return Err(Error::Usage(format!("unknown model `{model_ref}`")));
        };
        let workspace = Workspace::new(&root, &config.permissions.extra_dirs, options.env.home)
            .map_err(Error::io(root.display().to_string()))?;
        let session = SessionRules::default();
        let checker = Checker {
            permissions: &config.permissions,
            session: &session,
            workspace: &workspace,
        };
        let prepared = commit::prepare(&repo, self.scope(), &checker, self.hint.as_deref())?;
        let provider = provider::from_config(provider_config)
            .map_err(|e| Error::Failed(format!("provider {}: {e}", provider_config.name)))?;
        if io::stderr().is_terminal() {
            let dots = if unicode(config.tui.symbols, |name| std::env::var(name).ok()) {
                "…"
            } else {
                "..."
            };
            let files = if prepared.files == 1 { "file" } else { "files" };
            let kind = match self.scope() {
                Scope::Staged => "staged",
                Scope::All => "changed",
            };
            let _ = writeln!(
                io::stderr().lock(),
                "Describing {} {kind} {files} with {model_ref}{dots}",
                prepared.files
            );
        }
        tracing::info!(model = %model_ref, files = prepared.files, "commit");

        let runtime = crate::runtime::multi_thread()?;
        let trailer = config.commit.trailer.as_deref();
        let result = runtime.block_on(async {
            let cancel = CancellationToken::new();
            let stopped = Arc::new(AtomicU8::new(NO_SIGNAL));
            watch_signals(cancel.clone(), stopped.clone())?;
            let result =
                commit::describe(provider.as_ref(), model, &prepared, trailer, cancel).await;
            match stopped.load(Ordering::SeqCst) {
                NO_SIGNAL => result,
                SIGINT => Err(Error::Exit(INTERRUPTED)),
                _ => Err(Error::Exit(1)),
            }
        });
        runtime.shutdown_timeout(Duration::from_millis(100));
        write_stdout(&result?)
    }
}
