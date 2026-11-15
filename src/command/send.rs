//! `codeagent send` (see `spec/command.md`).

use clap::Args;
use clap_complete::ArgValueCompleter;

use super::common::{input_text, last_session_name};
use super::{Context, Execute};
use crate::client::with_client;
use crate::complete;
use crate::error::Result;
use crate::protocol::Request;

#[derive(Debug, Args)]
pub struct Send {
    /// Send to the session of this directory that changed last; then all
    /// words are the message.
    #[arg(short = 'c', long = "continue", id = "continue")]
    pub resume: bool,
    /// Session name.
    #[arg(required_unless_present = "continue", add = ArgValueCompleter::new(complete::sessions))]
    pub name: Option<String>,
    /// Message; `-` reads it from standard input.
    #[arg(required_unless_present = "continue")]
    pub message: Vec<String>,
}

impl Send {
    /// The words of the message: with `--continue` the first word too.
    pub fn words(&self) -> Vec<String> {
        if self.resume {
            self.name.iter().chain(&self.message).cloned().collect()
        } else {
            self.message.clone()
        }
    }
}

impl Execute for Send {
    fn name(&self) -> &'static str {
        "send"
    }

    fn run(&self, ctx: &Context) -> Result<()> {
        let text = input_text(&self.words(), "message")?.text;
        // Only `--continue` looks at the current directory.
        let workspace = if self.resume {
            Some(ctx.workspace(None)?)
        } else {
            None
        };
        let name = self.name.clone().unwrap_or_default();
        with_client(ctx, true, async |client| {
            let session = match &workspace {
                Some(workspace) => {
                    last_session_name(&client.list().await?, workspace, &ctx.startup)?
                }
                None => name,
            };
            client
                .request(Request::Send { session, text })
                .await
                .map(|_| ())
        })
    }
}
