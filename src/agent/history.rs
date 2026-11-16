//! The conversation of a session.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use serde_json::Value;

use crate::config::HistorySettings;
use crate::provider::{Block, Message, Role};
use crate::tools::group;

/// The result given to tool calls that a stopped server left without one.
pub const INTERRUPTED: &str = "not run or interrupted: the server stopped during this turn";

/// One change of a session's history, as saved in its file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case")]
pub enum Record {
    Message(Message),
    /// The whole output of a call whose result for the model was cut.
    Full {
        call_id: String,
        text: String,
    },
    /// Allow rules added to the session (`allow_always`), as text.
    Rules {
        rules: Vec<String>,
    },
}

/// Receives every change of the history as it happens.
pub type Journal = Arc<dyn Fn(Record) + Send + Sync>;

/// The messages sent to the model, and the full output of tool calls whose
/// result for the model was cut.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct History {
    messages: Vec<Message>,
    full: HashMap<String, String>,
}

impl History {
    pub fn messages(&self) -> &[Message] {
        &self.messages
    }

    /// The whole output of a tool call, for display.
    pub fn full_output(&self, call_id: &str) -> Option<&str> {
        self.full.get(call_id).map(String::as_str).or_else(|| {
            self.messages
                .iter()
                .flat_map(|m| &m.content)
                .find_map(|b| match b {
                    Block::ToolResult {
                        call_id: id,
                        content,
                        ..
                    } if id == call_id => Some(content.as_str()),
                    _ => None,
                })
        })
    }

    pub fn push(&mut self, role: Role, content: Vec<Block>) {
        if !content.is_empty() {
            self.messages.push(Message { role, content });
        }
    }

    /// Keeps the full output of a call when it differs from what the model
    /// got; returns whether it was kept.
    pub fn keep_full(&mut self, call_id: &str, model_text: &str, full: String) -> bool {
        if full == model_text {
            return false;
        }
        self.full.insert(call_id.to_owned(), full);
        true
    }

    /// The messages as the next request sends them: old tool results
    /// replaced with placeholders, in steps (RFC 0008, RFC 0028). The
    /// history itself keeps every result.
    pub fn for_request(&self, settings: &HistorySettings) -> Vec<Message> {
        let calls = Calls::of(&self.messages, settings);
        let line = calls.trim_line(settings);
        let mut messages = self.messages.clone();
        if line == 0 {
            return messages;
        }
        for message in &mut messages {
            for block in &mut message.content {
                let Block::ToolResult {
                    call_id,
                    content,
                    is_error,
                } = block
                else {
                    continue;
                };
                let Some(call) = calls.by_id.get(call_id.as_str()) else {
                    continue;
                };
                if call.exchange < line && call.trimmable {
                    *content = placeholder(call, content, *is_error);
                } else if let Some(earlier) = unchanged_target(content)
                    && calls
                        .by_id
                        .get(earlier)
                        .is_some_and(|e| e.exchange < line && e.trimmable)
                {
                    *content = format!(
                        "[unchanged since your read_file call {earlier}, which was trimmed; \
                         call the tool again if you need it]"
                    );
                }
            }
        }
        messages
    }

    /// The id of an earlier `read_file` call with the same arguments and
    /// the same result `text`, whose result the next request still sends
    /// whole (RFC 0028, "Reading an unchanged file again").
    pub fn unchanged_read(
        &self,
        arguments: &Value,
        text: &str,
        settings: &HistorySettings,
    ) -> Option<String> {
        let wanted = read_key(arguments)?;
        let calls = Calls::of(&self.messages, settings);
        let line = calls.trim_line(settings);
        let mut found = None;
        for block in self.messages.iter().flat_map(|m| &m.content) {
            let Block::ToolResult {
                call_id,
                content,
                is_error: false,
            } = block
            else {
                continue;
            };
            let Some(call) = calls.by_id.get(call_id.as_str()) else {
                continue;
            };
            if call.name == "read_file"
                && read_key(call.arguments) == Some(wanted.clone())
                && content == text
                && !(call.exchange < line && call.trimmable)
            {
                found = Some(call_id.clone());
            }
        }
        found
    }

