//! Command-line arguments, as specified in `spec/command.md`.

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::LazyLock;

use clap::error::ErrorKind;
use clap::{Args, CommandFactory, Parser, Subcommand, ValueHint};
use clap_complete::ArgValueCompleter;

use crate::command::Execute;
use crate::command::attach::Attach;
use crate::command::chat::Chat;
use crate::command::commit::Commit;
use crate::command::completion::Completion;
use crate::command::export::Export;
use crate::command::import::Import;
use crate::command::kill::Kill;
use crate::command::ls::Ls;
use crate::command::models::Models;
use crate::command::new::New;
use crate::command::run::Run;
use crate::command::send::Send;
use crate::command::server::Server;
use crate::complete;

/// The text of `--version`: the crate version, and the commit and date of
/// the build when `ci/build.sh` set them (RFC 0020).
static VERSION: LazyLock<String> = LazyLock::new(|| {
    version_text(
        env!("CARGO_PKG_VERSION"),
        option_env!("CLAI_BUILD_COMMIT"),
        option_env!("CLAI_BUILD_DATE"),
    )
});

/// `0.2.0`, `0.2.0 (abc1234)` or `0.2.0 (abc1234 2026-11-11)`.
fn version_text(version: &str, commit: Option<&str>, date: Option<&str>) -> String {
    let built: Vec<&str> = [commit, date]
        .into_iter()
        .flatten()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    if built.is_empty() {
        version.to_owned()
    } else {
        format!("{version} ({})", built.join(" "))
    }
}

/// AI coding agent for the terminal.
#[derive(Debug, Parser)]
#[command(name = "clai", version = VERSION.as_str(), disable_help_subcommand = true)]
pub struct Cli {
    /// Use FILE instead of the user config file.
    #[arg(long, global = true, value_name = "FILE", value_hint = ValueHint::FilePath)]
    pub config: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Command>,

    #[command(flatten)]
    pub top: TopArgs,
}

/// Arguments of `clai` without a command: a new session, or the short form
/// `clai TASK...` of `clai run`.
#[derive(Debug, Args)]
pub struct TopArgs {
    /// Model to use, as PROVIDER/MODEL.
    #[arg(long, value_name = "PROVIDER/MODEL", add = ArgValueCompleter::new(complete::models))]
    pub model: Option<String>,

    /// Session name (only without TASK).
    #[arg(long, conflicts_with = "task")]
    pub name: Option<String>,

    /// Allow tool calls that would ask (only with TASK).
    #[arg(long, requires = "task")]
    pub yes: bool,

    /// Never ask; deny tool calls that would ask (only with TASK).
    #[arg(long, requires = "task", conflicts_with = "yes")]
    pub no_ask: bool,

    /// Workspace (only with TASK).
    #[arg(short = 'C', value_name = "DIR", requires = "task", value_hint = ValueHint::DirPath)]
    pub dir: Option<PathBuf>,

    /// Task to run one-shot; `-` reads it from standard input.
    #[arg(value_name = "TASK")]
    pub task: Vec<String>,
}

/// The commands; each has its own struct and `run` in `src/command/`.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run one task without the TUI and exit.
    Run(Run),
    /// Create a session and attach the TUI to it.
    New(New),
    /// Attach the TUI to a running session.
    Attach(Attach),
    /// List sessions.
    Ls(Ls),
    /// End sessions.
    Kill(Kill),
    /// Send a message to a session without attaching.
    Send(Send),
    /// Chat with a session in the terminal, or answer questions of sessions.
    Chat(Chat),
    /// Write a session to a file.
    Export(Export),
    /// Add a saved session from a file written by `clai export`.
    Import(Import),
    /// List configured providers and models.
    Models(Models),
    /// Print a commit message for the changes in a directory.
    Commit(Commit),
    /// Run, stop or check the background server.
    Server(Server),
    /// Print the shell code for Tab completion.
    Completion(Completion),
}

impl Command {
    /// The command to run: the one dispatch over all commands.
    pub fn execute(&self) -> &dyn Execute {
        match self {
            Command::Run(c) => c,
            Command::New(c) => c,
            Command::Attach(c) => c,
            Command::Ls(c) => c,
            Command::Kill(c) => c,
            Command::Send(c) => c,
            Command::Chat(c) => c,
            Command::Export(c) => c,
            Command::Import(c) => c,
            Command::Models(c) => c,
            Command::Commit(c) => c,
            Command::Server(c) => c,
            Command::Completion(c) => c,
        }
    }
}

