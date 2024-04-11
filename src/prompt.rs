use std::path::PathBuf;

use anyhow::Result;
use tokio::fs;

use crate::config;
use crate::data::{Message, OpenAIRequest};

pub struct Prompt {
    pub path: PathBuf,
    pub messages: Vec<Message>,
}

impl Prompt {
    pub fn empty() -> Prompt {
        Prompt{
            path: PathBuf::from("prompt"),
            messages: Vec::new(),
        }
    }
    pub async fn load(prompt: &str) -> Result<Prompt> {
        let path = config::directory().join(prompt);
        let messages = if path.is_file() {
            let messages: Vec<Message> = serde_json::from_str(&fs::read_to_string(&path).await?)?;
            messages
        } else {
            Vec::new()
        };
        Ok(Prompt { path, messages })
    }
    pub fn extend(&mut self, messages: Vec<Message>) {
        self.messages.extend(messages);
    }
    pub async fn update(&self) -> Result<()> {
        fs::write(&self.path, serde_json::to_string(&self.messages)?).await?;
        Ok(())
    }
}

#[cfg(test)]
mod test {}
