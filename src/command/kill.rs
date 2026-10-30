//! `clai kill` (see `spec/command.md`).

use std::path::Path;

use clap::Args;

use crate::error::{Error, Result};

#[derive(Debug, Args)]
pub struct Kill {
    /// Names of the sessions to end.
    #[arg(required_unless_present = "all", conflicts_with = "all")]
    pub names: Vec<String>,
    /// End all sessions.
    #[arg(long)]
    pub all: bool,
}

impl Kill {
    pub fn run(&self, _config: Option<&Path>) -> Result<()> {
        Err(Error::Unsupported { what: "kill" })
    }
}
