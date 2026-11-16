# Fewer tokens, same answers

- RFC: 0028
- Status: done (bench baseline open, see step 2)
- Milestone: Context management
- Spec: [`spec/providers.md`](../spec/providers.md), "anthropic",
  "openai-compatible", open question 4;
  [`spec/agent-loop.md`](../spec/agent-loop.md), "Requests", "System
  prompt"; [`spec/tools.md`](../spec/tools.md), "Result size",
  "read_file", "run_command"; [`spec/sessions.md`](../spec/sessions.md)
  (token totals); [RFC 0008](0008-trim-old-tool-results.md)

## Summary

`codeagent` pays for fewer tokens per session without taking anything
from the model that it needs: requests keep a stable prefix and mark it
for the provider's prompt cache; RFC 0008's trimming is changed to
trim in steps, so it does not break that cache on every request; command
output loses what carries no information (colour codes, overwritten
progress lines, runs of the same line); and a file read again unchanged
points to the earlier result instead of repeating it. The token totals
count cached input on its own, and a fixed set of tasks measures cost and
success before and after each step.

## Motivation

Each request sends the system prompt, the tool specs and the whole
history again (see [`agent-loop.md`](../spec/agent-loop.md), "Requests").
In a turn of 30 requests the first message is paid for 30 times. Almost
all of that input is the same as in the request before: only the newest
answer and its results are new.

- Providers charge much less for input they have cached: Anthropic reads a
  cached prefix at a tenth of the input price, OpenAI-compatible servers
  that cache do it automatically for a prefix that stays the same. Today
  `codeagent` sends no `cache_control`, so Anthropic caches nothing, and
  nothing keeps the prefix stable on purpose.
- RFC 0008 shortens the history, but as drafted it changes an old message
  in nearly every request (the result that just reached `keep_results`
  becomes a placeholder). Each change breaks the cache from that point on,
  so trimming and caching built separately would work against each other.
- Command output is often mostly noise for the model: ANSI colour codes,
  progress bars redrawn with `\r`, hundreds of `Compiling …` or
  `Downloading …` lines. It counts against the 30 000-character cap and
  pushes out the lines that matter.
- Models read the same file again after a few steps, to be sure. When the
  file has not changed and the earlier result is still in the request,
  the second copy tells the model nothing new.
- The token totals add cached and uncached input together (see
  `providers.md`, "anthropic"), so neither the user nor we can see what
  caching saves.

"Without quality reduction" is the rule for every part of this RFC: the
model either sees the same information as before, or sees a line that
says exactly what was left out and gets it back with one call. Nothing is
summarized by a model and nothing is dropped silently.

## Scope

In:

- Prompt caching for the `anthropic` adapter, and a stable request prefix
  for all adapters.
- Cached input in the usage events, the session totals and the
  statistics.
- Trimming in steps: a change to RFC 0008's design, before it is built.
- Lossless cleaning of `run_command` output.
- `read_file` of an unchanged file whose earlier result is still sent.
- A measurement: fixed tasks, run with `codeagent run --report`, with
  tokens, cost and success per task.

Out:

- Compaction (a model summarizes old turns): it can lose details; it
  stays in `context.md` (planned).
- Smaller or cheaper models for parts of a turn, sub-agents.
- Shorter system prompt or tool descriptions as a goal in itself: they
  are small (about 1 500 characters of prompt) and paid once per cached
  request; wording changes go through the measurement like everything
  else.
- Lower limits (`[limits]`, the 30 000-character cap, `read_file`'s 2 000
  lines): they change what the model can see.
- Thinking blocks and their budget (`providers.md`, open question 1).

## Design

### A stable prefix

The cache works on the longest prefix of a request that is the same as in
an earlier one: tools, then system prompt, then messages. Anything that
changes there costs the rest of the request at full price. So:

- The tool specs keep a fixed order and text. `lsp`, `ask_user` and
  `report` are appended in that fixed order; whether they are offered is
  decided once per turn, not per request. **(proposal)**
- The system prompt is built once per turn. The date in it is the date
  the session's first turn started, not today's, so a session that runs
  over midnight keeps its cache. **(proposal)** A change of skills or of a
  language server applies from the next turn, as today it applies from the
  next request.
