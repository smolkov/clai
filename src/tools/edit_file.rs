use super::McpTool;
use anyhow::Result;

pub struct EditFileTool {
    allowed_root: PathBuf, // Sandbox-Wurzel!
}

#[async_trait]
impl McpTool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "type": "function",
            "function": {
                "name": "edit_file",
                "description": "Replaces a piece of text in a file with new text. \
                                 `old_str` must match the file content EXACTLY, including \
                                 whitespace and indentation. If `old_str` appears multiple \
                                 times in the file, either (1) include enough surrounding \
                                 context lines to make it unique, or (2) use the `occurrence` \
                                 parameter to specify which match to replace.",
                "parameters": {
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
            }
        })
    }

    async fn call(&self, args: Value) -> Result<String> {
        let path = args["path"].as_str().ok_or(ToolError::BadArgs)?;
        let old_str = args["old_str"].as_str().ok_or(ToolError::BadArgs)?;
        let new_str = args["new_str"].as_str().ok_or(ToolError::BadArgs)?;

        let safe_path = self.validate_path(path)?;
        let content = tokio::fs::read_to_string(&safe_path)
            .await
            .map_err(|e| ToolError::Io(e.to_string()))?;

        // Eindeutigkeit erzwingen — sonst kann das Model ungewollt
        // die falsche Stelle treffen
        let matches = content.matches(old_str).count();
        match matches {
            0 => return Err(anyhow::anyhow!("old_str nicht in Datei gefunden".into())),
            1 => {}
            n => {
                return Err(anyhow::anyhow!(format!(
                    "old_str kommt {n}x vor — muss eindeutig sein"
                )))
            }
        }

        let new_content = content.replacen(old_str, new_str, 1);

        // Backup vor dem Schreiben
        self.backup(&safe_path, &content).await?;

        tokio::fs::write(&safe_path, new_content)
            .await
            .map_err(|e| ToolError::Io(e.to_string()))?;

        Ok(format!("Datei {path} erfolgreich geändert"))
    }
}
