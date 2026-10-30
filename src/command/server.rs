//! `clai server`, `clai server stop`, `clai server status` (see `spec/command.md`).

use std::path::Path;

use clap::{Args, Subcommand};

use crate::error::{Error, Result};

#[derive(Debug, Args)]
pub struct Server {
    #[command(subcommand)]
    pub action: Option<ServerAction>,
}

#[derive(Debug, Subcommand)]
pub enum ServerAction {
    /// Stop the running server and end all its sessions.
    Stop {
        /// Do not ask for confirmation.
        #[arg(long)]
        force: bool,
    },
    /// Print whether a server runs, its PID and session count.
    Status,
}

impl Server {
    pub fn name(&self) -> &'static str {
        match self.action {
            None => "server",
            Some(ServerAction::Stop { .. }) => "server stop",
            Some(ServerAction::Status) => "server status",
        }
    }

    pub fn run(&self, _config: Option<&Path>) -> Result<()> {
        Err(Error::Unsupported { what: self.name() })
    }
}
