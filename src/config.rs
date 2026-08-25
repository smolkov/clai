use std::path::PathBuf;

use anyhow::Result;
use log;
use reqwest::header::{HeaderMap, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use tokio::fs;

pub const OPENAI_API_KEY: &str = "OPENAI_API_KEY";
pub const OPENAI_MODEL: &str = "OPENAI_MODEL";

pub const GEMINI_API_KEY: &str = "GEMINI_API_KEY";
pub const GEMINI_MODEL: &str = "GEMINI_MODEL";
#[derive(Serialize, Deserialize, Clone)]
pub struct ModelConfig {
    pub model: String,
    pub provider: String,
    pub api_key: String,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
    pub path: Option<PathBuf>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub provider: String,
    pub models: Vec<ModelConfig>,
}

impl ModelConfig {
    #[allow(dead_code)]
    pub fn check(&self) -> Result<()> {
        if self.api_key == "empty" {
            return Err(anyhow::anyhow!(
                "please set api key in config file or in 'OPENAI_API_KEY` environment variable"
            ));
        }
        Ok(())
    }
    pub fn with_api_key<S: Into<String>>(mut self, api_key: S) -> Self {
        self.api_key = api_key.into();
        self
    }
}

// Hand-rolled Debug so `api_key` never lands in logs, panics, or error
// messages via `{:?}`.
impl std::fmt::Debug for ModelConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelConfig")
            .field("model", &self.model)
            .field("provider", &self.provider)
            .field("api_key", &"<redacted>")
            .field("max_tokens", &self.max_tokens)
            .field("temperature", &self.temperature)
            .field("path", &self.path)
            .finish()
    }
}

impl Config {
    #[allow(dead_code)]
    async fn load_from_file(path: &PathBuf) -> Result<Config> {
        let config = toml::from_str(&fs::read_to_string(path).await?)?;
        Ok(config)
    }
    async fn save_to_file(&self, path: &PathBuf) -> Result<()> {
        let toml_str = toml::to_string(self)?;
        fs::write(path, toml_str).await?;
        // The file may contain API keys — keep it readable only by the owner.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let perms = std::fs::Permissions::from_mode(0o600);
            fs::set_permissions(path, perms).await?;
        }

        Ok(())
    }
    pub async fn load() -> Result<Config> {
        let config_dir = directory();
        if !config_dir.is_dir() {
            fs::create_dir_all(&config_dir).await?;
        }
        let config_path = config_dir.join("config.toml");
        if std::fs::exists(&config_path)? {
            return Self::load_from_file(&config_path).await;
        }
        let config = Config::default();
        log::info!(
            "config file not found, creating default config at {:?}",
            config_path
        );
        config.save_to_file(&config_path).await?;
        Ok(config)
    }
}

impl Default for ModelConfig {
    fn default() -> Self {
        ModelConfig {
            api_key: std::env::var(GEMINI_API_KEY).unwrap_or("empty".to_owned()),
            model: std::env::var(GEMINI_MODEL).unwrap_or("gemini-2.5-flash".to_owned()),
            provider: "gemini".to_owned(),
            max_tokens: None,
            temperature: None,
            path: None,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            provider: "gemini".to_owned(),
            models: vec![ModelConfig::default()],
        }
    }
}

pub fn directory() -> PathBuf {
    dirs::config_dir()
        .unwrap_or(PathBuf::from("."))
        .join("clai")
}
