use std::path::PathBuf;

use anyhow::Result;
use tokio::fs;
use uuid::Uuid;

use crate::config;
use crate::data::Message;

#[derive(Debug, Clone)]
pub struct Session {
    pub id: Uuid,
    pub name: String,
    pub path: PathBuf,
    pub messages: Vec<Message>,
}

impl Session {
    pub fn new(name: &str) -> Session {
        Session {
            id: Uuid::new_v4(),
            name: name.to_string(),
            path: PathBuf::from(name),
            messages: Vec::new(),
        }
    }
    pub async fn load(prompt: &str) -> Result<Session> {
        let path = config::directory().join(prompt);
        let messages = if path.is_file() {
            let messages: Vec<Message> = serde_json::from_str(&fs::read_to_string(&path).await?)?;
            messages
        } else {
            Vec::new()
        };
        Ok(Session {
            id: Uuid::new_v4(),
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
