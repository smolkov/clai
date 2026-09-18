pub mod read_file;
pub mod create_file;
pub mod edit_file;
pub mod list_files;

use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait McpTool: Send + Sync {
    fn name(&self) -> &str;
    fn schema(&self) -> serde_json::Value;
    async fn call(&self, args: serde_json::Value) -> Result<String>;
}



pub struct McpTools {
    tools: Vec<Box<dyn McpTool>>,
}

impl McpTools {
    pub fn new(tools: Vec<Box<dyn McpTool>>) -> Self {
        McpTools { tools }
    }
}