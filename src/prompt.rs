use anyhow::Result;
use tokio::fs;

use crate::data::Message;

pub struct Prompt {
    pub content: String,
}

impl Prompt {
    pub async fn load(prompt: &str) -> Result<Prompt> {
        // let path = dirs::config_dir()
            // .unwrap_or(PathBuf::from("."))
            // .join(&format!("clai/{prompt}"));
        let content = fs::read_to_string(prompt).await?;
        Ok(Prompt { content })
    }
    pub async fn messages(&self) -> Vec<Message> {
        let mut messages = Vec::new();
        for msg in self.content.split('\n').into_iter() {
			let msg = msg.trim();
            let message = if let Some((role, msg)) = msg.split_once(':') {
                Message {
                    role: role.to_owned(),
                    content: msg.to_owned(),
                }
            } else {
                Message {
                    role: "user".to_owned(),
                    content: msg.to_owned(),
                }
            };
            messages.push(message);
        }
        messages
    }
}

#[cfg(test)]
mod test {

    use super::*;

    #[tokio::test]
    async fn test_messages() {
		let prompt = r#"user:description1
		user:description2
		user:question"#;
		let prompt = Prompt{
			content: prompt.to_owned()
		};
		assert_eq!(vec![Message::new("user","description1"),Message::new("user","description2"),Message::new("user","question")],prompt.messages().await);

		let prompt = Prompt{
			content: r#"description
			user:question"#.to_owned(),
		};	
		assert_eq!(vec![Message::new("user","description"),Message::new("user","question")],prompt.messages().await)

	}

}
