# Refactoring the command layer

- RFC: 0009
- Status: done
- Milestone: none; it cleans the ground the "Context management" and
  "Commit" milestones build on
- Spec: [`spec/command.md`](../spec/command.md) (no change of behaviour);
  [`spec/architecture.md`](../spec/architecture.md), "Modules";
  [`implementation.md`](../implementation.md), "Modules"

## Summary

`src/cli.rs`, `src/command/` and `src/main.rs` are reordered without any change
of behaviour:

- **Dispatch:** one `match` instead of two, through an `Execute` trait that each
  command implements.
- **`Context`:** one value in place of the `--config` path threaded through every
  `run`. It also builds the configuration's load options, which four places
  build by hand today.
- **Shared code:** what other modules borrow from single commands goes to shared
  modules. Today the TUI and `clai chat` reach into `ls` and `new`.
- **Helpers:** the helpers several commands copy each exist once: text or
  standard input, the workspace check, the `clai: ` line on standard error, and
  table columns.
- **`clai run`:** it is split into named parts.

Every command, option, message, exit code and completion works exactly as
before. Tests pin the behaviour that no test covers yet, before anything moves.

## Motivation

More commands are coming (`clai commit`; maybe `clai status`, which RFC 0007
left out), and the command layer has grown uneven.

**A new command touches five places, and nothing checks that they agree.**
These are:

- the `Command` variant in `cli.rs`, and its `use`;
- the arm in `Command::name` in `command/mod.rs`;
- the arm in `Command::run` in `command/mod.rs`;
- the new file.

`server` already breaks the pattern of the first match: its name depends on its
action (`server stop`, `server status`).

**`--config` reaches every command, also where it means nothing.** Every command
gets it through `run(&self, config: Option<&Path>)`. `commit` and `completion`
name it `_config`. Four places build the same `LoadOptions` from it:

- `run`;
- `models`;
- `chat::symbols`;
- `tui::run`.

All four use the same `Environment::from_process()`, each with its own
workspace.

**Other modules reach into single commands.** Several modules use code that
lives in one command's file:

| Used by | Reaches into | For |
|---------|--------------|-----|
| `tui/app.rs` | `command::ls::state_name` | session states in the argument list |
| `tui/view/sessions.rs` | `command::ls::age` | the age column of the session list |
| `tui/mod.rs` | `command::new::create` | creating a session for `clai new` |
| `command/chat/talk.rs` | `command::ls::table` | `/sessions` |
| `command/chat/mod.rs` | `command::new::request` | `clai chat --new` |

`ls` and `new` cannot change without looking at the TUI and `chat`.

**The same helpers exist twice, slightly differently:**

- **Standard input:** `run` and `send` read `-` from standard input in two
  near-identical blocks. Only `run` also needs to know that it did: then nobody
  answers questions on the terminal.
- **The workspace check:** `run` has it inline, `new` as the function
  `workspace`. The error text is the same, but only `new` makes the path
  absolute. For `run` that changes nothing, because `Workspace::new` resolves
  the path anyway.
- **`clai: ` lines on standard error:** the command layer writes them with its
  own `writeln!` in twelve places:
  - three in `main` (`report` twice, `warn`);
  - one each in `kill` and `new::create`;
  - two each in `chat/mod.rs` and `chat/questions.rs`;
  - three in `chat/talk.rs` (attached, and two detached lines).
- **Table columns:** `ls::table` and `models::render_table` both pad columns to
  the widest cell, leave the last column free and trim the line. `models` aligns
  one column to the right.

**`Run::run` is one 80-line method.** It reads the task, decides who answers,
checks the workspace, loads the configuration, resolves the model, builds the
provider and the agent's setup, and picks the asker. Only then does it hand
everything to the free `turn` function.

Refactors without a change of behaviour normally need no RFC (see
[`rfcs/readme.md`](readme.md), "When to write one"). This one is written anyway
for two reasons: it spans the whole command layer (every file in
`src/command/`, `cli.rs` and `main.rs`, plus three imports in `src/tui/`), and
its choices shape how later commands are added.

## Scope

In:

- **Dispatch:** the `Execute` trait, one `match` beside the enum in `cli.rs`, and
  no `match` on the commands anywhere else.
- **`command::Context`:** made once in `main`, with the `--config` path and the
  load options.
