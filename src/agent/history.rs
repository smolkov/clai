//! The conversation of a session.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::provider::{Block, Message, Role};

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
