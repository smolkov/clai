# Reports from codeagent run

- RFC: 0027
- Status: done
- Milestone: none; an option of `codeagent run`
- Spec: [`spec/command.md`](../spec/command.md), "codeagent run";
  [`spec/tools.md`](../spec/tools.md), new "report";
  [`spec/agent-loop.md`](../spec/agent-loop.md), "Requests"

## Summary

`codeagent run --report` works on one task, an issue, and prints a report
instead of the model's last answer: as Markdown for people, or as JSON
with `--report=json` for scripts. The model hands in the result with a new
tool, `report`: a status, a summary, findings with a severity and a place,
and what is left to do. `codeagent` adds what it saw itself: every tool call of
the run, the changed files, the commands and their exit codes. Also,
`codeagent run` reads the task from a pipe without `-` when no words are given.

```sh
codeagent run --report "analyse this repository for security problems"
echo "Analyse this repository for security issues" | codeagent run --report
cat issue.md | codeagent run --report
codeagent run --report=json -C ~/ws/api < issue.md > report.json
```

## Motivation

- `codeagent run` prints the model's last answer as free text. A script cannot
  tell whether the task was solved, what was found and where, which files
  changed or which commands ran, without reading that text.
- Audits and reviews ("analyse this repository for security problems")
  need findings with a severity and a place, and a record of what the
  model looked at: which files it read and what it searched for. Without
  that, "no problems found" cannot be checked.
- Issues are written as files and sent through a pipe; `codeagent run` needs
  `-` for that, though without words a pipe can only be the task.
- `codeagent run` already has the rest: the agent in its own process, no
  server, answers without a person (`--yes`, `--no-ask`), tool lines on
  standard error, signals. A separate command would be a second name for
  the same code, so this is an option of `codeagent run`.

## Scope

In:

- `--report` and `--report=json` for `codeagent run`.
- Reading the task from a pipe without `-`.
- The `report` tool: offered only with `--report`, how the agent handles
  it, what happens when the model does not call it.
- The report: what the model gives, what `codeagent` adds from the events, the
  two formats.

Out:

- A session on the server, watching a run live with `codeagent attach`, or
  saving it. See "Open questions".
- Fetching issues from GitHub or other trackers: `gh issue view 12 | codeagent
  run --report` covers it. Writing the report back, commits, pull
  requests.
- Several tasks in one call.
- A command `codeagent issue`; it can come later as a short name, see "Open
  questions".

## Design

### How codeagent run works today

`src/command/run.rs`, in short:

1. `input_text` joins the words to the task; `-` reads standard input.
2. `Run::setup` resolves the workspace, loads the configuration, picks the
   model and its provider, turns on `--skill` skills, writes the system
   prompt and makes a language server pool for this run. The result is a
   `Setup`, from which an `Agent` is made: the same agent a session on the
   server runs.
3. `answering` decides who answers questions: `--yes` allows
   (`PolicyAsker::Allow`), `--no-ask` or no terminal denies
   (`PolicyAsker::Deny`), else the terminal asks (`LineAsker`).
4. `turn` runs `agent.run_turn(task, sink, asker, cancel)`. Each event goes
   through an `EventSink`; the `Renderer` writes tool lines to standard
   error and keeps the last answer. `Ctrl-C`, `SIGTERM` and `SIGHUP` cancel
   the turn.
5. On `done` the last answer goes to standard output. The agent is dropped;
   nothing is saved.

With `--report`, steps 2 to 4 stay as they are. The task gets a frame (1),
the agent gets the `report` tool (2), a collector sees the events next to
the `Renderer` (4), and the report is printed instead of the answer (5).
Without `--report`, `codeagent run` behaves as today, apart from the pipe.

### Options

```sh
codeagent run [--model PROVIDER/MODEL] [--yes | --no-ask] [-C DIR] [--skill NAME]...
         [--report[=text|json]] [TASK...]
```

