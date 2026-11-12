# Language server tools

- RFC: 0019
- Status: accepted
- Progress: step 2 of 6 done (transport and one server; configuration and pool)
- Milestone: none; it adds a tool, a configuration table and a question
- Spec: [`spec/tools.md`](../spec/tools.md);
  [`spec/config.md`](../spec/config.md), "Permissions", "Project files";
  [`spec/architecture.md`](../spec/architecture.md), "Components",
  "Trust boundaries";
  [`spec/sessions.md`](../spec/sessions.md), "Questions";
  [`spec/tui.md`](../spec/tui.md);
  [`spec/agent-loop.md`](../spec/agent-loop.md), "System prompt"

## Summary

A new tool, `lsp`, lets the model ask a language server about the code:
where a symbol is defined, where it is used, what type it has, which
symbols a file or the workspace has, who calls a function, and which
errors a file has. `clai` starts the servers the user configured (and
well-known ones found on `PATH`), keeps them running while sessions use
them, and keeps them in sync with the files. After `edit_file` and
`write_file`, the errors a running server reports for the changed file
are added to the tool's result, so the model sees a broken edit at once.

Starting a language server runs code of the project (build scripts,
macros), so it is a permission question, like a command.

## Motivation

The model finds code with `search_files` today. A regex finds text, not
meaning:

- **Definitions.** `fn parse` finds every `parse`; methods, traits,
  re-exports and macros are guessed. The model reads several files to
  find the one it wanted.
- **References.** Renaming or changing a function needs all its callers.
  A regex misses calls through a trait or an alias and finds comments
  and strings. A missed caller is found only when the build fails, if it
  is built at all.
- **Types.** `let x = foo.bar()?` says nothing about `x`. The model opens
  `bar`, then its return type, then the type's file.
- **Errors after an edit.** The model runs `cargo check` or `tsc` through
  `run_command`, which asks, takes long, and prints errors of the whole
  project. Often it does not run it, and the user finds the error.

Each of these costs requests and tokens and is often wrong. A language
server answers them exactly, in milliseconds once it has indexed, for
the languages the user already has servers for.

## Scope

In:

- The `lsp` tool: operations, arguments, results, errors.
- `[lsp]` in the user file: servers, built-in servers, timeouts,
  diagnostics after edits.
- Starting a server as a permission question; `lsp` in the `read` group
  for paths.
- Server processes: one per server and root, shared by the sessions of
  the clai server; start, readiness, sync of files, stop.
- Diagnostics added to `edit_file` and `write_file` results.
- TUI: the tool line, details, the start question, `/lsp` with the
  servers' state. `clai chat`: the tool line, the start question,
  `/lsp`.
- `clai run`: the same, with servers stopped when it exits.

Out:

- Rename, code actions, formatting and other operations that change
  files; rename comes in a later RFC. They would need `edit_file`'s
  diff question for many files at once.
- Completion and signature help: made for a cursor in an editor, not for
  a model that writes whole edits.
- Installing language servers. `clai` uses what is on `PATH` or
  configured.
- The Debug Adapter Protocol, and any language support of its own
  (tree-sitter outlines and the like).
- Servers over TCP or pipes other than standard input and output.

## Design

### The tool

One tool with an `operation`, so the model sees one tool in the list,
not nine.

```json
{
  "operation": "references",
  "path": "src/parse.rs",
  "line": 3,
  "symbol": "parse"
}
```

| Argument | Required | Meaning |
|----------|----------|---------|
| `operation` | yes | One of the operations below. |
| `path` | for all but `workspace_symbols` | The file. |
| `line` | for position operations | 1-based line, as `read_file` shows it. |
| `symbol` | for position operations, unless `column` | The name on that line to ask about. |
| `occurrence` | no | Which occurrence of `symbol` on the line, 1-based. Default `1`. |
| `column` | no | 1-based column, in characters, instead of `symbol`. |
| `query` | for `workspace_symbols` | Name or part of it. |
| `include_declaration` | no | `references`: also list the definition. Default `false`. |

The model names a symbol, not a column: models count characters badly,
and the name is in the line it just read. `clai` finds the symbol as a
whole word on the line and converts the position to the server's
encoding (UTF-16 by default, UTF-8 when the server offers it).

