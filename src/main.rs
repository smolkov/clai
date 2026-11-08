//! `clai`: AI coding agent for the terminal. Behaviour: `spec/`.

use std::process::ExitCode;

use clap::CommandFactory;
use clap_complete::CompleteEnv;

use clai::cli::Cli;
use clai::command::Context;
use clai::command::completion::COMPLETE_ENV;
use clai::error::Error;
use clai::log;
use clai::output::note;

fn main() -> ExitCode {
    // Answers `COMPLETE=<shell> clai -- <words>` and exits; must run before
    // anything is written to standard output.
    CompleteEnv::with_factory(Cli::command)
        .var(COMPLETE_ENV)
        .complete();

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
    let ctx = Context {
        config_file: cli.config.clone(),
    };
    let command = cli.into_command();
    let command = command.execute();
    tracing::info!(command = command.name(), "start");

    match command.run(&ctx) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            tracing::error!(command = command.name(), error = %err);
            report(&err);
            ExitCode::from(err.exit_code())
        }
    }
}

/// Prints an error on standard error; configuration errors one per line.
fn report(err: &Error) {
    match err {
        Error::Exit(_) => {}
        Error::Config(errors) => {
            for error in &errors.0 {
                note(&error.to_string());
            }
        }
        _ => note(&err.to_string()),
    }
}

fn warn(message: &str) {
    note(&format!("warning: {message}"));
}
