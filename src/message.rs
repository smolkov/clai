use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub message: String,
}

impl Message {
    pub fn new(msg: &str) -> Message {
        Message {
            role: "user".to_string(),
            message: msg.to_owned(),
        }
    }
    pub fn append(mut self, msg: &str) -> Message {
        self.message.push('\n');
        self.message.push_str(msg);
        self
    }
    pub fn tools_call(&self) -> bool {
        false
    }
    pub fn comment(mut self, comment: &str) -> Message {
        self.message.push('\n');
        self.message.push_str(comment);
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub role: String,
    pub tool_name: String,
    pub message: String,
    pub input: serde_json::Value,
}

pub enum ResponseMessage {
    Text(Message),
    ToolCall(ToolCall),
}
