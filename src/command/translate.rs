use crate::{chat::Chat, client::Client, data::Message};
use anyhow::Result;
use clap::Parser;

#[derive(Debug, Parser)]
pub struct Translate {
    /// English text for translate
    text: Vec<String>,
}

impl Translate {
    pub async fn run(&self, client: &Client) -> Result<()> {
        let user_text = self.text.join(" ");
        let chat = Chat::new(client);
        let message1 = Message::new("user", "Please translate into English:");
        let message2 = Message::new("user", &user_text);
        let response = chat.send(vec![message1, message2]).await?;
        println!("{}", response.choices.last().unwrap().message.content);
        Ok(())
    }
}
