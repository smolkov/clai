# Session switching

- RFC: 0005
- Status: done
- Milestone: the session list of "TUI sessions and settings" (see
  [`implementation.md`](../implementation.md))
- Spec: [`spec/tui.md`](../spec/tui.md), "Sessions", "Switching sessions",
  "Model picker", "Input editor", "Keys";
  [`spec/command.md`](../spec/command.md), "clai attach", "clai chat";
  [`spec/sessions.md`](../spec/sessions.md), "Session lifecycle"

## Summary

From the TUI, from `clai chat` and from the shell the user can start a new
session with a model of their choice, pick a session from the list of all
sessions, and attach the TUI or `clai chat` to it without leaving. In the TUI
this is the session list (`Ctrl-S`, `/sessions`), `/new` with the model picker,
`/attach NAME` and `Ctrl-N` / `Ctrl-P`; in `clai chat` it is `/sessions`,
`/new` and `/attach`; in the shell `clai attach` without a name opens the
session list when there are several sessions.

## Motivation

- The spec describes the session list, `/new`, `/attach` and switching, but
  none of it is built: the commands say "comes with a later version", and
  `Ctrl-S`, `Ctrl-N`, `Ctrl-P` do nothing.
- To work in a second session, or the same workspace with another model, the
  user has to detach, run `clai new --model …` or `clai attach NAME`, and
  start the TUI again. The `●1` on the session row says another session waits
  for an answer, but the TUI offers no way to go there.
- `clai attach` with several sessions fails with "several sessions: a, b, c;
  name one", a placeholder until the list exists.
- A killed session leaves the TUI with a disabled input where any key exits;
  the spec says it opens the session list.

## Scope

In:

- TUI: the session list with `Enter`, `n`, `r`, `k`, `Esc`; `/sessions`,
  `Ctrl-S`; `/new [NAME]` with the model picker; `/attach NAME`; `Ctrl-N` /
  `Ctrl-P`; view state kept per session; the list after the attached session
  ended.
- `clai attach` without a name and several sessions: the TUI opens on the
  session list.
- `clai chat`: `/sessions`, `/new [NAME] [MODEL]`, `/attach NAME`.
- Argument completion in the command list for `/attach` (session names) and
  `/model` (models), since both lists are at hand now. **(proposal)**

Out:

- A server notice for session state changes (protocol 4, `todo.md`): the list
  keeps polling `list` every 2 s like the session row.
- `/clear`, `/permissions`, `/commit`.
- Paged loading of long logs; switching replays the whole log as attach does
  today.
- Several sessions on one screen (split view).

## Design

### Switching in the TUI

A switch is a new attach, built on the reconnect path that exists already:

1. `App` returns a new effect `Effect::Switch(name)`.
2. The shell spawns a task that opens a **new** `Client`, sends `attach` and
   `list` (the existing `attach` function), and reports `Report::Attached`.
3. The shell replaces the `Connection`. Dropping the old one detaches it on
   the server, so no `detach` request is needed.
4. `App` stores the view state of the session it leaves and builds the new
   one from the backlog, as on a reconnect.

**(proposal)** A new connection rather than `detach` + `attach` on the same
one: the replay then arrives before the first frame, through the same code as
start-up and reconnect, and events of the old session cannot mix in. The cost
is one socket per switch, which is nothing. No protocol change is needed.

While the switch is on its way the old session stays on screen and the status
row says `attaching NAME…`. If it fails (session gone, server error), the old
connection is still there; the error goes to the conversation.

A question open in the session the user leaves stays open; the TUI was one of
its answering clients, and `●1` counts it from then on.

### View state per session

`App` gets `views: HashMap<u64, View>` keyed by session id:

```rust
struct View {
    top: usize,
    follow: bool,
    focus: Focus,
    selected: Option<usize>,
    /// Ids of expanded and collapsed tool calls the user toggled.
    toggled: HashMap<String, bool>,
    input: Editor,
}
```

On leaving, the current one is stored; on attaching to a session with a
stored view, it is restored after the replay. The conversation itself is not
kept: it is rebuilt from the log, which is also what a reconnect does. Views
live as long as the TUI; a killed session's view is dropped.

### Session list

`src/tui/sessions.rs` holds its state, `src/tui/view/sessions.rs` draws it
in the box code of `popup.rs`, like the picker.

```rust
pub struct SessionList {
    sessions: Vec<SessionInfo>,
    selected: usize,
    mode: Mode,
}

enum Mode {
    Browse,
    Rename(Editor),
    ConfirmKill,
    /// `n`: the workspace, then the name, then the model picker.
    NewWorkspace(Editor),
    NewName { workspace: PathBuf, name: Editor },
}
```

- Opens with `Ctrl-S`, `/sessions`, and at start for `clai attach` with
  several sessions. It sends `list` and opens on the answer, like the picker
  on `models`; the 2 s `list` keeps it fresh, the selection stays on the same
  session id.
