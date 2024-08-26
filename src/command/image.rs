use clap::Parser;
use anyhow::Result;

#[derive(Debug,Parser)]
pub struct Image{

}

impl Image {
	pub fn run(&self) -> Result<()> {
		Ok(())
	}
}