| Option | Meaning |
|--------|---------|
| `--report` | Print a report instead of the last answer: Markdown. |
| `--report=json` | The report as one JSON object. `--report=text` is the same as `--report`. |

The value needs the `=`: in `codeagent run --report json`, `json` is a word of
the task. **(proposal)** The short form `codeagent TASK...` has no `--report`,
as it has no `--skill`.

### The task from a pipe

- Without words, `codeagent run` reads the task from standard input when it is
  not a terminal. On a terminal without words: usage error (exit `2`),
  `codeagent: no task: give it as words or on standard input`. This is new for
  every `codeagent run`, with or without `--report`. **(proposal)**
- `-` as the only word reads standard input, as today.
- With words, standard input is not read, so a script with standard input
  open but silent does not hang.
- The text is trimmed. Empty: `the task is empty`, exit `2`, as today.
  Over 1 MiB: `the task is longer than 1 MiB`, exit `2`. Not UTF-8:
  `the task on standard input is not UTF-8`, exit `2`. **(proposal)**
- The task from standard input means no one can answer on the terminal, as
  for `-` today.

`codeagent` without a command and without words still opens the TUI; the pipe
rule is only for `codeagent run`.

### Questions

As today: with the task from words and a terminal on standard input,
questions are asked there; otherwise no one can answer, a tool call that
would ask is denied (allowed with `--yes`), a limit question is "stop",
and `ask_user` is not offered.

With the default permissions `read_file`, `list_files`, `search_files` and
`lsp` are allowed, and `edit_file`, `write_file` and `run_command` ask. So
`codeagent run --report "analyse this repository for security problems"` from a
script reads and searches the whole workspace without a question, and
changes nothing. To let it fix things, the user gives `--yes`, answers on
the terminal, or allows the tools in `[permissions]`.

### The frame of the task

With `--report`, the task goes to the model inside a short fixed frame, in
the code as `src/report/work.md`:

```
Work on this task in the workspace, to the end.

- Use the tools to look at the code itself; do not guess. Read the files
  that matter, search for the patterns the task is about.
- When the task asks for changes, make them and check them (build, tests)
  if you can.
- Questions may not be answered; when one is not, decide yourself and go
  on.
- When you are done, call the report tool once with the result, then end
  with a short answer.

The task:

<task>
…the task text…
</task>
```

The tags mark where the task ends, so the frame is not read as part of it.
**(proposal)**

### The report tool

The model hands in the result with a tool call, so the provider checks the
arguments against a schema and `codeagent` does not parse JSON out of text.

```json
{
  "status": "partly_solved",
  "summary": "Found two problems; fixed the path check, the other needs a decision.",
  "findings": [
    {
      "severity": "high",
      "title": "Path check follows links",
      "file": "src/tools/paths.rs",
      "line": 41,
      "detail": "A link inside the workspace can point outside it; resolve it first."
    }
  ],
  "follow_ups": ["Decide whether extra_dirs may hold links."]
}
```

| Argument | Required | Meaning |
|----------|----------|---------|
| `status` | yes | `solved`, `partly_solved` or `not_solved`. |
| `summary` | yes | What was done and the outcome, at most 4 000 characters. |
| `findings` | no | Problems or facts the task asked for, at most 200. |
| `severity` | yes | `critical`, `high`, `medium`, `low` or `info`. |
| `title` | yes | One line, at most 200 characters. |
| `file` | no | Path relative to the workspace. |
| `line` | no | Line number, from 1. |
| `detail` | no | What it is, why it matters, how to fix it; at most 4 000 characters. |
| `follow_ups` | no | What is left to do, one item each, at most 50. |

- **Offered** only with `--report`: `Agent::offer_report` turns it on,
  and `Agent::tool_specs` adds the spec then, as it adds `lsp` and
  `ask_user`. Sessions on the server and `codeagent run` without
  `--report` never see it.
