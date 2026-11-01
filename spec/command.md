# Commands

This file lists every command-line command of `clai`: syntax, options, output
and exit codes. What a command does in detail is described in the topic file
it links to.

Statements marked **(assumption)** need a decision from the user.

## Overview

| Command | Purpose | Details |
|---------|---------|---------|
| [`clai`](#clai) | New session in the current directory, attach the TUI. | [`sessions.md`](sessions.md), [`tui.md`](tui.md) |
| [`clai run`](#clai-run) | One-shot: run one task, print the result, exit. | this file |
| [`clai new`](#clai-new) | New session, attach or leave it running. | [`sessions.md`](sessions.md) |
| [`clai attach`](#clai-attach) | Attach the TUI to a running session. | [`sessions.md`](sessions.md) |
| [`clai ls`](#clai-ls) | List sessions. | [`sessions.md`](sessions.md) |
| [`clai kill`](#clai-kill) | End a session. | [`sessions.md`](sessions.md) |
| [`clai send`](#clai-send) | Send a message to a session without attaching. | [`sessions.md`](sessions.md) |
| [`clai chat`](#clai-chat) | Answer permission and limit questions in the terminal, without the TUI. | [`sessions.md`](sessions.md), "Questions" |
| [`clai models`](#clai-models) | List configured providers and models. | [`config.md`](config.md) |
| [`clai commit`](#clai-commit) | Print a commit message for the changes in a directory. | [`commit.md`](commit.md) |
| [`clai server`](#clai-server) | Run, stop or check the background server. | [`sessions.md`](sessions.md) |
| [`clai completion`](#clai-completion) | Print the shell code for Tab completion. | this file |

## Conventions

- Syntax: `[X]` is optional, `X...` may be repeated, `A | B` is one of them.
- Model references are written `PROVIDER/MODEL` as defined in the
  configuration (see [`config.md`](config.md), "Model references"). Only
  configured models are accepted.
- Standard output carries only the result of a command, so it can be piped.
  Progress, notices and errors go to standard error. Progress is shown only
  when standard error is a terminal.
- Commands that open the TUI need a terminal on standard input and output;
  otherwise they fail with exit code `2`.
- Commands that use sessions (`attach`, `ls`, `kill`, `send`, `chat`, `new`) start
  the server if it is not running (see [`sessions.md`](sessions.md),
  "Server"). `models`, `commit` and `run` work without the server.
  **(assumption)**

- Options of `clai` itself (`--model`, `--name`, `--yes`, `-C`) and a task
  cannot be combined with a command: `clai --model m ls` is an error (exit
  `2`); options of a command come after its name. Global options are the
  exception.
- There is no `help` command; `clai help` runs the task "help". Help is
  `-h` / `--help`.

## Global options

Accepted by every command, before or after the command name.

| Option | Meaning |
|--------|---------|
| `-h`, `--help` | Show help for `clai` or the given command, exit `0`. |
| `-V`, `--version` | Show the version, exit `0`. |
| `--config FILE` | Use `FILE` instead of the user config file. **(assumption)** |

## Exit codes

The same for all commands, unless a command says otherwise.

| Code | Meaning |
|------|---------|
| `0`  | Success. |
| `1`  | The command ran but failed: task failed or cancelled, session not found, nothing to commit, model request failed, … |
| `2`  | Invalid arguments or configuration, or no terminal where one is needed. |
| `130` | Interrupted with `Ctrl-C` (one-shot commands only). |

## clai

```sh
clai [--model PROVIDER/MODEL] [--name NAME]
```

Same as `clai new` in the current directory: creates a session and attaches
the TUI to it.

## clai run

```sh
clai run [--model PROVIDER/MODEL] [--yes] [-C DIR] TASK...
clai TASK...
```

Runs one task without the TUI and exits when it is done.

- `clai TASK...` is a short form of `clai run TASK...`. It is used when the
  first argument is not a command name; `clai run` is needed for a task that
  starts with a command name (e.g. `clai run "commit the docs"`).
- `TASK...` words are joined with spaces. `-` reads the task from standard
  input.
- Runs in a temporary session that is not listed and ends with the task.
- Standard output: the model's final answer as plain text (Markdown
  source): the text of the last answer of the turn, written when the turn
  ends with `done`. Nothing is written to it when the turn fails, is
  cancelled or stops at its limit.
- Standard error: one line per tool call when it has finished, the text the
  model wrote before its tool calls, notices and errors:

  ```
  Let me look at the test first.
  ✓ read_file tests/parse.rs (lines 1–80) — 80 lines
  ✗ run_command cargo test parse — exit code 101
  ⊘ edit_file src/parse.rs (line 12) — denied: needs permission, and no one can answer
  clai: limit reached
  ```

  A tool line has the state (`✓` done, `✗` error, `⊘` denied or
  cancelled), the tool, its summary and a short result: the number of lines
  for `read_file`, `list_files` and `search_files`, the last line for
  `run_command`, else the first line; each cut to 120 characters. With
  `tui.symbols = "ascii"`, or `auto` and a locale that is not UTF-8, the
  marks are `+`, `x` and `-`. Notices and errors start with `clai: `.
  There is no colour and no summary line. **(assumption)**
- Questions: when standard input is a terminal and the task is not read
  from it (`-`), `clai run` asks like [`clai chat`](#clai-chat): the
  question on standard error, the answer as a line on standard input:
  `y`, `a` (then a proposal's number, `Enter` for the first, or a typed
  rule), `n`, `e` (then the message) or `c`; `y`, `n` or `c` for a limit
  question. An answer the server does not accept is shown as
  `not accepted: …` and the question is asked again. The end of standard
  input counts as "no one can answer". New file content is shown up to 40
  lines, then `… N more lines`; `v` (the pager) is not offered here.
  **(assumption)**
- Otherwise there is no one to ask: a tool call whose action is `ask` is
  denied and the model is told so, and a limit question stops the turn with
  "limit reached" on standard error and exit code `1`.
- `--yes` allows tool calls that would ask, without asking; `deny` rules and
  the workspace boundary still apply (see [`config.md`](config.md),
  "Permissions"). It does not answer limit questions.
- `--no-ask` never asks, even on a terminal: `ask` is denied as above.

| Option | Meaning |
|--------|---------|
| `--model` | Model to use. Default: `default_model`. |
| `--yes` | Allow tool calls that would ask; `deny` rules still apply. |
| `--no-ask` | Do not ask on the terminal; deny tool calls that would ask. |
| `-C DIR` | Workspace. Default: the current directory. |

`--yes` and `--no-ask` cannot be combined (exit `2`).

| Exit | When |
|------|------|
| `0` | The turn ended with `done`. |
| `1` | The turn failed (provider error), stopped at its limit, or was cancelled with the `c` answer; or `SIGTERM` / `SIGHUP`. |
| `2` | Invalid options or configuration, an unknown `--model`, a `-C` that is not a directory, an empty task. |
| `130` | `Ctrl-C`. |

`Ctrl-C`, `SIGTERM` and `SIGHUP` cancel the turn as in the TUI (see
[`agent-loop.md`](agent-loop.md), "Cancellation"): a running command is
stopped, then `clai run` exits. Further signals while it stops are
ignored, so no command is left running. **(assumption)**

## clai new

```sh
clai new [-d] [--name NAME] [--model PROVIDER/MODEL] [DIR]
```

Creates a session with workspace `DIR` (default: the current directory) and
attaches the TUI to it.

| Option | Meaning |
|--------|---------|
| `-d`, `--detach` | Do not attach; print the session name on standard output. |
| `--name` | Session name. Default: the workspace directory name, made unique. |
| `--model` | Model of the session. Default: `default_model`. |

Fails with `1` if the name is taken.

## clai attach

```sh
clai attach [NAME]
```

Attaches the TUI to session `NAME`. Without `NAME`: attaches directly if
exactly one session exists, opens the session list if there are several, and
fails with `1` (`no sessions`) if there are none.

## clai ls

```sh
clai ls [--json]
```

Lists the sessions, one per line: name, workspace, provider/model, state
(`idle`, `working`, `waiting`), age. Sessions waiting for permission are
marked. No sessions: prints nothing, exit `0`.

| Option | Meaning |
|--------|---------|
| `--json` | Print a JSON array instead of the table. **(assumption)** |

## clai kill

```sh
clai kill NAME...
clai kill --all
```

Ends the named sessions; a running turn is cancelled first. Unknown names are
reported on standard error and give exit `1`; the known ones are still ended.

## clai send

```sh
clai send NAME MESSAGE...
```

Sends `MESSAGE` to session `NAME` as if typed in the TUI and returns at once;
it does not wait for the answer. `-` reads the message from standard input.
If a turn is running, the message is queued. **(assumption)**

## clai chat

```sh
clai chat [NAME...]
clai chat --list [--json] [NAME...]
clai chat NAME CHOICE [--id ID]
```

Answers questions of sessions in the terminal, without the TUI (see
[`sessions.md`](sessions.md), "Questions"). Useful over a plain SSH
connection, in a second terminal, or from a script.

**Interactive** (`clai chat [NAME...]`): attaches as an answering client
to the named sessions, or to all sessions. It prints each open question and
each new one as it opens, reads the answer as a line on standard input, and
keeps running until `Ctrl-C` or end of input. It does not show the
conversation.

```
── clai · ~/ws/rust/clai · q17 · 12:04 ──────────────────────────────────
run_command: cargo build && ./deploy.sh staging
  in       ~/ws/rust/clai, timeout 120 s
  part 1   cargo build          allow  allow run_command(cargo build)  config.toml:28
  part 2   ./deploy.sh staging  ask    default for run_command
  model    "Build passes; deploying to staging now."
[y] allow  [a] always  [n] deny  [e] deny with message  [c] cancel turn
> a
  1  run_command(./deploy.sh staging)
  2  run_command(./deploy.sh)
rule [1]>
```

- Diffs are shown in full, with colour when standard output is a terminal;
  new file content is shown up to 40 lines, then `… N more lines, [v] view
  all`. `v` shows everything through `$PAGER` (default `less`).
  **(assumption)**
- With several sessions, questions are shown one at a time, oldest first.
- A question answered elsewhere (TUI, other `clai chat`) while shown is
  replaced by `answered in tui pts/3: allow`.
- With no open question, it prints `waiting for questions (Ctrl-C to stop)`
  on standard error.

**List** (`--list`): prints the open questions, one per line: session, id,
age, summary. `--json` prints the full questions, with all fields of
[`sessions.md`](sessions.md), "Content". Exit `0`, also when there are none.

**Script** (`clai chat NAME CHOICE`): answers the open question of
session `NAME` and exits.

| `CHOICE` | Answer |
|----------|--------|
| `allow` | Allow once. |
| `always RULE` | Allow and add `RULE` to the session; must be a rule the server accepts for this call. |
| `deny [MESSAGE...]` | Deny, with an optional message for the model. |
| `cancel` | Deny and cancel the turn. |
| `continue`, `stop` | Answer a limit question. |

| Option | Meaning |
|--------|---------|
| `--list` | List open questions and exit. |
| `--json` | With `--list`: print JSON. |
| `--id ID` | Answer only if `ID` is still the open question; protects a script from answering a newer question it has not seen. |

Exit codes: `1` if the session has no open question, the id does not match,
the choice does not fit the question, or it was already answered; the
message says which.

## clai models

```sh
clai models [--json]
```

Lists all configured providers and models, one model per line, in config
order: `PROVIDER/MODEL`, model id, context window (rounded down, `32k`,
`1M`), and the marks `default` for `default_model` and `no key` for a
provider whose `api_key_env` variable is not set. `api_key_cmd` is not run,
so such providers are never marked. Fails with `2` if the configuration is
invalid.

```
anthropic/sonnet  claude-sonnet-5-5  200k  default, no key
local/qwen        qwen3-coder:30b     32k
```

`--json` prints an array of objects with `model`, `provider`, `name`, `id`,
`context_window`, `max_output_tokens` (or `null`), `default` and `key`
(`env`, `missing`, `command` or `none`). **(assumption)**

| Option | Meaning |
|--------|---------|
| `--json` | Print a JSON array instead of the table. **(assumption)** |

## clai commit

```sh
clai commit [--staged] [--model PROVIDER/MODEL] [--hint TEXT] [DIR]
```

Prints a generated commit message for the changes below `DIR` (default: the
current directory) on standard output. Commits nothing. How changes are
collected and the message is generated: [`commit.md`](commit.md).

| Option | Meaning |
|--------|---------|
| `--staged` | Describe only the staged changes. |
| `--model` | Model to use. Default: `commit.model`, then `default_model`. |
| `--hint TEXT` | Extra instruction for the model, e.g. `--hint "fixes #12"`. |

Standard output gets only the message, ending with one newline. Exit `1` if
there is nothing to commit, `DIR` is not in a git repository, a merge or
rebase is in progress, or the model request failed.

```sh
git add -A && clai commit | git commit -F -
clai commit --staged | git commit -F -
```

## clai server

```sh
clai server             # run the server in the foreground
clai server stop        # stop the running server and end all its sessions
clai server status      # print whether a server runs, its PID and session count
```

`clai server` fails with `1` if a server is already running. `stop` asks for
confirmation on a terminal when sessions are running; `--force` skips it.
`status` exits `0` if a server runs and `1` if not. **(assumption)**

## clai completion

```sh
clai completion [bash | zsh | fish | elvish | powershell]
```

Prints the shell code that turns on Tab completion for `clai`. Without a
shell name, the shell is taken from the file name in `$SHELL`; if that is not
one of the shells above, the command fails with `2`.

Install it by loading the code on every shell start, so it always matches
the installed `clai`:

```sh
echo 'source <(clai completion bash)' >> ~/.bashrc
echo 'source <(clai completion zsh)'  >> ~/.zshrc
echo 'clai completion fish | source'  >> ~/.config/fish/config.fish
```

The code does not contain a fixed list. On every Tab the shell runs
`COMPLETE=<shell> clai -- <words>`, and `clai` prints the candidates and
exits before doing anything else. So completion knows:

- commands, options and fixed values (e.g. shell names);
- `--model`: every configured `PROVIDER/MODEL`, with its model id. It reads
  the default user and project files; a `--config` on the same command line
  is not used. If the configuration cannot be loaded, there are no
  candidates and no error. **(assumption)**
- files for `--config`, directories for `-C` and `DIR`;
- session names for `attach`, `kill`, `send` and `chat` once sessions exist.

`COMPLETE` empty or `0` is the same as not set.