    /// Results for the tool calls of the last message if they have none:
    /// a server that stopped during a turn leaves them so.
    pub fn missing_results(&self) -> Vec<Block> {
        let Some(last) = self.messages.last() else {
            return Vec::new();
        };
        if last.role != Role::Assistant {
            return Vec::new();
        }
        last.content
            .iter()
            .filter_map(|b| match b {
                Block::ToolCall { id, .. } => Some(Block::ToolResult {
                    call_id: id.clone(),
                    content: INTERRUPTED.to_owned(),
                    is_error: true,
                }),
                _ => None,
            })
            .collect()
    }
}

/// The start of the result of a repeated read of an unchanged file.
const UNCHANGED: &str = "unchanged since your read_file call ";
const UNCHANGED_END: &str = "; the content above is current";

/// The result of a repeated read of an unchanged file: a pointer to the
/// earlier call.
pub fn unchanged_result(earlier: &str) -> String {
    format!("{UNCHANGED}{earlier}{UNCHANGED_END}")
}

/// The call an unchanged-read result points to.
fn unchanged_target(content: &str) -> Option<&str> {
    content.strip_prefix(UNCHANGED)?.strip_suffix(UNCHANGED_END)
}

/// `read_file` arguments with their defaults, to compare two calls.
fn read_key(arguments: &Value) -> Option<(String, u64, u64)> {
    let path = arguments.get("path")?.as_str()?.to_owned();
    let offset = arguments.get("offset").map_or(Some(1), Value::as_u64)?;
    let limit = arguments.get("limit").map_or(Some(2000), Value::as_u64)?;
    Some((path, offset, limit))
}

/// A tool call of the history, for trimming.
struct CallInfo<'a> {
    name: &'a str,
    arguments: &'a Value,
    /// Which answer with tool calls it belongs to, counted from 0.
    exchange: usize,
    /// Its result is larger than `min_chars`.
    trimmable: bool,
    /// Characters of its result.
    chars: usize,
}

struct Calls<'a> {
    by_id: HashMap<&'a str, CallInfo<'a>>,
    /// Number of answers with tool calls.
    exchanges: usize,
}

