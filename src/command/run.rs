//! `clai run` and its short form `clai TASK...` (see `spec/command.md`).

use std::path::{Path, PathBuf};

use clap::{Args, ValueHint};
use clap_complete::ArgValueCompleter;

use crate::complete;
use crate::error::{Error, Result};

#[derive(Debug, Args)]
pub struct Run {
    /// Model to use, as PROVIDER/MODEL.
    #[arg(long, value_name = "PROVIDER/MODEL", add = ArgValueCompleter::new(complete::models))]
    pub model: Option<String>,
    /// Allow tool calls that would ask; deny rules still apply.
    #[arg(long)]
    pub yes: bool,
    /// Workspace. Default: the current directory.
    #[arg(short = 'C', value_name = "DIR", value_hint = ValueHint::DirPath)]
    pub dir: Option<PathBuf>,
    /// Task; `-` reads it from standard input.
    #[arg(value_name = "TASK", required = true)]
    pub task: Vec<String>,
}

impl Run {
    pub fn run(&self, _config: Option<&Path>) -> Result<()> {
        Err(Error::Unsupported { what: "run" })
    }
}
