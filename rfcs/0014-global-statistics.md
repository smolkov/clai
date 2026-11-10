# Global statistics

- RFC: 0014
- Status: done
- Spec: [`spec/tui.md`](../spec/tui.md), "Status box";
  [`spec/sessions.md`](../spec/sessions.md), "Persistence";
  [`spec/architecture.md`](../spec/architecture.md) (protocol);
  [`spec/command.md`](../spec/command.md) (`clai chat`)

## Summary

The status box gets two tabs. **Session** is the box of RFC 0007, unchanged.
**Global** is new and shows statistics over all sessions of the server:

- **Totals:** sessions, turns, requests, retries, errors, tokens and working
  time.
- **Models:** a table per model with its requests and tokens.
- **Tokens per day:** a bar chart of input and output tokens per day.

A range applies to the whole tab: the last 7 days, the last 30 days, or all
time. The data comes from a new server request, `global_stats`, that counts
the logs of all saved sessions. `clai chat` gets `/stats` with the same data as
text.

## Motivation

- `/status` answers "what did this session cost?", but not "what did I use
  this week?" or "which model do I use most?". Today the user has to open
  every session and add the numbers.
- Tokens are the closest thing to cost that `clai` knows. A view per model and
  per day shows trends and spikes: a day with a runaway turn, or a switch to a
  more expensive model.
- RFC 0007 left "Statistics across sessions or across the server" out of
  scope. Since then every event has a timestamp, so counting by day is now
  possible.

## Scope

In:

- Tabs in the status box: **Session** and **Global**.
- `/stats [7d|30d|all]` in the TUI: opens the box on the Global tab.
  `/status` keeps opening it on the Session tab.
- A new `global_stats { range, offset }` request and its answer, protocol
  version 7.
- Counting per model: tokens and requests are given to the model the session
  used at that time.
- Counting per day, in the client's local time.
- `/stats [7d|30d|all]` in `clai chat`, as text.

Out:

- Cost in money, as in RFC 0007: there are no prices in the configuration.
- Sessions that were deleted: their log is gone, so their tokens are gone too.
  See the open questions.
- Exported bundles (`clai export`) that are not imported: they are not
  sessions of the server.
- Charts other than tokens per day (requests per day, tool calls per day).
  The data has them; the chart can follow.
- `clai stats` in the terminal without the TUI or chat.

## Design

### Tabs in the status box

```
┌ Status ─ Session │ Global ──────────────────────── 7 days │ 30 days │ all ┐
│ Totals         2026-11-04 – 2026-11-10                                   │
│   sessions     12 active of 31 ─ 4 running                               │
│   turns        87: 80 done, 3 cancelled, 2 limit, 2 error                │
│   requests     1.4k ─ 21 retries ─ 3 errors                              │
│   tokens       in 18.2M ─ out 612k                                       │
│   working      6h 40m                                                    │
│                                                                          │
│ Models         sessions  requests      in     out   share                │
│   anthropic/sonnet     9      1.1k   14.9M    498k   ████████████  81%   │
│   anthropic/haiku      4       260    2.8M     96k   ██░░░░░░░░░░  15%   │
│   openai/gpt-5         1        40    0.5M     18k   ░░░░░░░░░░░░   4%   │
│   all                 12      1.4k   18.2M    612k                       │
│                                                                          │
│ Tokens per day   █ in  █ out                                             │
│  4.0M ┤                    █                                             │
│       │             █      █                                             │
│  2.0M ┤       █     █      █      █                                      │
│       │ █     █     █   ▆  █      █   ▂                                  │
│     0 ┼─█─────█─────█───█──█──────█───█──                                │
│        Wed   Thu   Fri  Sat Sun  Mon  Tue                                │
│        max 4.1M on Sun 11-08 ─ avg 2.6M                                  │
│                                                                          │
│ Tab switch tab ─ 7 3 a range ─ Up Down scroll ─ Esc close                │
└──────────────────────────────────────────────────────────────────────────┘
```

**Keys:**

- `Tab` and `Shift-Tab` switch the tab. `Left` / `Right` do the same
  **(proposal)**.
- On the Global tab: `7` shows 7 days, `3` shows 30 days, `a` shows all time.
  `r` goes to the next range **(proposal)**.
- Scrolling and closing stay as in RFC 0007. Each tab keeps its own scroll
  position.
- The title shows both tabs and, on the Global tab, the three ranges; the
  current one is bold.

**Totals:**

- "sessions" counts the sessions with at least one event in the range, of all
  saved sessions, and says how many run now.
- "working" sums the working time in the range, as in RFC 0007. With events
  without a time it says `≥`.

**Models:**

- One row per `provider/model`, sorted by tokens (in plus out), most first.
- "sessions" counts the sessions that used the model in the range.
- "share" is the model's part of all tokens, with a bar. The bar uses the
  theme's `user` colour; with an ASCII terminal it is `[####....]`.
- A model that is no longer configured keeps its row, with its name dim.
- Tokens that cannot be given to a model (see below) go to a row `unknown`.

**Tokens per day:**

