# Status view

- RFC: 0007
- Status: draft
- Spec: [`spec/tui.md`](../spec/tui.md), "Input editor" (commands), "Footer";
  [`spec/sessions.md`](../spec/sessions.md), "Persistence";
  [`spec/architecture.md`](../spec/architecture.md) (protocol);
  [`spec/agent-loop.md`](../spec/agent-loop.md) (limits, retries);
  [`spec/command.md`](../spec/command.md) (`clai chat`)

## Summary

`/status` opens a box in the TUI with the full status of a session and its
model, and statistics over the whole session:

- **Session:** state, workspace, clients, queue and where the session is saved.
- **Model:** provider, API, key, context use and settings.
- **This turn:** progress against `[limits]`.
- **Totals:** turns, requests, retries, errors, tokens and time.
- **Tool calls:** a table per tool.
- **Questions:** counts per answer and per client.

`/status NAME` shows another session, running or saved, without attaching to
it. The box updates while it is open.

The data comes from a new server request, `session_status`. The session keeps
running counters for it, and builds them from its log on resume. Time needs a
timestamp per logged event, so the event log gets one. Retries need an event of
their own. `clai chat` gets the same data as text with `/status`, and `/usage`
stays as a short form.

## Motivation

- Today the TUI shows a single footer line: name, workspace, model, place,
  context and tokens. Nothing says what the limits are, how far this turn is
  from them, which clients are attached, or whether the key is set.
- Long sessions raise questions the user cannot answer from the screen:
  - How many requests and tokens did this take?
  - Which tools failed or were denied?
  - How long did the model work?
  - How often did the provider have to retry?
- A detached session that waits or works can only be inspected by attaching
  to it.
- `clai chat` has only `/usage`: two token totals.

## Scope

In:

- `/status [NAME]` in the TUI: the status box, live while open, scrollable.
  `NAME` is completed from the session names.
- A new `session_status` request and its answer, protocol version 4.
- Running statistics in the session, built from the log on resume and read
  from the log for a saved session.
- A timestamp on each event in `events.jsonl`. The format stays `1`; an event
  without one has an unknown time.
- A `retry` event when a model request is tried again, with a notice in the
  conversation.
- `/status [NAME]` in `clai chat`, as text. `/usage` stays and shows the token
  lines only.

Out:

- Cost in money: there are no prices in the configuration. This can come
  later with a `price` key per model.
- `clai status NAME` in the terminal: the data is there, so the command can
  follow in its own RFC.
- Times in replayed conversation lines (`tui.timestamps` for old events): the
  timestamp makes this possible, but it is not part of this RFC.
- Statistics across sessions or across the server.

## Design

### What the box shows

```
┌ Status ─ clai ───────────────────────────────────────────────────────────┐
│ Session                                                                  │
│   name         clai  (id 9)                                              │
│   state        ● waiting: run_command asks                               │
│   workspace    ~/ws/rust/clai                                            │
│   extra dirs   ~/ws/shared-lib  ⚠ missing                                │
│   created      2026-11-07 17:46  (2h 10m ago)                            │
│   last active  3m ago                                                    │
│   clients      2: tui pts/3 (this), chat pts/5 (watching)                │
│   queued       1 message                                                 │
│   saved in     ~/.local/state/clai/sessions/9  (1.2 MB)                  │
│                                                                          │
│ Model                                                                    │
│   model        anthropic/sonnet  (claude-sonnet-5-5)                     │
│   api          anthropic  https://api.anthropic.com                      │
│   key          set (ANTHROPIC_API_KEY)                                   │
│   context      48.2k / 200k  ██████░░░░░░░░░░░░░░  24%                   │
│   settings     temperature default ─ max_retries 3                       │
│   changes      2; first anthropic/haiku                                  │
│                                                                          │
│ This turn                                                                │
│   requests     7 / 50                                                    │
│   tool calls   12 / 120                                                  │
│   running      4m 12s                                                    │
│                                                                          │
│ Totals                                                                   │
│   turns        23: 20 done, 1 cancelled, 1 limit, 1 error                │
│   requests     142 ─ 3 retries ─ 1 error                                 │
│   tokens       in 2.1M ─ out 84.3k                                       │
│   working      1h 02m of 2h 10m                                          │
│                                                                          │
│ Tool calls     calls  done  error  denied  cancelled                     │
│   read_file       61    60      1       0          0                     │
│   edit_file       24    21      1       2          0                     │
│   run_command     18    15      2       0          1                     │
│   all            103    96      4       2          1                     │
│                                                                          │
│ Questions      18: 12 yes ─ 2 always ─ 2 no ─ 1 explain ─ 1 cancel       │
│                by tui pts/3 15 ─ chat pts/5 3                            │
│                                                                          │
│ Server         0.4.0 ─ pid 129493 ─ 3 sessions, 2 running ─ 3 clients    │
│                                                                          │
│ Up Down PgUp PgDn scroll ─ Esc close                                     │
└──────────────────────────────────────────────────────────────────────────┘
```

