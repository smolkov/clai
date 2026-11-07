# Refactoring the command layer

- RFC: 0009
- Status: draft
- Milestone: none; it cleans the ground the "Context management" and
  "Commit" milestones build on
- Spec: [`spec/command.md`](../spec/command.md) (no change of behaviour);
  [`spec/architecture.md`](../spec/architecture.md), "Modules";
  [`implementation.md`](../implementation.md), "Modules"

## Summary

`src/cli.rs`, `src/command/` and `src/main.rs` are reordered without any
change of behaviour: one dispatch instead of two hand-written matches, a
small context in place of the threaded `--config` path, the helpers that
several commands copy (`-` from standard input, the workspace check, the
`clai: ` line on standard error, table alignment) in one place each, and
`clai run` split into its concerns. Every command, option, message, exit
code and completion works exactly as before.

## Motivation

The command layer works but has grown uneven, and more commands are coming
(`clai commit`, a possible `clai status` of RFC 0007):

- **Every new command touches the same five places:** the `Command` enum
  and its import in `cli.rs`, the `match` in `Command::name`, the `match`
  in `Command::run` in `command/mod.rs`, and the new file. All five must
  agree; nothing checks that they do.
- **`--config` is passed everywhere, also where it means nothing:**
  `run(&self, config: Option<&Path>)` reaches `commit` and `completion`,
  whose parameters are named `_config`. The signature invites more
  threading later (`CLAI_*` overrides, terminal detection).
- **The same helpers are copied, slightly differently:**
  - `-` reads standard input in `run` and in `send`, two near-identical
    blocks.
  - The workspace check is inline in `run`, a function `workspace` in
    `new`, with the same error text written twice.
  - `clai: name` lines on standard error: `kill` prints its own failures,
    `new` its note, `main::report` the rest — three spellings of the same
    idea.
  - Column alignment for a table exists twice: `ls::table` and
    `models::render_table`.
  - `chat` reaches into another command (`crate::command::ls::age`) for a
    time format that has nothing to do with `ls`.
- **`run` is one 80-line method** that reads the task, decides who answers,
  validates the workspace, loads the config, resolves the model, builds the
  provider, picks the asker and hands it all to a free `turn` function, all
  in `Run::run`.
- `run` sets up an agent in its own process (see
  [`architecture.md`](../spec/architecture.md), "Modules"); everything that
  wants a session instead talks to the server through `with_client`. The
  two partnerships are hard to see in the file.

Refactors without a behaviour change normally need no RFC (see
[`rfcs/readme.md`](readme.md), "When to write one"); this one is written
anyway because it spans the whole command layer — every file in
`src/command/`, `cli.rs` and `main.rs` — and because its choices, once
made, shape how later commands are added.

## Scope

In:

- Dispatch: a `run` per command file, `command/mod.rs` shrinks, no second
  match on the command list anywhere outside `cli.rs`.
- A `command::Context` created once in `main`, carrying what commands need:
  the `--config` path.
- Shared helpers in one place each: text-or-standard-input, the workspace
  check, the `clai: ` standard-error line, table alignment, the session
  age format.
- `run` divided into named parts with the roles visible: task, answering,
  setup, one-shot turn.
- The same tests as before, plus tests for what moves.

Out:

- No change of behaviour: syntax, output text, exit codes, prefixes,
  notes and completions stay as `spec/command.md` says. Where a detail is
  not specified, the today's wording stays.
- `clai chat` stays three files (`mod`, `talk`, `questions`); only its
  imports change.
- `src/client/`, the protocol and the server: untouched, apart from calls
  that get shorter (`with_client` stays as it is).
- `src/complete.rs`: unchanged.
- New options or commands, `spec/command.md` edits, `clai commit` (its own
  milestone under `spec/commit.md`).

## Design

### Dispatch

Today `Command::name` and `Command::run` in `command/mod.rs` both match all
thirteen variants; the enum is defined in `cli.rs`. After the change:

