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
| [`clai models`](#clai-models) | List configured providers and models. | [`config.md`](config.md) |
| [`clai commit`](#clai-commit) | Print a commit message for the changes in a directory. | [`commit.md`](commit.md) |
| [`clai server`](#clai-server) | Run, stop or check the background server. | [`sessions.md`](sessions.md) |

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
- Commands that use sessions (`attach`, `ls`, `kill`, `send`, `new`) start
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
- Standard output: the model's final answer as plain text (Markdown source).
  Standard error: tool calls and their results, one line each.
- Permissions: there is no one to ask, so a tool call whose action is `ask`
  is denied and the model is told so. `--yes` allows it instead; `deny`
  rules and the workspace boundary still apply (see
  [`config.md`](config.md), "Permissions").
- Limits: when the turn reaches its limit, it stops with "limit reached" on
  standard error and exit code `1`; `--yes` does not change this (see
  [`sessions.md`](sessions.md), "Background work"). **(assumption)**

| Option | Meaning |
|--------|---------|
| `--model` | Model to use. Default: `default_model`. |
| `--yes` | Allow tool calls that would ask; `deny` rules still apply. |
| `-C DIR` | Workspace. Default: the current directory. |

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

## clai models

```sh
clai models [--json]
```

Lists all configured providers and models, one model per line:
`PROVIDER/MODEL`, model id, context window, and marks for `default_model`
and a provider whose API key is missing. Fails with `2` if the configuration
is invalid.

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
