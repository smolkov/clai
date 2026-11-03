# TUI core

- RFC: 0001
- Status: draft
- Milestone: TUI core (see [`implementation.md`](../implementation.md))
- Spec: [`spec/tui.md`](../spec/tui.md), [`spec/sessions.md`](../spec/sessions.md),
  [`spec/command.md`](../spec/command.md) (`clai`, `clai new`, `clai attach`),
  [`spec/config.md`](../spec/config.md) ("TUI")

## Summary

Build the full-screen interface as a client of the server: one attached
session, its conversation streamed live, the input editor, the permission
and limit prompts, cancellation and detach. With it, `clai`, `clai new`
(without `-d`) and `clai attach NAME` work. Session list, model picker,
command list, themes and syntax highlighting stay for the next milestone
(TUI sessions and settings).

## Motivation

Everything a session needs runs in the server and is tested; the only way
to use a session interactively today is `clai chat`, which is line based:
no scrolling back, no collapsed tool calls, no diff view while a question
waits. The three commands that are the main entry points of `clai` still
return "not implemented". This milestone makes them work and puts the
architecture rule "the TUI is only a client" into code.

## Scope

In:

- Terminal setup and restore: raw mode, alternate screen, bracketed paste,
  mouse capture (`tui.mouse`), keyboard enhancement for `Shift-Enter`;
  restore on exit, error, panic, `SIGTERM`, `SIGHUP`.
- Event loop with the redraw limit `tui.max_fps`.
- Layout: header, conversation, prompt bar, input, status line; "terminal
  too small" below 40 × 10.
- Conversation entries: user messages, Markdown model text (streaming),
  tool calls collapsed/expanded with state marks, diffs, command output,
  notices, errors; follow mode and "↓ new output".
- Input editor: multi-line, line-editing keys, message history, bracketed
  paste, typing while a turn runs.
- Permission prompt (`y`, `a` with `Tab` widening, `n`, `e`, `Esc`) and
  limit prompt (`y`, `n`, `Esc`); a question answered elsewhere closes the
  prompt and shows who answered.
- Keys of `spec/tui.md`, "Keys", except `Ctrl-S`, `Ctrl-O`, `Ctrl-N`,
  `Ctrl-P` (next milestone).
- `/help`, `/detach`, `/quit`, `/model PROVIDER/MODEL` (with argument only),
  `/rename NAME`, `/kill` (with confirmation). Other commands show
  "`/sessions` comes with a later version".
- `[tui]` keys: `symbols`, `mouse`, `tool_output_lines`,
  `expand_tool_calls`, `timestamps`, `scroll_lines`, `max_fps`.
- `NO_COLOR`; one fixed style set on the terminal's 16 colours (what the
  `terminal` theme will be).
- Connection loss: "server disconnected", retry for 10 s, re-attach.
- `clai`, `clai new`, `clai attach NAME`; `clai attach` without a name when
  exactly one session exists.

Out (next milestones):

- Session list, switching with kept view state, `Ctrl-N`/`Ctrl-P`,
  `/sessions`, `/new`, `/attach`.
- Model picker (`Ctrl-O`, `/model` without argument).
- Command list popup and argument completion.
- Themes other than the fixed one, `CLAI_THEME`, `syntax_highlight`,
  live reload of `[tui]`.
- `/clear` (needs a server request), `/permissions` (needs a request that
  returns the rules in effect), `/commit`.
- Context gauge beyond "tokens of the last request / context window".

## Design

### Module layout

```
src/tui/
├── mod.rs          run(target): connect, attach, terminal guard, event loop
├── terminal.rs     setup, restore, panic hook, signals, Guard (Drop restores)
├── app.rs          App: all view state; update(Input) -> Vec<Effect>
├── conversation.rs entries built from SessionEvents; per-entry line cache
├── markdown.rs     pulldown-cmark → ratatui lines (no highlighting yet)
├── editor.rs       input editor state and keys
├── prompt.rs       permission and limit prompt state
├── keys.rs         key → action, by focus and open prompt
├── view/           drawing only: header, conversation, prompt, input, status
├── style.rs        styles and symbols (unicode / ascii, NO_COLOR)
└── tests.rs        state tests and TestBackend snapshots
```

