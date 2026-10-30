# clai

`clai` is an AI coding agent for the terminal, written in Rust. You describe
a task in plain language; `clai` sends it to a large language model, runs the
tool calls the model asks for (read and edit files, run commands) behind
permission rules, and shows every change and command.

> Status: in development. The command line is parsed, but no command works
> yet; each one exits with "not implemented yet". See [`todo.md`](todo.md).

## Build

Needs a Rust toolchain with edition 2024 support.

```sh
cargo build --release
./target/release/clai --help
```

## Commands

| Command | Purpose |
|---------|---------|
| `clai` | New session in the current directory, with the TUI. |
| `clai TASK...`, `clai run TASK...` | Run one task without the TUI and exit. |
| `clai new`, `clai attach`, `clai ls`, `clai kill`, `clai send` | Manage sessions in the background server. |
| `clai models` | List configured providers and models. |
| `clai commit` | Print a commit message for the changes in a directory. |
| `clai server` | Run, stop or check the background server. |

All options and exit codes: [`spec/command.md`](spec/command.md).

## Configuration

Providers and models are defined in `~/.config/clai/config.toml`; see
[`spec/config.md`](spec/config.md).

## Logging

`clai` logs to `~/.local/state/clai/clai.log` (or `$XDG_STATE_HOME/clai/clai.log`).
Set `CLAI_LOG=debug` for more detail.

## Specification

The behaviour is specified in [`spec/`](spec/readme.md); the build order is in
[`implementation.md`](implementation.md).
