use std::path::PathBuf;

use anyhow::Result;
use tokio::fs;

use crate::config;
use crate::data::Message;

pub struct History {
    pub name: String,
    pub path: PathBuf,
    pub messages: Vec<Message>,
}

impl History {
    pub fn new(name: &str) -> History {
        History {
            name: name.to_string(),
            path: PathBuf::from(name),
            messages: Vec::new(),
        }
    }
    pub async fn load(prompt: &str) -> Result<History> {
        let path = config::directory().join(prompt);
        let messages = if path.is_file() {
            let messages: Vec<Message> = serde_json::from_str(&fs::read_to_string(&path).await?)?;
            messages
        } else {
            Vec::new()
        };
        Ok(History {
            name: prompt.to_string(),
            path,
            messages,
        })
    }
    pub fn extend(&mut self, messages: Vec<Message>) {
        self.messages.extend(messages);
    }
    pub async fn update(&self) -> Result<()> {
        fs::write(&self.path, serde_json::to_string(&self.messages)?).await?;
        Ok(())
    }
    pub fn get_history(&self) -> &Vec<Message> {
        &self.messages
    }
}

#[cfg(test)]
mod test {}
