# Architecture

How `clai` is divided into parts, what each part owns, and how the parts talk
to each other. This file describes structure and data flow. Behaviour is
described in the topic files it links to, and the code layout (modules,
crates, dependencies) in [`implementation.md`](../implementation.md). If this
file and a topic file disagree, the topic file wins.

Statements marked **(assumption)** need a decision from the user.

## Overview

`clai` is one binary that runs in one of three roles:

| Role | Started by | Lives | Does |
|------|------------|-------|------|
| Client | `clai`, `clai attach`, `clai chat`, `clai ls`, … | As long as the command | Talks to the server; shows sessions and questions; sends user input and answers. |
| Server | The first client that needs one, or `clai server` | Until idle for 60 s | Owns all sessions, runs their agent loops and tools, saves them and resumes saved ones. |
| One-shot | `clai run`, `clai TASK...` | One task | Runs one temporary session in its own process, without a server. |

```
  clients (one process each)                      server (one per user)
 ┌──────────────────────────┐                  ┌──────────────────────────────────────┐
 │ TUI            clai chat │                  │ session registry                     │
 │ clai ls/send/kill/new    │   Unix socket    │   ┌─ session "clai" ───────────────┐ │
 │ web UI bridge (later)    │◀────────────────▶│   │ state, history, session rules  │ │
 └──────────────────────────┘  versioned JSON  │   │ agent loop                     │ │
                               commands ▶      │   │   ├─ context                   │ │
                               ◀ events        │   │   ├─ provider ─ adapter ───────┼─┼──▶ provider API
                                               │   │   ├─ permissions               │ │     (HTTPS / local)
                                               │   │   └─ tools ────────────────────┼─┼──▶ workspace files,
                                               │   └────────────────────────────────┘ │     sh -c
                                               │   ┌─ session "docs" ─┐  …            │
                                               │   └──────────────────┘               │
                                               │ config (loaded, watched, reloaded)   │
                                               └──────────────────────────────────────┘

  one-shot: clai run = client output + one session + agent loop, in one process
```

## Components

