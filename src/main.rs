use clap::Parser;

use clai::data::{Message, OpenAIChatResponse, OpenAIRequest};
use std::time::Duration;

use clai::cli::Args;
use clai::config::Config;
use clai::prompt::{self, Prompt};


#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let question = args.input.join(" ");
    let config = Config::load().await.unwrap_or_default();
    config.check()?;

    let mut req = OpenAIRequest {
        model: config.model.clone(),
        messages: vec![],
    };
    let mut prompt = if let Some(prompt) = args.prompt.as_ref() {
        Prompt::load(&prompt).await?
    }else {
        Prompt::empty()
    };
    prompt.messages.push(Message {
        role: "user".to_owned(),
        content: question,
    });
    req.messages.extend(prompt.messages.clone());
    let answer = test_request_to_openai(&req, &config).await?;
    println!(
        "assistant:{}",
        answer.choices.last().unwrap().message.content
    );
    if args.prompt.is_some() && args.update {
        prompt.messages.extend(answer.choices.iter().map(|choice| choice.message.clone()).collect::<Vec<Message>>());
        prompt.update().await?;
    }
    Ok(())
}
async fn test_request_to_openai(
    req: &OpenAIRequest,
    config: &Config,
) -> anyhow::Result<OpenAIChatResponse> {
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

async fn save_prompt(req: &OpenAIRequest, resp: &OpenAIChatResponse, prompt: String) {
    println!("save prompt {}",prompt);
}
