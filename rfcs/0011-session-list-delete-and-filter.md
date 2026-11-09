# Session list: delete, marks and filter

- RFC: 0011
- Status: draft
- Milestone: none; it improves the session list of RFC 0005
- Spec: [`spec/tui.md`](../spec/tui.md), "Sessions", "Command list";
  [`spec/architecture.md`](../spec/architecture.md), "Protocol"

## Summary

The session list (`Ctrl-S`, `/sessions`) can already rename (`r`) and
kill (`k`) one session. This RFC makes deleting clear and makes it work
for many sessions at once:

1. **Delete, named as such.** `d` and `Del` delete; `k` stays as an
   alias. The confirmation says what is lost: a running turn, an open
   question, the number of turns and tokens. `/delete` joins `/kill` in
   the command list.
2. **Marks.** `Space` marks a session, `a` marks all shown, `d` deletes
   the marked ones together after one confirmation.
3. **Filter.** `/` filters the list by name, workspace, model and state;
   a word like `>7d` keeps sessions not used for more than seven days. So
   "delete all saved sessions older than a week" is `/ saved >7d`,
   `Enter`, `a`, `d`, `y`.

The age column shows the time since the session was last used, not since
it was created. There is no undo: the confirmation is the safety net, and
files are removed at once, as today.

## Motivation

- **"Kill" hides what happens.** For a saved session nothing is running,
  yet `k kill` deletes its files. The confirmation
  `Kill docs and delete its files?` names neither the running turn that
  stops nor the open question that closes, nor how much work is lost.
- **One at a time.** Sessions pile up: every `clai new`, every `n` in the
  list. Cleaning up ten old ones is ten rounds of select, `k`, `y`.
  `clai kill --all` is all or nothing.
- **No way to find a session.** With twenty sessions the list scrolls,
  and the only way to reach one is `Up` / `Down`.
- **The age is the wrong one for cleaning up.** `AGE` is the time since
  creation. A session made a month ago and used today is not old; one
  made yesterday and left is. The server knows the time of the last change
  (`Meta::updated`), but `SessionInfo` does not carry it.

## Scope

In:

- New keys and modes of the session list: delete, marks, filter.
- The confirmation text, for one session and for several.
- `SessionInfo` gains `updated` and `turns`.
- The age column of the list shows the time since `updated`.
- `/delete` as an alias of `/kill` in the command list.

Out:

- Undo or a trash for deleted sessions (decided: no undo).
- Changes to rename (`r` stays as it is).
- Sorting the list by other columns; the order stays the server's, which
  is also the order of `Ctrl-N` / `Ctrl-P`.
- New server requests: deleting several sessions sends one `kill` per
  session, as `clai kill NAME...` does.
- `clai ls` and `clai ls --json`: they keep their columns and their
  `AGE` from creation (see open question 3).

## Design

### Keys

| Key in list | Action |
|-------------|--------|
| `Up` / `Down` | Select a session among those shown; they stop at the ends. |
| `Enter` | Attach to the selected session, as today. |
| `Space` | Mark or unmark the selected session, then select the next one. |
| `a` | Mark every session shown; if all shown are marked already, unmark them. |
| `d`, `Del`, `k` | Delete: the marked sessions if any are marked, else the selected one. Asks first. |
| `/` | Filter: edit the filter text in the step line. |
| `n`, `r` | New session, rename, as today. |
| `Esc` | Clear the marks if any; else clear the filter if set; else close the list, as today. |

`k` is kept so that fingers used to it still work; the key hint shows
`d delete`. **(proposal)**

Marks belong to sessions, by id, not to rows: they survive the refresh
every 2 s and a change of the filter. A marked session that the server no
longer lists loses its mark. Marks end when the list closes.

### Rows

```
┌ Sessions ────────────────────────────────────────────────────────────────────┐
│       NAME       WORKSPACE     MODEL             STATE      USED             │
│     ✓ project    ~/ws/project  anthropic/sonnet  working    now              │
│ ▶ ■   old        ~/ws/old      anthropic/sonnet  saved      12d              │
│   ■   older      ~/ws/old      local/qwen        saved      30d              │
│                                                                              │
│ filter: saved >7d ─ 2 of 9 ─ 2 marked                                        │
│ Enter attach ─ Space mark ─ a all ─ d delete ─ / filter ─ Esc clear          │
└──────────────────────────────────────────────────────────────────────────────┘
```

