use clap::Parser;
use anyhow::Result;
use crate::client::Client;

#[derive(Debug,Parser)]
pub struct Image {
	text: Vec<String>,
}

impl Image {
	pub async fn run(&self,_client: &mut Client) -> Result<()> {
		Ok(())
	}
}