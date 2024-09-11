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
    config.check()?;
    
    let client = Client::new(config);
    args.command.run(&client).await?;

   
    Ok(())
}