- Before the name: the pointer, the mark (`■`, ASCII `#`), the attached
  session's `✓`. **(proposal: the symbol; `●` and `*` are taken by
  "waiting")** The new symbol joins `Symbols` as `marked`.
- The column is `USED`: the time since `updated`, in the same short form
  as today's age (`now`, `12m`, `3d`). **(proposal: the heading)**
- The step line, when nothing is asked, shows the filter, how many
  sessions are shown of how many, and how many are marked, each only when
  it applies. A message (a refused name, a failed delete) takes its place
  as today.

### Filter

`/` opens the filter in the step line, starting with the current text:
`Filter: saved >7d`. Every key edits it, and the list follows each key.
`Enter` or `Down` goes back to the list with the filter kept; `Esc`
empties it and goes back.

The text is split into words; a session is shown if every word matches
**(proposal)**:

| Word | Matches |
|------|---------|
| `>7d`, `>12h`, `>30m` | Not used for more than that: `now - updated` is larger. |
| `<1d` and the like | Used within that time. |
| Anything else | A part of the name, the workspace as shown, the model or the state (`idle`, `working`, `waiting`, `saved`), ignoring case. |

A word that looks like a time but does not parse (`>7x`) matches as text,
so it hides everything and the user sees the filter is wrong. The
attached session is shown only if it matches, like any other.

If the selected session is filtered out, the first one shown is
selected. With nothing shown, the list says `no session matches` and
`d`, `Enter`, `r` do nothing.

### Delete

One session:

```
Delete old? 14 turns, 210k tokens. [y] yes  [n] no
Delete project? It is working: the turn is cancelled. 3 turns, 41k tokens. [y] yes  [n] no
Delete docs? Its question is closed unanswered. 8 turns, 96k tokens. [y] yes  [n] no
```

Several:

```
Delete 5 sessions? 1 is working, 1 waits for an answer. [y] yes  [n] no
```

- `y` sends one `kill` per session, in list order, and clears the marks.
  `n` or `Esc` goes back with the marks kept.
- The answers come back one by one. When all are there, the step line
  says `deleted 5 sessions`, or for a failure
  `deleted 4 sessions; docs: no session named docs`. A session renamed by
  another client while the question was open fails this way; the user
  sees it and can try again. **(proposal: names, not ids, since `kill`
  takes a name; no protocol change)**
- Deleting the attached session, alone or among others, works as today:
  the TUI shows a notice and the list stays open. With every session
  gone, the list says `no sessions` and offers `n` new and `Esc` exit.
- The text uses `turns` and `usage` of `SessionInfo`; the tokens are
  input plus output, in the short form of the token counters (`41k`).

### Command list

`/delete` is an alias of `/kill`: the same question, now in the delete
wording above, for the attached session. The command list shows `/kill`
with its alias, as it shows `/detach, /quit`: `/kill, /delete`.
**(proposal)**

### Protocol

`SessionInfo` gains two fields:

```rust
/// Seconds since 1970 at the last change.
#[serde(default)]
pub updated: u64,
/// Turns started so far.
#[serde(default)]
pub turns: u64,
```

`Meta::info` fills them from `Meta`. Both have defaults and
`SessionInfo` does not deny unknown fields, so old and new clients and
servers read each other's messages, and the protocol version stays `5`.
With an old server, `updated` is `0`: the list falls back to `created`
for `USED` and for time words.

### Code

- `tui/sessions.rs`: `marked: BTreeSet<u64>`, `filter: String`;
  `Mode::Filter(Editor)` and `Mode::ConfirmDelete(Vec<String>)` (the
  latter replacing `ConfirmKill`); `shown()` returns the indices that pass
  the filter, and selection moves among them. `ListAction::Kill(String)`
  becomes `ListAction::Delete(Vec<String>)`.
- `tui/sessions/filter.rs` (new, pure): parsing the words and matching a
  `SessionInfo` at a time `now`. Unit tests only.
- `text::sessions`: `confirm_delete(&[&SessionInfo]) -> String` for the
  texts above, next to `age`, so the TUI and a later `clai chat` share it.
- `tui/app.rs`: one `Request::Kill` effect per name; a counter of open
  deletes so the result line is written once all answers are there.
- `tui/view/sessions.rs`: the mark column, the `USED` column, the step
  line with filter and counts, the new key hints.
