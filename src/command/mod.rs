//! One module per command of `spec/command.md`: its arguments and its `run`.

pub mod attach;
pub mod chat;
pub mod commit;
pub mod common;
pub mod completion;
pub mod export;
pub mod import;
pub mod kill;
pub mod ls;
pub mod models;
pub mod new;
pub mod run;
pub mod send;
pub mod server;
pub mod skills;

use std::future::Future;
use std::path::{Path, PathBuf};

use crate::config::{self, Config, LoadOptions};
use crate::error::{Error, Result};
use crate::paths::Paths;
use crate::startup::Startup;

/// A command of `spec/command.md`. `Command::execute` in `cli.rs` is the one
/// dispatch.
pub trait Execute {
    /// As used in log lines and messages: `kill`, `server stop`.
    fn name(&self) -> &'static str;
    fn run(&self, ctx: &Context) -> Result<()>;
}

/// Which async runtime a command needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runtime {
    /// One connection, little work: the client commands.
    Client,
    /// Sessions, tools and signals at once: the server and `codeagent run`.
    Work,
}

/// What every command gets from `main` (RFC 0026): the environment read
/// once, the paths made from it, and `--config`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Context {
    /// The path given by `--config`, if any.
    pub config_file: Option<PathBuf>,
    pub startup: Startup,
    pub paths: Paths,
}

impl Context {
    pub fn new(startup: Startup, config_file: Option<PathBuf>) -> Context {
        let paths = Paths::new(&startup, config_file.as_deref());
        Context {
            config_file,
            startup,
            paths,
        }
    }

    pub fn config(&self) -> Option<&Path> {
        self.config_file.as_deref()
    }

    /// The workspace: `dir` or the current directory, which must be a
    /// directory, absolute and with links resolved, as the server keeps a
    /// session's.
    pub fn workspace(&self, dir: Option<&Path>) -> Result<PathBuf> {
        let dir =
            match dir {
                Some(dir) => match &self.startup.current_dir {
                    Some(cwd) if dir.is_relative() => cwd.join(dir),
                    _ => dir.to_path_buf(),
                },
                None => self.startup.current_dir.clone().ok_or_else(|| {
                    Error::Failed("the current directory cannot be read".to_owned())
                })?,
            };
        if !dir.is_dir() {
            return Err(Error::Usage(format!("{}: not a directory", dir.display())));
        }
        dir.canonicalize()
            .map_err(Error::io(dir.display().to_string()))
    }

    /// Load options for `workspace`, with the environment of the process.
    pub fn load_options(&self, workspace: PathBuf) -> LoadOptions {
        LoadOptions {
            config_file: self.config_file.clone(),
            workspace,
            env: self.startup.clone(),
        }
    }

    /// The configuration for `workspace`, with its project file.
    pub fn load(&self, workspace: PathBuf) -> Result<Config> {
        config::load(&self.load_options(workspace))
    }

    /// The configuration of the user file alone: for a command about a
    /// session elsewhere, and the check before a server starts. The runtime
    /// directory has no project file.
    pub fn user_config(&self) -> Result<Config> {
        self.load(self.paths.runtime.clone())
    }

    /// Runs `f` to its end on a runtime of the kind given.
    pub fn block_on<T>(&self, kind: Runtime, f: impl Future<Output = Result<T>>) -> Result<T> {
        let runtime = match kind {
            Runtime::Client => crate::runtime::current_thread()?,
            Runtime::Work => crate::runtime::multi_thread()?,
        };
        runtime.block_on(f)
    }
}
