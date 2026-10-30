//! `clai attach` (see `spec/command.md`).

use std::path::Path;

use clap::Args;

use crate::error::{Error, Result};

#[derive(Debug, Args)]
pub struct Attach {
    /// Session name.
    pub name: Option<String>,
}

impl Attach {
    pub fn run(&self, _config: Option<&Path>) -> Result<()> {
        Err(Error::Unsupported { what: "attach" })
    }
}
