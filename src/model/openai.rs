use reqwest::header::{HeaderMap, AUTHORIZATION, CONTENT_TYPE};
use serde_json::json;
use std::time::Duration;

use super::Result;

use crate::config::ModelConfig;
use crate::history::History;
use crate::tools::McpTool;

pub struct OpenaiModel {
    client: reqwest::Client,
    config: ModelConfig,
}

impl OpenaiModel {
    pub fn new(config: ModelConfig) -> OpenaiModel {
        let client = reqwest::Client::new();
        OpenaiModel { client, config }
    }
    pub async fn generate(
        &mut self,
        message: &str,
        history: &History,
        tools: &Vec<serde_json::Value>,
    ) -> Result<String> {
        let mut header = HeaderMap::new();
        header.insert(
            AUTHORIZATION,
            format!("Bearer {}", self.config.api_key).parse()?,
        );
        header.insert(CONTENT_TYPE, "application/json".parse()?);

        let request = json!({
            "model": self.config.model.as_str(),
            "input": [
                {
                    "role": "user",
                    "content": message,
                },
            ]
        });
        let output = self
            .client
            .post("https://api.openai.com/v1/responses")
            .timeout(Duration::from_secs(40))
            .headers(header)
            .json(&request)
            .send()
            .await?;
        let json_result: serde_json::Value = output.json().await?;
        let value = &json_result["output"][0]["content"][0]["text"];
        let msg = value
            .as_str()
            .ok_or(anyhow::anyhow!("get value error"))?
            .trim_matches('"')
            .to_string();
        Ok(msg)
    }
}
