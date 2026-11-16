# Startup: one place for the environment, paths and the command context

- RFC: 0026
- Status: done
- Milestone: none; a refactoring of `src/main.rs`, `src/command/` and the
  modules that read the environment
- Spec: [`spec/architecture.md`](../spec/architecture.md), "Project tree",
  "Roles"; [`spec/config.md`](../spec/config.md), "Files", "Environment
  variables", "Logging"; [`spec/sessions.md`](../spec/sessions.md),
  "Server"

## Summary

`codeagent` reads its environment, works out its directories and loads its
configuration in many places: each command, the TUI, the client that
starts the server, the server, logging and the completion code do it on
their own. This RFC moves that into one step at startup. `main` reads the
process environment once into a `Startup` value (variables, directories,
current directory, terminals), and every command gets it through its
`Context`, together with one way to resolve a workspace, load the
configuration for it, connect to the server and run async code. Behaviour
stays the same, except where today's copies differ from each other; those
cases are listed and made one.

## Motivation

What `codeagent` reads at startup, and where:

| What | Read in |
|------|---------|
| `HOME`, `XDG_CONFIG_HOME`, `CODEAGENT_MODEL`, `CODEAGENT_THEME` | `config::Environment::from_process`, called from `command::Context`, `client::start`, `command::server`, `chat::talk`, `complete` |
| `HOME` alone | `command::common::shown`, `tui::sessions`, `tui::view::settings`, `log::state_dir` |
| `XDG_STATE_HOME` | `log::state_dir` (log file and saved sessions) |
| `XDG_RUNTIME_DIR`, the user id | `server::paths::runtime_dir` |
| The current directory | `command::common::workspace`, `command::models`, `command::chat`, `command::completion`, `complete` |
| Terminals | `tui::run`, `command::run`, `command::server` |

Problems this causes:

- **Paths are worked out in several places.** The user file
  (`config::user_config_path`), the state directory (`log::state_dir`), the
  sessions directory (`command::server`), the log file (`log::log_path`)
  and the socket (`server::paths`) each read the environment themselves.
  The rules agree today, but nothing keeps them so: `XDG_CONFIG_HOME` must
  be absolute in one place and is not checked in another
  (`skills::search_dirs` checks it again).
- **The workspace is resolved three ways.** `command::common::workspace`
  makes the path absolute, `codeagent skills` and `codeagent run --skill` also
  resolve links, the server resolves links for sessions, and
  `codeagent models` uses the current directory as it is. RFC 0025's
  `last_session` has to compare both forms for this reason.
- **Every command loads the configuration itself,** with its own
  `LoadOptions` and its own choice of workspace: the TUI attaching to a
  session loads it for the runtime directory, so that no project file
  applies; `codeagent chat` for the current directory; `client::start` for the
  runtime directory, to check the user file before it starts a server.
- **Every command builds its runtime.** `current_thread()` or
  `multi_thread()` is called in thirteen places, each with the same
  `block_on`.
- **Tests set process variables** (`HOME`, `XDG_*`) because the code reads
  them directly; unit tests of commands cannot give a home directory
  without a child process.
- **Logging starts before anything else is known.** The log file of a
  client and of the server is the same, and its lines do not say which
  process wrote them; a failed `log::init` is a warning on every command.

## Scope

In:

- A `Startup` value: the environment `codeagent` reads, read once in `main`.
- `Paths`: every directory and file `codeagent` uses, worked out from `Startup`
  in one module.
- `Context`: what every command gets, with `Startup`, `Paths`, `--config`,
  and the helpers for a workspace, the configuration, a client and a
  runtime.
- One workspace rule for all commands.
- Logging set up from `Paths`, with the role of the process in each line.
- Moving the commands and modules to these, one group at a time.

Out:

- New commands, options or files; changed messages, except where noted.
- The server's own startup after `serve` (sessions, reload, idle exit).
- The protocol.
- Windows or other platforms.

