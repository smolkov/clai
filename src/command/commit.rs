use anyhow::Result;
use clap::Parser;
use tokio::process::Command;

use crate::{chat::Chat, client::Client, data::Message};

#[derive(Debug, Parser)]
pub struct Commit {}

impl Commit {
    pub async fn run(&self, client: &Client) -> Result<()> {
        let output = Command::new("git").arg("diff").output().await?;
        let git_diff = String::from_utf8_lossy(&output.stdout);
        let chat = Chat::new(client);
        let message = format!("What is the best way to summarize these changes in a Git commit message: {}", git_diff);
        let message = Message::new("user", &message);
        let response = chat.send(vec![message]).await?;
        println!("{}", response.choices.last().unwrap().message.content);
		
        Ok(())
    }
}
