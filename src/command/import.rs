//! `clai import` (see `spec/command.md`).

use std::path::{Path, PathBuf};

use clap::{Args, ValueHint};

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

impl Import {
    pub fn run(&self, config: Option<&Path>) -> Result<()> {
        let path = std::path::absolute(&self.file).map_err(|source| Error::Io {
            context: self.file.display().to_string(),
            source,
        })?;
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
