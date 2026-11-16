# A scratch directory for each session

- RFC: 0029
- Status: done
- Spec: [`spec/sessions.md`](../spec/sessions.md), "Session lifecycle",
  "Persistence"; [`spec/config.md`](../spec/config.md), "Permissions",
  "Extra directories", "Built-in rules";
  [`spec/tools.md`](../spec/tools.md), "Paths", "run_command";
  [`spec/agent-loop.md`](../spec/agent-loop.md), "System prompt"

## Summary

Each session gets its own directory for temporary files, outside the
workspace. The model may write, read and delete files there without asking,
and commands get it as `TMPDIR`. It holds notes, test inputs, downloaded
logs, generated scripts and command output that would otherwise go into the
workspace or into a shared `/tmp`. It is removed with the session.

## Motivation

The model often needs a place for files that are not part of the work:

- a script to try something out, a test input, an expected output to diff
  against;
- the full output of a long command, to search it instead of running the
  command again (and to keep it out of the history; see RFC 0028);
- notes for a long task.

Today it has two places, and both are bad:

- **The workspace.** Each write asks the user (default `ask` for `edit`),
  the files show up in `git status`, and the model has to remember to
  delete them. Files it forgets stay behind in the user's repository.
- **`/tmp` through `run_command`.** `/tmp` is outside the workspace, so
  `write_file` and `read_file` refuse it; only commands with
  redirections reach it, and each one asks. Names collide between sessions
  and with other programs, and other users on the machine can often read
  it.

A private directory per session, writable without asking, gives the
model a place it may use freely, keeps the workspace clean, and is
cleaned up by `codeagent` instead of by the model.

## Scope

In:

- Creating, naming, protecting and removing the directory.
- Access for the file tools and `run_command` redirections, without
  questions.
- `TMPDIR` and `CODEAGENT_TMP` for commands.
- One line in the system prompt.
- Showing the path in the status view and in `codeagent ls --json`.
- A setting to turn it off.

Out:

- Limiting what commands may touch (needs a sandbox, open question 5 in
  `spec/readme.md`): a command can already write to `/tmp`; this RFC does
  not change that.
- Keeping files from the scratch directory after the session ends, or
  copying them into the workspace by itself: the model does that with the
  file tools, as for any other file, and the user is asked as usual.
- A size limit on the directory (open question 2).

## Design

### Where

`<runtime dir>/sessions/<session id>/tmp/`, where the runtime directory is
that of the server: `$XDG_RUNTIME_DIR/codeagent`, else
`/tmp/codeagent-$UID` (see `spec/sessions.md`, "Server"). **(proposal)**

- Every directory on the way has mode `0700`, like the socket directory,
  so other users cannot read it. A directory on the way that belongs to
  another user is an error, as for the socket.
- The runtime directory is usually a `tmpfs`: fast, in memory, emptied at
  logout or reboot. That fits temporary files. The saved session itself
  lives in the state directory and is not affected.
- `codeagent run` uses the same layout with its own session id.

The path is made when the session's first turn starts, not when the
session is created, so idle sessions leave nothing behind. **(proposal)**
If it was removed in between (reboot, logout, the user), it is made again,
empty, at the start of the next turn; the model is told it may be empty
(see below).

### Access

