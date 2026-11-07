# Trim old tool results

- RFC: 0008
- Status: draft
- Milestone: Context management, the history part
- Spec: [`spec/agent-loop.md`](../spec/agent-loop.md), "Requests", "Errors";
  [`spec/tools.md`](../spec/tools.md), "Result size";
  [`spec/config.md`](../spec/config.md), "Files", "Limits";
  [`spec/readme.md`](../spec/readme.md), open question 3

## Summary

Requests no longer send the whole history: when a request is built, the
result of a tool call from an older answer is replaced with a one-line
placeholder, so what the model reads each time grows with the turn being
worked on, not with the whole session. The history, the saved session, the
events, the TUI and `clai chat` keep every result in full; only the copy
that goes to the provider is shortened.

## Motivation

Every request sends the full session history, oldest first (see
[`agent-loop.md`](../spec/agent-loop.md), "Requests"). Tool results are by
far the largest part of it:

- One result may be 30 000 characters, about 7 000 tokens of code
  (see [`tools.md`](../spec/tools.md), "Result size").
- Each result is sent again with every later request, so a session of n
  such calls pays for each result n times: input grows with the square of
  the work.
- The per-call caps bound a single result, not the sum. Within the limits
  of one turn (50 requests, 120 tool calls) the results alone can reach
  3.6 M characters, about 900 000 tokens — several times the window of the
  largest configured models.
- When the history no longer fits, the turn fails with `ContextTooLong`
  (`the conversation is too long …; start a new session`) and nothing can
  shorten it. The context gauge of RFC 0003 shows the problem but cannot
  act on it.

A session that reads and commands a lot thus becomes slow and expensive
long before its work is done, and then fails outright. `spec/context.md`
is planned for the wider answer; the largest single lever, old tool
results, is small and safe enough to take on its own.

## Scope

In:

- Placeholders instead of old tool results in the messages of a request.
- Config keys for it in the user's config file.
- One sentence in the system prompt so the model knows results may be
  trimmed.

Out:

- The context gauge, a model change to a smaller window, and recovery
  from `ContextTooLong` within a turn: all for `spec/context.md`.
- Summarizing old turns with the model ("compaction").
- Trimming tool call arguments (`edit_file`'s `old_text` and `new_text`
  stay in the history as they were sent).
- Prompt caching (see [`providers.md`](../spec/providers.md), open
  question 4): it cuts price, not context use.
- The per-result cap of 30 000 characters and the tool result formats:
  unchanged.

## Design

### What the model sees

The answers that carry tool calls divide the history into exchanges: each
assistant message with tool calls is followed by a user message with
their results. The **age** of a result is the number of later assistant
messages that hold tool calls. The newest answer has age 0.

When a request is built, the messages are walked from the end. The result
of a call is replaced with a placeholder when its age is at least
`keep_results`; a result of at most `min_chars` characters is never
replaced. **(proposal)**

```
[trimmed read_file src/parse.rs offset 1 limit 2000: 28 540 characters, not an error; call the tool again if you need it]
```

- The placeholder names the tool and its arguments (the main one, then the
  others as `key value`), the size of the original result, whether it was
  an error, and how to get it again. It is one line, whatever the size of
  the result. **(proposal)**
- The `is_error` flag of the result block is kept, so the model still
  knows the last state of every call.
- Everything else is sent as before: user messages, assistant text, the
  tool call arguments, and the results of the newest `keep_results`
  answers.

With the defaults (`keep_results = 2`), the model sees the full results of
the answer it is working on and the one before, and placeholders for
everything older — a bounded amount per request instead of the whole
session. **(proposal)**

### What stays whole

The trimming happens only when the request is built; nothing is deleted
anywhere:

- The in-memory history keeps every result as it has kept it.
- The saved session (`history.jsonl`, `Record::Full`), the events of a
  turn, replay on attach, `clai export` / `clai import` and resume are
  unchanged; protocol and event log format stay as they are.
- The TUI, `clai chat` and the permission questions show the full result
  as before.
- The token totals count what the provider reports, so they fall on
  their own.

A result that the model asks for again is new and fresh; it is never
trimmed immediately.

### Config

```toml
[history]
trim_results = true   # replace old tool results with placeholders
keep_results = 2      # results of the newest 2 answers go to the model in full
min_chars    = 500    # results this size or smaller are never trimmed
```

- `trim_results` switches the whole thing off, for a model whose window is
  big enough that nothing needs to be cut. Range: `true` / `false`.
- `keep_results` is 1–100. `min_chars` is 0–30 000. **(proposal)**
- User file only, like `[tui]`: the project file sets neither key, and the
  reload applies them from the next request. **(proposal)**

