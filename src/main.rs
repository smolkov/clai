use clap::Parser;

use clai::cli::Args;
use clai::config::Config;
use clai::client::Client;



#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let config = Config::load().await.inspect_err(|e| eprintln!("load config error: {e}")).unwrap_or_default();
    config.check()?;
    let client = Client::new(config);
    args.command.run(&client).await?;
    Ok(())
}