use anyhow::Result;
use async_trait::async_trait;
use serde_json::json;
use std::path::Path;

use super::McpTool;
use crate::validator::{self, Validator};

const IGNORED_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".venv",
    "__pycache__",
    "dist",
    "build",
];
const MAX_ENTRIES: usize = 500; // hard cap gegen riesige Repos

pub struct ListFilesTool {
    validator: Validator,
}

#[async_trait]
impl McpTool for ListFilesTool {
    fn name(&self) -> &str {
        "list_files"
    }
	
    fn schema(&self) -> serde_json::Value {
        json!({
            "name": "list_files",
            "description": "Lists files and directories at the given path. Use this to \
                             explore the project structure before reading or editing files.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path to list, relative to the project root. Use \".\" for the root.",
                        "default": "."
                    },
                    "recursive": {
                        "type": "boolean",
                        "description": "If true, list files in subdirectories too",
                        "default": false
                    },
                    "max_depth": {
                        "type": "integer",
                        "description": "Maximum depth when recursive is true (default 3)",
                        "default": 3
                    }
                },
                "required": []
            }
        })
    }

    async fn call(&self, args: serde_json::Value) -> Result<String> {
        let rel_path = args["path"].as_str().unwrap_or(".");
        let recursive = args["recursive"].as_bool().unwrap_or(false);
        let max_depth = args["max_depth"].as_u64().unwrap_or(3) as usize;

        let safe_path = self.validator.validate_path(rel_path)?;

        let mut entries = Vec::new();
        walk_dir(
            &safe_path,
            &self.validator,
            0,
            recursive,
            max_depth,
            &mut entries,
        )?;

        if entries.is_empty() {
            return Ok("(empty directory)".to_string());
        }

        Ok(entries.join("\n"))
    }
}

impl ListFilesTool {
    pub fn new(validator: Validator) -> Self {
        ListFilesTool { validator }
    }
}

fn walk_dir(
    dir: &Path,
    validator: &Validator,
    depth: usize,
    recursive: bool,
    max_depth: usize,
    entries: &mut Vec<String>,
) -> Result<()> {
    if entries.len() >= MAX_ENTRIES {
        entries.push("... (truncated, too many entries)".into());
        return Ok(());
    }

    let mut read_entries: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| anyhow::anyhow!("Failed to read directory {}: {}", dir.display(), e))?
        .filter_map(|e| e.ok())
        .collect();

    // Sortiert: Verzeichnisse zuerst, dann alphabetisch — bessere Lesbarkeit fürs Model
    read_entries.sort_by_key(|e| {
        (
            !e.path().is_dir(),
            e.file_name().to_string_lossy().to_lowercase(),
        )
    });

    for entry in read_entries {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Versteckte Dateien und Rausch-Ordner überspringen
        if name.starts_with('.') || IGNORED_DIRS.contains(&name.as_str()) {
            continue;
        }

        let rel: std::path::Display<'_> = path.strip_prefix(validator.root()).unwrap_or(&path).display();
        let indent = "  ".repeat(depth);

        if path.is_dir() {
            entries.push(format!("{indent}{name}/"));
            if recursive && depth < max_depth {
                walk_dir(&path, validator, depth + 1, recursive, max_depth, entries)?;
            }
        } else {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            entries.push(format!("{indent}{name} ({} bytes)", size));
        }

        if entries.len() >= MAX_ENTRIES {
            entries.push("... (truncated, too many entries)".into());
            break;
        }
    }

    Ok(())
}
