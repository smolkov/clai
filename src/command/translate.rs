use anyhow::Result;
use clap::Parser;

use crate::agent::Agent;
#[derive(Debug, Parser)]
pub struct Translate {
    /// English text for translate
    text: Vec<String>,
}

impl Translate {
    pub async fn run(&self, agent: &mut Agent) -> Result<()> {
        let user_text = format!("Translate into English:{}", self.text.join(" "));
        let response = agent.generate(&user_text).await?;
        println!("{}", response);
        Ok(())
    }
}