### The system prompt

The system prompt gains one sentence, so the model does not guess from a
placeholder: old tool results may be replaced with short placeholders;
read or run the call again when you need them. **(proposal)**

### ContextTooLong

A turn can still overflow on its own: its limits allow far more results
than a window holds. The error, its message and the failed turn stay as
they are; `spec/context.md` may add a real answer later.

## Spec changes

- `spec/agent-loop.md`: the "Requests" table and the "The history is sent
  in full" paragraph change to describe the placeholders, in a new
  section "Trimmed results"; the `ContextTooLong` row stays and links to
  it.
- `spec/tools.md`: "Result size" keeps its 30 000-character cap and adds
  the pointer to "Trimmed results" in `agent-loop.md`.
- `spec/config.md`: a new "History" section with the three keys, and the
  "Files" table says the project file sets none of them.
- `spec/agent-loop.md`, "System prompt": the sentence enters the prompt's
  text there.
- `readme.md`: the `[history]` keys under "Configuration", and the note on
  the token row of the session list.
- `implementation.md`: the milestone "Context management" notes that RFC
  0008 builds its history part.

## Testing

- Unit tests of the trimming, as a pure function from the history and the
  settings to the messages of a request: age, `min_chars`, error flag,
  `trim_results` off, arguments in the placeholder.
- Agent-loop tests with the fake provider, which records each request: a
  turn of several requests shows the first placeholders exactly at
  `keep_results`, and the results of the newest answers stay whole.
- History tests stay green: journal, restore, replay and resume are
  untouched by the change.
- Config tests: parse and validate the keys, reject ranges, reject them in
  a project file, apply a reload in a running session.

## Implementation plan

Each step leaves a working, tested state.

1. **Trim when building a request**
   - Status: todo
   - Files: `src/agent/history.rs`, `src/agent/mod.rs`, `src/agent/tests.rs`
   - Result: `History` offers the messages for a request; the agent loop
     builds requests with placeholders, with the default settings
     hardcoded.
   - Tests: unit tests of the trimming rules; fake-provider requests hold
     placeholders and full results at the right places.

2. **Settings**
   - Status: todo
   - Files: `src/config/`, `src/agent/mod.rs`, `src/config/tests.rs` (where
     the config tests live)
   - Result: `[history]` is parsed, validated and read by the loop; a
     reload applies from the next request.
   - Tests: config parse, ranges, project-file rejection, reload.

3. **System prompt and spec**
   - Status: todo
   - Files: `src/agent/system_prompt.md`, `spec/agent-loop.md`,
     `spec/tools.md`, `spec/config.md`, `readme.md`, `implementation.md`
   - Result: the model is told about placeholders; the spec describes the
     behaviour where it will be specified.
   - Tests: the existing prompt and loop tests keep passing.

## Alternatives

- **Wait for `spec/context.md`.** It is planned for the whole answer, but
  it needs the gauge, model changes and recovery from `ContextTooLong`:
  a far bigger step. Every day without trimming, each request of a heavy
  session pays for the whole session again. Rejected: the largest gain is
  small and safe on its own.
- **Compaction: summarize old turns with the model.** Saves more than
  placeholders, but costs a request and output tokens each time, changes
  what the replay, the TUI and the saved session show, and risks losing
  details the model still needs. Can come later on top; the placeholders
  make it less urgent and it stays in `spec/context.md`.
- **Drop the oldest messages wholesale.** Loses the user's task and
  instructions first, which is worse than losing file contents the model
  can read again. Rejected.
- **Trim the history itself, not the request.** Would shrink the saved
  session and break the replay: a client that attaches later could no
  longer show the conversation the events promise. Rejected: only the
  copy that goes to the provider is shortened.
- **Prompt caching (Anthropic `cache_control`).** Cuts price and latency,
  not context use: the window still fills and `ContextTooLong` still
  comes. Complementary; open question 4 stays open.

## Open questions

1. The defaults: `keep_results = 2`, `min_chars = 500`. Two answers have
   been enough in similar agents; three costs more input per request,
   one risks the model still working from a result.
2. The placeholder wording: is one uniform line right, or should
   `run_command` keep its last line (`exit code 1`) and `read_file` its
   `… more lines` note, so the model knows the outcome without asking
   again?
3. Should large error results be trimmed too (proposed: yes, the
   `is_error` flag survives), or kept whole because they carry the
   reason a later fix may need?
4. Should the project file be able to *lower* `keep_results`, like
   `[limits]`? (Proposed no: it changes what the model sees, not what it
   may do.)
5. Per-model settings: a session with a huge-window local model may want
   `trim_results = false` while others keep the default. A `[models]`
   override can follow; needed now?
