# Commands

This file lists every command-line command of `codeagent`: syntax, options, output
and exit codes. What a command does in detail is described in the topic file
it links to.

Statements marked **(assumption)** need a decision from the user.

## Overview

| Command | Purpose | Details |
|---------|---------|---------|
| [`codeagent`](#codeagent) | New session in the current directory, attach the TUI. | [`sessions.md`](sessions.md), [`tui.md`](tui.md) |
| [`codeagent run`](#codeagent-run) | One-shot: run one task, print the result or a report, exit. | this file |
| [`codeagent new`](#codeagent-new) | New session, attach or leave it running. | [`sessions.md`](sessions.md) |
| [`codeagent attach`](#codeagent-attach) | Attach the TUI to a running session. | [`sessions.md`](sessions.md) |
| [`codeagent ls`](#codeagent-ls) | List sessions. | [`sessions.md`](sessions.md) |
| [`codeagent kill`](#codeagent-kill) | End a session. | [`sessions.md`](sessions.md) |
| [`codeagent send`](#codeagent-send) | Send a message to a session without attaching. | [`sessions.md`](sessions.md) |
| [`codeagent chat`](#codeagent-chat) | Chat with a session in the terminal, without the TUI, or answer the questions of sessions. | [`sessions.md`](sessions.md), "Questions" |
| [`codeagent models`](#codeagent-models) | List configured providers and models. | [`config.md`](config.md) |
| [`codeagent skills`](#codeagent-skills) | List the skills the current directory finds. | [`skills.md`](skills.md) |
| [`codeagent commit`](#codeagent-commit) | Print a commit message for the staged changes of a repository, or with `-a` all changes of tracked files. | [`commit.md`](commit.md) |
| [`codeagent export`](#codeagent-export), [`codeagent import`](#codeagent-import) | Write a session to a file, add one from a file. | [`sessions.md`](sessions.md), "Persistence" |
| [`codeagent server`](#codeagent-server) | Run, stop or check the background server. | [`sessions.md`](sessions.md) |
| [`codeagent completion`](#codeagent-completion) | Print the shell code for Tab completion. | this file |

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
- The workspace (`DIR`, `-C DIR`, or the current directory) must be a
  directory; it is used absolute and with links resolved, as the server
  keeps a session's (RFC 0026).
- Commands that use sessions (`attach`, `ls`, `kill`, `send`, `chat`, `new`) start
  the server if it is not running (see [`sessions.md`](sessions.md),
  "Server"). `models`, `commit` and `run` work without the server, and
  `codeagent server stop`, `codeagent server status`, `codeagent chat --list` and the
  script form of `codeagent chat` never start one.

- Options of `codeagent` itself (`--model`, `--name`, `--yes`, `-C`) and a task
  cannot be combined with a command: `codeagent --model m ls` is an error (exit
  `2`); options of a command come after its name. Global options are the
  exception.
- There is no `help` command; `codeagent help` runs the task "help". Help is
  `-h` / `--help`.

### Terminal output

Text that comes from outside `codeagent` (the model's answer, tool lines,
notices with file names, errors of a provider) may hold escape sequences:
OSC 52 writes the clipboard, `ESC [ 2K` and `\r` erase what is shown, OSC 8
plants a link. `codeagent run` and `codeagent chat` remove them before
writing (RFC 0035): escape sequences, control
characters other than newline and tab, and the bidirectional controls
U+202A–U+202E and U+2066–U+2069. `codeagent`'s own colours are added after
that. The final answer of `codeagent run` on standard output is cleaned the
same way; `--report=json` escapes them as JSON does.

Questions show such characters instead of removing them, so the user sees
that a command hides something: `␛` for `ESC`, `⟨U+202E⟩` for the others,
zero-width characters (U+200B–U+200F, U+FEFF) too (`\e` and `<U+202E>`
without Unicode), and a line `note: this question contains 2 hidden
characters, shown as ⟨…⟩`. The answer is built from the question as the
server sent it, so an "always" rule stays as proposed.

## Global options

Accepted by every command, before or after the command name.

| Option | Meaning |
|--------|---------|
| `-h`, `--help` | Show help for `codeagent` or the given command, exit `0`. |
| `-V`, `--version` | Show the version, exit `0`: `codeagent 0.2.0`, or `codeagent 0.2.0 (abc1234 2026-11-11)` with the commit and its date for a build of the pipelines (`CODEAGENT_BUILD_COMMIT`, `CODEAGENT_BUILD_DATE` at build time). |
| `--config FILE` | Use `FILE` instead of the user config file. **(assumption)** |

## Exit codes

The same for all commands, unless a command says otherwise.

| Code | Meaning |
|------|---------|
| `0`  | Success. |
| `1`  | The command ran but failed: task failed or cancelled, session not found, nothing to commit, model request failed, … |
| `2`  | Invalid arguments or configuration, or no terminal where one is needed. |
| `130` | Interrupted with `Ctrl-C` (one-shot commands only). |

## codeagent

```sh
codeagent [--model PROVIDER/MODEL] [--name NAME]
codeagent --continue [--model PROVIDER/MODEL]
```

Same as `codeagent new` in the current directory: creates a session and attaches
the TUI to it. This is the main way to use `codeagent` interactively.

`--continue` (`-c`) attaches to the last session of the current directory
instead (see [`sessions.md`](sessions.md), "The last session of a
workspace"), running or saved; a saved one is resumed. With `--model`, that
session's model is changed first, as `codeagent chat NAME --model` does;
`CODEAGENT_MODEL` does not change it. Without such a session it creates one, as
`codeagent` without the option, after the note
`codeagent: no session in ~/ws/project yet; starting one` on standard error.
`--continue` with `--name` or with a task is an error (exit `2`). The steps
below apply, with "attach" in the place of "create" in step 4.

`codeagent` without a command and without a task always opens the TUI; with a
task (`codeagent TASK...`) it is one-shot mode (see [`codeagent run`](#codeagent-run)).

| Option | Meaning |
|--------|---------|
| `--model` | Model of the new session. Default: `CODEAGENT_MODEL`, then `default_model`. |
| `--name` | Session name. Default: the workspace directory name, made unique. |
| `-c`, `--continue` | Attach to the last session of the current directory; without one, create one. |

`--yes`, `--no-ask` and `-C` belong to one-shot mode; without a task they
are an error (exit `2`).

Steps, in this order; each failure exits before the terminal is changed, so
its message is printed normally:

1. Load the configuration. No user file or an invalid one: the message of
   [`config.md`](config.md), "Validation", exit `2`.
2. Check that standard input and standard output are terminals. Otherwise
   exit `2` with `codeagent: the TUI needs a terminal; use "codeagent run TASK" or
   "codeagent chat"`.
3. Connect to the server, starting it if needed (see
   [`sessions.md`](sessions.md), "Server"). If it cannot start: exit `1`.
4. Create the session in the current directory. A taken `--name`: exit `1`;
   an invalid name or unknown `--model`: exit `2`. If other sessions work
   in the same workspace, the note of [`codeagent new`](#codeagent-new) is printed on
   standard error.
5. Open the TUI attached to the new session (see [`tui.md`](tui.md)).

The session is created even if the user leaves without sending a message;
it stays in `codeagent ls` like any other session. **(assumption)**

When the TUI exits, the terminal is restored first; then one line on
standard error says how to return, e.g.
`codeagent: detached from project; "codeagent attach project" to return`. **(assumption)**

| Exit | When |
|------|------|
| `0` | Detached (`Ctrl-D`, `Ctrl-C` twice, `/detach`, `/quit`), or the session ended (killed). |
| `1` | The server could not be started or the connection was lost for good; a taken name; `SIGTERM` or `SIGHUP` (the session keeps running). |
| `2` | Invalid options or configuration, no terminal, an invalid name, an unknown model. |

`SIGTERM` and `SIGHUP` restore the terminal and exit with `1`; the session
keeps running on the server.

## codeagent run

```sh
codeagent run [--model PROVIDER/MODEL] [--yes | --no-ask] [-C DIR] [--skill NAME]...
         [--report[=text|json]] [--keep-tmp] [TASK...]
codeagent TASK...
```

Runs one task without the TUI and exits when it is done.

- `codeagent TASK...` is a short form of `codeagent run TASK...`. It is used when the
  first argument is not a command name; `codeagent run` is needed for a task that
  starts with a command name (e.g. `codeagent run "commit the docs"`).
- `TASK...` words are joined with spaces. `-` reads the task from standard
  input, and so does no task when standard input is not a terminal (`echo
  "fix the build" | codeagent run`). No task on a terminal is a usage error:
  `codeagent: no task: give it as words or on standard input`. With words,
  standard input is not read. A task on standard input may have at most
  1 MiB (`the task is longer than 1 MiB`) and must be UTF-8; both are
  usage errors (RFC 0027).
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
  codeagent: limit reached
  ```

  A tool line has the state (`✓` done, `✗` error, `⊘` denied or
  cancelled), the tool, its summary and a short result: the number of lines
  for `read_file`, `list_files` and `search_files`, the last line for
  `run_command`, else the first line; each cut to 120 characters. With
  `tui.symbols = "ascii"`, or `auto` and a locale that is not UTF-8, the
  marks are `+`, `x` and `-`. Notices and errors start with `codeagent: `.
  There is no colour and no summary line. **(assumption)**
- Questions: when standard input is a terminal and the task is not read
  from it (`-`), `codeagent run` asks like [`codeagent chat`](#codeagent-chat): the
  question on standard error, the answer as a line on standard input:
  `y`, `a` (then a proposal's number, `Enter` for the first, or a typed
  rule), `n`, `e` (then the message) or `c`; `y`, `n` or `c` for a limit
  question. A question from the model (`ask_user`, see
  [`tools.md`](tools.md), "ask_user") is asked one question after
  another, with numbered options (see "Questions from the model" below).
  An answer the server does not accept is shown as
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
- Language servers (see [`tools.md`](tools.md), "lsp"): the start question
  is asked like other questions, with `y`, `n`, `e` or `c`; `--yes` starts
  servers without asking. Where no one can answer, the `lsp` tool is left
  out unless `[lsp] start = "allow"`. The servers run in the `codeagent run`
  process and stop when the turn ends.

| Option | Meaning |
|--------|---------|
| `--model` | Model to use. Default: `default_model`. |
| `--yes` | Allow tool calls that would ask; `deny` rules still apply. |
| `--no-ask` | Do not ask on the terminal; deny tool calls that would ask. |
| `-C DIR` | Workspace. Default: the current directory. |
| `--skill NAME` | Turn on skill `NAME` for the task; repeatable, in the order given (see [`skills.md`](skills.md)). Not in the short form `codeagent TASK...`. |
| `--keep-tmp` | Keep the run's scratch directory (see [`sessions.md`](sessions.md), "Scratch directory") and print `codeagent: scratch directory kept: PATH` on standard error. Without it the directory is removed when the run ends, also after a failure or a signal. Not in the short form. |
| `--report` | Print a report instead of the last answer, as Markdown; `--report=json` prints it as one JSON object, `--report=text` is the same as `--report`. The value needs the `=`: in `codeagent run --report json`, `json` is a word of the task. Not in the short form. See [Reports](#reports). |

`--yes` and `--no-ask` cannot be combined (exit `2`).

| Exit | When |
|------|------|
| `0` | The turn ended with `done`. |
| `1` | The turn failed (provider error), stopped at its limit, or was cancelled with the `c` answer; or `SIGTERM` / `SIGHUP`. |
| `2` | Invalid options or configuration, an unknown `--model` or `--skill` (the message lists the found skills), a `-C` that is not a directory, an empty task, no task on a terminal, a task on standard input longer than 1 MiB or not UTF-8, a `--report` value other than `text` or `json`. |
| `130` | `Ctrl-C`. |

`Ctrl-C`, `SIGTERM` and `SIGHUP` cancel the turn as in the TUI (see
[`agent-loop.md`](agent-loop.md), "Cancellation"): a running command is
stopped, then `codeagent run` exits. Further signals while it stops are
ignored, so no command is left running. **(assumption)**

### Reports

`codeagent run --report` (RFC 0027) works on the task as an issue and prints a
report: what the model handed in with the [`report`](tools.md#report)
tool, and what `codeagent` saw itself in the events of the run.

```sh
codeagent run --report "analyse this repository for security problems"
cat issue.md | codeagent run --report
codeagent run --report=json -C ~/ws/api < issue.md > report.json
```

- The task goes to the model inside a short fixed frame
  (`src/report/work.md`): look at the code with the tools, make and check
  changes when the task asks for them, decide yourself when a question is
  not answered, call `report` once at the end, then end with a short
  answer. The task is between `<task>` and `</task>`.
- The model gets the `report` tool; without `--report` it does not.
- When the turn ends with `done` and the model gave no report, `codeagent run`
  sends one more message: `Call the report tool now with the result of
  your work on the task; use no other tools.` Without a report after that,
  the report has the status `unknown` and the last answer of the work turn
  as its summary, and `codeagent: the model gave no report; its status is
  unknown` goes to standard error.
- Questions are answered as without `--report`. With the default
  permissions, a run without a terminal and without `--yes` reads, lists
  and searches, but edits nothing and runs no command: the safe way for an
  audit.
- Standard error gets what it gets without `--report`. Standard output
  gets only the report.

What the report holds:

| Field | From | Meaning |
|-------|------|---------|
| `version` | `codeagent` | `1`. Fields may be added within a version; renaming or removing one needs a new version. |
| `workspace`, `model` | `codeagent` | Absolute workspace; `provider/model`. |
| `outcome` | `codeagent` | How the work turn ended: `done`, `limit` or `failed`. |
| `status` | model | `solved`, `partly_solved`, `not_solved`, or `unknown` without a report. |
| `summary` | model | What was done and the outcome. Without a report: the last answer, or the error of a failed run. |
| `findings` | model | `severity` (`critical`, `high`, `medium`, `low`, `info`), `title`, `file`, `line`, `detail`. |
| `follow_ups` | model | What is left to do. |
| `changes` | `codeagent` | `edit_file` and `write_file` calls that were done: `path` and `kind` (`edited`, `written`), each path once, in order. Files in the scratch directory are not changes. |
| `scratch` | `codeagent` | The same for files in the scratch directory (RFC 0029), with absolute paths; text heading "Scratch files". |
| `commands` | `codeagent` | `run_command` calls that ran: `command` and `exit_code` (`null` after a timeout or a signal). |
| `files_read` | `codeagent` | Paths of `read_file` calls that were done, each once, in order. |
| `tool_calls` | `codeagent` | Every tool call but `report`, in order: `tool`, `summary`, `state` (`done`, `error`, `denied`, `cancelled`) and `result`, as on the tool line. |
| `counts` | `codeagent` | The calls by state: `done`, `error`, `denied`, `cancelled`. |
| `usage` | `codeagent` | `input_tokens`, `output_tokens` and `requests` of all turns of the run. |
| `secrets_hidden` | `codeagent` | Secrets hidden from the model in the task and the tool results (see [`agent-loop.md`](agent-loop.md), "Secrets"); the text says `2 secrets were hidden from the model.` when there are any. |
| `duration` | `codeagent` | Seconds. |

Paths are relative to the workspace when inside it.

**JSON** (`--report=json`): one object with these fields, indented, then
a newline.

**Text** (`--report`): Markdown, without colour. Empty sections are left
out:

```markdown
# Partly solved

~/ws/rust/codeagent · anthropic/sonnet · 3 min · 45.1k in, 3.2k out

Found two problems; fixed the path check, the other needs a decision.

## Findings

- **high** Path check follows links — `src/tools/paths.rs:41`
  A link inside the workspace can point outside it; resolve it first.

## Changes

- edited `src/tools/paths.rs`

## Follow-ups

- Decide whether extra_dirs may hold links.

## Tool calls

- ✓ read_file src/tools/paths.rs (lines 1–120) — 120 lines
- ✓ edit_file src/tools/paths.rs (line 41) — 1 change
```

After a limit the line `The run stopped at its limit.` follows the
second line, after a failed run `The run failed.`. With `tui.symbols =
"ascii"`, or `auto` and a locale that is not UTF-8, the marks are those of
the tool lines and `·` and `—` are `-`.

| The work turn | Standard output | Exit |
|---------------|-----------------|------|
| `done` | the report | `0`, whatever the `status` |
| `limit` | the report, `outcome` `limit` | `1` |
| `failed` | the report, `outcome` `failed`, status `unknown` | `1` |
| cancelled (`c`, `Ctrl-C`, a signal) | nothing | `1`, `130` after `Ctrl-C` |

## codeagent new

```sh
codeagent new [-d] [--name NAME] [--model PROVIDER/MODEL] [--skill NAME]... [DIR]
```

Creates a session with workspace `DIR` (default: the current directory) and
attaches the TUI to it. Without `-d` it follows the steps, messages and exit
codes of [`codeagent`](#codeagent), with `DIR` as the workspace.

| Option | Meaning |
|--------|---------|
| `-d`, `--detach` | Do not attach; print the session name on standard output. |
| `--name` | Session name. Default: the workspace directory name, made unique. |
| `--model` | Model of the session. Default: `CODEAGENT_MODEL`, then `default_model`. |
| `--skill NAME` | Turn on skill `NAME` in the session; repeatable (see [`skills.md`](skills.md)). |

Fails with `1` if the name is taken, and with `2` for an invalid name, an
unknown model, an unknown skill (no session is created) or a `DIR` that is
not a directory. If other sessions work in
the same workspace, a note on standard error names them.

## codeagent attach

```sh
codeagent attach [NAME]
```

Attaches the TUI to session `NAME`. Without `NAME`: attaches directly if
exactly one session exists, and fails with `1` (`no sessions`) if there are
none. With several, the TUI opens on the session list, over the first session
of the list; `Esc` there exits `0` with the detach line of that session.

An unknown `NAME` fails with `1`. Configuration and terminal are checked as
for [`codeagent`](#codeagent), before anything else.

## codeagent ls

```sh
codeagent ls [--json]
```

Lists the sessions, running and saved, one per line: name, workspace,
provider/model, state (`idle`, `working`, `waiting`, or `saved` for a
session on disk that is not loaded), age. A waiting session ends with `? `
and the question's summary. No sessions: prints nothing, exit `0`.

```
codeagent    ~/ws/rust/codeagent  anthropic/sonnet  waiting  12m  ? run_command: cargo test parse
docs    ~/ws/docs       local/qwen        idle     2h
old     ~/ws/old        local/qwen        saved    3d
```

| Option | Meaning |
|--------|---------|
| `--json` | Print a JSON array instead of the table: per session `name`, `workspace` (absolute), `model`, `state`, `created` (seconds since 1970), `queued` (messages waiting for the turn), `question` (`id`, `kind`, `summary`, or `null`), `input_tokens`, `output_tokens` and `cache_read_tokens` (used so far, see [`tui.md`](tui.md), "Token counters"), and `tmp`, the scratch directory (see [`sessions.md`](sessions.md), "Scratch directory"), or `null` when it does not exist. |

## codeagent kill

```sh
codeagent kill NAME...
codeagent kill --all
```

Ends the named sessions and deletes their saved files and scratch
directories; a running turn is
cancelled first. Saved sessions are deleted without resuming them. Unknown
names are reported on standard error and give exit `1`; the known ones are
still ended.

## codeagent export

```sh
codeagent export NAME FILE
```

Writes session `NAME`, running or saved, to `FILE` as one JSON file with
mode `0600` (replaced if it exists): its settings, history and event log,
as saved (see [`sessions.md`](sessions.md), "Persistence"). The session goes
on. Exit `1` if there is no such session or the file cannot be written.

## codeagent import

```sh
codeagent import FILE [--name NAME]
```

Adds the session in `FILE`, written by `codeagent export`, as a saved session
and prints its name: `--name`, or the name in the file made unique (`-2`,
…). It is resumed by the first command that names it. Exit `1` if the
file cannot be read, is of a newer format, or `--name` is taken; `2` for
an invalid name.

Session rules in the file are not imported (RFC 0035); each one is named
on standard error: `codeagent: left out the session rule allow
run_command(cargo test) (rules are not imported)`. They can be granted
again with "always".

## codeagent send

```sh
codeagent send NAME MESSAGE...
codeagent send --continue MESSAGE...
```

Sends `MESSAGE` to session `NAME` as if typed in the TUI and returns at once;
it does not wait for the answer. `-` reads the message from standard input.
If a turn is running, the message is queued. An empty message exits `2`, an
unknown session `1`.

`--continue` (`-c`) sends to the last session of the current directory (see
[`sessions.md`](sessions.md), "The last session of a workspace"); all words
are then the message. A saved session is resumed. Without such a session it
fails with `1` (`codeagent: no session in ~/ws/project`) and creates none: a
message for an empty new session is more likely a mistake of a script.

## codeagent chat

```sh
codeagent chat [NAME]
codeagent chat [--model PROVIDER/MODEL] NAME
codeagent chat --new [--name NAME] [--model PROVIDER/MODEL] [DIR]
codeagent chat --continue [--model PROVIDER/MODEL]
codeagent chat --questions [NAME...]
codeagent chat --list [--json] [NAME...]
codeagent chat NAME CHOICE [--id ID]
```

Talks to sessions in the terminal with lines of text, without the TUI:
a conversation with one session, or only its questions (see
[`sessions.md`](sessions.md), "Questions"). Useful over a plain SSH
connection, in a second terminal, or from a script.

**Conversation** (`codeagent chat [NAME]`): attaches to session `NAME` as an
answering client named `codeagent chat`. Without `NAME`, or with `--new`, it
first creates a session like `codeagent new -d` (workspace `DIR` or the current
directory, `--name`, `--model` or `CODEAGENT_MODEL`), then attaches. With
`--continue` (`-c`) it attaches to the last session of the current
directory, as `codeagent --continue` does, or creates one with the same note;
`--continue` with a `NAME`, `--new`, `--name`, `--questions`, `--list` or
`--id` is an error (exit `2`).

```
codeagent: attached to codeagent (~/ws/rust/codeagent, anthropic/sonnet); /help lists commands, Ctrl-D detaches
> fix the failing test in tests/parse.rs
Let me look at the test first.
✓ read_file tests/parse.rs (lines 1–80) — 80 lines
── codeagent · ~/ws/rust/codeagent · q4 ──
run_command: cargo test parse
  in       ~/ws/rust/codeagent, timeout 120 s
  part 1   cargo test parse  ask    default for run_command
[y] allow  [a] always  [n] deny  [e] deny with message  [c] cancel turn
? y
✓ run_command cargo test parse — test result: ok. 12 passed
The test passes now.
>
```

- On attach it shows the session's history first: user messages after
  `> `, model text, tool lines, notices and errors. Then it shows what
  happens as it happens; model text appears as it streams.
- The prompt `> ` shows that the session waits for a message. A line typed
  is sent as a message; while a turn runs it is queued by the session. The
  user's own messages are not shown again; messages from other clients are.
- Tool lines, notices and errors are those of `codeagent run`, on standard
  output, coloured when it is a terminal and `NO_COLOR` is not set.
- A question of the session (permission or limit) is shown like below, in
  "Questions", with the prompt `? `; the next line answers it, with the
  keys of `codeagent run` and `v` for the pager. An answer the server does not
  accept is shown as `not accepted: …` and asked again. A question answered
  elsewhere closes with `answered in tui pts/3: allow`.
- Commands: `/cancel` cancels the running turn, `/model [PROVIDER/MODEL]`
  shows or changes the model (`--default` also saves it, or with no model the
  current one, as `default_model`, as in the TUI), `/usage` prints the tokens used
  (`tokens: in 45.1k, out 3.2k; context 12.3k`), `/status [NAME]` prints the
  status and statistics of the attached session or of session `NAME` as the
  TUI's status box shows them (see [`tui.md`](tui.md), "Status box"), as text,
  `/stats [7d|30d|all]` prints the statistics over all sessions of the TUI's
  Global tab as text: Totals, Models, and one line per day with its tokens and
  a bar (see [`tui.md`](tui.md), "Global tab"),
  `/lsp` lists the language servers of the server (name, root, state, the
  sessions that use them, and for a failed one its last stderr line),
  `/lsp restart NAME` and `/lsp stop NAME` restart or stop one (see
  [`tui.md`](tui.md), "Language servers"),
  `/skills` prints the skills found for the session's workspace, one per
  line: `*` before an active one, its name, its directory, and its
  description, `shadowed by …`, `changed on disk; …` or `not found any
  more`, then the warnings; `/skills NAME` turns that skill on or off, and
  the change prints as `codeagent: skills: + NAME` (see [`skills.md`](skills.md)),
  `/detach` leaves, `/kill` ends the session and leaves, `/help` lists them.
  A `retry` event prints `codeagent: retrying (1/3): …`.
- Sessions: `/sessions` prints the table of `codeagent ls`, the attached session
  marked `*`. `/attach NAME` attaches to another session and shows its history.
  `/new [NAME] [PROVIDER/MODEL]` creates a session in the attached session's
  workspace, with the note of `codeagent new`, and attaches to it; a word with a `/`
  is the model, and without one it is `default_model`. `/models` lists the
  server's models as `codeagent models` does. The session left keeps running, and a
  question open there stays open for other clients; the line on leaving names
  the session attached last.
- `Ctrl-C` cancels the running turn. With no running turn it prints a hint;
  a second `Ctrl-C` within 1 s detaches.
- `Ctrl-D` (end of input) detaches; the session keeps running and `codeagent
  chat NAME` returns to it. When the session ends, `codeagent chat` ends too.
  Exit `0`; `1` if `NAME` is not a session; `2` for more than one `NAME`.

**Questions** (`codeagent chat --questions [NAME...]`): attaches as an answering client
(named `codeagent chat`) to the named sessions, or to all sessions, including
ones created later. It prints each open question and each new one as it
opens, on standard output, reads the answer as a line on standard input,
and keeps running until `Ctrl-C` or end of input (exit `0`). It does not
show the conversation. The keys are those of `codeagent run`; an answer the
server does not accept is shown as `not accepted: …` and asked again.

```
── codeagent · ~/ws/rust/codeagent · q17 · 12:04 ──────────────────────────────────
run_command: cargo build && ./deploy.sh staging
  in       ~/ws/rust/codeagent, timeout 120 s
  part 1   cargo build          allow  allow run_command(cargo build)  config.toml:28
  part 2   ./deploy.sh staging  ask    default for run_command
  model    "Build passes; deploying to staging now."
[y] allow  [a] always  [n] deny  [e] deny with message  [c] cancel turn
> a
  1  run_command(./deploy.sh staging)
  2  run_command(./deploy.sh)
rule [1]>
```

- Diffs are shown in full, with colour when standard output is a terminal
  and `NO_COLOR` is not set; new file content is shown up to 40 lines,
  then `… N more lines, [v] view all`. `v` shows everything through
  `$PAGER` (default `less`), then asks again.
- With several sessions, questions are shown one at a time, oldest first.
- A question answered elsewhere (TUI, other `codeagent chat`) while shown is
  replaced by `answered in tui pts/3: allow`.
- With no open question, it prints `waiting for questions (Ctrl-C to stop)`
  on standard error.

**Questions from the model** (`ask_user`), in the conversation, with
`--questions` and in `codeagent run`, one question after another:

```
── codeagent · ~/ws/rust/codeagent · q18 ──
  Both work with the current code; the choice decides the migration.
? Question from the model (1/2): Which database should the cache use?
  1) SQLite (recommended) — One file next to the binary, no server.
  2) Postgres — Shared with the API; needs the server running.
Answer: a number, or text of your own; "s" skips, "c" cancels the turn
>
```

- A number chooses an option; with `multiple`, several: `1,3` or `1 3`.
  Text after the numbers and a `:` is added (`1,3: keep the old format`).
- A line that is not numbers is a typed answer. `s` declines, `c` cancels
  the turn. A number out of range, or several without `multiple`, asks
  again.
- After the last of several questions the answers are listed and
  `send [y/n]>` asks; `n` starts again at the first question.

**List** (`--list`): prints the open questions, one per line: session, id,
age, summary. `--json` prints the full questions, with all fields of
[`sessions.md`](sessions.md), "Content". Exit `0`, also when there are none.

**Script** (`codeagent chat NAME CHOICE`): answers the open question of
session `NAME` and exits. The form is chosen by the second word: if it is
one of the choices below, this is the script form, otherwise the words are
session names.

| `CHOICE` | Answer |
|----------|--------|
| `allow` | Allow once. |
| `always RULE` | Allow and add `RULE` to the session; must be a rule the server accepts for this call. The words after `always` are joined with spaces. |
| `deny [MESSAGE...]` | Deny, with an optional message for the model. |
| `cancel` | Deny and cancel the turn. |
| `continue`, `stop` | Answer a limit question. |
| `decline` | Decline a question from the model. |

| Option | Meaning |
|--------|---------|
| `--new` | Start a new session and chat with it; takes `DIR` instead of `NAME`. |
| `--name` | With `--new`: as for `codeagent new`. |
| `--model` | The model of a new session; with `NAME`, switch the session to it first. This also resumes a saved session whose model is no longer configured. |
| `--questions` | Only answer questions, of the named sessions or all, without their conversations. |
| `--list` | List open questions and exit. |
| `--json` | With `--list`: print JSON. |
| `--id ID` | Answer only if `ID` is still the open question; protects a script from answering a newer question it has not seen. |

Exit codes: `1` if the session has no open question, the id does not match,
the choice does not fit the question, or it was already answered; the
message says which. `2` for a choice that lacks or has extra words, and
for `--id` without `NAME CHOICE`.

## codeagent models

```sh
codeagent models [--json | --set-default PROVIDER/MODEL]
```

Lists all configured providers and models, one model per line, in config
order: `PROVIDER/MODEL`, model id, context window (rounded down, `32k`,
`1M`), and the marks `default` for `default_model` (`default (CODEAGENT_MODEL)` or
`default (project)` when one of those sets it) and `no key` for a
provider whose `api_key_env` variable is not set. `api_key_cmd` is not run,
so such providers are never marked. Fails with `2` if the configuration is
invalid.

```
anthropic/sonnet  claude-sonnet-5-5  200k  default, no key
local/qwen        qwen3-coder:30b     32k
```

`--json` prints an array of objects with `model`, `provider`, `name`, `id`,
`context_window`, `max_output_tokens` (or `null`), `default`, `default_from`
(`user`, `project` or `env` for the default model, `null` for the others) and
`key` (`config`, `env`, `missing`, `command` or `none`). **(assumption)**

`--set-default MODEL` saves `MODEL` (also the short form) as `default_model`
in the user file, as `config.md`, "Writing the user file", says, and prints
`default model: local/qwen (saved in ~/.config/codeagent/config.toml)`. When
`CODEAGENT_MODEL` or the project file of the current directory still sets another
default, a note on standard error says so. An unknown model fails with `2`
and lists the models; the file is not changed. It needs no server.

| Option | Meaning |
|--------|---------|
| `--json` | Print a JSON array instead of the table. **(assumption)** |
| `--set-default MODEL` | Save `MODEL` as `default_model` in the user file. |

## codeagent skills

```sh
codeagent skills [--json]
```

Lists the skills the current directory would find as a workspace, with the
user file and the project file of that directory, in order of priority (see
[`skills.md`](skills.md)): name, source (`config`, `project`, `user`),
directory, and the description, or `shadowed by DIR` for a skill another one
of the same name wins over. Paths in the workspace are relative to it, those
in the home directory start with `~/`. Each skipped file and missing
configured directory is a `codeagent: warning: …` line on standard error. With no
skills, it prints the directories searched. It needs no server. Exit `0`;
`2` if the configuration is invalid.

```
release-check  config   ~/team/release-check  Steps before a release.
pdf            project  .agents/skills/pdf    Project PDF.
pdf            user     ~/.claude/skills/pdf  shadowed by .agents/skills/pdf
```

`--json` prints an object: `skills` (each with `name`, `description`,
`dir`, `source`, `shadowed_by` or `null`), `dirs` (each with `path` and
`source`) and `warnings`, with absolute paths.

## codeagent commit

```sh
codeagent commit [-a] [--model PROVIDER/MODEL] [--hint TEXT] [DIR]
```

Prints a generated commit message for the changes staged in the git
repository that holds `DIR` (default: the current directory) on standard
output. Commits nothing. Only staged changes are described, all of them,
also those outside `DIR`; with `-a`, also the unstaged changes of tracked
files, as `git commit -a` commits them. The project's commit rules come from its agent file
(`AGENTS.md`, `AGENT.md` or `agent.md` at the repository root). How changes
are collected and the message is generated: [`commit.md`](commit.md).

| Option | Meaning |
|--------|---------|
| `-a`, `--all` | Describe all changes of tracked files, staged or not, as `git commit -a` commits them. Untracked files are left out. |
| `--model` | Model to use. Default: `commit.model`, then `default_model`. |
| `--hint TEXT` | Extra instruction for the model, e.g. `--hint "fixes #12"`. |

Standard output gets only the message, ending with one newline. Standard
error shows a progress line when it is a terminal. Exit `1` if nothing is
staged (with `-a`: nothing changed), `DIR` is not in a git repository, a merge, rebase, cherry-pick or
revert is in progress, or the model request failed; `130` after `Ctrl-C`.
All errors: [`commit.md`](commit.md), "Errors".

```sh
git add -A && codeagent commit | git commit -F -
git add src && codeagent commit --hint "fixes #12" | git commit -F -
codeagent commit -a | git commit -a -F -
```

## codeagent server

```sh
codeagent server             # run the server in the foreground
codeagent server stop        # stop the running server; its sessions stay saved
codeagent server status      # print whether a server runs, its PID and session count
```

`codeagent server` fails with `1` if a server is already running, and with `2`
if the configuration is invalid. `stop` keeps every session saved; it asks
for confirmation on a terminal when sessions are working or waiting for an
answer (their turns are interrupted); `--force` skips it. It waits until
the server has stopped its sessions, and fails with `1` if no server runs.

`status` prints `running, pid 4242, 3 sessions (2 running, 1 busy), version
0.1.0` and exits `0`, or prints `not running` and exits `1`. Busy sessions
are working or waiting for an answer.

## codeagent completion

```sh
codeagent completion [bash | zsh | fish | elvish | powershell]
```

Prints the shell code that turns on Tab completion for `codeagent`. Without a
shell name, the shell is taken from the file name in `$SHELL`; if that is not
one of the shells above, the command fails with `2`.

Install it by loading the code on every shell start, so it always matches
the installed `codeagent`:

```sh
echo 'source <(codeagent completion bash)' >> ~/.bashrc
echo 'source <(codeagent completion zsh)'  >> ~/.zshrc
echo 'codeagent completion fish | source'  >> ~/.config/fish/config.fish
```

The code does not contain a fixed list. On every Tab the shell runs
`COMPLETE=<shell> codeagent -- <words>`, and `codeagent` prints the candidates and
exits before doing anything else. So completion knows:

- commands, options and fixed values (e.g. shell names);
- `--model`: every configured `PROVIDER/MODEL`, with its model id. It reads
  the default user and project files; a `--config` on the same command line
  is not used. If the configuration cannot be loaded, there are no
  candidates and no error. **(assumption)**
- files for `--config`, directories for `-C` and `DIR`;
- session names for `attach`, `kill`, `send` and `chat`, described by
  their workspace. They come from a running server; completion never starts
  one and gives up after 1 s.

`COMPLETE` empty or `0` is the same as not set.
