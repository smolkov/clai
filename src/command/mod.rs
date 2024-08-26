pub mod chat;
pub mod english;
pub mod image;
pub mod translate;
pub mod commit;

use chat::Chat;
use clap::Subcommand;
use commit::Commit;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Chat with ai
    Chat(Chat),
    
}
