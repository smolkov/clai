use serde::{Deserialize,Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct Message {
    pub role: String,
    pub content: String,
}
#[derive(Serialize,Deserialize,Debug)]
pub struct Choise {
	pub index: u32,
	pub message: Message,
}

#[derive(Serialize,Deserialize,Debug)]
pub struct Usage {
	pub prompt_tokens: u32 ,
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
#[derive(Serialize,Deserialize,Debug)]
pub struct OpenAIChatResponse {
	pub id: String,
	pub object: String,
	// FIXME: use chrono time
	pub created: u64,
	pub model: String,
	#[serde(rename = "system_fingerprint")]
	pub fingerprint: String,
	pub choices: Vec<Choise>,
	pub usage:Usage,

	
}