use super::McpTool;
use anyhow::{anyhow, Result};
use async_trait::async_trait;
use serde_json::json;
use std::path::PathBuf;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_TIMEOUT_SECS: u64 = 120;
const MAX_OUTPUT_BYTES: usize = 20_000;

pub struct ExecuteCommandTool {
    cwd: PathBuf,
    require_confirmation: bool,
}

impl ExecuteCommandTool {
    pub fn new(cwd: PathBuf) -> Self {
        ExecuteCommandTool {
            cwd,
            require_confirmation: true,
        }
    }

    #[cfg(test)]
    fn new_unconfirmed(cwd: PathBuf) -> Self {
        ExecuteCommandTool {
            cwd,
            require_confirmation: false,
        }
    }

    /// Asks the user on stdin/stderr whether `command` may run. Anything other than an
    /// explicit y/yes (including EOF, e.g. a non-interactive session) is treated as "no".
    async fn confirm(&self, command: &str) -> Result<bool> {
        eprint!(
            "execute_command wants to run in {}:\n  {command}\nAllow? [y/N] ",
            self.cwd.display()
        );
        Self::confirm_from(BufReader::new(tokio::io::stdin())).await
    }

    async fn confirm_from<R: tokio::io::AsyncRead + Unpin>(mut reader: BufReader<R>) -> Result<bool> {
        let mut line = String::new();
        reader
            .read_line(&mut line)
            .await
            .map_err(|e| anyhow!("Failed to read confirmation: {e}"))?;
        let answer = line.trim().to_lowercase();
        Ok(answer == "y" || answer == "yes")
    }
}

fn truncate(output: &str) -> String {
    if output.len() <= MAX_OUTPUT_BYTES {
        return output.to_string();
    }
    let mut end = MAX_OUTPUT_BYTES;
    while !output.is_char_boundary(end) {
        end -= 1;
    }
    format!(
        "{}\n... (truncated, {} bytes total)",
        &output[..end],
        output.len()
    )
}

#[async_trait]
impl McpTool for ExecuteCommandTool {
    fn name(&self) -> &str {
        "execute_command"
    }

    fn schema(&self) -> serde_json::Value {
        json!({
            "name": "execute_command",
            "description": "Executes a shell command on Linux, inside the project's workspace \
                             root, and returns its exit code, stdout and stderr. Use this for \
                             non-interactive commands only (builds, tests, git, file inspection); \
                             it has no way to answer interactive prompts. The command runs with \
                             the same privileges as this process and is NOT sandboxed beyond its \
                             working directory, so avoid destructive or irreversible commands \
                             unless explicitly asked to run them.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "description": "The shell command to run, e.g. \"cargo test\" or \"git status\""
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "Maximum time to let the command run before it is killed \
                                         (default 30, max 120)",
                        "default": DEFAULT_TIMEOUT_SECS
                    }
                },
                "required": ["command"]
            }
        })
    }

    async fn call(&self, args: serde_json::Value) -> Result<String> {
        let command = args["command"]
            .as_str()
            .ok_or(anyhow!("Missing or invalid 'command'"))?;
        if command.trim().is_empty() {
            return Err(anyhow!("'command' must not be empty"));
        }

        let timeout_secs = args["timeout_secs"]
            .as_u64()
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .clamp(1, MAX_TIMEOUT_SECS);

        if self.require_confirmation && !self.confirm(command).await? {
            return Err(anyhow!("Command execution declined by user: {command}"));
        }

        let mut child = Command::new("sh")
            .arg("-c")
            .arg(command)
            .current_dir(&self.cwd)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .map_err(|e| anyhow!("Failed to spawn command: {e}"))?;

        let output = match tokio::time::timeout(Duration::from_secs(timeout_secs), child.wait_with_output()).await {
            Ok(result) => result.map_err(|e| anyhow!("Failed to run command: {e}"))?,
            Err(_) => {
                return Err(anyhow!(
                    "Command timed out after {timeout_secs}s: {command}"
                ));
            }
        };

        let stdout = truncate(&String::from_utf8_lossy(&output.stdout));
        let stderr = truncate(&String::from_utf8_lossy(&output.stderr));
        let exit_code = output.status.code().unwrap_or(-1);

        Ok(format!(
            "exit code: {exit_code}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> ExecuteCommandTool {
        ExecuteCommandTool::new_unconfirmed(std::env::temp_dir())
    }

    #[tokio::test]
    async fn confirm_accepts_y_and_yes() {
        for input in ["y\n", "Y\n", "yes\n", "YES\n"] {
            let reader = BufReader::new(input.as_bytes());
            assert!(ExecuteCommandTool::confirm_from(reader).await.unwrap());
        }
    }

    #[tokio::test]
    async fn confirm_rejects_anything_else_including_eof() {
        for input in ["n\n", "no\n", "\n", ""] {
            let reader = BufReader::new(input.as_bytes());
            assert!(!ExecuteCommandTool::confirm_from(reader).await.unwrap());
        }
    }

    #[tokio::test]
    async fn captures_stdout_and_exit_code() {
        let result = tool()
            .call(json!({"command": "echo hi"}))
            .await
            .unwrap();
        assert!(result.contains("exit code: 0"));
        assert!(result.contains("hi"));
    }

    #[tokio::test]
    async fn captures_nonzero_exit_code_and_stderr() {
        let result = tool()
            .call(json!({"command": "echo oops 1>&2; exit 3"}))
            .await
            .unwrap();
        assert!(result.contains("exit code: 3"));
        assert!(result.contains("oops"));
    }

    #[tokio::test]
    async fn runs_in_the_given_cwd() {
        let dir = std::env::temp_dir();
        let result = tool().call(json!({"command": "pwd"})).await.unwrap();
        assert!(result.contains(&dir.canonicalize().unwrap().display().to_string()));
    }

    #[tokio::test]
    async fn empty_command_is_rejected() {
        let err = tool().call(json!({"command": "   "})).await.unwrap_err();
        assert!(err.to_string().contains("must not be empty"));
    }

    #[tokio::test]
    async fn times_out_long_running_commands() {
        let err = tool()
            .call(json!({"command": "sleep 5", "timeout_secs": 1}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("timed out"));
    }

    #[test]
    fn truncates_large_output() {
        let big = "x".repeat(MAX_OUTPUT_BYTES + 100);
        let result = truncate(&big);
        assert!(result.len() < big.len());
        assert!(result.contains("truncated"));
    }
}
