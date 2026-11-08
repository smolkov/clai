//! `clai new`, and `clai` without a command (see `spec/command.md`).

use std::path::PathBuf;

use clap::{Args, ValueHint};
use clap_complete::ArgValueCompleter;

use super::common::{create, request};
use super::{Context, Execute};
use crate::client::with_client;
use crate::complete;
use crate::error::Result;
use crate::output::write_stdout;
use crate::tui;

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

impl Execute for New {
    fn name(&self) -> &'static str {
        "new"
    }

    fn run(&self, ctx: &Context) -> Result<()> {
        let config = ctx.config();
        let request = request(self.name.clone(), self.model.clone(), self.dir.as_deref())?;
        if !self.detach {
            return tui::run(ctx, tui::Target::New(request));
        }
        let session = with_client(config, true, async |client| create(client, request).await)?;
        write_stdout(&format!("{}\n", session.name))
    }
}