## Design

### Startup

```rust
/// What `codeagent` reads from its process, once, in `main`.
pub struct Startup {
    pub home: Option<PathBuf>,
    pub xdg_config_home: Option<PathBuf>,   // only if absolute
    pub xdg_state_home: Option<PathBuf>,    // only if absolute
    pub xdg_runtime_dir: Option<PathBuf>,   // only if absolute
    pub uid: u32,
    pub current_dir: Option<PathBuf>,
    pub model: Option<String>,              // CODEAGENT_MODEL, not empty
    pub theme: Option<String>,              // CODEAGENT_THEME, not empty
    pub stdin_terminal: bool,
    pub stdout_terminal: bool,
}
```

- `Startup::from_process()` is the only function that reads the process
  environment, the current directory and the terminals; everything else
  takes `&Startup`. **(proposal)**
- Relative `XDG_*` values are dropped here, once, as the XDG rules say;
  today each reader does it or not.
- `config::Environment` becomes a part of `Startup` (or is replaced by
  it); `LoadOptions` takes `&Startup`.
- Tests build a `Startup` by hand.
- The server keeps its rule that `CODEAGENT_MODEL` and `CODEAGENT_THEME` come from
  each client: it clears them in its copy, as `command::server` does now.

### Paths

```rust
pub struct Paths {
    pub user_file: Option<PathBuf>,   // --config, else $XDG_CONFIG_HOME/codeagent/config.toml
    pub themes: Option<PathBuf>,      // next to the user file
    pub state: Option<PathBuf>,       // $XDG_STATE_HOME/codeagent
    pub sessions: Option<PathBuf>,    // state/sessions
    pub log: Option<PathBuf>,         // state/codeagent.log
    pub runtime: PathBuf,             // $XDG_RUNTIME_DIR/codeagent or /tmp/codeagent-UID
    pub socket: PathBuf,              // runtime/server.sock
    pub lock: PathBuf,                // runtime/server.lock
}
```

- `Paths::new(&Startup, config_file)` in `src/paths.rs` replaces
  `config::user_config_path`, `themes::directory`, `log::state_dir`,
  `log::log_path` and `server::paths::{runtime_dir, socket_path,
  lock_path}`. `server::paths::prepare` stays where it is.
- A path that cannot be worked out (no `HOME`) is `None`, and the command
  that needs it fails with today's message; commands that do not need it
  (`codeagent completion`, `codeagent --help`) never look.
- `~/…` for messages: one function `Paths::tilde(path)` with the home of
  `Startup`, instead of the four copies that read `HOME`.

### The workspace

One function, `Context::workspace(dir: Option<&Path>)`: `dir` or the
current directory, which must be a directory, made absolute and with links
resolved. **(proposal)** That is what the server does with a session's
workspace today, so client and server compare the same paths.

- `codeagent models` and `codeagent chat` use it too, so the project file they read is
  the one of the resolved directory (today: the same file, found through
  another spelling of the path).
- A workspace path in a message stays as the user sees it (`~/ws/project`).
- `command::common::last_session` then compares one form.

### Context

```rust
pub struct Context {
    pub startup: Startup,
    pub paths: Paths,
    /// `--config FILE`.
    pub config_file: Option<PathBuf>,
}

impl Context {
    pub fn workspace(&self, dir: Option<&Path>) -> Result<PathBuf>;
    /// The configuration with the project file of `workspace`.
    pub fn config(&self, workspace: &Path) -> Result<Config>;
    /// The user file only: for commands about a session elsewhere
    /// (`codeagent attach`, the server start check).
    pub fn user_config(&self) -> Result<Config>;
    /// Runs `f` on a current-thread runtime with a connected client.
    pub fn with_client<T>(&self, connect: Connect, f: impl AsyncFnOnce(&mut Client) -> Result<T>) -> Result<T>;
    /// Runs `f` on a runtime: current-thread for clients, multi-thread for
    /// the server and `codeagent run`.
    pub fn block_on<T>(&self, kind: Runtime, f: impl Future<Output = Result<T>>) -> Result<T>;
}
```

