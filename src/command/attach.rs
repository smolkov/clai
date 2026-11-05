//! `clai attach` (see `spec/command.md`).

use std::path::Path;

use clap::Args;
use clap_complete::ArgValueCompleter;

use crate::complete;
use crate::error::Result;
use crate::tui;

#[derive(Debug, Args)]
pub struct Attach {
    /// Session name.
    #[arg(add = ArgValueCompleter::new(complete::sessions))]
    pub name: Option<String>,
}

impl Attach {
    pub fn run(&self, config: Option<&Path>) -> Result<()> {
        tui::run(config, tui::Target::Attach(self.name.clone()))
    }
}
