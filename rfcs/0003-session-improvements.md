# Session improvements

- RFC: 0003
- Status: done
- Milestone: the model picker of "TUI sessions and settings" (see
  [`implementation.md`](../implementation.md)); part B of "Refactoring" in
  [`todo.md`](../todo.md)
- Spec: [`spec/tui.md`](../spec/tui.md), "Layout", "Model picker", "Keys";
  [`spec/sessions.md`](../spec/sessions.md), "Persistence";
  [`spec/architecture.md`](../spec/architecture.md), "Protocol";
  [`spec/command.md`](../spec/command.md), "clai ls", "clai chat"

## Summary

A session counts the input and output tokens it has used, keeps the count
when saved, and shows it in the TUI, `clai chat` and `clai ls --json`. The
model of a session can be changed with a picker (`Ctrl-O`, `/model`), and
the faults of the model change are fixed. The session code that these
changes touch is refactored first, without a change of behaviour.

## Motivation

- **Tokens.** Each request emits a `usage` event, but clients only use its
  context size. The user cannot see what a session has used, although the
  footer has room for it and the data is already in the log.
- **Model change.** `/model` without an argument says "comes with a later
  version", and `Ctrl-O` does nothing, so the user has to know the exact
  model name. The change also has faults:
  - On replay the TUI writes the wrong old model. `attached` sets the current
    model before it replays the log, so `a → b` comes back as `b → b`
    (`src/tui/app.rs`, `SessionEvent::ModelChanged`).
  - Changing the model of a saved session leaves no trace in the log, although
    `spec/tui.md` says a notice records each change (`Shared::resume`).
  - Choosing the current model adds a notice `a → a` (`Msg::SetModel`).
  - The TUI takes context windows from its own loaded configuration. That file
    can differ from the server's: another `--config`, or a reload since the TUI
    started. A model the TUI does not know shows no gauge at all.
- **Refactoring.** Adding the counters means changing the two copies of
  `SessionInfo` construction, and the picker adds to a request file that looks
  up sessions by name by hand in four places.

## Scope

In:

- Token totals per session: counted by the session, saved, shown.
- Model picker, the `models` request, and the fixes above.
- Refactoring: `Meta::info`, a session registry type, the question state
  machine in its own type.

Out:

- Cost in money, prices per model, per-model token breakdown.
- Limits on tokens per turn (`spec/agent-loop.md`, open question 4).
- The confirmation and shortening when history does not fit a new window: they
  need `spec/context.md`. The picker only marks such a model.
- The session list, the command list, `/model` argument completion and live
  reload of `[tui]`: they stay in "TUI sessions and settings".
- The rest of refactoring B (agent split, shared chat answering, `paths`,
  `signals`), and C and D.

## Design

### Refactoring

Done first, no behaviour change, existing tests unchanged.

- **`Meta::info(id, state, queued, question)`** builds `SessionInfo`. It
  replaces `saved_info` (`src/server/requests.rs`) and the fields written by
  hand in `Actor::info`.
- **`server::registry::Registry`** owns the `BTreeMap<u64, Entry>`, still behind
  the `Shared` mutex: `by_name`, `by_name_mut`, `is_taken`, `unique_name`,
  `insert`, `remove`, `handles`. It replaces four hand-written lookups by name
  and four "is the name taken" checks.
- **`session::questions::Questions`** owns `open`, `asking`, `answers`,
  `checking` and `closed`, with `Actor::answer`, `deliver`, `handle_ask` and
  `question_closed` as its methods. It sends no events and needs no task, so
  it gets its own unit tests.

### Token totals

Definition, written into the spec:

- **Input** and **output** are the sums over all requests of the session, from
  `usage` events. Input counts the history again with every request, because
  that is what is sent; it is not the context size.
- Anthropic's cached input is included in input, as the adapter already does.
  The counter is therefore not a bill.
- A cancelled request counts what the provider had reported by then. A provider
  that reports nothing adds nothing.

Types:

```rust
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
}
```

- `Meta` gets `#[serde(default)] usage: TokenTotals`. The file format stays `1`:
  a missing field means zero, and older versions ignore the new one.
- `SessionInfo` gets `usage: TokenTotals`, so that `list` can report a saved
  session without loading it.
- `Actor::publish` adds each `usage` event to `meta.usage` and saves the meta.
  That is at most one small atomic write per model request.
- `Actor::new` computes the totals by summing the log. The log is the truth: this
  covers sessions saved by an older `clai`, and a crash between writing the
  event and writing the meta. An old saved session shows zero in `clai ls` until
  it is first resumed. **(proposal)**

Clients:

