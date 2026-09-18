use super::McpTool;
use crate::validator::Validator;
use anyhow::{Result, anyhow};
use async_trait::async_trait;
use serde_json::json;
pub struct EditFileTool {
    validator: Validator,
}


impl EditFileTool {
    pub fn new(validator: Validator) -> Self {
        EditFileTool { validator }
    }

    async fn backup(&self, path: &std::path::Path, content: &str) -> Result<()> {
        let backup_path = path.with_extension("bak");
        tokio::fs::write(&backup_path, content)
            .await
            .map_err(|e| anyhow!("Failed to create backup file {}: {}", backup_path.display(), e))?;
        Ok(())
    }
}

#[async_trait]
impl McpTool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "name": "edit_file",
            "description": "Replaces a piece of text in a file with new text. \
                             `old_str` must match the file content EXACTLY, including \
                             whitespace and indentation. If `old_str` appears multiple \
                             times in the file, either (1) include enough surrounding \
                             context lines to make it unique, or (2) use the `occurrence` \
                             parameter to specify which match to replace.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to the file to edit"
                    },
                    "old_str": {
                        "type": "string",
                        "description": "Exact text to be replaced. Must match the file \
                                         content exactly, including whitespace. If this \
                                         text appears more than once in the file, add \
                                         surrounding lines until it is unique, or use \
                                         `occurrence` to pick a specific match."
                    },
                    "new_str": {
                        "type": "string",
                        "description": "New text to replace old_str with"
                    },
                    "occurrence": {
                        "type": "integer",
                        "description": "Which occurrence of old_str to replace, if it \
                                         appears multiple times (1 = first, 2 = second, \
                                         etc.). Use -1 to replace all occurrences. \
                                         Omit if old_str is already unique in the file.",
                        "default": 1
                    }
                },
                "required": ["path", "old_str", "new_str"]
            }
        })
    }

    async fn call(&self, args: serde_json::Value) -> Result<String> {
        let path = args["path"].as_str().ok_or(anyhow::anyhow!("Missing or invalid 'path'"))?;
        let old_str = args["old_str"].as_str().ok_or(anyhow::anyhow!("Missing or invalid 'old_str'"))?;
        let new_str = args["new_str"].as_str().ok_or(anyhow::anyhow!("Missing or invalid 'new_str'"))?;

        let safe_path: std::path::PathBuf = self.validator.validate_path(path)?;
        let content = tokio::fs::read_to_string(&safe_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read file {}: {}", safe_path.display(), e))?;

        // Enforce uniqueness; otherwise the model may accidentally target the wrong location.
        let matches = content.matches(old_str).count();
        match matches {
            0 => return Err(anyhow::anyhow!("old_str '{}' not found in file {}", old_str, path)),
            1 => {}
            n => {
                return Err(anyhow::anyhow!(
                    "old_str '{}' found {} times in file {}. Please make it unique or use the 'occurrence' parameter.",
                    old_str, n, path
                ));
            }
        }

        let new_content = content.replacen(old_str, new_str, 1);

        // Create a backup before writing
        self.backup(&safe_path, &content).await?;

        tokio::fs::write(&safe_path, new_content)
            .await
            .map_err(|e| anyhow!("Failed to write file {}: {}", safe_path.display(), e))?;

        Ok(format!("Datei {path} erfolgreich geändert"))
    }
}