| Operation | Position | Asks the server for | Result |
|-----------|----------|---------------------|--------|
| `definition` | yes | `textDocument/definition` | Locations. |
| `type_definition` | yes | `textDocument/typeDefinition` | Locations. |
| `implementation` | yes | `textDocument/implementation` | Locations. |
| `references` | yes | `textDocument/references` | Locations. |
| `hover` | yes | `textDocument/hover` | The type and docs, as text. |
| `incoming_calls` | yes | `callHierarchy/incomingCalls` | Callers with the call lines. |
| `outgoing_calls` | yes | `callHierarchy/outgoingCalls` | Called functions. |
| `document_symbols` | no | `textDocument/documentSymbol` | The outline of the file. |
| `workspace_symbols` | no | `workspace/symbol` | Matching symbols. |
| `diagnostics` | no | Pushed or pulled diagnostics | Errors and warnings of the file. |

An operation the server does not offer (its capabilities) is an error
naming the operations it has.

### Results

Paths are shown as in the other tools: relative to the workspace,
absolute outside it. Lines and columns are 1-based.

Locations, sorted by path and line, with the line's text cut to 200
characters:

```
3 references to parse:
src/main.rs:17:17: let lines = parse(&text);
src/lib.rs:4:9:    pub use parse::parse;
tests/parse.rs:8:5:  parse("a\nb");
```

- At most 100 locations; more ends with
  `… 240 more in 31 files; ask about a narrower symbol`.
- A location outside the workspace and the `extra_dirs` (a dependency's
  source, the standard library) shows the path and line, not the text:
  `~/.cargo/registry/src/…/serde-1.0.229/src/de/mod.rs:540:5 (outside the workspace)`.
  The model can learn where it is, but not read it this way, as with
  `read_file`.
- A location in a file that the `read_file` rules deny or that asks is
  left out and counted, as in `search_files`:
  `(2 locations skipped by permission rules)`.
- No location: `(none found)`.

`hover`: the server's text, Markdown kept, cut to 4 000 characters.
Hover on a symbol whose definition is in a denied file still shows the
text, since it comes from the file asked about.

`document_symbols`: an indented outline with kinds and line ranges:

```
struct Parser            12-40
  fn new                 18-22
  fn parse               24-39
fn parse                 43-47
```

`workspace_symbols`: one line per symbol, `kind name  path:line`, at
most 100.

`incoming_calls`, `outgoing_calls`: one line per caller or callee with
the location of each call.

`diagnostics`: one line per diagnostic, errors first, at most 50:

```
src/parse.rs:12:5: error[E0308]: mismatched types: expected `Vec<Line>`, found `()`
src/parse.rs:30:9: warning: unused variable: `rest`
```

`(no errors or warnings)` when empty. It never starts a check of the
whole project; it reports what the server has for this file.

### Errors

| Case | Error result |
|------|--------------|
| No server for the file's language | `no language server for .toml files; servers: rust-analyzer (.rs), gopls (.go)` |
| Starting was denied | `language server not started: denied by the user` |
| Server failed to start or crashed | `rust-analyzer failed: <last line of its stderr>; use search_files instead` |
| Still indexing after the timeout | `rust-analyzer is still indexing (42 %); try again later, or use search_files` |
| Symbol not on the line | `"parse" is not on line 3 of src/parse.rs: <the line>` |
| Request failed | `rust-analyzer: <the server's message>` |

A crashed server is started again once on the next call; after a second
crash in 5 minutes it stays failed until `/lsp restart` or a config
reload.

### Configuration

In the user file only. A project file may set `enabled = false` and
`start = "deny"` and nothing else, because a server is a command that
runs.

```toml
[lsp]
enabled = true                # false: no lsp tool, no servers
builtin = true                # the built-in servers below, if on PATH
start = "ask"                 # starting a server: ask, allow or deny
diagnostics_after_edit = true
start_timeout_s = 60          # waiting for initialize and indexing
request_timeout_s = 20

[lsp.servers.rust-analyzer]
command    = ["rust-analyzer"]
extensions = ["rs"]
roots      = ["Cargo.toml"]
# Sent as initializationOptions; and in answer to workspace/configuration.
settings   = { checkOnSave = false }

[lsp.servers.ruff]
command    = ["ruff", "server"]
extensions = ["py", "pyi"]
roots      = ["pyproject.toml", "ruff.toml"]
```

