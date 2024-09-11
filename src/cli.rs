
use clap::Parser;

use crate::command::Command;

#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Args {
    /// Subcommand
    #[command(subcommand)]
    pub command: Command,
}