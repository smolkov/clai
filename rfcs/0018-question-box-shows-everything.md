# Question box shows everything

- RFC: 0018
- Status: draft
- Milestone: none; it improves the box of RFC 0012
- Spec: [`spec/tui.md`](../spec/tui.md), "Questions from the model";
  [`rfcs/0012-questions-from-the-model.md`](0012-questions-from-the-model.md),
  "TUI"

## Summary

The box of a question from the model (`ask_user`) often hides part of
what the user needs to answer: the model's text and `context` above the
question, options below the screen, the keys line, most of a typed
answer. This RFC makes everything reachable and makes it visible when
something is hidden. The box grows with the terminal; the keys line and
messages stay fixed at the bottom; the rest scrolls with the wheel and
keys, with marks that say how much is above and below; moving between
options scrolls only as far as needed; a typed answer shows all its
rows; the review page repeats the questions; and `v` opens the whole
question in the details view.

## Motivation

The box is built in `src/tui/view/ask.rs`. On a real question it fails
in these ways:

- **Fixed width.** The box is 78 cells wide on any terminal. A model's
  `text` before the call and a `context` of up to 4 000 characters wrap
  into many rows, even on a 200 cell terminal with room to spare.
- **The top is scrolled away.** With `top == 0` the view puts the
  selected option on the last row (`(selected + 1) - room`). When the box
  is taller than the screen, the box opens with the context and the
  question itself scrolled out, so the user sees options without knowing
  what they answer.
- **`PgUp` cannot reach the top.** `top == 0` means "follow the
  selection", so scrolling up to the first row snaps back to the
  selected option. The context cannot be read once the box is taller
  than the screen.
- **`PgDn` overshoots.** `top += 10` is not limited; it is clamped only
  when drawn. After a few `PgDn` the user needs as many `PgUp` before
  the box moves again.
- **`Up` / `Down` reset the scroll.** Each move sets `top = 0`, so
  reading the context and choosing an option fight each other.
- **The keys line scrolls out.** It is the last row of the body; when
  the box is taller than the screen the user does not see which keys
  answer, skip or cancel. The same holds for the `[y] yes  [n] no`
  question of `Esc` and the `not accepted: …` line.
- **No sign of hidden rows.** Nothing says that there is more above or
  below, or that `PgUp` / `PgDn` scroll.
- **The mouse wheel scrolls the conversation behind the box**, which
  cannot be seen, instead of the box.
- **A typed answer shows one row.** The editor wraps the text, but only
  the cursor's row is drawn. The user cannot read back an answer of more
  than one row before sending it.
- **The review page shows headers only.** `1 Database  SQLite` is all
  that is left of the question; with similar headers the user cannot
  check what they answer.

## Scope

In:

- Size of the box, the fixed footer, scrolling, scroll marks.
- Keys and the mouse wheel for scrolling.
- The typed answer drawn in full, up to a limit.
- The review page with the question texts.
- `v`: the whole question in the details view.
- The `ask_user` part of `spec/tui.md`.

Out:

- The tool, its limits, the question kind and its answers (RFC 0012).
  Nothing changes in the protocol.
- `clai chat` and `clai run`: the text dialog prints everything already.
- Other boxes (status, settings, help). They may adopt the same footer
  and marks later.
- Keeping what was chosen across a session switch (already in
  `todo.md`).

## Design

### Size

**(proposal)** The box is as wide as the screen minus 4 cells, at most
100 cells of text, at least what the screen has. It is as tall as its
rows, at most the screen's height. Below 40 cells of width or 10 rows of
height it still opens, with the same rules; nothing is dropped, only
scrolled.

`WIDTH` in `src/tui/view/ask.rs` becomes `MAX_WIDTH = 100`. The
conversation's text width is not used: the box may be wider than the
conversation's wrap width when the screen allows.

### Parts of the box

The box has three parts, from top to bottom:

| Part | What | Scrolls |
|------|------|---------|
| Title | `Question ─ 1/2 Database`, and on the right the scroll marks. | No |
| Body | `not accepted: …`, the model's `text`, `context`, the question, the options, the text row. | Yes |
| Footer | One rule, then the keys line, or the `Esc` question. | No |

