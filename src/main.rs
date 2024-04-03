use reqwest::header::{HeaderMap, AUTHORIZATION, CONTENT_TYPE};
use std::time::Duration;
use clai::data::{Message, OpenAIRequest,OpenAIChatResponse};



#[tokio::main]
async fn main() -> anyhow::Result<()> {
    println!("Hello, world!");
    let answer = test_request_to_openai().await?;
    println!("{answer:?}");
    Ok(())
}
async fn test_request_to_openai() -> anyhow::Result<OpenAIChatResponse> {
    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        format!("Bearer {}", API_KEY).parse().unwrap(),
    );
    headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());
    let message = OpenAIRequest{
        model: "gpt-3.5-turbo".to_owned(),
        messages:vec![Message{ role: "user".to_owned(), content:"Hello!".to_owned()}],
    };
    let output = client
        .post("https://api.openai.com/v1/chat/completions")
        .timeout(Duration::from_secs(4))
        .headers(headers)
        .json(&message)
        .send()
        .await?
        .json()
        .await?;

        Ok(output)
}


