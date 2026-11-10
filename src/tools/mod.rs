//! The tools the model may call (see `spec/tools.md`).
//!
//! A call goes through [`prepare`]: its arguments are checked, its paths
//! resolved, its permission decided and the details for a question made.
//! The caller then asks the user if the decision is `ask`, and runs it with
//! [`PreparedCall::run`]. Nothing here knows sessions or clients.

pub mod ask_user;
mod edit_file;
mod list_files;
mod output;
mod question;
mod read_file;
mod run_command;
mod search_files;
mod text;
mod walk;
mod write_file;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::config::{Action, Permissions, Rule, Tool};
use crate::permissions::{Checker, Decision, Proposal, SessionRules, Workspace};
use crate::provider::ToolSpec;

pub use ask_user::{AskOption, AskQuestion, UserAnswer};
pub use output::{FULL_BYTES, ToolOutput, cut_for_model, group};
pub use question::{Details, PathDetails, Text};

/// Receives the output of a running command as it arrives, at most every
/// 100 ms.
pub type Progress = tokio::sync::mpsc::UnboundedSender<String>;

/// What a tool call is checked and run with: a snapshot of the session's
/// permissions. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Env {
    pub permissions: Arc<Permissions>,
    pub session: Arc<SessionRules>,
    pub workspace: Arc<Workspace>,
}

impl Env {
    pub fn checker(&self) -> Checker<'_> {
        Checker {
            permissions: &self.permissions,
            session: &self.session,
            workspace: &self.workspace,
        }
    }
}

/// The result of preparing a call.
#[derive(Debug)]
pub enum Prepared {
    /// Allowed, or needs the user's answer: see [`PreparedCall::action`].
    Ready(Box<PreparedCall>),
    /// Denied by the boundary, a rule or a default; the output is the
    /// error result for the model.
    Denied(ToolOutput),
    /// Wrong arguments or a problem found before running (file not found,
    /// `old_text` not found, …); the output is the error result.
    Failed(ToolOutput),
}

/// A call that may run.
#[derive(Debug)]
pub struct PreparedCall {
    pub tool: Tool,
    /// One line: `src/parse.rs (lines 12–14)`, the command, …
    pub summary: String,
    pub decision: Decision,
    pub details: Details,
    /// Rules for "always", narrowest first.
    pub proposals: Vec<Proposal>,
    op: Op,
}

#[derive(Debug)]
enum Op {
    Read(read_file::Op),
    List(list_files::Op),
    Search(search_files::Op),
    Edit(edit_file::Op),
    Write(write_file::Op),
    Run(run_command::Op),
}

impl PreparedCall {
    pub fn action(&self) -> Action {
        self.decision.action
    }

    /// Runs the call. Cancelling `cancel` stops it as soon as possible.
    pub async fn run(self, env: &Env, cancel: CancellationToken) -> ToolOutput {
        self.run_with_progress(env, cancel, None).await
    }

    /// As [`PreparedCall::run`]; `run_command` also sends its output to
    /// `progress` while it runs.
    pub async fn run_with_progress(
        self,
        env: &Env,
        cancel: CancellationToken,
        progress: Option<Progress>,
    ) -> ToolOutput {
        match self.op {
            Op::Read(op) => blocking(move || op.run()).await,
            Op::List(op) => {
                let env = env.clone();
                blocking(move || op.run(&env, &cancel)).await
            }
            Op::Search(op) => {
                let env = env.clone();
                blocking(move || op.run(&env, &cancel)).await
            }
            Op::Edit(op) => blocking(move || op.run()).await,
            Op::Write(op) => blocking(move || op.run()).await,
            Op::Run(op) => op.run(env, cancel, progress).await,
        }
    }

