# clai

`clai` is an AI coding agent for the terminal, written in Rust. You describe
a task in plain language; `clai` sends it to a large language model, runs the
tool calls the model asks for (read and edit files, run commands) behind
permission rules, and shows every change and command.

> Status: in development. The TUI (`clai`, `clai new`, `clai attach`), `clai run`
> (one task, without the TUI), sessions in the background server, saved and
> resumed (`ls`, `send`, `chat`, `kill`, `export`, `import`, `server`),
> `clai models`, `clai commit` and `clai completion` work. The TUI `/commit`
> dialog and context management come next. See [`todo.md`](todo.md).

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
| `clai commit` | Print a commit message for the staged changes (`-a`: all changes of tracked files): `clai commit -a \| git commit -a -F -`. |
| `clai export`, `clai import` | Write a session to a file, add one from a file. |
| `clai server` | Run, stop or check the background server. |
| `clai completion [SHELL]` | Print the shell code for Tab completion. |

All options and exit codes: [`spec/command.md`](spec/command.md).

## The TUI

`clai` creates a session in the current directory and opens the full-screen
interface; `clai new [DIR]` does the same for `DIR`, and `clai attach NAME`
returns to a session (without a name and with several sessions, it opens the
session list). Leaving only detaches: the session keeps running.

| Key | Action |
|-----|--------|
| `Enter` | Send the message; also while a turn runs (it is queued). |
| `Alt-Enter`, `Shift-Enter` | New line. `Shift-Enter` needs a terminal that reports it. |
| `Up` / `Down` | Earlier messages of the session. |
| `Esc`, `Ctrl-C` | Cancel the running turn; without one, `Ctrl-C` clears the input. |
| `Ctrl-C` twice, `Ctrl-D` on empty input | Detach. |
| `PgUp` / `PgDn`, mouse wheel | Scroll. `Ctrl-Home` / `Ctrl-End`: start / end, and follow again. |
| `Ctrl-O` | Choose the model: type to filter, `Up` / `Down` select, `Enter` changes it, `Esc` closes. |
| `Ctrl-S` | Session list: `Enter` attach, `n` new (workspace, name, then the model), `r` rename, `k` kill, `Esc` close. |
| `Ctrl-N` / `Ctrl-P` | Attach to the next / previous session. |
| `Tab` | Switch to the conversation: `Up` / `Down` select, `Enter` expands a tool call, `?` shows help. |

A tool call that needs permission opens a prompt: `y` yes, `a` always (`Tab`
widens the rule, `Enter` confirms), `n` no, `e` no with a message to the model,
`Esc` no and cancel the turn. A turn at its limit asks `y` continue, `n` stop.

Commands: `/help`, `/model [PROVIDER/MODEL]` (without one, choose it as with
`Ctrl-O`), `/sessions`, `/new [NAME]` (a session in the same workspace; the model
picker starts on `default_model`, so `Enter` takes it), `/attach [NAME]`,
`/rename NAME`, `/status [NAME]`, `/stats [7d|30d|all]`, `/settings`, `/theme [NAME]`, `/kill`, `/detach`, `/quit`. `/` in the empty input lists them:
type to search, `Up` / `Down` select, `Enter` runs, `Tab` completes, `Esc` closes
the list and keeps the text. After `/attach ` and `/model ` a list offers the
session names and models the same way. The list also shows commands of later
versions, dimmed. A message that starts with `/` is written `//…`.

Switching keeps each session's unsent input, scroll position and expanded tool
calls. When the attached session ends, the session list opens.

`/status` shows the attached session, `/status NAME` another one, running or
saved: state, workspace, clients, where it is saved; the model with its key,
context use and settings; the running turn against `[limits]`; and statistics
over the whole session: turns, requests, retries, errors, tokens, time spent
working, tool calls per tool, and questions by answer and client. It updates
while open; `Esc` closes it.

`/stats` opens the same box on its Global tab (`Tab` switches): statistics over
all sessions of the server for the last 7 days, 30 days or all time (`7`, `3`,
`a`): sessions, turns, requests, tokens and working time, a table per model
with its share of the tokens, and a chart of tokens per day. `R` counts again.

`/settings` shows the `[tui]` settings: change one with `Enter` or `Left` /
`Right` and see it at once, `d` sets the default, `s` saves into the config file
(comments and order stay), `Esc` closes. `/theme` lists the themes and previews
the selected one; `Enter` takes it and saves it as the default, `/theme NAME`
does so directly. Besides `terminal`, `dark` and `light`, `clai` brings
`atom-one-dark`, `nord`, `gruvbox-dark` and `solarized-light`. Your own themes go
in the config file as `[themes.NAME]` or as files in `~/.config/clai/themes/`,
e.g. `themes/mine.toml`:

```toml
base       = "dark"       # terminal | dark | light
background = "#282c34"    # optional: paints the whole screen
text       = "#abb2bf"
accent     = "#61afef"
```

The session row ends with the tokens the session has used, `in 45.1k out 3.2k`.
Input counts the history again with each request, so this is neither the context
size nor a bill.

Without a terminal the TUI exits with `2`; use `clai run` or `clai chat`.
`[tui]` in the configuration has `symbols`, `mouse` (off: the terminal's own
text selection works), `tool_output_lines`, `expand_tool_calls`, `timestamps`,
`scroll_lines`, `max_fps` and `activity`; `NO_COLOR` turns colours off.

While the session works, a line above the input shows what it does, since
when, and the output tokens of the turn:
`✻ Prestidigitating… (14s · ↓ 1.2k tokens · Esc cancel)`. The glyph turns to
the warning colour when the model has been quiet for 30 s.
`tui.activity = "plain"` says `Thinking…` instead of a playful verb, `"off"`
hides the line.

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
a decision: a permission or limit question, or a question from the model.

When a choice is yours (which design, which database), the model can ask
with the `ask_user` tool: one to four questions with options, answered in
a box in the TUI or as numbered lines in `clai chat` and `clai run`; you
can always type your own answer instead. The turn goes on with the answer.
`ask_user = "deny"` in `[permissions]` turns it off.

```sh
clai new -d --name api ~/ws/api       # start a session, print its name
clai attach api                       # open it in the TUI
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
commands (`/cancel`, `/model`, `/usage`, `/status [NAME]`, `/stats [RANGE]`, `/sessions`, `/attach NAME`,
`/new [NAME] [PROVIDER/MODEL]`, `/models`, `/detach`, `/kill`). It shows each question
with the diff or command and reads the answer as a line: `y` allow, `a` always (then a rule), `n` deny, `e` deny
with a message, `c` cancel the turn; for a limit question `y` continue,
`n` stop; for a question from the model an option's number (`1,3: text`
for several, with a text), your own text, `s` skip or `c` cancel.

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

Out of the box, `clai` keeps well-known secret files from the model:
`.env` and `.env.*` (not `.env.example` and the like), key files (`*.pem`,
`*.key`, `*.p12`, `*.pfx`, `id_rsa`, …), `.netrc` and `.pgpass`. They are
not read, listed or searched, and an allowed command that names one, such
as `cat .env`, asks first. No `allow` rule or `--yes` passes them; to let
the model read one, add it to `builtin_except` in your own config file:

```toml
[permissions]
builtin_except = ["deploy/certs/*.pem"]
```

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
