//! `clai kill` (see `spec/command.md`).

use std::io::Write;
use std::path::Path;

use clap::Args;
use clap_complete::ArgValueCompleter;

use crate::client::with_client;
use crate::complete;
use crate::error::{Error, Result};
use crate::protocol::Request;

#[derive(Debug, Args)]
pub struct Kill {
    /// Names of the sessions to end.
    #[arg(
        required_unless_present = "all",
        conflicts_with = "all",
        add = ArgValueCompleter::new(complete::sessions)
    )]
    pub names: Vec<String>,
    /// End all sessions.
    #[arg(long)]
    pub all: bool,
}

impl Kill {
    pub fn run(&self, config: Option<&Path>) -> Result<()> {
        let failed = with_client(config, true, async |client| {
            let names = if self.all {
                client.list().await?.into_iter().map(|s| s.name).collect()
            } else {
                self.names.clone()
            };
            let mut failed = false;
            for name in names {
                // Unknown names are reported; the known ones still end.
                if let Err(e) = client.request(Request::Kill { session: name }).await {
                    let _ = writeln!(std::io::stderr(), "clai: {e}");
                    failed = true;
                }
            }
            Ok(failed)
        })?;
        if failed { Err(Error::Exit(1)) } else { Ok(()) }
    }
}
