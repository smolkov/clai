use anyhow::Result;
use clap::Parser;
use crate::client::Client;
#[derive(Parser, Debug)]
pub struct Chat {
    text: Vec<String>,
}


impl Chat {
    pub async fn run(&self,client: &mut Client) -> Result<()> {
        let user_text = self.text.join(" ");
        let response = client.send_message(&user_text).await?;
        println!("{}", response);
        Ok(())
    }
}