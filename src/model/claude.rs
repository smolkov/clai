use reqwest::header::{HeaderMap, CONTENT_TYPE};
use serde_json::json;
use std::time::Duration;

use super::Result;
use crate::config::ModelConfig;
use crate::history::History;
use crate::tools::McpTool;

pub struct ClaudeModel {
    client: reqwest::Client,
    config: ModelConfig,
}

impl ClaudeModel {
    pub fn new(config: ModelConfig) -> ClaudeModel {
        let client = reqwest::Client::new();
        ClaudeModel { client, config }
    }

    pub async fn generate(
        &mut self,
        message: &str,
        history: &History,
        tools: &Vec<serde_json::Value>,
    ) -> Result<String> {
        let url = format!("https://api.anthropic.com/v1/messages",);
        let date = chrono::Local::now().date_naive();
        let mut header = HeaderMap::new();
        header.insert("X-Api-Key", self.config.api_key.as_str().parse()?);
        header.insert(CONTENT_TYPE, "application/json".parse()?);
        header.insert("anthropic-version", "2023-06-01".parse()?);
        let mut messages = history
            .get_history()
            .iter()
            .map(|msg| {
                json!({
                    "role": msg.role,
                    "content": msg.content,
                })
            })
            .collect::<Vec<_>>();
        messages.push(json!({
            "role": "user",
            "content": message,
        }));
        let request = json!({
            "max_tokens": 1024,
            "model": self.config.model,
            "temperature": 1,
            "messages": messages,
            "system": [
            {
              "text": format!("Today'\''s date is {}.",date.format("%Y-%m-%d")),
              "type": "text"
            }
          ],
          "tools": tools,
        });

        let output = self
            .client
            .post(&url)
            .timeout(Duration::from_secs(40))
            .headers(header)
            .json(&request)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        println!("Claude response: {:?}", output);
        if let Some(text) = output["content"][0]["text"].as_str() {
            Ok(text.to_string())
        } else {
            Err(anyhow::anyhow!("No completion found in response"))
        }
    }
}
