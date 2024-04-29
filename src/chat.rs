use anyhow::Result;

use crate::client::Client;
use crate::data::{OpenAIChatResponse, OpenAIRequest};
use crate::prompt::Prompt;

pub struct Chat<'a> {
    client: &'a Client,
}

impl<'a> Chat<'a> {
    pub fn new(client: &'a Client) -> Chat<'a> {
        Chat { client }
    }

    pub async fn send(&self, prompt: &Prompt) -> Result<OpenAIChatResponse> {
		let req: OpenAIRequest = OpenAIRequest {
			model: self.client.config.model.clone(),
			messages: prompt.messages.clone(),
		};
        let output = self.client.post("chat/completions", &req).await?;
        Ok(output)
    }
}
