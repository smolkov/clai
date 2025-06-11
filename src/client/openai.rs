use serde_json::json;
use std::time::Duration;

use super::Result;

use crate::config::Config;

pub struct OpenaiClient {
    client: reqwest::Client,
    config: Config,
}

impl OpenaiClient {
    pub fn new(config: Config) -> OpenaiClient {
        let client = reqwest::Client::new();
        OpenaiClient { client, config }
    }
    pub async fn send_message(&mut self, message: &str) -> Result<String> {
        //          curl https://api.openai.com/v1/chat/completions \
        //    -H "Content-Type: application/json" \
        //    -H "Authorization: Bearer $OPENAI_API_KEY" \
        //    -d '{
        //      "model": "gpt-4.1",
        //      "messages": [
        //        {
        //          "role": "developer",
        //          "content": "You are a helpful assistant."
        //        },
        //        {
        //          "role": "user",
        //          "content": "Hello!"
        //        }
        //      ]
        //    }'
        let request = json!({
            "model": self.config.model.as_str(),
            "messages": [
                {
                    "role": "user",
                    "content": message,
                },
            ]
        });
        let output = self
            .client
            .post("https://api.openai.com/v1/chat/completions")
            .timeout(Duration::from_secs(40))
            .headers(self.config.header()?)
            .json(&request)
            .send()
            .await?;
        let json_result: serde_json::Value = output.json().await?;
        let value = &json_result["choices"][0]["message"]["content"];
        let msg = value
            .as_str()
            .ok_or(anyhow::anyhow!("get value error"))?
            .trim_matches('"')
            .to_string();
        Ok(msg)
    }
}
