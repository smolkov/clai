//! `clai ls` (see `spec/command.md`).

use std::path::Path;

use clap::Args;

use crate::error::{Error, Result};

#[derive(Debug, Args)]
pub struct Ls {
    /// Print a JSON array instead of the table.
    #[arg(long)]
    pub json: bool,
}

impl Ls {
    pub fn run(&self, _config: Option<&Path>) -> Result<()> {
        Err(Error::Unsupported { what: "ls" })
    }
}
