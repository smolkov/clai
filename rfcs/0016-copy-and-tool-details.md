# Copy text and see tool details

- RFC: 0016
- Status: draft
- Spec: [`spec/tui.md`](../spec/tui.md), "Conversation entries", "Keys";
  [`spec/config.md`](../spec/config.md), "TUI"

## Summary

Two things the user cannot do well in the TUI today:

1. **Copy text from the conversation.** The TUI captures the mouse, so a drag
   does not select. `Shift`-drag works in most terminals, but nothing says so,
   and it copies the screen as drawn: across the side markers, cut at the
   screen edge, with the wrap of the window.
2. **See what a tool did after it finished.** A finished `edit_file`,
   `write_file` or `run_command` collapses to one line. The diff and the output
   are kept, but opening them needs `Tab`, arrows to select the call, and
   `Enter`. Few users find this.

This RFC adds:

- **Selection with the mouse** in the conversation, copied to the clipboard
  when the button is released.
- **Copy keys:** `y` copies the selected entry, `/copy` copies the last answer
  or a code block.
- **Tool previews:** finished edits keep their diff and finished commands keep
  the last lines of their output, below the summary line, by default.
- **A details view:** one key opens the full diff or output of a call in a
  scrollable view over the conversation, where it can be read and copied.
- **Direct keys:** a click on a call, or `Ctrl-T` from the input, without
  moving the focus first.

## Motivation

- Copying an answer, a command or a code block out of the TUI is one of the
  most common things to do with it. Today it needs knowledge of `Shift`-drag
  or `tui.mouse = false`, and then gets the terminal's wrap and markers.
- After a turn the user wants to check what changed and what ran: "which lines
  did it edit?", "did the tests really pass?". The answer is in the TUI, but
  hidden. The user goes to `git diff` or reruns the command instead.
- `tui.expand_tool_calls = true` shows everything, also every `read_file` and
  `search_files`. That is too much, so it is rarely the answer.

## Scope

In:

- Mouse selection in the conversation, and the clipboard.
- `y` in the conversation focus, `/copy [N|code]`.
- Previews of finished calls, by tool, with `tui.tool_preview`.
- The details view of a call.
- Clicking a call, `Ctrl-T`, and the hints in the footer and help.

Out:

- Selection in the input editor and the boxes (status, settings).
- `clai chat`: it prints plain text, and the terminal selects it already.
- Showing the file after an edit (more than the diff). The details view can
  get it later.
- Search in the conversation. The details view has its own search; a search
  over the whole conversation can follow.

## Design

### Clipboard

- Text goes to the clipboard with **OSC 52**, the terminal escape sequence for
  setting the clipboard. It works over SSH and in most terminals (kitty,
  WezTerm, foot, Alacritty, iTerm2, Windows Terminal, recent GNOME
  terminals). In tmux it needs `set-clipboard on`.
- When `wl-copy`, `xclip`, `xsel` or `pbcopy` is found and the session is
  local (no `SSH_TTY`), the text is also given to it **(proposal)**. Some
  terminals ignore OSC 52, and this covers them.
- `tui.clipboard = "auto"` (default), `"osc52"`, `"command"` or `"off"`. With
  `off` the copy keys only show a notice.
- After a copy the footer says `copied 12 lines` for two seconds. The TUI
  cannot know if the terminal took the text, so the notice says `copied`, not
  `in clipboard`.
- No new crate **(proposal)**: OSC 52 is one escape sequence, and the
  commands run with `tokio::process`. `arboard` would need X11 / Wayland
  libraries and fails over SSH.

### Mouse selection

```
> fix the failing test

The test fails because `parse` drops the last line. Let me look.
▏for i in 0..lines.len() {▕          ← selected text in reverse colours
▸ read_file src/parse.rs                                          ✓ 120 lines
```

- With `tui.mouse = true` (default), a left-button drag in the conversation
  selects text, shown in reverse colours. Release copies it. A click without
  a drag selects nothing, and works on entries as before.
- Double click selects a word, triple click a line **(proposal)**.
- Dragging above or below the conversation scrolls it, so a selection can be
  longer than the screen.
- **What is copied:** the text as the user reads it, not as it is drawn:
  - lines that the TUI wrapped are joined again;
  - the markers of the TUI (`▸ ▾`, the gutter, the "… N more lines" line,
    state glyphs) are left out;
  - a selection that covers a whole code block copies its source, without
    the fence;
  - trailing spaces are removed.
- To do this, each drawn row keeps where it came from: entry, source line,
  and whether it continues the row above. This sits next to the render cache
  of each entry.
- `Esc`, a key, or new output that moves the view clears the selection.
  While the view follows the end, a selection stops the following until it is
  cleared **(proposal)**, so the text does not move under the mouse.
