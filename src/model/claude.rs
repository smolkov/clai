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

const MAX_TOOL_ITERATIONS: usize = 8;

impl ClaudeModel {
    pub fn new(config: ModelConfig) -> ClaudeModel {
        let client = reqwest::Client::new();
        ClaudeModel { client, config }
    }

    async fn call_api(
        &self,
        header: &HeaderMap,
        messages: &[serde_json::Value],
        tool_schemas: &[serde_json::Value],
    ) -> Result<serde_json::Value> {
        let date = chrono::Local::now().date_naive();
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
          "tools": tool_schemas,
        });

        let output = self
            .client
            .post("https://api.anthropic.com/v1/messages")
            .timeout(Duration::from_secs(40))
            .headers(header.clone())
            .json(&request)
            .send()
            .await?
            .json::<serde_json::Value>()
            .await?;

        if let Some(err) = output.get("error") {
            return Err(anyhow::anyhow!(
                "Claude API error: {}",
                err["message"].as_str().unwrap_or("Unknown error")
            ));
        }
        Ok(output)
    }

    pub async fn generate(
        &mut self,
        message: &str,
        history: &History,
        tools: &[Box<dyn McpTool>],
    ) -> Result<String> {
        let mut header = HeaderMap::new();
        header.insert("X-Api-Key", self.config.api_key.as_str().parse()?);
        header.insert(CONTENT_TYPE, "application/json".parse()?);
        header.insert("anthropic-version", "2023-06-01".parse()?);

        let tool_schemas: Vec<serde_json::Value> = tools.iter().map(|t| t.schema()).collect();

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

        for _ in 0..MAX_TOOL_ITERATIONS {
            let output = self.call_api(&header, &messages, &tool_schemas).await?;

            let content_blocks = output["content"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let stop_reason = output["stop_reason"].as_str().unwrap_or("");

            messages.push(json!({
                "role": "assistant",
                "content": content_blocks,
            }));

            if stop_reason != "tool_use" {
                let text = content_blocks
                    .iter()
                    .filter_map(|block| block["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
                return Ok(text);
            }

            let mut tool_results = Vec::new();
            for block in &content_blocks {
                if block["type"].as_str() != Some("tool_use") {
                    continue;
                }
                let tool_name = block["name"].as_str().unwrap_or_default();
                let tool_id = block["id"].as_str().unwrap_or_default();
                let input = block["input"].clone();

                let result = match tools.iter().find(|t| t.name() == tool_name) {
                    Some(tool) => tool
                        .call(input)
                        .await
                        .unwrap_or_else(|e| format!("Error: {e}")),
                    None => format!("Error: unknown tool '{tool_name}'"),
                };

                tool_results.push(json!({
                    "type": "tool_result",
                    "tool_use_id": tool_id,
                    "content": result,
                }));
            }
            messages.push(json!({
                "role": "user",
                "content": tool_results,
            }));
        }

        Err(anyhow::anyhow!(
            "Exceeded maximum of {MAX_TOOL_ITERATIONS} tool-use iterations"
        ))
    }
}
