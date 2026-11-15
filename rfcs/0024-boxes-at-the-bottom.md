# Boxes at the bottom, settings on the whole screen

- RFC: 0024
- Status: done
- Milestone: none; it changes the layout of RFC 0001 and follows RFC 0022,
  "Questions at the bottom"
- Spec: [`spec/tui.md`](../spec/tui.md), "Layout", "Sessions", "Model
  picker", "Skills", "Status box", "Settings", "Theme list", "Language
  servers", "Details view", "Input editor"

## Summary

The boxes that commands open (the model picker, the session list,
`/skills`, `/status`, `/stats`, `/lsp`, `/theme`, help, the delete
confirmation) open at the bottom of the screen, in the place of the input,
as a question from the model does since RFC 0022. They take the full width,
cover the input while they are open, and the conversation stays visible
above them. The settings box of `/settings` takes the whole screen instead.

## Motivation

- Every box opens in the middle of the screen today (`popup::open`). It
  hides the part of the conversation the user was reading, while the input
  below it stays visible but takes no keys. That looks as if typing went
  into the input.
- The question from the model and the permission prompt already open at
  the bottom (RFC 0022). Boxes in the middle and questions at the bottom
  are two places for the same kind of thing; the eyes of the user are at
  the bottom, where they type the command.
- Boxes have a fixed width of about 76 columns. On a wide terminal long
  names, paths and descriptions are cut although there is room; on a
  narrow one the box and the screen edge compete.
- The settings box has many rows and a help line per setting. In the
  middle of the screen it is small, scrolls early, and its preview of a
  theme or symbols is hard to judge with a box over the conversation; on
  the whole screen there is room for all settings and their help.

## Scope

In:

- Where each box is drawn, its width and height, and what happens to the
  input, the activity line and the conversation while it is open.
- The settings box on the whole screen.
- The frame: rules with a title instead of a border with sides.

Out:

- What the boxes show, their keys and their requests: unchanged.
- The command list and the value lists of `/model `, `/skills ` and the
  like: they already lie directly above the input and stay there.
- The question from the model and the permission prompt (RFC 0022): they
  stay as they are and set the pattern.
- `codeagent chat`: it has no boxes.

## Design

### Which boxes

| Box | Opened by | Place |
|-----|-----------|-------|
| Model picker | `/model`, `Ctrl-O`, `/new` | bottom |
| Session list | `/sessions`, `Ctrl-S`, `/attach` | bottom |
| Skills | `/skills` | bottom |
| Status box (both tabs) | `/status`, `/stats` | bottom **(proposal)** |
| Language servers | `/lsp` | bottom |
| Theme list | `/theme` | bottom |
| Help | `/help` | bottom |
| Delete confirmation | `/kill`, `/delete` | bottom |
| Details view | `o`, `/diff`, double click | whole screen, as it already is (see "Answers") |
| Settings | `/settings` | whole screen |

### At the bottom

```
● The parser drops the last line; the fix is in src/parse.rs.

▸ edit src/parse.rs (line 12)
── Model for "project" ────────────────────────────────────────────────────────
Filter: _

anthropic
▶ sonnet  claude-sonnet-5-5  200k  ✓ current, default
  opus    claude-opus-5-5    200k
local
  qwen    qwen3-coder:30b     32k  ⚠ no key

[ ] Also make it the default for new sessions

Enter select ─ Space default ─ Esc cancel
──────────────────────────────────────────────────────────────────────────────
  idle · /help · Ctrl-D detach
  project · ~/ws/project · anthropic/sonnet · 1/3 · 12.3k/200k
```

- A box takes the rows of the input and of the rules around it, directly
  above the status row, like a question from the model: a rule with the
  title on top (`── Model for "project" ──`), a rule below, no side
  borders, the **full width** of the terminal. A title on the right (the
  tabs of the status box) goes at the right end of the top rule.
- **The input is covered** while a box is open: it is not drawn and takes
  no keys. Its text, cursor and history position are kept and come back
  when the box closes. This is the rule of RFC 0022 for questions.
