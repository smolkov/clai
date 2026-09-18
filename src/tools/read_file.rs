use super::McpTool;
use crate::validator::Validator;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;

pub struct ReadFileTool {
    validator: Validator,
}

impl ReadFileTool {
    pub fn new(validator: Validator) -> Self {
        ReadFileTool { validator }
    }
}

#[async_trait]
impl McpTool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "name": "read_file",
            "description": "Reads the content of a file, relative to the project root.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path of the file to read, relative to the project root"
                    }
                },
                "required": ["path"]
            }
        })
    }

    async fn call(&self, args: serde_json::Value) -> Result<String> {
        let path = args["path"]
            .as_str()
            .ok_or(anyhow::anyhow!("Missing or invalid 'path'"))?;

        let safe_path = self.validator.validate_path(path)?;

        tokio::fs::read_to_string(&safe_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read file {}: {}", safe_path.display(), e))
    }
}
