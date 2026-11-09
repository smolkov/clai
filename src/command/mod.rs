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

use std::path::{Path, PathBuf};

use crate::config::{self, Config, Environment, LoadOptions};
use crate::error::Result;

/// A command of `spec/command.md`. `Command::execute` in `cli.rs` is the one
/// dispatch.
pub trait Execute {
    /// As used in log lines and messages: `kill`, `server stop`.
    fn name(&self) -> &'static str;
    fn run(&self, ctx: &Context) -> Result<()>;
}

/// What every command gets from `main`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Context {
    /// The path given by `--config`, if any.
    pub config_file: Option<PathBuf>,
}

impl Context {
    pub fn config(&self) -> Option<&Path> {
        self.config_file.as_deref()
    }

    /// Load options for `workspace`, with the process environment.
    pub fn load_options(&self, workspace: PathBuf) -> LoadOptions {
        LoadOptions {
            config_file: self.config_file.clone(),
            workspace,
            env: Environment::from_process(),
        }
    }

    /// The configuration for `workspace`.
    pub fn load(&self, workspace: PathBuf) -> Result<Config> {
        config::load(&self.load_options(workspace))
    }
}
