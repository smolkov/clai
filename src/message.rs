use crate::message;

pub struct Message {
    pub message: String,
}

impl Message {
    pub fn new(msg: &str) -> Message {
        Message {
            message: msg.to_owned(),
        }
    }
    pub fn append(mut self, msg: &str) -> Message {
        self.message.push('\n');
        self.message.push_str(msg);
        self
    }

    pub fn comment(mut self, comment: &str) -> Message {
        self.message.push('\n');
        self.message.push_str(comment); 
        self
    }
}
