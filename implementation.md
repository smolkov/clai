# Implementation plan

How `clai` is built from the specification in [`spec/`](spec/readme.md):
architecture, crates, and the order of work in milestones. The spec decides
behaviour; this file only decides how and in which order it is built. If the
two disagree, the spec wins and this file is fixed.

Each milestone ends with a working, tested state (`cargo fmt`, `cargo clippy
--all-targets -- -D warnings`, `cargo test` pass) and updates `readme.md` and
`todo.md` (see [`agent.md`](agent.md)). Each milestone still needs its own
approved plan before work starts.

## Architecture

```
 ┌───────────── clai binary ──────────────────────────────────────────────┐
 │                                                                        │
 │  cli ──▶ client ──(unix socket, versioned JSON)──▶ server              │
 │   │        ▲                                         │                 │
 │   │      tui                                    session actors         │
 │   │                                                  │                 │
 │   └──▶ run (one-shot, in process) ──────────▶ agent loop               │
 │                                               │    │    │              │
 │                                        provider  tools  permissions    │
 │                                               │                        │
 │                                           adapters (anthropic, openai) │
 │  config ─────────────── used by all ───────────────────────────────    │
 └────────────────────────────────────────────────────────────────────────┘
```

- **One binary, one crate** at first, split into modules. A workspace with
  several crates is introduced only if build times or boundaries need it.
- **The agent loop knows no transport.** It takes user messages and
  decisions as input and emits session events as output. The server and
  the one-shot mode both drive the same loop; one-shot answers questions
  with the "no client" rules (see `spec/sessions.md`, "Background work").
- **A session is an actor:** one tokio task owns the session state (history,
  model, session rules, state `idle`/`working`/`waiting`). Clients send it
  commands over a channel; it publishes events to all attached clients. An
  error or panic inside one session task never reaches the others.
- **The TUI is only a client.** It holds view state (scroll, expanded calls,
  unsent input), never session state.

## Modules

| Module | Content | Spec |
|--------|---------|------|
| `error` | Crate error type (`thiserror`), `Result` alias. | `agent.md`, "Never crash" |
| `config` | Loading, merging user/project/env/CLI, validation with file:line, reloading, model references. | `config.md` |
| `provider` | Shared types, `Provider` trait, retries, `fake` provider for tests. | `providers.md` |
| `provider::anthropic`, `provider::openai` | Adapters: encode requests, decode SSE streams, map errors. | `providers.md` |
| `tools` | `read_file`, `list_files`, `search_files`, `edit_file`, `write_file`, `run_command`; tool specs. | `tools.md` (planned) |
| `permissions` | Tool defaults, rules, glob matching, path resolution, command splitting, session rules. | `config.md`, "Permissions" |
| `agent` | Agent loop: requests, streaming, tool calls, limits, cancellation, questions. | `agent-loop.md` (planned), `sessions.md` |
| `context` | System prompt, history size, truncation. | `context.md` (planned) |
| `session` | Session actor, state machine, message queue, event log for attach. | `sessions.md` |
| `protocol` | Client–server messages, versioning, framing. | `sessions.md`, "Server" |
| `server` | Socket, start on demand, idle exit, session registry. | `sessions.md` |
| `client` | Connect, start the server if needed, request/response and event stream. | `sessions.md` |
| `tui` | Layout, conversation, input editor, prompts, session list, model picker, commit dialog, themes. | `tui.md`, `config.md` "TUI" |
| `commit` | Collect git changes, generate message, run git. | `commit.md` |
| `cli` | Argument parsing, commands, exit codes. | `command.md` |

## Dependencies

Chosen for: pure Rust, works with a static musl build, maintained.
**(proposal, to be approved with the first milestone that uses each)**

