use anyhow::Result;
use async_trait::async_trait;

use crate::history::History;
use crate::tools::McpTool;

pub mod claude;
pub mod gemini;
pub mod openai;

pub type DefaultModel = gemini::GeminiModel;

#[async_trait]
pub trait GenModel: Send + Sync {
    async fn generate(&self, prompt: &str) -> Result<String>;
}

pub enum Model {
    Gemini(gemini::GeminiModel),
    Openai(openai::OpenaiModel),
    Claude(claude::ClaudeModel),
}

impl Model {
    pub async fn generate(
        &mut self,
        message: &str,
        history: &History,
        tools: &[Box<dyn McpTool>],
    ) -> Result<String> {
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
