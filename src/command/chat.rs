use anyhow::Result;
use clap::Parser;
use crate::client::Client;
use crate::data::Message;
use crate::{chat, cli};
#[derive(Parser, Debug)]
pub struct Chat {
    text: Vec<String>,
}


impl Chat {
    pub async fn run(&self,client: &Client) -> Result<()> {
        let user_text = self.text.join(" ");
        let chat = chat::Chat::new(client);
        let message = Message::new("user", &user_text);
        let response = chat.send(vec![message]).await?;
        println!("{}", response.choices.last().unwrap().message.content);
        Ok(())
    }
}