| Component | Owns | Uses | Spec |
|-----------|------|------|------|
| Config | The loaded user and project configuration; validation; reloading. | — | [`config.md`](config.md) |
| Provider | Shared request, message, event and error types; retries. | Config | [`providers.md`](providers.md) |
| Adapters | Encoding and decoding for one API type (`anthropic`, `openai-compatible`). | Provider types, HTTP | [`providers.md`](providers.md), "Adapters" |
| Tools | The six tools: arguments, execution, results, question details. | Permissions (per-entry checks), file system, `sh` | [`tools.md`](tools.md) |
| Permissions | Rule matching, path resolution, command splitting, the decision and its reason. | Config | [`config.md`](config.md), "Permissions" |
| Context | System prompt, project instructions, history size and truncation. | Config | `context.md` (planned) |
| Agent loop | One turn: requests, streaming, tool calls, limits, cancellation, questions. | Provider, tools, permissions, context | [`agent-loop.md`](agent-loop.md) |
| Session | State (`idle`, `working`, `waiting`), history, model, session rules, open question, event log, message queue. | Agent loop | [`sessions.md`](sessions.md) |
| Server | Socket, session registry, start on demand, idle exit, config reload. | Session, protocol | [`sessions.md`](sessions.md), "Server" |
| Protocol | Client–server messages, framing, version handshake. | — | [Protocol](#protocol) |
| Client | Connecting, starting the server, request/response, event stream. | Protocol | [`sessions.md`](sessions.md) |
| TUI | View state: layout, scroll, expanded entries, input editor, prompts. | Client | [`tui.md`](tui.md) |
| Commands | Parsing the command line and running one command. | Client, config, commit | [`command.md`](command.md) |
| Commit | Collecting staged git changes and asking a model for a message. | Provider, config, permissions | [`commit.md`](commit.md) |

### Layers

Each component uses only components below it; nothing calls upwards.

```
 commands · TUI · clai chat             (clients: show and send)
 client · protocol                      (transport)
 server · session                       (state and lifetime)
 agent loop                             (one turn)
 provider · tools · permissions · context
 adapters · config
```

Consequences:

- **The agent loop knows no transport and no client.** It takes user
  messages, answers and cancellation as input and emits session events as
  output. The server and the one-shot mode drive the same loop.
- **Tools and permissions know no session.** They get the call, the
  workspace, the rules and a cancellation signal, and return a result or
  question details.
- **Adapters know no tools.** Tools reach them only as `ToolSpec` values
  and tool calls in the shared types (see [`providers.md`](providers.md)).

## Project tree

Where the components live in the repository. Entries marked *planned* do not
exist yet; they are added by the milestones in
[`implementation.md`](../implementation.md), which also lists each module's
content. When files are added, moved or removed, this tree is updated in the
same change.

```
clai/
├── Cargo.toml, Cargo.lock      one crate, one binary `clai`
├── clippy.toml                 lint settings (never-crash rules)
├── readme.md                   user-facing overview and status
├── agent.md                    rules for the coding agent on this repository
├── implementation.md           modules, dependencies, milestones
├── todo.md                     progress per milestone
│
├── spec/                       the specification (this book)
│   ├── book.toml, SUMMARY.md   mdBook configuration and table of contents
│   ├── readme.md               introduction, terms, index, open questions
│   ├── architecture.md         this file
│   ├── command.md, tui.md, sessions.md, commit.md
│   ├── config.md, providers.md, tools.md, agent-loop.md
│   └── context.md              planned
│
├── src/
│   ├── main.rs                 entry point: parse, run, exit code
│   ├── lib.rs                  crate root; the modules below
│   ├── cli.rs                  command-line arguments (command.md)
│   ├── complete.rs             values for shell completion
│   ├── command/                one file per command, with its arguments and `run`
│   │   ├── mod.rs              `Command`: name, run
│   │   └── run.rs, new.rs, attach.rs, ls.rs, kill.rs, send.rs, export.rs, import.rs,
│   │       models.rs, commit.rs, server.rs, completion.rs
│   │   └── chat/               clai chat: mod.rs (options, --list, script form),
│   │                           talk.rs (conversation), questions.rs (--questions)
│   ├── commit/                 commit messages (commit.md)
│   │   ├── mod.rs              prepare, request, clean, describe
│   │   ├── git.rs              running git: root, state, staged files, subjects
│   │   ├── changes.rs          size budget and protected files
│   │   └── prompt.rs, prompt.md   agent file, system prompt, user message
│   ├── config/                 Config (config.md)
│   │   ├── mod.rs              files, precedence, validation
│   │   ├── file.rs             TOML files as written
│   │   ├── errors.rs           errors with file:line
│   │   ├── permissions.rs      loading and merging `[permissions]`
│   │   ├── secret.rs           API keys, never shown
│   │   ├── tui.rs              `[tui]` and `[themes]`
│   │   └── example.toml        example printed when no config exists
│   ├── provider/               Provider and adapters (providers.md)
│   │   ├── mod.rs              `Provider` trait
│   │   ├── types.rs            shared request, message, event types
│   │   ├── error.rs            `ProviderError`
│   │   ├── remote.rs           key, retries, streaming, cancellation
│   │   ├── http.rs             HTTP client, timeouts, status codes
│   │   ├── sse.rs              server-sent events parser
│   │   ├── anthropic.rs        adapter `anthropic`
│   │   ├── openai.rs           adapter `openai-compatible`
│   │   └── fake.rs             scripted provider for tests
│   ├── permissions/            Permissions (config.md, "Permissions")
│   │   ├── mod.rs              exports
│   │   ├── check.rs            checking order, decisions and reasons
│   │   ├── workspace.rs        workspace, extra dirs, resolving paths
│   │   ├── path_pattern.rs     path patterns
│   │   ├── command_pattern.rs  command patterns, wrappers, pipelines
│   │   ├── shell.rs            splitting command lines into parts
│   │   └── session.rs          session rules, proposals for "always"
│   ├── tools/                  Tools (tools.md)
│   │   ├── mod.rs              specs, `prepare`, `PreparedCall::run`
│   │   ├── read_file.rs, list_files.rs, search_files.rs,
│   │   │   edit_file.rs, write_file.rs, run_command.rs
│   │   ├── question.rs         details of permission questions
│   │   ├── walk.rs             directory walking for list and search
│   │   ├── text.rs             binary files, safe writing, similar names
│   │   └── output.rs           results and their size limits
│   ├── context/                system prompt, history, truncation        planned
│   ├── agent/                  Agent loop (agent-loop.md)
│   │   ├── mod.rs              `Agent::run_turn`: requests, tool calls, limits
│   │   ├── stream.rs           collecting a streamed answer
│   │   ├── question.rs         questions, answers, `Asker`, `PolicyAsker`
│   │   ├── events.rs           session events, `EventSink`
│   │   ├── history.rs          messages and full tool outputs
│   │   ├── prompt.rs           system prompt, today's date
│   │   ├── system_prompt.md    system prompt text
│   │   └── tests.rs            turns against a scripted provider
│   ├── session/                Session (sessions.md)
│   │   ├── mod.rs              the actor: state, queue, event log, questions
│   │   ├── questions.rs        the open question, waiting answers, first one wins
│   │   ├── store.rs            saved sessions: files, loading, repair, bundles
│   │   └── outbox.rs           messages to one client, slow-client limit
│   ├── protocol/               messages, framing, handshake; tests.rs
│   ├── server/                 Server (sessions.md, "Server")
│   │   ├── mod.rs              socket, connections, idle exit, reload
│   │   ├── registry.rs         all sessions, running and saved, by id and name
│   │   ├── requests.rs         requests: create, resume, kill, export, import
│   │   ├── paths.rs            runtime directory and its mode
│   │   └── tests.rs            the server in-process with scripted providers
│   ├── client/                 connect, start the server, requests, events
│   │   └── connection.rs       Connection: reader and writer tasks, for the TUI
│   ├── tui/                    full-screen interface (tui.md)
│   │   ├── mod.rs              run: connect, attach, terminal, event loop
│   │   ├── terminal.rs         setup, restore, panic hook; Guard restores on Drop
│   │   ├── app.rs              App: all view state; update(Input) -> Vec<Effect>
│   │   ├── conversation.rs     entries from events, line cache per entry
│   │   ├── markdown.rs         Markdown as lines
│   │   ├── wrap.rs             styled text wrapped to terminal cells
│   │   ├── editor.rs           the input editor
│   │   ├── prompt.rs           the open question and the keys that answer it
│   │   ├── picker.rs           the model picker: models, filter, selected row
│   │   ├── keys.rs             key to action, by focus
│   │   ├── style.rs            styles and symbols (Unicode or ASCII, NO_COLOR)
│   │   ├── view/               drawing only: conversation, prompt bar, input,
│   │   │                       status and session rows, popups, model picker
│   │   ├── snapshots/          expected frames (insta)
│   │   └── tests.rs            state tests and frames with TestBackend
│   ├── text/                   plain-text output for clai run and clai chat
│   │   ├── events.rs           tool lines, notices, the final answer
│   │   ├── ansi.rs             colours, used when the terminal has them
│   │   ├── conversation.rs     a conversation as streamed text (clai chat)
│   │   ├── question.rs         questions as text, colour, pager hint
│   │   ├── dialog.rs           answering a question line by line
│   │   ├── asker.rs            `LineAsker`: answers read as lines
│   │   └── tests.rs
│   ├── output.rs               writing command results to standard output
│   ├── log.rs                  logging to a file, the state directory
│   ├── runtime.rs              async runtimes of the commands
│   ├── sync.rs                 locking without panics
│   └── error.rs                crate error type, `Error::io`
│
└── tests/                      tests of the binary and live tests
    ├── cli.rs, completion.rs, models.rs
    ├── live.rs                 against real APIs, ignored by default
    ├── run.rs                  clai run against a local server
    ├── server.rs               the server and session commands as processes
    ├── configs.rs              the sample configurations load or fail as expected
    ├── configs/                sample configurations: valid/, invalid/ (with .err), project/
    └── fixtures/               recorded SSE streams per adapter
        ├── anthropic/
        └── openai/
```

Unit tests live next to their code (`#[cfg(test)]`, `config/tests.rs`,
`provider/tests_http.rs`); `tests/` holds tests that run the `clai` binary or
need the network. Build output goes to `target/`, the spec book to
`target/spec-book/`.

## State and ownership

| State | Owner | Others see |
|-------|-------|------------|
| Configuration | Server (one-shot: its process) | A snapshot, taken by each session when it is created and replaced on reload for new requests. |
| History, model, session rules, open question, limits | The session | Events; the full event log on attach. Saved to `$XDG_STATE_HOME/clai/sessions/` as it changes (see [`sessions.md`](sessions.md), "Persistence"). |
| Session list and names | Server registry | `clai ls`, the TUI session list. |
| Scroll position, expanded entries, unsent input, theme | Each TUI | Nobody. |
| API keys | Provider, in memory, after first use | Nobody; never logged or sent to clients. |

- A session is the only writer of its state. Clients send it commands; it
  decides, changes its state, and publishes events. Two clients therefore
  never disagree about a session; at most one of them is behind by the
  events still in flight.
- A client holds no session state that it cannot rebuild from the event
  log. Detaching or a crashed client loses nothing of the session.
- Sessions share nothing but the configuration and the server. Session rules
  never pass from one session to another (see [`sessions.md`](sessions.md)).

## Protocol

Clients and the server exchange JSON messages over the Unix socket, one
message per line (at most 16 MiB). Every message is an object with a
`type`. The first message in each direction is `hello` with the protocol
version (now `6`); on a mismatch the server sends `error` and closes the
connection, and the message tells the user to restart the server with
`clai server stop` (see [`sessions.md`](sessions.md), "Server"). The code
is in `src/protocol/`.

Message kinds:

| Direction | Kind | Content |
|-----------|------|---------|
| client → server | `hello` | Protocol version; client kind (`tui`, `chat`, `cli`, `bridge`) and name (e.g. `tui pts/3`, `clai chat`), which is shown as who answered; whether it answers questions. |
| client → server | `request` | An id and a command: `new`, `list`, `kill`, `rename`, `attach`, `detach`, `send` (message), `set_model`, `cancel`, `answer`, `export`, `import`, `status`, `session_status`, `global_stats`, `models`, `stop`. Each gets exactly one `response`. Sessions are named by their current name; `attach`, `send` and `set_model` resume a saved session. |
| server → client | `hello` | Protocol version and the server's PID. |
| server → client | `response` | The request's id and its `result`: `ok` with a payload (`done`, a session, the session list, the status, a session's status, the global statistics, the models; a session is its id, name, workspace, model, state, created and last updated time, turns, queued messages, open question and token totals) or `error` with a message, marked `usage` when the request itself was wrong (the command then exits `2`). |
| server → client | `event` | Something that happened in an attached session, with the session's id, and `replay` when it is part of the log sent on attach (see below). |
| server → client | `notice` | Server-wide: `config_reloaded`, `config_rejected` (with the errors), `session_created`, `stopping`. |
| server → client | `error` | The connection cannot go on: wrong version, a message that is not valid. |

