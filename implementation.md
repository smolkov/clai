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

The architecture (roles, components, layers, protocol, data flow) is
specified in [`spec/architecture.md`](spec/architecture.md). Below: how it
maps to the code.

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
  on the terminal, or with the "no client" rules (see `spec/sessions.md`,
  "Background work").
- **Questions are data, not UI.** A question carries its full details
  (diff, command parts, reason, proposals) as a protocol message; the TUI,
  `clai chat` and a later web UI only render it and send an answer that
  the server checks (see `spec/sessions.md`, "Questions").
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
| `tools` | `read_file`, `list_files`, `search_files`, `edit_file`, `write_file`, `run_command`; tool specs. | `tools.md` |
| `permissions` | Tool defaults, rules, glob matching, path resolution, command splitting, session rules. | `config.md`, "Permissions" |
| `agent` | Agent loop: requests, streaming, tool calls, limits, cancellation, questions. | `agent-loop.md`, `sessions.md` |
| `context` | System prompt, history size, truncation. | `context.md` (planned) |
| `session` | Session actor, state machine, message queue, event log for attach. | `sessions.md` |
| `protocol` | Client–server messages, versioning, framing. | `sessions.md`, "Server" |
| `server` | Socket, start on demand, idle exit, session registry. | `sessions.md` |
| `client` | Connect, start the server if needed, request/response and event stream. | `sessions.md` |
| `tui` | Layout, conversation, input editor, prompts, session list, model picker, commit dialog, themes. | `tui.md`, `config.md` "TUI" |
| `commit` | Collect git changes, generate message, run git. | `commit.md` |
| `cli` | `Cli`, the `Command` enum (one struct per variant), parsing, `Cli::into_command`. | `command.md` |
| `complete` | Value completers for Tab completion (`--model`, later session names). | `command.md`, "clai completion" |
| `command` | One file per command (`command/models.rs`, …) with its argument struct and `run`; `impl Command { name, run }` in `command/mod.rs`. Commands not built yet return "not implemented". | `command.md` |

## Dependencies

Chosen for: pure Rust, works with a static musl build, maintained.
**(proposal, to be approved with the first milestone that uses each)**

| Purpose | Crate |
|---------|-------|
| Async runtime | `tokio` (rt-multi-thread, net, process, signal, fs, time), `tokio-util` (cancellation), `futures-util` (streams) |
| CLI | `clap` (derive) |
| Config | `serde`, `toml` (spans for file:line errors) |
| JSON | `serde_json` |
| HTTP | `reqwest` with `rustls` (aws-lc-rs) and `webpki-root-certs`; no OpenSSL |
| SSE parsing | own small parser over the byte stream (few lines, fully tested) |
| Errors | `thiserror` |
| Logging | `tracing`, `tracing-subscriber` (to a file, never to the TUI screen) |
| Globs | `globset` |
| File search | `ignore` (respects `.gitignore`), `regex` |
| Diffs | `similar` |
| TUI | `ratatui`, `crossterm` |
| Markdown | `pulldown-cmark` |
| Syntax highlighting | `syntect` (pure-Rust regex backend) |
| Config file watching | none: the server polls the file's time and size |
| Paths | `dirs` or reading `XDG_*` directly |
| Shell completion | `clap_complete` with `unstable-dynamic` |
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
- API key from `api_key` / `api_key_env` / `api_key_cmd`, resolved on first use, never
  logged (wrapper type whose `Debug` prints `***`).
- No user file: explanation and example, exit `2`.
- `clai models` (with `--json`).

Tests: one fixture per validation error in `config.md`, merge order,
key redaction in `Debug`, `clai models` output snapshot.

Status: done. Reloading moved to Server and sessions; the glob check of rule
patterns to Tools and permissions. The crate is now a library (`src/lib.rs`)
with a thin `main.rs`.

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

Status: done, except the musl build check (target not installed; moved to
Release). HTTP behaviour is tested against a local test server in
`src/provider/tests_http.rs`; backoff is a pure function, so no paused
clock is needed. `tests/live.rs` holds the ignored tests against real APIs,
including a TLS check that needs only network.

### Tools and permissions

- The six tools, with workspace boundary and `extra_dirs`; `run_command`
  with timeout, process group, `SIGTERM` then `SIGKILL`.
- `edit_file` as exact-text replacement; diff for display.
- Glob syntax check of rule patterns at load time.
- Detailed rules (`[[permissions.rules]]`: `paths`, `commands`, `except`,
  `message`), tool groups `read` and `edit`, `extra_dirs` with `access`.
- Permission checking in the order of `config.md`: boundary (including
  read-only extra dirs), deny, ask, allow, default; session rules; the
  deciding rule and its source kept with the decision.
- Loading `[permissions.allow]`, `[permissions.ask]`, `[permissions.deny]`
  (tool or group → patterns) in place of the old `allow = [...]` lists.
- Word-based command patterns: shell-like word splitting, prefix match,
  per-word globs, `**` for any words, file name of the first word,
  assignments and wrappers (`sudo`, `env`, `xargs`, …) looked through for
  `deny`/`ask`, `sh -c` strings checked recursively, pipeline patterns.