- Columns as in `spec/tui.md`: name, workspace, model, state, age. A waiting
  session shows `● waiting`; a saved one `saved`. The attached session is
  marked. On a narrow terminal the age, then the workspace, are cut first.
- Order: the server's order, which is also the order of `1/3`, so `Ctrl-N` /
  `Ctrl-P` and the list agree.
- `Enter` on another session switches; on the attached one it closes.
- `r` renames inline: the name becomes an editor, `Enter` sends `rename`,
  `Esc` goes back. A refused name shows the server's message in the box.
- `k` asks `kill NAME? y/n` in the box's last line; `y` sends `kill`. Killing
  the attached session from the list works like `/kill`.
- `n` asks for the workspace (default: the attached session's, `~` expanded,
  relative to it), then the name (empty: the server's default), then opens the
  model picker on `default_model`. `Enter` in the picker sends `new` and
  switches to the session it returns. `Esc` at any step goes back one step.
- `Esc` closes.
- A question that opens in the attached session closes the list, as it does
  the picker.

### New session

- `/new [NAME]` opens the model picker titled `Model for new session`,
  preselected on `default_model`, with the attached session's workspace. `Enter`
  sends `new { workspace, name, model }` and switches to it. `Esc` creates
  nothing.
- The picker gets a `purpose: Purpose` field: `Change` (today's behaviour,
  marks `✓ current`) or `New { workspace, name }` (no `current` mark, no history
  mark, `Enter` on any model creates).
- `n` in the list ends in the same picker.
- The note "other sessions work in this workspace" of `clai new` becomes a
  notice in the new session's conversation. **(proposal)**

### Attach

- `/attach NAME` switches. An unknown name gets the server's error as a notice
  and the input stays.
- `/attach` without a name opens the session list. **(proposal)**
- `Ctrl-N` / `Ctrl-P` switch to the next / previous session in the last `list`,
  wrapping around. With one session they do nothing.

### After the attached session ends

The spec's rule replaces the placeholder: when the attached session ends, the
TUI shows the notice and opens the session list. If other sessions exist, keys
work in the list and `Esc` exits with `0`. With none left, the box says
`no sessions` with `n new ─ Esc exit`. `clai server stop` still exits as
today: the TUI does not start the server again.

### `clai attach`

Without `NAME`: one session attaches as today; none fails with `no sessions`;
several open the TUI with the session list over the first one in the list.
`Esc` without choosing exits `0` with the detach line of that session.
**(proposal)** Attaching to the first one first keeps `App` simple: it always
has a session.

### `clai chat`

`clai chat` stays one session at a time; switching means detach, then attach
on the same connection (`detach` + `attach`; the replay is printed as on
start).

- `/sessions` prints the `clai ls` table, the attached session marked with `*`.
- `/new [NAME] [MODEL]` creates a session in the attached session's
  workspace and attaches to it. `MODEL` is `PROVIDER/MODEL` or the short form;
  without it, `default_model`. A word with a `/` is taken as the model, so
  `/new openai/gpt` works without a name. **(proposal)**
- `/attach NAME` attaches to another session.
- `/models` lists the configured models, from the `models` request, so the
  user can see what `/new` and `/model` accept. **(proposal)**
- A question open in the left session stays open, as in the TUI.
- The line on leaving names the session attached last.

### Argument completion

With the command list closed after a space, `/attach ` and `/model ` show a
list of values above the input, in the same box as the command list: session
names (with workspace and state) and models (with id). Typing searches them as
it does commands; `Tab` completes, `Enter` runs. The values come from the last
`list` and `models` answers. **(proposal)**

## Spec changes

Written when the RFC is accepted.

- `spec/tui.md`: the list's columns, order and modes; `Ctrl-N` / `Ctrl-P` wrap;
  `/attach` without a name; the picker for a new session; the status row while
  attaching; the list after a session ended, replacing the placeholder;
  argument completion, replacing "not built yet".
- `spec/command.md`: `clai attach` with several sessions; `/sessions`, `/new`,
  `/attach`, `/models` in `clai chat`.
- `readme.md`: the new keys and commands; `/help` in the TUI and `clai chat`.

## Testing

- `SessionList`: keys in each mode, selection kept across a refresh by id,
  `Esc` steps back, `n` through to a `new` request.
- `App`: `Effect::Switch` from `Enter`, `/attach`, `Ctrl-N` / `Ctrl-P` (wrap,
  one session); view state stored and restored after a switch and back;
  a failed switch keeps the old session; a question in the attached session
  closes the list; the list after `ended`, with and without other sessions.
- Picker with `Purpose::New`: no `current` mark, `Enter` sends `new`.
- `insta` snapshots: the list in Unicode and ASCII, narrow, with a waiting and
  a saved session, in rename and kill mode; the picker for a new session;
  argument completion.
- Server, in-process: a client that switches by a new connection is detached
  from the old session, and a question there goes on to other clients.
- `clai chat`, in-process: `/new` with and without a model, `/attach`,
  `/sessions`; the replay of the second session is printed and events of the
  first stop.
- Binary: `clai attach` with several sessions needs a terminal; the old error
  test changes.

## Implementation plan

1. **Switching in the TUI**
   - Status: done
   - Files: `src/tui/mod.rs`, `src/tui/app.rs`
   - Result: `Effect::Switch`, view state per session; `/attach NAME`,
     `Ctrl-N`, `Ctrl-P` work.
   - Tests: switch, restore, failure, wrap.
2. **Session list**
   - Status: done
   - Files: `src/tui/sessions.rs`, `src/tui/view/sessions.rs`,
     `src/tui/view/popup.rs`, `src/tui/app.rs`, `src/tui/keys.rs`,
     `src/tui/commands.rs`
   - Result: `Ctrl-S`, `/sessions`, `/attach`; `Enter`, `r`, `k`, `Esc`; the
     list after the session ended.
   - Tests: list keys and snapshots.
3. **New session**
   - Status: done
   - Files: `src/tui/picker.rs`, `src/tui/sessions.rs`, `src/tui/app.rs`,
     `src/tui/view/picker.rs`
   - Result: `/new [NAME]` and `n` in the list create with a chosen model and
     switch.
   - Tests: picker purpose, `n` steps, snapshots.
4. **`clai attach` without a name**
   - Status: done
   - Files: `src/tui/mod.rs`, `src/command/attach.rs`, `tests/`
   - Result: several sessions open the list.
   - Tests: unit test of the target choice; binary test.
5. **`clai chat`**
   - Status: done
   - Files: `src/command/chat/talk.rs`
   - Result: `/sessions`, `/new`, `/attach`, `/models`.
   - Tests: in-process chat tests.
6. **Argument completion**
   - Status: done
   - Files: `src/tui/commands.rs`, `src/tui/app.rs`, `src/tui/view/commands.rs`
   - Result: `/attach ` and `/model ` offer values.
   - Tests: search and keys; snapshots.
7. **Docs**
   - Status: done
   - Files: `spec/`, `readme.md`, `implementation.md`, `todo.md`,
     `rfcs/readme.md`
   - Result: spec changes above; status `done`.
   - Tests: `cargo fmt`, `clippy`, `test`.

## Alternatives

- **`detach` + `attach` on the TUI's connection.** One socket, but the replay
  then streams into the running event loop, mixed with live events of nothing
  yet: the first frame of the new session would show a half-built
  conversation, and the code path differs from start-up and reconnect. `clai
  chat` uses it because it prints lines and has no frame.
- **Attach to all sessions at once and filter.** Instant switching and live
  `●` counts, but every event of every session goes to every TUI, and the
  server would send all open questions with choices to a TUI that shows only
  one. Better left to the planned state notice.
- **Keep each session's conversation in memory.** Faster switching back, but
  more memory and a second source of truth next to the log; the replay is
  fast enough until measured otherwise (`todo.md`).
- **`/new` without the picker, always `default_model`.** Less to build; the
  user then needs `/model` after it. The request is to choose the model when
  starting, so the picker stays.

## Open questions

Answered when the RFC was accepted, each as this RFC proposed:

1. **`clai chat /new` syntax.** `/new [NAME] [MODEL]`; a word with a `/` is the
   model.
2. **Argument completion.** In this RFC.
3. **`clai attach` with several sessions.** The TUI opens on the list over the
   first session.
4. **Workspace step of `n`.** Asked, with the attached session's as default.

## Notes from building

Details the design left open, or changed while building:

- The attached session is marked `✓` in the list: `*` is the ASCII form of `●`
  (waiting), so it could not tell the two apart.
- `Esc` in the model picker of a new session creates nothing and does not go
  back to the name step; `Esc` in the list's own steps goes back one step.
- `/attach NAME` checks the name against the last `list` first: an unknown name
  stays in the input with a notice, as the spec has it for unknown commands. A
  session made since the last `list` (at most 2 s) is found after it.
- The argument list searches session names only, not their workspace: many
  sessions share one, so `/attach project` would select another session. Models
  are searched by name and id.
- The `models` request for `/model ` is sent when that list first opens and no
  models are known yet; it opens no picker.
- `clai chat` attaches to the new session before it detaches from the old one,
  so a failed attach leaves it where it was.
- With `clai server stop` the TUI opens no list, since it must not start the
  server again; it ends as before.
- The user asked during the build that a new session starts on the default
  model: the picker for `/new` and `n` opens on `default_model`, so one `Enter`
  does it. A test covers it.
