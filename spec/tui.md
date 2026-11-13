# TUI

Interactive sessions are shown in a full-screen terminal user interface (TUI).
The TUI is a client of the `clai` server: it attaches to one session at a
time, and can detach, switch to another session or start a new one. Sessions
keep running without a TUI (see `spec/sessions.md`).
One-shot mode (`clai "task"`) and runs where standard output is not a
terminal do not use the TUI; they print plain text (see
[`command.md`](command.md)).

Statements marked **(assumption)** need a decision from the user.

## Goals

- Show the conversation, tool calls and their results as they happen,
  including the model's answer while it streams.
- Make every permission decision visible and quick: the change or command is
  shown in full before the user answers.
- Stay responsive: the screen redraws and keys work while the model answers
  or a command runs.
- Never leave the terminal broken: on exit, error or panic in a dependency,
  the terminal is restored (raw mode off, cursor shown, main screen back).

## Layout

```
> fix the failing test

The test fails because `parse` drops the last line. Let me look.

▸ read_file src/parse.rs                                          ✓ 120 lines
▾ edit_file src/parse.rs                                                ? ask
  - for i in 0..lines.len() - 1 {
  + for i in 0..lines.len() {

──────────────────────────────────────────────────────────────────────────────
edit_file: src/parse.rs  asks: default for edit_file
[y] yes  [a] always  [n] no  [e] explain  [Esc] cancel
──────────────────────────────────────────────────────────────────────────────
> _
──────────────────────────────────────────────────────────────────────────────
  asking · Esc cancel · /help · Ctrl-D detach
  project · ~/ws/project · anthropic/sonnet · 1/3 ●1 · 12.3k/200k · in 45.1k out 3.2k
```

There is no box around the screen: the conversation takes the full width, and
plain lines set off the prompt bar and the input. From top to bottom:

