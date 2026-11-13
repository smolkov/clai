# Conversation layout: markers, short tool names, clicks, questions

- RFC: 0022
- Status: done
- Milestone: none; it extends the conversation of RFC 0001 and RFC 0016
- Spec: [`spec/tui.md`](../spec/tui.md), "Layout", "Conversation entries",
  "Keys", "Copy", "Appearance", "Questions from the model";
  [`spec/tools.md`](../spec/tools.md), "Display"

## Summary

Four changes to how the conversation looks and reacts:

1. **A marker for answers:** model text gets its own symbol on the left
   (`●`), different from the markers of tool calls (`▸ ▾`).
2. **Short tool names:** the conversation shows `edit`, `run`, `read`, `ls`,
   `search` instead of `edit_file`, `run_command`, `read_file`, `list_files`,
   `search_files`. Only the display changes; the model, the config and the
   permission rules keep the real names.
3. **Clicks on tool calls:** one click opens the short view, the next click
   closes it; a double click opens the full view, the next click closes it.
4. **Questions at the bottom:** a question from the model (`ask_user`)
   opens at the bottom of the screen, in the place of the input, instead
   of a box in the middle of the conversation. The input is covered while
   the question is open and comes back, with its text, when it closes.

## Motivation

- Model text has no marker, tool calls have `▸ ▾`, user messages `>`. A
  short answer between two calls looks like part of a call's output.
- `edit_file`, `run_command`, `search_files` are names for the model. On
  screen they take 10 to 12 columns of every call line, cut the main
  argument earlier, and say nothing the shorter word does not.
- A click on a call goes round collapsed, expanded and full. To see the
  whole output the user clicks twice, or three times when a preview shows,
  and has to know in which state the call is. The two views the user wants,
  "a quick look" and "everything", should each be one gesture.