- **TUI:** the session row ends with the counters, which are the first thing cut
  on a narrow terminal, and they are left out until the first `usage` event:

  ```
  project · ~/ws/project · anthropic/sonnet · 1/3 ●1 · 12.3k/200k · in 45.1k out 3.2k
  ```

  The words `in` and `out` are used rather than arrows: `↓` already means "new
  output" in the status row, and words need no ASCII fallback. **(proposal)**

  `App` sums `usage` events and resets the sum in `attached`, as it already does
  for the context size. It does not read `SessionInfo.usage`: the attach response
  comes before the replay, so adding the two would count everything twice, and a
  reconnect must not add to the old sum.
- **`clai chat`:** a new `/usage` prints `tokens: in 45.1k, out 3.2k; context
  12.3k`. **(proposal)**
- **`clai ls --json`:** new fields `input_tokens` and `output_tokens`. The table
  stays as it is. **(proposal)**
- `clai run` prints nothing new.

`tui::view::info::tokens` and `command::models::short_tokens` move to `text`, so
that all clients share them.

### Model change

- **`ModelChanged`** gets `from`, the old `provider/model`, set by the session:

  ```rust
  ModelChanged {
      #[serde(default, skip_serializing_if = "String::is_empty")]
      from: String,
      model: String,
  }
  ```

  The TUI writes the notice from the event alone. An old log has no `from`; the
  notice is then `model: b`.
- **Saved session:** `Shared::resume` appends `model_changed` to the log when the
  resolved model differs from `meta.model`.
- **Same model:** `Msg::SetModel` still applies the provider and model, which may
  come from a reloaded configuration, but publishes nothing.
- **Unknown context window:** the row shows `12.3k` without a limit, instead of
  hiding the context size.

### The `models` request

The picker asks the server, because the server is what `set_model` checks against
and what makes the requests. Its configuration is the one that counts, and so is
its environment for the API key.

```rust
Request::Models,
Payload::Models { models: Vec<ModelInfo>, config: PathBuf },

pub struct ModelInfo {
    /// `provider/model`.
    pub model: String,
    pub provider: String,
    pub name: String,
    pub id: String,
    pub context_window: u64,
    pub default: bool,
    /// The key's status as in `clai models --json`; `api_key_cmd` is not run.
    pub key: String,
}
```

A new request needs a new protocol version: `3`. The session state notice that
`todo.md` plans for protocol 3 then takes the next one. A running older server
gets the existing version mismatch error.

The TUI replaces `Settings.windows` with the answer. It asks on
`Notice::ConfigReloaded` and whenever the picker opens, and starts from its own
configuration as today, so the first frame has a gauge without waiting.

### Model picker

`App` stays pure. A new `picker: Option<Picker>` holds the models, the filter text
and the selected row. `Dialog` stays as it is, since it is `Copy` and has no state.

- `Ctrl-O` or `/model` sends `Request::Models`; the answer opens the picker. An
  error goes to the conversation like other failed requests. While a question is
  open the keys stay with the prompt bar, as `spec/tui.md` says, so the picker
  does not open.
- A model change during a turn is allowed and applies from the next request, as
  it does for `/model NAME` today.
- Layout, filter and marks as in `spec/tui.md`, "Model picker": grouped by
  provider, in config order, filtered by provider, name and id. Marks:
  `✓ current`, `default`, `⚠ no key`, and `⚠ history 41k > 32k` when the last
  context size is larger than the window. ASCII symbols come from `Symbols`.
- `Enter` sends `Request::SetModel` with the selected row and closes. On the
  current model it only closes. `Esc` closes. `Up`/`Down` select; other keys edit
  the filter. A filter that matches nothing shows `no model`, and `Enter` does
  nothing.
- With no models it shows the path of the configuration file and a short example.
- `src/tui/view/picker.rs` draws it; `popup.rs` shares its box code.

`clai chat` keeps `/model NAME` and gets no picker.

## Spec changes

Written when the RFC is accepted.

- `spec/tui.md`: the session row with the counters and their definition; the
  row without a known window; the picker's `Enter` on the current model; a model
  that does not fit is only marked until `context.md` exists; no notice when the
  model stays the same.
- `spec/architecture.md`: protocol `3`, the `models` request, `from` in
  `model_changed`, `usage` in the session info.
- `spec/sessions.md`, "Persistence": token totals in `session.json`, summed from
  the log on resume; a model change on a saved session is logged.
- `spec/command.md`: `/usage` in `clai chat`; new fields in `clai ls --json`.
- `readme.md`: `Ctrl-O`, `/model`, `/usage`. `/help` in the TUI likewise.

## Testing

- `Questions`: unit tests of first answer wins, a rejected answer, a late answer,
  answers waiting for the agent to ask.
- `Registry`: unit tests of lookups, taken names, unique names.
- Totals: `Actor` adds `usage` events; in-process restart keeps them; a log
  without totals in the meta sums to the right value; export and import keep
  them.
- TUI: counters from live and replayed events; **a reconnect does not double
  them**; hidden before the first `usage`; replay shows `a → b`, and an old log
  without `from` shows `model: b`.
