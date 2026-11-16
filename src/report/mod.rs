//! The report of `codeagent run --report` (RFC 0027; see `spec/command.md`,
//! "codeagent run"): what the model handed in with the `report` tool, and what
//! `codeagent` saw itself in the events of the run.

use std::collections::HashMap;
use std::path::Path;

use serde::Serialize;
use serde_json::Value;

use crate::agent::{CallState, EndReason, SessionEvent};
use crate::text::{cut, main_argument, marker, short_result, tokens};
use crate::tools::report::{self, Finding, Reported, Status};

/// The shape of the JSON; see "Formats" in the spec.
pub const VERSION: u32 = 1;

/// Longest summary and result on a tool line of the text, in characters.
const LINE_CHARS: usize = 120;

/// The frame of the task; `{task}` is replaced.
const WORK: &str = include_str!("work.md");

/// The message of the turn after a work turn without a report.
pub const ASK_AGAIN: &str = "Call the report tool now with the result of your work on the \
                             task; use no other tools.";

/// The task inside its frame, as the model gets it.
pub fn work_message(task: &str) -> String {
    WORK.replace("{task}", task)
}

/// How the work turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Done,
    Limit,
    Failed,
}

impl Outcome {
    /// `None` for a cancelled turn, which gets no report.
    pub fn of(reason: EndReason) -> Option<Outcome> {
        match reason {
            EndReason::Done => Some(Outcome::Done),
            EndReason::Limit => Some(Outcome::Limit),
            EndReason::Failed => Some(Outcome::Failed),
            EndReason::Cancelled => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Edited,
    Written,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Change {
    pub path: String,
    pub kind: ChangeKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandRun {
    pub command: String,
    /// `None` when it timed out or was killed.
    pub exit_code: Option<i32>,
}

/// One tool call, as its tool line shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolCall {
    pub tool: String,
    pub summary: String,
    pub state: CallState,
    pub result: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Counts {
    pub done: usize,
    pub error: usize,
    pub denied: usize,
    pub cancelled: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    /// The part of the input read from the prompt cache.
    pub cache_read_tokens: u64,
    pub requests: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    pub version: u32,
    pub workspace: String,
    pub model: String,
    pub outcome: Outcome,
    pub status: Status,
    pub summary: String,
    pub findings: Vec<Finding>,
    pub follow_ups: Vec<String>,
    pub changes: Vec<Change>,
    /// Files edited or written in the scratch directory (RFC 0029).
    pub scratch: Vec<Change>,
    pub commands: Vec<CommandRun>,
    pub files_read: Vec<String>,
    pub tool_calls: Vec<ToolCall>,
    pub counts: Counts,
    pub usage: Usage,
    /// Secrets hidden from the model in the task and the tool results
    /// (RFC 0030).
    pub secrets_hidden: usize,
    /// Seconds.
    pub duration: u64,
}

/// A finished call, with what the report needs of it.
#[derive(Debug, Clone)]
struct Seen {
    call: ToolCall,
    arguments: Value,
    /// The full result text.
    text: String,
}

/// Gathers the facts of a run from its events.
#[derive(Debug, Default)]
pub struct Collector {
    /// Summaries of calls, by id, from their earlier events.
    summaries: HashMap<String, String>,
    calls: Vec<Seen>,
    usage: Usage,
    /// The last error event: the summary of a failed run.
    error: Option<String>,
    secrets_hidden: usize,
}

impl Collector {
    pub fn event(&mut self, event: &SessionEvent) {
        match event {
            SessionEvent::UserMessage { text } => {
                self.secrets_hidden += crate::secrets::count(text);
            }
            SessionEvent::ToolCall(call) => {
                if let Some(summary) = &call.summary {
                    self.summaries.insert(call.id.clone(), summary.clone());
                }
                if !call.state.is_final() || call.name == report::NAME {
                    return;
                }
                let text = call
                    .result
                    .as_ref()
                    .map(|r| r.text.clone())
                    .unwrap_or_default();
                self.secrets_hidden += crate::secrets::count(&text);
                let summary = self
                    .summaries
                    .get(&call.id)
                    .cloned()
                    .unwrap_or_else(|| main_argument(&call.arguments));
                self.calls.push(Seen {
                    call: ToolCall {
                        tool: call.name.clone(),
                        summary,
                        state: call.state,
                        result: short_result(&call.name, call.state, &text),
                    },
                    arguments: call.arguments.clone(),
                    text,
                });
            }
            SessionEvent::TurnEnd(end) => {
                self.usage.input_tokens += end.input_tokens;
                self.usage.output_tokens += end.output_tokens;
                self.usage.cache_read_tokens += end.cache_read_tokens;
                self.usage.requests += end.requests;
            }
            SessionEvent::Error { message } => self.error = Some(message.clone()),
            _ => {}
        }
    }

    /// The last error of the run.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// The report: `reported` from the model, or `None` for one with status
    /// `unknown` and `fallback` as its summary.
    pub fn report(&self, run: Run<'_>, reported: Option<&Reported>, fallback: &str) -> Report {
        let (status, summary, findings, follow_ups) = match reported {
            Some(r) => (
                r.status,
                r.summary.clone(),
                r.findings
                    .iter()
                    .map(|f| Finding {
                        file: f.file.as_deref().map(|p| relative(run.workspace, p)),
                        ..f.clone()
                    })
                    .collect(),
                r.follow_ups.clone(),
            ),
            None => (
                Status::Unknown,
                fallback.trim().to_owned(),
                Vec::new(),
                Vec::new(),
            ),
        };
        let path = |seen: &Seen| {
            seen.arguments
                .get("path")
                .and_then(Value::as_str)
                .map(|p| relative(run.workspace, p))
        };
        let mut changes: Vec<Change> = Vec::new();
        let mut scratch: Vec<Change> = Vec::new();
        let mut files_read: Vec<String> = Vec::new();
        let mut commands = Vec::new();
        let mut counts = Counts::default();
        for seen in &self.calls {
            match seen.call.state {
                CallState::Done => counts.done += 1,
                CallState::Error => counts.error += 1,
                CallState::Denied => counts.denied += 1,
                _ => counts.cancelled += 1,
            }
            let done = seen.call.state == CallState::Done;
            match seen.call.tool.as_str() {
                "edit_file" | "write_file" if done => {
                    let kind = if seen.call.tool == "edit_file" {
                        ChangeKind::Edited
                    } else {
                        ChangeKind::Written
                    };
                    let Some(path) = path(seen) else { continue };
                    let list = match run.scratch {
                        Some(dir) if Path::new(&path).starts_with(dir) => &mut scratch,
                        _ => &mut changes,
                    };
                    if !list.iter().any(|c| c.path == path) {
                        list.push(Change { path, kind });
                    }
                }
                "read_file" if done => {
                    if let Some(path) = path(seen)
                        && !files_read.contains(&path)
                    {
                        files_read.push(path);
                    }
                }
                "run_command" if done || seen.call.state == CallState::Error => {
                    if let Some(command) = seen.arguments.get("command").and_then(Value::as_str) {
                        commands.push(CommandRun {
                            command: command.to_owned(),
                            exit_code: exit_code(&seen.text),
                        });
                    }
                }
                _ => {}
            }
        }
        Report {
            version: VERSION,
            workspace: run.workspace.display().to_string(),
            model: run.model.to_owned(),
            outcome: run.outcome,
            status,
            summary,
            findings,
            follow_ups,
            changes,
            scratch,
            commands,
            files_read,
            tool_calls: self.calls.iter().map(|s| s.call.clone()).collect(),
            counts,
            usage: self.usage,
            secrets_hidden: self.secrets_hidden,
            duration: run.duration,
        }
    }
}

/// What the report knows about the run besides its events.
#[derive(Debug, Clone, Copy)]
pub struct Run<'a> {
    pub workspace: &'a Path,
    pub model: &'a str,
    /// The scratch directory of the run, if it has one.
    pub scratch: Option<&'a Path>,
    pub outcome: Outcome,
    /// Seconds.
    pub duration: u64,
}

/// A path relative to the workspace when inside it.
fn relative(workspace: &Path, path: &str) -> String {
    let p = Path::new(path);
    match p.strip_prefix(workspace) {
        Ok(rest) if p.is_absolute() && !rest.as_os_str().is_empty() => rest.display().to_string(),
        _ => path.to_owned(),
    }
}

/// The exit code of a `run_command` result: its last line, `exit code N`.
fn exit_code(text: &str) -> Option<i32> {
    text.lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .and_then(|l| l.trim().strip_prefix("exit code "))
        .and_then(|n| n.parse().ok())
}

/// The report as JSON: one object and a newline.
pub fn render_json(report: &Report) -> String {
    match serde_json::to_string_pretty(report) {
        Ok(text) => format!("{text}\n"),
        // Plain data; serializing cannot fail. Still, never crash.
        Err(e) => format!("{{\"error\": \"{e}\"}}\n"),
    }
}

/// The report as Markdown. `place` is the workspace as shown (`~/ws/x`).
pub fn render_text(report: &Report, place: &str, unicode: bool) -> String {
    let dot = if unicode { "·" } else { "-" };
    let dash = if unicode { "—" } else { "-" };
    let mut out = String::new();
    let status = report.status.text();
    let mut title = status.to_owned();
    if let Some(first) = title.get(..1) {
        title = format!("{}{}", first.to_uppercase(), title.get(1..).unwrap_or(""));
    }
    out.push_str(&format!("# {title}\n\n"));
    out.push_str(&format!(
        "{place} {dot} {} {dot} {} {dot} {} in, {} out\n",
        report.model,
        duration_text(report.duration),
        crate::text::input_tokens(report.usage.input_tokens, report.usage.cache_read_tokens),
        tokens(report.usage.output_tokens),
    ));
    match report.outcome {
        Outcome::Done => {}
        Outcome::Limit => out.push_str("\nThe run stopped at its limit.\n"),
        Outcome::Failed => out.push_str("\nThe run failed.\n"),
    }
    match report.secrets_hidden {
        0 => {}
        1 => out.push_str("\n1 secret was hidden from the model.\n"),
        n => out.push_str(&format!("\n{n} secrets were hidden from the model.\n")),
    }
    if !report.summary.is_empty() {
        out.push_str(&format!("\n{}\n", report.summary));
    }
    if !report.findings.is_empty() {
        out.push_str("\n## Findings\n\n");
        for f in &report.findings {
            out.push_str(&format!("- **{}** {}", f.severity.name(), f.title));
            if let Some(file) = &f.file {
                match f.line {
                    Some(line) => out.push_str(&format!(" {dash} `{file}:{line}`")),
                    None => out.push_str(&format!(" {dash} `{file}`")),
                }
            }
            out.push('\n');
            if let Some(detail) = &f.detail {
                for line in detail.lines() {
                    out.push_str(&format!("  {line}\n"));
                }
            }
        }
    }
    if !report.changes.is_empty() {
        out.push_str("\n## Changes\n\n");
        for c in &report.changes {
            let kind = match c.kind {
                ChangeKind::Edited => "edited",
                ChangeKind::Written => "written",
            };
            out.push_str(&format!("- {kind} `{}`\n", c.path));
        }
    }
    if !report.scratch.is_empty() {
        out.push_str("\n## Scratch files\n\n");
        for c in &report.scratch {
            let kind = match c.kind {
                ChangeKind::Edited => "edited",
                ChangeKind::Written => "written",
            };
            out.push_str(&format!("- {kind} `{}`\n", c.path));
        }
    }
    if !report.follow_ups.is_empty() {
        out.push_str("\n## Follow-ups\n\n");
        for item in &report.follow_ups {
            out.push_str(&format!("- {item}\n"));
        }
    }
    if !report.tool_calls.is_empty() {
        out.push_str("\n## Tool calls\n\n");
        for call in &report.tool_calls {
            out.push_str(&format!("- {} {}", marker(call.state, unicode), call.tool));
            if !call.summary.is_empty() {
                out.push_str(&format!(" {}", cut(&call.summary, LINE_CHARS, unicode)));
            }
            if !call.result.is_empty() {
                out.push_str(&format!(
                    " {dash} {}",
                    cut(&call.result, LINE_CHARS, unicode)
                ));
            }
            out.push('\n');
        }
    }
    out
}

/// `12 s`, `3 min`.
fn duration_text(seconds: u64) -> String {
    if seconds < 60 {
        format!("{seconds} s")
    } else {
        format!("{} min", seconds.div_ceil(60))
    }
}

#[cfg(test)]
mod tests;