### State and effects

The TUI is split into a pure part and a thin shell, so nearly all behaviour
is tested without a terminal or a server:

- `App` holds the view state: conversation entries, scroll offset and follow
  flag, focus, selected entry, expanded calls (by call id), editor, open
  prompt, session info, connection state.
- `App::update(input) -> Vec<Effect>` takes one input — a key, a paste, a
  mouse event, a resize, a `ServerMessage`, a tick — and returns effects:
  `Request(Request)`, `Redraw`, `Exit(ExitReason)`.
- `view::draw(&App, &mut Frame)` only reads `App`.
- The shell in `mod.rs` runs the loop: it feeds inputs to `update`, sends
  the requests, and draws at most `max_fps` times per second when something
  changed.

### Event loop

One task with `tokio::select!` over:

- terminal events (`crossterm::event::EventStream`),
- server messages (from the connection's reader task, below),
- responses to requests the TUI sent,
- a frame timer that only ticks while the screen is dirty (no busy redraw
  while idle), and a 1 s timer for the spinner and the double `Ctrl-C`,
- `SIGTERM` / `SIGHUP`: restore the terminal and exit; the session keeps
  running.

Streaming text events only mark the screen dirty; they are drawn with the
next frame, so a fast stream costs at most `max_fps` redraws.

### Client connection

`Client` reads responses and events from one stream in `request()`, which
blocks the caller while it waits. The TUI must never wait for the network,
so a new `client::Connection` **(proposal)** is added next to `Client`:

- `Connection::spawn(Client)` moves the socket halves into a reader task and
  a writer task.
- `request(Request) -> oneshot::Receiver<Result<Payload>>`: the reader
  routes responses by id; the TUI turns them into inputs when they arrive.
- `messages: mpsc::Receiver<ServerMessage>` for events and notices.
- A closed or broken socket is one input, `Disconnected`.

`Client` stays as it is for the terminal commands.

The hello names the client `tui <tty>` (`tui pts/3`) from `ttyname` of
standard input, or `tui` if that fails, and declares `answers: true`.

### Attach and replay

- `attach` is answered after the replayed log; events with `replay: true`
  build the conversation without redraws, then one frame is drawn at the end.
  A whole replay is applied before the first frame **(proposal)**; the
  "latest page in 200 ms, older history in the background" assumption is
  measured on a long session and only built if the full replay is too slow.
- An open question arrives as part of the session info and opens the prompt
  at once.
- On reconnect after a lost connection, the conversation is rebuilt from the
  replay; scroll position, expanded calls (by call id) and unsent input are
  kept.

### Conversation

- Entries: `User`, `Text` (Markdown, open or done), `ToolCall` (latest
  `ToolCallEvent` plus accumulated live output), `Notice`, `Error`,
  `ModelChanged` (as a notice).
- `TextDelta` appends to the open text entry; `TextDone` replaces it with
  the whole text (the source of truth).
- Each entry caches its wrapped lines per width; a change or a resize clears
  only that cache. The view keeps cumulative heights and draws only the
  visible slice, so 10 000 lines scroll without reflowing everything.
- An open text block is re-parsed as Markdown on each frame while it
  streams; finished blocks are parsed once.
- Tool calls show `▸`/`▾`, tool name, summary and state mark (`✓` with a
  short result, `✗`, `⊘`, `?`, spinner). Expanded: the diff (`+` green,
  `-` red) for edits, an excerpt for reads, command, working directory and
  output for commands, cut at `tool_output_lines` with "… N more lines";
  `Enter` on the marker shows the whole result.
- Follow mode: new output keeps the view at the end unless the user scrolled
  up; then the status line shows "↓ new output" and `Ctrl-End` resumes.

### Input editor

- A small own editor (`Vec<String>` lines, cursor in chars) **(proposal)**:
  the keys in `spec/tui.md` are few, and a crate such as `tui-textarea`
  pins ratatui versions and brings key bindings we would override anyway.
- `Enter` sends, `Shift-Enter` / `Alt-Enter` insert a newline. `Shift-Enter`
  needs the keyboard enhancement flags of crossterm; terminals without them
  send `Enter` and the user uses `Alt-Enter`.
- `Up`/`Down` on the first/last line browse the session's earlier user
  messages (taken from the conversation, so they survive reattach).
- A line starting with `//` sends the message without the first `/`.
- **Typing while a turn runs (proposal):** the message is sent at once and
  the server queues it (it already does, and keeps the queue in
  `session.json`). The TUI shows it as "queued" until its `user_message`
  event arrives. The effect is the one `spec/tui.md` describes ("sent when
  the turn ends"), and other clients see the queue too. The spec sentence is
  changed to say so.

### Prompts

- A `Question` event with `kind: permission` expands the call, scrolls it
  into view and opens the prompt bar with `reason_text` and the choices;
  keys go only to the prompt bar.
- `a` shows the first proposal; `Tab` steps to the next, wider one; `Enter`
  sends `allow_always` with that rule set; `Esc` goes back to the choices.
- `e` opens a one-line editor in the prompt bar; `Enter` sends
  `deny_message`, `Esc` goes back.
- A rejected answer (response error) is shown in the prompt bar; the
  question stays open.
- `question_closed` closes the prompt and adds "answered by tui pts/4:
  allow" when another client answered.
- Limit question: `y` → `continue`, `n` / `Esc` → `stop`.

### Keys and exit

- Order of handling: open prompt, then confirmation dialogs (`/kill`), then
  focus (input or conversation), then global keys.
- `Esc` / `Ctrl-C`: cancel the running turn (`cancel` request). With no
  turn, `Ctrl-C` clears the input. `Ctrl-C` twice within 1 s, or `Ctrl-D`
  with empty input, detaches and exits with `0`.
- Session ended (killed here or elsewhere): a notice "session ended:
  killed", input disabled, any key exits with `0` **(proposal, until the
  session list exists)**.
- Server disconnected: shown in the status line; reconnect every 500 ms for
  10 s (starting the server if needed, since sessions are saved), then exit
  with `1` and "server disconnected".

### Commands

Startup order, checks, exit codes and the detach line of `clai` and
`clai new`: [RFC 0002](0002-clai-without-command.md).

- `clai [--model M] [--name N]` and `clai new [--name N] [--model M] [DIR]`:
  send `new` (reusing `command::new::request` and `create`, which also print
  the "other sessions in this workspace" note before the TUI starts), then
  attach.
- `clai attach NAME`: attach; an unknown name fails with `1` before the
  terminal is touched.
- `clai attach` without a name: one session → attach; none → `no sessions`
  (exit `1`); several → **(proposal, until the session list exists)** fail
  with `1` and "several sessions: a, b, c; name one".
- Standard input or output not a terminal: exit `2` with "the TUI needs a
  terminal; use `clai chat` or `clai run`" (as `spec/command.md`,
  "Conventions", says).

