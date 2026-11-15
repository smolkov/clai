# Continue the last session of the workspace

- RFC: 0025
- Status: done
- Milestone: none; it adds an option to the commands of RFC 0002
- Spec: [`spec/command.md`](../spec/command.md), "codeagent", "codeagent attach",
  "codeagent chat", "codeagent send"; [`spec/sessions.md`](../spec/sessions.md),
  "Session lifecycle"

## Summary

`codeagent --continue` (short `-c`) attaches the TUI to the session of the
current directory that changed last, running or saved, instead of creating
a new one. Without such a session it creates one, as `codeagent` does. The same
option works for `codeagent chat` and `codeagent send`, so the last session of a
workspace can be reached without knowing its name.

## Motivation

- `codeagent` in a project always creates a new session. To go on with
  yesterday's work the user runs `codeagent ls`, finds the session of the
  workspace, and types `codeagent attach NAME`. Names are made unique
  (`project`, `project-2`, …), so the newest is not always the one with
  the plain name.
- `codeagent attach` without a name picks the only session, or opens the
  session list over all sessions of every workspace; it does not know the
  current directory.
- Other agents (Claude Code `--continue`, OpenCode `--continue`) have the
  same option; users expect it.
- The data is there: every session has its workspace (`path`) and the time
  of its last change (`updated`, RFC 0011). The choice can be made by the
  client from `list`; no new request is needed.

## Scope

In:

- `--continue` / `-c` for `codeagent`, `codeagent chat` and `codeagent send`.
- Which session is "the last one of the workspace", and what happens
  without one.
- How it combines with `--model`, `--name` and the other options.

Out:

- A `/continue` command in the TUI: the session list already shows the
  sessions of all workspaces with their age, and `/attach` completes the
  names.
- `codeagent run --continue`: `codeagent run` works without the server, in a
  session of its own that is not saved. See "Open questions".
- Continuing a session that was killed: its files are deleted.
- Forking a session (a new session with the history of an old one).

## Design

### The last session of a workspace

- The workspace is the current directory, resolved like the workspace of a
  new session (absolute, links followed). A session belongs to it when its
  `path` is that directory. Sessions of parent directories or
  subdirectories do not count. **(proposal)**
- Among them, the one with the latest `updated` wins: the last message,
  answer, model change or rename. Running and saved sessions count alike;
  a saved one is resumed by the attach. With the same `updated` (or `0`
  from an older server), the higher id wins, which is the one created
  later.
- A session that is `working` or `waiting` counts too: the TUI attaches to
  it as any client does, and the user sees the turn or the question.

### codeagent --continue

```sh
codeagent --continue [--model PROVIDER/MODEL]
codeagent -c
```

- With a session of the workspace: attaches the TUI to it. The steps of
  `codeagent` apply (configuration, terminal, server), with "attach" in the
  place of "create".
- Without one: creates a new session, as `codeagent` without the option does,
  with a note on standard error: `codeagent: no session in ~/ws/project yet;
  starting one`. **(proposal)**
- `--model` changes the model of the session it attaches to, as
  `codeagent chat NAME --model` does; for a new session it is the model of the
  new session.
- `--name` and `--continue` together are an error (exit `2`): `--name`
  names a new session. **(proposal)**
- With a task (`codeagent -c TASK...`): see "Open questions"; until decided it
  is an error (exit `2`), so that `-c` is not taken as part of the task.
- The detach line when the TUI exits is the one of `codeagent attach`.

### codeagent chat --continue and codeagent send --continue

```sh
codeagent chat --continue [--model PROVIDER/MODEL]
codeagent send --continue MESSAGE...
```

- `codeagent chat --continue` attaches to the last session of the workspace,
  or creates one as `codeagent chat --new` does, with the same note.
- `codeagent send --continue MESSAGE` sends to the last session of the
  workspace. Without one it fails with `1`
  (`no session in ~/ws/project`): a message sent into a new, empty session
  would surprise more than help. **(proposal)**
- For `codeagent chat`, `--continue` and a `NAME` together are an error (exit
  `2`). For `codeagent send`, all words after `--continue` are the message.

### Code

- `src/command/common.rs`: `last_session(sessions, workspace) ->
  Option<&SessionInfo>`, used by all three commands.
- `src/cli.rs`: `-c` / `--continue` in `TopArgs` (without a task), in
  `Chat` and in `Send`, with the conflicts above.
- `src/tui/mod.rs`: a new `Target::Continue(Request)`: list, pick, attach;
  with no session, the `new` request it carries.
- `src/command/chat/`, `src/command/send.rs`: the same choice.
- No protocol change: `list` has `path`, `updated` and `id`.

## Spec changes

- `spec/command.md`, "codeagent": `--continue`, `-c`, its steps and messages;
  "codeagent chat" and "codeagent send": `--continue`.
