pub mod chat;
pub mod english;
pub mod image;
pub mod translate;

use chat::Chat;
use clap::Subcommand;

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Chat with ai
    Chat(Chat),
}
