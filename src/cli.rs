use clap::{Parser};


#[derive(Debug,Parser)]
#[command(author, version, about)]
pub struct Args{

	/// Load prompt name
    #[arg(short, long)]
    pub prompt: Option<String>,

	// User question
	pub input: Vec<String>
}