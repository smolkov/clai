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
                    let summary = self.summaries.get(&call.id).map(String::as_str);
                    out.push_str(&tool_line(call, summary, self.unicode));
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
}

/// `✓ read_file src/main.rs (lines 1–40) — 40 lines`, for a call in its
/// final state; `summary` from its earlier events.
pub fn tool_line(call: &ToolCallEvent, summary: Option<&str>, unicode: bool) -> String {
    let summary = summary.map_or_else(|| main_argument(&call.arguments), str::to_owned);
    let result = call
        .result
        .as_ref()
        .map(|r| short_result(&call.name, call.state, &r.text))
        .unwrap_or_default();
    let dash = if unicode { "—" } else { "-" };
    let mut line = format!("{} {}", marker(call.state, unicode), call.name);
    if !summary.is_empty() {
        line.push(' ');
        line.push_str(&cut(&summary, RESULT_CHARS, unicode));
    }
    if !result.is_empty() {
        line.push_str(&format!(" {dash} {}", cut(&result, RESULT_CHARS, unicode)));
    }
    line
}

/// The argument that says most about a call the tool did not summarize
/// (a denied or failed call).
pub(crate) fn main_argument(arguments: &Value) -> String {
    ["command", "path", "pattern"]
        .iter()
        .find_map(|key| arguments.get(key).and_then(Value::as_str))
        .map(|s| s.lines().next().unwrap_or_default().to_owned())
        .unwrap_or_default()
}

/// What a tool line says about an `lsp` result: `3 references`,
/// `1 error, 2 warnings`, `12 symbols`, `none found`.
fn lsp_result(lines: &[&str]) -> String {
    // Hover text: the code, not its fence.
    let lines: Vec<&str> = lines
        .iter()
        .copied()
        .filter(|l| !l.starts_with("```"))
        .collect();
    let first = lines.first().copied().unwrap_or_default();
    // `3 references to parse:`, `1 definition of parse:`, `2 callers of x:`.
    if first.ends_with(':') && first.starts_with(|c: char| c.is_ascii_digit()) {
        let head = first.trim_end_matches(':');
        for cut in [" to ", " of ", " from "] {
            if let Some((count, _)) = head.split_once(cut) {
                return count.to_owned();
            }
        }
        return head.to_owned();
    }
    match first {
        "(none found)" => return "none found".to_owned(),
        "(no errors or warnings)" => return "no errors".to_owned(),
        "(no symbols)" => return "no symbols".to_owned(),
        _ => {}
    }
    // Diagnostics: `path:1:2: error[E1]: …`.
    let count = |level: &str| {
        lines
            .iter()
            .filter(|l| {
                l.split(": ")
                    .nth(1)
                    .is_some_and(|w| w == level || w.starts_with(&format!("{level}[")))
            })
            .count()
    };
    let (errors, warnings) = (count("error"), count("warning"));
    if errors + warnings > 0 {
        let plural = |n: usize, word: &str| {
            if n == 1 {
                format!("1 {word}")
            } else {
                format!("{n} {word}s")
            }
        };
        let mut parts = Vec::new();
        if errors > 0 {
            parts.push(plural(errors, "error"));
        }
        if warnings > 0 {
            parts.push(plural(warnings, "warning"));
        }
        return parts.join(", ");
    }
    // An outline or a list of symbols: one per line.
    let symbols = lines
        .iter()
        .filter(|l| !l.starts_with('(') && !l.starts_with('…'))
        .count();
    let listed = lines
        .iter()
        .all(|l| l.starts_with('(') || l.starts_with('…') || l.contains("  "));
    if listed && symbols > 0 {
        return if symbols == 1 {
            "1 symbol".to_owned()
        } else {
            format!("{symbols} symbols")
        };
    }
    first.to_owned()
}

/// What a tool line says about the result.
pub(crate) fn short_result(tool: &str, state: CallState, text: &str) -> String {
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
        ("lsp", CallState::Done) => lsp_result(&lines),
        // The answers, without the questions: `SQLite; Search, Export`.
        ("ask_user", CallState::Done) if lines.len() > 1 => lines
            .iter()
            .filter(|l| l.starts_with("   "))
            .map(|l| l.trim())
            .collect::<Vec<_>>()
            .join("; "),
        _ => lines.first().copied().unwrap_or_default().to_owned(),
    }
}