- One column per day, input and output stacked: input in the theme's `user`
  colour, output in `assistant` **(proposal)**. The top of a bar uses the
  eighth blocks `▁▂▃▄▅▆▇█`; with an ASCII terminal the bars are `#` and `+`.
- The height is 8 rows **(proposal)**. The y axis has three labels: 0, the
  middle and the top, rounded to a short number (`2.0M`).
- **7 days:** one column per day, labelled by weekday, as wide as the box
  allows.
- **30 days:** one column per day, labelled every 7th day with `MM-DD`.
- **All time:** one column per day from the first event to today. When there
  are more days than columns, a column is a week, then a month
  **(proposal)**; the line under the chart says which.
- Days without tokens are an empty column; they are not left out.
- Under the chart: the highest day and the mean over the days of the range.
- Tokens of events without a time are not in the chart. The line under the
  chart says `1.2M tokens without a date` then.
- In a box narrower than the 7-day chart (about 40 columns), the chart is left
  out and a line says so.

**Behaviour:**

- The Global tab asks with `global_stats` when it is opened, and again when
  the range changes. The answers are kept, so switching back is instant.
- It does not poll **(proposal)**: global numbers move slowly, and counting all
  logs is the most expensive request. The title shows how old the answer is
  once it is older than a minute, and `R` asks again.
- `/stats` works without an attached session.

### Counting per model

`usage` events do not carry a model. While the counter walks a session's log
it keeps the model in effect:

- It starts with the `from` of the first `model_changed`, or with the model in
  `session.json` when the session never changed it.
- Each `model_changed` sets the model for the events after it.
- A first `model_changed` without `from` (logs of an older `clai`) gives the
  events before it to `unknown`.

**(proposal)** New `usage` events also get `model`. It makes the count exact
when the model changes while a request runs, and later logs need no walking.
Old events without it are counted as above.

### Counting per day

- An event's day is its `at` plus the client's UTC offset, divided by a day.
  The client sends its offset (`tui::local_offset()`), so the server needs no
  time zone.
- The range is the last N days in that time zone, today included.
- An event without `at` is counted in "all" only, and only in the totals and
  the models table.

### On the server

- `GlobalStats::of_logs` runs a counter over the log of each saved session.
  It reuses the `Stats::count` of RFC 0007 per session and adds the model and
  the day of each event. The per-day buckets are `BTreeMap<day, DayTokens>`.
- Running sessions are counted from their log too: each event is appended
  before it is sent, so the log is up to date.
- **Cache (proposal):** the server keeps one summary per session, keyed by the
  size of `events.jsonl`. A log that grew is counted again from the start (the
  summary is small, the logs are not read often). A request for another range
  or offset uses the same per-session summaries: they keep tokens per day in
  UTC hours, so any offset can be applied later **(proposal)**.
- A log that fails to load is left out, and the answer counts it in
  `skipped`, shown as a line in the tab.

### Protocol

- `global_stats { range, offset }`: `range` is `7d`, `30d` or `all`; `offset`
  is seconds east of UTC.
- The answer `global_stats { stats }` has:
  - **range:** the first and last day, as days since 1970.
  - **sessions:** in the range, all saved, running now.
  - **totals:** turns, requests, retries, errors, tokens and working time, as
    in `Stats`.
  - **models:** a list of `{ model, configured, sessions, requests,
    input_tokens, output_tokens }`.
  - **days:** a list of `{ day, input_tokens, output_tokens }`, one per day of
    the range, empty days included.
  - **undated_tokens** and **skipped**.
- Weeks and months for "all" are made by the client from the days: the server
  does not know the width of the box.
- `VERSION` goes to 7.

### TUI

- **Files:**
  - `tui/status.rs`: `StatusBox` gets `tab`, a scroll position per tab, the
    range, and the last `GlobalStats` per range.
  - `tui/view/status_box.rs`: draws the tabs in the title and the tab in use.
  - `tui/view/day_chart.rs` (new): the stacked bar chart, pure on its input,
    so it is tested by snapshots alone. ratatui's `BarChart` does not stack
    two series and has no y axis, so the chart draws its own cells.
  - `text/global.rs` (new): the rows of the Totals and Models sections, for the
    box and for `clai chat`, as `text::status` does.
- **In `App`:** `/stats [RANGE]` opens the box on the Global tab with that
  range, 7 days by default **(proposal)**. `/stats` goes into the command list
  and its argument into completion.

### `clai chat`

`/stats [7d|30d|all]` prints the Totals and Models sections as text, and the
days as one line each with a bar:

```
Tokens per day (in + out)
  Wed 11-04   1.2M  ██████
  Thu 11-05   2.3M  ███████████
  ...
```

## Spec changes

- `spec/tui.md`:
  - `/stats [RANGE]` in the command table.
  - "Status box": the tabs, their keys, and a new part "Global tab" with the
    sections and the chart above.
- `spec/command.md`: `/stats` in `clai chat`.
- `spec/sessions.md`, "Persistence": `model` in `usage`; how statistics are
  counted per model and per day.
- `spec/architecture.md`: `global_stats`; protocol 7.

## Testing