- The footer hint `? help` gets `drag copy` while the mouse is on.
- With `tui.mouse = false` nothing changes: the terminal selects.

### Copy keys

In the conversation focus:

| Key | Copies |
|-----|--------|
| `y` | The selected entry: a user message or model answer as its Markdown source; a tool call as below. |
| `Y` | The code block under the selection, or the last code block of the selected answer. |

For a tool call, `y` copies:

| Tool | Text |
|------|------|
| `edit_file`, `write_file` | The diff, as a unified diff with the file name, so it can be applied with `git apply`. A new file is its content. |
| `run_command` | `$ command`, then the output. |
| `read_file` | The lines read. |
| others | The result. |

In the input, as commands:

| Command | Copies |
|---------|--------|
| `/copy` | The last model answer, as Markdown. |
| `/copy N` | The answer N back (`/copy 2` is the one before the last). |
| `/copy code` | The last code block of the last answer. |

### Previews of finished calls

```
▸ read_file src/parse.rs                                          ✓ 120 lines
▾ edit_file src/parse.rs                                          ✓ +1 −1
  @@ -12,3 +12,3 @@
  - for i in 0..lines.len() - 1 {
  + for i in 0..lines.len() {
▾ run_command cargo test parse                                    ✓ exit 0
  $ cargo test parse
  … 38 lines
  test parse::last_line ... ok
  test result: ok. 4 passed; 0 failed; 0 ignored
  o open  y copy
```

- A finished call shows a **preview** below its summary when its tool shows
  one:
  - **`edit_file`, `write_file`:** the diff, up to `tool_output_lines` lines
    (default 20). A new file shows its first lines.
  - **`run_command`:** the command, and the **last** 6 lines of output
    **(proposal)**, because the end (test summary, error) matters most. Above
    them a `… N lines` line says how much is hidden.
  - **Failed calls of any tool:** the error.
  - **`read_file`, `list_files`, `search_files`:** no preview; they stay one
    line.
- **While a command runs**, the same last lines show its live output, so
  `cargo build` can be watched.
- The last row of an expanded or previewed call is a dim hint `o open  y copy`
  when the call is selected **(proposal)**.
- `Enter` on a call still moves between collapsed, expanded and full, as
  today. A collapsed call stays collapsed: the user's choice wins.
- **Setting:** `tui.tool_preview = "changes"` (default: edits, writes,
  commands, errors), `"all"` (every tool, as `expand_tool_calls = true`), or
  `"none"` (today). `expand_tool_calls` is read as `"all"` when set and is
  deprecated **(proposal)**.

### Details view

`o` on a selected call, or `Enter` on its `… N lines` line, opens the call
over the conversation, at full size:

```
┌ edit_file src/parse.rs ─ ✓ +1 −1 ─────────────────────────────────────────┐
│ @@ -12,3 +12,3 @@                                                         │
│      let lines: Vec<&str> = text.lines().collect();                       │
│ -    for i in 0..lines.len() - 1 {                                        │
│ +    for i in 0..lines.len() {                                            │
│          out.push(parse_line(lines[i])?);                                 │
│                                                                           │
│ Up Down PgUp PgDn scroll ─ / search ─ n next ─ y copy ─ w wrap ─ Esc close │
└───────────────────────────────────────────────────────────────────────────┘
```

- It shows all of the call: for an edit the diff with its context, for a
  command the command, the working directory, the exit code and the whole
  output the TUI has. The output is as the tool returned it to the model;
  the limit of [`tools.md`](../spec/tools.md), "Result size", applies, and the
  view says so when the output was cut there.
- Keys: scroll, `/` search with `n` / `N`, `y` copy all, `w` toggle wrap
  (long lines scroll sideways with `Left` / `Right` when off), `Esc` / `q`
  close. The mouse selects and copies in it as in the conversation.
- A question that opens closes it, as for the other boxes.
- `/diff` opens a view with the diffs of all edits of the last turn, one after
  the other **(proposal)**: "what did it change?" in one place.

### Direct keys

- **Click** on a call's summary line toggles it, as `Enter` does with the
  focus on it. The focus does not move to the conversation, so typing goes on.
- **`Ctrl-T`** from the input: show or hide the previews of all calls of the
  last turn. **(proposal)**
- **`Ctrl-Y`** from the input: copy the last answer, as `/copy`.
- The help (`?`, `/help`) lists the copy keys, the details view, and
  "`Shift`-drag uses the terminal's own selection".

## Spec changes

