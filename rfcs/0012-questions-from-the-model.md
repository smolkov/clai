# Questions from the model

- RFC: 0012
- Status: done
- Milestone: none; it adds a tool and a question kind
- Spec: [`spec/tools.md`](../spec/tools.md);
  [`spec/sessions.md`](../spec/sessions.md), "Questions";
  [`spec/tui.md`](../spec/tui.md), "Permission prompts";
  [`spec/command.md`](../spec/command.md), "clai chat", "clai run";
  [`spec/agent-loop.md`](../spec/agent-loop.md), "System prompt";
  [`spec/config.md`](../spec/config.md), "Permissions"

## Summary

A new tool, `ask_user`, lets the model ask the user one to four questions
in one call. Each question has two to six options with a short
description, may allow several options, and always lets the user type
their own answer instead. The question goes to the user like a permission
question does: as a dialog box in the TUI, as numbered lines in
`clai chat` and in `clai run` on a terminal. The answer goes back to the
model as the tool's result, and the turn goes on.

It builds on the questions that exist (`spec/sessions.md`, "Questions"):
a new kind `ask_user` next to `permission` and `limit`, with its own
choices and answer. A session waits for it like for any question, the
`●` counter counts it, and the first valid answer from any client counts.

## Motivation

Today the model has two ways to get a decision from the user, both bad:

- **End the turn with a question in its answer.** The user reads it,
  types a reply, and a new turn starts. The model loses its train of
  thought between turns only in part (history stays), but the work stops,
  the options are prose the user has to retype, and in a background
  session nobody sees that a decision is needed: the session is `idle`,
  not `waiting`.
- **Guess.** The model picks one of several reasonable designs and goes
  on. If the guess was wrong, the user finds out after the files changed.

A question with options fixes both: the turn keeps running and waits, the
session shows `waiting` with a summary, the user answers with a key, and
the model gets a precise answer it can act on.

## Scope

In:

- The `ask_user` tool: schema, checks, result text.
- The question kind `ask_user`, its choices and its answer, checked by the
  server.
- The TUI dialog box.
- The text dialog in `clai chat` and in `clai run` on a terminal.
- When the tool is offered to the model, and the `ask_user` key in
  `[permissions]` to turn it off.
- One bullet in the system prompt.
- Statistics: questions from the model are counted in `/status`.

Out:

- Questions that are not a choice (a form, a file picker, a number
  field). The typed answer covers the rest.
- Images or diffs in a question. The question may carry Markdown text.
- Asking several sessions' users at once, or a deadline for an answer.
  Questions never time out, as today.
- A web UI; the messages are designed so it needs nothing more.

## Design

### The tool

```json
{
  "questions": [
    {
      "question": "Which database should the cache use?",
      "header": "Database",
      "options": [
        {"label": "SQLite", "description": "One file next to the binary, no server."},
        {"label": "Postgres", "description": "Shared with the API; needs the server running."}
      ],
      "multiple": false
    }
  ],
  "context": "Both work with the current code; the choice decides the migration."
}
```

| Argument | Required | Meaning |
|----------|----------|---------|
| `questions` | yes | 1 to 4 questions, asked together. |
| `question` | yes | The full question, one or two sentences, ending with `?`. At most 500 characters. |
| `header` | no | A short label for tabs and lists, at most 16 characters. Default: the question, cut. |
| `options` | yes | 2 to 6 options. |
| `label` | yes | The option, 1 to 5 words, at most 60 characters. Labels of one question differ. |
| `description` | no | What choosing it means, at most 300 characters. |
| `multiple` | no | `true`: the user may choose several. Default `false`. |
| `context` | no | Markdown shown above the questions: why the model asks. At most 4 000 characters. |

A wrong call is an error result naming the problem, as for every tool
(`spec/tools.md`, "Calls and results"); no question opens. No option for
"Other" is listed: the user can always type an answer instead of, or
together with, the options. The tool description tells the model this,
and to put the option it recommends first, with `(recommended)` in the
label. **(proposal)**

The model's text before the call in the same answer is shown with the
question, as for permission questions.

### When the model gets the tool

The tool is in the request only if someone can answer **(proposal)**:

| Where | Offered |
|-------|---------|
| A session on the server | Yes: clients can attach later, the session waits. |
| `clai run` with a terminal | Yes. |
| `clai run` without a terminal, `--yes`, `--no-ask` | No. |
| `ask_user = "deny"` in `[permissions]` | No. |

`[permissions]` gains the key `ask_user`, `allow` (default) or `deny`.
`ask` is a load error ("asking to ask"); rules cannot name it. A project
file may set `deny`, since that is stricter. If the model still calls the
tool where it is not offered (history from an earlier turn), the result
is an error: `ask_user is not available: no one can answer; decide
yourself and say what you chose in your answer`.