- **Shared modules:** `text::sessions` for the session table, states and ages;
  `command::common` for what several commands and the TUI use (new-session
  request, `create`, workspace, text or standard input).
- **Helpers:** `output::note` for `clai: ` lines; `output::columns` for tables.
- **`clai run`** in named parts: task, answering, setup, asker, one shot.
- **Tests:** new tests that pin what is not tested yet. The moved code keeps its
  tests.

Out:

- **Behaviour:** nothing changes. Syntax, output text, exit codes, prefixes,
  notes and completions stay as `spec/command.md` says, and where a detail is not
  specified, today's wording stays.
- **Rendered `clai: ` lines:** those that the text renderers (`text::events`,
  `text::conversation`) and the TUI's exit line return as strings stay where
  they are. Only the places that write to standard error themselves use
  `output::note`.
- **`clai chat`:** it stays three files (`mod`, `talk`, `questions`); only its
  imports and its standard-error lines change. `deny MESSAGE...` does not learn
  to read `-`: that would be new behaviour.
- **Client, protocol and server:** `src/client/`, the protocol and the server
  are untouched, and `with_client` stays as it is.
- **`src/complete.rs`:** unchanged.
- **New commands and options**, `spec/command.md` edits, and `clai commit`
  (which has its own milestone).

## Design

### Dispatch

Each command implements one trait, in its own file:

```rust
/// A command of `spec/command.md`.
pub trait Execute {
    /// As used in log lines and messages: `kill`, `server stop`.
    fn name(&self) -> &'static str;
    fn run(&self, ctx: &Context) -> Result<()>;
}

impl Execute for Kill {
    fn name(&self) -> &'static str { "kill" }
    fn run(&self, ctx: &Context) -> Result<()> { … }
}
```

- **The trait's name:** it is `Execute`, not `Run`. `Run` is the argument struct
  of `clai run`, and a trait of the same name would make every `use` ambiguous.
- **`name` is a method, not a constant:** `Server` answers by its action, as it
  does now.
- **One `match` beside the enum:** `cli.rs` gets the only `match` on the
  commands, next to the enum it dispatches on:

  ```rust
  impl Command {
      pub fn execute(&self) -> &dyn Execute {
          match self {
              Command::Run(c) => c,
              Command::New(c) => c,
              // … one arm per command
          }
      }
  }
  ```

  `main` calls `command.execute().name()` and `command.execute().run(&ctx)`. The
  two full matches in `command/mod.rs` are gone.
- **Adding a command** then takes three places, all checked by the compiler:
  - the variant with its `use`;
  - the arm in `execute`;
  - the file with its `impl Execute`.

  A missing arm does not compile, and neither does a variant whose type does not
  implement `Execute`. `implementation.md` gets this as a short checklist.

### Context

`main` makes one `Context` and passes it to every `run`:

```rust
pub struct Context {
    /// The path given by `--config`, if any.
    pub config_file: Option<PathBuf>,
}

impl Context {
    /// Load options for `workspace`, with the process environment.
    pub fn load_options(&self, workspace: PathBuf) -> LoadOptions { … }
    /// `config::load` with those options.
    pub fn load(&self, workspace: PathBuf) -> Result<Config> { … }
}
```

- **The four copies:** `run`, `models`, `chat::symbols` and `tui::run` use these
  methods instead of their own `LoadOptions`.
- **Unused parameters:** a command that needs no configuration ignores the
  context, so no `_config` parameter is left.
- **`tui::run` and `talk::talk`** take `&Context` instead of `Option<&Path>`.
- **Nothing more for now:** no terminal detection and no `std::env` wrappers.
  Nothing needs them yet, and they belong to their first use.

### Shared modules

| What | From | To | Used by |
|------|------|----|---------|
| `table(sessions, now)`, `state_name`, `age` | `command::ls` | `text::sessions` | `ls`, `chat` (`/sessions`), TUI session list and argument list |
| `request(name, model, dir)`, `create(client, request)` | `command::new` | `command::common` | `new`, `chat --new`, `chat /new`, TUI `clai new` |
| `workspace(dir)` | `command::new`, inline in `run` | `command::common` | `new`, `run` |
| `input_text(words, what)` | `run`, `send` | `command::common` | `run`, `send` |

- **`text::sessions`:** the session table is text output shared by a command, a
  chat and the TUI, like the other renderers in `text`. `ls` keeps only its
  options, its JSON and its `run`.
