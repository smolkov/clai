//! The conversation of a session.

use std::collections::HashMap;

use crate::provider::{Block, Message, Role};

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
    /// got.
    pub fn keep_full(&mut self, call_id: &str, model_text: &str, full: String) {
        if full != model_text {
            self.full.insert(call_id.to_owned(), full);
        }
    }
}
