# clai

`clai` is an AI coding agent for the terminal, written in Rust. You describe
a task in plain language; `clai` sends it to a large language model, runs the
tool calls the model asks for (read and edit files, run commands) behind
permission rules, and shows every change and command.

> Status: in development. `clai run` (one task, without the TUI), sessions
> in the background server, saved and resumed (`clai new -d`, `ls`,
> `send`, `chat`, `kill`, `export`, `import`, `server`), `clai models` and `clai completion` work; the TUI (`clai`,
> `clai attach`) and `clai commit` exit with "not implemented yet". See
> [`todo.md`](todo.md).

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
| `clai chat` | Chat with a session in the terminal, or answer its questions. |
| `clai models` | List configured providers and models. |
| `clai commit` | Print a commit message for the changes in a directory. |
| `clai export`, `clai import` | Write a session to a file, add one from a file. |
| `clai server` | Run, stop or check the background server. |
| `clai completion [SHELL]` | Print the shell code for Tab completion. |

All options and exit codes: [`spec/command.md`](spec/command.md).

## Running a task

```sh
clai fix the failing test in tests/parse.rs
clai run --model local/qwen "explain src/main.rs"
clai - < task.txt                               # task from standard input
clai --yes -C ~/ws/app "run the tests and fix what fails"
```

The final answer goes to standard output, one line per tool call to
standard error. When `clai` runs in a terminal it asks before writing files
or running commands that the rules do not allow; in scripts such calls are
denied unless `--yes` is given. `deny` rules always apply. Exit codes: `0`
done, `1` failed or stopped, `2` usage or configuration error, `130`
`Ctrl-C`.

## Sessions in the background

Sessions run in a background server, one per user, which the first session
command starts and which exits after 60 s without sessions and clients. A
session keeps working while no one watches it, and waits only when it needs
a decision: a permission or limit question.

```sh
clai new -d --name api ~/ws/api       # start a session, print its name
clai send api "run the tests and fix what fails"
clai ls                               # name, workspace, model, state, age
clai chat                             # new session here, chat in the terminal
clai chat api                         # chat with a running session
clai chat --questions                 # only answer questions, of all sessions
clai chat --list                      # open questions, one per line
clai chat api allow --id q3           # answer from a script
clai kill api                         # end it and delete its saved files
clai server status                    # running, pid, sessions
clai server stop                      # stop the server; sessions stay saved
clai export api api.json              # write a session to one file
clai import api.json                  # add it again (e.g. on another machine)
```

`clai chat` is a plain-text chat: lines you type go to the model, its text
streams back with one line per tool call. `Ctrl-C` cancels the running
turn, `Ctrl-D` detaches (the session keeps working), `/help` lists the
commands (`/cancel`, `/model`, `/detach`, `/kill`). It shows each question
with the diff or command and reads the answer as a line: `y` allow, `a` always (then a rule), `n` deny, `e` deny
with a message, `c` cancel the turn; for a limit question `y` continue,
`n` stop.

Sessions are saved as they go, in `~/.local/state/clai/sessions/`
(`$XDG_STATE_HOME`), so they survive a server stop, a crash or a reboot.
`clai ls` shows them as `saved`, and `clai chat NAME` (or `send`) resumes
one with its whole conversation and the rules you allowed. A turn that was
running is interrupted, not continued. If its model is gone from the
configuration, resume with `clai chat NAME --model PROVIDER/MODEL`. The
files hold the conversation, including file contents and command output,
so they are readable only by you; API keys are never saved. The
server reloads `config.toml` when it changes; new sessions use the new
file. Tab completion knows session names.

The socket is `$XDG_RUNTIME_DIR/clai/server.sock` (or
`/tmp/clai-$UID/server.sock`), in a directory only you can read. Details:
[`spec/sessions.md`](spec/sessions.md).

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

A project can add `.clai/config.toml` to set `default_model`, `[commit]`, lower `[limits]` and
stricter `[permissions]`. `CLAI_MODEL` overrides `default_model` and
`CLAI_THEME` overrides the theme. All keys: [`spec/config.md`](spec/config.md). Sample files to start from
(minimal, Ollama, every key, read-only, for scripts) are in
[`tests/configs/`](tests/configs/readme.md).

## Permissions

Every tool call of the model is allowed, asked about or denied. By default
reading is allowed, and changing files and running commands ask. Rules in
`[permissions]` change that, grouped by action; each key is a tool or a
group (`read`, `edit`):

```toml
[permissions.allow]
run_command = ["ls", "cat", "cargo build", "cargo test", "git status", "git diff"]
edit        = ["src/**", "tests/**"]

[permissions.ask]
run_command = ["git push"]

[permissions.deny]
run_command = ["rm", "sudo", "git push --force"]
read        = [".env*", "secrets/**"]
```

Command patterns match the first words of each command: `git push` also
matches `git push origin main`, and `rm` denies every `rm`, also through
`sudo`, `xargs` or `sh -c`. A command with several parts (`a && b`, pipes,
`$( )`) is checked part by part, and redirections like `> file` are checked
as writes. Nothing outside the workspace (and `extra_dirs`) is ever read or
written. Details, exceptions (`except`) and messages for the model:
[`spec/config.md`](spec/config.md), "Permissions".

## Limits

A turn pauses and asks whether to continue after 50 model requests or 120
tool calls, so a confused model cannot loop forever. Change it with:

```toml
[limits]
max_requests   = 50
max_tool_calls = 120
```

A project file may only lower these values. Details:
[`spec/agent-loop.md`](spec/agent-loop.md), "Limits".

## Logging

`clai` logs to `~/.local/state/clai/clai.log` (or `$XDG_STATE_HOME/clai/clai.log`).
Set `CLAI_LOG=debug` for more detail.

## Specification

The behaviour is specified in [`spec/`](spec/readme.md); the build order is in
[`implementation.md`](implementation.md).