- Server: a saved session changed to another model has `model_changed` in its log
  after the restart; setting the same model publishes nothing; the `models`
  request lists the server's models, also after a reload.
- Picker: key handling on `App` (filter, selection, `Enter`, `Esc`, nothing while a
  prompt is open); `insta` snapshots with marks, in Unicode and ASCII, and with no
  match.
- Protocol: round trips of the new messages; a version mismatch with `2`.
- Binary: `clai ls --json` has the new fields.

## Implementation plan

1. **`Meta::info` and `Registry`**
   - Status: done
   - Files: `src/session/store.rs`, `src/session/mod.rs`,
     `src/server/registry.rs`, `src/server/mod.rs`, `src/server/requests.rs`
   - Result: One way to build `SessionInfo` and to look up a session. No change of
     behaviour.
   - Tests: Existing tests unchanged; unit tests of `Registry`.
2. **`Questions`**
   - Status: done
   - Files: `src/session/questions.rs`, `src/session/mod.rs`
   - Result: The actor delegates questions. No change of behaviour.
   - Tests: Existing tests unchanged; unit tests of `Questions`.
3. **Totals in the session**
   - Status: done
   - Files: `src/session/store.rs`, `src/session/mod.rs`, `src/protocol/mod.rs`,
     `src/command/ls.rs`
   - Result: `usage` in `session.json` and `SessionInfo`; `clai ls --json`.
   - Tests: Totals section above.
4. **Counters in the clients**
   - Status: done
   - Files: `src/text/`, `src/tui/app.rs`, `src/tui/view/info.rs`,
     `src/command/chat/talk.rs`, `src/command/models.rs`
   - Result: The TUI session row and `/usage`; a row without a known window.
   - Tests: TUI tests above; updated frame snapshots.
5. **Model change fixes**
   - Status: done
   - Files: `src/agent/events.rs`, `src/session/mod.rs`, `src/server/requests.rs`,
     `src/tui/app.rs`
   - Result: `from`, correct replay, saved sessions logged, no notice for the same
     model.
   - Tests: Replay and server tests above.
6. **`models` request**
   - Status: done
   - Files: `src/protocol/mod.rs`, `src/server/requests.rs`, `src/tui/app.rs`
   - Result: Protocol `3`; the TUI takes context windows from the server.
   - Tests: Protocol round trips; the request before and after a reload.
7. **Model picker**
   - Status: done
   - Files: `src/tui/picker.rs`, `src/tui/app.rs`, `src/tui/keys.rs`,
     `src/tui/view/picker.rs`, `src/tui/view/popup.rs`, `src/tui/view/mod.rs`,
     `src/tui/style.rs`
   - Result: `Ctrl-O` and `/model` open the picker.
   - Tests: Picker tests above.
8. **Docs**
   - Status: done
   - Files: `spec/`, `readme.md`, `implementation.md`, `todo.md`, `rfcs/readme.md`
   - Result: Spec changes above; refactoring items and the picker ticked; status
     `done`.
   - Tests: `cargo fmt`, `clippy`, `test`.

## Alternatives

- **Only the clients sum the log, nothing is saved.** No store or protocol change,
  and the TUI works the same. Rejected because `clai ls` and the later session
  list could not show totals without loading each session's whole log.
- **The TUI reads the totals from `SessionInfo`.** One source, but the attach
  response comes before the replay, and live events after it must be told apart
  from replayed ones. Summing is one line and cannot double.
- **Totals from `turn_end`.** It has the counts too, but a turn repaired after a
  crash reports zero, and a running turn would only count once it ended.
- **Picker from the client's own configuration.** No new request or protocol
  version, but it can list models the server rejects and misreport the key.
- **A `↑ ↓` counter.** Shorter, but `↓` already means "new output".
- **Leave the picker for "TUI sessions and settings".** Possible: the fixes and
  counters stand without it. See the open questions.

## Open questions

Answered when the RFC was accepted, each as this RFC assumed:

1. **Does "model change" include the picker?** Yes.
2. **Which refactoring points?** The three that the session work touches; the
   rest of B stays in `todo.md`.
3. **Where should the counter show?** The TUI, `clai chat` `/usage` and
   `clai ls --json`; no column in `clai ls`, no line after `clai run`.

## Notes from building

Details the design left open:

- The session row shows no context size while the window is unknown and nothing
  was counted yet: a bare `0` says nothing.
- A question that opens closes the picker, and an answer on its way does not open
  it. Otherwise the picker would hide the prompt bar while keys go to it, and
  typing a filter could answer the question.
- A reload refreshes an open picker; the selected model stays selected if it can.
- `Up` and `Down` stop at the ends of the list. Editing the filter selects the
  first match.

Left as designed, and open in `todo.md`: the TUI asks for the server's models only
when the picker opens and after a reload. A TUI started with another `--config`
therefore shows its own windows until then, so that fault of the motivation is
only partly fixed.
