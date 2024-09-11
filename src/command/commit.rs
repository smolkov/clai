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
        let message = format!("Write a Git commit message for these changes: {}", git_diff);
        // println!("{}", message);
        let message = Message::new("user", &message);
        // let message1 = Message::new("user", "Write a Git commit message for following changes:");
        // let message2 = Message::new("user", &git_diff);
        let response = chat.send(vec![message]).await?;
        println!("{}", response.choices.last().unwrap().message.content);
        Ok(())
    }
}