- `user_config` replaces "load for the runtime directory, so that no
  project file applies", which today is spelled out in `tui::run` and
  `client::start`. **(proposal)**
- `Connect` is built from `Paths` (`socket`), not from `runtime_dir()`.
- Commands keep their `Execute::run(&self, ctx)`; they stop calling
  `std::env`, `config::load` and `runtime::*` themselves.

### Logging

- `log::init(&Paths, role)` after the arguments are parsed, with the role
  `client`, `server` or `run`; every line carries it and the pid
  (`role=server pid=4242`). **(proposal)**
- A log that cannot be opened is a warning only for the server and
  `codeagent run`, which do work worth logging; a client goes on without one
  quietly. **(proposal)** See "Open questions".
- `codeagent completion` and the completion callback (`COMPLETE=…`) do not open
  the log.

### main

```rust
fn main() -> ExitCode {
    complete_if_asked();                         // before any output
    let cli = parse_or_exit();
    let startup = Startup::from_process();
    let paths = Paths::new(&startup, cli.config.as_deref());
    log::init(&paths, cli.role());
    let ctx = Context { startup, paths, config_file: cli.config.clone() };
    run(cli.into_command(), &ctx)                // report, exit code
}
```

The order is the one today, written down: completion, parse, environment,
paths, log, command.

### Behaviour changes

Only these, each from two copies that differ today:

- `codeagent models` and `codeagent chat` resolve links in the workspace, as the
  server does.
- A relative `XDG_STATE_HOME` or `XDG_RUNTIME_DIR` is ignored everywhere
  (today: ignored, but by each reader on its own).
- A client without a log file prints no warning (if accepted).
- Log lines name their process.

## Spec changes

- `spec/architecture.md`, "Project tree": `src/startup.rs`, `src/paths.rs`;
  "Roles": startup order of a process.
- `spec/config.md`, "Environment variables": read once at startup; relative
  `XDG_*` ignored. "Logging": role and pid in each line; the warning only
  for the server and `codeagent run`.
- `spec/command.md`, "Conventions": the workspace rule (absolute, links
  resolved).

## Testing

- `src/startup.rs`: `from_process` with relative and absolute `XDG_*`,
  empty `CODEAGENT_MODEL`, no `HOME` (in a child process, as now).
- `src/paths.rs` unit tests: every path from a `Startup` built by hand;
  `--config`; no `HOME`; `tilde`.
- `Context::workspace`: a link, a relative directory, a file, a missing
  directory.
- The command tests that set `HOME` and `XDG_*` for a child process stay
  and must pass unchanged: they are the check that behaviour did not
  change.
- `src/log.rs`: the role and pid in a line written to a temporary file.

## Implementation plan

1. **Startup and Paths**
   - Status: done
   - Files: `src/startup.rs`, `src/paths.rs`, `src/lib.rs`, `src/main.rs`,
     `src/command/mod.rs`
   - Result: `main` builds both and puts them in `Context`; nothing uses
     them yet.
   - Tests: their unit tests.

2. **Configuration and directories**
   - Status: done
   - Files: `src/config/mod.rs`, `src/config/themes.rs`, `src/log.rs`,
     `src/server/paths.rs`, `src/command/server.rs`, `src/client/mod.rs`,
     `src/skills/mod.rs`
   - Result: `Environment` and the path functions read `Startup` and
     `Paths`; the old functions are gone.
   - Tests: config, server and client tests.

3. **Commands**
   - Status: done
   - Files: `src/command/*.rs`, `src/command/chat/`, `src/tui/mod.rs`,
     `src/complete.rs`
   - Result: every command uses `ctx.workspace`, `ctx.config`,
     `ctx.with_client`, `ctx.block_on`; no `std::env` outside `startup.rs`.
   - Tests: all process tests unchanged; a check in `ci/check.sh` that
     `std::env::var` appears only in `src/startup.rs` (and tests).

