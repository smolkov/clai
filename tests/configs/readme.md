# Sample configurations

Configuration files for trying `clai` and for the tests in
`tests/configs.rs`. Keys and rules: [`spec/config.md`](../../spec/config.md).

| Directory | Content | Checked by the tests |
|-----------|---------|----------------------|
| `valid/` | User files that load: `minimal`, `ollama` (local model, no key), `full` (every key), `read-only` (the model may only read), `scripts` (for `clai run` in CI). | `clai --config FILE models` exits `0` with nothing on standard error. |
| `invalid/` | One mistake per file; `NAME.err` holds the exact errors, with `CONFIG` for the file's path. | Exit `2` and exactly those errors. |
| `project/` | `config.toml`: a project file that only makes things stricter. `loosens.toml`: one that tries to loosen them, with `loosens.err`. | `config.toml` loads with every valid user file; `loosens.toml` fails. |

Try one with:

```sh
clai --config tests/configs/valid/ollama.toml run "list the files"
```

To add a case, add a `.toml` file; for an invalid one also its `.err`,
e.g. from `clai --config FILE models 2>&1 | sed "s|FILE|CONFIG|"`.