### Terminal safety

- `terminal::Guard` enables raw mode, the alternate screen, bracketed paste,
  mouse capture (if `tui.mouse`) and keyboard enhancement; its `Drop` and an
  explicit `restore()` undo all of them. `restore()` is idempotent.
- A panic hook installed before setup calls `restore()` first, then the
  previous hook, so the panic message lands on the main screen.
- All errors return through `Result` to `run`, which restores before
  printing the error.
- In raw mode `Ctrl-C` and `Ctrl-Z` arrive as keys, not signals. `Ctrl-Z`
  is ignored in this milestone (see open questions).

## Dependencies

To approve with this RFC (latest versions at the time of the change):

| Crate | Use |
|-------|-----|
| `ratatui` | Layout, widgets, `TestBackend`. |
| `crossterm` with `event-stream` | Backend, async terminal events. |
| `pulldown-cmark` (no default features) | Markdown parsing. |
| `insta` (dev) | Frame snapshots. |
| `nix` feature `term` | `ttyname` for the client name. |

`unicode-width` comes with ratatui and is used for cursor and wrapping.

## Spec changes in the same change

- `spec/tui.md`: library assumption confirmed; "typing while a turn runs"
  rewritten as above; "older history loads in the background" kept as an
  assumption with a note that the first version replays all; small-terminal
  assumption confirmed.