- The history is only appended to, except for the trimming steps below.
- JSON is written with a fixed key order (it already is, through the
  types in `provider/types.rs`); a test pins it.

### Prompt caching in the anthropic adapter

The adapter marks up to three breakpoints with
`"cache_control": {"type": "ephemeral"}`: **(proposal)**

| Breakpoint | On | Why |
|------------|----|-----|
| 1 | The last tool spec | Tools and prompt change only between turns. |
| 2 | The last block of the system prompt | Same. |
| 3 | The last block of the last message | The next request of the turn reads everything before it from the cache. |

- The adapter decides alone; the shared `Request` type does not change.
  This answers `providers.md`, open question 4: yes, and in the adapter.
- A provider can turn it off with `cache = false` in its table, for a
  proxy that rejects the field. Default `true`.
- The cache lives five minutes. A turn usually sends its next request
  within seconds; a user who answers after ten minutes pays for one full
  write, as today without the cache.
- Writing the cache costs 25 % more than plain input, once per prefix.
  It pays for itself from the second request that reads it.

The openai-compatible adapter sends nothing new: OpenAI caches prefixes
of 1 024 tokens and more by itself, and local servers (llama.cpp,
vLLM) reuse a prefix they still hold in memory. The stable prefix is what
they need. The adapter reads `usage.prompt_tokens_details.cached_tokens`
when the server sends it.

### Counting cached input

`Usage` gets two fields, `cache_read_tokens` and `cache_write_tokens`;
`input_tokens` stays the sum of all input, so old sessions, old totals and
the context gauge mean what they did. **(proposal)**

- The session totals, `codeagent ls`, the status view and the global
  statistics (RFC 0014) show cached input next to input:
  `in 412k (cached 371k) · out 9k`.
- The run report (RFC 0027) has the same numbers in its JSON.
- A provider that reports nothing about the cache gives zeros.

### Trimming in steps

RFC 0008 replaces the result of a call with a placeholder once it is
`keep_results` answers old. Done in every request, that changes an old
message each time and breaks the cache from there. Instead, trimming
happens in steps: **(proposal)**

- Results are trimmed only when the untrimmed part of the history (the
  results not yet replaced) is larger than `trim_after` characters,
  default 60 000.
- Then every result older than `keep_results` answers is replaced at
  once, and the line of trimming stays there: later requests do not trim
  again until the untrimmed part grows past `trim_after` again.
- Between two steps the prefix does not change, so every request of that
  stretch reads it from the cache; one step costs one cache write.
- Which results are placeholders is a function of the history and the
  settings only, so a resumed session and a restarted server build the
  same requests.

This is a change to RFC 0008 before it is built: its design section,
config (`trim_after` joins `keep_results` and `min_chars`) and tests take
it over, and RFC 0008 links here. Everything else there stays: the
history, the saved session, the TUI and replay keep every result in full.

### Clean command output

Before the size cap is applied, `run_command` output goes through a
cleaner. It removes only what the model cannot use, and says so:

- ANSI escape sequences (colour, cursor movement) are removed. `TERM=dumb`
  already stops most of them; tools that ignore it still send some.
- A line overwritten with `\r` keeps only its last state, as a terminal
  would show it.
- A run of more than three identical lines keeps the first and a line
  `… same line 57 more times`.
- A run of more than 20 lines that differ only after a common word at the
  start (`Compiling`, `Downloading`, `Downloaded`, `Fresh`, `Checking`)
  keeps the first 3, the last 3 and a line `… 213 more "Compiling"
  lines`. **(proposal)** The list of words is fixed in the code, not
  configured.