| Key | Meaning |
|-----|---------|
| `command` | Program and arguments; the program found on `PATH` or absolute. Talks over standard input and output. |
| `extensions` | File extensions it serves, without the dot. |
| `roots` | Files or directories that mark a project root, looked for upwards from the file, stopping at the workspace. Default: the workspace. |
| `settings` | A table, sent as `initializationOptions` and returned for `workspace/configuration`. |
| `env` | Extra environment variables. |

Built-in servers, used when `builtin = true` and the
program is on `PATH`, and when no server of the user serves the
extension:

| Server | Command | Extensions | Roots |
|--------|---------|------------|-------|
| rust-analyzer | `rust-analyzer` | `rs` | `Cargo.toml` |
| gopls | `gopls` | `go` | `go.mod` |
| clangd | `clangd` | `c`, `h`, `cc`, `cpp`, `hpp` | `compile_commands.json`, `.clangd` |
| pyright | `pyright-langserver --stdio` | `py`, `pyi` | `pyproject.toml`, `setup.py` |
| typescript-language-server | `typescript-language-server --stdio` | `ts`, `tsx`, `js`, `jsx` | `package.json`, `tsconfig.json` |

A user server with the same name replaces the built-in one; `command =
[]` turns one off. Two servers for one extension is a load error.

### Permissions

`lsp` is a new tool key in `[permissions]`, default `allow`, and a member
of the `read` group. Its `path` is checked with the `lsp` rules, so
`deny read(secrets/**)` keeps the model from asking about files there.
Results are filtered with the `read_file` rules as above. `a` proposes
`lsp(PATH)` and wider, as `read_file` does.

**Starting a server is its own question.** A language server runs code
of the project: rust-analyzer runs build scripts and procedural macros,
gopls runs `go list`, the TypeScript server loads plugins from
`tsconfig.json`. For a cloned repository this is the same as
`cargo build`, which asks. So the first call that needs a server in a
session asks:

```
Start language server rust-analyzer in ~/ws/clai?
  command: rust-analyzer
  It may run build scripts and macros of the project.
[y] yes  [a] always for this server  [n] no
```

- `y` starts it for this session; `a` adds a session rule for this server
  and root; `n` gives the "denied by the user" result, and the session
  does not ask again for that server this turn.
- `[lsp] start = "allow"` in the user file skips the question;
  `"deny"` never starts servers (the tool is then left out). Default
  `"ask"`. A project file may set `"deny"`.
- When the server already runs for another session, the question is
  still asked: the permission belongs to the session, as session rules
  do (`spec/architecture.md`, "State and ownership").
- Where no one can answer (`clai run` without a terminal, `--no-ask`),
  `ask` is `deny`, and the tool is left out of the request unless
  `start = "allow"`. `--yes` allows.

The question is a new kind of `details`, `lsp_start`, with the server's
name, command, root and environment. Protocol `8`.

### Servers and their processes

```
 clai server
 ├─ session "clai" ──┐
 ├─ session "fix" ───┼──▶ lsp pool ──▶ rust-analyzer  (root ~/ws/clai)
 └─ session "docs" ──┘              └─▶ pyright        (root ~/ws/docs)
```

- **Pool.** The clai server (or the `clai run` process) owns a pool of
  servers, keyed by server name and root. Sessions with the same root
  share one process.
- **Start.** On the first call that needs it, after the start question.
  `initialize` with the root, the client capabilities `clai` handles,
  and `settings`; then `initialized`. The process gets its own process
  group, as `run_command`, and the environment of the clai server plus
  `env`.
- **Readiness.** A request waits until `initialize` answered, then sends.
  While the server reports `$/progress` work (indexing), a request that
  gets no answer within `request_timeout_s` gives the "still indexing"
  error with the last percentage. The first call waits up to
  `start_timeout_s`.
- **Stop.** A server without requests for 10 minutes is shut down
  (`shutdown`, `exit`, then the process group is killed after 2 s). All
  stop when the clai server or `clai run` exits, and when `[lsp]` changes
  on reload.
- **Stderr** goes to the log at `debug`; the last 50 lines are kept for
  `/lsp` and error results.
