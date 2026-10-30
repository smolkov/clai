//! `clai`: AI coding agent for the terminal. Behaviour: `spec/`.

mod cli;
mod error;
mod log;

use std::io::Write;
use std::process::ExitCode;

use crate::cli::Cli;
use crate::error::{Error, Result};

fn main() -> ExitCode {
    let cli = match Cli::parse_args(std::env::args_os()) {
        Ok(cli) => cli,
        Err(err) => {
            // Help and version exit 0, usage errors exit 2.
            let _ = err.print();
            return ExitCode::from(u8::try_from(err.exit_code()).unwrap_or(2));
        }
    };

    if let Err(err) = log::init() {
        warn(&format!("{err}; logging is off"));
    }
    tracing::info!(command = cli.command_name(), "start");

    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(command = cli.command_name(), error = %err);
            let _ = writeln!(std::io::stderr(), "clai: {err}");
            ExitCode::from(err.exit_code())
        }
    }
}

/// Runs the parsed command. No command is built yet.
fn run(cli: &Cli) -> Result<()> {
    Err(Error::Unsupported {
        what: cli.command_name(),
    })
}

fn warn(message: &str) {
    let _ = writeln!(std::io::stderr(), "clai: warning: {message}");
}
