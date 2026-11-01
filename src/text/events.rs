//! Session events as lines on standard error, and the final answer.

use std::collections::HashMap;

use serde_json::Value;

use crate::agent::{CallState, EndReason, SessionEvent, ToolCallEvent};
use crate::tools::group;

use super::{cut, marker};

/// Longest short result on a tool line, in characters.
const RESULT_CHARS: usize = 120;

/// Turns events into text. Keeps the text of the current answer until it is
/// known whether tool calls follow it.
#[derive(Debug, Default)]
pub struct Renderer {
    unicode: bool,
    /// Text of the current answer not written yet.
    pending: String,
    /// Summaries of calls, by id, from their earlier events.
    summaries: HashMap<String, String>,
    /// Set when the turn ended with `done`: the final answer.
    answer: Option<String>,
}

impl Renderer {
    pub fn new(unicode: bool) -> Renderer {
        Renderer {
            unicode,
            ..Renderer::default()
        }
    }

    /// The text for standard error that this event adds, if any.
    pub fn event(&mut self, event: &SessionEvent) -> String {
        match event {
            SessionEvent::TextDone { text } => {
                if !self.pending.is_empty() {
                    self.pending.push('\n');
                }
                self.pending.push_str(text);
                String::new()
            }
            SessionEvent::ToolCall(call) => {
                // Text before a tool call is not the final answer.
                let mut out = self.flush();
                if let Some(summary) = &call.summary {
                    self.summaries.insert(call.id.clone(), summary.clone());
                }
                if call.state.is_final() {
                    out.push_str(&self.call_line(call));
                    out.push('\n');
                }
                out
            }
            SessionEvent::Notice { text } => format!("clai: {text}\n"),
            SessionEvent::Error { message } => format!("clai: error: {message}\n"),
            SessionEvent::TurnEnd(end) => {
                if end.reason == EndReason::Done {
                    self.answer = Some(std::mem::take(&mut self.pending));
                    String::new()
                } else {
                    self.flush()
                }
            }
            _ => String::new(),
        }
    }

    /// The final answer, once the turn ended with `done`.
    pub fn answer(&self) -> Option<&str> {
        self.answer.as_deref()
    }

    fn flush(&mut self) -> String {
        if self.pending.is_empty() {
            return String::new();
        }
        let mut text = std::mem::take(&mut self.pending);
        if !text.ends_with('\n') {
            text.push('\n');
        }
        text
    }

    /// `✓ read_file src/main.rs (lines 1–40) — 40 lines`
    fn call_line(&self, call: &ToolCallEvent) -> String {
        let summary = self
            .summaries
            .get(&call.id)
            .cloned()
            .unwrap_or_else(|| main_argument(&call.arguments));
        let result = call
            .result
            .as_ref()
            .map(|r| short_result(&call.name, call.state, &r.text))
            .unwrap_or_default();
        let dash = if self.unicode { "—" } else { "-" };
        let mut line = format!("{} {}", marker(call.state, self.unicode), call.name);
        if !summary.is_empty() {
            line.push(' ');
            line.push_str(&cut(&summary, RESULT_CHARS, self.unicode));
        }
        if !result.is_empty() {
            line.push_str(&format!(
                " {dash} {}",
                cut(&result, RESULT_CHARS, self.unicode)
            ));
        }
        line
    }
}

/// The argument that says most about a call the tool did not summarize
/// (a denied or failed call).
fn main_argument(arguments: &Value) -> String {
    ["command", "path", "pattern"]
        .iter()
        .find_map(|key| arguments.get(key).and_then(Value::as_str))
        .map(|s| s.lines().next().unwrap_or_default().to_owned())
        .unwrap_or_default()
}

/// What a tool line says about the result.
fn short_result(tool: &str, state: CallState, text: &str) -> String {
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    match (tool, state) {
        ("run_command", _) => lines.last().copied().unwrap_or_default().to_owned(),
        ("read_file" | "list_files" | "search_files", CallState::Done) => {
            let n = lines.len();
            if n == 1 {
                "1 line".to_owned()
            } else {
                format!("{} lines", group(n))
            }
        }
        _ => lines.first().copied().unwrap_or_default().to_owned(),
    }
}