An answer is the `answer` request: session, question id, choice, and the
rule or text the choice needs (see [`sessions.md`](sessions.md),
"Answers"). Its response comes when the session has decided: `ok` if the
answer counted, otherwise an error with the reason (`already answered by
tui pts/3`, a rule that does not allow the call); the question then stays
open.

The `attach` response carries the session's info, including its open
question; then come the log and the live events, in this order.

Session events, in the order they happen:

| Event | Content |
|-------|---------|
| `state` | `idle`, `working` or `waiting`, and why. |
| `user_message` | A message as the session received it (also from other clients). |
| `text_delta`, `text_done` | Streamed model text. |
| `tool_call` | A tool call and its state: `pending`, `waiting`, `running`, `done`, `error`, `denied`, `cancelled`; with output as it arrives, and once its `details` (as a permission question shows them: the diff of an edit, the command and its directory). |
| `question`, `question_closed` | A question with all its details, and its answer with the client that gave it (see [`sessions.md`](sessions.md), "Questions"). |
| `usage` | Tokens of a request, context size, and the request's `provider/model` (none in a log of an older `clai`). The session sums them; the session info has the totals as `usage`. |
| `error` | A provider or session error that ended a request or turn. |
| `retry` | A model request failed and is tried again: the attempt (from 1), `max_retries` and the reason. |
| `turn_end` | The turn ended: done, cancelled, limit reached or failed. |
| `model_changed`, `renamed`, `ended` | Session settings and lifetime; `model_changed` has the new model and, as `from`, the old one (none in a log of an older `clai`); `ended` has a reason: `killed`, `failed`, `server stopped`. |