- `spec/tui.md`:
  - "Conversation entries": previews of finished calls; live output of a
    running command.
  - A new section "Copy": selection, the clipboard, `y`, `Y`, `/copy`,
    `Ctrl-Y`.
  - A new section "Details view".
  - "Keys": `o`, `y`, `Y`, `Ctrl-T`, `Ctrl-Y`, click on a call; `/copy` and
    `/diff` in the command table.
- `spec/config.md`, "TUI": `clipboard`, `tool_preview`; `expand_tool_calls`
  deprecated; `mouse` says that a drag selects.

## Testing

- **Copy text:** unit tests of what a selection copies: wrapped lines joined,
  markers left out, whole code blocks as source, a selection across entries.
- **Clipboard:** the OSC 52 sequence (base64, the terminal's length limit);
  the command chosen by `auto` with and without `SSH_TTY`, with a fake
  `wl-copy` on `PATH`.
- **`y` per tool:** the unified diff applies with `git apply` in a temp repo;
  the command text; the answer as Markdown.
- **Previews:** snapshot tests of each tool, a failed call, a running command
  with live output, a long output cut from the top, and `tool_preview` set to
  each value.
- **Details view:** snapshot tests; search; wrap on and off; output cut by the
  result limit.
- **App:** drag and release copies and clears; click toggles a call without
  moving the focus; `Ctrl-T`; `/copy N`; a question closes the details view.
- **Check:** a pseudo-terminal run of the real TUI. A turn edits a file and
  runs a command; the preview shows both, `o` opens the diff, and `y` writes
  the OSC 52 sequence.

## Implementation plan

1. **Clipboard**
   - Status: todo
   - Files: `tui/clipboard.rs` (new), `config/`, `spec/config.md`
   - Result: OSC 52 and the clipboard commands, with `tui.clipboard`
   - Tests: sequence and command choice
2. **Copy keys**
   - Status: todo
   - Files: `tui/app.rs`, `tui/conversation.rs`, `tui/commands.rs`,
     `spec/tui.md`
   - Result: `y`, `Y`, `/copy [N|code]`, `Ctrl-Y`
   - Tests: text per entry and tool; `git apply` of a copied diff
3. **Tool previews**
   - Status: todo
   - Files: `tui/conversation.rs`, `config/`, `spec/tui.md`,
     `spec/config.md`
   - Result: diffs and command tails after the call; live command output;
     `tui.tool_preview`
   - Tests: preview snapshots
4. **Click and `Ctrl-T`**
   - Status: todo
   - Files: `tui/app.rs`, `tui/view/conversation.rs`
   - Result: a call opens without moving the focus
   - Tests: App tests
5. **Details view**
   - Status: todo
   - Files: `tui/details.rs` (new), `tui/view/details.rs` (new), `tui/app.rs`
   - Result: `o`, the full call, search, wrap, copy; `/diff`
   - Tests: snapshots and App tests
6. **Mouse selection**
   - Status: todo
   - Files: `tui/selection.rs` (new), `tui/conversation.rs`,
     `tui/view/conversation.rs`, `tui/app.rs`
   - Result: drag selects and copies in the conversation and the details view
   - Tests: copy text tests; App tests for drag, scroll and clear
7. **Docs**
   - Status: todo
   - Files: `readme.md`, `todo.md`, this RFC
   - Result: status `done`

Mouse selection is last: it is the largest step, and the copy keys and
previews already answer most of the need.

## Alternatives

- **Turn mouse capture off by default:** the terminal selects at once, but the
  wheel and clicks stop working, and the copy still has the TUI's wrap and
  markers.
- **Copy the screen as drawn:** simple, but copies `▸`, cut lines and the
  gutter, which is what `Shift`-drag already does.
- **`arboard` for the clipboard:** no escape sequences, but native libraries,
  and no clipboard over SSH, where OSC 52 works.
- **Expand all finished calls:** `expand_tool_calls = true` exists and shows
  too much. Previews by tool show what was asked for.
- **The head of command output:** the head of `cargo test` is the build; the
  result is at the end.
- **Open the diff in `$PAGER` or `$EDITOR`:** full power, but it leaves the
  TUI and needs the alternate screen to be given up for a moment. The details
  view stays in the TUI. An `e` key "open in `$PAGER`" can be added later.

## Open questions

1. **Command preview length:** the last 6 lines (proposal), or
   `tool_output_lines` as for diffs?
2. **Default `tool_preview`:** `changes` (proposal), or keep today's collapsed
   calls and only add the keys?
3. **Clipboard commands:** also use `wl-copy` / `xclip` locally (proposal), or
   only OSC 52?
4. **`/diff` for the last turn:** in this RFC (proposal) or later?
5. **Selection while output streams:** stop following the end (proposal), or
   keep following and clear the selection?
