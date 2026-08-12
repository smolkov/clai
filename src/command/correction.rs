use crate::agent::Agent;
use anyhow::Result;
use clap::Parser;

#[derive(Debug, Parser)]
pub struct Correction {
    /// English text for correction
    text: Vec<String>,
}

impl Correction {
    pub async fn run(&self, agent: &mut Agent) -> Result<()> {
        let user_text = format!("Correct and improve my English:{}", self.text.join(" "));
        let response = agent.send_message(&user_text).await?;
        println!("{}", response);
        Ok(())
    }
}
