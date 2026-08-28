use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: Content,
}

impl Message {
    pub fn new(msg: &str) -> Message {
        Message {
            role: "user".to_string(),
            content: Content::Text(msg.to_string()),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub name: String,
    pub id: String,
    pub message: String,
    pub input: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    #[serde(rename = "type")]
    pub tool_type: String,
    pub name: String,
    pub tool_use_id: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Content {
    Text(String),
    ToolCall(ToolCall),
    ToolResult(ToolResult),
}

impl Content {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Content::Text(text) => Some(text),
            _ => None,
        }
    }
    pub fn is_text(&self) -> bool {
        matches!(self, Content::Text(_))
    }
    pub fn is_tool_call(&self) -> bool {
        matches!(self, Content::ToolCall(_))
    }
    pub fn is_tool_result(&self) -> bool {
        matches!(self, Content::ToolResult(_))
    }
}
