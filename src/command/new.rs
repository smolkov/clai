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
    /// Turn on skill NAME in the session; repeatable. `clai skills` lists
    /// them.
    #[arg(long = "skill", value_name = "NAME")]
    pub skills: Vec<String>,
    /// Workspace. Default: the current directory.
    #[arg(value_hint = ValueHint::DirPath)]
    pub dir: Option<PathBuf>,
    /// `clai --continue`: attach to the last session of the workspace if
    /// there is one (RFC 0025).
    #[arg(skip)]
    pub resume: bool,
}

impl Execute for New {
    fn name(&self) -> &'static str {
        "new"
    }

    fn run(&self, ctx: &Context) -> Result<()> {
        let request = request(
            ctx,
            self.name.clone(),
            self.model.clone(),
            self.dir.as_deref(),
            self.skills.clone(),
        )?;
        if self.resume {
            return tui::run(
                ctx,
                tui::Target::Continue {
                    request,
                    model: self.model.clone(),
                },
            );
        }
        if !self.detach {
            return tui::run(ctx, tui::Target::New(request));
        }
        let session = with_client(ctx, true, async |client| create(client, request).await)?;
        write_stdout(&format!("{}\n", session.name))
    }
}