- **Server requests.** `workspace/configuration` gets `settings`;
  `window/workDoneProgress/create` and `client/registerCapability` are
  acknowledged; `window/showMessageRequest` gets no choice; anything else
  gets "method not found". `clai` never applies a `workspace/applyEdit`.
- **Malformed messages** fail the server, never the session (`agent.md`,
  "Never crash"). A header without `Content-Length`, a header line over
  1 KiB, a body over 64 MiB or a body that is not JSON-RPC is malformed.

### Files in sync

The server must see what is on disk, also after `edit_file`,
`write_file`, `run_command` (`cargo fmt`, `git checkout`) and the user's
editor.

- A file asked about is opened (`didOpen`) with its text from disk, and
  kept open with its size and modification time. Before each request the
  open files of that server are checked; a changed one gets `didChange`
  with the full text. At most 50 files stay open; the least recently
  used is closed.
- After `edit_file` and `write_file`, an open file gets `didChange` and
  `didSave` at once.
- Other files: the client capabilities do not offer file watching
  (`didChangeWatchedFiles`), so servers watch the root themselves, as
  rust-analyzer, gopls and pyright do without a client watcher.

- Files outside the workspace and `extra_dirs` are never opened.

### Diagnostics after an edit

When `diagnostics_after_edit` is on and a server already runs for the
edited file in this session, the result of `edit_file` and `write_file`
gets the file's errors:

```
Replaced 1 occurrence in src/parse.rs (lines 12–14).

rust-analyzer reports 1 error in src/parse.rs:
src/parse.rs:13:9: error[E0425]: cannot find value `lnes` in this scope
(and 2 warnings; lsp diagnostics lists them)
```

- `clai` waits for diagnostics of the new version, from
  `textDocument/publishDiagnostics` or a `textDocument/diagnostic` pull,
  up to 3 s; then it adds what it has, or nothing.
- Errors only, at most 10; warnings counted.
- An edit never starts a server and never asks; the question would come
  in the middle of a change.
- The edit result stays a success; diagnostics are information.

### Display

| Client | Shown |
|--------|-------|
| Tool line | `lsp references parse src/parse.rs:3`, `lsp document_symbols src/parse.rs`; when done, the count (`3 references`). |
| Details | The result. |
| Start question | A permission prompt with the command and root, keys as above. |
| `/lsp` (TUI, `clai chat`) | Servers with root, state (`starting`, `indexing 42 %`, `ready`, `failed`), the sessions using them, and the last stderr lines; in the TUI, `r` restarts and `k` stops the selected one. |
| `/status` | The number of `lsp` calls, with the other tools. |

`lsp_status` and `lsp_restart` requests; protocol `8`.

### System prompt

A new bullet, only when the tool is offered:

- For definitions, references, types and callers of a symbol in
  <languages>, use lsp before search_files. After an edit, errors the
  language server reports are added to the result; fix them before you
  go on.

### Code

- `src/lsp/` (new):
  - `transport.rs`: `Content-Length` framing, JSON-RPC requests,
    responses, notifications over any `AsyncRead` / `AsyncWrite`; ids,
    `$/cancelRequest` on cancellation.
  - `server.rs`: one process: start, initialize, capabilities, progress,
    server requests, stderr tail, stop.
  - `docs.rs`: open files, versions, sync, position encoding.
  - `pool.rs`: servers by name and root; idle stop; crash restart.
  - `config.rs`: built-ins and the merge with `[lsp]`.
- `src/tools/lsp.rs` (new): arguments, symbol to position, operations,
  result texts, filtering.
- `src/tools/edit_file.rs`, `src/tools/write_file.rs`: the diagnostics
  hook, through `Env`.
- `src/tools/mod.rs`: `Op::Lsp`; `Env` gains the pool handle and the
  session's started servers.
- `src/config/`: `[lsp]` parsing, validation, project-file rule;
  `Tool::Lsp` in the `read` group.
- `src/agent/`: the `lsp_start` question; leaving the tool out.
- `src/server/`, `src/command/run.rs`: the pool's owner;
  `lsp_status`, `lsp_restart`.
- `src/tui/`: tool line, prompt, `/lsp` box; `src/command/chat/`: `/lsp`.

## Dependencies

