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
| Client      | A process that talks to the server: a TUI, or a CLI command such as `clai ls`. |
| Session     | One conversation with the model in one workspace, with its history, settings and permission rules. It lives in the server, not in a client. |
| Attach      | Connect a TUI to a session: the TUI shows its conversation and sends user input to it. |
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
  | Permission question | A tool call is `ask` (see [`config.md`](config.md), "Permissions"). | Stops at that call until an attached TUI answers. Tool calls after it in the same answer wait too; calls before it have already run. |
  | Limit question | The turn reached its limit of model requests or tool calls. | Stops before the next model request or tool call until an attached TUI answers "continue" or "stop". **Continue** starts a new budget of the same size; **stop** ends the turn with the notice "limit reached". |

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
- With no client that can answer (one-shot `clai run`), a permission
  question counts as denied (allowed with `--yes`) and a limit question
  always counts as "stop" (see [`command.md`](command.md)). **(assumption)**

## Several clients on one session

- A TUI is attached to at most one session at a time.
- More than one TUI may attach to the same session, e.g. from two terminals.
  All of them show the same conversation live. Each has its own scroll
  position and input. **(assumption)**
- A question (permission or limit) is shown in all attached TUIs; the first
  answer counts and the others close the question.
- Messages typed in several TUIs are queued in the order the server receives
  them.

## Commands

Sessions are managed with `clai`, `clai new`, `clai attach`, `clai ls`,
`clai kill`, `clai send` and `clai server`; syntax and options are in
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
