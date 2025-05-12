use std::time::Duration;
use serde_json::json;
use reqwest::header::{HeaderMap, CONTENT_TYPE};

use super::Result;
use crate::config::Config;





pub struct GeminiClient {
	client : reqwest::Client,
	config: Config,
}


impl GeminiClient {
	pub fn new(config: Config) -> GeminiClient {
        let client = reqwest::Client::new();
        GeminiClient { client, config }
    }
	pub async fn send_message(&mut self, message: &str) -> Result<String> {

		let url = format!("https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",self.config.model,self.config.api_key);
		let mut header = HeaderMap::new();
        header.insert(CONTENT_TYPE, "application/json".parse()?);
		
// curl "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.0-flash:generateContent?key=$GEMINI_API_KEY" \
//  -H 'Content-Type: application/json' \
//  -X POST \
//  -d '{
//    "contents": [
//      {
//        "parts": [
//          {
//            "text": "How does AI work?"
//          }
//        ]
//      }
//    ]
//  }'
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
		let result:serde_json::Value = output.json().await?;
		let msg = result["candidates"][0]["content"]["parts"][0]["text"].to_string();
        Ok(msg)
	}
}