- `lsp-types`, for the request and result types, in the version current
  when built. The framing and JSON-RPC are written in `src/lsp/transport.rs`
  on `tokio`, about 200 lines, instead of `tower-lsp` or `async-lsp`,
  which are made for writing servers and bring their own runtime layers.

No new `tokio` features: `process` and `io-util` are on.

## Spec changes

- `spec/tools.md`: `lsp` in the tools table; a section `## lsp` with
  arguments, operations, results, errors, permissions; "Questions" with
  `lsp` and `lsp_start` details; `edit_file` and `write_file` with
  diagnostics in their results.
- `spec/config.md`: an `## LSP` section with `[lsp]`, the built-in table,
  the project-file rule; "Permissions" with the `lsp` key and the `read`
  group.
- `spec/architecture.md`: the component "LSP" and the pool in the
  picture; "Trust boundaries": language servers run project code and are
  started only after the question; "Protocol": version `8`.
- `spec/sessions.md`, "Questions": the `lsp_start` kind of details.
- `spec/tui.md` and `spec/command.md`: the tool line, `/lsp`.
- `spec/agent-loop.md`, "System prompt": the bullet.

## Testing

- `src/lsp/transport.rs`: framing (split and joined messages, bad
  headers, too large bodies), ids, cancellation, a server that closes.
- A fake language server in the tests, in process over a `tokio` duplex
  stream: answers each operation, reports progress, publishes
  diagnostics, crashes on request.
  - Each operation's result text; limits; outside-workspace and denied
    locations; `(none found)`.
  - Symbol to position: several occurrences, Unicode before the symbol
    (UTF-16 and UTF-8 encodings), symbol not on the line.
  - Sync: a file changed on disk gets `didChange` before the next
    request; the 51st file closes the oldest.
  - Diagnostics after an edit: arrive in time, arrive late (left out),
    no server running (nothing added, no question).
  - Still indexing gives the error; a crash restarts once, twice stays
    failed.
- `src/agent/tests.rs`: the start question opens once per server and
  session; `n`, `a`, `start = "allow"`, `"deny"` leaves the tool out;
  `clai run` without a terminal leaves it out.
- `src/config/tests.rs`: `[lsp]` validation; two servers for one
  extension; a project file may set only `enabled = false` and
  `start = "deny"`.
- `src/protocol/tests.rs`: `lsp_start` details, `lsp_status` round-trip.
- `src/tui/tests.rs`: the tool line, the start prompt, the `/lsp` box;
  snapshots.
- `tests/live.rs`, `--ignored`: rust-analyzer on a small crate in a
  temporary directory: definition, references, diagnostics after a
  broken edit.

## Implementation plan

1. **Transport and one server**
   - Status: done
   - Files: `src/lsp/transport.rs`, `src/lsp/server.rs`,
     `src/lsp/docs.rs`, `Cargo.toml`, tests with the fake server
   - Result: a server can be started, initialized, asked, synced and
     stopped from a test.
   - Tests: transport, server life, sync, position encoding.
   - Built: `src/lsp/` with `mod.rs` (`lsp::Error`), `transport.rs`,
     `server.rs`, `docs.rs` and `tests.rs` (a fake server over a duplex
     stream, and real processes for a missing program and one that
     exits). Choices made while building it:
     - `workspace/applyEdit` is answered with `applied: false`, not
       "method not found", so the server knows the edit was refused.
     - `workspace/configuration`: a section named in `settings` gets that
       part, any other section the whole table.
     - A failed start or a closed connection reports the last stderr
       line of the server.
     - Not used yet outside the tests; the tool comes in step 3.

