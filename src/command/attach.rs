//! `clai attach` (see `spec/command.md`).

use std::path::Path;

use clap::Args;
use clap_complete::ArgValueCompleter;

use crate::complete;
use crate::error::{Error, Result};

#[derive(Debug, Args)]
pub struct Attach {
    /// Session name.
    #[arg(add = ArgValueCompleter::new(complete::sessions))]
    pub name: Option<String>,
}

impl Attach {
    pub fn run(&self, _config: Option<&Path>) -> Result<()> {
        Err(Error::Unsupported { what: "attach" })
    }
}
