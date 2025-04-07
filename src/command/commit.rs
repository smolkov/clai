use anyhow::Result;
use clap::Parser;
use tokio::process::Command;

use crate::client::Client;


#[derive(Debug, Parser)]
pub struct Commit {}

impl Commit {
    pub async fn run(&self, client: &mut Client) -> Result<()> {
        let output = Command::new("git").arg("diff").output().await?;
        let git_diff = String::from_utf8_lossy(&output.stdout);
        let message = format!("What is the best way to summarize these changes in a Git commit message: {}", git_diff);
        let response = client.send_message(&message).await?;
        println!("{}", response);
		
        Ok(())
    }
}