It counts as a tool call against the turn's limit. Waiting for the answer
is not working time in `/status`.

### The question

A new kind next to `permission` and `limit`:

| Field | Meaning |
|-------|---------|
| `kind` | `ask_user`. |
| `summary` | `ask_user: ` and the headers joined, or the first question cut to one line. |
| `call` | Tool name, the call's id, the arguments exactly as sent. |
| `questions` | The checked questions: `question`, `header`, `options` (`label`, `description`), `multiple`. |
| `context` | The model's `context`, if any. |
| `text` | The model's text before the call, if any. |
| `choices` | `reply`, `decline`, `cancel`. |

### Answers

| Choice | Effect |
|--------|--------|
| `reply` | The answer names, for each question in order, the chosen options by index and an optional typed text. |
| `decline` | The user does not answer; the model is told so and the turn goes on. |
| `cancel` | Cancel the turn, as for the other kinds. |

```json
{"choice": "reply", "answers": [{"selected": [0], "text": null}]}
```

The server checks a `reply` and rejects it, with the question staying
open, if:

- the number of answers differs from the number of questions;
- an index is out of range or repeated;
- a question without `multiple` has more than one index;
- an answer has neither an index nor a non-empty text;
- a text is longer than 4 000 characters.

### What the model gets

A `reply` is a normal (not error) result, one block per question:

```
The user answered:

1. Which database should the cache use?
   SQLite

2. Which features should the first version have?
   Search, Export
   The user added: keep the old export format too.
```

A text without options reads `The user wrote: …`. `decline` gives
`the user declined to answer; decide yourself, or ask in your final
answer`, also not an error. **(proposal for the texts)**

### TUI

The question opens as a box over the conversation, like the status box,
because it is larger than the prompt bar. The tool call line in the
conversation shows `? ask_user Database`, and the session row counts the
question in `●`.

```
┌ Question ─ 1/2 Database ─────────────────────────────────────────────────────┐
│ Both work with the current code; the choice decides the migration.           │
│                                                                              │
│ Which database should the cache use?                                         │
│                                                                              │
│ ▶ 1 SQLite (recommended)                                                     │
│       One file next to the binary, no server.                                │
│   2 Postgres                                                                 │
│       Shared with the API; needs the server running.                         │
│   3 Type an answer                                                           │
│                                                                              │
│ Up Down choose ─ Enter next ─ Tab question ─ Esc skip ─ Ctrl-C cancel turn   │
└──────────────────────────────────────────────────────────────────────────────┘
```

| Key | Action |
|-----|--------|
| `Up` / `Down`, `1`–`9` | Select an option or the "type an answer" row. |
| `Space` | With `multiple`: tick or untick the option. Without: select it. |
| `Enter` | On an option: take it (without `multiple`) and go to the next question. On the text row: edit the text in the row; `Enter` again keeps it. |
| `Tab` / `Shift-Tab` | Next or previous question, keeping what was chosen. |
| `PgUp` / `PgDn` | Scroll the box when it is taller than the screen. |
| `Esc` | Skip: answers `decline`, after `y`/`n` if anything was chosen. |
| `Ctrl-C` | Cancel the turn (`cancel`). |

- With `multiple`, ticked options show `[x]`, others `[ ]` (ASCII the
  same); `Enter` goes on with the ticked ones.
- With one question and no `multiple`, `Enter` on an option sends at once.
  Otherwise, after the last question a review page lists each answer;
  `Enter` sends, `Tab` goes back to a question.
- A text can be added to options: on the text row of a question that
  already has a choice, the text is sent with it ("The user added").
- `context` and `text` are drawn as Markdown, like the model's answers.
- While the box is open the keys belong to it, as for the prompt bar. The
  keys that work during a permission prompt without answering it
  (`Ctrl-N` / `Ctrl-P`, `Ctrl-S`, `Ctrl-D`) work here too; the question
  stays open, and what was chosen is kept for this TUI until it is sent.
- When another client answers, the box closes and the conversation shows
  who answered and what, as for other questions.

### `clai chat` and `clai run`

The text dialog (`text::dialog`) gains a stage for this kind. It asks one
question after another:

```
? Question from the model (1/2): Which database should the cache use?
  Both work with the current code; the choice decides the migration.
  1) SQLite (recommended) — One file next to the binary, no server.
  2) Postgres — Shared with the API; needs the server running.
Answer: a number, or text of your own; "s" skips, "c" cancels the turn:
```

- With `multiple`: `1,3` or `1 3`; text after the numbers, after a `:`,
  is added (`1,3: keep the old format`).
- Anything that is not numbers is taken as a typed answer.
- After the last question the dialog prints the answers and asks
  `Send? [y/n]` when there was more than one question.
