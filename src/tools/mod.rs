pub mod create_file;
pub mod edit_file;
pub mod execute_command;
pub mod list_files;
pub mod read_file;

use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait McpTool: Send + Sync {
    fn name(&self) -> &str;
    fn schema(&self) -> serde_json::Value;
    async fn call(&self, args: serde_json::Value) -> Result<String>;
}

pub struct McpTools {
    _tools: Vec<Box<dyn McpTool>>,
}

impl McpTools {
    pub fn new(_tools: Vec<Box<dyn McpTool>>) -> Self {
        McpTools { _tools }
    }
}
