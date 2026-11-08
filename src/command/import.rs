//! `clai import` (see `spec/command.md`).

use std::path::PathBuf;

use clap::{Args, ValueHint};

use super::{Context, Execute};
use crate::client::{unexpected, with_client};
use crate::error::{Error, Result};
use crate::output::write_stdout;
use crate::protocol::{Payload, Request};

#[derive(Debug, Args)]
pub struct Import {
    /// File written by `clai export`.
    #[arg(value_hint = ValueHint::FilePath)]
    pub file: PathBuf,
    /// Session name. Default: the name in the file, made unique.
    #[arg(long)]
    pub name: Option<String>,
}

impl Execute for Import {
    fn name(&self) -> &'static str {
        "import"
    }

    fn run(&self, ctx: &Context) -> Result<()> {
        let config = ctx.config();
        let path =
            std::path::absolute(&self.file).map_err(Error::io(self.file.display().to_string()))?;
        let request = Request::Import {
            path,
            name: self.name.clone(),
        };
        let name = with_client(config, true, async |client| {
            match client.request(request).await? {
                Payload::Session { session } => Ok(session.name),
                other => Err(unexpected(&other)),
            }
        })?;
        write_stdout(&format!("{name}\n"))
    }
}
