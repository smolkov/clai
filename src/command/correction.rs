use clap::Parser;
use anyhow::Result;
use crate::{chat::Chat, client::Client, data::Message};

#[derive(Debug,Parser)]
pub struct Correction {
	/// English text for correction
	text: Vec<String>,
}

impl Correction {
	pub async fn run(&self,client: &Client) -> Result<()> {
		let user_text = self.text.join(" ");
        let chat = Chat::new(client);
        let message1 = Message::new("user", "Correct my English:");
        let message2 = Message::new("user", &user_text);
        let response = chat.send(vec![message1, message2]).await?;
        println!("{}", response.choices.last().unwrap().message.content);
        Ok(())
	}
}