The footer is never scrolled out. The `not accepted: …` line stays at the
top of the body, but the view scrolls to it when it appears.

```
┌ Question ─ 1/2 Database ──────────────────────────────────── ↑ 4 ↓ 0 ┐
│ …second half of the context, scrolled.                               │
│                                                                      │
│ Which database should the cache use?                                 │
│                                                                      │
│ ▶ 1 SQLite (recommended)                                             │
│       One file next to the binary, no server.                        │
│   2 Postgres                                                         │
│       Shared with the API; needs the server running.                 │
│   3 Type an answer                                                   │
│ ──────────────────────────────────────────────────────────────────── │
│ Up Down choose ─ Enter next ─ Tab question ─ PgUp more ─ v all ─ Esc │
└──────────────────────────────────────────────────────────────────────┘
```

### Scroll marks

When the body does not fit, the title's right side shows `↑ N` and `↓ M`:
the rows above and below the view. `↑` and `↓` come from `tui.symbols`;
the ASCII set uses `^` and `v`. When everything fits, nothing is shown.
The keys line then also names the scroll key (`PgUp more` or `PgDn
more`), and drops keys from the right when it is too long, keeping
`Esc` and `Ctrl-C` last to go. **(proposal)**

### Where the view starts and how it moves

`Ask::top` becomes a real first row, no longer "0 means follow".
**(proposal)**

- When the box opens, and when the page changes (`Tab`, `Shift-Tab`,
  `Enter` to the next question), the view starts at the first row, so
  the context and the question are read first. If then the selected
  option is not in view, the view starts at the question's first row
  instead, and `↑` shows that context is above.
- `Up` / `Down` and the number keys scroll only when the selected row
  (the option and its description) would leave the view, and only as
  far as needed to keep it in.
- `PgUp` / `PgDn` scroll by the body's height minus one row, clamped to
  the rows there are. They do not move the selection.
- The mouse wheel over the box scrolls it by `tui.scroll_lines`, as
  `PgUp` / `PgDn` do.
- `Ctrl-Home` / `Ctrl-End` go to the first and last row.

Clamping needs the body's height, which only the view knows. The view
writes the last height into `Ask` (a `Cell<usize>` or a field set by
`App` after drawing, as the details view does for its page), and the
keys use it. **(proposal: follow what `tui/details.rs` does)**

### Typed answer

While the text row is typed, all rows of the editor are drawn, at most 6;
a longer text scrolls inside those rows to keep the cursor in view. A
kept text (`Answer: …`, `Added: …`) is drawn in full, as now.

### Review page

Each answer gets the question's text under its header, dim, wrapped:

```
1 Database  SQLite
            Which database should the cache use?
2 Features  Search, Export; "keep the old export format too"
            Which features should the first version have?
```

### The whole question

`v` (not while typing) opens the question in the details view of
RFC 0016: the model's `text`, `context`, then every question with all
its options and descriptions, and what is chosen so far, as Markdown.
The details view brings search (`/`), wrap and copy. `Esc`, `q` or
`Ctrl-C` close it, as for tool calls, and return to the box with
everything chosen kept; `Ctrl-C` there does not cancel the turn. The
question stays open meanwhile; an answer from another client closes
both. **(proposal)**

`v` is free in the box: letters do nothing there today, and typing
starts only with `Space` or `Enter` on the text row.

### Code

- `src/tui/view/ask.rs`: width from the screen, body and footer split,
  the scroll marks in the title (`popup::open_titled` with a right
  title), editor rows, review texts.
- `src/tui/ask.rs`: `top` as a first row, clamped; keep-in-view on
  moves; `PgUp` / `PgDn` by the body's height; `Ctrl-Home` / `Ctrl-End`;
  reset on page change; `v` returns an action to open the details view.
- `src/tui/app.rs`: the wheel scrolls the box when it is open;
  `v` opens the details view with a text built from the question; the
  box is drawn again when it closes.
- `src/tui/details.rs`: a source for a question (next to tool calls),
  if the details view needs one to know what it shows.

## Spec changes

