use clap::Parser;

use std::time::Duration;
use clai::data::{Message, OpenAIRequest,OpenAIChatResponse};
use std::env;

use clai::cli::Args;
use clai::prompt::Prompt;
use clai::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {

    let args = Args::parse();
    let question = args.input.join(" ");
    let config = Config::load().await.unwrap_or_default();
    config.check()?;

    let mut req = OpenAIRequest{
        model: config.model.clone(),
        messages:vec![],
    };
    if let Some(prompt) = args.prompt {
        let prompt = Prompt::load(&prompt).await?;
        req.messages.extend(prompt.messages().await);
    }
    req.messages.push(Message{ role: "user".to_owned(), content:question});

    let answer = test_request_to_openai(&req,&config).await?;
    println!("assistant:{}",answer.choices.last().unwrap().message.content);
    Ok(())
}
async fn test_request_to_openai(req: &OpenAIRequest,config:&Config) -> anyhow::Result<OpenAIChatResponse> {
    let client = reqwest::Client::new();
   
   
    let output = client
        .post("https://api.openai.com/v1/chat/completions")
        .timeout(Duration::from_secs(40))
        .headers(config.header()?)
        .json(req)
        .send()
        .await?
        .json()
        .await?;

        Ok(output)
}


