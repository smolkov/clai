# TUI command list

- RFC: 0004
- Status: done
- Milestone: the command list of "TUI sessions and settings" (see
  [`implementation.md`](../implementation.md))
- Spec: [`spec/tui.md`](../spec/tui.md), "Input editor", "Command list", "Keys"

## Summary

Typing `/` in the empty input opens a list of the commands above the input.
Further typing searches the list, and `Enter` runs the selected command. Every
box of the TUI that the user can open has a command, so the list is one way to
reach all of them.

## Motivation

The commands exist, but the user has to know them: the only way to find one is
`/help`, and a typing error ends in "unknown command". `spec/tui.md` describes a
command list; it was left out of RFC 0001 and RFC 0003.

## Scope

In:

- The list: opening, search, selection, running, completing, drawing.
- One table of commands, used by the list and by `/help`.
- A rule and a test: every box the user can open has a command.

Out, for "TUI sessions and settings" later:

- Argument completion (`/model` offering models, `/attach` offering sessions).
  The list closes at the first space; the rest is typed as today.
- Building the session list, `/permissions`, `/clear`, `/new`, `/attach` and
  `/commit`. See "Layouts and their commands".

## Design

### Opening and closing

The spec's rules stay. The decisions below it leaves open are marked.

- The list opens when `/` is typed into the empty input with focus on the
  input. **(proposal)** Pasted text and a message from the history do not open
  it: they are not a command being looked for.
- It closes when:
  - the text stops being a search: it no longer starts with `/`, it holds a
    space or a new line, or it starts with `//`, so a message that starts with
    `/` can still be typed **(proposal)**;
  - the user presses `Esc`, or completes or runs a command;
  - a question opens, or the model picker opens with `Ctrl-O`. The text stays.
    Keys belong to the prompt bar or the picker then, as in RFC 0003.
    **(proposal)**
- A closed list opens again only from an empty input. **(proposal)** After
  `Esc`, `Backspace` does not bring it back. `Enter` then works as it does
  today, with the "unknown command" and "usage" notices.

### Search

- The text after the `/` is the search. It looks at the command's names, in any
  case: those that start with it first, then those that contain it, each in
  table order. The first is selected.
- `/quit` is another name of `/detach`: they share a row, and either name finds
  it.
- Descriptions are not searched, as the spec says. Adding them later would
  change nothing else.

### Keys

As in the spec's table. Keys it does not name **(proposal)**:

| Key | Action while the list is open |
|-----|-------------------------------|
| `Esc` | Closes the list and does nothing else. The typed text stays, and a running turn goes on. A second `Esc` cancels the turn, as without the list. |
| `Up` / `Down` | Stop at the first and last row; they do not wrap, as in the picker. |
| `Tab` | With no match, does nothing. Focus does not change. A command with arguments gets a space after its name. |
| `Enter` | With no match, does nothing: no notice, the list already says `no command`. |
| `PgUp`, `PgDn`, `Ctrl-Home`, `Ctrl-End`, mouse wheel | Scroll the conversation. |
| `Ctrl-C`, `Ctrl-D`, `Ctrl-O` | As without the list. If `Ctrl-C` clears the input, the list closes. |

`Enter` on a command that needs an argument (`/rename`) completes it, as the
spec says. `/model` does not need one: it opens the picker.

### Drawing

```
> earlier output of the conversation
──────────────────────────────────────────────────────────────────────────────
▶ /model [PROVIDER/MODEL]   Change the session's provider and model
  /rename NAME              Rename the attached session
──────────────────────────────────────────────────────────────────────────────
> /m_
──────────────────────────────────────────────────────────────────────────────
```

- It lies over the last rows of the conversation, under a plain line, and uses
  plain lines like the rest of the screen rather than the box in the spec's
  sketch. **(proposal)** The spec's sketch dates from before the screen lost its
  box. The sketch in the spec changes.
- It does not change the layout, so the conversation does not jump when it
  opens and closes.
- At most 8 rows; the selected row stays visible. Long rows are cut, as in the
  picker. With no match it has one row: `no command`.

### Layouts and their commands

| Layout | Command | State |
|--------|---------|-------|
| Help | `/help` | exists |
| Model picker | `/model` | exists |
| Confirmation of kill | `/kill` | exists |
| Session list | `/sessions` | not built; says "comes with a later version" |
| Permission rules | `/permissions` | not built; same |
| Commit dialog | `/commit` | not built; same |
| Prompt bar | none | opens only when the agent asks a question |

So every layout that exists can already be opened with a command, and the list
makes that visible. New rule in the spec: a box the user can open has a command
in the table. A test runs the command of each `Dialog` and of the picker from
the list, and the match on `Dialog` in that test has no `_` arm, so a new
dialog does not compile until it has a command.

