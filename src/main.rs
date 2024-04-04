use clap::Parser;
use reqwest::header::{HeaderMap, AUTHORIZATION, CONTENT_TYPE};
use std::time::Duration;
use clai::data::{Message, OpenAIRequest,OpenAIChatResponse};
use std::env;

use clai::cli::Args;


#[tokio::main]
async fn main() -> anyhow::Result<()> {

    let args = Args::parse();
    let question = args.input.join(" ");
    println!("user:{}",question);
    let message = OpenAIRequest{
        model: "gpt-3.5-turbo".to_owned(),
        messages:vec![Message{ role: "user".to_owned(), content:question}],
    };
    let api_key = env::var("OPENAI_API_KEY")?;
    let answer = test_request_to_openai(&message,&api_key).await?;
    println!("assistant:{}",answer.choices.last().unwrap().message.content);
    Ok(())
}
async fn test_request_to_openai(req: &OpenAIRequest,api_key:&str) -> anyhow::Result<OpenAIChatResponse> {
    let client = reqwest::Client::new();
    let mut headers = HeaderMap::new();
    
    headers.insert(
        AUTHORIZATION,
        format!("Bearer {}",api_key).parse().unwrap()
    );
    headers.insert(CONTENT_TYPE, "application/json".parse().unwrap());
   
    let output = client
        .post("https://api.openai.com/v1/chat/completions")
        .timeout(Duration::from_secs(4))
        .headers(headers)
        .json(req)
        .send()
        .await?
        .json()
        .await?;

        Ok(output)
}