| Purpose | Crate |
|---------|-------|
| Async runtime | `tokio` (rt-multi-thread, net, process, signal, fs, time) |
| CLI | `clap` (derive) |
| Config | `serde`, `toml` (spans for file:line errors) |
| JSON | `serde_json` |
| HTTP | `reqwest` with `rustls`, `stream`; no OpenSSL |
| SSE parsing | own small parser over the byte stream (few lines, fully tested) |
| Errors | `thiserror` |
| Logging | `tracing`, `tracing-subscriber` (to a file, never to the TUI screen) |
| Globs | `globset` |
| File search | `ignore` (respects `.gitignore`), `regex` |
| Diffs | `similar` |
| TUI | `ratatui`, `crossterm` |
| Markdown | `pulldown-cmark` |
| Syntax highlighting | `syntect` (pure-Rust regex backend) |
| Config file watching | `notify` |
| Paths | `dirs` or reading `XDG_*` directly |
| Tests | `tempfile`, `insta` (snapshots of encoded requests and TUI frames) |

## Never-crash rules in code

From the first commit:

- `#![forbid(unsafe_code)]` in `main.rs`.
- Clippy lints in `Cargo.toml` (`[lints.clippy]`), denied: `unwrap_used`,
  `expect_used`, `panic`, `indexing_slicing`, `unreachable`, `todo`,
  `unimplemented`. Tests allow them with `#[cfg(test)]` attributes.
- Every spawned task returns `Result`; the server logs task errors and
  panics (`JoinError`) and turns them into a session error, never a server
  exit.
- The TUI installs a panic hook and signal handlers that restore the
  terminal first.
- Release profile keeps `panic = "unwind"`, so a panic in a dependency is
  caught at the task boundary.

## Milestones

The order follows the dependencies: everything a session needs is built and
tested before the server and the TUI, and one-shot mode gives a usable
program early. **(proposal; answers open question 2 in `spec/readme.md` with
"one-shot first")**

### Foundation

- `Cargo.toml`: dependencies of this milestone, lints, release profile.
- `error` module, `tracing` setup to `$XDG_STATE_HOME/clai/clai.log`.
- `cli` skeleton with all commands from `command.md`; unbuilt commands
  return an "unsupported" error with exit `1`.
- `readme.md` and `todo.md` created.

Tests: argument parsing of every command, exit codes for unknown commands.

Status: done. The musl static build check moved to the Providers milestone,
where `reqwest` is added (the `x86_64-unknown-linux-musl` target is not
installed yet).

### Configuration

- Load user file, project file, environment, CLI options; precedence.
- Reject unknown keys, providers/`[tui]`/`[themes]` in project files, and
  permission loosening in project files, with file:line errors.
- Model references `provider/model` and the short form.
- API key from `api_key_env` / `api_key_cmd`, resolved on first use, never
  logged (wrapper type whose `Debug` prints `***`).
- No user file: explanation and example, exit `2`.
- `clai models` (with `--json`).

Tests: one fixture per validation error in `config.md`, merge order,
key redaction in `Debug`, `clai models` output snapshot.

### Providers

- Shared types and `Provider` trait as in `providers.md`.
- SSE parser.
- `anthropic` adapter, then `openai-compatible` adapter.
- Retry with backoff before the first event, `retry_after`, cancellation.
- `fake` provider that replays scripted events and errors.
- Static build check for `x86_64-unknown-linux-musl` with an HTTPS request.

Tests: recorded wire fixtures per adapter (text, tool calls, interleaved
calls, missing ids, broken JSON, cut stream, every HTTP error), encoding
snapshots, retry timing with tokio's paused clock. A manual test against
the real APIs, behind an ignored test that needs a key.

### Tools and permissions

- The six tools, with workspace boundary and `extra_dirs`; `run_command`
  with timeout, process group, `SIGTERM` then `SIGKILL`.
- `edit_file` as exact-text replacement; diff for display.
- Permission checking in the order of `config.md`: boundary, deny, ask,
  allow, default; session rules.
- Command splitting for `;`, `&&`, `||`, `|`, `$( )`, backticks,
  redirection; anything not split safely asks.

Tests: each tool in a temp dir; path escapes (`..`, symlinks) denied; rule
order; project-file merge; a table of command lines and their split parts,
including the `cargo test && rm -rf /` case.

Needs: `spec/tools.md` (arguments, results, output limits).

### Agent loop

- Turn: build request from context, stream events, collect tool calls,
  check permissions, ask questions, run tools, append results, repeat.
