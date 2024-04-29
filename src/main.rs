use clap::Parser;

use clai::data::Message;
use clai::cli::Args;
use clai::config::Config;
use clai::client::Client;
use clai::chat::Chat;



#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let config = Config::load().await.unwrap_or_default();
    let mut prompt = args.prompt().await?;

    config.check()?;
    
    let client = Client::new(config);
    let chat =Chat::new(&client);
    let answer = chat.send(&prompt).await?;

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