- **Counting:** unit tests over hand-made logs.
  - A model change in the middle: tokens split between the two models.
  - An old log with `model_changed` without `from`: the `unknown` row.
  - `usage` with and without `model`.
  - Days around midnight with a positive and a negative offset.
  - Events without `at`: in "all", not in 7 days, counted as undated.
  - Ranges: an event 7 days ago is in 7 days only if it is today's date minus 6.
- **Cache:** a log that grew is counted again; the same log is not read twice.
- **Server tests:** `global_stats` over several saved sessions and a running
  one; a broken log is skipped and counted.
- **Chart:** snapshot tests of 7 days, 30 days, all time with months, empty
  days, all days empty, ASCII, and a narrow box.
- **TUI:** App tests for `/stats`, `/stats 30d`, switching tabs and ranges,
  `R`, and a status box that keeps its Session tab working as before.
- **`clai chat`:** the text output of `/stats`.

## Implementation plan

1. **Model in `usage`**
   - Status: done
   - Files: `agent/events.rs`, the agent loop, `session/store.rs`,
     `spec/sessions.md`
   - Result: new `usage` events carry the model; old logs load unchanged
   - Tests: round trip; old log
2. **Global counting**
   - Status: done
   - Files: `protocol/stats.rs` (`GlobalStats`, `DayTokens`, `ModelTotals`),
     `session/global.rs` (new)
   - Result: summaries per session, combined for a range and an offset
   - Tests: counting tests above
3. **`global_stats` request**
   - Status: done
   - Files: `protocol/mod.rs`, `server/requests.rs`, `server/tests.rs`,
     `spec/architecture.md`
   - Result: protocol 7; the answer with the cache
   - Tests: server and cache tests
4. **Tabs in the status box**
   - Status: done
   - Files: `tui/status.rs`, `tui/view/status_box.rs`, `tui/app.rs`,
     `text/global.rs`, `tui/commands.rs`, `tui/tests.rs`, `spec/tui.md`
   - Result: `/stats`, the Global tab with Totals and Models, ranges
   - Tests: App and snapshot tests
5. **Tokens per day chart**
   - Status: done
   - Files: `tui/view/day_chart.rs`, `tui/view/status_box.rs`
   - Result: the chart for all three ranges
   - Tests: chart snapshots
6. **`clai chat`**
   - Status: done
   - Files: `command/chat/talk.rs`, `spec/command.md`
   - Result: `/stats [RANGE]` as text
   - Tests: text output
7. **Docs**
   - Status: done
   - Files: `readme.md`, `todo.md`, `implementation.md`, this RFC
   - Result: status `done`

## Alternatives

- **A separate box for `/stats`:** simpler state, but two boxes that look
  alike and one more thing to learn. Tabs keep both in one place, as asked.
- **A ledger file of daily totals, written by the server:** fast, and it would
  keep the tokens of deleted sessions. But it can drift from the logs, and it
  is a new file format. The cache gives most of the speed without it.
- **Counting in the client from `session_status` per session:** no new
  request, but one request per session and a slow tab with many sessions.
- **ratatui's `BarChart`:** less code, but one series per bar, no stacking and
  no y axis.
- **Days in UTC:** no offset in the request, but a late evening turn would
  land on the next day.

## Decisions

Built with the proposals of the open questions; the user asked for the RFC to
be built as it stands. Where the build differs from the design above:

- **Tab names:** the first tab is named by the session it shows (`project`),
  as the title did before; the second is **Global**. The keys line says
  `Tab global` and `Tab session`.
- **Colours:** there is no `assistant` style; output tokens use `accent`.
- **Ranges in the title:** a box too narrow for both titles names only the
  range in use, after the tabs.
- **Asking:** a box opened by `/stats` asks for the session's status only when
  its tab is first shown. A first global answer that fails shows the error in
  the tab, or closes the box when nothing else is shown.
- **Columns:** at most 8 cells and a gap per day. The y axis top is 1, 2, 4, 5
  or 8 times a power of ten, so its half is short too. With more months than
  columns, the oldest months are left out.
- **Cache:** per-session counts by quarter hour in UTC (every time zone is a
  whole number of quarter hours), keyed by the size and the modification time
  of `events.jsonl`. Counts of sessions that are gone are dropped.
- **Sessions:** "of N" counts all saved sessions, skipped logs included.
- **Model in `usage`:** the agent reads the model and its name at once at the
  start of each request, so a change during a request is counted right.

## Open questions

Answered with the proposals when it was built (see "Decisions"); the first
stays open for a later RFC.

1. **Deleted sessions.** Should their tokens stay in the global numbers? This
   needs the ledger above (a small `usage.jsonl` with one line per session and
   day, written on delete). Proposal: not in this RFC.
2. **Default range** for `/stats`: 7 days (proposal), 30 days, or the last one
   used, saved in `[tui]`?
3. **Chart series:** stacked input and output (proposal), or one bar of the
   total per day, or one colour per model?
4. **Live update:** no polling and `R` to refresh (proposal), or refresh
   every minute while the tab is open?
5. **Model in `usage`:** add it (proposal), or derive it only from
   `model_changed`?