- Every `ToolCall` gets exactly one `ToolResult` (also denied, cancelled).
- Limits per turn; limit question with continue/stop.
- Cancellation at any point.
- Emits session events: text deltas, tool call states, questions, notices,
  errors, usage, turn end.
- Minimal context: system prompt and full history; `ContextTooLong` is
  reported as an error until truncation exists.

Tests: all against the `fake` provider: plain answer, tool round trip,
denied call, cancelled turn, limit reached then continue and stop, provider
error mid-stream, malformed tool arguments.

Needs: `spec/agent-loop.md` (limit config keys and defaults, system prompt
basics).

### One-shot mode

- `clai run` and `clai TASK...`: temporary in-process session, plain-text
  output (answer on stdout, tool lines on stderr), `--yes`, `-C`, exit codes,
  `Ctrl-C` exit `130`.

Tests: end-to-end with the `fake` provider selected by a test-only config
`api` value **(proposal)**, checking stdout, stderr and exit codes.

After this milestone `clai` is usable from scripts.

### Server and sessions

- `protocol`: length-prefixed or newline-delimited JSON messages with a
  version handshake **(proposal: newline-delimited JSON)**.
- `server`: socket in `$XDG_RUNTIME_DIR/clai/` with mode `0700`, start on
  demand, `clai server` foreground, idle exit after 60 s, `stop`,
  `status`.
- `session` actor: state machine of `sessions.md`, background work,
  questions shown to all clients with first answer winning, message queue,
  event log so attach replays the full history.
- Commands `clai new -d`, `ls`, `kill`, `send`.
- Configuration reload on file change.

Tests: server in a temp runtime dir; several sessions in parallel with the
`fake` provider; detach during a turn; question answered by one of two
clients; a session task that fails does not affect another; version
mismatch error.

### TUI core

- Terminal setup and restore (panic hook, signals), event loop with redraw
  limit, layout from `tui.md`.
- Conversation entries: Markdown, streaming text, tool calls collapsed and
  expanded, diffs, notices, errors.
- Input editor with history and bracketed paste.
- Permission prompt and limit prompt.
- `clai`, `clai new`, `clai attach`, detach keys, cancellation.

Tests: rendering snapshots with ratatui's `TestBackend`; key handling as
unit tests on the TUI state, without a terminal.

### TUI sessions and settings

- Session list, switching with kept view state, `Ctrl-N`/`Ctrl-P`.
- Model picker and model change of a running session.
- Command list with filtering and completion; all `/` commands.
- `[tui]` settings, themes, `NO_COLOR`, `CLAI_THEME`, ASCII symbols,
  syntax highlighting, live reload.

Tests: snapshots of list, picker, command list and each built-in theme.

### Commit

- `clai commit` and the TUI `/commit` dialog.

Needs: `spec/commit.md`, which is linked from the spec but not written.

### Context management

- Context window gauge, history truncation, model change to a smaller
  window.

Needs: `spec/context.md`.

### Release

- Static build for `x86_64-unknown-linux-musl` (and `aarch64` if wanted),
  size and start-time check, version from `Cargo.toml`.
- `readme.md`: install, configuration example, commands.

## Spec work needed before building

| Before milestone | Spec to write or decide |
|------------------|-------------------------|
| Tools and permissions | `spec/tools.md`; open question 5 (sandbox). |
| Agent loop | `spec/agent-loop.md`: limit keys and defaults, streaming and cancellation details. |
| One-shot mode | Open question 2 (one-shot first, as proposed here). |
| Server and sessions | Open questions in `spec/sessions.md`, at least persistence (1) and several sessions in one workspace (4). |
| Commit | `spec/commit.md`. |
| Context management | `spec/context.md`. |
| Release | Open question 4 (platforms). |

## Risks

| Risk | Handling |
|------|----------|
| Safe command splitting is hard. | Split conservatively; anything unusual asks. A table-driven test grows with every found case. |
| `openai-compatible` servers differ. | Adapter quirks are listed in `providers.md` and each one has a fixture. |
| TUI rendering is hard to test. | Keep TUI state separate from drawing; test state as units and frames as snapshots. |
| `syntect` and `reqwest` with musl. | Check the static build in the Providers milestone with a real HTTPS request, not at release. |
