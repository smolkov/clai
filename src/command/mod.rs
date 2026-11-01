//! One module per command of `spec/command.md`: its arguments and its `run`.

pub mod attach;
pub mod commit;
pub mod completion;
pub mod kill;
pub mod ls;
pub mod models;
pub mod new;
pub mod run;
pub mod send;
pub mod server;

use std::path::Path;

use crate::cli::Command;
use crate::error::Result;

impl Command {
    /// Name of the command, as used in messages.
    pub fn name(&self) -> &'static str {
        match self {
            Command::Run(_) => "run",
            Command::New(_) => "new",
            Command::Attach(_) => "attach",
            Command::Ls(_) => "ls",
            Command::Kill(_) => "kill",
            Command::Send(_) => "send",
            Command::Models(_) => "models",
            Command::Commit(_) => "commit",
            Command::Server(server) => server.name(),
            Command::Completion(_) => "completion",
        }
    }

    /// Runs the command; `config` is the `--config` option.
    pub fn run(&self, config: Option<&Path>) -> Result<()> {
        match self {
            Command::Run(c) => c.run(config),
            Command::New(c) => c.run(config),
            Command::Attach(c) => c.run(config),
            Command::Ls(c) => c.run(config),
            Command::Kill(c) => c.run(config),
            Command::Send(c) => c.run(config),
            Command::Models(c) => c.run(config),
            Command::Commit(c) => c.run(config),
            Command::Server(c) => c.run(config),
            Command::Completion(c) => c.run(config),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::cli::Cli;
    use crate::error::Error;

    /// Commands not built yet report their own name.
    #[test]
    fn unbuilt_commands_are_unsupported() {
        let cases: &[(&[&str], &str)] = &[
            (&[], "new"),
            (&["new"], "new"),
            (&["attach"], "attach"),
            (&["ls"], "ls"),
            (&["kill", "a"], "kill"),
            (&["send", "a", "hi"], "send"),
            (&["commit"], "commit"),
            (&["server"], "server"),
            (&["server", "stop"], "server stop"),
            (&["server", "status"], "server status"),
        ];
        for (args, name) in cases {
            let cli = Cli::parse_args(std::iter::once("clai").chain(args.iter().copied())).unwrap();
            let command = cli.into_command();
            assert_eq!(command.name(), *name);
            match command.run(None) {
                Err(Error::Unsupported { what }) => assert_eq!(what, *name, "{args:?}"),
                other => panic!("{args:?}: {other:?}"),
            }
        }
    }
}
