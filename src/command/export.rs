//! `clai export` (see `spec/command.md`).

use std::path::PathBuf;

use clap::{Args, ValueHint};
use clap_complete::ArgValueCompleter;

use super::{Context, Execute};
use crate::client::with_client;
use crate::complete;
use crate::error::{Error, Result};
use crate::protocol::Request;

#[derive(Debug, Args)]
pub struct Export {
    /// Session name.
    #[arg(add = ArgValueCompleter::new(complete::sessions))]
    pub name: String,
    /// File to write; replaced if it exists.
    #[arg(value_hint = ValueHint::FilePath)]
    pub file: PathBuf,
}

impl Execute for Export {
    fn name(&self) -> &'static str {
        "export"
    }

    fn run(&self, ctx: &Context) -> Result<()> {
        let config = ctx.config();
        let path =
            std::path::absolute(&self.file).map_err(Error::io(self.file.display().to_string()))?;
        let request = Request::Export {
            session: self.name.clone(),
            path,
        };
        with_client(config, true, async |client| {
            client.request(request).await.map(|_| ())
        })
    }
}