- **`command::common`:** this is code for several commands, not owned by any
  one of them. The TUI imports `create` from there rather than from `new`.
- **`input_text`:** it reads `-` from standard input or joins the words, trims
  them, and refuses an empty text with `the {what} is empty` (exit `2`). It
  returns `Input { text, from_stdin }`, because `run` asks no questions on the
  terminal when the task came from standard input.

### Output helpers

- **`output::note(text)`:** writes `clai: {text}` and a newline to standard
  error, and ignores a failure as the `writeln!` calls do today.
  - It replaces the twelve `writeln!(stderr, "clai: …")` calls of the command
    layer.
  - `main::warn` becomes `note(&format!("warning: {message}"))`.
  - `main::report` writes each configuration error and the final error through
    it.
- **`output::columns(rows, aligns)`:** pads each column to its widest cell with
  two spaces between columns, leaves the last column as it is, and trims each
  line. `aligns` says left or right per padded column, so `models` keeps its
  right-aligned context column.
  - `ls` adds the question summary after the columns, as now.
  - Both tables keep their columns, wording and tests.

### `clai run`

`Run::run` becomes a short sequence of named parts:

```rust
fn run(&self, ctx: &Context) -> Result<()> {
    let task = input_text(&self.task, "task")?;
    let answering = answering(self.yes, self.no_ask, !task.from_stdin && io::stdin().is_terminal());
    let setup = self.setup(ctx)?;                 // workspace, config, model, provider, Setup
    let asker = asker(answering, &setup.place, setup.unicode);
    one_shot(Agent::new(setup.agent), task.text, asker, setup.unicode)
}
```

- **`setup`** returns the agent's `Setup`, the workspace as shown, and the
  symbol choice. It holds the config and model resolution that is inline today.
- **`asker`** is the `match` on `Answering` that is inline today.
- **`one_shot`** wraps the runtime, the free `turn` function (renamed, with its
  signal handling and event rendering) and the runtime's shutdown. The
  in-process partnership is then one unit: the agent runs here, not in the
  server.

Nothing about what `run` does changes: the same options, messages, exit codes
(`130`, `1`, `2`), `Renderer` and `LineAsker`.

## Spec changes

`spec/command.md` lists the commands and their behaviour, and all of it stays.
`spec/architecture.md`, "Modules" says "one file per command with its argument
struct and `run`", and that stays true.

`implementation.md`, "Modules" changes:

