use anyhow::Result;

use crate::config::ModelConfig;

pub mod gemini;
pub mod openai;

use async_trait::async_trait;

#[async_trait]
pub trait GenModel: Send + Sync {
    async fn generate(&self, prompt: &str) -> Result<String>;
}

pub enum Model {
    Gemini(gemini::GeminiModel),
    Openai(openai::OpenaiModel),
}

impl Model {
    pub async fn generate(&mut self, message: &str) -> Result<String> {
        match self {
            Self::Gemini(model) => model.generate(message).await,
            Self::Openai(model) => model.generate(message).await,
        }
    }
}