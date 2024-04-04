use anyhow::Result;

use clap::{Parser, Subcommand};


#[derive(Debug,Parser)]
pub struct Args{
	pub input: Vec<String>
}