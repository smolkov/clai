# Sessions

`clai` can run several sessions at the same time. Sessions run in a
background process, the server, and keep running when no TUI shows them. A
TUI attaches to one session at a time; it can detach, switch to another
session, or start a new one. This works like `tmux` or `screen`, for agent
sessions.

Statements marked **(assumption)** need a decision from the user.

## Terms

| Term        | Meaning |
|-------------|---------|
| Server      | Background `clai` process of one user that owns all sessions and runs their agent loops and tools. |
| Client      | A process that talks to the server: a TUI, a terminal command such as `clai ls` or `clai chat`, or later a web UI (see [Questions](#questions)). |
| Session     | One conversation with the model in one workspace, with its history, settings and permission rules. It lives in the server, not in a client. |
| Attach      | Connect a client to a session: it receives the session's events (conversation, questions) and may send user input and answers. |
| Question    | A decision the session waits for: a permission question for a tool call, or a limit question. Sent to every attached client that can answer. |
| Detach      | Disconnect a TUI from its session; the session keeps running. |

## Server

- One server per user and machine. Clients reach it over a Unix domain
  socket at `$XDG_RUNTIME_DIR/clai/server.sock` (fallback
  `/tmp/clai-$UID/server.sock`). The socket directory has mode `0700`, so
  only the user can connect. **(assumption)**
- The server is started on demand: a client that finds no server starts one
  in the background and then connects. `clai server` runs it in the
  foreground, for debugging.
- The server exits when it has had no sessions and no clients for 60 s.
  **(assumption)**
- The server is a local, single-user process. It never listens on a network
  port.
- The server must not crash (see `agent.md`, "Never crash"). An error in one
  session is reported in that session and never affects other sessions.
- Client and server exchange versioned messages. A client that does not match
  the server's protocol version gets a clear error that tells the user to
  restart the server (`clai server stop`).

## Session lifecycle

```
          new                 attach              detach / TUI exits
 (none) ───────▶ idle ◀──────────────────▶ attached ────────────────▶ idle / working
                  │  ▲                                                     │
       user msg   ▼  │ turn ends                                           │
                 working ──▶ waiting (question open) ───────────────────────┘
                  │
       kill      ▼
                ended
```

- **New:** a session is created with a workspace (the client's current
  directory unless given), a provider and model from the configuration
  (`default_model` unless given), and a name. The default name is the
  workspace directory name, with `-2`, `-3`, … added if it is taken.
  Names are unique per server.
- **Working and waiting:** see [Background work](#background-work).
- **Kill:** ends the session. A running turn is cancelled first (same as
  cancelling in the TUI). Attached TUIs show "session ended" and offer the
  session list.
- Each session keeps its own history, provider and model, allow rules and
  limits. The provider and model can be changed at any time from the TUI
  (see `spec/tui.md`, "Model picker"). Allow
  rules granted in one session never apply to another.

## Background work

A session works in the background and waits for the user only when a
decision is needed.

- A session always runs in the server. Model requests, allowed tool calls,
  retries and errors go on the same way whether a TUI is attached or not,
  and whether the attached TUI shows this session or not.
- A session waits for the user only for a **decision**:

  | Decision | When | The session |
  |----------|------|-------------|
  | Permission question | A tool call is `ask` (see [`config.md`](config.md), "Permissions"). | Stops at that call until a client answers. Tool calls after it in the same answer wait too; calls before it have already run. |
  | Limit question | The turn reached its limit of model requests or tool calls. | Stops before the next model request or tool call until a client answers "continue" or "stop". **Continue** starts a new budget of the same size; **stop** ends the turn with the notice "limit reached". |

  What a question contains and how it is answered: [Questions](#questions).

  While a question is open, the session's state is `waiting`. A detached
  session waits; it never decides on its own. `clai ls` and the TUI session
  list show waiting sessions.
- A finished turn is not a question: when the model answers without tool
  calls (often with a question for the user in its text), the session
  becomes `idle` and waits for the next message.
- A session never waits for anything else: not for a TUI to attach, not for
  a TUI to show its output, not for errors to be confirmed. Everything that
  happened while no TUI was attached is shown on the next attach (see
  [`tui.md`](tui.md), "Switching sessions").
- While a session works, its TUI stays usable: the user can type and queue
  messages, switch sessions or detach.
- A one-shot `clai run` asks on the terminal when standard input is one.
  Otherwise no one can answer: a permission question counts as denied
  (allowed with `--yes`) and a limit question counts as "stop" (see
  [`command.md`](command.md)). **(assumption)**

## Several clients on one session

- A TUI is attached to at most one session at a time.
- More than one TUI may attach to the same session, e.g. from two terminals.
  All of them show the same conversation live. Each has its own scroll
  position and input. **(assumption)**
- A question (permission or limit) is sent to all attached clients that can
  answer; the first answer counts and the others close the question (see
  [Questions](#questions)).
- Messages typed in several TUIs are queued in the order the server receives
  them.

## Questions

A question is a message from the session to its clients. It carries
everything needed to decide without looking anywhere else, so every kind of
client can show it and answer it the same way: the TUI as a prompt bar, the
terminal client `clai chat` as lines of text (see
[`command.md`](command.md)), and a later web UI as a page. Clients only show
questions and send answers; the decision is checked and applied by the
server.

### Open questions and clients

- A session has at most one open question at a time, because tool calls run
  one after another.
- When a question opens, it is sent to every attached client that declared,
  when attaching, that it can answer. Clients that only watch (e.g. a
  second TUI in read-only mode, or `clai ls`) see that the session is
  `waiting` and why, but get no choices. **(assumption)**
- A client that attaches while a question is open receives it at once, as
  part of the session's state.
- A question stays open until it is answered or the turn is cancelled. It
  never times out and is never answered by the server itself, except in
  `clai run` (see [Background work](#background-work)).
- The first valid answer counts. The server then sends `question closed`,
  with the answer and the client that gave it (e.g. `tui pts/3`,
  `clai chat`), to all clients; they remove the prompt and show the
  outcome in the conversation. A later answer to the same question is
  rejected with `already answered by tui pts/3`.

### Content

Every question has:

| Field | Meaning |
|-------|---------|
| `id` | Unique on the server; an answer names it. |
| `session` | Session name and workspace. |
| `kind` | `permission` or `limit`. |
| `opened` | Time the question opened. |
| `turn` | Number of the turn, and how many model requests and tool calls it has used. |
| `summary` | One line for lists and notifications, e.g. `run_command: cargo test parse` or `edit_file: src/parse.rs (lines 12–14)`. |
| `choices` | The answers this question accepts (see below). |

A permission question also has:

| Field | Meaning |
|-------|---------|
| `call` | Tool name, the call's id from the model, and its arguments exactly as the model sent them (JSON). |
| `details` | What the tool will do, prepared by the tool: resolved paths, the diff or new content, the command with its parts and working directory. Per tool in [`tools.md`](tools.md), "Questions". |
| `reason` | Why it asks: the deciding rule and its source (`config.toml:24`, `.clai/config.toml:3`) or `default for edit_file`, and the rule's `message` if it has one (see [`config.md`](config.md), "Checking order"). |
| `proposals` | Allow rules for "always", narrowest first (see [`tools.md`](tools.md), "Permissions"). |
| `text` | The model's text in the same answer before the tool call, if any, so the user sees what the model said it was about to do. |

A limit question has the limit and the counts reached instead of `call`,
`details` and `proposals`.

Texts in a question are plain UTF-8; diffs are unified diffs; a client adds
colour and layout itself. Large parts (file content, long diffs) are sent in
full up to 1 MiB, with a mark when cut, so a client can show a short view
first and the whole on request. **(assumption)**

### Answers

| Choice | For | Effect |
|--------|-----|--------|
| `allow` | permission | Run this call once. |
| `allow_always` | permission | Run this call and add allow rules to the session. The answer names the rules: one of `proposals` (each is a set of rules), or rules the user typed for this tool (for `run_command` also `write_file`/`read_file` rules for its redirections). Together they must allow this call; otherwise the answer is rejected and the question stays open. |
| `deny` | permission | Do not run the call; the model gets `denied by the user`. |
| `deny_message` | permission | As `deny`, with a text for the model: `denied by the user: <text>`. |
| `cancel` | both | Deny, and cancel the turn. |
| `continue` | limit | Start a new budget of the same size. |
| `stop` | limit | End the turn with "limit reached". |

An answer names the question `id` and the choice, plus the rule or text the
choice needs. The server checks it against the open question; a wrong id,
choice or rule is rejected with a message, and the question stays open.

Before running an `edit_file` or `write_file` call that was allowed, the
server checks the file again; if it changed since the question opened, the
call fails (see [`tools.md`](tools.md), "edit_file").

### Clients

| Client | Shows a question as | Answers with |
|--------|---------------------|--------------|
| TUI | Expanded tool call and prompt bar (see [`tui.md`](tui.md), "Permission prompts"). | Keys `y`, `a`, `n`, `e`, `Esc`. |
| Terminal: `clai chat` | The fields as text, the diff or command, and a numbered list of choices (see [`command.md`](command.md)). | A line on standard input, or options for scripts. |
| Terminal: `clai run` | The same text on standard error, when standard input is a terminal. | A line on standard input. |
| Web UI (later) | A page with the same fields. | The same answers. |

The web UI is not part of the first version (see [`readme.md`](readme.md),
"Non-goals"). The question and answer messages are designed so it needs
nothing else from the server; it would reach the server through a local
bridge that the user starts and authenticates, because the server itself
never listens on a network port. **(assumption)**

## Commands

Sessions are managed with `clai`, `clai new`, `clai attach`, `clai ls`,
`clai kill`, `clai send`, `clai chat` and `clai server`; syntax and options are in
[`command.md`](command.md). In the TUI, the session list and the `/sessions`,
`/new`, `/attach`, `/rename`, `/kill` commands do the same (see
[`tui.md`](tui.md)).

## Persistence

Sessions live only as long as the server. If the server stops, its sessions
end; a later server starts empty. **(assumption, see open questions)**

## Open questions

1. Should sessions be saved to disk, so they survive a server restart or
   reboot and can be resumed?
2. Should a detached session that waits for an answer notify the user
   (desktop notification, terminal bell in other attached TUIs)?
3. Is a limit on the number of sessions running at the same time wanted, to
   bound API cost?
4. Should several sessions be allowed in the same workspace? They could
   overwrite each other's file changes.
