use std::path::PathBuf;

use anyhow::Result;
use reqwest::header::{HeaderMap, AUTHORIZATION, CONTENT_TYPE};
use serde::{Deserialize, Serialize};
use tokio::fs;

pub const OPENAI_API_KEY: &str = "OPENAI_API_KEY";
pub const OPENAI_MODEL: &str = "OPENAI_MODEL";

#[derive(Serialize, Deserialize, Debug)]
pub struct Config {
    pub api_key: String,
    pub model: String,
}

impl Config {
    #[allow(dead_code)]
    pub fn check(&self) -> Result<()> {
        if self.api_key == "empty" {
            return Err(anyhow::anyhow!(
                "please set api key in config file or in 'OPENAI_API_KEY` environment variable"
            ));
        }
        Ok(())
    }
    #[allow(dead_code)]
    pub async fn load() -> Result<Config> {
        let config_dir = directory();
        if !config_dir.is_dir() {
            fs::create_dir_all(&config_dir).await?;
        }
        let path = config_dir.join("config.toml");
        let config = toml::from_str(&fs::read_to_string(path).await?)?;
        Ok(config)
    }

    pub fn header(&self) -> Result<HeaderMap> {
        let mut header = HeaderMap::new();
        header.insert(AUTHORIZATION, format!("Bearer {}", self.api_key).parse()?);
        header.insert(CONTENT_TYPE, "application/json".parse()?);
        Ok(header)
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            api_key: std::env::var(OPENAI_API_KEY).unwrap_or("empty".to_owned()),
            model: std::env::var(OPENAI_MODEL).unwrap_or("gpt-4o-mini".to_owned()),
        }
    }
}


pub fn directory() -> PathBuf {
    dirs::config_dir()
        .unwrap_or(PathBuf::from("."))
        .join("clai")
}