# Todo

Milestones and their order: [`implementation.md`](implementation.md).

## Foundation

- [x] `Cargo.toml`: dependencies, never-crash lints, release profile.
- [x] Error type with exit codes.
- [x] Command-line parsing of all commands in `spec/command.md`; unbuilt
      commands exit `1` with "not implemented yet".
- [x] Logging to `$XDG_STATE_HOME/clai/clai.log`, level from `CLAI_LOG`.
- [x] Unit tests for parsing, integration tests for exit codes and output.
- [x] `readme.md`, `todo.md`.
- [ ] Static musl build check (moved to Release; target not installed).

## Configuration

- [x] Load user file, project file, `CLAI_MODEL` / `CLAI_THEME`; precedence.
- [x] Validation with file:line errors; project-file restrictions.
- [x] Permissions loading and merging.
- [x] `[tui]` and `[themes]` validation (colours used by the TUI later).
- [x] Model references with short form.
- [x] API keys from env or `api_key_cmd`, never shown.
- [x] `clai models`, `--json`.
- [x] Reload on file change (done in Server and sessions).
- [x] Glob syntax check of rule patterns (done in Tools and permissions).
- [x] `[permissions.allow|ask|deny]` tables, `[[permissions.rules]]`,
      `extra_dirs` tables (done in Tools and permissions).
- [x] Limit keys `[limits]` (done in Agent loop).

## Shell completion

- [x] `clai completion [SHELL]` with dynamic completion (`COMPLETE=<shell>`).
- [x] `--model` completes configured models; paths for `--config`, `-C`, `DIR`.
- [x] Session names for `attach`, `kill`, `send`, `chat` (Server and sessions).

## Providers

- [x] Shared types, `Provider` trait, `from_config`.
- [x] SSE parser.
- [x] `anthropic` and `openai-compatible` adapters.
- [x] Retries with backoff before the first event, timeouts, cancellation.
- [x] `fake` provider for agent-loop tests.
- [x] Live tests (`cargo test --test live -- --ignored`).
- [ ] Static musl build check (moved to Release; target not installed).

## Tools and permissions

- [x] Rule loading with sources (file:line), groups, detailed rules,
      `except`, `message`, read-only `extra_dirs`.
- [x] Path resolution (links, `..`, dangling links) and the workspace boundary.
- [x] Path patterns; command patterns with wrappers, assignments, `**`,
      pipelines; shell parser for parts, substitutions, redirections.
- [x] Checking order, decisions with reasons, denial texts.
- [x] Session rules and proposals for "always"; `accept_rules`.
- [x] The six tools with question details, result size limits.
- [x] `run_command`: process group, timeout, cancellation, background
      processes stopped.
- [x] Questions and answers as data, checked by the loop (Agent loop).
- [x] Questions on the wire (Server and sessions).
- [ ] The TUI prompt (TUI core).

## Agent loop

- [x] `[limits]`: `max_requests`, `max_tool_calls`; project file only lowers.
- [x] `Agent::run_turn`: requests, streaming, tool calls in order, results
      in one message, every call with one result.
- [x] Permission and limit questions through `Asker`; answers checked,
      wrong ones rejected; `PolicyAsker` for `clai run`.
- [x] Stop reasons, empty answers, cut tool calls.
- [x] Cancellation while streaming, waiting, running; provider errors,
      `ContextTooLong`.
- [x] Session events (serde), live `run_command` output.
- [x] System prompt with workspace, extra dirs, platform, date.
- [x] Model change during a turn (Server and sessions).
- [x] Deserializing questions and events in clients (Server and sessions).

## One-shot mode

- [x] `clai run`, `clai TASK...`: task words or `-` from stdin, `-C`,
      `--model`, `--yes`, `--no-ask`.
- [x] Final answer on stdout; tool lines, model text, notices and errors
      on stderr; `tui.symbols`.
- [x] Questions on the terminal (`LineAsker`); policies without one.
- [x] Exit codes; `Ctrl-C` (130), `SIGTERM` and `SIGHUP` cancel the turn.
- [x] End-to-end tests against a local `openai-compatible` server.
- [x] Colour and the `v` pager in question text (done in `clai chat`).
- [ ] An end-to-end test of terminal questions through a pseudo-terminal.

## Server and sessions

- [x] Protocol: one JSON message per line, `hello` with the version,
      requests and responses, events with `replay`, notices.
- [x] Server: socket in a `0700` runtime directory, lock against a second
      server, start on demand, idle exit after 60 s, `stop`, `status`.
- [x] Session actor: state, message queue, event log replayed on attach,
      questions to answering clients, first answer counts, `kill`,
      `rename`, `set_model`; a failed session ends alone.
- [x] Configuration reload (polled every 2 s), notices to clients.
- [x] `clai new -d`, `ls`, `kill`, `send`, `chat` (interactive, `--list`,
      script form), `server`, `server stop`, `server status`.
- [x] Tests: in-process server with scripted providers; the commands as
      processes against a local model server.
- [ ] `clai`, `clai new` and `clai attach` with the TUI (TUI core).
- [ ] An end-to-end test of `clai chat` through a pseudo-terminal (colour,
      pager).

## Next milestones

- [ ] TUI core
- [ ] TUI sessions and settings
- [ ] Commit (needs `spec/commit.md`)
- [ ] Context management (needs `spec/context.md`)
- [ ] Release