2. **Configuration and pool**
   - Status: done
   - Files: `src/lsp/config.rs`, `src/lsp/pool.rs`, `src/config/`,
     `src/config/tests.rs`
   - Result: `[lsp]` loads and is checked; built-ins found on `PATH`;
     roots found; servers shared, stopped when idle and on reload.
   - Tests: config tests, pool tests.
   - Built: `src/config/lsp.rs` (`Lsp`, `LspServer`, the checks and the
     project-file rule), `src/lsp/config.rs` (`BUILTINS`, `servers`,
     `for_file`, `find_root`, `on_path`) and `src/lsp/pool.rs` (`Pool`),
     with tests in `src/config/tests.rs`, `src/lsp/tests.rs` and the
     config files of `tests/configs/`. Choices made while building it:
     - A table named like a built-in server takes the keys it leaves out
       from that server, so `[lsp.servers.rust-analyzer]` with only
       `settings` is enough. Any other server needs `command` and
       `extensions`; `command = []` is allowed only for built-in names.
     - A built-in server gives up only the extensions a user server
       takes: with `ruff` on `py`, `pyright` still serves `pyi`.
     - User servers are not looked for on `PATH`; a missing program is
       the start error. Built-in ones are, and only in absolute `PATH`
       entries, as executable files.
     - Server names: letters, digits, `.`, `_`, `-`. Roots: names
       relative to the project root, no `..`, `/` or `~` at the start.
       `start_timeout_s` is 1–3600, `request_timeout_s` 1–600.
     - A project file's `[lsp]` is checked by hand: only
       `enabled = false` and `start = "deny"`; any other key or value is
       an error naming it.
     - A failed start counts as a crash, so a broken server is not
       started on every call: two in 5 minutes leave it failed. The error
       then says `stays stopped until /lsp restart`.
     - `Pool::restart` forgets the crashes and starts at once;
       `Pool::stop` stops one server, which starts again on the next
       call; `Pool::apply_config` stops and forgets all servers when
       `[lsp]` differs from the last one it got.
     - The pool takes a starter function, so the tests run it with the
       in-process fake server; `spawn_reaper` checks every minute (a
       tenth of the idle time when shorter).
     - Not used yet outside the tests; the clai server and `clai run` own
       a pool from step 3.

3. **The tool**
   - Status: todo
   - Files: `src/tools/lsp.rs`, `src/tools/mod.rs`,
     `src/config/permissions.rs`, `src/agent/`, `src/server/`,
     `src/command/run.rs`, `src/protocol/`, `src/agent/system_prompt.md`
   - Result: the model can call `lsp` in a session and in `clai run`;
     the start question; filtering; protocol `8`.
   - Tests: tool tests, agent tests, protocol tests.

4. **Diagnostics after edits**
   - Status: todo
   - Files: `src/tools/edit_file.rs`, `src/tools/write_file.rs`,
     `src/lsp/`
   - Result: a broken edit shows its errors in the result.
   - Tests: the diagnostics tests.

5. **Clients**
   - Status: todo
   - Files: `src/tui/`, `src/command/chat/`, `src/text/`, snapshots
   - Result: tool lines, the start prompt, `/lsp` with restart and
     stop.
   - Tests: TUI snapshots and keys; chat text.

6. **Spec, readme and a live test**
   - Status: todo
   - Files: the spec files above, `readme.md`, `todo.md`,
     `tests/live.rs`
   - Result: the spec says what the code does; the readme explains
     `[lsp]` and the start question.
   - Tests: `cargo fmt`, `clippy`, `test`; the live test by hand.

## Alternatives

- **One tool per operation** (`find_references`, `hover`, …). Clearer
  names, but nine more tools in every request, most of them unused in a
  turn. One tool with an enum costs one schema.
- **Diagnostics only, through `run_command`** (`cargo check`). Works
  today, but asks, is slow, and gives no definitions or references.
- **tree-sitter outlines and a symbol index of our own.** No server
  needed and no project code runs, but no types, no references through
  traits, and one grammar per language to maintain. Could come later
  for `document_symbols` where no server is configured.
- **Start servers without asking.** Faster and what editors do, but an
  editor's user chose to open the project; the model may be pointed at
  any clone, and `run_command` asks for the same risk.
- **A server per session.** Simple, but rust-analyzer takes seconds to
  minutes and hundreds of MiB to index; three sessions on one repo would
  index three times.
- **`tower-lsp` / `async-lsp`.** Built for the server side, or bring a
  `tower` stack; the client side needs only framing and a few requests.

## Decisions

Answered by the user when the RFC was accepted:

1. One tool with `operation`, not one tool per operation.
2. `rename` comes in a later RFC.
3. Built-in servers are on by default when found on `PATH`.
4. Diagnostics after an edit come only from a running server; an edit
   never starts one.
5. Hover and symbol names are shown for symbols defined in a file that
   the `read_file` rules deny.
6. `lsp-types` for the message types.
7. "Always" for starting a server lasts for the session only.
