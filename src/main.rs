use clap::Parser;

use clai::agent::AgentBuilder;
use clai::cli::Args;
use clai::config::Config;
use clai::tools::{execute_command, list_files, read_file, McpTool};
use clai::workspace::WORKSPACE;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let config = Config::load()
        .await
        .inspect_err(|e| eprintln!("load config error: {e}"))
        .unwrap_or_default();

    let validator = clai::validator::Validator::new(WORKSPACE.root());
    let tools: Vec<Box<dyn McpTool>> = vec![
        Box::new(read_file::ReadFileTool::new(validator.clone())),
        Box::new(list_files::ListFilesTool::new(validator.clone())),
        Box::new(execute_command::ExecuteCommandTool::new(
            WORKSPACE.root().to_path_buf(),
        )),
    ];

    let builder = AgentBuilder::new().tools(tools);
    let mut agent = builder.build(config)?;
    args.command.run(&mut agent).await?;
    Ok(())
}