- **Height:** as many rows as the box needs, up to the screen height minus
  the status and session rows and minus 3 rows of conversation, so the
  last lines of the conversation stay in sight. **(proposal)** A box that
  needs more rows scrolls inside its area, as each box does now; its keys
  line stays its last row.
- **The conversation** shrinks to the rows above the box and keeps its
  place: when it followed the end, it still does; when the user had
  scrolled up, the same line stays at the top. The wheel over the
  conversation scrolls it; the wheel over the box scrolls the box, where
  the box scrolls (session list, status box, details).
- **The activity line** is hidden while a box is open, as for a question;
  the status row still says `working` and counts the time.
- **Width:** the full width. Columns that are padded today (names, ids,
  states) keep their padding; the last column (description, path, marks)
  gets the rest and is cut with `…` only at the screen edge.
- A box over a question (`Ctrl-S`, the model picker from the session
  list): it takes the question's place while it is open, and the question
  comes back when it closes. One thing at the bottom at a time.
  **(proposal)**
- A question that opens closes the boxes, as now; the box does not come
  back.
- Below 10 rows the "terminal too small" rule applies as before; at the
  smallest size a box gets all rows but the status and session rows.

### Settings on the whole screen

```
── Settings ─ ~/.config/codeagent/config.toml ──────────────────────────────────────
▶ theme              dark
  symbols            auto            ⚠ the terminal is not UTF-8
  mouse              on
  tool_preview       errors
  tool_output_lines  20
  clipboard          auto
  syntax_highlight   on
  timestamps         off
  scroll_lines       3
  max_fps            30
  activity           on

The colour theme of the TUI. Left / Right: previous / next; t: the theme
list with a preview.

Up Down choose ─ Left Right change ─ Enter save ─ Esc cancel
───────────────────────────────────────────────────────────────────────────────
  idle · /help · Ctrl-D detach
  project · ~/ws/project · anthropic/sonnet · 1/3 · 12.3k/200k
```

- `/settings` takes every row of the screen but the status and session
  rows: the conversation and the input are not drawn while it is open.
  Their state is kept as for a box at the bottom.
- The frame is the same as at the bottom: a rule with the title, here also
  the user file it saves to, and a rule above the status row.
- All settings show at once on a screen of usual height; the help of the
  selected setting has room for several lines, word wrapped.
- **The preview:** a change of the theme or the symbols applies at once to
  the settings screen itself, its rules and the status rows, as it does to
  the box now. The conversation is not seen while the settings are open.
  **(proposal)** See "Open questions".

### Drawing

- `popup::open` and `popup::open_titled` are replaced by one function
  that takes the area at the bottom from the layout (`view::areas`) and
  draws the two rules with the title: `view::panel`. **(proposal)**
- `Areas` gains `panel: Option<Rect>`; `areas` gives the box its rows the
  way it gives a question its rows now, and the input gets no rows.
  The settings screen gets all rows above the footer.
- Each box's draw function asks for the rows it wants (as now with
  `wanted`) and draws into the area it gets; its width is the area's.
- The question of RFC 0022 and the boxes share this code; the question
  keeps its title and keys line.

## Spec changes

Written into the spec when the RFC is accepted:

- `spec/tui.md`, "Layout": the bottom area for boxes and questions; the
  input covered; the settings screen.
- Each box section ("Sessions", "Model picker", "Skills", "Status box",
  "Theme list", "Language servers", "Details view"): its sketch at the
  bottom, the full width.
- "Settings": the whole screen.
- "Input editor": the input covered while a box is open, kept.

## Testing

- `src/tui/tests.rs`: for each box, the input is not drawn and keeps its
  text and cursor after the box closes; the box takes the full width; the
  conversation keeps 3 rows above a tall box; the activity line is hidden;
  a box opened over a question takes its place and gives it back.
- Settings: the whole screen; the conversation and the input are not
  drawn; the text of the input is there after `Esc`.
- Snapshots: every box at 80 x 24 and at 120 x 30 (the full width), the
  settings screen, the smallest screen (40 x 10), ASCII mode.
- The existing snapshots of the boxes change; each is checked by eye.

## Implementation plan