- `spec/sessions.md`, "Session lifecycle": "the last session of a
  workspace", for `--continue`.
- `readme.md`: `codeagent -c` in the commands and in "Sessions in the
  background".

## Testing

- `src/command/common.rs` unit tests: `last_session` picks the latest
  `updated` of the workspace; ignores other workspaces, parents and
  subdirectories; prefers the higher id on a tie and with `updated = 0`;
  counts saved and working sessions; none.
- `src/cli.rs` tests: `-c` and `--continue` parse for `codeagent`, `codeagent chat`
  and `codeagent send`; `--continue` with `--name`, with a `NAME`, and with a
  task are usage errors.
- `tests/server.rs` (process): two sessions in a workspace and one in
  another, `codeagent send --continue` goes to the newest of the workspace; a
  saved session is resumed by it; without a session it fails with `1`.
- `codeagent --continue` needs a terminal; its choice is the shared function,
  and the attach is the one of `codeagent attach`, both tested already.

## Implementation plan

1. **The choice**
   - Status: done
   - Files: `src/command/common.rs`
   - Result: `last_session` for a workspace.
   - Tests: its unit tests.

2. **codeagent --continue**
   - Status: done
   - Files: `src/cli.rs`, `src/command/new.rs`, `src/tui/mod.rs`
   - Result: `codeagent -c` attaches to the last session or starts one.
   - Tests: CLI parse tests.

3. **codeagent chat and codeagent send**
   - Status: done
   - Files: `src/command/chat/mod.rs`, `src/command/send.rs`,
     `tests/server.rs`
   - Result: `--continue` for both.
   - Tests: the process tests.

4. **Spec and readme**
   - Status: done
   - Files: `spec/command.md`, `spec/sessions.md`, `readme.md`, `todo.md`
   - Result: the option is described.
   - Tests: `ci/check.sh`, `ci/test.sh`.

## Alternatives

- **`codeagent attach` without a name picks the workspace's last session.**
  No new option, but it changes what `codeagent attach` does today (the only
  session, or the list), and the list over all workspaces is useful as it
  is.
- **The server picks (`attach { workspace, last: true }`).** One request
  instead of two, but the client has all it needs from `list`, and a
  protocol change would need a new version.
- **The session last attached, not the last changed.** Closer to "where I
  was", but the server does not record attaches, and a session that a
  `codeagent send` or another client moved on is more likely the one to go on
  with.
- **Fail without a session instead of creating one.** Clearer for
  scripts, but `codeagent -c` is typed by hand at the start of the day; a new
  session is what the user wants then. `codeagent send --continue` fails,
  because it is used from scripts.

## Open questions

All answered; see "Answers".

1. Which sessions belong to the workspace: only those of exactly the
   current directory (proposal), or also those of a parent directory
   (started at the repository root, `codeagent -c` typed in `src/`)?
2. Without a session: start one with a note (proposal), or fail with `1`?
3. `codeagent -c TASK...` and `codeagent run --continue TASK`: send the task to the
   last session and print the answer like `codeagent run`, through the server?
   Or leave it out (proposal for now: a usage error)?
4. The short option `-c`: free in `codeagent` today, but `codeagent run` has `-C`
   for the directory. Is `-c` next to `-C` too close (then only
   `--continue`), or fine (proposal)?
5. "Last" by the last change (`updated`, proposal) or by the last attach
   of a TUI?

## Answers

1. to 5. The user asked to build it, so the proposals hold, as for
   RFC 0021: only sessions of exactly the current directory; without one,
   `codeagent -c` and `codeagent chat -c` start one with a note and `codeagent send -c`
   fails with `1`; no `codeagent -c TASK` and no `codeagent run --continue` (a usage
   error); `-c` is the short option; "last" is the latest `updated`.

## Notes from building

- **`codeagent send --continue`:** clap cannot tell a session name from the
  first word of a message, so with `--continue` all words are the message
  (`codeagent send -c fix the docs`), and `NAME` is not an error there but part
  of the text. `codeagent chat -c NAME` is an error, as planned.
- **The model:** only `--model` changes the model of the session that is
  continued; `CODEAGENT_MODEL` from the environment is used, as before, only
  for a new session.
- **One second:** `updated` is in seconds, so of two sessions changed in
  the same second the one created later wins; the process test waits a
  second for this reason.
- Shared code: `command::common::{last_session, continue_session,
  last_session_name}`; `tui::Target::Continue` and
  `chat::talk::Target::Continue` call `continue_session`; `New::resume`
  is set only by `codeagent -c` (`#[arg(skip)]`).
- Tests: `last_session` (workspace, parents and subdirectories, ties,
  saved and working sessions), the CLI parse and conflicts, and a process
  test for `codeagent send -c` with three sessions, a restart and none.