- `clai chat --questions` lists these questions with the others and
  answers them the same way.

### Code

- `src/tools/ask_user.rs` (new): the argument types, `check` (the limits
  above) and `result_text(questions, answers)`. Pure, unit tested.
- `src/agent/question.rs`: `QuestionKind::AskUser(Box<AskUser>)`,
  `Choice::Reply` and `Choice::Decline`, `Answer::Reply { answers }`,
  `UserAnswer { selected, text }`, `ASK_USER_CHOICES`.
- `src/agent/mod.rs`: in `one_call`, a call to `ask_user` checks the
  arguments, opens the question through `question` and `wait` (with the
  reply checks as `validate`), and turns the answer into the result.
  `Setup` gains whether someone can answer, so the request leaves the
  tool out when no one can.
- `src/tools/mod.rs`: the spec of `ask_user` in `specs()`, filtered by
  the agent.
- `src/config/permissions.rs`: `Tool::AskUser` with the `allow` / `deny`
  key; not in any group.
- `src/protocol/mod.rs`: version `6` (a new question kind, choices and
  answer).
- `src/text/question.rs`, `src/text/dialog.rs`: the text form.
- `src/tui/ask.rs` (new, the dialog state) and `src/tui/view/ask.rs` (new,
  the box); `App` opens it for this kind instead of the prompt bar.
- `src/protocol/stats.rs`: questions from the model counted with the
  others, by kind.

## Spec changes

- `spec/tools.md`: the tools table gains `ask_user`; a new section
  `## ask_user` with the arguments, checks, result texts and when it is
  offered.
- `spec/sessions.md`, "Questions": the kind `ask_user` in "Content", the
  choices `reply` and `decline` in "Answers", the clients table.
- `spec/tui.md`: a section "Questions from the model" after "Limit
  prompt", with the box and its keys.
- `spec/command.md`: "clai chat" and "clai run" show the text dialog.
- `spec/config.md`, "Permissions": the `ask_user` key.
- `spec/agent-loop.md`, "System prompt": the new bullet:
  - Ask the user with ask_user only for a decision that is theirs and
    that the code and the task do not settle; give two to four options,
    the one you recommend first. Do not ask what you can find out
    yourself.
- `spec/architecture.md`, "Protocol": version `6`.

## Testing

- `src/tools/ask_user.rs`: every limit of the schema; labels that repeat;
  result texts for one and several questions, with and without typed
  text, and for `decline`.
- `src/agent/tests.rs` with the fake provider:
  - a call opens an `ask_user` question with the checked questions;
  - a valid `reply` gives the result text and the turn goes on;
  - each invalid `reply` is rejected and the question stays open;
  - `decline` and `cancel`;
  - `NoOne` (no one can answer) gives the "not available" error;
  - the tool is left out of the request without an answerer and with
    `ask_user = "deny"`.
- `src/config/tests.rs`: `ask_user = "ask"` is a load error; a project
  file may set `deny` but not `allow`.
- `src/protocol/tests.rs`: the question and the answer round-trip.
- `src/text/dialog.rs`: numbers, `1,3: text`, plain text, `s`, `c`, a
  number out of range asks again.
- `src/tui/tests.rs`: snapshots of the box (one question, several with
  `multiple`, the review page, narrow, ASCII); keys: select, tick, `Tab`,
  typed text, `Enter` sends the right answer, `Esc` asks before declining
  when something was chosen, `Ctrl-C` cancels; the box closes when
  another client answers.
- `src/server/tests.rs`: a session waits with the question; `clai chat`
  style answer from a second client counts; the first answer wins.