| Area           | Content |
|----------------|---------|
| Conversation   | Scrollable history of the session: user messages, model text, tool calls. |
| Activity line  | Shown only while the session works: what it does, for how long, and the tokens so far. See [Activity line](#activity-line). |
| Prompt bar     | Shown only while a question (permission or limit) is open; replaces nothing else. |
| Input          | Multi-line editor for the next user message, with a line above and below it. |
| Status row     | Current state (idle, waiting for model, running tool, asking), turn counters, key hints. |
| Session row    | Session name, workspace path, provider and model (`anthropic/sonnet`), position among all sessions (`1/3`), number of other sessions waiting for an answer (`●1`), context usage (tokens used / limit; only the tokens if the server does not list the model, nothing until the first count), tokens the session has used (`in 45.1k out 3.2k`). |

While the session works and no question is open, the activity line takes the
last row of the conversation area, with an empty row above it:

```
▸ read_file src/parse.rs                                          ✓ 120 lines

✻ Prestidigitating… (14s · ↓ 1.2k tokens · Esc cancel)
──────────────────────────────────────────────────────────────────────────────
> _
```

The conversation area takes all remaining height. If the terminal is smaller
than 40 × 10 cells, `clai` shows only a "terminal too small" message until it
is resized.

## Sessions

The TUI shows exactly one session, the attached one. Other sessions keep
running on the server; the session row counts the ones waiting for an answer to a
question (see [`sessions.md`](sessions.md), "Background work"), so the user
knows to switch to them.

The session list opens with `Ctrl-S`, `/sessions`, or `clai attach` without a
name:

```
┌ Sessions ────────────────────────────────────────────────────────────────────┐
│       NAME       WORKSPACE     MODEL             STATE      USED             │
│     ✓ project    ~/ws/project  anthropic/sonnet  working    12m              │
│ ▶ ■   project-2  ~/ws/project  local/qwen        idle       2h               │
│       docs       ~/ws/docs     anthropic/opus    ● waiting  3m               │
│   ■   old        ~/ws/old      anthropic/sonnet  saved      12d              │
│                                                                              │
│ 2 marked                                                                     │
│ Enter attach ─ Space mark ─ a all ─ d delete ─ / filter ─ n new ─ r rename … │
└──────────────────────────────────────────────────────────────────────────────┘
```

- The list holds every session the server knows, running and saved, in the
  server's order, which is also the order of `1/3` and of `Ctrl-N` / `Ctrl-P`.
  `✓` marks the attached session; it is selected when the list opens. `■`
  (ASCII `#`) marks a session marked for deleting.
- `USED` is the time since the session last changed (`SessionInfo.updated`;
  from an older server without it, since it was created).
- It asks the server for the sessions when it opens and every 2 s while open;
  the selection stays on the same session.
- On a narrow terminal the time, then the workspace, are left out; the key
  line leaves out `a all` first, then `r rename`, then `n new`.
- A question that opens in the attached session closes the list: keys belong to
  the prompt bar then.
- The line above the keys shows, when nothing is asked, the filter, how many
  sessions are shown of how many, and how many are marked, each only when it
  applies: `filter: saved >7d ─ 2 of 9 ─ 2 marked`. A message (a refused name,
  the result of a delete) takes its place.

| Key in list | Action |
|-------------|--------|
| `Up` / `Down`, `k` / `j` | Select a session among those shown; they stop at the ends. |
| `Enter`     | Attach to the selected session (detaches from the current one). On the attached session it only closes the list. |
| `Space`     | Mark or unmark the selected session, then select the next one. |
| `a`         | Mark every session shown; if all shown are marked already, unmark them. |
| `d`, `Del` | Delete: the marked sessions if any are marked (also those the filter hides), else the selected one, after a `y`/`n` confirmation. |
| `/`         | Filter: edit the filter text in the line above the keys. |
| `n`         | New session: asks for the workspace (default: the attached session's; `~` is the home directory, a relative path starts there) and the name (empty: the default name), then opens the model picker on `default_model`; `Enter` there creates the session and attaches. `Esc` goes back one step; in the picker it creates nothing. |
| `r`         | Rename the selected session, in place; `Enter` renames, `Esc` goes back. A refused name is shown in the list. |
| `Esc`       | Clear the marks if any; else clear the filter if set; else close the list, stay on the current session. Where the list stands for a choice not yet made (after the attached session ended, or for `clai attach` without a name), `Esc` exits the TUI. |

Marks belong to sessions, not to rows: they stay through the refresh and a
change of the filter. A session the server no longer lists loses its mark.
Marks end when the list closes.

### Filter

`/` opens the filter, starting with the current text: `Filter: saved >7d`.
Each key edits it, and the list follows. `Enter` or `Down` goes back to the
list with the filter kept; `Esc` empties it and goes back.

The text is split into words; a session is shown if every word matches:

| Word | Matches |
|------|---------|
| `>7d`, `>12h`, `>30m`, `>2w`, `>90s` | Not used for more than that. |
| `<1d` and the like | Used within that time. |
| Anything else | A part of the name, the workspace as shown, the model or the state (`idle`, `working`, `waiting`, `saved`), ignoring case. |

A word that looks like a time but does not parse (`>7x`) matches as text,
so it hides everything and shows that the filter is wrong. The attached
session is shown only if it matches. If the selected session is filtered
out, the first one shown is selected. With nothing shown, the list says `no
session matches`, and `Enter`, `d`, `r` and `Space` do nothing.

So "delete all saved sessions not used for a week" is `/ saved >7d`,
`Enter`, `a`, `d`, `y`.

### Delete

The confirmation says what is lost:

```
Delete old? 14 turns, 210k tokens. [y] yes  [n] no
Delete project? It is working: the turn is cancelled. 3 turns, 41k tokens. [y] yes  [n] no
Delete docs? Its question is closed unanswered. No turns yet. [y] yes  [n] no
Delete 5 sessions? 1 is working, 1 waits for an answer. [y] yes  [n] no
```

The tokens are input plus output. `y` sends one `kill` per session, in list
order, and clears the marks; `n` or `Esc` goes back with the marks kept.
When all are answered, the line says `deleted 5 sessions`, or with a failure
`deleted 4 sessions; docs: no session named docs` (for example a session
renamed by another client meanwhile), and the list is asked for again.
There is no undo: the files are removed at once. Deleting the attached
session, alone or among others, works as `/kill`: the TUI shows a notice and
the list stays open.

`/new [NAME]` creates a session in the attached session's workspace the same
way: the model picker opens on `default_model`, so `Enter` alone starts the new
session with it. If other sessions work in that workspace, the new session's
conversation starts with the note `clai new` prints.

## Token counters

The session row ends with the tokens the session has used. They are the first
thing cut on a narrow terminal, and left out until the first `usage` event.

- **Input** and **output** are the sums over all requests of the session.
  Input counts the history again with every request, because that is what is
  sent; it is not the context size.
- Anthropic's cached input is included in input, so the counter is not a bill.
- A cancelled request counts what the provider had reported by then. A provider
  that reports nothing adds nothing.

The TUI sums the `usage` events of the log it replays, and starts again on a
reconnect, so a reconnect never counts anything twice.

When the provider retries a request, the conversation shows a notice:
`retrying (2/3): HTTP 503: busy`.

## Model picker

Each session has its own provider and model. The model picker chooses them.
It opens with `Ctrl-O`, `/model` without an argument, or when creating a
session; then its title is "Model for a new session", it starts on
`default_model`, and it has no `current` or history marks.

```
┌ Model for "project" ──────────────────────────────────────────────────────┐
│ Filter: _                                                                 │
│                                                                           │
│ anthropic                                                                 │
│ ▶ sonnet      claude-sonnet-5-5         200k   ✓ current, default         │
│   opus        claude-opus-5-5           200k                              │
│ local                                                                     │
│   qwen        qwen3-coder:30b            32k   ⚠ history 41k > 32k        │
│                                                                           │
│     already the default for new sessions                                  │
│                                                                           │
│                                                                           │
│ Enter select ─ Space default ─ Esc cancel                                 │
└───────────────────────────────────────────────────────────────────────────┘
```

- The picker lists exactly the providers and models of the configuration
  (`spec/config.md`), grouped by provider, in config order. There is no
  free-text model id: a model that is not configured cannot be chosen.
- Typing filters by provider name, model name and model id.
- Each row shows the model's config name, its `id` and context window. Marks:
  current model of the session, `default_model`, a provider whose API key is
  missing (`⚠ no key`; still selectable, the error comes on first use), and a
  model whose context window is smaller than the session's history
  (`⚠ history 41k > 32k`, from the last context size). Until
  `spec/context.md` exists such a model is only marked.
- The list comes from the server (the `models` request), since its
  configuration and environment are what the requests use. The TUI takes context
  windows from the same answer; it asks when the picker opens and when the
  configuration was reloaded.
- `Enter` selects and closes; on the current model it only closes. `Esc` closes.
  `Up` / `Down` select; other keys edit the filter, except `Space` (below). With
  no match the list says `no model` and `Enter` does nothing.
- **Default for new sessions** (RFC 0021). Under the list, `[ ] Also make it the
  default for new sessions`; `Space` ticks or unticks it (model names and ids
  have no spaces, so `Space` does not edit the filter; pasted text still does).
  With the tick, `Enter` also saves the selected model as `default_model` in
  the user file the server reads (see [`config.md`](config.md), "Writing the
  user file"); on the session's current model it only saves. `Esc` saves
  nothing. The tick starts unticked each time the picker opens.
  - On the row that already is the user file's default the line reads
    `already the default for new sessions` and `Space` does nothing.
  - With the tick, the line under it names the file:
    `saved as default_model in ~/.config/clai/config.toml`.
  - When the default does not come from the user file, that line warns,
    ticked or not: `⚠ CLAI_MODEL=local/qwen overrides the saved default`, or
    `⚠ .clai/config.toml of this workspace sets default_model = local/qwen`.
    The save still happens: other workspaces and later starts use it.
  - After the save the conversation shows
    `default model: anthropic/opus (saved in …)`, and the warning again if
    something else wins; a failed save shows
    `default model not saved: …` and keeps the selection.
  - The next start uses the saved model: `clai`, `clai new`, `/new` and the
    session list's `n`, `clai run` and `clai commit` without a model, also
    after a server restart. A running server reads the changed file before
    it creates a session or answers `models`, so a start right after the save
    gets it, not only after the next 2 s poll.
- The picker does not open while a question is open, and a question that opens
  closes it: keys belong to the prompt bar then.
- `/model <provider>/<model>` (or `/model <model>` when unambiguous) switches
  without the picker; an unknown name is an error that lists the configured
  models.
- If no model is configured, the picker shows the path of the user config file
  and a short example instead of a list.

Changing the model of a running session:

- The new provider and model apply from the next model request. A request that
  is already running finishes with the old model. **(assumption)**
- The conversation history is kept and converted to the new provider's
  format. If it does not fit the new context window, the oldest parts are
  shortened (see `spec/context.md`); the TUI asks for confirmation first.
- A notice in the conversation records each change, e.g.
  "model: anthropic/sonnet → local/qwen". Choosing the model the session already
  has adds no notice. A log of an older `clai` has no old model; its notice is
  "model: local/qwen".
- The change applies to this session only. Other sessions and
  `default_model` are not affected, unless the default tick is set (above) or
  `/model … --default` is used.

## Switching sessions

- Each session keeps the TUI's view state while the TUI is attached to
  another one: scroll position, expanded tool calls and unsent input. It is
  restored on switching back. **(assumption)**
- On attach, the TUI loads the session's full history from the server, then
  follows it live. A turn that ran while detached appears as if it had been
  watched.
- A switch attaches on a new connection to the server and drops the old one,
  which detaches it. Until the new session is there, the old one stays on
  screen and the status row says `attaching NAME…`; if it fails, the error is
  shown and the old session stays. A question open in the session left stays
  open for other clients and is counted in `●`.
- If the attached session is killed (by this or another client), the TUI
  shows a notice and opens the session list. If no session is left, the list
  says `no sessions` and offers `n` new or `Esc` exit. `Esc` exits with `0`.
  Until the list is there, any key exits. For `clai server stop` the TUI
  opens no list: the notice, the input is disabled, and any key exits with
  `0`; it does not start the server again.
- If the connection to the server is lost, the TUI shows "server
  disconnected" and retries every 500 ms for 10 s, starting the server if
  needed, since sessions are saved. The conversation is rebuilt from the log;
  scroll position, expanded tool calls and unsent input are kept. After 10 s
  it exits with an error; the terminal is restored. **(assumption)**

## Conversation entries

| Entry          | Display |
|----------------|---------|
| User message   | Prefixed with `>`, in the user colour. |
| Model text     | Rendered Markdown: headings, emphasis, lists, inline code, code blocks with syntax highlighting. Streams in as it arrives. |
| Tool call      | One summary line: marker, tool name, main argument, state. |
| Tool preview   | Below the summary of a finished edit, write or command, and of a failed call: see below. |
| Tool details   | Below the summary when expanded: diff for edits, file excerpt for reads, command and output for commands. |
| Error          | Red line with the error message; the session continues. |
| Notice         | Dim line for events such as "turn cancelled" or "limit reached". |

Tool call state is one of: running (spinner, moved by the frame tick of
[Performance](#performance)), done (`✓` plus a short result,
e.g. line count or exit code), failed (`✗`), denied (`⊘`), waiting for
permission (`?`).

Tool calls are collapsed (`▸`) by default and expanded (`▾`) while they wait
for permission. Long outputs are cut to `tui.tool_output_lines` lines (default
20) in the view, with a "… N more lines" marker; the full text stays
available in the expanded entry and the [details view](#details-view). See
[`config.md`](config.md), "TUI".

```
▸ read_file src/parse.rs                                          ✓ 120 lines
▾ edit_file src/parse.rs                                          ✓ edited …
  @@ -12,3 +12,3 @@
  - for i in 0..lines.len() - 1 {
  + for i in 0..lines.len() {
▾ run_command cargo test parse                                    ✓ exit code 0
  $ cargo test parse
  … 38 lines
  test parse::last_line ... ok
  test result: ok. 4 passed; 0 failed; 0 ignored
  exit code 0
```

**Previews.** With `tui.tool_preview = "changes"` (the default), a finished
call shows a preview (`▾`) below its summary:

- `edit_file`, `write_file`: the diff, without its `---` / `+++` header, up to
  `tool_output_lines` lines; a new file its first lines.
- `run_command`: the command and the **last** 6 lines of its output, with
  `… N lines` above them for what is hidden. While a command runs, the same
  shows its live output.
- A failed call of any tool: its error, up to `tool_output_lines` lines.
- `read_file`, `list_files`, `search_files` and others that did not fail: no
  preview; they stay one line.

`"all"` expands every call, `"none"` keeps every finished call on one line.
`Enter` on a call with a preview expands it, as from collapsed; what the user
chose for a call wins over the setting. `Ctrl-T` shows the previews of all
calls of the last turn (expanding calls without one), or collapses them all
when any of them shows more than its summary.

The details of an edit come with the call's `tool_call` event (see
[`architecture.md`](architecture.md), "Protocol"), also when no question was
asked, so its diff is known after it ran.

## Activity line

A line above the input while the session works, so that the user sees at
once that something happens and for how long:

```
✻ Prestidigitating… (14s · ↓ 1.2k tokens · Esc cancel)
```

- **Place:** the last row of the conversation area, with an empty row above
  it. The conversation is two rows shorter while it is shown and follows the
  end as before. It is not part of the conversation: not saved, not
  scrolled, not selected. When the user scrolled up, it stays at the bottom.
- **Hidden** when the session is `idle` or `saved`, while a permission, limit
  or question prompt is open (the prompt says what is needed), after the
  session ended, below 14 rows of terminal, and with `tui.activity = "off"`.

| Phase | When | Text |
|-------|------|------|
| Thinking | after the request is sent, before its text | the verb of the turn: `Prestidigitating…` |
| Writing | text deltas arrive | the verb, as above |
| Tool | a tool call is pending or runs | `Running cargo test…`, `Reading src/parse.rs…`, `Editing src/parse.rs…`; with several calls the first is named and `+2` follows |
| Retry | after a `retry` event, until the next text or tool call | `Retrying (2/3): rate limited…` |
| Cancelling | after `Esc` or `Ctrl-C` cancelled the turn, until it ends | `Cancelling…` |

- **Verb:** one of about 50 playful gerunds (`Pondering`, `Untangling`,
  `Percolating`, …), chosen at random when the turn starts and kept for the
  turn. With `tui.activity = "plain"` the two phases say `Thinking…` and
  `Writing…`.
- **Tool text:** the tool as a verb (`run_command` Running, `read_file`
  Reading, `edit_file` Editing, `write_file` Writing, `list_files` Listing,
  `search_files` Searching; other tools `Running NAME`) and the main argument,
  as on the call's summary line.
- **In brackets:**
  - The time since the turn started: `14s`, `2m 05s`, `1h 03m`. The TUI notes
    it when it sees `working` after `idle`; after an attach to a session that
    already works it asks `session_status` once for the start of the running
    turn. Until the answer the time is left out.
  - `↓ 1.2k tokens`: the output tokens of the turn. Between `usage` events
    they are estimated from the text received (4 characters per token), and
    the count of a `usage` event replaces the estimate. Left out while it is
    0.
  - `Esc cancel`, while the input is empty and has the focus, so that `Esc`
    really cancels; not while cancelling.
- **Room:** parts that do not fit are dropped from the right: the hint, then
  the tokens, then the time. Then the text is cut with `…`. The glyph stays.
- After the turn the line goes; no "done in 14s" entry is left, the
  `turn_end` notice is enough.

Animation:

- **Glyph:** `· ✢ ✳ ✶ ✻ ✽ ✻ ✶ ✳ ✢`, one per frame, in the `accent` colour;
  with ASCII symbols `| / - \`.
- **Shimmer:** the verb is drawn `dim`, with a band of three characters in
  the `text` colour that moves from left to right about once per two seconds.
  Not with `plain`, and with `NO_COLOR` the verb is plain text and only the
  glyph moves.
- **Stalled:** while thinking, when no event has come for 30 s, the glyph
  turns to the `warning` colour. The time stays. Nothing else changes: the
  provider's own timeouts and retries decide whether the request failed.

## Permission prompts

When a tool call needs permission, the session sends a permission question
with the call's details, the reason and the proposed rules (see
[`sessions.md`](sessions.md), "Questions", and [`tools.md`](tools.md),
"Questions"). The same question may be answered in another TUI or with
`clai chat`; the TUI then closes its prompt and shows who answered.

The TUI:

1. Expands the tool call and scrolls it into view, showing the question's
   details: diff or content, command parts with their decisions, working
   directory.
2. Shows the prompt bar with the reason (`asks: default for edit_file`) and
   the choices.
3. Takes keys for the prompt bar only, until the user answers or cancels.
   What answers nothing still works: `PgUp` / `PgDn`, `Ctrl-Home` / `Ctrl-End`
   and the mouse wheel scroll, to read what is asked; `Ctrl-N` / `Ctrl-P`
   attach to another session and `Ctrl-S` opens the session list over the
   prompt, which then takes the keys until it closes; `Ctrl-D` detaches. The
   question stays open, for later or for another client: attaching to the
   session again shows the prompt again. `Ctrl-C` acts like `Esc`.

| Key   | Action |
|-------|--------|
| `y`   | Allow this call once. |
| `a`   | Allow this call and add an allow rule for the session. The prompt bar shows the proposed rule (this command, this file); `Tab` widens it step by step (e.g. `cargo test`, `src/**`), `Enter` confirms, `Esc` goes back. See [`config.md`](config.md), "Session rules". |
| `n`   | Deny; the model is told the call was denied. |
| `e`   | Deny and type a short message to the model explaining why or what to do instead. |
| `Esc` | Deny and cancel the turn. |

Edits are shown as a unified diff with added lines green and removed lines
red. Commands are shown exactly as they will run, with the working directory.

Starting a language server (see [`tools.md`](tools.md), "lsp") is asked in
the same prompt bar, below the `lsp` call's own details:

```
Start language server rust-analyzer in ~/project?
command: rust-analyzer  It may run build scripts and macros of the project.
[y] yes  [n] no  [e] explain  [Esc] cancel
```

It has no `a`: "yes" already lasts for the session.

## Limit prompt

When a turn reaches its limit of model requests or tool calls, the session
waits (see [`sessions.md`](sessions.md), "Background work") and the TUI shows
the prompt bar:

```
Turn limit reached (50 requests, 120 tool calls). Continue?
[y] yes  [n] stop
```

| Key   | Action |
|-------|--------|
| `y`   | Continue the turn with a new budget of the same size. |
| `n`   | Stop the turn; a "limit reached" notice is added. |
| `Esc` | Same as `n`. |

The prompt takes keys only for the prompt bar, as for permission prompts.

## Questions from the model

A question from the model (`ask_user`, see [`tools.md`](tools.md),
"ask_user") opens as a box over the conversation instead of the prompt
bar, because it is larger. The tool call line shows `? ask_user Database`;
the session row counts the question in `●`, as for other questions.

```
┌ Question ─ 1/2 Database ─────────────────────────────────────────────────────┐
│ Both work with the current code; the choice decides the migration.           │
│                                                                              │
│ Which database should the cache use?                                         │
│                                                                              │
│ ▶ 1 SQLite (recommended)                                                     │
│       One file next to the binary, no server.                                │
│   2 Postgres                                                                 │
│       Shared with the API; needs the server running.                         │
│   3 Type an answer                                                           │
│                                                                              │
│ Up Down choose ─ Enter next ─ Tab question ─ Esc skip ─ Ctrl-C cancel turn   │
└──────────────────────────────────────────────────────────────────────────────┘
```

| Key | Action |
|-----|--------|
| `Up` / `Down` | Select an option or the "Type an answer" row. |
| `1`–`9` | Select that row; as `Space` on it. |
| `Space` | With `multiple`: tick or untick the option. Without: choose it. On the text row: type. |
| `Enter` | On an option: choose it (with `multiple`, go on with the ticked ones, or this one if none is ticked) and go to the next question. On the text row: type; `Enter` again keeps the text and goes on, `Esc` goes back. |
| `Tab` / `Shift-Tab` | Next or previous question, keeping what was chosen; after the last, the review page. |
| `PgUp` / `PgDn` | Scroll the box when it is taller than the screen. |
| `Esc` | Skip: answers `decline`; asks `[y] yes  [n] no` first when something was chosen. |
| `Ctrl-C` | Cancel the turn (`cancel`). |

- With `multiple`, options show `[x]` or `[ ]`; a chosen option is drawn
  in the accent colour.
- With one question and no `multiple`, `Enter` on an option sends at once.
  Otherwise the review page after the last question lists each answer;
  `Enter` sends, or goes to the first question without an answer.
- A text on the row of a question that has a choice is sent with it ("The
  user added").
- `context` and the model's `text` are drawn as Markdown.
- While the box is open the keys belong to it. `Ctrl-N` / `Ctrl-P`,
  `Ctrl-S` and `Ctrl-D` (when no text is typed) work as during a permission
  prompt; the question stays open. What was chosen is kept while the TUI
  stays on the session; switching to another session drops it.
- A rejected answer shows `not accepted: …` at the top of the box.
- When another client answers, the box closes and the conversation shows
  `answered by clai chat: reply`.

## Input editor

- `Enter` sends the message; `Shift-Enter` or `Alt-Enter` inserts a newline.
  **(assumption: terminals that cannot report `Shift-Enter` use `Alt-Enter`)**
- Standard line editing: arrows, `Home`/`End`, `Ctrl-A`/`Ctrl-E`,
  `Ctrl-W` delete word, `Ctrl-U` delete to line start.
- `Up`/`Down` on the first/last line of the input browse previous messages of
  this session.
- Pasted text is inserted as is, including newlines (bracketed paste); it is
  not sent until `Enter`.
- The user may type and send while a turn runs. The message goes to the server
  at once, which queues it until the turn ends (other clients see the queue
  too); the TUI shows it as "queued" until the turn that takes it starts.
- Lines starting with `/` are commands for `clai`, not messages to the model:

| Command    | Action |
|------------|--------|
| `/help`    | Show keys and commands. |
| `/clear`   | Start a new conversation in the same session. |
| `/model [PROVIDER/MODEL]` | Change the session's provider and model; without an argument, open the model picker. With `--default` (before or after the model), also save it as `default_model`; `/model --default` saves the session's model without changing it. |
| `/permissions` | Show the permission rules in effect: configuration, project file and session rules, with their source. |
| `/sessions` | Open the session list. |
| `/new [NAME]` | Start a new session in the same workspace and attach to it; the model picker opens on `default_model` first. |
| `/attach [NAME]` | Attach to another session; without a name, open the session list. A name that is not in the list stays in the input with a notice. |
| `/rename NAME` | Rename the attached session. |
| `/status [NAME]` | Open the status box: the full status of the attached session, or of session `NAME`, running or saved, and its statistics. See [Status box](#status-box). |
| `/stats [RANGE]` | Open the status box on its Global tab: statistics over all sessions of the server, for `7d` (the default), `30d` or `all`. See [Global tab](#global-tab). |
| `/lsp` | Open the language server box: the servers of the clai server, their state and the sessions using them; restart or stop one. See [Language servers](#language-servers). |
| `/settings` | Open the settings box: the `[tui]` settings, changed with a preview and saved into the user file. See [Settings](#settings). |
| `/theme [NAME]` | Open the theme list; with a name, switch to that theme. Either way the theme is saved as the default. See [Theme list](#theme-list). |
| `/kill`, `/delete` | Delete the attached session, after the confirmation of [Delete](#delete), and open the session list. |
| `/commit [HINT]` | Generate a commit message for all changes in the workspace and commit them after confirmation; see [Commit](#commit) and [`commit.md`](commit.md). |
| `/copy [N\|code]` | Copy the last answer of the model as Markdown; `N` the answer N back (`/copy 2` is the one before the last); `code` the last code block of the last answer. See [Copy](#copy). |
| `/diff` | Open the details view with the diffs of all edits and writes of the last turn, one after the other. |
| `/detach`, `/quit` | Detach and exit the TUI; the session keeps running. |

### Command list

Typing `/` into the empty input, with focus on the input, opens the command
list above the input. Pasted text and a message from the history do not open it.

```
──────────────────────────────────────────────────────────────────────────────
▶ /model [PROVIDER/MODEL]  Change the model; without one, choose it
  /permissions             later version
  /rename NAME             Rename the session
  /commit [HINT]           later version
──────────────────────────────────────────────────────────────────────────────
> /m_
──────────────────────────────────────────────────────────────────────────────
```

- The list shows every command with its arguments and a one-line
  description, in the order of the table above. A command that is not built yet
  is dim and says `later version`; `Enter` on it says so too.
- It lies over the last rows of the conversation and shows at most 8 rows; the
  selected row stays visible. The layout does not change, so the conversation
  does not jump.
- Each further character searches the list: commands with a name that contains
  the typed text, in any case, those that start with it first, each in table
  order. The best match is selected. `/quit` shares a row with `/detach`, and
  either name finds it.
- If nothing matches, the list shows "no command" and `Enter` sends nothing;
  the input stays so it can be corrected. To send a message that starts with
  `/` to the model, start it with `//`; the first `/` is removed, and the second
  `/` closes the list. **(assumption)**
- A space or a new line closes the list too: arguments follow.
- A question that opens, and `Ctrl-O`, close the list and keep the text. Keys
  belong to the prompt bar or the picker then.
- A closed list opens again only from an empty input. With it closed, `Enter`
  runs the text as it stands; an unknown command gets a notice and stays in the
  input, and `/rename` without a name gets its usage.

| Key in list         | Action |
|---------------------|--------|
| `Up` / `Down`       | Select the previous / next command; they stop at the ends. |
| `Tab`               | Complete the selected command into the input and close the list, so arguments can be typed. With no match, nothing. Focus does not change. |
| `Enter`             | Run the selected command. If it needs an argument, complete it like `Tab` instead. |
| `Esc`               | Close the list and nothing else: the typed text stays and a running turn goes on. A second `Esc` cancels the turn. |
| `Backspace` on the `/` | Delete the `/` and close the list. |

Other keys work as without the list: the conversation scrolls, and `Ctrl-C`
cancels the turn or clears the input, which closes the list.

Every box the user can open has a command in the table above. The prompt bar has
none: it opens when the agent asks a question.

After `/attach `, `/model `, `/stats `, `/status ` and `/theme ` (typed, or
completed with `Tab`), a list of values lies in the same place: `/attach` offers
the other sessions, with their workspace and state, `/model` the server's
models, with their id, `/stats` the ranges, `/status` the sessions,
`/theme` the themes, with where they come from. Typing searches them like
commands, by name, and models also by id. `Up` / `Down` select, `Tab` completes
the selected value, `Enter` runs the command with it, and with no match
(`no match`) `Enter` runs the text as typed. `Esc` closes the list until the
input no longer is such a command. A second word closes it.

While the command list is open, `Up`/`Down` select in the list instead of
browsing the message history.

### Commit

`/commit [HINT]` commits all changes in the session's workspace with a
message written by the model. Nothing is committed without the user's
confirmation. Which changes are included and how the message is generated is
described in [`commit.md`](commit.md); `clai commit` on the command line uses
the same rules but only prints the message.

Steps:

1. Collect the changes of the workspace and generate the message (see
   [`commit.md`](commit.md)). If there is nothing to commit or the directory
   cannot be committed, a notice explains why and nothing else happens.
   The request and answer are not added to the conversation history; the
   conversation only gets a notice. **(assumption)**
2. Confirm: the message is shown in a dialog together with the list of files
   that will be committed:

   ```
   ┌ Commit 3 files in ~/ws/project ──────────────────────────────────────────┐
   │ Fix last-line loss in parse                                              │
   │                                                                          │
   │ The loop stopped one line early, so the last line of every input was     │
   │ dropped. Iterate over all lines and add a test for single-line input.    │
   │                                                                          │
   │ M src/parse.rs   A tests/parse_single_line.rs   M Cargo.lock             │
   ├──────────────────────────────────────────────────────────────────────────┤
   │ Enter commit ─ e edit ─ r regenerate ─ d diff ─ Esc cancel               │
   └──────────────────────────────────────────────────────────────────────────┘
   ```

   | Key     | Action |
   |---------|--------|
   | `Enter` | Commit with the shown message. |
   | `e`     | Edit the message in the dialog (multi-line editor, same keys as the input). |
   | `r`     | Generate a new message. |
   | `d`     | Show the full diff that will be committed. |
   | `Esc`   | Cancel; nothing is staged or committed. |

3. Commit: stage the changes of the workspace (`git add -A -- <workspace>`)
   and commit only them (`git commit -F - -- <workspace>`). Changes already
   staged outside the workspace stay staged but are not committed. Git hooks
   run as usual; `clai` never passes `--no-verify`.
4. Result: the conversation gets a notice with the short hash and subject. If
   git fails (hook rejects, no `user.name`), its output is shown and the
   dialog opens again with the message, so the user can retry or cancel.

`/commit` never pushes, amends, or changes branches. If files change while
the dialog is open, `Enter` first shows "changes differ from the generated
message" and offers `r` regenerate or `Enter` commit anyway.

With a remote provider, the dialog header names the provider that received
the diff.

## Keys

| Key                  | Action |
|----------------------|--------|
| `Esc` / `Ctrl-C`     | Cancel the running turn. With no running turn, `Ctrl-C` clears the input. |
| `Ctrl-C` twice within 1 s, or `Ctrl-D` with empty input | Detach and exit the TUI; the session keeps running. |
| `Ctrl-S`             | Open the session list. |
| `Ctrl-O`             | Open the model picker for the attached session. |
| `Ctrl-N` / `Ctrl-P`  | Attach to the next / previous session in the list, round the end. With one session, nothing. |
| `PgUp` / `PgDn`      | Scroll the conversation. |
| `Ctrl-Home` / `Ctrl-End` | Jump to start / end of the conversation. |
| `Tab`                | Move focus between input and conversation. |
| `Enter` (conversation focus) | Expand or collapse the selected tool call. |
| `Up` / `Down` (conversation focus) | Select the previous / next entry. |
| `o` (conversation focus) | Open the selected tool call in the [details view](#details-view). |
| `y` / `Y` (conversation focus) | Copy the selected entry / the last code block of the selected answer. See [Copy](#copy). |
| `?` (conversation focus) | Show help. |
| `Ctrl-Y`             | Copy the last answer, as `/copy`. |
| `Ctrl-T`             | Show or hide the previews of the tool calls of the last turn. |
| Click on a tool call | Expand or collapse it, as `Enter`; the focus does not move. A click on its `… N lines` row opens the details view. |

`Ctrl-Z` does nothing for now: the TUI does not suspend.

While new output arrives, the view follows the end of the conversation,
unless the user scrolled up; then it stays put and the status line shows
"↓ new output". `Ctrl-End` resumes following.

Mouse wheel scrolls the conversation, a drag selects text and copies it (see
[Copy](#copy)). Mouse capture can be turned off with `tui.mouse = false` so
the terminal's own text selection works (see [`config.md`](config.md), "TUI");
with it on, `Shift`-drag uses the terminal's selection in most terminals.

With the focus on the conversation, the footer says `Enter expand ─ o open`
when a call is selected, and `y copy`, `? help` and, with the mouse on,
`drag copy`.

## Copy

```
> fix the failing test

The test fails because `parse` drops the last line. Let me look.
for i in 0..lines.len() {            ← selected text in reverse colours
▸ read_file src/parse.rs                                          ✓ 120 lines
```

- **Selection:** with `tui.mouse = true`, a left-button drag in the
  conversation or the details view selects text, shown in reverse colours;
  the release copies it. The selection stays shown until `Esc`, a key or a
  click clears it; `Esc` then does nothing else. A double click selects and
  copies a word (letters, digits and `_-./:~@#%+`, without a `.` or `:` at
  its end), a triple click the line with its wrapped rows. Dragging above or
  below the area scrolls it, so a selection can be longer than the screen.
  While the view follows the end, a selection stops the following until it is
  cleared, so the text does not move under the mouse.
- **What is copied** is the text as it is read, not as it is drawn: rows the
  TUI wrapped are joined again; the markers of the TUI (`▸ ▾`, the `> ` of a
  message, quote marks, indentation, the state of a call, `… N more lines`,
  rules) are left out; list markers stay; trailing spaces go. Code blocks are
  drawn without fences, so their rows copy as their source.
- **`y`** copies the selected entry: a message or answer as its Markdown
  source; for a tool call:

  | Tool | Text |
  |------|------|
  | `edit_file`, `write_file` | The diff as a unified diff with `a/PATH` and `b/PATH` headers, so it applies with `git apply`; a new file is its content. Without details (a log of an older `clai`), the old and new text as `-` and `+` lines. |
  | `run_command` | `$ command`, then the output with its exit code. |
  | others, and failed calls | The result. |

- **`Y`** copies the last code block of the selected answer, or of the last
  answer when the selected entry has none. **`/copy`** and **`Ctrl-Y`** copy
  the last answer, `/copy N` the answer N back, `/copy code` its last code
  block.
- **The clipboard:** `tui.clipboard` ([`config.md`](config.md), "TUI").
  `auto` writes the terminal's OSC 52 sequence (which works over SSH; in tmux
  it needs `set-clipboard on`) and, when the session is local (neither
  `SSH_TTY` nor `SSH_CONNECTION` is set), also gives the text to `wl-copy`
  (Wayland), `xclip` or `xsel` (X11) or `pbcopy` (macOS), the first one
  installed. `osc52` and `command` use only one of the two; `off` copies
  nothing and says so. OSC 52 carries at most 100 kB of text; the rest is
  left out of it.
- After a copy the footer says `copied 12 lines` for two seconds. The TUI
  cannot know whether the terminal took the text, so it says `copied`. A
  clipboard command that cannot be started says `cannot copy: …`.

## Details view

`o` on a selected call, a click on its `… N lines` row, or `/diff` opens a
view over the whole screen:

```
┌ edit_file src/parse.rs ─ ✓ edited src/parse.rs: replaced 1 occurrence ─────┐
│ @@ -12,3 +12,3 @@                                                          │
│      let lines: Vec<&str> = text.lines().collect();                        │
│ -    for i in 0..lines.len() - 1 {                                         │
│ +    for i in 0..lines.len() {                                             │
│          out.push(parse_line(lines[i])?);                                  │
│                                                                            │
│ Up Down scroll ─ / search ─ n next ─ y copy ─ w wrap ─ Esc close           │
└────────────────────────────────────────────────────────────────────────────┘
```

- It shows all of the call, as its expanded entry does without the limit:
  for an edit the diff with its context, for a command the command, its
  directory and the whole output with the exit code. The output is the one
  the tool gave; a text cut at 1 MiB is said under it in the warning colour.
- `/diff` shows the diffs of the edits and writes of the last turn, each under
  its tool and file; the title counts the files. Without any, a notice says
  `the last turn changed no file`.
- Keys: `Up` / `Down` (also `k` / `j`), `PgUp` / `PgDn` (also `Space`),
  `Home` / `End` scroll; `/` types a search, `Enter` finds it (in any case)
  and `Esc` drops it; `n` / `N` find the next / previous match, round the end;
  the line found is marked, and `not found: …` says when there is none. `y`
  copies all of it, `w` turns wrapping off and on (without it, `Left` /
  `Right` scroll sideways), `Esc`, `q` or `Ctrl-C` close. The mouse wheel
  scrolls it and a drag selects and copies in it, as in the conversation.
- A question that opens closes it, as it closes the other boxes; so does a
  switch to another session.

## Cancellation and exit

- Cancelling a turn stops the model request and kills a running command
  (process group, `SIGTERM`, then `SIGKILL` after 2 s). Tool calls already
  done stay done; a notice is added to the conversation.
- Exiting the TUI only detaches; a running turn continues on the server. To
  stop it, cancel the turn or kill the session.
- On exit, the terminal is restored before anything else is printed. A panic
  hook and signal handlers (`SIGTERM`, `SIGHUP`) restore it too.

## Status box

The status box has two tabs, named in its title: the session's name, and
**Global**. `/status` opens it on the first, `/stats` on the second.

`/status [NAME]` opens a box with the status of a session and statistics over
its whole log, from the `session_status` request (see
[`architecture.md`](architecture.md), "Protocol"). `NAME` is completed from the
session list, the attached session included; a name that is not in the list
stays in the input with a notice. A saved session is shown without being
resumed. The box opens at once ("asking the server…") and fills with the
answer; a first answer that fails closes it with the error.

```
┌ Status ─ project │ Global ───────────────────────────────────────────────────┐
│ Session                                                                      │
│   name         project  (id 1)                                               │
│   state        working                                                       │
│   workspace    ~/ws/project                                                  │
│   extra dirs   ~/ws/shared-lib  ⚠ missing                                    │
│   created      2026-11-01 01:00  (2h 10m ago)                                │
│   last active  2h 08m ago                                                    │
│   clients      2: chat pts/5 (watching), tui pts/3 (this)                    │
│   queued       1 message                                                     │
│   saved in     ~/.local/state/clai/sessions/1  (1.2 MB)                      │
│                                                                              │
│ Model                                                                        │
│   model        anthropic/sonnet  (claude-sonnet-5-5)                         │
│   api          anthropic  https://api.anthropic.com                          │
│   key          set in the environment                                        │
│   context      170k / 200k  █████████████████░░░  85%                        │
│   settings     temperature default ─ max output default ─ max_retries 3      │
│   changes      1; first anthropic/haiku                                      │
│                                                                              │
│ This turn                                                                    │
│   requests     0 / 50                                                        │
│   tool calls   0 / 120                                                       │
│   running      2h 08m                                                        │
│                                                                              │
│ Totals                                                                       │
│   turns        1: 1 done                                                     │
│   requests     1 ─ 1 retry                                                   │
│   tokens       in 2.1M ─ out 84.3k                                           │
│   working      2h 09m of 2h 10m                                              │
│                                                                              │
│ Tool calls     calls  done  error  denied  cancelled                         │
│   read_file        2     2      0       0          0                         │
│   edit_file        1     0      0       1          0                         │
│   all              3     2      0       1          0                         │
│                                                                              │
│ Questions      1: 1 yes                                                      │
│                by tui pts/3 1                                                │
│                                                                              │
│ Server         0.4.0 ─ pid 4242 ─ 3 sessions, 2 running ─ 3 clients          │
│                                                                              │
│ Tab global ─ Up Down PgUp PgDn scroll ─ Esc close                            │
└──────────────────────────────────────────────────────────────────────────────┘
```

- Rows that do not apply are left out: extra dirs when there are none, the
  clients and "This turn" of a saved session, "queued" without a queue,
  "changes" when the model never changed. "This turn" is shown while a turn
  runs; its counts start again after `continue` on a limit question.
- The model rows are the configuration in effect now for the session's
  workspace; a model that is no longer configured is marked. The key row says
  where the key comes from; `api_key_cmd` is not run. The context row turns to
  the warning colour from 80 % of the context window on.
- Times come from the event log. A log of an older `clai` has none: its
  stretches count as unknown, and "working" then shows `≥`; a session with no
  known time shows `?`. "working" counts the time in `working`, not the time
  waiting for an answer.
- "requests" counts the model requests that answered: from `usage` events, and
  from the counts at the end of a turn for requests that reported no usage.
  "retries" counts `retry` events, "error" the `error` events.
- The box updates while it is open: every second while the session works, and
  at the next second after an event of the attached session changed it. It
  does not poll an idle session. A refresh that fails keeps what is shown.
- `Up` / `Down`, `PgUp` / `PgDn` and `Home` / `End` scroll a box taller than
  the screen; `Esc`, `q` and `Ctrl-C` close it. A question that opens closes
  the box, as it closes the other boxes; so does a switch to another session,
  and the end of the session it shows.
- With ASCII symbols, the bar is `[####......]`, separators are `-` and the
  warning mark is `!`.
- `Tab`, `Shift-Tab`, `Left` and `Right` switch the tab. Each tab keeps its
  scroll position. The tab in use is bold in the title. A box opened by
  `/stats` asks for the session's status when its tab is first shown.

### Global tab

`/stats [RANGE]` opens the status box on the Global tab: statistics over the
logs of all saved sessions of the server, from the `global_stats` request (see
[`architecture.md`](architecture.md), "Protocol"; how they are counted is in
[`sessions.md`](sessions.md), "Statistics"). `RANGE` is `7d` (the default),
`30d` or `all`; another word stays in the input with its usage. It works for
all sessions, whichever is attached.

```
┌ Status ─ project │ Global ─────────────────────────── 7 days │ 30 days │ all ┐
│ Totals         2026-11-04 – 2026-11-10                                       │
│   sessions     12 active of 31 ─ 4 running                                   │
│   turns        87: 80 done, 3 cancelled, 2 limit, 2 failed                   │
│   requests     1.4k ─ 21 retries ─ 3 errors                                  │
│   tokens       in 10.9M ─ out 340k                                           │
│   working      6h 40m                                                        │
│                                                                              │
│ Models            sessions  requests      in     out   share                 │
│   anthropic/sonnet       9      1.1k    9.9M    300k   ██████████░░  90%     │
│   openai/gpt-old         4       300      1M     40k   █░░░░░░░░░░░   9%     │
│   all                   12      1.4k   10.9M    340k                         │
│                                                                              │
│ Tokens per day  █ in  █ out                                                  │
│    5M ┤                                                                      │
│       │                                    ▅▅▅▅▅▅▅▅                          │
│       │                                    ████████                          │
│       │                                    ████████                          │
│  2.5M ┤         ▆▆▆▆▆▆▆▆                   ████████ ▂▂▂▂▂▂▂▂                 │
│       │         ████████                   ████████ ████████                 │
│       │▇▇▇▇▇▇▇▇ ████████          ▄▄▄▄▄▄▄▄ ████████ ████████                 │
│       │████████ ████████          ████████ ████████ ████████ ▇▇▇▇▇▇▇▇        │
│     0 ┼───────────────────────────────────────────────────────────────       │
│        Wed      Thu      Fri      Sat      Sun      Mon      Tue             │
│        max 4.1M on Sun 11-08 ─ avg 1.6M                                      │
│                                                                              │
│ Tab session ─ 7 3 a range ─ R count again ─ Up Down scroll ─ Esc close       │
└──────────────────────────────────────────────────────────────────────────────┘
```

- **Range:** the last 7 or 30 days, today included, or all time, in the
  client's time zone. At the right of the title, the range in use is bold; a
  box too narrow for them names only that one after the tabs. `7`, `3` and `a`
  choose it, `r` goes to the next one. Each answer is kept while the box is
  open, so a range shown before comes back at once.
- **Totals:** the dates of the range; the sessions with an event in it, of all
  saved sessions, and how many run now; turns, requests, retries, errors and
  tokens as on the Session tab; the time in `working` in the range, with `≥`
  when some stretch has no time. A log that cannot be read is left out and
  counted in a warning row "skipped".
- **Models:** one row per `provider/model`, most tokens first: the sessions
  that used it in the range, its requests, input and output tokens, and its
  share of all tokens with a bar in the `user` colour (`[###.....]` in ASCII).
  A model that is no longer configured is dim; tokens that cannot be given to a
  model are in a row `unknown`. With more than one model, a row `all` sums
  them. No model used: `Models  none`.
- **Tokens per day:** a bar chart, input (`user` colour) and output (`accent`
  colour) stacked, 8 rows high, the top of a bar in eighth blocks; in ASCII
  `#` and `+` in whole cells. The y axis is labelled at 0, the middle and the
  top, which is 1, 2, 4, 5 or 8 times a power of ten. One column per day, as
  wide as the box allows (at most 8 cells and a gap); days without tokens are
  empty columns. Labels: the weekday for 7 days, `MM-DD` every 7th day for 30
  days. For all time, a column is a day while they fit, then a week (from
  Monday), then a month (`YYYY-MM`; the oldest are left out when even those do
  not fit); the line under the chart says so. That line names the highest
  column and the mean per day of the range. Tokens of events without a time
  are not in the chart; a line says how many there are. A range without tokens
  says `no tokens in this range`; a box narrower than 40 cells leaves the
  chart out and says so.
- **Asking:** the tab asks with `global_stats` when it is shown and the range
  has no answer yet, and when the range changes ("counting the logs of all
  sessions…" until it comes). It does not poll: `R` counts again, and the
  title says how old an answer is once it is older than a minute
  (`Global ─ 2m 30s old`). A refresh that fails keeps what is shown; a first
  answer that fails shows the error in the tab, or closes the box with it when
  nothing else is shown.

## Settings

`/settings` opens a box with the `[tui]` keys of the user file
([`config.md`](config.md), "TUI"):

```
┌ Settings ─ ~/.config/clai/config.toml ───────────────────────────────────────┐
│   theme              terminal  Colour theme                                  │
│   symbols            auto      Unicode or ASCII symbols                      │
│ ▶ mouse             *on        Capture the mouse: wheel, drag copies         │
│   tool_output_lines  20        Lines of tool output before it is cut         │
│   tool_preview       changes   What finished tool calls show                 │
│   clipboard          auto      How copied text reaches the clipboard         │
│   syntax_highlight   on        Colour code by its language                   │
│   timestamps         off       Time of each entry                            │
│   scroll_lines       3         Lines per mouse-wheel step                    │
│   max_fps            30        Redraw limit                                  │
│   activity           playful   Line while the session works                  │
│ mouse: on: the wheel scrolls, a drag selects and copies, a click opens a cal…│
│ Enter change ─ Left/Right value ─ d default ─ s save ─ Esc close             │
└──────────────────────────────────────────────────────────────────────────────┘
```

- One row per key, with the value and a description; the line under the list
  explains the selected key. A key the file does not set shows its default,
  dim. `*` marks a value changed and not saved.
- The `theme` row says when something else decides it: `⚠ CLAI_THEME=nord for
  this run`, or `⚠ colours are off (NO_COLOR)`. `symbols = unicode` in a locale
  that is not UTF-8 is marked too, and still applied.
- **Preview:** a change applies to this TUI at once; other TUIs see it only once
  it is saved.
- The title names the file `s` writes: the user file of this TUI (`--config` or
  the default path).

| Key | Action |
|-----|--------|
| `Up` / `Down` | Select a row. |
| `Left` / `Right` | Previous / next value: on/off, one of a list, one step of a number, the themes in the order of the theme list. |
| `Enter`, `Space` | On/off: toggle. List: next value. Number: type it in the row; `Enter` takes it, `Esc` goes back, a value out of range says so and stays. Theme: open the [theme list](#theme-list); its `Enter` only sets the row. |
| `d` | Back to the default; the key is removed from the file on save. |
| `s` | Save all changes. |
| `Esc` | Close. With unsaved changes it asks `save changes? [s] save  [n] discard  [Esc] back`; discarding puts the file's values back. |

Saving (also for the theme list):

- The keys are set or removed in `[tui]` of the user file; the table is added if
  it is missing. Everything else stays as it is, comments and order included.
- The new text is loaded like at start first. If that fails, nothing is written
  and the box shows the error.
- If the file changed since it was read, nothing is written either:
  `the file changed since it was read; open it again`.
- The file is replaced atomically with its mode kept; a link is written through
  to its target. The server notices it and sends `config_reloaded`; every TUI
  then applies the new `[tui]`.

Opening the box or the theme list reads the user file and the theme files
again, so a theme file edited on its own is listed. A question that opens
closes the box and the theme list and undoes their preview, as for the model
picker.

## Theme list

`/theme` opens a list of all themes ([`config.md`](config.md), "Themes"):

```
┌ Theme ───────────────────────────────────────────────────────────────────────┐
│ Filter:                                                                      │
│                                                                              │
│ built in                                                                     │
│   terminal         ✓ current                                                 │
│ ▶ dark                                                                       │
│   light                                                                      │
│ bundled                                                                      │
│   atom-one-dark    base dark                                                 │
│   nord             base dark                                                 │
│   gruvbox-dark     base dark                                                 │
│   solarized-light  base light                                                │
│ theme files                                                                  │
│   bad              ⚠ themes/bad.toml:1: base: …                              │
│                                                                              │
│ Enter take and save as default ─ Esc back to terminal                        │
└──────────────────────────────────────────────────────────────────────────────┘
```

- Groups: built in, bundled (a theme file that replaces one shows its path
  there), theme files by name, then the `[themes.NAME]` tables of the user file
  in file order. A broken theme file is listed dim with its error and cannot
  be taken. Typing filters by name; `Up` / `Down` select.
- **Preview:** the selected theme applies at once, on the real conversation.
  With the selection on a broken file or no match, the last theme shown stays.
  `Esc` goes back to the theme in effect when the list opened.
- `Enter` takes the theme and saves it into the user file as `tui.theme`, the
  default theme: every TUI started from then on uses it, and open ones switch
  to it on `config_reloaded`. Nothing is saved before `Enter`. Opened from the
  settings box, `Enter` only sets the row there.
- `/theme NAME` does the same without the list; an unknown name is a notice
  that names `/theme`, and the input stays. After `/theme ` the argument list
  offers the theme names.
- With `CLAI_THEME` set, the title says so. A theme taken in the list wins over
  it for the rest of the run, and is saved for later runs.
- With `NO_COLOR` the title says that colours are off.

## Language servers

`/lsp` opens a box with the language servers of the clai server (see
[`config.md`](config.md), "LSP"), one row each:

```
┌ Language servers ─────────────────────────────────────────────────────────┐
│ ▶ rust-analyzer  .         ready  ─ clai, fix                             │
│   pyright        ../docs   failed: exited with status 1                   │
│                                                                           │
│ Last lines of rust-analyzer:                                              │
│ INFO waiting for flycheck                                                 │
│                                                                           │
│ r restart ─ k stop ─ Up Down choose ─ Esc close                           │
└───────────────────────────────────────────────────────────────────────────┘
```

- A row has the server's name, its root (relative to the session's
  workspace, else absolute), its state (`starting`, `indexing 42 %`,
  `ready`, `stopped`, `failed: <reason>`) and the sessions that used it, by
  name. A failed server is red.
- Below the rows: the last lines of the selected server's standard error.
- `r` restarts the selected server (it forgets its crashes), `k` stops it
  (it starts again when the model uses it); the box shows the new list, or
  the error. `Enter` or `F5` asks again. `Up` / `Down` select; `Esc`, `q`
  and `Ctrl-C` close.
- With no server yet: "No language server has run. One starts when the
  model uses lsp."

`lsp` calls in the conversation show the operation, symbol and place
(`lsp references parse src/parse.rs:3`) and, when done, a count
(`✓ 3 references`, `✓ 1 error, 2 warnings`, `✓ 12 symbols`); see
[`tools.md`](tools.md), "lsp".

## Appearance

- Colours come from the theme set in `tui.theme`: `terminal` (default, the
  terminal's 16-colour palette, so it follows the terminal's own theme),
  the built-in `dark` and `light`, or a custom theme. `NO_COLOR` turns
  colours off. Themes and all other TUI settings: [`config.md`](config.md),
  "TUI".
- Unicode symbols (`✓ ✗ ▸ ▾`) fall back to ASCII (`+ x > v`) when the locale
  is not UTF-8, or always with `tui.symbols = "ascii"`.
- Resizing the terminal re-wraps all text; no content is lost.

### Syntax highlighting

With `tui.syntax_highlight = true` (the default), code is coloured by its
language:

| Place | Language from |
|-------|---------------|
| Code blocks of answers and of questions from the model (prompt bar, ask box) | The fence: ```` ```rust ````, `rs`, `py`, `sh`, `js`, … An indented block or an unknown name is not highlighted. |
| Diffs of `edit_file` and `write_file`: previews, expanded calls, permission questions, the details view, `/diff` | The file's extension (and `Makefile`, `Dockerfile`), else its first line (`#!/bin/sh`). |
| New files of `write_file` | As for diffs. |
| The `$ command` line of `run_command` | Shell. The output is not highlighted. |

- **Diffs:** the `+` and `-` keep the `diff_add` and `diff_remove` colours;
  the code after them is highlighted. Each hunk is highlighted twice, its old
  side (`-` and context lines) and its new side (`+` and context lines), so a
  comment opened on a `-` line does not colour the `+` lines. The `@@` line
  stays `accent`. With true colour and a theme that sets `background`,
  changed lines also get a tint of their diff colour behind them. A file of
  no known language keeps whole lines in the diff colours.
- **Colours** come from the syntax theme the theme's `code` key names
  ([`config.md`](config.md), "Themes"). Without one: `ansi` for `terminal`,
  `base16-ocean.dark` for `dark`, `InspiredGitHub` for `light`. The bundled
  themes use the syntax colours of the same scheme: `atom-one-dark`, `nord`
  and `gruvbox-dark` are `clai`'s own, made from each scheme's palette
  (keywords, strings, numbers, functions, types, variables, operators, tags,
  comments in italic), and `solarized-light` uses `Solarized (light)`. `ansi`
  is `clai`'s own too, on the terminal's 16 colours: keywords magenta, strings
  green, numbers and constants yellow, functions blue, types cyan, comments
  bright black and italic. Other syntax themes are
  exact with true colour and the nearest of the 256 colours otherwise. Only
  foregrounds, bold and italic are used, never a syntax theme's background.
  `NO_COLOR` turns it off.
- **Loading:** the grammars load in the background when the TUI starts; until
  then code is shown in its plain style, and it is laid out again when they
  are there.
- **Streaming:** an open code block is highlighted line by line as it grows;
  the line without its newline yet keeps the `code` style until it is
  complete. Highlighted texts are cached (the last 512), and a text that grows
  goes on from where it stopped.
- **Limits:** a block or diff side of more than 5000 lines, or with a line
  longer than 2000 characters, is not highlighted.
- Copying is not changed: colours are not copied.
- Languages: those `syntect` bundles (Rust, Python, JavaScript, Go, C, C++,
  Java, Shell, JSON, YAML, Markdown, HTML, CSS, SQL, Ruby, PHP, Makefile, …).
  TOML and TypeScript are not among them; a `ts` fence uses JavaScript.

## Performance

- Redraw at most 30 times per second (`tui.max_fps`); streaming text is
  batched between frames.
- An idle TUI wakes once per second. While the session works and the
  activity line or a tool call spinner is on the screen, a frame tick runs
  every 120 ms: it moves the glyph, the shimmer and the spinners, and only
  the activity line and the running calls are built again.
- Input never waits for the network or a tool: the UI runs on its own task and
  receives events from the server over the client connection.
- Switching to a session with a long history shows its latest page within
  200 ms; older history loads in the background. **(assumption)** The first
  version replays the whole log before its first frame; paged loading is built
  if that turns out to be too slow.
- A session with 10 000 conversation lines scrolls without visible delay.

## Implementation notes

- Library: `ratatui` with the `crossterm` backend.
- The TUI only displays state and sends user actions to the server; it does
  not call the provider or run tools itself. The agent loop runs in the
  server and does not depend on the UI, so one-shot mode and tests can run
  without a terminal.
- Rendering is tested with `ratatui`'s test backend: given a state, the
  produced screen buffer is compared with an expected one.

## Open questions

1. Is an external editor (`Ctrl-G` opens `$EDITOR` for the input) wanted?
2. Answered ([RFC 0006](../rfcs/0006-tui-settings.md)): `/theme [NAME]`
   switches the theme and saves it as the default; see [Theme list](#theme-list).
3. Should keys be configurable (`[tui.keys]`)?