1. **The bottom area**
   - Status: done
   - Files: `src/tui/view/mod.rs`, `src/tui/view/popup.rs`,
     `src/tui/view/ask.rs`, `src/tui/app.rs`
   - Result: `Areas::panel`, `view::panel` with the rules and the title;
     the question of RFC 0022 drawn with it.
   - Tests: the layout tests; the question snapshots unchanged.

2. **The boxes at the bottom**
   - Status: done
   - Files: `src/tui/view/picker.rs`, `sessions.rs`, `skills.rs`,
     `status_box.rs`, `lsp.rs`, `themes.rs`, `popup.rs` (help, delete),
     `details.rs`, `src/tui/app.rs` (wheel over the box)
   - Result: every box opens at the bottom, full width, over the input.
   - Tests: TUI tests and the box snapshots.

3. **Settings on the whole screen**
   - Status: done
   - Files: `src/tui/view/settings.rs`, `src/tui/view/mod.rs`
   - Result: `/settings` takes the screen; the help wraps.
   - Tests: TUI tests and snapshots.

4. **Spec and readme**
   - Status: done
   - Files: `spec/tui.md`, `readme.md`, `todo.md`
   - Result: the spec and readme describe the new places.
   - Tests: `ci/check.sh`, `ci/test.sh`.

## Alternatives

- **Keep the boxes in the middle and only hide the input.** Smaller
  change, but the box still covers the conversation the user reads, and
  boxes and questions stay in two places.
- **All boxes on the whole screen.** Most room, but the user loses the
  conversation also for a short choice such as a model or a theme, which
  is usually made with the conversation in mind.
- **A side panel.** Keeps the input, but needs a wide terminal and moves
  the eyes away from the input; at 80 columns it leaves too little for
  either side.
- **The box above the input, the input still visible.** The input takes
  no keys while a box is open, so showing it invites typing into it; and
  the box gets fewer rows.

## Open questions

All answered; see "Answers".

1. The details view (`o`, `/diff`) shows a whole tool result or diff:
   at the bottom like the other boxes (proposal), or on the whole screen
   like the settings, since it is a reader?
2. The status box has many rows: at the bottom (proposal), or on the
   whole screen too?
3. Settings: should the conversation stay visible behind or beside the
   settings, so the preview of a theme shows on real content, or is the
   preview on the settings screen itself enough (proposal)?
4. Height of a box at the bottom: 3 rows of conversation kept above it
   (proposal, as for questions), or a fixed share such as half the
   screen?
5. Help (`/help`) is long: at the bottom and scrolling (proposal), or on
   the whole screen?

## Answers

1. The details view stays on the whole screen. It already takes the
   whole screen (`view::details`), which the draft did not take into
   account; at the bottom it would get fewer rows, though it is the view
   for reading a whole result. Like the settings, it is a full-screen view.
2. to 5. The user asked to build it, so the proposals hold: the status
   box at the bottom; the preview of a theme on the settings screen
   itself; 3 rows of conversation kept above a box; help at the bottom.

## Notes from building

- **Layout:** `view::areas` gives the box that takes the keys
  (`view::panel`: status, lsp, themes, picker, skills, sessions, dialog,
  in the order `App` routes keys) its rows as `Areas::panel`, the way it
  gave a question its rows; `Areas::screen` holds the settings. Each box
  module has `wanted(app, width)` for its rows, and its `draw` takes the
  area; `view::open_panel` clears it and draws the two rules with the
  title, the right title too. `popup::open` and `open_titled` are gone.
- **A minimum height:** on a small screen a box gets at least 8 rows (or
  what it needs) before rows are kept for the conversation; with 3 rows
  kept, a 12-row screen left the picker 6 rows, too few for its filter,
  the selected model and the keys.
- **The wheel:** over the conversation it scrolls the conversation, as
  before; over the status box it scrolls the box. The lists (sessions,
  models, skills, themes) move with their selection, not the wheel.
- **Help does not scroll:** a help taller than the box is cut at the
  bottom, as it was cut by the screen before. Scrolling it needs state the
  dialog does not have; it is in `todo.md`.
- **One box at a time:** a box under another (the session list under the
  picker opened from it) is not drawn while the other is open.
- Snapshots: all box snapshots changed, each checked by eye; a narrow
  session list now shows all its rows.