- `tests/run.rs`: `clai run` without a terminal does not offer the tool
  (the request's `tools` has 6 entries); with an `ask_user` call anyway,
  the result is the "not available" error.

## Implementation plan

Each step leaves a working, tested state.

1. **Types, checks and texts**
   - Status: done
   - Files: `src/tools/ask_user.rs`, `src/agent/question.rs`,
     `src/protocol/mod.rs`, `src/protocol/tests.rs`
   - Result: the question kind, choices and answer exist and are checked;
     the result texts are built; protocol `6`.
   - Tests: the tool unit tests, protocol round-trips.

2. **The tool in the loop**
   - Status: done
   - Files: `src/agent/mod.rs`, `src/agent/tests.rs`, `src/tools/mod.rs`,
     `src/config/permissions.rs`, `src/config/tests.rs`,
     `src/command/run.rs`, `src/server/requests.rs`,
     `src/agent/system_prompt.md`, `src/protocol/stats.rs`
   - Result: the model can ask in a session and in `clai run` on a
     terminal; the answer comes back as the result; the tool is left out
     where no one can answer or the user turned it off.
   - Tests: the agent, config and server tests; `tests/run.rs`.

3. **Text clients**
   - Status: done
   - Files: `src/text/question.rs`, `src/text/dialog.rs`,
     `src/command/chat/questions.rs`, `src/command/run.rs`
   - Result: `clai chat` (also `--questions`) and `clai run` on a
     terminal show and answer the question.
   - Tests: the dialog tests.

4. **TUI dialog box**
   - Status: done
   - Files: `src/tui/ask.rs`, `src/tui/view/ask.rs`, `src/tui/app.rs`,
     `src/tui/view/mod.rs`, `src/tui/tests.rs`, snapshots
   - Result: the box opens for the question, takes the keys above, sends
     the answer, closes when someone else answers.
   - Tests: the TUI snapshots and key tests.

5. **Spec and readme**
   - Status: done
   - Files: the spec files above, `readme.md`, `todo.md`
   - Result: the spec says what the code does; the readme mentions that
     the model can ask.
   - Tests: `cargo fmt`, `clippy` and `test` clean.

## Alternatives

- **The model asks in its final answer.** What happens today. The turn
  ends, the session looks idle, the user retypes an option. Kept as the
  fallback where the tool is not offered.
- **Questions in the prompt bar.** Fits a yes/no, not six options with
  descriptions and several questions. The box is the status box's pattern.
- **One question per call.** Simpler, but a model that needs three
  decisions would ask three times, each time waiting. Four together is
  the same limit other agents use and fits on one screen with review.
- **An "other" option the model must list.** Models forget it or word it
  oddly. The typed answer is always there instead.
- **Offer the tool always and answer `NoOne` with an error.** The model
  would plan around a tool that never works in `clai run` scripts.
  Leaving it out is clearer; the error covers old history.
- **Make it a permission-checked tool with `ask`.** Asking permission to
  ask is two questions for one. `allow` or `deny` is all it needs.

## Open questions

1. `Esc` in the box: skip the question (`decline`, proposal), or cancel
   the turn like `Esc` in the permission prompt? Ctrl-C does the other.
2. The limits: 1–4 questions, 2–6 options, the text lengths. Enough?
3. Should a session that waits for an `ask_user` question also notify
   like a permission question (the `●` counter and the session list's
   `waiting`, as proposed), or stand out more, since only the user can
   move it on?
4. Should the user's answer be shown in the conversation as a user
   message (it is the user speaking), or as the tool call's result (as
   proposed)?

## Notes from building

- `Esc` skips (`decline`) as proposed; `Ctrl-C` cancels the turn. Open
  questions 2–4 stay as proposed: the limits as in "The tool", waiting
  shown like other questions, the answer as the tool call's result.
- `ask_user` is not a `config::Tool`. The key is `ask_user` in
  `PermissionsFile`, and `Permissions::ask_user_denied` holds it. That
  keeps it out of rules, groups and `Tool::ALL` without special cases.
  For the same reason the question's `call` is its own type, `AskCall`
  (`tool` is the string `ask_user`), not `CallInfo`.
- The tool's argument types, checks, `UserAnswer` and the result texts
  live in `src/tools/ask_user.rs`; `agent::question` re-uses `AskQuestion`
  and `UserAnswer` from there.
- `Setup::can_ask`: `true` for server sessions, and for `clai run` when
  answering on the terminal. A call where the tool is not offered, and a
  `NoOne` reply, give the "not available" error with the call `denied`.
- Labels of one question must differ ignoring case. A default `header`
  is the question's first line cut to 16 characters, ending in `…`.
- The text dialog: `Dialog::restart` now returns the text to show again
  (the first question from the model), so `clai chat` prints it after a
  rejected answer. The script form of `clai chat` gains `decline`.
- The tool line for a done `ask_user` call shows the answers
  (`SQLite; Search, Export`), not "The user answered:".
- TUI: the box state is `tui::ask::Ask`, held in `Prompt::ask`. The bar
  is not drawn for this kind, and the box draws under the session list
  and the picker, which can open over it. In the box, `PgUp` / `PgDn`
  scroll the box, not the conversation. `Ctrl-D` does not detach while
  text is typed. What was chosen is kept across a reconnect, but not when
  switching to another session and back (left in `todo.md`).
- `/status` counts the answers `reply` and `decline` (shown as `answered`
  and `declined`); `AnswerCounts` gains `reply` and `decline` with serde
  defaults.
- Config reload does not reach running sessions, as for every permission,
  so `ask_user = "deny"` applies to sessions started after it.
- Tests: 26 new (671 in all): the tool checks and texts, agent, config, protocol,
  stats, the text dialog, `LineAsker`, the server (first valid answer),
  `clai run` without a terminal, and the TUI keys with four snapshots.