- **`cli`:** "the `Command` enum and `Command::execute`, the one dispatch".
- **`command`:** "one file per command with its argument struct and `impl
  Execute`; `Context`; `common` for what several commands share".
- **`text`:** gains "the session table (`text::sessions`)".
- **New checklist** "Adding a command": the variant, the arm in `execute`, the
  file, the spec section, a test in `tests/cli.rs`.

## Testing

The point is that no behaviour changes, so the tests are pinned first and then
left alone:

- **Existing integration tests:** `tests/cli.rs`, `tests/completion.rs`,
  `tests/configs.rs`, `tests/models.rs`, `tests/run.rs` and `tests/server.rs`
  pass unchanged after every step. `tests/live.rs` is not run by default and is
  not touched.
- **New pins, added before anything moves:**
  - `clai send NAME ""` and `clai send NAME -` with empty input both say "the
    message is empty" and exit `2`.
  - `clai run -C` with a missing directory says "… not a directory" and exits
    `2`. Today only `clai new` has this test.
  - The `clai: warning: …` line when logging cannot start.
  - The `clai: ` lines of `chat --list` for an unknown session.
- **Moved code keeps its tests:**
  - `ls::age` and `ls::table` tests move to `text::sessions`.
  - `models::render_table` keeps its expected strings.
  - `output::columns` and `output::note` get their own tests.
  - `input_text` gets tests for words, `-`, and empty text.
- **Unit tests in `cli.rs`:**
  - `unbuilt_commands_are_unsupported` uses `execute()` and a default
    `Context`.
  - A new test checks that `execute().name()` gives each command's name,
    `server stop` and `server status` included.
- **Checks:** `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings` and
  `cargo test` after every step, as `agent.md` says.

## Implementation plan

Each step leaves a working, tested state.

1. **Pin the behaviour**
   - Status: done
   - Files: `tests/server.rs`, `tests/run.rs`, `tests/cli.rs`
   - Result: the messages and exit codes listed under Testing are covered.
   - Tests: the new tests pass against today's code.

2. **Dispatch and context**
   - Status: done
   - Files: `src/command/mod.rs` (`Execute`, `Context`), `src/cli.rs`
     (`execute`), `src/main.rs`, every command file, `src/tui/mod.rs`,
     `src/command/chat/*.rs`, `implementation.md`
   - Result: one `match`; every command implements `Execute` and takes
     `&Context`; the four `LoadOptions` copies use `Context::load_options`.
     This touches every command file once, for both changes.
   - Tests: the parse tests, `unbuilt_commands_are_unsupported`, and the new
     name test.

3. **Shared modules**
   - Status: done
   - Files: `src/text/sessions.rs` (new), `src/command/common.rs` (new),
     `src/command/ls.rs`, `src/command/new.rs`, `src/command/run.rs`,
     `src/command/send.rs`, `src/command/chat/*.rs`, `src/tui/app.rs`,
     `src/tui/view/sessions.rs`, `src/tui/mod.rs`
   - Result: no module outside `command/` imports a single command's file;
     `-`, the workspace check and new-session creation exist once.
   - Tests: the moved tests pass where they land; `input_text` tests.

4. **Output helpers**
   - Status: done
   - Files: `src/output.rs`, `src/main.rs`, `src/command/kill.rs`,
     `src/command/common.rs`, `src/command/chat/mod.rs`,
     `src/command/chat/questions.rs`, `src/command/chat/talk.rs`,
     `src/text/sessions.rs`,
     `src/command/models.rs`
   - Result: every `clai: ` line of the command layer goes through `note`; both
     tables use `columns`.
   - Tests: `note` and `columns`; the table tests keep their expected strings.

5. **`run` into named parts**
   - Status: done
   - Files: `src/command/run.rs`
   - Result: `Run::run` reads as the five-line sequence above.
   - Tests: `tests/run.rs` unchanged; `who_answers` stays.

## Alternatives

- **Leave it as it is.** The layer works and the tests hold it together. But
  every new command pays the five-place cost and copies the helpers again, the
  TUI keeps depending on `ls` and `new`, and `run` grows further. Rejected.
- **A constant `NAME` per command** (the first draft of this RFC). It is simpler
  than a method, but `server` has three names. A constant would need a second
  `match` for it, and that is one of the two the RFC removes. Rejected.
- **`enum_dispatch` or a declarative macro.** These generate the one `match`.
  Rejected: a new dependency or a macro hides what one plain `match` shows, for
  thirteen arms.
- **A registry, where each file registers itself.** It avoids `cli.rs` knowing
  every command, but clap needs the enum anyway, so a second list only adds a
  place to forget. Rejected.
- **The dispatch stays in `command/mod.rs`.** It works, but then the enum and
  its only `match` live in two files, and a new variant is added in one and
  matched in the other. Rejected in favour of `cli.rs`.
- **Split `cli.rs` into one file per command's arguments.** The argument
  structs already live in the command files; `cli.rs` holds the enum, the
  top-level options and the parse checks, about 140 lines besides tests. Not
  the problem; rejected.
- **Fold `complete.rs` into `cli.rs`.** Completion has its own quietness rules
  (never an error, never output). Rejected: it stays its own module.

## Open questions

None left. The user asked to build it; the proposals were taken:

1. The shared module is `command::common`; `new.rs` keeps only its command.
2. The attached-session marks of `/sessions` (`*`) and the TUI (`✓` / `+`)
   stay with their clients; only the table moved to `text::sessions`.

## Notes from building

- **`Context::config()`:** commands that only pass the path on to
  `with_client` call `ctx.config()`, which gives `Option<&Path>` as before.
  `with_client` and `Connect` are unchanged.
- **Unused context:** `commit` and `completion` take it as `_: &Context`; no
  named parameter is left unused.
- **`clai run`'s workspace:** the shared check makes the path absolute
  before `Workspace::new` resolves it. Messages and behaviour are the same:
  `tests/run.rs` checks "missing: not a directory".
- **`input_text` for testing:** it reads standard input through a function
  given to `input_from`, so its tests need no terminal.
- **New pins:** the exact text of an empty `send` message (from words and
  from `-`), and `chat --list` with an unknown session. They were added
  before the move and passed on both sides of it.