    /// Checks the rules the user chose for "always": each must be a valid
    /// rule for this tool (or a redirection of the command), and together
    /// they must allow this call. Returns the rules to add to the session.
    pub fn accept_rules(&self, env: &Env, texts: &[String]) -> Result<Vec<Rule>, String> {
        if texts.is_empty() {
            return Err("no rule given".into());
        }
        let home = env.workspace.home();
        let mut rules = Vec::new();
        for text in texts {
            let rule = Rule::parse_session(text, home)?;
            let fits = rule.tools.contains(&self.tool)
                || (self.tool == Tool::RunCommand
                    && rule
                        .tools
                        .iter()
                        .any(|t| matches!(t, Tool::WriteFile | Tool::ReadFile)));
            if !fits {
                return Err(format!("rule `{text}` is not for {}", self.tool));
            }
            rules.push(rule);
        }
        let mut session = (*env.session).clone();
        session.add(rules.iter().cloned());
        let trial = Env {
            session: Arc::new(session),
            ..env.clone()
        };
        if self.recheck(&trial) == Action::Allow {
            Ok(rules)
        } else {
            Err("the rules do not allow this call".into())
        }
    }

    fn recheck(&self, env: &Env) -> Action {
        let checker = env.checker();
        match &self.op {
            Op::Read(op) => checker.check_path(Tool::ReadFile, &op.path).action,
            Op::List(op) => checker.check_path(Tool::ListFiles, &op.path).action,
            Op::Search(op) => checker.check_path(Tool::SearchFiles, &op.path).action,
            Op::Edit(op) => checker.check_path(Tool::EditFile, &op.path).action,
            Op::Write(op) => checker.check_path(Tool::WriteFile, &op.path).action,
            Op::Run(op) => checker.check_command(&op.command).decision.action,
        }
    }
}

/// Checks a call and makes everything needed to ask about it and run it.
pub fn prepare(env: &Env, name: &str, arguments: &Value) -> Prepared {
    let Some(tool) = Tool::from_name(name) else {
        return Prepared::Failed(ToolOutput::error(format!(
            "unknown tool `{name}`; tools are {}",
            Tool::ALL.map(Tool::name).join(", ")
        )));
    };
    let result = match tool {
        Tool::ReadFile => read_file::prepare(env, args(tool, arguments)),
        Tool::ListFiles => list_files::prepare(env, args(tool, arguments)),
        Tool::SearchFiles => search_files::prepare(env, args(tool, arguments)),
        Tool::EditFile => edit_file::prepare(env, args(tool, arguments)),
        Tool::WriteFile => write_file::prepare(env, args(tool, arguments)),
        Tool::RunCommand => run_command::prepare(env, args(tool, arguments)),
    };
    match result {
        Ok(call) if call.decision.action == Action::Deny => Prepared::Denied(ToolOutput::error(
            call.denial.unwrap_or_else(|| call.decision.denial()),
        )),
        Ok(call) => Prepared::Ready(Box::new(PreparedCall {
            tool,
            summary: call.summary,
            decision: call.decision,
            details: call.details,
            proposals: call.proposals,
            op: call.op,
        })),
        Err(Failure::Denied(text)) => Prepared::Denied(ToolOutput::error(text)),
        Err(Failure::Error(text)) => Prepared::Failed(ToolOutput::error(text)),
    }
}

/// What a tool's `prepare` returns.
struct Call {
    summary: String,
    decision: Decision,
    /// A denial text other than the decision's own (commands name the part).
    denial: Option<String>,
    details: Details,
    proposals: Vec<Proposal>,
    op: Op,
}

enum Failure {
    /// Denied before the details could be made.
    Denied(String),
    Error(String),
}

impl From<String> for Failure {
    fn from(text: String) -> Failure {
        Failure::Error(text)
    }
}

/// Parses the arguments of a tool.
fn args<T: DeserializeOwned>(tool: Tool, arguments: &Value) -> Result<T, Failure> {
    if let Value::String(raw) = arguments {
        return Err(Failure::Error(format!(
            "the arguments of {tool} are not valid JSON: {}",
            cut_for_model(raw)
        )));
    }
    serde_json::from_value(arguments.clone())
        .map_err(|e| Failure::Error(format!("invalid arguments for {tool}: {e}")))
}

/// Runs blocking file work off the async threads.
async fn blocking(f: impl FnOnce() -> ToolOutput + Send + 'static) -> ToolOutput {
    tokio::task::spawn_blocking(f)
        .await
        .unwrap_or_else(|e| ToolOutput::error(format!("the tool failed: {e}")))
}

