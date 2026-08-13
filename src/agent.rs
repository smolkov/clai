use crate::config::Config;
use crate::tools::McpTool;
use anyhow::Result;

pub struct Agent {
    tools: Vec<Box<dyn McpTool>>,
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
        Agent { tools: Vec::new() }
    }
}