- **Handled** by the agent, like `ask_user`: not permission-checked, not a
  file or command action. A valid call stores the report in the agent and
  gives the result `report saved; end with a short answer`. A wrong call
  (missing `status`, an unknown severity, a text too long) is an error
  result naming the problem, and nothing is stored, so the model can call
  it again. A later valid call replaces the report. **(proposal)**
- **Shown** as a tool line: `✓ report partly solved — 1 finding`.
- **Not called.** When the turn ends with `done` and no report is stored,
  `codeagent run` runs one more turn with the message `Call the report tool now
  with the result of your work on the task; use no other tools.` Without a
  report after that, the report gets `status` `unknown`, the last answer of
  the work as `summary`, and no findings; a note on standard error says
  so. **(proposal)**

### The tool calls in the report

Every tool call of the run is in the report, in order, except `report`
itself. This is the record of what the model looked at and did, from the
events, not from the model:

| Field | Value |
|-------|-------|
| `tool` | `read_file`, `search_files`, … |
| `summary` | The call's summary as on the tool line: `src/tools/paths.rs (lines 1–120)`, `unsafe in src/**/*.rs`. |
| `state` | `done`, `error`, `denied` or `cancelled`. |
| `result` | The short result of the tool line: `120 lines`, `exit code 101`, `denied: needs permission, and no one can answer`. |

From the same calls `codeagent` adds:

| Field | From |
|-------|------|
| `changes` | `edit_file` and `write_file` calls with state `done`: the path and `edited` or `written`, each path once, in order. |
| `commands` | `run_command` calls with state `done` or `error`: the command and its exit code. |
| `files_read` | The paths of `read_file` calls with state `done`, each once, in order. |
| `counts` | The number of calls by state. |

A collector in the run's `EventSink`, next to the `Renderer`, gathers
these.

### Formats

**JSON** (`--report=json`): one object, then a newline. Paths are relative
to the workspace when inside it, else absolute. Fields may be added within
a `version`; renaming or removing one needs a new `version`.
**(proposal)**

```json
{
  "version": 1,
  "workspace": "/home/u/ws/rust/codeagent",
  "model": "anthropic/sonnet",
  "outcome": "done",
  "status": "partly_solved",
  "summary": "Found two problems; fixed the path check, the other needs a decision.",
  "findings": [
    {"severity": "high", "title": "Path check follows links",
     "file": "src/tools/paths.rs", "line": 41,
     "detail": "A link inside the workspace can point outside it; resolve it first."}
  ],
  "follow_ups": ["Decide whether extra_dirs may hold links."],
  "changes": [{"path": "src/tools/paths.rs", "kind": "edited"}],
  "commands": [{"command": "cargo test paths", "exit_code": 0}],
  "files_read": ["src/tools/paths.rs", "src/permissions/mod.rs"],
  "tool_calls": [
    {"tool": "search_files", "summary": "canonicalize in src/**/*.rs",
     "state": "done", "result": "12 lines"},
    {"tool": "read_file", "summary": "src/tools/paths.rs (lines 1–120)",
     "state": "done", "result": "120 lines"},
    {"tool": "edit_file", "summary": "src/tools/paths.rs (line 41)",
     "state": "done", "result": "1 change"},
    {"tool": "run_command", "summary": "cargo test paths",
     "state": "done", "result": "test result: ok. 9 passed"}
  ],
  "counts": {"done": 4, "error": 0, "denied": 0, "cancelled": 0},
  "usage": {"input_tokens": 45100, "output_tokens": 3200, "requests": 9},
  "duration": 184
}
```

`outcome` is how the work turn ended: `done`, `limit` or `failed`.
`duration` is in seconds.

**Text** (`--report`): Markdown, for a terminal, a file or a ticket
comment, without colour. Empty sections are left out; the tool calls are
one line each, as on standard error. **(proposal)**

