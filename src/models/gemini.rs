use reqwest::header::{HeaderMap, CONTENT_TYPE};
use serde_json::json;
use std::time::Duration;

use super::Result;
use crate::config::ModelConfig;

pub struct GeminiModel {
    client: reqwest::Client,
    config: ModelConfig,
}

impl GeminiModel {
    pub fn new(config: ModelConfig) -> GeminiModel {
        let client = reqwest::Client::new();
        GeminiModel { client, config }
    }
    pub async fn generate(&mut self, message: &str) -> Result<String> {
        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent",
            self.config.model
        );
        let mut header = HeaderMap::new();
        header.insert("x-goog-api-key", self.config.api_key.as_str().parse()?);
        header.insert(CONTENT_TYPE, "application/json".parse()?);

        //curl "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash:generateContent" \
        //  -H "x-goog-api-key: $GEMINI_API_KEY" \
        //  -H 'Content-Type: application/json' \
        //  -X POST \
        //  -d '{
        //    "contents": [
        //     {
        //        "parts": [
        //          {
        //            "text": "Explain how AI works in a few words"
        //          }
        //        ]
        //      }
        //    ]
        //  }'
        //
        let request = json!({
            "contents": [
                {
                    "parts": [
                        {
                            "text": message
                        }
                    ]
                }
            ]
        });
        let output = self
            .client
            .post(&url)
            .timeout(Duration::from_secs(40))
            .headers(header)
            .json(&request)
            .send()
            .await?;
        let result: serde_json::Value = output.json().await?;
        if result.get("error").is_some() {
            return Err(anyhow::anyhow!(
                "Gemini API error: {}",
                result["error"]["message"]
                    .as_str()
                    .unwrap_or("Unknown error")
            ));
        }
        let value = &result["candidates"][0]["content"]["parts"][0]["text"];
        let msg = value
            .as_str()
            .ok_or(anyhow::anyhow!("gemini: get text value unknown error"))?
            .trim_matches('"')
            .to_string();
        Ok(msg)
    }
}
