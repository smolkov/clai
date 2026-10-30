//! `clai commit` (see `spec/command.md`).

use std::path::{Path, PathBuf};

use clap::{Args, ValueHint};
use clap_complete::ArgValueCompleter;

use crate::complete;
use crate::error::{Error, Result};

#[derive(Debug, Args)]
pub struct Commit {
    /// Describe only the staged changes.
    #[arg(long)]
    pub staged: bool,
    /// Model to use, as PROVIDER/MODEL.
    #[arg(long, value_name = "PROVIDER/MODEL", add = ArgValueCompleter::new(complete::models))]
    pub model: Option<String>,
    /// Extra instruction for the model.
    #[arg(long, value_name = "TEXT")]
    pub hint: Option<String>,
    /// Directory. Default: the current directory.
    #[arg(value_hint = ValueHint::DirPath)]
    pub dir: Option<PathBuf>,
}

impl Commit {
    pub fn run(&self, _config: Option<&Path>) -> Result<()> {
        Err(Error::Unsupported { what: "commit" })
    }
}