impl<'a> Calls<'a> {
    fn of(messages: &'a [Message], settings: &HistorySettings) -> Calls<'a> {
        let mut by_id = HashMap::new();
        let mut exchanges = 0;
        for message in messages {
            let mut has_calls = false;
            for block in &message.content {
                match block {
                    Block::ToolCall {
                        id,
                        name,
                        arguments,
                    } => {
                        has_calls = true;
                        by_id.insert(
                            id.as_str(),
                            CallInfo {
                                name,
                                arguments,
                                exchange: exchanges,
                                trimmable: false,
                                chars: 0,
                            },
                        );
                    }
                    Block::ToolResult {
                        call_id, content, ..
                    } => {
                        if let Some(call) = by_id.get_mut(call_id.as_str()) {
                            call.chars = content.chars().count();
                            // A pointer to an earlier read is rewritten,
                            // not trimmed.
                            call.trimmable = call.chars > settings.min_chars
                                && unchanged_target(content).is_none();
                        }
                    }
                    Block::Text { .. } => {}
                }
            }
            if has_calls {
                exchanges += 1;
            }
        }
        Calls { by_id, exchanges }
    }

    /// The first answer whose results are sent whole: results of earlier
    /// answers are trimmed. The line moves only when the results not yet
    /// trimmed pass `trim_after`, and then to `keep_results` answers before
    /// the newest; so it stays put between steps and the request keeps its
    /// start for the prompt cache. It is replayed from the history alone,
    /// so a resumed session trims the same results.
    fn trim_line(&self, settings: &HistorySettings) -> usize {
        if !settings.trim_results {
            return 0;
        }
        let mut sizes = vec![0usize; self.exchanges];
        for call in self.by_id.values() {
            if call.trimmable
                && let Some(size) = sizes.get_mut(call.exchange)
            {
                *size += call.chars;
            }
        }
        let mut line = 0;
        let mut untrimmed = 0usize;
        for (k, size) in sizes.iter().enumerate() {
            untrimmed += size;
            if untrimmed > settings.trim_after {
                let new = (k + 1).saturating_sub(settings.keep_results);
                if new > line {
                    let cut: usize = sizes.get(line..new).unwrap_or_default().iter().sum();
                    untrimmed = untrimmed.saturating_sub(cut);
                    line = new;
                }
            }
        }
        line
    }
}

/// What the model gets instead of an old result.
fn placeholder(call: &CallInfo<'_>, content: &str, is_error: bool) -> String {
    let error = if is_error { "an error" } else { "not an error" };
    format!(
        "[trimmed {}: {} characters, {error}; call the tool again if you need it]",
        call_text(call.name, call.arguments),
        group(content.chars().count())
    )
}

/// `read_file src/a.rs offset 1 limit 2000`: the tool, its main argument,
/// then the others as `key value`, each cut to 80 characters.
fn call_text(name: &str, arguments: &Value) -> String {
    const MAIN: [&str; 3] = ["path", "command", "pattern"];
    let value = |v: &Value| {
        let text = match v {
            Value::String(s) => s.split_whitespace().collect::<Vec<_>>().join(" "),
            other => other.to_string(),
        };
        if text.chars().count() > 80 {
            format!("{}…", text.chars().take(80).collect::<String>())
        } else {
            text
        }
    };
    let mut out = name.to_owned();
    let Value::Object(map) = arguments else {
        return out;
    };
    let main = MAIN.iter().find_map(|k| map.get(*k).map(|v| (*k, v)));
    if let Some((_, v)) = main {
        out.push(' ');
        out.push_str(&value(v));
    }
    for (key, v) in map {
        if main.is_some_and(|(k, _)| k == key) {
            continue;
        }
        out.push_str(&format!(" {key} {}", value(v)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn settings(keep: usize, after: usize) -> HistorySettings {
        HistorySettings {
            trim_results: true,
            keep_results: keep,
            min_chars: 10,
            trim_after: after,
        }
    }

    /// A history of `results.len()` answers with one `read_file` call each.
    fn history(results: &[&str]) -> History {
        let mut h = History::default();
        h.push(
            Role::User,
            vec![Block::Text {
                text: "task".into(),
            }],
        );
        for (i, result) in results.iter().enumerate() {
            let id = format!("c{i}");
            h.push(
                Role::Assistant,
                vec![Block::ToolCall {
                    id: id.clone(),
                    name: "read_file".into(),
                    arguments: json!({"path": format!("f{i}.rs"), "limit": 50}),
                }],
            );
            h.push(
                Role::User,
                vec![Block::ToolResult {
                    call_id: id,
                    content: (*result).to_owned(),
                    is_error: i == 0,
                }],
            );
        }
        h
    }

    fn contents(messages: &[Message]) -> Vec<String> {
        messages
            .iter()
            .flat_map(|m| &m.content)
            .filter_map(|b| match b {
                Block::ToolResult { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn nothing_is_trimmed_below_trim_after() {
        let big = "x".repeat(100);
        let h = history(&[&big, &big, &big]);
        assert_eq!(h.for_request(&settings(1, 1_000)), h.messages());
    }

    #[test]
    fn trimming_happens_in_steps() {
        let big = "x".repeat(100);
        let s = settings(1, 250);
        // 100 + 100 = 200: not yet.
        let two = history(&[&big, &big]);
        assert_eq!(two.for_request(&s), two.messages());
        // 300 > 250: all but the newest answer are trimmed.
        let three = history(&[&big, &big, &big]);
        let sent = contents(&three.for_request(&s));
        assert_eq!(
            sent[0],
            "[trimmed read_file f0.rs limit 50: 100 characters, an error; call the tool again if you need it]"
        );
        assert!(
            sent[1].starts_with("[trimmed read_file f1.rs limit 50: 100 characters, not an error")
        );
        assert_eq!(sent[2], big);
        // The line stays while the untrimmed part (100 + 100) is below
        // trim_after: the start of the request does not change.
        let four = history(&[&big, &big, &big, &big]);
        let sent4 = contents(&four.for_request(&s));
        assert_eq!(sent4[..2], sent[..2]);
        assert_eq!(sent4[2], big);
        assert_eq!(sent4[3], big);
        // The next step moves it again.
        let five = history(&[&big, &big, &big, &big, &big]);
        let sent5 = contents(&five.for_request(&s));
        assert!(sent5[3].starts_with("[trimmed"));
        assert_eq!(sent5[4], big);
        // The history keeps everything.
        assert!(contents(five.messages()).iter().all(|c| *c == big));
    }

    #[test]
    fn small_results_and_switched_off() {
        let big = "x".repeat(100);
        let h = history(&["small", &big, &big, &big]);
        let sent = contents(&h.for_request(&settings(1, 150)));
        assert_eq!(sent[0], "small");
        assert!(sent[1].starts_with("[trimmed"));
        let off = HistorySettings {
            trim_results: false,
            ..settings(1, 150)
        };
        assert_eq!(h.for_request(&off), h.messages());
    }

    #[test]
    fn unchanged_reads() {
        let text = "     1→fn main() {}".repeat(3);
        let mut h = history(&["x", &text]);
        let s = settings(1, 1_000_000);
        let args = json!({"path": "f1.rs", "limit": 50});
        assert_eq!(h.unchanged_read(&args, &text, &s), Some("c1".into()));
        // Other arguments, other text, or defaults spelled out.
        assert_eq!(h.unchanged_read(&json!({"path": "f1.rs"}), &text, &s), None);
        assert_eq!(h.unchanged_read(&args, "changed", &s), None);
        assert_eq!(
            h.unchanged_read(
                &json!({"path": "f1.rs", "offset": 1, "limit": 50}),
                &text,
                &s
            ),
            Some("c1".into())
        );
        // Once the earlier result is trimmed, the content is read again,
        // and an older pointer to it says so.
        h.push(
            Role::Assistant,
            vec![Block::ToolCall {
                id: "c2".into(),
                name: "read_file".into(),
                arguments: args.clone(),
            }],
        );
        h.push(
            Role::User,
            vec![Block::ToolResult {
                call_id: "c2".into(),
                content: unchanged_result("c1"),
                is_error: false,
            }],
        );
        let big = "y".repeat(500);
        let mut more = h.clone();
        for i in 3..6 {
            let id = format!("c{i}");
            more.push(
                Role::Assistant,
                vec![Block::ToolCall {
                    id: id.clone(),
                    name: "list_files".into(),
                    arguments: json!({}),
                }],
            );
            more.push(
                Role::User,
                vec![Block::ToolResult {
                    call_id: id,
                    content: big.clone(),
                    is_error: false,
                }],
            );
        }
        let s = settings(1, 600);
        assert_eq!(more.unchanged_read(&args, &text, &s), None);
        let sent = contents(&more.for_request(&s));
        assert_eq!(
            sent[2],
            "[unchanged since your read_file call c1, which was trimmed; call the tool again if you need it]"
        );
    }

    #[test]
    fn call_texts() {
        assert_eq!(
            call_text(
                "run_command",
                &json!({"timeout_s": 5, "command": "cargo  test\nparse"})
            ),
            "run_command cargo test parse timeout_s 5"
        );
        let long = "a".repeat(100);
        assert_eq!(
            call_text("search_files", &json!({"pattern": long})),
            format!("search_files {}…", "a".repeat(80))
        );
        assert_eq!(call_text("list_files", &json!({})), "list_files");
    }
}
