use clap::Parser;

#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Args {
    /// Load prompt name
    #[arg(short, long)]
    pub prompt: Option<String>,
    /// Load prompt name
    #[arg(short, long, default_value_t = false)]
    pub update: bool,
    // User question
    pub input: Vec<String>,
}
