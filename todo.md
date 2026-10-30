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
- [ ] Static musl build check (moved to Providers; target not installed).

## Next milestones

- [ ] Configuration
- [ ] Providers
- [ ] Tools and permissions (needs `spec/tools.md`)
- [ ] Agent loop (needs `spec/agent-loop.md`)
- [ ] One-shot mode
- [ ] Server and sessions
- [ ] TUI core
- [ ] TUI sessions and settings
- [ ] Commit (needs `spec/commit.md`)
- [ ] Context management (needs `spec/context.md`)
- [ ] Release
