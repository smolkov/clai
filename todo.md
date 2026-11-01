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
- [ ] Reload on file change (moved to Server and sessions).
- [x] Glob syntax check of rule patterns (done in Tools and permissions).
- [x] `[permissions.allow|ask|deny]` tables, `[[permissions.rules]]`,
      `extra_dirs` tables (done in Tools and permissions).
- [ ] Limit keys `[limits]` (moved to Agent loop).

## Shell completion

- [x] `clai completion [SHELL]` with dynamic completion (`COMPLETE=<shell>`).
- [x] `--model` completes configured models; paths for `--config`, `-C`, `DIR`.
- [ ] Session names for `attach`, `kill`, `send` (Server and sessions).

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
- [ ] Questions on the wire and the TUI prompt (Agent loop, Server, TUI).

## Next milestones

- [ ] Agent loop (spec: `spec/agent-loop.md`)
- [ ] One-shot mode
- [ ] Server and sessions
- [ ] TUI core
- [ ] TUI sessions and settings
- [ ] Commit (needs `spec/commit.md`)
- [ ] Context management (needs `spec/context.md`)
- [ ] Release
