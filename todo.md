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
- [x] The TUI prompt (done in TUI core).

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
- [x] `clai chat [NAME]`: conversation in the terminal (new session or
      attach, history on attach, streamed text, inline questions,
      `/` commands, `Ctrl-C` cancel, `Ctrl-D` detach); the old mode is
      `clai chat --questions`.
- [ ] Line editing with history in `clai chat` (e.g. `rustyline`).
- [x] `clai`, `clai new` and `clai attach` with the TUI (done in TUI core).
- [ ] An end-to-end test of `clai chat` through a pseudo-terminal (colour,
      pager).

## Session persistence

- [x] Store: `session.json`, `history.jsonl`, `events.jsonl`, modes
      `0700`/`0600`, format version, cut last line, repair of an open turn.
- [x] History journal and restore in the agent; allow rules kept.
- [x] Saved state in the server: lazy resume, `kill` deletes, `stop` and
      idle exit keep sessions; a failed session becomes saved.
- [x] `clai export`, `clai import`, `clai chat NAME --model`; protocol 2.
- [x] Tests: store, in-process restarts, binary resume.
- [ ] Bound the event log (in memory and on disk) for very long sessions.

## TUI core

- [x] Terminal setup and restore: panic hook, `SIGTERM`, `SIGHUP`, `Guard`.
- [x] `client::Connection`: reader and writer tasks, responses by id.
- [x] `App::update` and effects; entries from events; replay before the first
      frame; header and status line; `1/3 ●1` from `list` every 2 s.
- [x] Markdown, per-entry line cache, visible slice, follow mode, mouse wheel.
- [x] Input editor: multi-line, line keys, history, paste, `//`, queued mark.
- [x] Tool details: diffs, cut output, selection, symbols, `NO_COLOR`.
- [x] Permission and limit prompts; rejected answers; answered elsewhere.
- [x] Cancel, double `Ctrl-C`, `Ctrl-D`, session ended, reconnect for 10 s.
- [x] `/help`, `/detach`, `/quit`, `/model ARG`, `/rename`, `/kill`.
- [x] `clai`, `clai new`, `clai attach`; exit codes and the detach line.
- [ ] Manual run in kitty or foot (`Shift-Enter`, mouse); RFC 0001 and 0002
      are marked `done` already.
- [ ] A pseudo-terminal end-to-end test in the test suite (start, `Ctrl-D`,
      detach line, session still in `clai ls`).
- [ ] Measure the full replay on a long session; paged loading if too slow.
- [ ] `Ctrl-Z` suspends (restore, `SIGTSTP`, set up again on `SIGCONT`).

## Session improvements

- [x] Token totals: counted, saved, summed from the log on resume; the TUI
      session row, `/usage` in `clai chat`, `clai ls --json`.
- [x] Model change: `from` in `model_changed`, right notice on replay, saved
      sessions logged, no notice for the same model.
- [x] `models` request, protocol 3; the TUI takes context windows from it.
- [x] Model picker: `Ctrl-O`, `/model`, filter, marks.
- [ ] The TUI asks for the server's models only on opening the picker and after a
      reload, as RFC 0003 says, so a TUI with another `--config` has its own
      windows until then. Decide whether to ask at start and after a reconnect.

## Command list

- [x] `/` in the empty input opens the list; search, `Up` / `Down`, `Tab`,
      `Enter`, `Esc`; one table for the list and `/help` (RFC 0004).
- [x] Argument completion: `/model` offers models, `/attach` session names
      (RFC 0005).
- [ ] Commands marked `later version`: `/clear`, `/permissions`, `/commit`.
      Each box they open keeps the rule that a box has a command.

## Session switching

- [x] TUI: switching on a new connection, view state per session, `/attach`,
      `Ctrl-N` / `Ctrl-P` (RFC 0005).
- [x] Session list: `Ctrl-S`, `/sessions`; `Enter`, `n`, `r`, `k`, `Esc`; opens
      when the attached session ended.
- [x] New session: `/new [NAME]` and `n` with the model picker on
      `default_model`.
- [x] `clai attach` without a name and several sessions opens the list.
- [x] `clai chat`: `/sessions`, `/attach NAME`, `/new [NAME] [MODEL]`, `/models`.
- [ ] A pseudo-terminal test of switching in the real TUI (part of the TUI
      core pseudo-terminal test above).

## Settings and themes

- [x] Themes drawn: `terminal`, `dark`, `light`, custom; `background`; the
      256-colour fallback (RFC 0006).
- [x] Theme files in `themes/` next to the user file; bundled `atom-one-dark`,
      `nord`, `gruvbox-dark`, `solarized-light`.
- [x] `[tui]` applied live: from `/settings`, `/theme` and `config_reloaded`.
- [x] Saving `[tui]` into the user file with `toml_edit`: comments kept,
      checked, atomic, refused when the file changed.
