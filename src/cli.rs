use clap::{Parser};


#[derive(Debug,Parser)]
pub struct Args{
	pub input: Vec<String>
}