//! `clai send` (see `spec/command.md`).

use std::path::Path;

use clap::Args;

use crate::error::{Error, Result};

#[derive(Debug, Args)]
pub struct Send {
    /// Session name.
    pub name: String,
    /// Message; `-` reads it from standard input.
    #[arg(required = true)]
    pub message: Vec<String>,
}

impl Send {
    pub fn run(&self, _config: Option<&Path>) -> Result<()> {
        Err(Error::Unsupported { what: "send" })
    }
}
