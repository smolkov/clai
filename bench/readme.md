# Token bench

Fixed tasks to measure what a change to requests costs and whether the
model still gets the work done (RFC 0028, "Measuring"). Each change that
claims to save tokens is run before and after on the same model; it is kept
only when no task is done less often and the tokens fall.

```sh
ci/bench.sh --model anthropic/sonnet --runs 3
ci/bench.sh --model local/qwen off-by-one
```

It needs a configured model and costs money: run it by hand, not in the
pipelines. The output is one tab-separated line per run:

| Column | Meaning |
|--------|---------|
| `task` | Directory under `bench/tasks`. |
| `run` | 1 to `--runs`. |
| `ok` | `yes` when the task's `check.sh` passes in the workspace afterwards. |
| `requests` | Model requests of the run. |
| `input` | All input tokens, cached or not. |
| `cached` | The part of `input` read from the prompt cache. |
| `output` | Output tokens. |

Models vary from run to run; compare sums over several runs
(`--runs 3` or more), not single lines.

## Tasks

Each task is a directory:

| File | Content |
|------|---------|
| `repo/` | The workspace. It is copied and committed to a fresh git repository for each run, so `check.sh` can use `git diff`. |
| `task.md` | The task, given to `codeagent run` on standard input. |
| `check.sh` | Run with `sh` in the workspace after the run; exit 0 means done. |

| Task | What it needs |
|------|---------------|
| `off-by-one` | Read a function, find a bug against its tests, fix it without touching the tests. |
| `add-function` | Add a function to a crate; hidden tests check it. |
| `find-the-caller` | Find where a function is called and answer in a file, changing nothing else. |

The tasks are small Rust crates without dependencies, so `cargo test
--offline` works everywhere. A new task should be solvable in a few
minutes by a person, have a check that cannot pass by accident, and
exercise reading, searching, editing or commands.

## Results

Write results below with the date, the model, the `codeagent` commit and the
sums per task, so later changes have a baseline.
