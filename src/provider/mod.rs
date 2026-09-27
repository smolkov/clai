use anyhow::Result;
use async_trait::async_trait;

use crate::message::Message;
use crate::session::Session;
use crate::tools::McpTool;

pub mod claude;
pub mod gemini;
pub mod openai;

pub type DefaultModel = gemini::GeminiModel;

#[async_trait]
pub trait GenProvider: Send + Sync {
    async fn generate(&self, prompt: &str) -> Result<Message>;
}

pub enum Provider {
    Gemini(gemini::GeminiModel),
    Openai(openai::OpenaiModel),
    Claude(claude::ClaudeModel),
}

impl Provider {
    pub async fn generate(
        &mut self,
        message: &str,
        history: &Session,
        tools: &[Box<dyn McpTool>],
    ) -> Result<Message> {
        match self {
            Self::Gemini(model) => model.generate(message, history, tools).await,
            Self::Openai(model) => model.generate(message, history, tools).await,
            Self::Claude(model) => model.generate(message, history, tools).await,
        }
    }
    pub fn new(config: crate::config::ModelConfig) -> Result<Self> {
        match config.provider.as_str() {
            "gemini" => Ok(Self::Gemini(gemini::GeminiModel::new(config))),
            "openai" => Ok(Self::Openai(openai::OpenaiModel::new(config))),
            "claude" => Ok(Self::Claude(claude::ClaudeModel::new(config))),
            _ => Err(anyhow::anyhow!(
                "Unknown model provider: {}",
                config.provider
            )),
        }
    }
}
