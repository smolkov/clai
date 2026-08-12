use clap::Parser;
use anyhow::Result;
use crate::agent::Agent;

#[derive(Debug,Parser)]
pub struct Image {
	text: Vec<String>,
}

impl Image {
	pub async fn run(&self,_agent: &mut Agent) -> Result<()> {
		Ok(())
	}
}