4. **Logging**
   - Status: done
   - Files: `src/log.rs`, `src/main.rs`
   - Result: role and pid per line; the warning rule.
   - Tests: the log test.

5. **Spec and readme**
   - Status: done
   - Files: the spec files above, `todo.md`
   - Result: the spec describes the startup.
   - Tests: `ci/check.sh`, `ci/test.sh`.

## Alternatives

- **Leave it, fix the copies when they drift.** No work now, but every new
  command (`codeagent skills`, `--continue` this week) adds a copy, and the
  workspace forms already differ.
- **A global `OnceLock<Startup>`.** Fewer parameters, but tests could not
  give different environments to code in one process, which is the point.
- **An application struct that owns the runtime, the client and the
  configuration for the whole process.** Fewer calls, but the server, the
  TUI and `codeagent run` need different runtimes and lifetimes of the
  configuration (the server reloads it); one owner would have to know all
  of them.

## Open questions

All answered; see "Answers".

1. Should `Startup` replace `config::Environment` entirely (proposal), or
   wrap it, so the config module keeps its own type for its tests?
2. Workspace with links resolved for every command (proposal), or keep the
   current directory as typed for `codeagent models` and `codeagent chat`?
3. Clients without a log file: quiet (proposal), or keep the warning on
   every command?
4. Should the check "no `std::env::var` outside `startup.rs`" be in
   `ci/check.sh` (proposal), or only a review rule in `AGENTS.md`?
5. One log file for all processes with the role in each line (proposal), or
   a separate `server.log`?

## Answers

1. to 5. The user asked to build it, so the proposals hold: `Startup`
   replaces `config::Environment` (the name stays as an alias, so the
   config module and its tests read as before); the workspace is resolved
   with links for every command; a client without a log is quiet; the
   check is in `ci/check.sh` (`ci/env-check.sh`); one log file, the role
   and pid on each line.

## Notes from building

- **Startup keeps every variable.** Many parts look variables up by name
  (`api_key_env`, `LANG` and `LC_*` for the symbols, `PAGER`, `SHELL`,
  `PATH`, `NO_COLOR`, `COLORTERM`, the clipboard's `WAYLAND_DISPLAY` and
  `DISPLAY`), so `Startup` holds a copy of all of them with `var`,
  `var_os` and `getenv`; the typed fields (`home`, `xdg_*`, `model`,
  `theme`) are made from that copy. Tests build one with `from_vars`.
- **The provider reads `api_key_env` from it.** `provider::from_config`
  takes the `Startup`; the server's provider factory holds the server's.
- **The TUI** gets the home directory through `Theming::home` (the
  settings title, `~` in a typed workspace of the session list) and the
  clipboard's `PATH` from the load options.
- **Completion** runs before `main` builds a context, on every Tab, so it
  makes its own with `Startup::from_process()`.
- **The server's runtime** stays its own `multi_thread()` with a
  shutdown timeout; `Context::block_on` serves the client commands.
- **The check** is `ci/env-check.sh`, run by `ci/check.sh` and tested in
  `tests/ci.sh`. Test code is exempt: files named `tests.rs`, files under
  `tests/`, and a file's rest after its first `#[cfg(test)]`.
  `std::env::args_os`, `split_paths`, `consts` and `current_exe` (in
  `startup::current_exe`) read no environment and are allowed.
- **Behaviour changes, as listed:** links resolved in every workspace
  (`last_session` compares one form now); a relative `XDG_STATE_HOME` or
  `XDG_RUNTIME_DIR` ignored in one place; no log warning for clients
  (the test `unwritable_log_warns_only_where_work_is_logged`); the role
  and pid on each log line.
- `Startup::default()` is `from_vars` with no variables, so its
  temporary directory is `/tmp` as in a real start.
