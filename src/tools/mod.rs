pub mod read_file;
pub mod create_file;

use anyhow::Result;
use async_trait::async_trait;

#[async_trait]
pub trait McpTool: Send + Sync {
    fn name(&self) -> &str;
    fn schema(&self) -> serde_json::Value;
    async fn call(&self, args: serde_json::Value) -> Result<String>;
}