- The question box (`src/tui/view/ask.rs`) opens as a popup in the middle
  of the screen. It hides the conversation the user needs to answer (the
  model's last text and calls), while the input below it stays visible
  but takes no keys. A permission question already uses the bottom (the
  prompt bar); a question from the model should open in the same place,
  where the user's eyes are when they type.

## Scope

In:

- The answer marker, its colour and its ASCII form.
- Short display names of the built-in tools, wherever the TUI shows a call.
- Single and double click on a call's summary line.
- What a selection copies of the new marker.
- Where the question of `ask_user` is drawn, and what happens to the
  input, the activity line and the conversation while it is open.

Out:

- The spacing between entries: it stays as it is (the user found it
  good).
- Renaming the tools for the model, in the config or in permission rules.
- `clai chat` and `clai run` output: they keep the long names (chosen by
  the user). They print plain lines that scripts may read.
- The details view (`o`): it stays as it is.
- The `Enter` key on a selected call: it keeps its cycle.
- The keys and content of the question (options, review page, typed
  answer, scrolling): RFC 0012 and RFC 0018. Only its place changes here.
- The permission prompt bar: already at the bottom.

## Design

### The new look

```
> fix the failing test

● The test fails because `parse` drops the last line. Let me look.

▸ read src/parse.rs                                               ✓ 120 lines
▾ edit src/parse.rs                                               ✓ +1 −1
  @@ -12,3 +12,3 @@
  - for i in 0..lines.len() - 1 {
  + for i in 0..lines.len() {
▸ run cargo test parse                                            ✓ exit 0

● Fixed: the loop now reads the last line, and the tests pass.

> thanks, now commit it
```

### Markers

| Entry | Marker | ASCII | Colour |
|-------|--------|-------|--------|
| User message | `>` | `>` | user colour (as now) |
| Model text | `●` | `*` | the theme's accent colour **(proposal)** |
| Tool call, collapsed | `▸` | `>` | as now |
| Tool call, open | `▾` | `v` | as now |

- The marker stands on the first row of an answer; the other rows are
  indented by two columns, so the text forms one column next to it, as the
  details of a call do below its summary.
- An answer that is cut by tool calls (text, calls, text) gets the marker
  again on each part, since each part is its own entry.
- While an answer streams, the marker is there from its first row; it does
  not blink or spin. The activity line already says that the model works.
- A code block in an answer is indented with the text.
- `●` (chosen by the user) is also the waiting-sessions count of the
  session row (`●1`). The two never meet: one is at the left edge of the
  conversation, the other inside the session row at the bottom.

### Short tool names

| Tool | Shown as |
|------|----------|
| `read_file` | `read` |
| `list_files` | `ls` |
| `search_files` | `search` |
| `edit_file` | `edit` |
| `write_file` | `write` (chosen by the user) |
| `run_command` | `run` |
| `ask_user` | not shown as a call (as now) |
| others (MCP tools, the language server tool of RFC 0019) | their own name, until they get a short one |

- The short name is shown on the call's summary line, in the prompt bar of
  a permission question, and in the title of the details view.
- Where the user types or reads a name the config uses, the real name
  stays: the rule texts of a permission question (`asks: default for
  edit_file`; chosen by the user), the rules `a` proposes
  (`edit_file(src/**)`), `/status`, `/settings`, error messages from the
  config. Otherwise the user would learn a name that does not work in the
  config.
- The names live in one table next to the tools (`Tool::label`), so a new
  tool brings its own.
- No setting to bring the long names back **(proposal)**.

### Clicks on a tool call

A call is in one of three views; a click on its summary line moves it:

| View now | Click | Double click |
|----------|-------|--------------|
| Closed (`▸`) | short | full |
| Short (`▾`) | closed | full |
| Full (`▾`) | closed | closed |

- **Short** is what a quick look needs: the preview of the tool (the diff,
  the last lines of a command, the error), or for tools without a preview
  (`read`, `ls`, `search`) their details cut at `tui.tool_output_lines`. A
  call that shows a preview by `tui.tool_preview` counts as short, so the
  first click closes it.
- **Full** is the whole result in the conversation, without a cut, as the
  `Full` state today.
- **Double click without a delay:** the first click acts at once (closed →
  short, or short → closed), and the second click of the double click,
  on the same row within the double-click time, turns the call to full.
  So a single click never waits, and a double click always ends in full,
  whatever the view was. The double-click time is the one the selection
  already uses for words.
- A double click on the summary line of a call no longer selects a word
  there; on every other row (text, a call's output) it still does. A triple
  click on a summary line counts as a double click and the next click
  **(proposal)**: full, then closed.
- A click on the `… N lines` row still opens the details view.
- The focus does not move, as now.
- `Enter` on a selected call keeps its cycle (closed, expanded, full), and
  `Ctrl-T` keeps its meaning. **(proposal)**

### Questions at the bottom

```
● Both work with the current code; the choice decides the migration.

? ask_user Database
── Question ─ 1/2 Database ───────────────────────────────────────────────────
Which database should the cache use?

▶ 1 SQLite (recommended)
      One file next to the binary, no server.
  2 Postgres
      Shared with the API; needs the server running.
  3 Type an answer

Up Down choose ─ Enter next ─ Tab question ─ Esc skip ─ Ctrl-C cancel turn
──────────────────────────────────────────────────────────────────────────────
  asking · Esc skip · /help · Ctrl-D detach
  project · ~/ws/project · anthropic/sonnet · 1/3 ●1 · 12.3k/200k · in 45.1k out 3.2k
```

- The question takes the rows of the input and of the rules around it,
  directly above the status row. It is drawn like the prompt bar: a rule
  with the title on top (`── Question ─ 1/2 Database ──`), a rule below,
  no side borders, the full width of the terminal. **(proposal)**
- **The input is covered** while the question is open: it is not drawn
  and takes no keys, as the keys belong to the question already. The text
  typed so far, the cursor and the history position are kept; when the
  question closes (answered, skipped, cancelled, or answered by another
  client), the input comes back as it was. Nothing typed is lost.
- **Height:** the question grows upwards as far as its rows need, up to
  the screen height minus the status and session rows and minus 3 rows
  of conversation **(proposal)**, so the model's last text or the
  `? ask_user` line stays in sight above it. When it needs more rows, it
  scrolls inside its area as RFC 0018 describes; the keys line stays its
  last row.
- **The conversation** shrinks to the rows above and follows its end, so
  the call that asks is the last line above the question. The user may
  scroll it with the wheel over it, or `PgUp` / `PgDn` after a click into
  it; the wheel over the question scrolls the question.
- **The activity line** is hidden while the question is open; the status
  row says `asking`, as for a permission question.
- `context` and the model's text stay part of the question, at its top,
  as now; on a short screen they scroll with it.
- The boxes that open over a question (`Ctrl-S`, the model picker, the
  session list) still lie over everything; when they close, the question
  is at the bottom again.
- Below 10 rows the "terminal too small" rule applies as before; at the
  smallest size the question gets all rows but the status and session
  rows.
- A permission question that opens while an `ask_user` question is open
  waits, as now (one question at a time).

This also gives the question the full width, one of the problems of
RFC 0018 ("Fixed width"); RFC 0018 then keeps only the scrolling and
content parts.

### Copy

- The `●` marker and the two-column indent of an answer are left out, as
  the other markers are.
- `y` on a tool call is unchanged; its text is about the call's content, not
  its name.

### Narrow terminals

At 40 columns, the shorter names leave more room for the main argument. The
marker and indent take two columns of each answer row; Markdown wraps
inside the remaining width.

## Spec changes

Written into the spec when the RFC is accepted:

- `spec/tui.md`:
  - "Layout": the example with the new look.
  - "Conversation entries": the answer marker, the short names in the table
    and the examples.
  - "Keys": click and double click on a call.
  - "Copy": the answer marker; double click on a summary line.
  - "Appearance": `●` and its ASCII form `*`.
  - "Layout": the question takes the place of the input; the table row
    of the input says it is covered while a question from the model is
    open.
  - "Questions from the model": the question at the bottom, its height,
    the covered input, the hidden activity line, the wheel.
- `spec/tools.md`, "Display": the short name of each tool, and where the
  real name stays.
- The examples in other sections of `spec/tui.md` and in `readme.md` use the
  short names.

## Testing

- **Markers:** snapshots of a short answer, a long answer that wraps, an
  answer with a code block and a list, a streaming answer; the same in ASCII.
- **Names:** a test over every built-in tool that it has a label; snapshots
  of the summary line, the prompt bar and the details view title; the rule
  texts and `a` proposals keep the real names.
- **Clicks:** App tests for click on closed, short and full calls; double
  click from closed and from short; a slow second click acts as a single
  click; a triple click; a double click on a text row still selects a word;
  a call with a preview closes on the first click.
- **Copy:** a selection across two answers and a call: the marker and the
  indent are left out.
- **Question:** snapshots of a short question (input covered, conversation
  above), a question taller than the screen (3 conversation rows kept,
  keys line visible), the smallest terminal, ASCII. App tests: text
  typed before the question is still in the input after answer, skip,
  cancel and an answer from another client; the wheel over the question
  scrolls it, over the conversation scrolls the conversation; the
  activity line is hidden while it is open; the picker opens over it and
  the question is back after `Esc`.
- **Check:** a pseudo-terminal run of the real TUI with a turn that reads,
  edits and runs a command; click and double click on the calls.

## Implementation plan

1. **Short tool names**
   - Status: done
   - Files: `src/tools/mod.rs`, `src/tui/conversation.rs`,
     `src/tui/view/` (prompt bar, details view), `src/tui/tests.rs`,
     snapshots
   - Result: `read`, `ls`, `search`, `edit`, `write`, `run` on screen; the
     real names in rules and config texts.
   - Tests: label table, snapshots.
   - Built: `Tool::label` and `tools::label` (any other name, such as
     `ask_user` or `lsp`, is shown as it is); the summary line, the details
     view title, `/diff` headings and the prompt bar use it. Choices made
     while building it:
     - The prompt bar replaces only the leading `edit_file:` of the
       question's summary; the summary itself, as sent by the server and
       shown by `clai chat`, keeps the real name.
     - The activity line already says `Editing …`, `Running …`; it is
       unchanged.
2. **The answer marker**
   - Status: done
   - Files: `src/tui/conversation.rs`, `src/tui/view/conversation.rs`,
     `src/tui/selection.rs`, `src/tui/tests.rs`, snapshots
   - Result: `●` and the indent, the copy rules.
   - Tests: snapshots, selection tests.
   - Built: `Symbols::answer` (`●`, `*`) in the accent colour, and
     `Conversation::answer`, which renders the Markdown two columns
     narrower and puts the marker or the indent before each row. Choices
     made while building it:
     - The marker and indent count as the row's prefix (`Source::prefix`),
       as a timestamp does, so the selection code needed no change; with
       `tui.timestamps` the time comes first, then the marker.
     - An empty answer (the first moment of a stream) is one row with the
       marker.
3. **Clicks**
   - Status: done
   - Files: `src/tui/app.rs`, `src/tui/conversation.rs`, `src/tui/tests.rs`
   - Result: click short / closed, double click full / closed.
   - Tests: App tests.
   - Built: `Conversation::click`, `double_click` and `view`; the click
     handling in `src/tui/app/copying.rs`. Choices made while building it:
     - A single click still acts on the release, so a drag that starts on
       a summary line selects text; the second click of a double click
       acts on its press.
     - The App keeps the call the last click turned and its view before
       (`last_call_click`), so the second click knows whether the call was
       full before the double click.
     - `Enter` keeps `Conversation::toggle`.
4. **Questions at the bottom**
   - Status: done
   - Files: `src/tui/view/mod.rs` (layout areas), `src/tui/view/ask.rs`,
     `src/tui/app.rs` (wheel), `src/tui/tests.rs`, snapshots
   - Result: the question in the place of the input; the input covered
     and kept; the conversation above it.
   - Tests: snapshots and App tests above.
   - Built: `Areas::ask` in `view::areas`; `view::ask::height`,
     `max_top` and `draw` into that area, with `titled_rule`; `App::ask_area`
     for the wheel. Choices made while building it:
     - The input's text, cursor and history were already kept while a
       question is open, and the activity line already hidden; only the
       layout changed.
     - The 3 rows above a tall question shrink on screens below 13 rows,
       to none at the smallest one (10 rows).
     - When the question scrolls, its keys line (one or more rows) stays
       at the bottom; the wheel and `PgUp` / `PgDn` scroll the rows above
       it, never past the end.
     - `PgUp` / `PgDn` scroll the question, as before; a click into the
       conversation does not move them to it. The wheel over the
       conversation scrolls the conversation.
5. **Spec and readme**
   - Status: done
   - Files: `spec/tui.md`, `spec/tools.md`, `readme.md`, `todo.md`, this
     RFC
   - Result: the spec describes the new look; status `done`.
   - Tests: `ci/check.sh`, `ci/test.sh`.
   - Built: `spec/tui.md` ("Layout", "Conversation entries", "Keys",
     "Copy", "Appearance", "Questions from the model", the examples) and
     `spec/tools.md` ("Display"); the readme's TUI section; `todo.md`.
     The check in a real terminal (see "Testing") is left in `todo.md`.

## Alternatives

- **Rename the tools for the model too.** One name everywhere, but it breaks
  every config with `[permissions]` rules, saved sessions and logs that
  name the old tools, and the long names tell the model more. The display
  name costs one table.
- **A box or a rule around each turn.** Clear, but costs two rows a turn and
  the side columns, and the spec avoids boxes in the conversation.
- **Wait for the double-click time before a single click acts.** No change
  of view before the second click, but every single click then feels slow.
- **Double click opens the details view.** `o` and the `… N lines` row do
  that already; the user asked for the full view in place.
- **The question at the bottom, with the input still visible below it.**
  Keeps the typed text in sight, but the input takes no keys while the
  question is open, and it costs three rows a short screen needs for the
  options.
- **The question as a box at the bottom** (borders on all sides). Looks
  like the other boxes, but costs two columns, and the prompt bar of a
  permission question has no side borders either.

## Open questions

1. ~~Empty rows between tool calls~~: dropped; the spacing stays as it is.
2. ~~The answer marker~~: answered, `●` (see "Markers").
3. ~~`write_file` shown as `write`~~: answered, yes (see "Short tool names").
4. ~~Names in the permission question~~: answered, the real name in its
   rule text (see "Short tool names").
5. ~~`Enter` and the clicks~~: answered, `Enter` keeps its cycle.
6. ~~`clai chat`~~: answered, it keeps the long names (see "Scope").
7. ~~Where the question of `ask_user` opens~~: answered, at the bottom,
   covering the input (see "Questions at the bottom").
8. ~~Rows above a tall question~~: answered, 3.

## Answers

1. The spacing between entries needs no change, said by the user; it is
   out of scope.
2. The answer marker is `●`, chosen by the user.
3. `write_file` is shown as `write`, chosen by the user.
4. The rule texts of a permission question keep the real tool name, chosen
   by the user.
6. `clai chat` and `clai run` keep the long names, chosen by the user.
5. `Enter` on a selected call keeps its cycle (closed, expanded, full),
   chosen by the user.
7. A question from the model opens at the bottom and covers the input
   while it is open, chosen by the user.
8. 3 conversation rows stay above a question taller than the screen,
   chosen by the user.

The other proposals were accepted as written: `●` in the accent colour,
no setting for the long names, a triple click on a summary line as a
double click and the next click, and the question drawn like the prompt
bar.