- `tui/commands.rs`: the `delete` alias.

## Spec changes

- `spec/tui.md`, "Sessions": the picture, the keys table, marks, the
  filter, the delete texts, and the `USED` column.
- `spec/tui.md`, "Command list": `/kill` with alias `/delete`.
- `spec/architecture.md`, "Protocol": `SessionInfo` has `updated` and
  `turns`.

## Testing

- `tui/sessions/filter.rs`: words with and without times, `>`, `<`,
  units, case, a bad time word, every word must match.
- `tui/sessions.rs`:
  - `Space` marks and moves on; `a` marks all shown and then unmarks;
    marks survive `set_sessions` and drop for sessions gone.
  - The filter hides rows, selection moves to the first shown, `Esc`
    clears marks before the filter before closing.
  - `d` with marks gives `Delete` with the marked names in list order;
    without marks, the selected name; `k` and `Del` the same; nothing
    with no session shown.
- `text::sessions`: each delete text, one and several, working and
  waiting.
- `tui/tests.rs` snapshots: the list with marks and a filter; the delete
  question for one and for several; the result line after a failure.
- `tui/tests.rs` app tests: confirming sends one `kill` per name; the
  result line waits for all answers; deleting the attached session among
  others keeps the list open.
- `protocol/tests.rs`: `SessionInfo` without `updated` and `turns` still
  reads (an old server).
- `src/server/tests.rs`: a `list` answer carries `updated` and `turns`,
  and `updated` moves after a turn.

## Implementation plan

Each step leaves a working, tested state.

1. **`updated` and `turns` in `SessionInfo`**
   - Status: todo
   - Files: `src/protocol/mod.rs`, `src/session/store.rs`,
     `src/protocol/tests.rs`, `src/server/tests.rs`, fixtures that build
     `SessionInfo`
   - Result: the server sends both; old messages still read.
   - Tests: the protocol and server tests.

2. **Delete wording and keys**
   - Status: todo
   - Files: `src/text/sessions.rs`, `src/tui/sessions.rs`,
     `src/tui/app.rs`, `src/tui/view/sessions.rs`, `src/tui/commands.rs`,
     `src/tui/tests.rs`, snapshots
   - Result: `d`, `Del` and `k` delete one session with the new
     question; `/delete` works; the `USED` column.
   - Tests: the text tests, key tests, snapshots.

3. **Marks**
   - Status: todo
   - Files: `src/tui/sessions.rs`, `src/tui/app.rs`,
     `src/tui/view/sessions.rs`, `src/tui/style.rs` (`marked`),
     `src/tui/tests.rs`, snapshots
   - Result: `Space` and `a` mark, `d` deletes the marked ones with one
     question, the result line.
   - Tests: the mark and app tests, snapshots.

4. **Filter**
   - Status: todo
   - Files: `src/tui/sessions/filter.rs` (new), `src/tui/sessions.rs`,
     `src/tui/view/sessions.rs`, `src/tui/tests.rs`, snapshots
   - Result: `/` filters by text and time; `a` marks what is shown.
   - Tests: the filter unit tests, list tests, a snapshot.

5. **Spec**
   - Status: todo
   - Files: `spec/tui.md`, `spec/architecture.md`, `todo.md`
   - Result: the spec says what the list does.
   - Tests: `cargo fmt`, `clippy` and `test` clean.

## Alternatives

- **A trash with undo.** Safer, but needs a new request, a place for the
  files and a rule for when they go. Decided against: the confirmation
  names what is lost.
- **A "delete old sessions" command with a day count.** One more dialog
  for one case. The filter with `>7d` plus `a` does the same and also
  serves for finding a session.
- **A `kill` request with several names.** Would make the result one
  answer, but changes the protocol for something one request per session
  does well enough.
- **Filter only by name.** Simpler, but the cleaning-up case needs state
  and time, and the workspace is the second way people name sessions.
- **Keep `AGE` from creation.** Matches `clai ls`, but is the wrong time
  for deciding what is old; see open question 3.

## Open questions

1. Keep `k` as a hidden alias of `d`, or drop it?
2. The mark symbol: `■` / `#`, or another?
3. Should `clai ls` also switch its `AGE` column to the time since last
   use (and be called `USED`), so both lists agree?
4. Should `clai kill` gain a `delete` alias, to match `/delete`?