Unbuilt commands stay in the list, as the spec's "every command" says, dim and
with `later version` in place of the description. `Enter` gives today's notice.
See "Open questions".

### Code

- `src/tui/commands.rs` (new): the table `COMMANDS` (names, arguments as shown,
  description, whether an argument is needed, whether it is built), and
  `search(text) -> Vec<&Command>`. Pure.
- `src/tui/app.rs`: `command_list: Option<CommandList>` holds only the selected
  row. The search is always the editor's text, so there is nothing to keep in
  step. One function after each edit closes the list when the text stopped being
  a search. `update` closes it with the picker's check for an open question or
  an ended session.
- `src/tui/view/commands.rs` (new): draws the rows.
- `src/tui/view/popup.rs`: the commands part of the help text comes from the
  table, so the two cannot differ.
- No change to the protocol, the server or the saved files. No new crate.

### Changes to existing behaviour

`Enter` on `/nope` and on `/rename` now goes through the list, so two tests
change:

- `/nope`: no "unknown command" notice while the list is open; the list says
  `no command` and the input stays.
- `/rename`: completes to `/rename ` instead of the "usage" notice.

Both notices stay for text that reached the input with the list closed, and
those tests move to that case.

## Spec changes

- `spec/tui.md`, "Command list": the new sketch; opening only from typing;
  closing rules; `//` closes it; the keys above; argument completion marked as
  not built.
- `spec/tui.md`, "Input editor" or "Layout": the rule that a box the user can
  open has a command.
- `readme.md`: the list in the keys and commands.
- `todo.md`, `implementation.md`: the command list done, argument completion
  open.

## Testing

Unit, without a terminal:

- Search: starts-with before contains, table order, any case, either name of a
  row, no match.
- Every name in the table is known to `App::command`: it never gives "unknown
  command". This keeps the table and the match together.

`App`, with keys in and effects out:

- Opens on `/` in the empty input. Does not open on paste, a history message,
  `/` after other text, or with focus on the conversation.
- Closes on `//`, a space, a new line, `Backspace` on the `/`, `Ctrl-C`,
  `Ctrl-O`, a question, the end of the session.
- `Esc` closes the list and keeps the text, with no request sent. During a
  running turn, the first `Esc` only closes the list and the second sends
  `cancel`. After `Esc`, typing and `Backspace` do not open the list again.
- `Up` / `Down` select and do not browse the history. A new search selects the
  first row.
- `Tab` completes and focus stays. `Enter` runs each kind: `/help`, `/model`,
  `/kill`, `/detach` by both names; `/rename` completes; no match does nothing.
- A message typed as `//text` is still sent as `/text`.
- The test of commands for every layout, above.

Frames (insta): all commands, a search, no match, a scrolled list on a small
screen, ASCII symbols. Existing frames do not change.

A run of the real binary in tmux against an isolated server: `/`, search, `Tab`,
`Enter`, `Esc`, `/model` into the picker.

## Implementation plan

1. **Command table**
   - Status: done
   - Files: `src/tui/commands.rs`, `src/tui/view/popup.rs`, `src/tui/mod.rs`
   - Result: `/help` text comes from the table; no visible change except the
     order, which becomes the spec's.
   - Tests: search; every name is known to `App::command`.
2. **List state and keys**
   - Status: done
   - Files: `src/tui/app.rs`
   - Result: the list works without being drawn.
   - Tests: the `App` tests above; the two changed tests.
3. **Drawing**
   - Status: done
   - Files: `src/tui/view/commands.rs`, `src/tui/view/mod.rs`
   - Result: the list is on the screen.
   - Tests: frames.
4. **Docs and the real binary**
   - Status: done
   - Files: `spec/tui.md`, `readme.md`, `todo.md`, `implementation.md`, this RFC
   - Result: status `done`.
   - Tests: tmux run; `cargo fmt`, `clippy -D warnings`, `cargo test`.

## Alternatives

- **A box in the middle of the screen, like the picker.** It hides more, and the
  text being typed is in the input, so the list belongs next to it.
- **The list has its own filter, like the picker.** Then `/` would have to leave
  the input and come back. The spec wants the search in the input.
- **Shrink the conversation while the list is open.** The text would jump on
  each opening and with each search that changes the number of rows.

## Open questions

Answered when the RFC was accepted, with the defaults:

1. **What does "all layouts can be opened with a command" cover?** The layouts
   that exist. This RFC builds no new layout; the session list, permission rules
   and commit dialog get a box when they are built.
2. **Do unbuilt commands appear in the list?** Yes, dim and marked
   `later version`.

## Notes from building

- `Enter` on an unbuilt command gives its notice even if it needs an argument
  (`/attach`), since completing it would lead nowhere.
- A picker can open late, when the server answers. If `/` was typed in between,
  the picker closes the list, like a question does.
- `Tab` completes a row's main name: `/q` becomes `/detach`.
