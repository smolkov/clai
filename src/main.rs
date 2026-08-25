use clap::Parser;

use clai::agent::AgentBuilder;
use clai::cli::Args;
use clai::config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let config = Config::load()
        .await
        .inspect_err(|e| eprintln!("load config error: {e}"))
        .unwrap_or_default();
    let builder = AgentBuilder::new();
    let mut agent = builder.build(config)?;
    args.command.run(&mut agent).await?;
    Ok(())
}