**Rows:**

- Rows that do not apply are left out. Examples: "This turn" when idle, extra
  dirs when there are none, "queued" with none, "changes" when the model never
  changed.
- A time that is unknown, because the event came from an older log, shows `?`.
  "working" then counts only the turns with known times, and says so:
  `≥ 1h 02m`.
- The context bar uses the theme's `user` / `dim` colours and turns `warning`
  at 80 % **(proposal)**. With an ASCII terminal it is `[######......]`.
- The model's `key` row comes from `models` (as in `clai models`).
  `api_key_cmd` is not run.
- For a saved session: "state" says `saved`. There are no clients and no
  "This turn". "context" is the last `usage` of the log.

**Box behaviour:**

- The box is as wide as the settings box. It scrolls when it is taller than
  the screen, and closes with `Esc`, `q` and `Ctrl-C`.
- It updates while open: it asks again at most once per second while the
  session works, and once after a `state`, `usage` or `turn_end` event
  **(proposal)**. When idle it does not poll.
- A question that opens closes the box, as for the other boxes.

### Statistics in the session

A `Stats` value in the session actor is updated by `Stats::count(&event, at)`
for every event the session logs:

| Counter | From |
|---------|------|
| turns, by end reason | `turn_end.reason` |
| requests | `turn_end.requests`, plus the running turn's count |
| retries | `retry` (new, below) |
| errors | `error` events; requests that failed after the retries |
| tokens | `usage` (as `TokenTotals` today) |
| tool calls per tool and final state | `tool_call` in a final state |
| questions per answer, per client | `question_closed` (`answered` by, `cancelled`, `closed`) |
| model changes, first model | `model_changed` |
| working time | the `at` times of `state` events: working until idle or waiting |
| last activity | the `at` time of the last event |

- **On resume:** the same function runs over the replayed log, as
  `TokenTotals::of_log` does today. `TokenTotals` becomes part of `Stats`.
- **For a saved session:** `session_status` reads `events.jsonl` and counts it
  without resuming the session, as `export` reads it.
- The counters are not written to `session.json` **(proposal)**: the log is the
  truth, and counting it is cheap compared to replaying it to a client.

### Timestamps in the log

- Each line of `events.jsonl` gets `"at": <milliseconds since 1970>` next to
  the event's own fields.
- Loading ignores a missing `at`, so old logs load as before and the format
  stays `1`.
- The protocol's `event` message is unchanged **(proposal)**: clients do not
  need the time for this RFC.

### Retries

- The provider's retry loop calls a callback before each new try, with the
  attempt, the limit and the reason (`rate limited`, `HTTP 503`, a network
  error).
- The agent emits `retry { attempt, max, reason }`. The event is logged and
  sent to clients.
- The TUI shows it as a notice: `retrying (2/3): rate limited`. The next
  successful text replaces nothing. `clai chat` prints the same line.

### Protocol

- `session_status { session }` answers `session_status { status }`.
- It works for running sessions and saved ones; a saved session is not
  resumed.
- The payload has:
  - **session:** `SessionInfo` (as in `list`), extra dirs with whether each
    exists, the session directory and its size, and the clients as
    `{ name, answers }`.
  - **model:** `ModelInfo` (as in `models`), with the API, the base URL,
    `temperature` and `max_retries` from the configuration in effect. It is
    `null` when the model is no longer configured.
  - **limits:** the limits in effect (user file, lowered by the project file).
    While a turn runs, also its requests and tool calls so far, and its start
    time.
  - **context_tokens:** from the last `usage`.
  - **stats:** `Stats` as above.
  - **server:** `Status`, as the `status` request returns it.
- `VERSION` goes to 4. An older client gets the usual "restart the server"
  error.

### TUI

- **Files:**
  - `tui/status.rs`: `StatusBox` with the last answer, the scroll position and
    whether a refresh is on its way.
  - `tui/view/status_box.rs`: draws it. `view/status.rs` is the footer and
    keeps its name.
