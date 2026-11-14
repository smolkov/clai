# Changelog

All notable changes to `clai`. One section per version, newest first.
Changes go into "Unreleased" as they are made; a release moves them into a
section `X.Y.Z - YYYY-MM-DD` (see `readme.md`, "Release"). The release
notes are taken from that section by `ci/notes.sh`.

## Unreleased

- Boxes at the bottom (RFC 0024): the model picker, session list, skills,
  status, language servers, themes and help open in the place of the
  input, at the full width, with the conversation above; the settings
  take the whole screen.

## 0.3.1 - 2026-11-14

The release of 0.3.0 did not complete: the arm64 Debian package failed its
lintian check. 0.3.0 and 0.2.1 were not published, so this release also
brings the changes of 0.3.0 (skills) and 0.2.1 below.

- Debian package for arm64: lintian override for the static binary.
- A server that stops no longer sets the time of the last change of every
  session: `clai -c` and the ages in the session list stay right after a
  restart.
- `clai -c` / `--continue` (RFC 0025): continue the last session of the
  current directory, or start one; also `clai chat -c` and
  `clai send -c MESSAGE`.

## 0.3.0 - 2026-11-14

- Skills (RFC 0023): `SKILL.md` directories of OpenCode, Claude Code and
  `.agents`, also in subdirectories (`synced/<id>/<skill>/`), and
  `[skills] dirs`; `clai skills [--json]`; `/skills` box and
  `/skills NAME` in the TUI, `/skills` in `clai chat`, `--skill` for
  `clai run` and `clai new`. An active skill's instructions go into the
  system prompt and its directory is readable. Protocol 9: restart the
  server after the update (`clai server stop`).

## 0.2.1 - 2026-11-14

The same code as 0.2.0, whose release did not complete: four tests failed
on macOS.

- Tests: paths compared resolved, as macOS links `/etc` and the temporary
  directory; `pbcopy` expected on macOS.

## 0.2.0 - 2026-11-13

First published release; the same code as 0.1.0, whose release did not
complete. See 0.1.0 for the features.

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
