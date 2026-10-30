//! `clai new`, and `clai` without a command (see `spec/command.md`).

use std::path::{Path, PathBuf};

use clap::{Args, ValueHint};
use clap_complete::ArgValueCompleter;

use crate::complete;
use crate::error::{Error, Result};

#[derive(Debug, Default, Args)]
pub struct New {
    /// Do not attach; print the session name.
    #[arg(short, long)]
    pub detach: bool,
    /// Session name.
    #[arg(long)]
    pub name: Option<String>,
    /// Model of the session, as PROVIDER/MODEL.
    #[arg(long, value_name = "PROVIDER/MODEL", add = ArgValueCompleter::new(complete::models))]
    pub model: Option<String>,
    /// Workspace. Default: the current directory.
    #[arg(value_hint = ValueHint::DirPath)]
    pub dir: Option<PathBuf>,
}

impl New {
    pub fn run(&self, _config: Option<&Path>) -> Result<()> {
        Err(Error::Unsupported { what: "new" })
    }
}
