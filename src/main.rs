use clap::Parser;

use clai::agent::AgentBuilder;
use clai::cli::Args;
use clai::config::Config;
use clai::tools::read_file;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let config = Config::load()
        .await
        .inspect_err(|e| eprintln!("load config error: {e}"))
        .unwrap_or_default();
    let builder = AgentBuilder::new().tool(Box::new(read_file::ReadFileTool {}));
    let mut agent = builder.build(config)?;
    args.command.run(&mut agent).await?;
    Ok(())
}