- `spec/command.md`: remove "Until the TUI exists, only `clai new -d` is
  built"; note the temporary behaviour of `clai attach` with several
  sessions.
- `spec/architecture.md`: `src/tui/` tree instead of "planned";
  `client::Connection`.
- `implementation.md`: status of TUI core; `todo.md`: tasks done and moved.
- `readme.md`: `clai`, `clai new`, `clai attach` and the TUI keys.

## Testing

- **State tests** (`App::update`, no terminal): replay builds entries;
  delta then done; tool call states and live output; a permission question
  opens the prompt and routes keys; `a` + `Tab` + `Enter` sends the widened
  proposal; `e` sends `deny_message`; rejection keeps the prompt;
  `question_closed` from another client; limit prompt; `Esc`/`Ctrl-C`
  cancel; double `Ctrl-C` and `Ctrl-D` detach; `//` escape; message history;
  queued messages; follow mode and "↓ new output"; session ended;
  disconnect and reconnect keep view state.
- **Editor tests**: every key of "Input editor", multi-byte text, paste with
  newlines.
- **Markdown tests**: headings, emphasis, lists, inline code, code blocks,
  an unfinished block while streaming.
- **Snapshots** with `TestBackend` and `insta`: the layout of `spec/tui.md`,
  a permission prompt with a diff, a limit prompt, a command with cut output,
  ASCII symbols, `NO_COLOR`, "terminal too small", 80 × 24 and 120 × 40.
- **Connection tests** against the in-process server of `src/server/tests.rs`
  with a scripted provider: attach, stream a turn, answer a question through
  `Connection`, a second client answers first, server stop → `Disconnected`.
- **Performance check**: a test that builds 10 000 lines and asserts that a
  frame of the end and a scroll step stay under a time bound (debug build,
  generous bound, to catch accidental full reflows).
- **Binary tests**: `clai attach` without a terminal exits `2`; unknown name
  exits `1`. A pseudo-terminal end-to-end test stays in `todo.md`.

## Implementation plan

Each step ends with `cargo fmt --all`,
`cargo clippy --all-targets -- -D warnings` and `cargo test` passing, so the
work can stop after any step.

1. **Terminal**
   - Files: `Cargo.toml`, `src/tui/mod.rs`, `src/tui/terminal.rs`,
     `src/command/new.rs`, `src/command/attach.rs`, `src/lib.rs`
   - Result: Dependencies added; `Guard`, `restore()`, panic hook, signals;
     `clai`, `clai new`, `clai attach NAME` open an empty screen and leave
     on `Ctrl-D` (steps of RFC 0002).
   - Tests: No-terminal exit `2`; `restore()` twice is harmless.
2. **Connection**
   - Files: `src/client/connection.rs`, `src/client/mod.rs`
   - Result: `Connection` with reader and writer tasks, responses by id,
     `Disconnected`.
   - Tests: Against the in-process server: request while events stream, two
     clients, server stop.
