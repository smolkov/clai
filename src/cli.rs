use anyhow::Result;

use clap::Parser;

use crate::data::Message;
use crate::prompt::Prompt;

#[derive(Debug, Parser)]
#[command(author, version, about)]
pub struct Args {
    /// Load prompt name
    #[arg(short, long)]
    pub prompt: Option<String>,
    /// Save prompt
    #[arg(short, long, default_value_t = false)]
    pub update: bool,
    // User question
    pub input: Vec<String>,
}

impl Args {
    pub async fn prompt(&self) -> Result<Prompt> {
        let question = self.input.join(" ");
        let mut prompt = if let Some(prompt) = self.prompt.as_ref() {
            Prompt::load(prompt).await?
        } else {
            Prompt::empty()
        };
        prompt.messages.push(Message {
            role: "user".to_owned(),
            content: question,
        });
        Ok(prompt)
    }
}
