//! `clai attach` (see `spec/command.md`).

use clap::Args;
use clap_complete::ArgValueCompleter;

use super::{Context, Execute};
use crate::complete;
use crate::error::Result;
use crate::tui;

#[derive(Debug, Args)]
pub struct Attach {
    /// Session name.
    #[arg(add = ArgValueCompleter::new(complete::sessions))]
    pub name: Option<String>,
}

impl Execute for Attach {
    fn name(&self) -> &'static str {
        "attach"
    }

    fn run(&self, ctx: &Context) -> Result<()> {
        tui::run(ctx, tui::Target::Attach(self.name.clone()))
    }
}
