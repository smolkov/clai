# Agent loop

The agent loop runs one turn of a session: it sends the conversation to the
model, streams the answer, runs the tool calls the model asks for, sends
their results back, and repeats until the model answers without tool calls.
It is the only part of `clai` that talks to both the provider (see
[`providers.md`](providers.md)) and the tools (see [`tools.md`](tools.md)).

The loop knows no transport and no client (see
[`architecture.md`](architecture.md)). It takes user messages, answers and
cancellation as input and emits session events as output. The session
actor in the server and the one-shot `clai run` both drive the same loop.

Statements marked **(assumption)** need a decision from the user.

## Terms

| Term | Meaning |
|------|---------|
| Turn | One user message and everything the loop does until it waits for the next one. |
| Request | One model request in a turn: a `Request` sent to the provider and its streamed answer. |
| Answer | The assistant message of one request: its text and its tool calls. |
| Budget | The number of requests and tool calls a turn may still use before it asks the user (see [Limits](#limits)). |
| Question | A permission or limit question the loop waits on (see [`sessions.md`](sessions.md), "Questions"). |

## Inputs and outputs

| Direction | What | Notes |
|-----------|------|-------|
| in | User message | Starts a turn. Messages that arrive during a turn are queued by the session (see [Queued messages](#queued-messages)). |
| in | Answer | The answer to the open question, already checked by the session against the question's id and choices. |
| in | Cancel | Cancels the turn at any point (see [Cancellation](#cancellation)). |
| out | Events | Everything that happens, in order (see [Events](#events)). |
| out | History | The messages of the turn, appended to the session history. |

The loop gets a snapshot of the session for each turn: the history, the
provider and model, the permissions, the session rules and the workspace.
Session rules added by an "always" answer during the turn apply to the
following calls of the same turn.

In Rust (`src/agent/`):

```rust
impl Agent {
    /// History, model, limits, permissions with session rules, workspace.
    pub fn new(setup: Setup) -> Agent;

    /// Runs one turn and returns how it ended. Events go to `events`;
    /// questions are answered through `asker`.
    pub async fn run_turn(
        &mut self,
        message: String,
        events: &EventSink,
        asker: &dyn Asker,
        cancel: CancellationToken,
    ) -> TurnEnd;
}

pub trait Asker: Send + Sync {
    /// An answer and who gave it, or `NoOne` when no one can answer.
    fn ask<'a>(&'a self, question: &'a Question) -> BoxFuture<'a, Reply>;
    /// The answer was not accepted; the question stays open.
    fn reject(&self, question: &Question, by: &str, message: &str);
}
```

`Asker` is how the loop gets a decision. The session opens a question for
its clients and waits; `clai run` asks on the terminal, or answers by its
policy without asking (`PolicyAsker`): `ask` is denied (no terminal,
`--no-ask`) or allowed (`--yes`), and a limit question gets `NoOne`, which
counts as `stop` (see [`command.md`](command.md), "clai run").

The loop checks every answer itself: the choice must be one of the
question's `choices`, a `deny_message` needs a text, and the rules of an
`allow_always` must pass `accept_rules` (see [`sessions.md`](sessions.md),
"Answers"). A wrong answer goes back to the asker with `reject` and the
same question is asked again, with the same id.

## A turn

```
 user message
      │
      ▼
 ┌──────────────┐   budget used up   ┌────────────────┐  stop
 │ next request │ ─────────────────▶ │ limit question │ ──────▶ end: limit
 └──────────────┘ ◀───────────────── └────────────────┘
      │                 continue
      ▼
 stream answer ── provider error ─────────────────────────────▶ end: failed
      │
      ▼
 tool calls? ── no ───────────────────────────────────────────▶ end: done
      │ yes
      ▼
 for each call, in order:
   prepare ─▶ deny ─▶ error result
           ─▶ ask  ─▶ question ─▶ allow / always ─▶ run
                                ─▶ deny           ─▶ error result
                                ─▶ cancel         ─▶ end: cancelled
           ─▶ allow ─▶ run
      │
      ▼
 all results as one user message ─▶ next request
```

1. The user message is appended to the history and the session state
   becomes `working`.
2. The loop checks the budget, builds a [request](#requests) and streams the
   answer (see [Streaming](#streaming)).
3. The answer is appended to the history as one assistant message.
4. If the answer has no tool calls, the turn ends with `done`.
5. Otherwise the tool calls run one after another, in the order the model
   sent them (see [Tool calls](#tool-calls)). Their results are appended as
   one user message, in the same order, and the loop goes back to step 2.

Every `ToolCall` in the history gets exactly one `ToolResult` in the next
user message, also when it was denied, failed, cancelled or not run
because the turn stopped (see [`providers.md`](providers.md), "Message").
The loop never sends a request while a tool call has no result.

## Requests

A request is built from:

| Field | Value |
|-------|-------|
| `model` | The session's model `id`. A model change during a turn applies from the next request. **(assumption)** |
| `system` | The [system prompt](#system-prompt). |
| `messages` | The whole session history, oldest first, including this turn so far. |
| `tools` | The specs of all six tools (`tools::specs()`), also for tools whose default is `deny`, so the model gets a clear denial instead of not knowing the tool. **(assumption)** |
| `max_output_tokens`, `temperature` | From the model's configuration. |

The history is sent in full. If it no longer fits, the provider returns
`ContextTooLong` and the turn fails (see [Errors](#errors)); shortening the
history is part of `context.md` (planned).

### System prompt

The first version has a short, fixed system prompt. Its text lives in the
code (`src/agent/system_prompt.md`, built in with `include_str!`) and gets
these values filled in:

| Value | Example |
|-------|---------|
| Workspace path | `/home/user/ws/rust/clai` |
| Extra directories and their access | `~/ws/docs (read only)` |
| Platform and shell | `linux, commands run with sh -c` |
| Today's date | `2026-11-01` |

The prompt says what `clai` is (a coding agent working in that
workspace), that paths are relative to the workspace, that commands start in
the workspace each time and get no input, that edits need `read_file`
first to get the exact text, that denied calls should not be repeated
unchanged, and that the final answer is shown to the user as Markdown.

Project instructions (e.g. an `AGENTS.md` file) and anything else that
grows the prompt belong to `context.md` (planned). **(assumption)**

## Streaming

Provider events (see [`providers.md`](providers.md), "Events") become
session events as they arrive:

| Provider event | The loop |
|----------------|----------|
| `TextDelta` | Appends the text to the answer and emits `text_delta`. |
| `ToolCallStart` | Emits `tool_call` with state `pending`, the name and empty arguments. A text block before it is finished with `text_done`. |
| `ToolCallArgsDelta` | Collects the arguments; emits nothing, so a client is not sent half JSON. **(assumption)** |
| `ToolCallEnd` | Keeps the call with its parsed arguments and emits `tool_call` `pending` with them. |
| `Usage` | Keeps the counts; the last one of the request counts. |
| `Done` | Ends the answer: emits `text_done` if text is open, then `usage`. |

How the answer ends decides what follows:

| `stop_reason` | The loop |
|---------------|----------|
| `EndTurn` | Ends the turn with `done`, unless the answer has tool calls; then they run, as for `ToolUse`. |
| `ToolUse` | Runs the tool calls. An answer with `ToolUse` and no tool calls ends the turn with `done`. |
| `MaxTokens` | The answer was cut. Complete tool calls run; a call that was started but not ended is left out of the answer. If nothing is left to run, the turn ends with `done` and the notice `answer cut at max_output_tokens`. **(assumption)** |
| `Other` | As `EndTurn`, with the notice `the model stopped: other reason` when there are no tool calls; the adapter logs the provider's own reason. **(assumption)** |

An empty answer (no text, no tool calls) is kept out of the history and
ends the turn with `done` and the notice `the model gave an empty answer`.
**(assumption)**

## Tool calls

Tool calls of one answer run one after another, never in parallel, in the
order the model sent them, because a later call may depend on an earlier
one (an edit after a read, a test run after an edit). **(assumption)**

Each call is handled when its turn comes, not all at the start, so it sees
the files as the calls before it left them and the session rules added by
answers before it:

1. **Budget:** if the tool call budget is used up, the limit question opens
   first (see [Limits](#limits)).
2. **Prepare:** `tools::prepare` checks the arguments and the permission and
   builds the question's details (see [`tools.md`](tools.md)).
   - Wrong arguments or an error found before running (file not found,
     `old_text` not found): the call gets that error result, state `error`.
   - `deny`: the call gets the denial text, state `denied`.
   - `allow`: the call runs.
   - `ask`: a permission question opens, state `waiting`.
3. **Question:** the question has the call, its details, the reason, the
   proposals, and the answer's text before the call (see
   [`sessions.md`](sessions.md), "Content"). The answer decides:

   | Answer | The loop |
   |--------|----------|
   | `allow` | Runs the call. |
   | `allow_always` | Adds the rules (checked with `accept_rules`) to the session, emits a notice with them, and runs the call. |
   | `deny` | Result `denied by the user`, state `denied`. |
   | `deny_message` | Result `denied by the user: <text>`, state `denied`. |
   | `cancel` | Result `cancelled by the user`, then the turn is cancelled (see [Cancellation](#cancellation)). |

4. **Run:** state `running`, then `PreparedCall::run`. For `run_command`,
   output is sent with `tool_call` events as it arrives, at most every
   100 ms. **(assumption)** The result is the tool's output; its state is
   `done`, or `error` when the result is an error.

The model gets the result's model text (at most 30 000 characters); events
and the history keep the full text, up to 1 MiB, for display (see
[`tools.md`](tools.md), "Result size"). **(assumption: the history keeps
both)**

A denied call does not stop the turn: the model gets the denial as an error
result and may try something else. A model that keeps making the same
denied call uses up its budget and reaches the limit question.

## Limits

A turn has a budget of model requests and tool calls, so a confused model
cannot loop forever or run up costs while no one watches.

```toml
[limits]
max_requests   = 50     # model requests per budget
max_tool_calls = 120    # tool calls per budget
```

| Key | Default | Range | Counts |
|-----|---------|-------|--------|
| `max_requests` | 50 | 1–10000 | Model requests, including the first. A request retried by the adapter counts once. |
| `max_tool_calls` | 120 | 1–10000 | Tool calls, including denied ones and ones with wrong arguments. |

- The keys may be set in the user file and in the project file. Like
  permissions, a project file may only make them stricter: for each key the
  lower value wins. **(assumption)**
- The request budget is checked before each request, the tool call budget
  before each call. When one is used up, the loop asks a limit question with
  the limit and the counts reached (e.g. `50 requests, 87 tool calls`) and
  waits.
- **Continue** gives a new budget of the same size for both counts; the
  loop goes on where it stopped.
- **Stop** ends the turn with `limit`. Calls of the current answer that have
  not run get the result `not run: the turn was stopped at its limit`, so
  every call has a result. The notice `limit reached` is emitted.
- The budget belongs to the turn; a new turn starts with a full one.
- Limits are read when the turn starts; a configuration reload applies from
  the next turn.

There is no time limit for a turn: a turn that waits for a question waits
as long as it takes (see [`sessions.md`](sessions.md), "Background work").
Each command has its own timeout (see [`tools.md`](tools.md),
"run_command").

## Cancellation

A turn can be cancelled at any point: `Esc` in the TUI, `cancel` as a
question's answer, `clai kill`, `Ctrl-C` in `clai run`. The loop stops what
it is doing at once and keeps the history valid:

| Cancelled while | The loop |
|-----------------|----------|
| Waiting for the budget check or before a request | Nothing to undo. |
| Streaming | Stops the request (the adapter closes the connection). The text so far is kept as the assistant message; tool calls of the unfinished answer are left out, since none of them ran. **(assumption)** |
| Waiting on a permission question | Closes the question; the call gets `cancelled by the user`. |
| Running a tool | Stops it (see [`tools.md`](tools.md), "Cancellation"); the call gets `cancelled by the user` with the output so far. |
| Waiting on a limit question | Closes the question. |

Calls of the current answer that have not run yet get `cancelled by the
user`. A limit question offers `cancel` too, as in
[`sessions.md`](sessions.md), "Answers". Calls already done keep their results. The turn ends with
`cancelled` and the notice `turn cancelled`.

The cancelled turn stays in the history, so the model sees in the next turn
what was done and what was cancelled.

## Errors

The loop never panics and never ends the session (see
[`agent.md`](../agent.md), "Never crash"). A tool error is a result for the
model, not an error of the turn.

| Error | The loop |
|-------|----------|
| Provider error before the first event | The adapter has already retried where it should (see [`providers.md`](providers.md), "Errors"). The loop emits `error` with the message, and the turn ends with `failed`. |
| Provider error during the stream | As for cancellation while streaming: the text so far is kept, the tool calls of the broken answer are left out; then `error` and `failed`. Not retried. **(assumption, see `providers.md` open question 3)** |
| `ContextTooLong` | `error` with `the conversation is too long for <model> (<window> tokens); start a new session`, and `failed`. Until `context.md` adds truncation. |
| `Auth` | `error` with the provider's message, and `failed`. The next turn tries again. |
| A tool task that panics or fails to join | The call gets the error result `the tool failed: …`; the turn goes on. |

After a failed turn the session is `idle` again. The user message and
everything the turn did stay in the history; the next message continues
from there. Neighbouring messages of the same role (a user message after a
failed request) are merged by the adapter where the API needs it (see
[`providers.md`](providers.md), "anthropic").

## Queued messages

Messages the user sends while a turn runs are queued by the session in the
order they arrive. The loop does not add them to a running turn; when the
turn ends, the session starts a new turn with the next queued message.
**(assumption)** A cancelled turn does not clear the queue.

## Events

The loop emits these session events (see [`architecture.md`](architecture.md),
"Protocol"):

| Event | When |
|-------|------|
| `state` | `working` at the start, `waiting` while a question is open, `working` again after the answer, `idle` at the end. |
| `user_message` | The message that started the turn. |
| `text_delta`, `text_done` | Streamed answer text; `text_done` ends a text block. |
| `tool_call` | Each change of a call: `pending` (when it starts, and again with its arguments when it ends), `waiting` (question open), `running` (with output as it arrives), then `done`, `error`, `denied` or `cancelled`, with the result. Each event has the call's id, tool name and arguments; `waiting` and `running` also the summary. A call left out of a broken, cancelled or cut answer gets a final `error` or `cancelled` event too, so no client shows it as pending. |
| `question`, `question_closed` | A permission or limit question, and how it was closed: `answered` (with the answer and who gave it), `no_one` or `cancelled`. |
| `notice` | `rules added: allow run_command(cargo test)`, `answer cut at max_output_tokens`, `limit reached`, `turn cancelled`, … |
| `usage` | After each request: its input and output tokens, and the context size (input plus output tokens of the last request). |
| `error` | A provider or session error that ended a request. |
| `retry` | The adapter tries the request again (see [`providers.md`](providers.md), "Adapters"): the attempt, `max_retries` and the reason. Sent before the wait. |
| `turn_end` | How the turn ended: `done`, `cancelled`, `limit` or `failed`; the counts of requests and tool calls and the tokens used. |

The events of a turn are complete: a client that shows only the events
shows the same conversation as the history holds, so an attach that
replays the event log needs nothing else.

## Testing

The loop is tested with the `fake` provider, which replays scripted events
and errors (see [`providers.md`](providers.md), "Testing"), and with tools
in a temporary workspace. Each test checks the emitted events and the
history:

- a plain answer; text, then tool calls, then a final answer;
- a tool round trip with several calls in one answer, run in order;
- a denied call, a call with wrong arguments and invalid JSON;
- a permission question answered with each choice, including
  `allow_always` making a later call in the same answer run without asking;
- cancellation while streaming, while waiting and while a command runs;
- the request limit and the tool call limit reached, then continue and
  stop;
- a provider error before and during the stream, and `ContextTooLong`;
- `MaxTokens` with a cut tool call, `Other`, and an empty answer;
- the invariant that every tool call in the history has exactly one result
  before the next request, checked in every test.

## Open questions

1. Should independent read-only calls of one answer (`read_file`,
   `list_files`, `search_files`) run in parallel, to save time?
2. Should a queued message be added to the running turn at the next request
   instead of waiting for the turn to end, so the user can steer the model
   while it works?
3. Should the loop stop a turn by itself when the model repeats the same
   failing or denied call several times, before the limit is reached?
4. Should there be a limit on tokens or cost per turn, in addition to
   requests and tool calls?
