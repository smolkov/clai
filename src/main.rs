//! `codeagent`: AI coding agent for the terminal. Behaviour: `spec/`.

use std::process::ExitCode;

use clap::CommandFactory;
use clap_complete::CompleteEnv;

use codeagent::cli::Cli;
use codeagent::command::Context;
use codeagent::command::completion::COMPLETE_ENV;
use codeagent::error::Error;
use codeagent::log;
use codeagent::output::note;
use codeagent::paths::{self, Moved};
use codeagent::startup::Startup;

fn main() -> ExitCode {
    // Answers `COMPLETE=<shell> codeagent -- <words>` and exits; must run before
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

    // The order of RFC 0026: the environment once, the paths from it, the
    // log, then the command.
    let startup = Startup::from_process();
    // `codeagent` was `clai` before 0.5.0: its directories move once.
    for moved in paths::move_old_dirs(&startup) {
        match moved {
            Moved::Done(old, new) => note(&format!(
                "moved {} to {}",
                startup.tilde(&old),
                startup.tilde(&new)
            )),
            Moved::Kept(old, why) => warn(&format!("{} not moved: {why}", startup.tilde(&old))),
        }
    }
    let ctx = Context::new(startup, cli.config.clone());
    let role = cli.role();
    if let Err(err) = log::init(&ctx.paths, &ctx.startup, role)
        && role.warns()
    {
        warn(&format!("{err}; logging is off"));
    }
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
