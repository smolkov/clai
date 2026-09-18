use crate::workspace::WORKSPACE;

use super::McpTool;
use anyhow::Result;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use serde_json::{json, Value};

const MAX_FILE_SIZE: usize = 1_000_000; // 1 MB
pub struct CreateFileTool {
    allowed_root: PathBuf,
}

impl CreateFileTool {
    pub fn new<T: AsRef<Path>>(path: T) -> Self {
        CreateFileTool {
            allowed_root: path.as_ref().to_path_buf(),
        }
    }
    pub async fn backup(&self, path: &PathBuf, old_content: &str) -> Result<()> {
        let backup_path = path.with_extension("bak");
        tokio::fs::write(&backup_path, old_content)
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "Failed to create backup file {}: {}",
                    backup_path.display(),
                    e
                )
            })?;
        Ok(())
    }
}

#[async_trait]
impl McpTool for CreateFileTool {
    fn name(&self) -> &str {
        "create_file"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "name": "create_file",
            "description": "Creates a new file with the given content. Fails if the \
                             file already exists, unless `overwrite` is set to true. \
                             Automatically creates any missing parent directories.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path of the file to create, relative to the project root"
                    },
                    "content": {
                        "type": "string",
                        "description": "Full content to write to the file"
                    },
                    "overwrite": {
                        "type": "boolean",
                        "description": "If true, overwrite the file if it already exists. \
                                         Default false — use edit_file for existing files instead.",
                        "default": false
                    }
                },
                "required": ["path", "content"]
            }
        })
    }
    async fn call(&self, args: Value) -> Result<String> {
        let path = args["path"]
            .as_str()
            .ok_or(anyhow::anyhow!("Create file tools missing path parameter"))?;
        let content = args["content"].as_str().ok_or(anyhow::anyhow!(
            "Create file tools missing content parameter"
        ))?;
        let overwrite = args["overwrite"].as_bool().unwrap_or(false);

        // let safe_path = self.validate_path(path)?;
        if content.len() > MAX_FILE_SIZE {
            return Err(anyhow::anyhow!(
                "Content zu groß: {} bytes (max {MAX_FILE_SIZE})",
                content.len()
            ));
        }
        let safe_path = WORKSPACE.validate_path(path)?;
        // Backup falls überschrieben wird
        if safe_path.exists() {
            if !overwrite {
                return Err(anyhow::anyhow!(
                    "File {} already exists. Use overwrite=true to replace it.",
                    safe_path.display()
                ));
            }
            let old_content = tokio::fs::read_to_string(&safe_path).await.map_err(|e| {
                anyhow::anyhow!(
                    "Failed to read existing file {}: {}",
                    safe_path.display(),
                    e
                )
            })?;
            self.backup(&safe_path, &old_content).await?;
        }
        // Fehlende Parent-Verzeichnisse anlegen
        if let Some(parent) = safe_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                anyhow::anyhow!(
                    "Failed to create parent directories for {}: {}",
                    safe_path.display(),
                    e
                )
            })?;
        }

        tokio::fs::write(&safe_path, content).await.map_err(|e| {
            anyhow::anyhow!("Failed to write to file {}: {}", safe_path.display(), e)
        })?;

        Ok(format!(
            "File {path} created successfully ({} bytes)",
            content.len()
        ))
    }
}
