# Agent information file

`clai` AI-Coding-Agent

## Project files

| File                     | Purpose                                                        |
|--------------------------|----------------------------------------------------------------|
| `spec/`                  | Source of truth for behaviour, one file per topic. The user edits it directly. |
| `spec/readme.md`         | Start of the specification: purpose, goals, terms; it is the source of truth for `clai`. |
| `readme.md`              | User documentation.                                            |
| `implementation.md`      | Architecture, modules, dependencies and milestone order of the implementation. |
| `todo.md`                | Tasks and their status.                                        |

## Workflow

- Plan first: before changing files for a task, write a plan (what changes,
  in which files, which decisions the spec leaves open, how it is tested) and
  ask the user to approve it. Build only after the approval, and only what
  the plan says; a change of the plan needs a new approval. Questions and
  read-only investigation need no plan.
- Re-read the spec files of the task (index: `spec/readme.md`) before starting a task ("reload spec"); the user may have
  changed it since the last read. Implement what it says.
- When behaviour changes or a decision is made, update the spec file of that topic in the same change.
- If the spec is unclear or contradicts itself, ask instead of guessing.
- Update `readme.md` when commands, options, keys, configuration or the release
  process change.
- Update `todo.md` when tasks are identified or completed.

## Before committing

Run these and fix all findings:

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test
```

`ci/check.sh` (format check and `clippy`) and `ci/test.sh` (tests, also of
the `ci/` scripts) run the same as the pipelines do.

## Commits

- Commit or push only when asked.

## Markdown

- No numbers in headings.

## Never crash (outranks feature work)

`clai` runs sessions in a long-lived background server; a panic would end every
session of the user.

- Non-test code never panics. Forbidden: `unwrap()`, `expect()`, `panic!`,
  `unreachable!`, `todo!`, `unimplemented!`, unchecked indexing that can go out of
  bounds, and `unsafe`.
- Return the crate's error type through `Result` and `?`. Use `ok_or(..)?`,
  `let … else`, `if let`, or `.get()` for indexing.
- Use a fallback (`unwrap_or…`) only where it can't silently corrupt a document
  or a permission decision.
- An unfinished feature returns an "unsupported" error or is disabled.
- Sole exception: a provably infallible literal, written as
  `#[allow(clippy::expect_used)]` `.expect("why it can't fail")`.