- Each command file implements one trait, next to its arguments:

  ```rust
  pub trait Named {
      /// The command's name, as used in messages.
      const NAME: &'static str;
  }

  impl Run for Kill { fn run(&self, ctx: &Context) -> Result<()> { … } }
  ```

  **(proposal for the shape; the file layout — arguments struct, `run`,
  tests in the same file — stays as the spec's "Modules" table lists it.)**

- `command/mod.rs` keeps one thing: the `match` that calls each `run` — or
  the `impl Command` moves to `cli.rs`, beside the enum it dispatches on,
  so one file owns the command list. **(proposal: move it to `cli.rs`; one
  command, one enum, one dispatch, all next to each other.)**
- `NAME` replaces `Command::name`'s match; `tracing::info!(command = …)`
  in `main` reads it from the enum once, with a light `match` that only
  names commands — the two duplicated full matches are gone.

The number of places to touch for a new command falls from five to three:
the enum, the dispatch beside it, and the new file itself.

### Context

`main` builds one `Context` and hands it to every `run`:

```rust
pub struct Context {
    /// The path given by `--config`, if any.
    pub config_file: Option<PathBuf>,
}
```

Commands that do not need it simply ignore the field; no `_config`
parameter. The context is the one place later features can grow (terminal
detection, environment overrides), without touching every signature again.
**(proposal)**

### Shared helpers

| Helper | Moves | Used by |
|--------|-------|---------|
| `text_or_stdin(words) -> Result<String>` | `command::Context` or `command::text` | `run`, `send` |
| `workspace(dir) -> Result<PathBuf>` | `command` (from `new`) | `run`, `new` |
| `stderr_line(text)` — the `clai: ` prefix | `output` (next to `write_stdout`) | `kill`, `new`, `main::report` |
| table column alignment | `output` | `ls`, `models` |
| `age(secs) -> String` | `text` (from `command::ls`) | `ls`, `chat` |

- `output` gains the standard-error twin of its `write_stdout`; every
  "clai: …" line goes through it, so the prefix is spelled once.
- Table alignment becomes a small `align(rows: &[Vec<String>]) -> String`
  in `output`; `ls` and `models` keep their own columns, wording and tests.
- `age` moves from `command/ls.rs` to `text`; `chat` no longer reaches into
  another command. **(proposal: `text`, next to `short_tokens`, which the
  two tables already share.)**

### `clai run`

`Run::run` gives way to named parts, each short and matched by a test:

- `task(words) -> Result<String>` — the task text, `-` from standard input,
  never empty.
- `answering(…) -> Answering` — as today.
- `workspace` and the model/provider resolution — through the shared
  helpers; the config load stays `config::load`.
- `one_shot(agent, asker,…) -> Result<()>` — the free `turn` function,
  renamed and wrapped with its signal handling and event rendering, so the
  in-process partnership (agent run here, not in the server) is a unit.

Nothing about what `run` does changes: same options, messages, exit codes
(`130`, `1`, `2`), same `Renderer` and `LineAsker`.

## Spec changes

None. `spec/command.md` lists commands and their behaviour, and all of it
stays; `spec/architecture.md`, "Modules" says "one file per command with its
argument struct and `run`" — that stays true. If the dispatch moves to
`cli.rs`, the module table's wording in `implementation.md` gains a
sentence. **(proposal)**

## Testing

The point of the refactor is that no behaviour changes, so the existing
tests are the test:

- `tests/cli.rs`, `tests/completion.rs`, `tests/models.rs`, `tests/run.rs`,
  `tests/server.rs` pass unchanged.
- Unit tests move with the code that moves: `ls::age`'s tests to `text`,
  the alignment helper's tests in `output`, `run`'s parts tested where they
  were tested (`answering` keeps its table test, `task` gains tests for
  `-` and the empty task).
- `cargo fmt --all`, `cargo clippy --all-targets -- -D warnings` and
  `cargo test` after every step, as `agent.md` says.

## Implementation plan

Each step leaves a working, tested state.

1. **Context and the standard-error line**
   - Status: todo
   - Files: `src/command/mod.rs` (new `Context`), `src/output.rs`
     (`stderr_line`), `src/main.rs`, all command files (signature change)
   - Result: commands take `&Context`; `kill` and `new` write their
     standard-error lines through one helper; `main::report` uses it too.
   - Tests: the existing command tests pass; `stderr_line` has its own.

2. **Helpers into one place**
   - Status: todo
   - Files: `src/command/run.rs`, `src/command/send.rs`,
     `src/command/new.rs`, `src/command/ls.rs`, `src/command/models.rs`,
     `src/command/chat/mod.rs`, `src/text/mod.rs`, `src/output.rs`
   - Result: `-` from standard input, the workspace check, column
     alignment and `age` exist once; `ls` keeps only its columns and JSON,
     `models` the same.
   - Tests: the moved tests pass where they land; all output tests
     unchanged in wording (`ls::table` and the two renderers keep their
     expected strings).

3. **Dispatch**
   - Status: todo
   - Files: `src/cli.rs`, `src/command/mod.rs`, `src/command/*.rs`,
     `src/main.rs`, `implementation.md`
   - Result: each command file has `const NAME` and its `run`; the two
     matches in `command/mod.rs` are gone; the dispatch lives next to the
     enum.
   - Tests: `unbuilt_commands_are_unsupported` and every parse test pass
     unchanged.

4. **`run` into named parts**
   - Status: todo
   - Files: `src/command/run.rs`
   - Result: `Run::run` reads as a short sequence: task, answering,
     workspace, setup, one shot.
   - Tests: `tests/run.rs` passes unchanged; new unit tests cover `task`
     and the moved setup.

## Alternatives

- **Leave it as it is.** The layer works and the tests hold it together.
  But every added command pays the five-place cost and copies the helpers
  again, and `run` grows further. Rejected: the coming commands would build
  on the uneven parts.
- **A macro that generates the dispatch.** One line per command, nothing to
  forget. Rejected: a declarative macro hides what is now plain to read,
  and the gain over one `match` next to the enum is small.
- **A command registry: each file registers itself in a list.** Avoids
  `cli.rs` knowing every command, but clap needs the enum anyway, so a
  second list only adds a place to forget. Rejected.
- **Split `cli.rs` into one file per command's arguments.** The argument
  structs are the interface to `spec/command.md`; keeping the enum and
  all options in one file makes the review of the surface easy. Rejected:
  `cli.rs` at ~400 lines of mostly declared options is not the problem.
- **Fold `complete.rs` into `cli.rs`.** Completion is its own concern with
  its own quietness rules (never an error, never output). Rejected: stays
  its own module.

## Open questions

1. Should the dispatch (`impl Command { name, run }`) move to `cli.rs`, as
   proposed, or stay in `command/mod.rs` as the module table says today?
   (Moving it means one sentence in `implementation.md`.)
2. The `Context` shape: a struct of the `--config` path now; should it
   already offer methods (terminal detection? `std::env` wrappers?) or
   stay bare until a second consumer needs more?
3. Should `Run` and `Send` share the `-` reading also with `clai chat
   NAME CHOICE deny MESSAGE...`, or does that path stay as it is, writing
   its own?
4. Table alignment in `output` as one generic function, or a thin `Row`
   builder per table, closer to what `ls` and `models` render today?
5. Is `one_shot` the right name for `run`'s turn-plus-signals unit, given
   the spec calls `clai run` one-shot ("run one task without the TUI and
   exit")?