```markdown
# Partly solved

~/ws/rust/codeagent · anthropic/sonnet · 3 min · 45.1k in, 3.2k out

Found two problems; fixed the path check, the other needs a decision.

## Findings

- **high** Path check follows links — `src/tools/paths.rs:41`
  A link inside the workspace can point outside it; resolve it first.

## Changes

- edited `src/tools/paths.rs`

## Follow-ups

- Decide whether extra_dirs may hold links.

## Tool calls

- ✓ search_files canonicalize in src/**/*.rs — 12 lines
- ✓ read_file src/tools/paths.rs (lines 1–120) — 120 lines
- ✓ edit_file src/tools/paths.rs (line 41) — 1 change
- ✓ run_command cargo test paths — test result: ok. 9 passed
```

### Progress

Standard error gets what `codeagent run` writes there today. Standard output
gets only the report.

### Ends, signals and exit codes

| The work turn | Without `--report` (today) | With `--report` | Exit |
|---------------|----------------------------|-----------------|------|
| `done` | the last answer | the report | `0` |
| `limit` | nothing | the report, `outcome` `limit`, with the model's report if it gave one | `1` |
| `failed` (provider error, after retries) | nothing | the report: `status` `unknown`, the error as `summary` | `1` |
| cancelled (the `c` answer) | nothing | nothing | `1` |

- The model's `status` does not change the exit code: `not_solved` with
  outcome `done` exits `0`; a script reads `status`. **(proposal)**
- `Ctrl-C` exits `130`, `SIGTERM` and `SIGHUP` exit `1`, with no report, as
  today.
- Usage errors exit `2` before the model is asked, as today, and also for
  `--report=xml` and for no task on a terminal.

### Modules

- `src/command/run.rs`: the option, the pipe rule, the frame, the
  fallback turn, a collector in the sink, printing the report.
- `src/report/mod.rs`: `Report`, `Finding`, `Status`, `Outcome`,
  `ToolCall` (serde types), `Collector` (facts from events),
  `render_text`.
- `src/report/work.md`: the frame of the task.
- `src/tools/report.rs`: the `report` tool: spec and argument check, pure
  like `src/tools/ask_user.rs`. The agent keeps the report and handles the
  call in `src/agent/mod.rs`, like `ask_user`.

## Spec changes

- `spec/command.md`, "codeagent run": the syntax with `--report`, the pipe
  rule, the report, its formats, the exit table with `--report`.
- `spec/tools.md`: a row and a section "report", offered only with `codeagent
  run --report`.
- `spec/agent-loop.md`, "Requests": `report` among the tools that are only
  sometimes sent.
- `spec/architecture.md`: the new modules.

## Testing

- `src/tools/report.rs`: valid calls, each wrong argument, the spec.
- `src/report`: the collector over a list of events (changes once per
  path, commands with exit codes, files read, counts, `report` left out),
  `render_text` for a full and an empty report, the JSON shape.
- Input: words, `-`, a pipe without words, a terminal without words, empty
  and too long text; `--report json` takes `json` as a task word.
- Integration tests with the test provider:
  - a run that reads and searches, then calls `report`: exit `0`; the JSON
    has the findings and every tool call;
  - an edit and a command without `--yes` and without a terminal: denied,
    in `tool_calls` with state `denied`, no `changes`; with `--yes`: done,
    in `changes` and `commands`;
  - no `report` call, then one in the fallback turn; no call at all:
    `status` `unknown`, a note, exit `0`;
  - a wrong `report` call, then a right one;
  - a limit: `outcome` `limit`, exit `1`; a provider error: `status`
    `unknown`, exit `1`;
  - without `--report`: the output of today, and the model gets no
    `report` tool.

## Implementation plan

1. **The task from a pipe**
   - Status: done
   - Files: `src/command/run.rs`, `src/command/common.rs`
   - Result: `echo task | codeagent run` works; a terminal without words is a
     usage error.
   - Tests: input tests; existing `codeagent run` tests.

