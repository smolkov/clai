# clai

`clai` is an AI coding agent for the terminal, written in Rust. You describe
a task in plain language; `clai` sends it to a large language model, runs the
tool calls the model asks for (read and edit files, run commands) behind
permission rules, and shows every change and command.

> Status: in development. Only `clai models` and `clai completion` work; the other commands exit
> with "not implemented yet". See [`todo.md`](todo.md).

## Build

Needs a Rust toolchain with edition 2024 support.

```sh
cargo build --release
./target/release/clai --help
```

## Shell completion

```sh
echo 'source <(clai completion bash)' >> ~/.bashrc   # bash
echo 'source <(clai completion zsh)'  >> ~/.zshrc    # zsh
echo 'clai completion fish | source'  >> ~/.config/fish/config.fish
```

Tab then completes commands, options, and the configured models for
`--model`. `elvish` and `powershell` work too; see
[`spec/command.md`](spec/command.md), "clai completion".

## Commands

| Command | Purpose |
|---------|---------|
| `clai` | New session in the current directory, with the TUI. |
| `clai TASK...`, `clai run TASK...` | Run one task without the TUI and exit. |
| `clai new`, `clai attach`, `clai ls`, `clai kill`, `clai send` | Manage sessions in the background server. |
| `clai models` | List configured providers and models. |
| `clai commit` | Print a commit message for the changes in a directory. |
| `clai server` | Run, stop or check the background server. |
| `clai completion [SHELL]` | Print the shell code for Tab completion. |

All options and exit codes: [`spec/command.md`](spec/command.md).

## Configuration

Providers and models are defined in `~/.config/clai/config.toml`
(`$XDG_CONFIG_HOME/clai/config.toml`); `clai` has no built-in list. A
minimal file:

```toml
default_model = "anthropic/sonnet"

[providers.anthropic]
api         = "anthropic"
api_key_env = "ANTHROPIC_API_KEY"

[providers.anthropic.models.sonnet]
id             = "claude-sonnet-5-5"
context_window = 200000
```

Check it with `clai models`, which lists every configured model and marks
the default and providers whose key variable is not set.

A project can add `.clai/config.toml` to set `default_model`, `[commit]` and
stricter `[permissions]`. `CLAI_MODEL` overrides `default_model` and
`CLAI_THEME` overrides the theme. All keys: [`spec/config.md`](spec/config.md).

## Logging

`clai` logs to `~/.local/state/clai/clai.log` (or `$XDG_STATE_HOME/clai/clai.log`).
Set `CLAI_LOG=debug` for more detail.

## Specification

The behaviour is specified in [`spec/`](spec/readme.md); the build order is in
[`implementation.md`](implementation.md).
