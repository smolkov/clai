use clap::Parser;
use anyhow::Result;

#[derive(Debug,Parser)]
pub struct English {

}

impl English {
	fn run(&self) -> Result<()> {
		Ok(())
	}
}