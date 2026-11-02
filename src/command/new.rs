//! `clai new`, and `clai` without a command (see `spec/command.md`).

use std::io::Write;
use std::path::{Path, PathBuf};

use clap::{Args, ValueHint};
use clap_complete::ArgValueCompleter;

use crate::client::{unexpected, with_client};
use crate::complete;
use crate::config::MODEL_ENV;
use crate::error::{Error, Result};
use crate::output::write_stdout;
use crate::protocol::{Payload, Request};

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

impl New {
    pub fn run(&self, config: Option<&Path>) -> Result<()> {
        if !self.detach {
            // Attaching needs the TUI.
            return Err(Error::Unsupported { what: "new" });
        }
        let workspace = workspace(self.dir.as_deref())?;
        // `CLAI_MODEL` of this client; the server checks the model.
        let model = self.model.clone().or_else(|| {
            std::env::var(MODEL_ENV)
                .ok()
                .filter(|v| !v.trim().is_empty())
        });
        let request = Request::New {
            workspace,
            name: self.name.clone(),
            model,
        };
        let (session, others) = with_client(config, true, async |client| {
            let session = match client.request(request).await? {
                Payload::Session { session } => session,
                other => return Err(unexpected(&other)),
            };
            let others: Vec<String> = client
                .list()
                .await?
                .into_iter()
                .filter(|s| s.path == session.path && s.id != session.id)
                .map(|s| s.name)
                .collect();
            Ok((session, others))
        })?;
        if !others.is_empty() {
            let _ = writeln!(
                std::io::stderr(),
                "clai: note: {} also {} in {}; sessions may change the same files",
                others.join(", "),
                if others.len() == 1 { "works" } else { "work" },
                session.workspace
            );
        }
        write_stdout(&format!("{}\n", session.name))
    }
}

/// `DIR` or the current directory, absolute; it must be a directory.
pub fn workspace(dir: Option<&Path>) -> Result<PathBuf> {
    let dir = match dir {
        Some(dir) => dir.to_path_buf(),
        None => std::env::current_dir().map_err(|source| Error::Io {
            context: "current directory".to_owned(),
            source,
        })?,
    };
    if !dir.is_dir() {
        return Err(Error::Usage(format!("{}: not a directory", dir.display())));
    }
    std::path::absolute(&dir).map_err(|source| Error::Io {
        context: dir.display().to_string(),
        source,
    })
}
