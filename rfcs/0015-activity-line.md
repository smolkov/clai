# Activity line

- RFC: 0015
- Status: done
- Spec: [`spec/tui.md`](../spec/tui.md), "Layout", "Conversation entries",
  "Appearance", "Performance"; [`spec/config.md`](../spec/config.md), "TUI"

## Summary

While the agent works, the TUI shows an animated line between the
conversation and the input:

```
✻ Prestidigitating… (14s · ↓ 1.2k tokens · Esc cancel)
```

- A glyph that pulses, so the user sees at once that something happens.
- A playful verb, chosen at random for each turn. It changes to a plain
  phrase when there is something concrete to say: `Running cargo test…`.
- The time since the turn started, the tokens received in this turn, and how
  to stop it.

The line is gone when the session is idle or a question is open.

## Motivation

- While the model thinks, nothing on the screen moves. The footer says
  `waiting for model… 2 requests, 1 tool call`, but this text does not change
  for many seconds. The user cannot tell a slow model from a hung one.
- The footer is at the bottom, in dim text. The eye is on the input and the
  last line of the conversation.
- The tool call spinner moves once per second (the TUI's tick), and only while
  a tool runs.
- A line with time and tokens answers "is it still alive?" and "is it worth
  waiting?" without opening `/status`.

## Scope

In:

- The activity line in the TUI, its phases, the verbs and the animation.
- A faster animation tick while the session works, also used by the tool call
  spinner.
- `tui.activity` to change or turn it off, in `/settings` too.

Out:

- `clai chat`: it prints text as it comes, and a line rewritten in place
  would mix with streamed output. It can follow, for example as a spinner
  only before the first text.
- New protocol events. The TUI already gets everything it needs: `state`,
  text deltas, `tool_call`, `usage`, `retry`.
- Progress bars for tools (bytes read, test counts).

## Design

### Where it is

```
The test fails because `parse` drops the last line. Let me look.

▸ read_file src/parse.rs                                          ✓ 120 lines

✻ Prestidigitating… (14s · ↓ 1.2k tokens · Esc cancel)
──────────────────────────────────────────────────────────────────────────────
> _
──────────────────────────────────────────────────────────────────────────────
  waiting for model… 2 requests, 1 tool call · Esc cancel · /help · Ctrl-D detach
  project · ~/ws/project · anthropic/sonnet · 1/3 · 12.3k/200k · in 45.1k out 3.2k
```

- It takes the last row of the conversation area, with an empty row above it
  **(proposal)**. The conversation is two rows shorter while it is shown, and
  follows the end as before. Nothing jumps: the rows of the conversation move
  up by two when the turn starts and back down when it ends.
- It is not part of the conversation: it is not saved, not scrolled, and not
  selected with the conversation focus.
- When the user scrolled up, the line stays at the bottom, so it is always
  visible.
- It is hidden when:
  - the session is `idle` or `saved`;
  - a permission, limit or question prompt is open: the prompt bar says what
    is needed (the session state is `waiting` then);
  - the terminal is shorter than 14 rows **(proposal)**.

### What it says

| Phase | When | Text |
|-------|------|------|
| Thinking | after the request is sent, before the first text | a verb: `Prestidigitating…` |
| Writing | text deltas arrive | a verb, as above |
| Tool | a tool call runs | `Running cargo test…`, `Reading src/parse.rs…`, `Editing src/parse.rs…` |
| Retry | after a `retry` event, until the next text | `Retrying (2/3): rate limited…` |
| Cancelling | after `Esc`, until the turn ends | `Cancelling…` |

- **Verbs:** a built-in list of about 50 playful gerunds (`Pondering`,
  `Prestidigitating`, `Untangling`, `Percolating`, `Conjuring`, …). One is
  chosen at random when the turn starts and kept for the turn
  **(proposal)**, so the line does not flicker between words.
- **Tool phase:** the text comes from the tool and its main argument, as on
  the tool call's summary line, cut to fit. With several tools running, the
  first is named and `+2` follows.
- **In brackets:**
  - The time since the turn started: `14s`, `2m 05s`, `1h 03m`.
  - `↓ 1.2k tokens`: the output tokens of this turn so far. Between `usage`
    events this is estimated from the text received (4 characters per token
    **(proposal)**), and the estimate is replaced by the counted number when a
    `usage` event arrives. It is left out until the first text.
  - `Esc cancel`, while the input is empty and has the focus. Otherwise `Esc`
    does something else, and the hint would be wrong.
- Parts that do not fit are dropped from the right: the hint first, then the
  tokens, then the time. The verb is cut last, with `…`.
- After a turn the line disappears. No "done in 14s" line is left in the
  conversation **(proposal)**; the `turn_end` notice is enough.

### Animation

- **Glyph:** the frames `· ✢ ✳ ✶ ✻ ✽ ✻ ✶ ✳ ✢`, one per 120 ms
  **(proposal)**, in the theme's `accent` colour. With
  `tui.symbols = "ascii"`: `| / - \`.
- **Shimmer (proposal):** the verb is drawn in the `dim` colour with a
  brighter band of three characters that moves from left to right, once per
  two seconds. With `NO_COLOR` the verb is plain and only the glyph moves.
- **Stalled:** when no event has arrived for 30 s while thinking, the glyph
  turns to the `warning` colour and the time stays visible. This shows a slow
  provider without saying it failed. The provider's own timeouts and retries
  are not changed.

### Tick

- Today the TUI ticks once per second. While the session works and the line
  or a tool spinner is on the screen, a second interval of 120 ms runs and
  sends `Input::Frame`. It stops when the session is idle, so an idle TUI
  still wakes once per second.
- `Frame` only moves the animation and marks the activity line and running
  tool calls dirty. Nothing else is redrawn from cache. The limit of 30 frames
  per second (see "Performance") is kept.
- The tool call spinner uses the same frame counter, so it moves smoothly too.

### Turn start time

- The TUI notes the time when it sees `state: working` after `idle`.
- When the TUI attaches to a session that is already working, it takes the
  turn's start time from `session_status` (`stats`, the running turn's `started`, from
  RFC 0007) **(proposal)**, so the time is right after `/attach` and after a
  switch. Until that answer arrives the time is left out.

### State

- `tui/activity.rs` (new): `Activity { started, phase, verb, frame,
  tokens_counted, chars_since_usage, last_event }`. It is pure: `App` feeds it
  events and frames, and the view reads it. A seeded random number generator
  chooses the verb, so tests are stable.
- `tui/view/activity.rs` (new): draws the line from `Activity`, the look and
  the width.
- `tui/view/mod.rs`: the layout gives the conversation one row less while the
  line is shown.
- `tui/style.rs`: `Symbols::activity` with the frames for Unicode and ASCII.

### Configuration

```toml
[tui]
activity = "playful"   # "playful" (default), "plain", "off"
```

- `playful`: as above.
- `plain`: no verbs; the thinking and writing phases say `Thinking…` and
  `Writing…`. No shimmer.
- `off`: no line. The tool call spinner still moves at 120 ms.
- `/settings` lists it with the other `[tui]` settings and previews it.

Custom verb lists are left out **(proposal)**; see the open questions.

## Spec changes

- `spec/tui.md`:
  - "Layout": the activity line in the drawing and in the table of areas.
  - A new section "Activity line" with the phases, the parts, the animation
    and when it is hidden.
  - "Conversation entries": the tool call spinner moves with the frame tick.
  - "Performance": the 120 ms frame tick while working; idle ticks stay at one
    second.
- `spec/config.md`, "TUI": `tui.activity`.

## Testing

- **`Activity`:** unit tests over event sequences:
  - The phases: thinking, writing, tool, retry, cancelling, and back.
  - The token estimate, and its replacement by `usage`.
  - The stalled state after 30 s without events.
  - The same seed gives the same verb; a new turn gives a new choice.
- **Layout:** snapshot tests of the screen with the line: thinking, a running
  tool, several tools, a retry, a narrow terminal where parts are dropped,
  ASCII, `plain`, and `off`.
- **App:**
  - The line appears on `working` and goes on `idle`.
  - It hides while a prompt is open and comes back after the answer.
  - `Frame` inputs run only while working.
  - Attach to a working session takes the start time from `session_status`.
  - The `Esc cancel` hint is shown only when `Esc` cancels.
- **Check:** a pseudo-terminal run of the real TUI with a fake model server
  that waits 5 s before its answer, as in RFC 0007.

## Implementation plan

1. **Frame tick**
   - Status: done
   - Files: `tui/mod.rs`, `tui/app.rs`, `tui/conversation.rs`, `spec/tui.md`
   - Result: a 120 ms tick while working; the tool spinner uses it
   - Tests: App tests for when frames run
2. **Activity state**
   - Status: done
   - Files: `tui/activity.rs`, `tui/app.rs`
   - Result: phases, verbs, time and tokens follow the events
   - Tests: `Activity` unit tests
3. **Activity line**
   - Status: done
   - Files: `tui/view/activity.rs`, `tui/view/mod.rs`, `tui/style.rs`,
     `spec/tui.md`
   - Result: the line on the screen, animated, cut to fit
   - Tests: snapshot tests
4. **Start time on attach**
   - Status: done
   - Files: `tui/app.rs`
   - Result: the right time after `/attach` and a switch
   - Tests: App test
5. **Setting**
   - Status: done
   - Files: `config/`, `tui/settings.rs`, `spec/config.md`
   - Result: `tui.activity` in the file and in `/settings`
   - Tests: config and settings tests
6. **Docs**
   - Status: done
   - Files: `readme.md`, `todo.md`, this RFC
   - Result: status `done`

## Alternatives

- **Animate the footer only:** no new row, but the footer is dim, far from
  the eye, and already full of hints.
- **A spinner entry in the conversation:** it would scroll away when the user
  scrolls up, and it would have to be kept out of the log.
- **One fixed word (`Working…`):** simpler, and it is what `plain` does. The
  playful verbs make long waits feel lighter, and they cost nothing.
- **Redraw the whole screen at 120 ms:** simpler, but it wastes work on long
  conversations. Only the line and running calls are dirty.
- **A server-side "still alive" event:** the TUI would know the server is
  waiting on the provider, not hung. But the TUI's connection already notices
  a lost server, and a quiet provider is what the stalled colour shows.

## Notes from building

- The proposals were taken as they stand: above the input, one verb per
  turn, the shimmer, output tokens only. Custom verbs are left for later
  (`todo.md`).
- The line takes two rows of the conversation (the empty row and itself);
  the first draft said one.
- The tool phase counts pending calls too, as their spinners do. The
  stalled colour applies while thinking only: no text, no tool and no retry
  on the line; a long tool run or a stream of text is no stall.
- The retry phase also ends at the next tool call, which shows the request
  went through.
- `Ctrl-C` that cancels shows `Cancelling…` as `Esc` does.
- `session_status` for the start is asked once, on the next frame or tick
  after an attach; the answer also serves an open status box. Without a
  `started` in the running turn, `working_since` is used.
- `App` keeps the verb generator's seed across turns; the shell seeds it
  from the clock, the tests from a constant.
- Check: a pseudo-terminal run (tmux) of the real TUI with a fake model
  server that waits 5 s and then streams for 4 s. It showed the glyph moving
  at the frame tick, the time counting, the token estimate during streaming,
  the line gone after the turn with a new verb for the next one, and the
  right time (`2s`) right after attaching to a session 2 s into its turn. It
  found that the shell replaced the activity state after the first replay;
  the shell now only reseeds it.

## Open questions

All answered by the proposals; see "Notes from building".