The `models` request answers with the models of the server's configuration, in
config order (`model`, `provider`, `name`, `id`, `context_window`, `default`,
`key` as in `clai models --json`, without running `api_key_cmd`), the path of
the user config file, and `default_from` (`user`, `project`, `env`): what sets
the default. It may name a `workspace`; then the default is the one new
sessions there get, with that workspace's project file. Both fields are
optional on the wire, so the protocol version stays the same.

Before a request uses the configuration, the server compares the user file's
stamp with the one it read and reloads it if it changed (with
`config_reloaded`), as the 2 s poll does; so a file saved just now counts at
once.

On `attach`, the session first sends its event log, then live events, so a
client that attaches late shows the same conversation as one that was there
from the start (see [`tui.md`](tui.md), "Switching sessions"). The log is
kept in memory, whole, while the session runs, and on disk without the
streamed pieces; a resumed session replays it from disk.

The `session_status` request names a session, running or saved, and answers
with its full status: the session info; its statistics, counted from the log
(tokens, context size, turns by how they ended, requests, retries, errors, tool
calls per tool and final state, answers per kind and per client, model
changes, time in `working`, last activity, and the running turn's progress);
the attached clients; the session directory as shown and its size; the extra
directories of the workspace and whether they exist; the model as configured
now (with API, base URL, `temperature`, `max_output_tokens`, `max_retries`), or
none when it is no longer configured; the limits in effect for the workspace;
the server's clock and its `status`. A saved session is counted from its
`events.jsonl` and stays saved (see [`sessions.md`](sessions.md),
"Statistics").

The `global_stats` request has a `range` (`7d`, `30d`, `all`) and the client's
`offset` from UTC in seconds, and answers with statistics over the logs of all
saved sessions (see [`sessions.md`](sessions.md), "Statistics over all
sessions"): the range as its first and last day (days since 1970 in the
client's time zone); the sessions active in the range, saved and running; the
totals (turns by how they ended, requests, retries, errors, tokens, time in
`working` and stretches without a time); per model its name, whether it is
configured now, sessions, requests, input and output tokens, most tokens first;
the input and output tokens of each day of the range, empty days included;
the tokens without a date; the logs skipped; and the server's clock. Weeks and
months for a chart are made by the client. It came with protocol 7.

The session list has the state `saved` besides `idle`, `working` and
`waiting`: a session on disk, not loaded. `status` reports the sessions,
how many run, and how many are busy (`working` or `waiting`).

All texts in messages are UTF-8 and carry no colour or layout; clients
render them. A client that does not know an event kind of the same protocol
version is a bug, not an expected case; a new kind needs a new version.

## A turn

```
 client             session              agent loop            provider        tools/permissions
   │ send(message)     │                      │                     │                  │
   │──────────────────▶│ state: working       │                     │                  │
   │◀── user_message ──│─── run turn ────────▶│── request ─────────▶│                  │
   │◀── text_delta ────│◀── events ───────────│◀── stream ──────────│                  │
   │                   │                      │── check call ──────────────────────────▶│
   │                   │                      │◀── ask + details ───────────────────────│
   │◀── question ──────│◀── question ─────────│  (waits)            │                  │
   │── answer ────────▶│── decision ─────────▶│── run tool ────────────────────────────▶│
   │◀── tool_call ─────│◀── events ───────────│◀── result ──────────────────────────────│
   │                   │                      │── request (with result) ─▶│             │
   │                   │        …  until an answer without tool calls  …   │             │
   │◀── turn_end ──────│◀── done ─────────────│                     │                  │
   │                   │ state: idle          │                     │                  │
```

- Calls that are allowed run without a question; denied calls get an error
  result without running. Either way, every tool call gets exactly one
  result before the next model request (see [`tools.md`](tools.md)).
- Cancellation goes down the same path: the session cancels the turn, the
  agent loop stops the stream or the running tool (`SIGTERM`, then
  `SIGKILL`), records `cancelled` results, and ends the turn.

## Concurrency

- Sessions run independently, each as its own task in the server. A slow
  model or a long command in one session never delays another.
- Within a session, everything happens in order: one turn at a time,
  messages queued while a turn runs, tool calls one after another, at most
  one open question.
- Within a turn, streaming from the provider, running a tool and listening
  for cancellation happen at the same time, so cancelling works at any
  point.
- Clients are independent of each other and of the sessions: a client that
  reads slowly gets its events late but never blocks the session. If more
  than 4096 live messages wait for it, the server drops the connection and
  the client reattaches and replays the log. Responses and the replayed log
  are not counted.

## Trust boundaries

| Source | Trusted | Guard |
|--------|---------|-------|
| User config file | Yes | Validation only. |
| Project config file | No: comes with the repository | May only make permissions stricter; no providers, models or `[tui]` (see [`config.md`](config.md), "Files"). |
| Model output (text, tool calls) | No | Argument checks, workspace boundary, permission rules including the built-in rules for secret files (see [`config.md`](config.md), "Built-in rules"), questions (see [`tools.md`](tools.md)). |
| Tool output (file contents, command output) | No | Sent to the model as data, and the system prompt says so; never interpreted by `clai`, never logged. |
| Clients | Yes, same user | Socket directory mode `0700`; the server never listens on the network. |
| A later web UI | Only through a bridge the user starts and authenticates | Same protocol and same answer checks as any client (see [`sessions.md`](sessions.md), "Clients"). |
| Provider APIs | For their answers only | HTTPS (or a local address the user configured); keys only to the provider they belong to. |

Answers are checked by the server, not by the client: a client can only pick
one of the choices of the open question, and an `always` rule must match the
call. A buggy client can therefore not allow more than the user chose.

## Errors

Errors are values that pass up the layers, each layer adding what it knows,
until one decides what the user sees. Nothing on the way panics or exits the
server (see [`agent.md`](../agent.md), "Never crash").

| Error in | Becomes |
|----------|---------|
| Tool (bad arguments, file not found, command failed) | An error result for the model; the turn goes on. |
| Provider (network, rate limit, overload) | Retried; then an `error` event, the turn ends, the session stays. |
| Session task (bug, panic) | An `error` event and `ended` for that session; other sessions go on. |
| Configuration reload | A `notice` to all clients; the previous configuration stays. |
| Client (lost connection) | The client exits or reconnects; the session goes on. |

## Extending

| To add | Change | Spec |
|--------|--------|------|
| An API type | One adapter; a new value of `api`. Nothing above the provider layer. | [`providers.md`](providers.md), "Adapters" |
| A tool | The tool, its schema, its permission check and question details, its default action. Rules and questions need no other change. | [`tools.md`](tools.md) |
| A client | Speak the protocol; render events and questions. The server and sessions need no change. | [Protocol](#protocol) |

## Open questions

1. Should the event log be bounded (e.g. by size), with older events only
   kept as history for the model, to limit server memory for long sessions?
