# Changelog

All notable changes to `codeagent`. One section per version, newest first.
Changes go into "Unreleased" as they are made; a release moves them into a
section `X.Y.Z - YYYY-MM-DD` (see `readme.md`, "Release"). The release
notes are taken from that section by `ci/notes.sh`.

## Unreleased

- Ten more bundled themes, each with matching syntax colours (RFC 0031):
  `emacs`, `flexoki-dark`, `flexoki-light`, `material-oceanic`, `monokai`,
  `nightfox`, `one-light`, `solarized-dark`, `textual-dark` and
  `tokyo-night`. A theme's `code` key takes the new names too.

## 0.8.0 - 2026-11-16

- Secrets are hidden from the model (RFC 0030): API keys, tokens,
  passwords and private keys in messages, file contents, command output,
  the prompt and the commit diff become placeholders like `[secret:
  github token #81c0]`. File edits keep the real values; commands never
  get them. Tool lines, the status view and run reports count them.
  `[secrets]` sets it up.

## 0.6.0 - 2026-11-16

- Fewer tokens, same answers (RFC 0028):
  - The requests of a turn start the same (tools and system prompt are
    fixed for the turn; a change of skills applies from the next turn),
    and `anthropic` requests are marked for the prompt cache. Turn it off
    with `cache = false` in a provider's table.
  - Cached input is counted on its own: `/usage`, `/status`, `/stats`,
    `codeagent ls --json` (`cache_read_tokens`) and run reports show it.
  - Old tool results go to the model as one-line placeholders, in steps
    so the cache keeps working (RFC 0008); `[history]` sets
    `trim_results`, `keep_results`, `min_chars` and `trim_after`. The TUI
    and the saved session keep every result.
  - Command output reaches the model without colour codes, redrawn
    progress lines and long runs of the same line.
  - A repeated `read_file` of an unchanged file points to the earlier
    result.
  - `ci/bench.sh` runs fixed tasks against a model and prints tokens and
    success per run (`bench/`).
- A scratch directory for each session (RFC 0029), in the runtime
  directory with mode `0700`: the file tools use it without asking,
  commands get it as `TMPDIR` and `CODEAGENT_TMP`, and the system prompt
  names it. It is deleted with the session; `codeagent run` deletes it at
  the end unless `--keep-tmp` is given. Shown as `scratch` in the status
  view, as `tmp` in `codeagent ls --json`, and under `scratch` in run
  reports. `[session] scratch = false` turns it off.
- The TUI draws Markdown tables as aligned columns, with cells that wrap
  to fit the screen; before, a table showed as one run-on line.

## 0.5.0 - 2026-11-15

- `clai` is now `codeagent`: the command, the crate, the Debian package and
  the release files. The variables are `CODEAGENT_MODEL`, `CODEAGENT_THEME`
  and `CODEAGENT_LOG`; the directories `~/.config/codeagent` and
  `~/.local/state/codeagent`, the log `codeagent.log`. On its first start
  `codeagent` moves `~/.config/clai` and `~/.local/state/clai` to the new
  names, with a note; stop a running `clai` server first (`clai server
  stop`). Project files move from `.clai/` to `.codeagent/` by hand.

## 0.4.0 - 2026-11-15

- Reports (RFC 0027): `clai run --report` prints a report instead of the
  last answer, as Markdown or with `--report=json` as JSON: the status,
  summary, findings and follow-ups the model hands in with the new
  `report` tool, and the changed files, commands, files read and every
  tool call of the run. `clai run` without words reads the task from a
  pipe, without `-`.
- Startup (RFC 0026): the environment is read once; every workspace is
  used with links resolved; only the server and `clai run` warn about a
  log they cannot open; each log line names its process (`server[4242]`).
- Syntax colours: in `atom-one-dark`, `nord` and `gruvbox-dark`, `let`,
  `struct` and the like have the keyword colour, not the type colour; in
  `ansi`, `struct`, `enum`, `impl` and `class` do, like `fn`. In all four,
  type names (`struct P`, `enum E`) have the type colour; they had none.
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
