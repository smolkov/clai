use crate::agent::Agent;
use anyhow::Result;
use clap::Parser;
#[derive(Parser, Debug)]
pub struct Chat {
    text: Vec<String>,
}

impl Chat {
    pub async fn run(&self, agent: &mut Agent) -> Result<()> {
        let user_text = self.text.join(" ");
        let response = agent.generate(&user_text).await?;
        println!("{}", response.content.as_text().unwrap());
        Ok(())
    }
}