- Command splitting for `;`, `&&`, `||`, `|`, `&`, `$( )`, backticks,
  subshells; redirection targets checked as `write_file` / `read_file`;
  anything not split safely asks.
- Per-entry checks in `list_files` and `search_files`; proposed session
  rules and their wider steps.

Tests: each tool in a temp dir; path escapes (`..`, symlinks) denied; rule
order; `except`; name-only, relative and absolute path patterns; project-file
merge; a table of command lines and their split parts, including the
`cargo test && rm -rf /` and `echo x > ~/.bashrc` cases; a table of command
patterns and parts that match or not (`ls*`, `rm` vs `rmdir`, `sudo rm`,
`git ** --force`, `sh -c "rm x"`); `search_files`
skipping denied files.

Spec: [`spec/tools.md`](spec/tools.md).

Status: done. Matching lives in `src/permissions/`, the tools in
`src/tools/`; both are a library API (`tools::prepare`, `PreparedCall::run`,
`accept_rules`) that the agent loop will drive. Decisions the spec left
open (file-name matching only for deny/ask, wrappers, keywords, dynamic
command names, pipelines, background processes after exit, proposals as
rule sets) are written into `spec/config.md` and `spec/tools.md`.

### Agent loop

- Turn: build request from context, stream events, collect tool calls,
  check permissions, ask questions, run tools, append results, repeat.
- Every `ToolCall` gets exactly one `ToolResult` (also denied, cancelled).
- Limits per turn; limit question with continue/stop.
- Cancellation at any point.
- Emits session events: text deltas, tool call states, questions (with the
  details each tool prepares), notices, errors, usage, turn end.
- Validates answers against the open question (id, choice, `always` rule).
- Minimal context: system prompt and full history; `ContextTooLong` is
  reported as an error until truncation exists.
- Loading `[limits]` (`max_requests`, `max_tool_calls`; the project file
  only lowers them).

Tests: all against the `fake` provider: plain answer, tool round trip,
denied call, cancelled turn, limit reached then continue and stop, provider
error mid-stream, malformed tool arguments.

Spec: [`spec/agent-loop.md`](spec/agent-loop.md).

Status: done. `src/agent/` is a library API (`Agent::run_turn`, `Asker`,
`PolicyAsker`, `SessionEvent`) that `clai run` and the session actor will
drive. A turn has a fixed model for now; changing it at the next request
comes with Server and sessions.

### One-shot mode

- `clai run` and `clai TASK...`: temporary in-process session, plain-text
  output (answer on stdout, tool lines on stderr), `--yes`, `--no-ask`,
  `-C`, exit codes, `Ctrl-C` exit `130`.
- Questions on the terminal when standard input is one, with the text
  renderer shared with `clai chat`.

Tests: end-to-end against a local HTTP server in the test that speaks the
`openai-compatible` format, checking stdout, stderr and exit codes; the
binary has no test hooks.

After this milestone `clai` is usable from scripts.

Status: done. `src/command/run.rs` drives the agent loop; `src/text/`
(tool lines, question text, `LineAsker`) is ready for `clai chat`.

### Server and sessions

- `protocol`: newline-delimited JSON messages with a version handshake.
- `server`: socket in `$XDG_RUNTIME_DIR/clai/` with mode `0700`, start on
  demand, `clai server` foreground, idle exit after 60 s, `stop`,
  `status`.
- `session` actor: state machine of `sessions.md`, background work,
  questions shown to all clients with first answer winning, message queue,
  event log so attach replays the full history.
- Commands `clai new -d`, `ls`, `kill`, `send`, `chat` (interactive,
  `--list`, script form).
- Clients declare on attach whether they answer questions; open questions
  are sent on attach; `question closed` names the answering client.
- Tab completion of session names for `attach`, `kill`, `send`, `chat`.
- Configuration reload on file change.

Tests: server in a temp runtime dir; several sessions in parallel with the
`fake` provider; detach during a turn; question answered by one of two
clients; `clai chat` in script form with a stale `--id`; an `always` rule
that does not match the call is rejected; a session task that fails does
not affect another; version mismatch error.

Status: done, except what needs the TUI (`clai`, `clai new` without `-d`,
`clai attach`). Decisions: sessions live in memory only; several sessions
may share a workspace (`clai new` prints a note); the user file is polled
every 2 s instead of using `notify`; a slow client is dropped after 4096
waiting messages. Sessions are tested in-process in `src/server/tests.rs`
through real clients with scripted providers; `tests/server.rs` runs the
commands as processes.

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

- Static build check with an HTTPS request (`cargo test --test live
  tls_reaches_anthropic -- --ignored` on the musl build).
- Static build for `x86_64-unknown-linux-musl` (and `aarch64` if wanted),
  size and start-time check, version from `Cargo.toml`.
- `readme.md`: install, configuration example, commands.

## Spec work needed before building

| Before milestone | Spec to write or decide |
|------------------|-------------------------|
| Tools and permissions | Open question 5 in `spec/readme.md` (sandbox); open questions in `spec/tools.md`. |
| Agent loop | Assumptions and open questions in `spec/agent-loop.md`. |
| One-shot mode | Open question 2 (one-shot first, as proposed here). |
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