/// The tool specs sent to the model, the same for every provider.
pub fn specs() -> Vec<ToolSpec> {
    let spec = |name: &str, description: &str, input_schema: Value| ToolSpec {
        name: name.into(),
        description: description.into(),
        input_schema,
    };
    vec![
        spec(
            "read_file",
            "Read lines of a text file. Each line is shown with its number. Use offset and \
             limit to read a long file in parts.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "File to read, relative to the workspace or absolute."},
                    "offset": {"type": "integer", "minimum": 1, "description": "First line, counted from 1. Default 1."},
                    "limit": {"type": "integer", "minimum": 1, "maximum": 2000, "description": "Most lines to return. Default 2000."}
                },
                "required": ["path"],
                "additionalProperties": false
            }),
        ),
        spec(
            "list_files",
            "List files and directories that match a glob, respecting .gitignore. Directories \
             end with /.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "Directory to start in. Default: the workspace."},
                    "pattern": {"type": "string", "description": "Glob relative to path: * within one directory, ** across directories, ? one character, {a,b} alternatives. Default *."}
                },
                "additionalProperties": false
            }),
        ),
        spec(
            "search_files",
            "Search the contents of text files for a regular expression, respecting \
             .gitignore. Returns path:line: text for each matching line.",
            json!({
                "type": "object",
                "properties": {
                    "pattern": {"type": "string", "description": "Regular expression (Rust regex syntax), matched per line."},
                    "path": {"type": "string", "description": "File or directory to search. Default: the workspace."},
                    "glob": {"type": "string", "description": "Only files whose name or path below path matches this glob."},
                    "ignore_case": {"type": "boolean", "description": "Case-insensitive search. Default false."}
                },
                "required": ["pattern"],
                "additionalProperties": false
            }),
        ),
        spec(
            "edit_file",
            "Replace exact text in an existing file. old_text must match exactly, including \
             indentation, and occur once unless replace_all is set. Line numbers from \
             read_file are not part of the text.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "File to change."},
                    "old_text": {"type": "string", "description": "Text to replace, exactly as in the file."},
                    "new_text": {"type": "string", "description": "Replacement; may be empty to delete old_text."},
                    "replace_all": {"type": "boolean", "description": "Replace every occurrence. Default false."}
                },
                "required": ["path", "old_text", "new_text"],
                "additionalProperties": false
            }),
        ),
        spec(
            "write_file",
            "Create a file, or replace one completely with overwrite. Missing parent \
             directories are created. To change part of a file, use edit_file.",
            json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "File to write."},
                    "content": {"type": "string", "description": "The whole new content."},
                    "overwrite": {"type": "boolean", "description": "Allow replacing an existing file. Default false."}
                },
                "required": ["path", "content"],
                "additionalProperties": false
            }),
        ),
        spec(
            "run_command",
            "Run a shell command with sh -c in the workspace and return its output and exit \
             code. Standard input is empty; each call starts in the workspace again.",
            json!({
                "type": "object",
                "properties": {
                    "command": {"type": "string", "description": "Command line."},
                    "timeout_s": {"type": "integer", "minimum": 1, "maximum": 600, "description": "Seconds before the command is stopped. Default 120."}
                },
                "required": ["command"],
                "additionalProperties": false
            }),
        ),
    ]
}

/// A range check for an optional integer argument.
fn in_range(
    name: &str,
    value: Option<u64>,
    min: u64,
    max: u64,
    default: u64,
) -> Result<u64, Failure> {
    let value = value.unwrap_or(default);
    if (min..=max).contains(&value) {
        Ok(value)
    } else {
        Err(Failure::Error(format!(
            "{name} must be {min}–{max}, not {value}"
        )))
    }
}

/// The text for a cancelled call, with the output so far.
fn cancelled(so_far: &str) -> ToolOutput {
    if so_far.is_empty() {
        ToolOutput::error("cancelled by the user")
    } else {
        ToolOutput::error(format!("{so_far}\ncancelled by the user"))
    }
}
