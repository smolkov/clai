use clap::Parser;

use clai::cli::Args;
use clai::config::Config;



#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let config = Config::load().await.inspect_err(|e| eprintln!("load config error: {e}")).unwrap_or_default();
    config.check()?;
    let mut client = clai::client::create_client(config);
    args.command.run(&mut client).await?;
    Ok(())
}