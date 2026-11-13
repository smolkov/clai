# Changelog

All notable changes to `clai`. One section per version, newest first.
Changes go into "Unreleased" as they are made; a release moves them into a
section `X.Y.Z - YYYY-MM-DD` (see `readme.md`, "Release"). The release
notes are taken from that section by `ci/notes.sh`.

## Unreleased

## 0.1.0 - 2026-11-13

First release.

- `clai`, `clai new`, `clai attach`: sessions with the TUI, in a background
  server; saved and resumed.
- `clai run`: one task without the TUI.
- `clai ls`, `send`, `chat`, `kill`, `export`, `import`, `server`, `models`,
  `commit`, `completion`.
- Tools behind permission rules: read, list, search, edit and write files,
  run commands, ask the user.
- Language servers (`lsp` tool, `[lsp]`, `/lsp`): definitions, references,
  types, callers, symbols and errors; errors added after edits. Starting a
  server asks.
- Providers: `anthropic` and `openai-compatible`.
- Themes, settings, syntax highlighting, status and statistics views.
- Model picker with context windows; `Space` makes a model the default,
  `/model NAME --default`, `clai models --set-default`.
- Conversation layout: short tool names, `●` before answers, clicks on
  tool calls, questions from the model at the bottom.
- Static Linux binaries (x86_64, aarch64), macOS binaries, Debian
  packages.
