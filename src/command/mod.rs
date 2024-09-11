use anyhow::Result;
use clap::Subcommand;

pub mod chat;
pub mod correction;
pub mod image;
pub mod translate;
pub mod commit;



use commit::Commit;
use chat::Chat;
use correction::Correction;
use translate::Translate;
use crate::client::Client;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Chat with ai
    Chat(Chat),
    /// Commit git message
    Commit(Commit),
    /// Correct my english
    Correction(Correction) ,
    /// Translate to english
    Translate(Translate)
}


impl Command {
    pub async fn run(&self,client: &Client) -> Result<()> {
        match self {
            Command::Chat(cmd)  => cmd.run(client).await,
            Command::Commit(cmd) => cmd.run(client).await,
            Command::Correction(cmd) => cmd.run(client).await,
            Command::Translate(cmd) => cmd.run(client).await,
        }
    }
}

