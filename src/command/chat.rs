use anyhow::Result;
use clap::Parser;
use crate::client::Client;

#[derive(Parser, Debug)]
pub struct Chat {
    text: Vec<String>,
}


impl Chat {
    pub async fn run(&self,client: &Client) -> Result<()> {
        
        Ok(())
    }
}