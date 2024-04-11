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
}
