use super::McpTool;

pub struct ReadFileTool;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;

#[async_trait]
impl McpTool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "name": "read_file",
            "description": "Liest den Inhalt einer Datei",
            "input_schema": {
                "type": "object",
                "properties": { "path": { "type": "string" } },
                "required": ["path"]
            }
        })
    }

    async fn call(&self, args: serde_json::Value) -> Result<String> {
        let path = args["path"]
            .as_str()
            .ok_or(anyhow::anyhow!("Missing path"))?;

        // Whitelist / Pfad-Validierung statt rohem cat!
        // if !is_allowed_path(path) {
        // return Err(anyhow::anyhow!("Forbidden path"));
        // }

        tokio::fs::read_to_string(path)
            .await
            .map_err(|e| anyhow::anyhow!(e.to_string()))
    }
}
