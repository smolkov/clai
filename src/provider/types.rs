//! Shared types: the same for every provider (see `spec/providers.md`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    /// Model id from the configuration, sent as is.
    pub model: String,
    /// System prompt; may be empty.
    pub system: String,
    /// Oldest first.
    pub messages: Vec<Message>,
    pub tools: Vec<ToolSpec>,
    pub max_output_tokens: Option<u64>,
    pub temperature: Option<f64>,
}

/// Saved in session files (see `spec/sessions.md`, "Persistence"); the
/// serialized form must stay readable by later versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<Block>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Message {
        Message {
            role: Role::User,
            content: vec![Block::Text { text: text.into() }],
        }
    }

    pub fn assistant(text: impl Into<String>) -> Message {
        Message {
            role: Role::Assistant,
            content: vec![Block::Text { text: text.into() }],
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Block {
    Text {
        text: String,
    },
    /// In assistant messages.
    ToolCall {
        id: String,
        name: String,
        /// A JSON object, or the raw text as a JSON string if the model sent
        /// invalid JSON.
        arguments: Value,
    },
    /// In user messages; answers the `ToolCall` with the same id.
    ToolResult {
        call_id: String,
        content: String,
        is_error: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    /// JSON Schema of the arguments (an `object` schema).
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    TextDelta {
        text: String,
    },
    ToolCallStart {
        id: String,
        name: String,
    },
    ToolCallArgsDelta {
        id: String,
        json: String,
    },
    ToolCallEnd {
        id: String,
        arguments: Value,
    },
    /// Token counts of this request; the last one counts.
    Usage {
        input_tokens: u64,
        output_tokens: u64,
    },
    /// Last event of a stream.
    Done {
        stop_reason: StopReason,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopReason {
    EndTurn,
    ToolUse,
    MaxTokens,
    Other,
}

/// Arguments of a finished tool call: empty text is `{}`; text that is not
/// a JSON object is kept as a JSON string, so the agent loop can report it.
pub fn parse_arguments(raw: &str) -> Value {
    if raw.trim().is_empty() {
        return Value::Object(serde_json::Map::new());
    }
    match serde_json::from_str::<Value>(raw) {
        Ok(value @ Value::Object(_)) => value,
        _ => Value::String(raw.to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn arguments() {
        assert_eq!(parse_arguments(""), json!({}));
        assert_eq!(parse_arguments(" \n"), json!({}));
        assert_eq!(parse_arguments(r#"{"path":"a"}"#), json!({"path": "a"}));
        assert_eq!(parse_arguments(r#"{"path":"#), json!(r#"{"path":"#));
        assert_eq!(parse_arguments("[1]"), json!("[1]"));
    }
}