impl Cli {
    /// Parses the arguments, including the checks clap cannot express.
    /// Help and version also come back as an error, with exit code 0.
    pub fn parse_args<I, T>(args: I) -> Result<Cli, clap::Error>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let cli = Cli::try_parse_from(args)?;
        let top = &cli.top;
        let top_used = top.model.is_some()
            || top.name.is_some()
            || top.yes
            || top.dir.is_some()
            || !top.task.is_empty();
        if cli.command.is_some() && top_used {
            return Err(Cli::command().error(
                ErrorKind::ArgumentConflict,
                "options of `clai` without a command and a task cannot be used with a command; \
                 put command options after the command name",
            ));
        }
        Ok(cli)
    }

    /// The command to run. `clai` alone is `clai new`, and `clai TASK...`
    /// is `clai run TASK...`.
    pub fn into_command(self) -> Command {
        match self.command {
            Some(command) => command,
            None if self.top.task.is_empty() => Command::New(New {
                model: self.top.model,
                name: self.top.name,
                ..New::default()
            }),
            None => Command::Run(Run {
                model: self.top.model,
                yes: self.top.yes,
                no_ask: self.top.no_ask,
                dir: self.top.dir,
                task: self.top.task,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::parse_args(std::iter::once("clai").chain(args.iter().copied()))
            .unwrap_or_else(|e| panic!("{args:?}: {e}"))
    }

    fn parse_err(args: &[&str]) -> clap::Error {
        match Cli::parse_args(std::iter::once("clai").chain(args.iter().copied())) {
            Ok(cli) => panic!("{args:?} parsed as {cli:?}"),
            Err(e) => e,
        }
    }

    #[test]
    fn definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn version_text_with_and_without_build_info() {
        assert_eq!(version_text("0.2.0", None, None), "0.2.0");
        assert_eq!(version_text("0.2.0", Some(""), Some(" ")), "0.2.0");
        assert_eq!(
            version_text("0.2.0", Some("abc1234"), None),
            "0.2.0 (abc1234)"
        );
        assert_eq!(
            version_text("0.2.0", Some("abc1234"), Some("2026-11-11")),
            "0.2.0 (abc1234 2026-11-11)"
        );
    }

    #[test]
    fn no_arguments_is_new_session() {
        let cli = parse(&["--model", "m", "--name", "x"]);
        assert!(cli.command.is_none());
        let Command::New(new) = cli.into_command() else {
            panic!("not new");
        };
        assert_eq!(new.model.as_deref(), Some("m"));
        assert_eq!(new.name.as_deref(), Some("x"));
        assert!(!new.detach);
    }

    #[test]
    fn top_level_model_and_name() {
        let cli = parse(&["--model", "anthropic/opus", "--name", "x"]);
        assert_eq!(cli.top.model.as_deref(), Some("anthropic/opus"));
        assert_eq!(cli.top.name.as_deref(), Some("x"));
    }

    #[test]
    fn short_form_is_run() {
        let cli = parse(&["--yes", "-C", "/tmp", "fix", "the", "test"]);
        assert!(cli.command.is_none());
        assert_eq!(cli.top.task, ["fix", "the", "test"]);
        assert!(cli.top.yes);
        assert_eq!(cli.top.dir, Some(PathBuf::from("/tmp")));
        let Command::Run(run) = cli.into_command() else {
            panic!("not run");
        };
        assert_eq!(run.task, ["fix", "the", "test"]);
        assert!(run.yes);
        assert_eq!(run.dir, Some(PathBuf::from("/tmp")));
    }

    #[test]
    fn short_form_with_command_name_later() {
        let cli = parse(&["fix", "ls"]);
        assert_eq!(cli.top.task, ["fix", "ls"]);
    }

    #[test]
    fn help_is_a_task() {
        let cli = parse(&["help"]);
        assert_eq!(cli.top.task, ["help"]);
    }

    #[test]
    fn task_from_stdin() {
        let cli = parse(&["-"]);
        assert_eq!(cli.top.task, ["-"]);
    }

    #[test]
    fn run_command() {
        let cli = parse(&["run", "--model", "local/qwen", "--yes", "commit", "docs"]);
        let Some(Command::Run(args)) = cli.command else {
            panic!("not run");
        };
        assert_eq!(args.task, ["commit", "docs"]);
        assert_eq!(args.model.as_deref(), Some("local/qwen"));
        assert!(args.yes);
    }

    #[test]
    fn no_ask() {
        let cli = parse(&["--no-ask", "fix"]);
        let Command::Run(run) = cli.into_command() else {
            panic!("not run");
        };
        assert!(run.no_ask && !run.yes);
        let Some(Command::Run(run)) = parse(&["run", "--no-ask", "fix"]).command else {
            panic!("not run");
        };
        assert!(run.no_ask);
        for args in [
            &["--yes", "--no-ask", "x"][..],
            &["run", "--yes", "--no-ask", "x"][..],
        ] {
            assert_eq!(parse_err(args).kind(), ErrorKind::ArgumentConflict);
        }
        assert_eq!(
            parse_err(&["--no-ask"]).kind(),
            ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn run_needs_task() {
        assert_eq!(
            parse_err(&["run"]).kind(),
            ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn name_with_task_is_error() {
        assert_eq!(
            parse_err(&["--name", "x", "fix"]).kind(),
            ErrorKind::ArgumentConflict
        );
    }

    #[test]
    fn yes_and_dir_without_task_are_errors() {
        assert_eq!(
            parse_err(&["--yes"]).kind(),
            ErrorKind::MissingRequiredArgument
        );
        assert_eq!(
            parse_err(&["-C", "/tmp"]).kind(),
            ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn new_command() {
        let cli = parse(&["new", "-d", "--name", "a", "--model", "m", "/w"]);
        let Some(Command::New(args)) = cli.command else {
            panic!("not new");
        };
        assert!(args.detach);
        assert_eq!(args.name.as_deref(), Some("a"));
        assert_eq!(args.model.as_deref(), Some("m"));
        assert_eq!(args.dir, Some(PathBuf::from("/w")));
    }

    #[test]
    fn attach_ls_models() {
        assert!(matches!(
            parse(&["attach"]).command,
            Some(Command::Attach(Attach { name: None }))
        ));
        assert!(matches!(
            parse(&["attach", "docs"]).command,
            Some(Command::Attach(Attach { name: Some(n) })) if n == "docs"
        ));
        assert!(matches!(
            parse(&["ls", "--json"]).command,
            Some(Command::Ls(Ls { json: true }))
        ));
        assert!(matches!(
            parse(&["models"]).command,
            Some(Command::Models(Models {
                json: false,
                set_default: None
            }))
        ));
    }

    #[test]
    fn kill_names_or_all() {
        assert!(matches!(
            parse(&["kill", "a", "b"]).command,
            Some(Command::Kill(Kill { names, all: false })) if names == ["a", "b"]
        ));
        assert!(matches!(
            parse(&["kill", "--all"]).command,
            Some(Command::Kill(Kill { all: true, .. }))
        ));
        assert_eq!(
            parse_err(&["kill"]).kind(),
            ErrorKind::MissingRequiredArgument
        );
        assert_eq!(
            parse_err(&["kill", "a", "--all"]).kind(),
            ErrorKind::ArgumentConflict
        );
    }

    #[test]
    fn send_command() {
        assert!(matches!(
            parse(&["send", "docs", "hello", "there"]).command,
            Some(Command::Send(Send { name, message })) if name == "docs" && message == ["hello", "there"]
        ));
        assert_eq!(
            parse_err(&["send", "docs"]).kind(),
            ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn commit_command() {
        let cli = parse(&["commit", "--hint", "fixes #12", "src"]);
        let Some(Command::Commit(args)) = cli.command else {
            panic!("not commit");
        };
        assert_eq!(args.hint.as_deref(), Some("fixes #12"));
        assert_eq!(args.dir, Some(PathBuf::from("src")));
        assert!(!args.all);
        let cli = parse(&["commit", "-a"]);
        let Some(Command::Commit(args)) = cli.command else {
            panic!("not commit");
        };
        assert!(args.all);
    }

    fn name(args: &[&str]) -> &'static str {
        parse(args).into_command().execute().name()
    }

    #[test]
    fn server_commands() {
        assert_eq!(name(&["server"]), "server");
        assert_eq!(name(&["server", "stop", "--force"]), "server stop");
        assert_eq!(name(&["server", "status"]), "server status");
    }

    /// Each command answers to its own name in log lines and messages.
    #[test]
    fn every_command_has_its_name() {
        let cases: &[(&[&str], &str)] = &[
            (&["run", "x"], "run"),
            (&["fix", "it"], "run"),
            (&[], "new"),
            (&["new"], "new"),
            (&["attach"], "attach"),
            (&["ls"], "ls"),
            (&["kill", "s"], "kill"),
            (&["send", "s", "hi"], "send"),
            (&["chat"], "chat"),
            (&["export", "s", "f"], "export"),
            (&["import", "f"], "import"),
            (&["models"], "models"),
            (&["commit"], "commit"),
            (&["completion", "bash"], "completion"),
        ];
        for (args, expected) in cases {
            assert_eq!(name(args), *expected, "{args:?}");
        }
    }

    #[test]
    fn config_before_and_after_command() {
        let before = parse(&["--config", "c.toml", "ls"]);
        let after = parse(&["ls", "--config", "c.toml"]);
        assert_eq!(before.config, Some(PathBuf::from("c.toml")));
        assert_eq!(after.config, Some(PathBuf::from("c.toml")));
        assert!(matches!(before.command, Some(Command::Ls(_))));
    }

    #[test]
    fn config_with_short_form() {
        let cli = parse(&["--config", "c.toml", "fix", "it"]);
        assert_eq!(cli.config, Some(PathBuf::from("c.toml")));
        assert_eq!(cli.top.task, ["fix", "it"]);
    }

    #[test]
    fn top_level_option_before_command_is_error() {
        let err = parse_err(&["--model", "m", "ls"]);
        assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn unknown_option_is_error() {
        assert_eq!(parse_err(&["--nope"]).kind(), ErrorKind::UnknownArgument);
        assert_eq!(parse_err(&["--nope"]).exit_code(), 2);
    }
}
