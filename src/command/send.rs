//! `clai send` (see `spec/command.md`).

use clap::Args;
use clap_complete::ArgValueCompleter;

use super::common::input_text;
use super::{Context, Execute};
use crate::client::with_client;
use crate::complete;
use crate::error::Result;
use crate::protocol::Request;

#[derive(Debug, Args)]
pub struct Send {
    /// Session name.
    #[arg(add = ArgValueCompleter::new(complete::sessions))]
    pub name: String,
    /// Message; `-` reads it from standard input.
    #[arg(required = true)]
    pub message: Vec<String>,
}

impl Execute for Send {
    fn name(&self) -> &'static str {
        "send"
    }

    fn run(&self, ctx: &Context) -> Result<()> {
        let config = ctx.config();
        let text = input_text(&self.message, "message")?.text;
        let request = Request::Send {
            session: self.name.clone(),
            text,
        };
        with_client(config, true, async |client| {
            client.request(request).await.map(|_| ())
        })
    }
}
