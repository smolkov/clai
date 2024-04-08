use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize,PartialEq, Debug)]
pub struct Message {
    pub role: String,
    pub content: String,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct Choise {
    pub index: u32,
    pub message: Message,
    pub finish_reason: Option<String>,
    pub logprobs: Option<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Usage {
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub total_tokens: u32,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct OpenAIRequest {
    #[serde(rename = "model")]
    pub model: String,
    #[serde(rename = "messages")]
    pub messages: Vec<Message>,
}
#[derive(Serialize, Deserialize, Debug)]
pub struct OpenAIChatResponse {
    pub id: String,
    pub object: String,
    // FIXME: use chrono time
    pub created: u64,
    pub model: String,
    #[serde(rename = "system_fingerprint")]
    pub fingerprint: String,
    pub choices: Vec<Choise>,
    pub usage: Usage,
}

impl Message {
    pub fn new(role: &str, content: &str) -> Message {
        Message {
            role: role.to_owned(),
            content: content.to_owned(),
        }
    }
}

// {
// 	"choices": [
// 	  {
// 		"finish_reason": "stop",
// 		"index": 0,
// 		"message": {
// 		  "content": "The 2020 World Series was played in Texas at Globe Life Field in Arlington.",
// 		  "role": "assistant"
// 		},
// 		"logprobs": null
// 	  }
// 	],
// 	"created": 1677664795,
// 	"id": "chatcmpl-7QyqpwdfhqwajicIEznoc6Q47XAyW",
// 	"model": "gpt-3.5-turbo-0613",
// 	"object": "chat.completion",
// 	"usage": {
// 	  "completion_tokens": 17,
// 	  "prompt_tokens": 57,
// 	  "total_tokens": 74
// 	}
//   }
