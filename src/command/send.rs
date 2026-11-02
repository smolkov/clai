//! `clai send` (see `spec/command.md`).

use std::io::Read;
use std::path::Path;

use clap::Args;
use clap_complete::ArgValueCompleter;

use crate::client::with_client;
use crate::complete;
use crate::error::{Error, Result};
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

impl Send {
    pub fn run(&self, config: Option<&Path>) -> Result<()> {
        let text = if self.message == ["-"] {
            let mut text = String::new();
            std::io::stdin()
                .read_to_string(&mut text)
                .map_err(Error::io("standard input"))?;
            text
        } else {
            self.message.join(" ")
        };
        let text = text.trim().to_owned();
        if text.is_empty() {
            return Err(Error::Usage("the message is empty".to_owned()));
        }
        let request = Request::Send {
            session: self.name.clone(),
            text,
        };
        with_client(config, true, async |client| {
            client.request(request).await.map(|_| ())
        })
    }
}