The scratch directory is an extra directory with `write` access, added by
`codeagent`, not by the configuration (see `spec/config.md`, "Extra
directories"). On top of that, a built-in `allow` rule for the `read` and
`edit` groups covers every path inside it, so the file tools do not ask.
**(proposal)**

- The built-in `deny` rules for secrets (`.env`, `*.pem`, …) still apply
  inside it: `deny` beats `allow`, as everywhere.
- A `deny` or `ask` rule of the user that names the path still wins.
- `run_command` redirections into it (`> $TMPDIR/out.txt`) are allowed
  without asking, as the redirection target falls under the same allow
  rule. The command itself is checked as before: running `cargo test`
  still asks if it asked before.
- Paths that leave it (`../`, links pointing out) are resolved first and
  checked as the place they point to, as for all paths (see
  `spec/tools.md`, "Paths").

### Commands

`run_command` sets two variables: **(proposal)**

| Variable | Value |
|----------|-------|
| `TMPDIR` | The scratch directory, so `mktemp`, compilers, test runners and most tools put their temporary files there. |
| `CODEAGENT_TMP` | The same path, for the model to name in commands and for scripts that should not depend on `TMPDIR`. |

The command still starts in the workspace.

### The system prompt

The prompt gets one line next to the workspace path:

```
Scratch directory: /run/user/1000/codeagent/sessions/3f2a/tmp
(read and write without asking; also $TMPDIR in commands). Use it for
temporary files instead of the workspace; it may be empty when a turn
starts and is deleted with the session.
```

The path is fixed for the session, so the prompt stays the same between
requests (RFC 0028, "A stable prefix").

### Lifetime

| When | What happens |
|------|--------------|
| First turn of the session | The directory is made. |
| Each later turn | Made again if missing; its content is left alone. |
| Server stops | Kept: the session resumes with its files, unless the runtime directory was emptied. |
| `codeagent kill <id>` / session deleted in the TUI | Removed, with everything in it. |
| `codeagent run` ends | Removed, unless `--keep-tmp` was given; then its path is printed on standard error. **(proposal)** |
| Server start | Directories of sessions that no longer exist are removed. |

Removal does not follow links out of the directory.

### Showing it

- The status view (RFC 0007) has a row `scratch` with the path and its
  size.
- `codeagent ls --json` gets `"tmp": "<path>"` per session (`null` when it
  was not made yet).
- The run report (RFC 0027) lists files the model wrote there under
  `scratch`, apart from `changes`, so changes to the workspace stay easy
  to see. **(proposal)**

### Config

```toml
[session]
scratch = true   # a scratch directory per session; false: none
```

User file only. With `false`, there is no directory, no variable, no
prompt line and no allow rule: today's behaviour. **(proposal)**

## Spec changes

- `spec/sessions.md`: a section "Scratch directory": place, mode,
  lifetime; "Persistence" says that it is not part of the saved session.
- `spec/config.md`: "Built-in rules" gets the allow rule; "Extra
  directories" mentions the directory added by `codeagent`; a `[session]`
  section with `scratch`.
- `spec/tools.md`: "run_command" lists `TMPDIR` and `CODEAGENT_TMP`.
- `spec/agent-loop.md`, "System prompt": the line about the directory.
- `spec/command.md`: `codeagent run --keep-tmp`; the `tmp` field of
  `codeagent ls --json`.
- `readme.md`: the directory, the setting, `--keep-tmp`.

## Testing

- Path and mode: made at the first turn with `0700` on every level; an
  existing directory of another user is refused; made again when missing.
- Permissions: `write_file`, `edit_file`, `read_file` inside it are
  allowed without a question; `.env` inside it is still denied; a user
  `deny` rule on it wins; a path with `../` out of it is checked as
  outside; a link out of it is checked as its target.
- `run_command`: `TMPDIR` and `CODEAGENT_TMP` are set; a redirection into
  it does not ask; the command itself is checked as before.
- Lifetime: removed on kill and delete, kept on server stop, stale
  directories removed on server start, `codeagent run` removes it unless
  `--keep-tmp`; removal does not follow links.
- `scratch = false`: no directory, no variable, no prompt line, file tools
  ask or refuse as today.
- Prompt: the line is there with the path and stays the same across
  requests.

## Implementation plan

1. **Directory and lifetime**
   - Status: done
   - Files: `src/paths.rs`, `src/session/mod.rs`, `src/session/store.rs`,
     `src/server/`
   - Result: each session has its directory from its first turn; kill,
     delete and server start clean up.
   - Tests: path, mode, recreation, cleanup.

2. **Access**
   - Status: done
   - Files: `src/permissions/workspace.rs`, `src/permissions/check.rs`,
     `src/permissions/tests.rs`
   - Result: file tools and redirections use it without questions; deny
     rules still apply.
   - Tests: the permission cases above.

3. **Commands and prompt**
   - Status: done
   - Files: `src/tools/run_command.rs`, `src/agent/prompt.rs`,
     `src/agent/system_prompt.md`
   - Result: `TMPDIR`, `CODEAGENT_TMP`, the prompt line.
   - Tests: environment of a command, prompt text.

4. **`codeagent run`, status and listing**
   - Status: done
   - Files: `src/command/run.rs`, `src/command/ls.rs`, `src/text/status.rs`,
     `src/report/`, `src/cli.rs`
   - Result: `--keep-tmp`, the `scratch` row, `tmp` in JSON, scratch files
     in the report.
   - Tests: run with and without `--keep-tmp`, JSON field, report.

5. **Setting and spec**
   - Status: done
   - Files: `src/config/`, spec files above, `readme.md`
   - Result: `[session] scratch`; the spec describes the behaviour.
   - Tests: config parse and project-file rejection; `scratch = false`.

## Alternatives

- **A directory inside the workspace (`.codeagent/tmp/`).** No new
  permission rule needed beyond one path, but it sits in the user's
  repository, needs a `.gitignore` entry, and is shared by all sessions of
  the workspace. Rejected.
- **The state directory (`~/.local/state/codeagent/sessions/<id>/tmp`).**
  Survives reboots, but on disk and in the user's backups, and it mixes
  throwaway files with the saved session. Temporary files belong in the
  runtime directory.
- **Let the model use `/tmp` freely.** Shared with other programs and
  users, no cleanup, and an allow rule for all of `/tmp` reaches far more
  than the session's own files. Rejected.
- **Ask once per session for the directory.** Safer on paper, but the
  directory is private, made by `codeagent` and holds nothing of the
  user's; one more question buys nothing.

## Changes while building

- **The key.** The directory is named after the session's saved directory
  (`~/.local/state/codeagent/sessions/<key>`), not the session id, which
  a server numbers anew at each start. A session that cannot be saved uses
  `mem-<server pid>-<id>`, `codeagent run` uses `run-<pid>`.
- **Server start.** Directories whose key is not a saved session are
  removed, except `run-<pid>` of a process that still runs, so a server
  starting during a `codeagent run` leaves its files alone.
- **`$TMPDIR` in redirections.** The shell parser keeps the name of a
  variable a word starts with, so `> $TMPDIR/out.txt` and
  `> "${CODEAGENT_TMP}/x"` are checked as the path in the directory.
  That holds only when the command line names the variable nowhere else
  (`TMPDIR=/etc; … > $TMPDIR/x` asks); a path that leaves the directory
  is checked where it points.
- **Failure to make it** is a `notice`; the turn goes on without it.
- **The prompt line** is the only place the model learns the path; the
  directory is not in the "Other directories" list.
- **Showing it.** `tmp` and the `scratch` row appear only when the
  directory exists; the report section is called "Scratch files" and
  lists absolute paths.
- **Secrets.** The built-in rules deny reading, as everywhere: a `.env`
  in the directory is not read, but may be written.

## Open questions

Answered while building:

1. The runtime directory, as proposed. A session resumed after a reboot
   finds it empty; the prompt says it may be.
2. No size limit: commands could not be held to it without a sandbox.
3. No: `--keep-tmp` is not the default after a failure, so runs in a loop
   do not fill the runtime directory. Give it when the files matter.
4. Not built; `[session]` rejects unknown keys, so `scratch_dir` can be
   added later.

Asked in the draft:

1. Runtime directory (proposed: memory, emptied at reboot) or state
   directory (disk, kept)? A long session that is resumed the next morning
   finds an empty directory with the proposal.
2. A size limit: should writes fail once the directory holds, say,
   500 MiB, so a runaway command cannot fill a `tmpfs`? `run_command`
   could not be held to it without a sandbox.
3. Should `codeagent run --keep-tmp` be the default when the run fails, so
   the files can be looked at?
4. Should the user be able to point it elsewhere (`[session] scratch_dir =
   "~/scratch"`), e.g. for a large disk?
