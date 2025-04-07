use clap::Parser;
use anyhow::Result;
use crate::client::Client;

#[derive(Debug,Parser)]
pub struct Correction {
	/// English text for correction
	text: Vec<String>,
}

impl Correction {
	pub async fn run(&self,client: &mut Client) -> Result<()> {
        let user_text = format!("Correct my English:{}", self.text.join(" "));
        let response = client.send_message(&user_text).await?;
        println!("{}", response);
        Ok(())
	}
}