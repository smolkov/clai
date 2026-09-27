use crate::config::Config;
use crate::provider::Provider;
use crate::session::Session;
use crate::tools::McpTool;
use anyhow::Result;
use crate::message::Message;

pub struct Agent {
    tools: Vec<Box<dyn McpTool>>,
    model: Provider,
    session: Session,
}

impl Agent {
    pub async fn generate(&mut self, _message: &str) -> Result<Message> {
        self.model
            .generate(_message, &self.session, &self.tools)
            .await
    }
    pub async fn status(&self) {
        println!("Agent status: {:?}", self.session);
    }
    pub async fn start(&mut self) -> Result<()> {
        println!("Starting agent with session: {:?}", self.session);
        Ok(())  
    }

}

pub struct AgentBuilder {
    model: Option<Provider>,
    history: Option<Session>,
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

    pub fn history(mut self, history: Session) -> Self {
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

                Provider::new(model_config.clone())?
            }
        };
        let history = match self.history {
            Some(history) => history,
            None => Session::new("default"),
        };
        let tools = self.tools;

        Ok(Agent {
            tools,
            model,
            session: history,
        })
    }
}