2. **The report tool**
   - Status: done
   - Files: `src/tools/report.rs`, `src/agent/mod.rs`, `src/text/events.rs`
   - Result: an agent with `report: true` offers the tool, checks calls and
     keeps the report; others are unchanged.
   - Tests: the unit tests of `src/tools/report.rs`; existing agent tests.

3. **Report and collector**
   - Status: done
   - Files: `src/report/mod.rs`, `src/lib.rs`
   - Result: a `Report` from the stored report and the events; both
     formats render.
   - Tests: the unit tests of `src/report`.

4. **The option**
   - Status: done
   - Files: `src/command/run.rs`, `src/report/work.md`
   - Result: `codeagent run --report` works as designed.
   - Tests: the integration tests above.

5. **Spec and docs**
   - Status: done
   - Files: `spec/command.md`, `spec/tools.md`, `spec/agent-loop.md`,
     `spec/architecture.md`, `readme.md`, `CHANGELOG.md`, `todo.md`, this
     RFC
   - Result: the spec describes `--report`.

## Alternatives

- **A command `codeagent issue`.** It would be `codeagent run` with the report under
  another name: the same setup, questions, progress and signals. One
  option keeps one command and one place in the spec. A short name can be
  added later if it is wanted.
- **A session on the server.** The run would be saved, could be watched
  with `codeagent attach` and continued later. It needs the server, answers
  questions over the protocol, and the `report` tool would have to exist
  for server sessions too. Most such runs come from scripts where no one
  watches; `codeagent run` is built for that.
- **The report as JSON in the final answer, parsed by `codeagent`.** No new
  tool, but models often wrap or break the JSON, and the provider does not
  check it. With a tool call the schema is checked and a wrong call is
  answered with the problem.
- **Exit `1` for `not_solved`.** The model's view of its own work is not a
  failure of the command; a script can check `status`.

## Answers

The proposals were taken as the answers when the RFC was built:

1. Words and standard input are not combined: with words, standard input
   is not read.
2. No `--save` yet; it is in `todo.md`.
3. `not_solved` with outcome `done` exits `0`.
4. No `codeagent issue`; `alias issue='codeagent run --report'` does it.
5. The tool calls carry their summary and short result, not their
   arguments.

## Notes from building

- The `report` tool lives in `src/tools/report.rs`, next to `ask_user`,
  as a pure check; the plan had it in `src/agent`. The agent stores the
  report and turns the tool on with `Agent::offer_report`, so `Setup` and
  its users did not change.
- The task is read from standard input with a limit of 1 MiB + 1 byte,
  so a large pipe is not read into memory; the message therefore says
  "longer than 1 MiB" and not the size.
- The JSON is indented, one object, then a newline.
- A report after a limit says `The run stopped at its limit.` in the text
  form, after a failed run `The run failed.`.
- The test `run_needs_task` of `src/cli.rs` became
  `run_takes_the_task_from_a_pipe`: `codeagent run` without words parses now;
  the command checks for a terminal.

## Open questions

None left; see "Answers".

## Questions that were open

1. Words and standard input together: should `cat issue.md | codeagent run
   --report "focus on the auth code"` send both, the words as an
   instruction and standard input as the task? Proposal: not in the first
   version.
2. Keep a record of the run: a `--save` option that hands the history to
   the server as `codeagent import` does, so `codeagent attach` can show it later?
   Proposal: later.
3. Exit code for `not_solved`: `0` (proposal), `1`, or an option
   `--fail-unsolved`?
4. A short name `codeagent issue` for `codeagent run --report`? Proposal: not now;
   an alias in the shell does it (`alias issue='codeagent run --report'`).
5. Should the tool calls in the JSON carry the arguments too (the full
   `search_files` pattern, the `read_file` range), not only the summary?
   Proposal: the summary only; the arguments can be large.