- **In `App`:**
  - `/status [NAME]` sends `session_status`, and the answer opens the box.
  - An unknown name stays in the input with a notice, as for `/attach`.
  - Live refresh comes from `Tick` and from events, as above.
  - The box closes on a switch, and when its session ends.
- **Commands and completion:** `/status` is added to the command list and to
  argument completion with the session names.

### `clai chat`

`/status [NAME]` prints the same sections as text, without the bar, and
aligned as in the box. `/usage` prints only the "tokens" and "context" rows.

## Spec changes

- `spec/tui.md`:
  - `/status [NAME]` in the command table.
  - A new section "Status box" with the rows and keys above.
  - The `retry` notice.
- `spec/command.md`: `/status` and `/usage` in `clai chat`.
- `spec/sessions.md`, "Persistence": `at` in `events.jsonl`, and the counting
  of statistics from the log.
- `spec/architecture.md`: `session_status`, and the `retry` event; protocol 4.
- `spec/agent-loop.md`: retries emit `retry`.

## Testing

- **`Stats::count`:** unit tests over hand-made logs.
  - All counters, and turns cut by a crash.
  - Old events without `at`: the "≥" working time.
  - Questions answered by several clients.
- **Log:** loading a log without `at` and with it; the round trip keeps `at`.
- **Provider:** an HTTP test where the server answers 503 twice and then 200
  gives two `retry` events, then the answer.
- **Server tests:**
  - `session_status` for a running session, a working one (turn progress),
    and a saved one, which stays saved.
  - A model that is no longer configured.
- **TUI:**
  - App tests for `/status`, `/status NAME`, an unknown name, live refresh
    while working and none when idle, close on switch and on question.
  - Snapshot tests of the box: a full one, a saved session, ASCII, a narrow
    terminal.
- **`clai chat`:** the text output of `/status` and `/usage`.

## Implementation plan

1. **Timestamps in the log**
   - Status: todo
   - Files: `session/store.rs`,
     `spec/sessions.md`
   - Result: new events carry `at`; old logs load unchanged
   - Tests: round trip; old log
2. **Retry event**
   - Status: todo
   - Files: `provider/remote.rs`, `agent/events.rs`, `agent/` loop,
     `tui/conversation.rs`, `command/chat/`, `spec/agent-loop.md`
   - Result: retries are logged and shown as notices
   - Tests: HTTP test with 503s; TUI and chat notice
3. **Statistics**
   - Status: todo
   - Files: `session/stats.rs` (new), `session/mod.rs`, `protocol/mod.rs`
     (`TokenTotals` into `Stats`)
   - Result: running counters, built on resume and from a saved log
   - Tests: `Stats::count` unit tests
4. **`session_status` request**
   - Status: todo
   - Files: `protocol/mod.rs`, `server/requests.rs`, `session/mod.rs`,
     `server/tests.rs`, `spec/architecture.md`
   - Result: protocol 4; the status of running and saved sessions
   - Tests: server tests above
5. **Status box in the TUI**
   - Status: todo
   - Files: `tui/status.rs`, `tui/view/status_box.rs`, `tui/app.rs`,
     `tui/commands.rs`, `tui/tests.rs`, `spec/tui.md`
   - Result: `/status [NAME]`, live, scrollable
   - Tests: App and snapshot tests
6. **`clai chat`**
   - Status: todo
   - Files: `command/chat/talk.rs`, `spec/command.md`
   - Result: `/status [NAME]` as text; `/usage` short
   - Tests: text output
7. **Docs**
   - Status: todo
   - Files: `readme.md`, `todo.md`, `implementation.md`, this RFC
   - Result: status `done`

## Alternatives

- **TUI only, from the events it already gets:** no protocol change, but no
  limits, clients, extra dirs or provider settings, and no status for a
  session that is not attached. Rejected by the user.
- **Counters saved in `session.json`:** a saved session's status would be
  read without the log. But the counters could drift from the log after a
  crash, the format would change, and reading the log is cheap enough.
- **Server pushes the status to clients:** no polling, but it is sent to
  every client on every event, for a box that is rarely open.
- **Retries counted only in the provider and sent with `usage`:** a request
  that fails after all retries has no `usage`, and the user would not see
  the retries while they happen.

## Open questions

1. Should the box update while open, as proposed, or only show the moment it
   was opened (with `r` to refresh)?
2. Should the context bar turn to the warning colour at 80 %, and should the
   footer do the same?
3. Should a turn's time be shown per turn, for example the five longest
   turns, or are the totals enough?
4. Should the `retry` notice stay in the conversation, or show only in the
   status row while it lasts?