- [x] `/theme [NAME]` with the theme list; `/settings` with the settings box.
- [ ] Syntax highlighting (`syntax_highlight`, a theme's `code` key).
- [ ] Look at the `dark` and `light` palettes in a real terminal (RFC 0006,
      open question 10).

## Status view

- [x] Times in `events.jsonl` (`at`), statistics counted from the log, also for
      a saved session (RFC 0007).
- [x] `retry` event and notice.
- [x] `session_status` request, protocol 4.
- [x] `/status [NAME]` in the TUI (live, scrollable) and in `clai chat`.

## Copy text and see tool details (RFC 0016)

- [x] Clipboard: OSC 52 and `wl-copy` / `xclip` / `xsel` / `pbcopy`,
      `tui.clipboard`.
- [x] `y`, `Y`, `/copy [N|code]`, `Ctrl-Y`; a copied edit applies with
      `git apply`.
- [x] Previews of finished edits, writes, commands and errors; live command
      output; `tui.tool_preview`, `expand_tool_calls` deprecated.
- [x] `tool_call` events carry the call's details once.
- [x] Click on a call, `Ctrl-T`.
- [x] Details view: `o`, search, wrap, copy; `/diff`.
- [x] Mouse selection in the conversation and the details view, double and
      triple click.
- [ ] Check the selection and OSC 52 in a real terminal (kitty, foot, tmux).
- [ ] `e` in the details view: open in `$PAGER` (RFC 0016, alternatives).

## Global statistics (RFC 0014)

- [x] `usage` events carry the model.
- [x] Counting per model and per day, cached per log; `global_stats`,
      protocol 7.
- [x] Status box tabs; `/stats [7d|30d|all]` with Totals, Models and the
      chart of tokens per day.
- [x] `/stats` in `clai chat`.
- [ ] Keep the tokens of deleted sessions (RFC 0014, open question 1).

## Security hardening (RFC 0010)

- [x] Log file `0600`, state directory `0700`, narrowed on start.
- [x] Built-in deny rules for secret files, `builtin_except`.
- [x] Allowed commands that name a file denied for reading ask; protocol 5.
- [x] System prompt: tool output is data; denied files are not reached
      another way.

## Questions from the model (RFC 0012)

- [x] `ask_user` tool: checks, result texts; offered only when someone can
      answer; `[permissions] ask_user`; protocol 6.
- [x] Question kind `ask_user` with `reply`, `decline`, `cancel`, checked by
      the server; counted in `/status`.
- [x] Text dialog in `clai chat` (also `--questions`, `decline` in the
      script form) and `clai run` on a terminal.
- [x] TUI question box: options, ticks, typed text, review page, skip.
- [ ] Keep what was chosen in the box when switching sessions and back.

## Session list: delete, marks and filter (RFC 0011)

- [x] `SessionInfo` carries `updated` and `turns`; old messages still read.
- [x] `d`, `Del` delete after a question that says what is lost; `j` / `k` move;
      `/delete` as alias of `/kill`; `USED` column.
- [x] Marks: `Space`, `a`; one question for all marked; a result line once
      all `kill`s are answered.
- [x] Filter: `/` with text and time words (`>7d`, `<1h`).
- [ ] `clai ls`: `USED` instead of `AGE` too (RFC 0011, open question 3).

## Activity line (RFC 0015)

- [x] 120 ms frame tick while the session works; tool call spinners use it.
- [x] Activity line above the input: verb or tool, time, output tokens,
      `Esc cancel`; cut to fit; hidden below 14 rows and while a prompt is
      open; stalled colour after 30 s.
- [x] Start time of a running turn from `session_status` after an attach.
- [x] `tui.activity` (`playful`, `plain`, `off`), in `/settings`.
- [ ] `clai chat`: a spinner before the first text (RFC 0015, out of scope).
- [ ] Custom verb lists, `tui.activity_words` (RFC 0015, open question 3).

## Commit messages (RFC 0013)

- [x] `clai commit`: staged changes only, from the repository of `DIR`;
      `--staged` removed.
- [x] Git without programs from the repository's config; size budget;
      protected files named only.
- [x] Agent file (`AGENTS.md`, `AGENT.md`, `agent.md`) at the root; recent
      subjects; `--hint`; `[commit] model` and `trailer`.
- [x] One request without tools; cleaned message on standard output.
- [x] `-a` / `--all`: also unstaged changes of tracked files, as
      `git commit -a`.
- [ ] TUI `/commit` dialog (`spec/tui.md`, "Commit").

## Refactoring

- [x] A: `Choice::name`, `text::ansi`, `text::Terminal`, `sync::lock`,
      `Error::io`, `runtime` (no behaviour change).
- [x] B, the part in RFC 0003: session registry type, `Meta::info`, question
      state machine in the session.
- [ ] B, the rest: agent split, shared chat answering, `paths`, `signals`.
- [x] The command layer (RFC 0009): `Execute` and one dispatch, `Context`,
      `command::common`, `text::sessions`, `output::note` and `columns`,
      `clai run` in named parts.
- [ ] C: shared test tools (model server, `Env`, fake provider), split
      server tests.
- [ ] D (to decide): split `SessionEvent`, typed store errors, one text
      renderer.

## Next milestones

- [ ] TUI sessions and settings: the rest (syntax highlighting; a server notice
      for session state changes, a new protocol version, in place of polling
      `list`)
- [ ] Commit: the TUI `/commit` dialog (`clai commit` is built, RFC 0013)
- [ ] Context management (history trimming of old tool results: RFC 0008,
      draft; the rest needs `spec/context.md`)
- [ ] Release
