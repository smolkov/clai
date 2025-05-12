use anyhow::Result;

use crate::config::Config;

pub mod gemini;
pub mod openai;

pub enum Client {
	Gemini(gemini::GeminiClient),
	Openai(openai::OpenaiClient),
}


impl Client {
	pub async fn send_message(&mut self, message: &str) -> Result<String>{
		match self {
			Self::Gemini(client)	 => client.send_message(message).await,
			Self::Openai(client)	 => client.send_message(message).await,
		}
	}
}


pub fn create_client(config: Config) -> Client {
	match config.provider.as_str() {
		"openai" => Client::Openai(openai::OpenaiClient::new(config)),
		"gemini" => Client::Gemini(gemini::GeminiClient::new(config)),
		_ => Client::Gemini(gemini::GeminiClient::new(config)),
	}
}