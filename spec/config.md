# Configuration

All providers and models `clai` can use are defined in the configuration.
`clai` has no built-in list of providers or models: a model that is not in
the configuration cannot be chosen, neither in the TUI nor on the command
line.

Statements marked **(assumption)** need a decision from the user.

## Files

| File                          | Scope   | May define |
|-------------------------------|---------|------------|
| `~/.config/clai/config.toml`  | User    | Everything: providers, models, defaults, `[permissions]`, `[commit]`, `[tui]`, `[themes]`. |
| `<workspace>/.clai/config.toml` | Project | Defaults and limits only: `default_model`, limits, `[permissions]` (restricted, see [Permissions](#permissions)), `[commit]`. Not providers, models, `[tui]` or `[themes]`. |

`$XDG_CONFIG_HOME` replaces `~/.config` when it is set.

Providers and models are only allowed in the user file. A project file comes
with a repository the user may not trust; if it could define a provider, it
could send the user's API key or code to any address. A project file that
contains `[providers]` is rejected with an error naming the file.

TUI settings and themes are personal, so they are only allowed in the user
file too; a project file with `[tui]` or `[themes]` is rejected the same way.

Precedence, later wins: user file, project file, environment variables,
command-line options.

Format: TOML. Unknown keys are an error, not ignored, so typos are found.
**(assumption)**

## Permissions

Permissions decide, for every tool call of the model, whether it runs
without asking (`allow`), needs the user's answer (`ask`), or is refused
(`deny`). A refused call is reported to the model as denied; it never runs.

```toml
[permissions]
# Action for each tool when no rule below matches.
read_file      = "allow"
list_files     = "allow"
search_files   = "allow"
edit_file      = "ask"
write_file     = "ask"
run_command    = "ask"

# Rules: "TOOL" or "TOOL(PATTERN)". First the deny rules are checked, then
# ask, then allow; the first matching list decides.
allow = [
  "run_command(cargo build)",
  "run_command(cargo test *)",
  "run_command(git status)",
  "edit_file(src/**)",
]
ask = [
  "read_file(.env*)",
]
deny = [
  "run_command(git push *)",
  "run_command(rm -rf *)",
  "edit_file(.git/**)",
]

# Directories outside the workspace the file tools may use, with the same
# rules as the workspace. Default: none.
extra_dirs = ["~/ws/shared-lib"]
```

### Tool defaults

The values above are the defaults: reading is allowed, changing files and
running commands asks. A tool key may be `allow`, `ask` or `deny`. A tool not
named in `[permissions]` keeps its default. Unknown tool names are a load
error. The tools themselves are described in `tools.md` (planned).

### Rules

A rule is `TOOL` (every call of that tool) or `TOOL(PATTERN)`.

| Tool | Pattern matches | Example |
|------|-----------------|---------|
| `read_file`, `list_files`, `search_files`, `edit_file`, `write_file` | The path, relative to the workspace, as a glob: `*` within one directory, `**` across directories. | `edit_file(src/**)`, `read_file(*.md)` |
| `run_command` | The command line, as a glob over the whole text: `*` matches any characters. | `run_command(cargo test *)` |

Checking order for a call:

1. Paths outside the workspace and `extra_dirs` are denied, whatever the
   rules say. Paths are resolved first (`..`, symbolic links), so a rule
   cannot be passed by spelling the path differently.
2. If any `deny` rule matches: deny.
3. If any `ask` rule matches: ask.
4. If any `allow` rule matches, including rules the user added in the session
   (see below): allow.
5. Otherwise the tool's default action.

Commands that contain several commands (`;`, `&&`, `||`, `|`, `$( )`,
backticks, redirection to a file) are split into their parts, and every part
must be allowed on its own; one part that needs asking makes the whole
command ask. So `cargo test && rm -rf /` is not allowed by
`run_command(cargo test *)`. If a command cannot be split safely, it asks.
**(assumption)**

### Session rules

Pressing `a` ("always") in a TUI permission prompt adds an allow rule to the
session, in the same syntax, e.g. `run_command(cargo test)` or
`edit_file(src/parse.rs)`. Session rules are shown with `/permissions` and
end with the session; they are not written to a file. **(assumption)**
`deny` rules from the configuration still win over session rules.

### Project files

A project file may only make permissions stricter, because it comes with the
repository:

- It may set tool keys to `ask` or `deny`, and add `ask` and `deny` rules.
- It may not set a tool key to `allow`, add `allow` rules, or set
  `extra_dirs`. Such a file is rejected with an error naming the file and key.
- Its rules are added to the user file's rules; for a tool key, the stricter
  value wins (`deny` over `ask` over `allow`).

### No one to ask

In `clai run` (one-shot) there is no TUI to answer, so `ask` is treated as
`deny`. `--yes` turns `ask` into `allow` for that run; `deny` rules and the
workspace boundary still apply. See [`command.md`](command.md).

## Commit

Settings for commit messages, used by `clai commit` and the TUI command
`/commit` (see [`commit.md`](commit.md)).
Allowed in the user and the project file.

```toml
[commit]
model   = "local/qwen"   # model for commit messages; default: session model or default_model
trailer = "Co-Authored-By: clai <clai@localhost>"   # added after the body; default: none
```

## TUI

Settings for the full-screen interface (see [`tui.md`](tui.md)). Only allowed
in the user file. All keys are optional; the values shown are the defaults.

```toml
[tui]
theme             = "terminal"  # terminal | dark | light | name of a [themes.NAME] table
symbols           = "auto"      # auto | unicode | ascii
mouse             = true        # capture the mouse; false keeps the terminal's text selection
tool_output_lines = 20          # lines of tool output shown before "… N more lines"; 0 = no limit
expand_tool_calls = false       # show tool calls expanded instead of collapsed
syntax_highlight  = true        # highlight code blocks and diffs
timestamps        = false       # show the time of each user message and answer
scroll_lines      = 3           # lines per mouse-wheel step
max_fps           = 30          # redraw limit, 1–120
```

| Key | Meaning |
|-----|---------|
| `theme` | Colour theme, see [Themes](#themes). |
| `symbols` | `auto` uses Unicode symbols (`✓ ✗ ▸ ▾`) when the locale is UTF-8, otherwise ASCII (`+ x > v`). |
| `mouse` | When `true`, the mouse wheel scrolls the conversation and clicks select entries; the terminal's own selection then needs `Shift` held (in most terminals). |
| `tool_output_lines` | Maximum lines of a tool's output shown in an expanded entry before it is cut. The full text is shown with `Enter` on the "… N more lines" marker. |
| `expand_tool_calls` | When `true`, finished tool calls stay expanded. Calls waiting for permission are always expanded. |
| `syntax_highlight` | Turn highlighting off for slow terminals or plain output. |
| `timestamps` | Prefix entries with `HH:MM`. |
| `scroll_lines` | Mouse-wheel scroll step. |
| `max_fps` | Upper limit for redraws per second. |

### Themes

A theme gives a colour to each part of the interface.

| Theme | Meaning |
|-------|---------|
| `terminal` | Uses only the terminal's 16-colour palette, so it follows the terminal's own theme. Default. |
| `dark` | Built-in theme for dark backgrounds, using 256 colours. |
| `light` | Built-in theme for light backgrounds, using 256 colours. |
| `NAME` | A theme defined in the configuration as `[themes.NAME]`. |

A custom theme starts from a built-in one and changes some colours:

```toml
[themes.solar]
base = "dark"               # theme to start from: terminal | dark | light
user        = "cyan"
accent      = "#e5a50a"
diff_add    = "#26a269"
diff_remove = "#c01c28"
code        = "Solarized (dark)"   # syntax highlighting theme
```

| Colour key | Used for |
|------------|----------|
| `text` | Normal text: model answers. |
| `dim` | Notices, hints, collapsed tool results. |
| `user` | User messages. |
| `accent` | Header, selected entries, active border. |
| `border` | Inactive borders and separators. |
| `selection` | Background of the selected row in lists. |
| `success` | `✓` and successful tool results. |
| `error` | `✗`, errors. |
| `warning` | `?` waiting for permission, `●` waiting sessions, `⚠` marks. |
| `diff_add` | Added lines in diffs. |
| `diff_remove` | Removed lines in diffs. |
| `code` | Name of the syntax highlighting theme for code blocks. |

Colour values: one of the 16 ANSI colour names (`black`, `red`, `green`,
`yellow`, `blue`, `magenta`, `cyan`, `white`, and `bright_` + each of them),
a 256-colour index (`0`–`255`), `"#rrggbb"`, or `"default"` (the terminal's
default foreground or background). In a terminal without true-colour support
(`COLORTERM` is not `truecolor` or `24bit`), `#rrggbb` is shown as the
nearest 256-colour value. **(assumption)**

A theme may not set `base` to itself or to another custom theme; an unknown
colour key or value is a load error.

The environment variable `NO_COLOR` (any value) turns all colours off,
whatever the theme. `CLAI_THEME` overrides `tui.theme` for one run.

The TUI reads `[tui]` and `[themes]` itself when it starts and when the user
file changes; a change applies at once to the running TUI. An invalid change
keeps the previous settings and shows the error in the status line.

## Providers and models

```toml
# Model used for new sessions unless another one is chosen.
# Written as "<provider>/<model>", using the names defined below.
default_model = "anthropic/sonnet"

[providers.anthropic]
api         = "anthropic"                 # API type, see below
api_key_env = "ANTHROPIC_API_KEY"         # environment variable with the key

[providers.anthropic.models.sonnet]
id                = "claude-sonnet-5-5"   # model id sent to the API
context_window    = 200000                # tokens
max_output_tokens = 64000

[providers.anthropic.models.opus]
id                = "claude-opus-5-5"
context_window    = 200000
max_output_tokens = 64000

[providers.local]
api      = "openai-compatible"
base_url = "http://localhost:11434/v1"    # e.g. Ollama; no key needed

[providers.local.models.qwen]
id             = "qwen3-coder:30b"
context_window = 32768
```

### Provider keys

| Key            | Required | Meaning |
|----------------|----------|---------|
| `api`          | yes      | API type: `anthropic` or `openai-compatible`. Decides which adapter encodes requests and decodes answers, see [`providers.md`](providers.md). **(assumption: these two first)** |
| `base_url`     | no       | API address. Default: the official address of the API type. Required for `openai-compatible`. |
| `api_key_env`  | no       | Name of the environment variable that holds the API key. |
| `api_key_cmd`  | no       | Command whose output is the API key, e.g. `pass show anthropic`. Run once when the provider is first used. **(assumption)** |
| `timeout_s`    | no       | Request timeout in seconds. Default 300. |
| `max_retries`  | no       | Retries for network errors and rate limits. Default 3. |

At most one of `api_key_env` and `api_key_cmd` is set. With neither, requests
are sent without a key. The key itself is never stored in a config file and
never written to a log or shown in the TUI.

### Model keys

| Key                 | Required | Meaning |
|---------------------|----------|---------|
| `id`                | yes      | Model id the provider's API expects. |
| `display_name`      | no       | Name shown in the TUI. Default: the model's config name. |
| `context_window`    | yes      | Maximum context size in tokens; used for the header gauge and history truncation. |
| `max_output_tokens` | no       | Maximum tokens per answer. Default: provider default. |
| `temperature`       | no       | Sampling temperature. Default: provider default. |

### Model references

A model is referred to as `<provider>/<model>`, using the names of the config
tables, e.g. `anthropic/opus` or `local/qwen`. This is the form used by
`default_model`, `--model`, `/model`, `clai ls` and the TUI. If only one
provider defines a model with that name, the provider part may be left out
(`--model opus`). **(assumption)**

## Validation

The configuration is checked when it is loaded. Errors name the file, line and
key, e.g. `config.toml:14: providers.local.models.qwen: missing context_window`.

It is an error when:

- no provider or no model is defined;
- `default_model` is missing or does not name a defined model;
- a provider has an unknown `api`, or both `api_key_env` and `api_key_cmd`;
- an `openai-compatible` provider has no `base_url`.

A missing API key (variable not set, command fails) is not a load error; it
is reported when the provider is first used, in the session that uses it, so
other providers still work.

If there is no user config file at all, `clai` explains where to create it
and prints an example, then exits with code `2`.

## Logging

`clai` writes a log to `$XDG_STATE_HOME/clai/clai.log` (default
`~/.local/state/clai/clai.log`), appending to it. It never logs to the
terminal. **(assumption)**

- The environment variable `CLAI_LOG` sets the level filter, e.g. `debug`
  or `clai=trace` (syntax of `tracing`'s `EnvFilter`). Default `info`; an
  invalid value falls back to `info`.
- If the log file cannot be opened, `clai` prints one warning on standard
  error and runs without a log.
- API keys and other secrets are never written to the log.

## Reloading

The server reads the configuration when it starts and again when the user
file changes. **(assumption)**

- A valid new configuration applies to new sessions and to the model pickers
  at once.
- An invalid new configuration is rejected; the server keeps the previous one
  and shows the errors in all attached TUIs.
- A running session keeps its provider and model even if they were removed or
  changed in the configuration; its next model change must pick a model that
  exists. The TUI marks such a model as "not in config".
- The project file is read when a session is created in that workspace.
