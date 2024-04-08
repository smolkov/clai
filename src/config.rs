use std::path::PathBuf;

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub const OPENAI_API_KEY: &str = "OPENAI_API_KEY";

#[derive(Serialize, Deserialize, Debug)]
struct Config {
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
    pub fn load() -> Result<Config> {
        let path = dirs::config_dir().unwrap_or(PathBuf::from(".")).join("clai/clai.toml");
        let config =serde_json::from_str(&std::fs::read_to_string(path)?)?;
        Ok(config)
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            api_key: std::env::var(OPENAI_API_KEY).unwrap_or("empty".to_owned()),
            model: "gpt-3.5-turbo".to_owned(),
        }
    }
}


impl Config {

}