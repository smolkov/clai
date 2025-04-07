use std::fmt::format;

use anyhow::Result;
use clap::Parser;

use crate::client::Client;
#[derive(Debug, Parser)]
pub struct Translate {
    /// English text for translate
    text: Vec<String>,
}

impl Translate {
    pub async fn run(&self, client: &mut Client) -> Result<()> {
        let user_text = format!("Translate into English:{}",self.text.join(" "));
        let response = client.send_message(&user_text).await?;
        println!("{}", response);
        Ok(())
    }
}
