//! `codeagent import` (see `spec/command.md`).

use std::path::PathBuf;

use clap::{Args, ValueHint};

use super::{Context, Execute};
use crate::client::{unexpected, with_client};
use crate::error::{Error, Result};
use crate::output::write_stdout;
use crate::protocol::{Payload, Request};

#[derive(Debug, Args)]
pub struct Import {
    /// File written by `codeagent export`.
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
        let path =
            std::path::absolute(&self.file).map_err(Error::io(self.file.display().to_string()))?;
        let request = Request::Import {
            path,
            name: self.name.clone(),
        };
        let (name, dropped) = with_client(ctx, true, async |client| {
            match client.request(request).await? {
                Payload::Imported {
                    session,
                    dropped_rules,
                } => Ok((session.name, dropped_rules)),
                other => Err(unexpected(&other)),
            }
        })?;
        for rule in dropped {
            crate::output::note(&format!(
                "left out the session rule {} (rules are not imported)",
                crate::text::safe::terminal(&rule)
            ));
        }
        write_stdout(&format!("{name}\n"))
    }
}