3. **App and replay**
   - Files: `src/tui/app.rs`, `src/tui/conversation.rs`, `src/tui/view/`
     (header, status)
   - Result: `App::update` and effects; entries from events; replay before
     the first frame; header and status line.
   - Tests: State tests of replay, deltas, tool call states; snapshot of an
     idle session.
4. **Markdown and scrolling**
   - Files: `src/tui/markdown.rs`, `src/tui/conversation.rs`,
     `src/tui/view/conversation.rs`
   - Result: Markdown lines, per-entry line cache, visible slice, follow
     mode, "↓ new output", mouse wheel.
   - Tests: Markdown tests; follow-mode tests; 10 000-line timing test.
5. **Editor**
   - Files: `src/tui/editor.rs`, `src/tui/keys.rs`, `src/tui/view/input.rs`
   - Result: Multi-line input, line keys, history, paste, `//`, sending,
     queued mark.
   - Tests: Editor key tests; send and queued state tests.
6. **Tool details**
   - Files: `src/tui/conversation.rs`, `src/tui/view/conversation.rs`,
     `src/tui/style.rs`
   - Result: Expand/collapse, diffs, command output cut at
     `tool_output_lines`, conversation focus and selection, symbols,
     `NO_COLOR`.
   - Tests: Snapshots of a diff, cut output, ASCII, `NO_COLOR`.
7. **Prompts**
   - Files: `src/tui/prompt.rs`, `src/tui/view/prompt.rs`, `src/tui/keys.rs`
   - Result: Permission prompt with `a`/`Tab` widening and `e` message;
     limit prompt; rejected answers; answered elsewhere.
   - Tests: Prompt state tests; snapshots of both prompts.
8. **Leaving**
   - Files: `src/tui/mod.rs`, `src/tui/app.rs`
   - Result: Cancel, double `Ctrl-C`, `Ctrl-D`, session ended, reconnect for
     10 s, `SIGTERM`/`SIGHUP`, detach line.
   - Tests: State tests of each exit; reconnect keeps view state.
9. **Commands**
   - Files: `src/tui/app.rs`, `src/tui/view/` (help, confirm)
   - Result: `/help`, `/detach`, `/quit`, `/model ARG`, `/rename`, `/kill`
     with confirmation; others say they come later.
   - Tests: State tests per command.
10. **Docs**
   - Files: `spec/tui.md`, `spec/command.md`, `spec/architecture.md`,
     `implementation.md`, `todo.md`, `readme.md`, `rfcs/readme.md`
   - Result: Spec changes listed above; milestone status; RFC status `done`.
   - Tests: Full test run; manual run in a real terminal (kitty or foot, and
     one without keyboard enhancement).

## Risks

| Risk | Handling |
|------|----------|
| Markdown re-parse of a long streaming block each frame. | Parse only the open block; bound by `max_fps`; measure in the performance test. |
| `Shift-Enter` not reported by many terminals. | `Alt-Enter` always works; the help says so. |
| A panic between raw-mode setup and the hook leaves the terminal broken. | Install the hook before setup; the guard restores on `Drop` too. |
| Snapshot churn from layout tweaks. | Few, focused snapshots; state tests carry the behaviour. |
| Full replay of a very long session is slow. | Measured; the event-log bound in `todo.md` and paged loading are the fallbacks. |

## Open questions

1. Header `1/3 ●1` needs the state of other sessions. Proposal: show the
   position and waiting count from a `list` on attach, refreshed every 2 s
   in this milestone; a server notice for session state changes (protocol
   3) comes with the session list. Alternative: add the notice now.
2. `Ctrl-Z`: suspend the TUI (restore, `SIGTSTP`, set up again on
   `SIGCONT`) now, or ignore it?
3. While the session is killed or not loaded: exit on any key (proposed),
   or offer "new session / exit" as the spec says for "no session left"?
4. `/rename` and `/kill` are cheap to build now; keep them in this
   milestone or move them with `/sessions` to the next?
