use clap::Parser;
use anyhow::Result;

#[derive(Debug,Parser)]
pub struct Translate{

}

impl Translate {
	fn run(&self) -> Result<()> {
		Ok(())
	}
}