- Error and warning lines are never collapsed, whatever they start with.
- The exit code line and the start-and-end cut of
  [Result size](../spec/tools.md#result-size) stay as they are, applied
  after the cleaning.

The TUI and `codeagent chat` still show the raw output, as they already
show more than the model gets.

### Reading an unchanged file again

When `read_file` is called with a path, offset and limit it was called
with before in this session, and:

- the file's size, modification time and content hash are the same as
  then, and
- that earlier result is still sent to the model in full (not trimmed,
  not cut by the size cap),

the result is one line: `unchanged since your read_file call in step 12;
the content above is current`. **(proposal)** In every other case the file
is read and returned as today. An edit or write through `codeagent` to the
file, or any change on disk, gives the full content again.

A partial overlap (a different offset or limit) reads in full; the rule
is only for an exact repeat, which is what models do.

### Measuring

A directory `bench/` holds tasks: a small fixed repository, a prompt, and
a check command that says whether the task was done (`cargo test`, a
`grep`, a diff against an expected file). `ci/bench.sh` runs each task
with `codeagent run --report=json` against a real model and writes, per
task: success, requests, input, cached input, output and price from the
model's configured prices. **(proposal)**

- Each step of the plan is measured before and after on the same model.
  A step is kept only when success does not fall and tokens do.
- The bench needs a key and costs money; it is run by hand, not in the
  pipelines.

## Dependencies

None. ANSI stripping is a small state machine in `src/tools/output.rs`;
the content hash uses the hash the file tools already use, or `std`'s
`DefaultHasher` if there is none. **(proposal)**

## Spec changes

- `spec/providers.md`: `Usage` gets `cache_read_tokens` and
  `cache_write_tokens`; "anthropic" gets the breakpoints and the `cache`
  model key; "openai-compatible" reads `cached_tokens`; open question 4
  is answered.
- `spec/agent-loop.md`, "Requests": a section "Stable prefix" (tools and
  prompt fixed per turn, the session's date). The trimming section of RFC
  0008 describes the steps.
- `spec/tools.md`: "run_command" describes the cleaning; "read_file" the
  unchanged-file result; "Result size" says the cap applies after the
  cleaning.
- `spec/config.md`: `cache` under the model keys; `trim_after` under
  `[history]` (with RFC 0008).
- `spec/sessions.md`: cached input in the totals.
- `readme.md`: the new keys and the cached column; "Release" is not
  touched. `bench/` gets its own short readme.

## Testing

- Anthropic encoding tests (expected JSON files): breakpoints on the
  last tool, the system prompt and the last message; none with
  `cache = false`; usage with cache fields parsed into the new fields.
- OpenAI-compatible fixtures with and without `cached_tokens`.
- Agent-loop tests with the fake provider, which records each request:
  in a turn of several requests, every request starts with the bytes of
  the one before (the prefix test); a skill change mid-turn changes the
  prompt only from the next turn.
- Trimming tests: no change of earlier messages between steps; a step
  replaces exactly the results older than `keep_results`; the same
  history gives the same requests after a resume.
- Cleaner tests, from recorded outputs of `cargo build`, `npm install`,
  `pytest` and `git clone`: escapes gone, `\r` lines resolved, runs
  collapsed, error and warning lines kept, the exit code line last.
- `read_file` tests: exact repeat of an unchanged file gives the short
  line; a change on disk, an edit, a trimmed or cut earlier result, and a
  different range each give the full content.
- Totals tests: old sessions without the new fields load with zeros.

## Implementation plan

1. **Count cached input**
   - Status: done
   - Files: `src/provider/types.rs`, `src/provider/anthropic.rs`,
     `src/provider/openai.rs`, `src/session/`, `src/command/ls.rs`, the
     status view, statistics, `src/report/`
   - Result: totals show cached input; nothing about requests changes yet.
   - Tests: usage parsing, totals, loading old sessions.

2. **Bench**
   - Status: in progress (script and tasks done; the baseline needs a key
     and is not run yet)
   - Files: `bench/`, `ci/bench.sh`, `bench/readme.md`
   - Result: a baseline per task on one model, written down in this RFC.
   - Tests: `ci/test.sh` checks the script's argument handling and output
     format with the fake provider.

3. **Stable prefix and anthropic caching**
   - Status: done
   - Files: `src/agent/mod.rs`, `src/agent/prompt.rs`,
     `src/provider/anthropic.rs`, `src/config/`
   - Result: requests in a turn share their prefix; Anthropic reads it
     from the cache.
   - Tests: prefix test, encoding tests, `cache = false`; bench.

4. **Clean command output**
   - Status: done
   - Files: `src/tools/run_command.rs`, `src/tools/output.rs`,
     `src/tools/tests.rs`
   - Result: command results without escapes, progress lines and runs.
   - Tests: cleaner tests from recorded output; bench.

5. **Unchanged file reads**
   - Status: done
   - Files: `src/tools/read_file.rs`, `src/agent/history.rs`
   - Result: an exact repeat read of an unchanged file is one line.
   - Tests: `read_file` tests above; bench.

6. **Trimming in steps (with RFC 0008)**
   - Status: done
   - Files: those of RFC 0008, plus `trim_after`
   - Result: RFC 0008 built with steps; long sessions stay small and
     cached.
   - Tests: RFC 0008's tests, trimming tests above; bench.

7. **Spec and readme**
   - Status: done
   - Files: the spec files above, `readme.md`, `rfcs/0008-…`
   - Result: the spec describes what was built; numbers from the bench in
     this RFC.
   - Tests: the existing tests keep passing.

Steps 3 to 6 do not depend on each other; each is measured on its own.

## Changes while building

Approved with the build ("implement rfc0028"); the open questions took the
proposals. Where the build differs from the design above:

- **`cache` is a provider key**, `[providers.x] cache = false`, not a
  model key: proxies that reject the field are per provider.
- **The date** in the prompt is the date the session was loaded, as
  before; the prompt was already built once per session, so it stays the
  same across requests without a change.
- **Unchanged re-reads** compare the result text with the earlier result
  of a call with the same arguments, instead of size, time and a hash of
  the file: equal text is exactly what makes the pointer lossless, and no
  file state has to be kept. The earlier result must stay whole in the
  next request (the trimming line is computed with the new answer), and a
  pointer whose target is trimmed later is rewritten to say so.
- **The cleaner** lives in `src/tools/clean.rs`; its words also include
  `Documenting`, `Installing` and `Unpacking`.
- **The bench** has no price column: the configuration has no prices.
  `$CODEAGENT_BENCH_TASKS` replaces `bench/tasks`, for the script's own
  tests in `tests/ci.sh`.
- **Cached input** shows in `/usage`, `/status`, the totals of `/stats`,
  `codeagent ls --json` and run reports; the per-model and per-day rows of
  `/stats` do not split it yet.
- **`[history]`** applies to sessions started after a change, like
  `[limits]`; RFC 0008 proposed the next request.
- **Prompt line:** one bullet tells the model about placeholders and
  pointers.

## Alternatives

- **Compaction.** Saves the most, but a model decides what to keep, and
  what it drops is gone for the rest of the session. Against the rule of
  this RFC; stays in `context.md`.
- **Lower caps and shorter reads by default.** Fewer tokens per call, but
  more calls, and the model sees less of a file or a build log at once.
  The cleaner removes the noise instead of the content.
- **Cache breakpoints chosen by the agent loop.** Would need a new field
  in the shared `Request` and every adapter would have to ignore or use
  it. Only Anthropic needs explicit marks today; the adapter knows its
  API best. Can move up if a second API needs it.
- **Trimming every request, as RFC 0008 drafts it.** Smaller requests,
  but each one breaks the cache from the newly trimmed result on. With
  cached input at a tenth of the price, a slightly larger cached request
  costs less than a smaller uncached one.
- **Diffs instead of full content on a changed re-read.** Saves tokens
  when a file changed a little, but the model must apply the diff in its
  head to an older copy that may be trimmed already. Not lossless in
  practice; left out.

## Open questions

1. The `trim_after` default of 60 000 characters: lower trims more
   often (more cache writes), higher sends more per request. The bench
   should decide; is 60 000 a good start?
2. The list of collapsible prefixes for command output: fixed in the code
   as proposed, or configurable under `[tools]`?
3. Should the session's date in the system prompt be the date of the
   first turn (proposed, stable cache) or of the current turn (correct
   date, one cache miss per day)?
4. Which model and which tasks for the bench: one Anthropic model and
   one local openai-compatible model, five to ten tasks from this
   repository's history?
5. Should a turn that sees a cache read of zero on its second request log
   a warning, so a broken prefix is found early?
