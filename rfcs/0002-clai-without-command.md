# `clai` without a command opens the TUI

- RFC: 0002
- Status: draft
- Milestone: TUI core (see [`implementation.md`](../implementation.md)),
  the entry point part of [RFC 0001](0001-tui-core.md)
- Spec: [`spec/command.md`](../spec/command.md), "clai" and "clai new";
  [`spec/tui.md`](../spec/tui.md); [`spec/sessions.md`](../spec/sessions.md),
  "Server"

## Summary

`clai` with no command and no task creates a session in the current
directory and opens the TUI attached to it. Today it parses to
`Command::New` and fails with `'new' is not implemented yet` (exit `1`).
This RFC fixes the startup order, the checks before the terminal is touched,
the exit codes and the message after leaving, so the first thing a user
types does what the readme promises.

## Motivation

`clai` alone is the main way to use the program. The spec gave it one
sentence ("same as `clai new`"), which leaves open what happens without a
terminal, in which order errors are reported, and what the user sees after
detaching. Those cases decide whether a mistake (no config, wrong model,
piped input) gives a clear message or a broken terminal.

## Behaviour

Written into `spec/command.md`, "clai", in this change:

- `clai [--model M] [--name N]` → new session in the current directory,
  TUI attached. `clai TASK...` stays one-shot mode; `--yes`, `--no-ask` and
  `-C` without a task are an error (exit `2`), as the parser already does.
- Steps, each failing before the terminal changes:
  1. Load the configuration (exit `2`).
  2. Standard input and output must be terminals (exit `2`, with a hint to
     `clai run TASK` or `clai chat`).
  3. Connect, starting the server if needed (exit `1` if it cannot start).
  4. `new` request in the current directory; note about other sessions in
     the workspace on standard error (taken name `1`, invalid name or
     unknown model `2`).
  5. Open the TUI.
- After the TUI exits: terminal restored, then
  `clai: detached from NAME; "clai attach NAME" to return` on standard
  error. **(assumption)**
- Exit `0` for detach and for a killed session; `1` for a server that is
  lost for good; `SIGTERM` / `SIGHUP` restore and exit `1`, the session keeps
  running.
- `clai new [DIR]` without `-d` follows the same steps with `DIR`.

## Design

### Parsing

No change: `Cli::into_command` already maps no command and no task to
`Command::New` with `--model` and `--name`, and `Cli::parse_args` already
rejects top-level options together with a command.

### `command::new`

`New::run` without `-d` changes from `Error::Unsupported` to:

```rust
let request = request(self.name.clone(), self.model.clone(), self.dir.as_deref())?;
tui::run(config, tui::Target::New(request))
```

`tui::run` (RFC 0001) does steps 1–5 in order:

- Step 1 loads the configuration once in the client: it is needed anyway
  for `[tui]` (symbols, mouse, `max_fps`) and gives the exit `2` messages of
  `config.md` before anything else. `client::start` loads it again only when
  it has to start the server.
- Step 2 uses `std::io::IsTerminal` on stdin and stdout, before connecting,
  so a piped `clai` never starts a server or creates a session.
- Step 4 reuses `command::new::create`, which already prints the workspace
  note; it runs before the alternate screen, so the note stays visible
  after exit.
- Step 5 creates the terminal guard only now. Every earlier error is a
  normal `Result` printed by `main`.

`clai attach NAME` uses the same `tui::run` with `Target::Attach(name)` and
skips step 4.

### Leaving

`tui::run` returns an `Exit` value (`Detached { name }`, `Ended { name,
reason }`, `Lost`). After the guard has restored the terminal, `run` prints
the one line on standard error and maps the value to the exit code. The
name is the session's current name, so a `/rename` during the TUI is
reflected.

### Errors on the terminal

No message is written while the alternate screen is active; an error after
step 5 is kept and printed after restore. A panic restores through the hook
of RFC 0001.

## Spec changes

- `spec/command.md`, "clai": options, steps, exit codes, the detach line
  (done in this change).
- `spec/command.md`, "clai new": without `-d` it follows the steps of
  `clai` (done). The sentence "Until the TUI exists, only `clai new -d` is
  built" is removed when the TUI core is built.
- `readme.md`: `clai` opens the TUI, once built.

## Testing

- Binary tests (`tests/cli.rs`, no pseudo-terminal needed):
  - `clai` with stdin from `/dev/null`: exit `2`, the terminal hint, no
    server started, no session created (checked with `clai ls` against the
    test runtime dir).
  - `clai` without a user file: the config explanation, exit `2`.
  - `clai --yes`, `clai -C dir`, `clai --no-ask` without a task: exit `2`.
  - `clai new /no/such/dir` with stdin from `/dev/null`: exit `2` for the
    directory, which is checked while building the request, before the
    terminal check. Model and name are checked by the server (step 4), so
    they need the pseudo-terminal test.
- Unit tests: `Exit` → exit code and detach line, including a renamed
  session.
- A pseudo-terminal test of the whole flow (start, `Ctrl-D`, detach line,
  session still in `clai ls`) stays in `todo.md` with the other
  pseudo-terminal tests.

## Implementation plan

Built as the first step ("Terminal") of the implementation plan of
[RFC 0001](0001-tui-core.md); the TUI shown at the end of the steps is the
empty screen of that step until the later steps fill it.

1. **Entry**
   - Status: todo
   - Files: `src/tui/mod.rs`
   - Result: `tui::run(config, Target)`: config, terminal check, connect,
     create or attach, then the guard; `Exit` value.
   - Tests: Unit tests of `Exit` → code and detach line.
2. **Commands**
   - Status: todo
   - Files: `src/command/new.rs`, `src/command/attach.rs`
   - Result: `New::run` without `-d` and `Attach::run` call `tui::run`; no
     more `Unsupported`.
   - Tests: Binary tests of the section above.
3. **Leaving**
   - Status: todo
   - Files: `src/tui/mod.rs`, `src/main.rs`
   - Result: Detach line after restore; errors kept until restore; exit
     codes.
   - Tests: Renamed session in the detach line.
4. **Docs**
   - Status: todo
   - Files: `spec/command.md`, `readme.md`, `todo.md`, `rfcs/readme.md`
   - Result: Remove "Until the TUI exists…"; readme says `clai` opens the
     TUI; RFC status `done`.
   - Tests: `cargo fmt`, `clippy`, `test`.

## Alternatives

- **Attach to an existing session of this workspace** instead of always
  creating one (like `claude --continue`). Rejected as default: it hides
  which session one talks to, and several sessions per workspace are
  allowed. It can come later as `clai attach` without a name picking the
  workspace's session first, or a `--continue` option.
- **Fall back to plain text without a terminal** (`echo task | clai` as
  `clai run -`). Rejected: a missing terminal is usually a mistake, and
  silent one-shot mode could run tools the user did not expect.
- **Delete a session that never got a message** when its creating TUI
  exits. Simpler to keep it (like `tmux`); see open questions.

## Open questions

1. Should an empty session (no message sent) be deleted when the TUI that
   created it exits, to avoid clutter in `clai ls`?
2. Should `-C DIR` also work without a task, as the workspace of the new
   session (today only `clai new DIR` does that)?
3. Is the detach line wanted, or should `clai` exit silently like `tmux
   detach` prints `[detached (from session NAME)]`?
