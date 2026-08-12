use crate::config::Config;
use anyhow::Result;

pub struct Agent {
}

impl Agent {
    pub async fn send_message(&self, _message: &str) -> Result<String> {
        Ok("".to_string())
    }
}

pub struct AgentBuilder {}

impl AgentBuilder {
    pub fn new() -> Self {
        AgentBuilder {}
    }

    pub fn build(&self, _config: Config) -> Agent {
        Agent {}
    }
}
