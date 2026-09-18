use crate::config::Config;
use crate::history::{self, History};
use crate::model::Model;
use crate::tools::McpTool;
use anyhow::Result;

pub struct Agent {
    tools: Vec<Box<dyn McpTool>>,
    model: Model,
    history: History,
}

impl Agent {
    pub async fn generate(&mut self, _message: &str) -> Result<String> {
        self.model
            .generate(_message, &self.history, &self.tools)
            .await
    }
}

pub struct AgentBuilder {
    model: Option<Model>,
    history: Option<History>,
    tools: Vec<Box<dyn McpTool>>,
}

impl AgentBuilder {
    pub fn new() -> Self {
        AgentBuilder {
            model: None,
            history: None,
            tools: Vec::new(),
        }
    }
    pub fn tool(mut self, tool: Box<dyn McpTool>) -> Self {
        self.tools.push(tool);
        self
    }

    pub fn tools(mut self, tools: impl IntoIterator<Item = Box<dyn McpTool>>) -> Self {
        self.tools.extend(tools);
        self
    }

    pub fn history(mut self, history: History) -> Self {
        self.history = Some(history);
        self
    }

    pub fn build(self, config: Config) -> Result<Agent> {
        let model = match self.model {
            Some(model) => model,
            None => {
                let model_config = config
                    .models
                    .iter()
                    .find(|m| m.name == config.model)
                    .or_else(|| config.models.first())
                    .ok_or(anyhow::anyhow!("no model config found"))?;

                Model::new(model_config.clone())?
            }
        };
        let history = match self.history {
            Some(history) => history,
            None => History::new("default"),
        };
        let tools = self.tools;

        Ok(Agent {
            tools,
            model,
            history,
        })
    }
}
