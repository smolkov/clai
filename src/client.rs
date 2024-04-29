use std::time::Duration;

use anyhow::Result;
use serde::{de::DeserializeOwned, Serialize};
// use reqwest::Request;

use crate::config::Config;

pub struct Client {
    pub config: Config,
    pub client: reqwest::Client,
}

impl Client {
    pub fn new(config: Config) -> Client {
        let client = reqwest::Client::new();
        Client { config, client }
    }

    pub async fn post<T, O>(&self, path: &str, request: &T) -> Result<O>
    where
        T: Serialize,
        O: DeserializeOwned,
    {
        let output = self
            .client
            .post(&format!("https://api.openai.com/v1/{}", path))
            .timeout(Duration::from_secs(40))
            .headers(self.config.header()?)
            .json(request)
            .send()
            .await?
            .json()
            .await?;
        Ok(output)
    }
}
