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
        let occurrence = args.get("occurrence").and_then(|v| v.as_i64());

        if old_str.is_empty() {
            return Err(anyhow!("'old_str' must not be empty"));
        }

        let safe_path: std::path::PathBuf = self.validator.validate_path(path)?;
        let content = tokio::fs::read_to_string(&safe_path)
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read file {}: {}", safe_path.display(), e))?;

        let match_count = content.matches(old_str).count();
        if match_count == 0 {
            return Err(anyhow!("old_str '{}' not found in file {}", old_str, path));
        }

        let (new_content, replacements) = match occurrence {
            None => {
                // No occurrence given — old_str must already be unambiguous.
                if match_count > 1 {
                    return Err(anyhow!(
                        "old_str '{}' found {} times in file {}. Add surrounding context to make \
                         it unique, or set 'occurrence' (1-{}), or -1 to replace all.",
                        old_str, match_count, path, match_count
                    ));
                }
                (content.replacen(old_str, new_str, 1), 1)
            }
            Some(-1) => (content.replace(old_str, new_str), match_count),
            Some(n) if n >= 1 => {
                let n = n as usize;
                if n > match_count {
                    return Err(anyhow!(
                        "old_str '{}' found {} time(s) in file {}, but occurrence {} was requested",
                        old_str, match_count, path, n
                    ));
                }
                (replace_nth(&content, old_str, new_str, n), 1)
            }
            Some(n) => {
                return Err(anyhow!(
                    "Invalid 'occurrence' value {n}: must be >= 1, or -1 to replace all occurrences"
                ));
            }
        };

        // Create a backup before writing
        self.backup(&safe_path, &content).await?;

        tokio::fs::write(&safe_path, new_content)
            .await
            .map_err(|e| anyhow!("Failed to write file {}: {}", safe_path.display(), e))?;

        Ok(format!(
            "File {path} updated successfully ({replacements} replacement{})",
            if replacements == 1 { "" } else { "s" }
        ))
    }
}

/// Replaces only the `n`-th (1-indexed) occurrence of `old` in `content` with `new`.
fn replace_nth(content: &str, old: &str, new: &str, n: usize) -> String {
    let Some((start, _)) = content.match_indices(old).nth(n - 1) else {
        return content.to_string();
    };
    let end = start + old.len();
    let mut result = String::with_capacity(content.len());
    result.push_str(&content[..start]);
    result.push_str(new);
    result.push_str(&content[end..]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn replace_nth_picks_the_right_occurrence() {
        assert_eq!(replace_nth("a a a", "a", "X", 2), "a X a");
        assert_eq!(replace_nth("a a a", "a", "X", 1), "X a a");
        assert_eq!(replace_nth("a a a", "a", "X", 3), "a a X");
    }

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    async fn tool_in_temp_file(content: &str) -> (EditFileTool, std::path::PathBuf, String) {
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("clai_edit_file_test_{}_{id}", std::process::id()));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let file_name = "file.txt";
        tokio::fs::write(dir.join(file_name), content).await.unwrap();
        let validator = Validator::new(&dir);
        (EditFileTool::new(validator), dir, file_name.to_string())
    }

    #[tokio::test]
    async fn replaces_unique_match_without_occurrence() {
        let (tool, dir, file) = tool_in_temp_file("hello world").await;
        tool.call(json!({"path": file, "old_str": "world", "new_str": "there"}))
            .await
            .unwrap();
        let result = tokio::fs::read_to_string(dir.join(&file)).await.unwrap();
        assert_eq!(result, "hello there");
    }

    #[tokio::test]
    async fn errors_on_ambiguous_match_without_occurrence() {
        let (tool, _dir, file) = tool_in_temp_file("a a a").await;
        let err = tool
            .call(json!({"path": file, "old_str": "a", "new_str": "b"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("found 3 times"));
    }

    #[tokio::test]
    async fn occurrence_selects_specific_match() {
        let (tool, dir, file) = tool_in_temp_file("a a a").await;
        tool.call(json!({"path": file, "old_str": "a", "new_str": "b", "occurrence": 2}))
            .await
            .unwrap();
        let result = tokio::fs::read_to_string(dir.join(&file)).await.unwrap();
        assert_eq!(result, "a b a");
    }

    #[tokio::test]
    async fn occurrence_minus_one_replaces_all() {
        let (tool, dir, file) = tool_in_temp_file("a a a").await;
        tool.call(json!({"path": file, "old_str": "a", "new_str": "b", "occurrence": -1}))
            .await
            .unwrap();
        let result = tokio::fs::read_to_string(dir.join(&file)).await.unwrap();
        assert_eq!(result, "b b b");
    }

    #[tokio::test]
    async fn occurrence_out_of_range_errors() {
        let (tool, _dir, file) = tool_in_temp_file("a a").await;
        let err = tool
            .call(json!({"path": file, "old_str": "a", "new_str": "b", "occurrence": 5}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("but occurrence 5 was requested"));
    }

    #[tokio::test]
    async fn empty_old_str_is_rejected() {
        let (tool, _dir, file) = tool_in_temp_file("hello").await;
        let err = tool
            .call(json!({"path": file, "old_str": "", "new_str": "x"}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }
}