`spec/tui.md`, "Questions from the model":

- The picture with the footer and the scroll marks.
- The size rule.
- The keys table: `PgUp` / `PgDn` by a page, the wheel, `Ctrl-Home` /
  `Ctrl-End`, `v`.
- Bullets: where the view starts on a new page, that the footer always
  shows, that a typed answer shows up to 6 rows, the review page with
  the questions.

## Testing

`src/tui/tests.rs`, with the fake session and snapshots:

- A long `context` on an 80 x 20 screen: the box opens at the first row,
  `↓` shows the rows below, the footer shows; snapshot.
- `PgDn` to the end, `PgUp` back to row 0 in as many presses; `PgDn`
  past the end does nothing.
- `Down` to the last option scrolls just enough; `Up` back does not
  scroll while the row is in view.
- `Tab` to the next question starts at its first row.
- The wheel over the box scrolls the box, not the conversation.
- A wide screen (160 cells): the box is 104 cells wide; a narrow one
  (50 cells): the box fills it; ASCII marks; snapshots.
- A typed answer of three rows shows all three; of ten rows shows six
  with the cursor's row in them.
- The review page shows the question texts; snapshot.
- `v` opens the details view with all questions and options; `Esc`
  returns with the choices kept; another client's answer closes both.
- The `Esc` question and `not accepted: …` stay visible on a short
  screen.

## Implementation plan

1. **Scrolling that works**
   - Status: todo
   - Files: `src/tui/ask.rs`, `src/tui/view/ask.rs`, `src/tui/app.rs`,
     `src/tui/tests.rs`
   - Result: `top` is a real row, clamped; the view starts at the top;
     moves keep the selection in view; `PgUp` / `PgDn` by a page;
     `Ctrl-Home` / `Ctrl-End`; the wheel scrolls the box.
   - Tests: the scrolling and wheel tests.

2. **Footer, marks and size**
   - Status: todo
   - Files: `src/tui/view/ask.rs`, `src/tui/view/popup.rs` if needed,
     snapshots
   - Result: the footer stays, the marks show hidden rows, the box
     grows with the screen up to 100 cells.
   - Tests: the snapshots for short, wide, narrow and ASCII screens.

3. **Typed answer and review page**
   - Status: todo
   - Files: `src/tui/view/ask.rs`, snapshots
   - Result: all rows of a typed answer, up to 6; the review page with
     the questions.
   - Tests: the editor rows and review snapshots.

4. **The whole question in the details view**
   - Status: todo
   - Files: `src/tui/ask.rs`, `src/tui/app.rs`, `src/tui/details.rs`,
     `src/tui/tests.rs`
   - Result: `v` opens it, `Esc` returns, an answer elsewhere closes
     both.
   - Tests: the `v` tests.

5. **Spec and readme**
   - Status: todo
   - Files: `spec/tui.md`, `readme.md` if keys are listed there,
     `todo.md`
   - Result: the spec says what the box does.
   - Tests: `cargo fmt`, `clippy` and `test` clean.

## Alternatives

- **Fold the context to a few rows, `c` unfolds it.** Saves room, but
  hides the very text the model wrote to explain the question, and adds
  a state to the box. Scrolling with marks shows everything without it.
- **Keep the question and options fixed, scroll only the context.** Two
  scrolled areas in one box; on a short screen each gets only a few
  rows, and long option lists would need scrolling too.
- **Full-screen box.** Simplest, but hides the conversation entirely
  even for a one-line question. The box still fits small questions
  tightly.
- **Use the details view only.** Search and copy for free, but answering
  needs the box's keys; two views for one question is enough as an
  extra (`v`), not as the main way.

## Open questions

1. Width: at most 100 cells of text (proposal), the full screen, or the
   conversation's wrap width?
2. Where the view starts on a box taller than the screen: at the top
   with the context (proposal), or at the question with `↑` marking the
   context above?
3. The key for the whole question: `v` (proposal), `o` as in the
   conversation for tool details, or `?`.
4. Should the other boxes (status, settings, help) get the same footer
   and marks in this RFC, or later?
5. Rows for a typed answer: 6 (proposal), or more?
