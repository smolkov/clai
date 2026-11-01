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
| `<workspace>/.clai/config.toml` | Project | Defaults and limits only: `default_model`, limits (keys not defined yet, see `agent-loop.md`), `[permissions]` (restricted, see [Permissions](#permissions)), `[commit]`. Not providers, models, `[tui]` or `[themes]`. |

`$XDG_CONFIG_HOME` replaces `~/.config` when it is set.

Providers and models are only allowed in the user file. A project file comes
with a repository the user may not trust; if it could define a provider, it
could send the user's API key or code to any address. A project file that
contains `[providers]` is rejected with an error naming the file.

TUI settings and themes are personal, so they are only allowed in the user
file too; a project file with `[tui]` or `[themes]` is rejected the same way.

Precedence, later wins: user file, project file, environment variables,
command-line options.

`--config FILE` replaces the user file. If `FILE` does not exist, that is a
load error (exit `2`), not the "no user config file" message.

The project file is `.clai/config.toml` in the workspace directory itself;
parent directories are not searched.

### Environment variables

| Variable | Overrides | Meaning |
|----------|-----------|---------|
| `CLAI_MODEL` | `default_model` | Model for new sessions, as a model reference. **(assumption)** |
| `CLAI_THEME` | `tui.theme` | Theme for this run. |
| `NO_COLOR` | — | Any value turns all colours off. |
| `CLAI_LOG` | — | Log level, see [Logging](#logging). |
| `XDG_CONFIG_HOME`, `HOME` | — | Where the user file is. A relative `XDG_CONFIG_HOME` is ignored. |

An empty value counts as not set. An invalid value is a load error that
names the variable, e.g. `CLAI_MODEL: unknown model `gpt``.

Format: TOML. Unknown keys are an error, not ignored, so typos are found.
**(assumption)**

## Permissions

Permissions decide, for every tool call of the model, whether it runs
without asking (`allow`), needs the user's answer (`ask`), or is refused
(`deny`). A refused call is reported to the model as denied; it never runs.
What each tool checks (which path, which parts of a command) is described in
[`tools.md`](tools.md), "Permissions".

```toml
[permissions]
# Action for each tool when no rule below matches.
read_file      = "allow"
list_files     = "allow"
search_files   = "allow"
edit_file      = "ask"
write_file     = "ask"
run_command    = "ask"

# Short rules: "TOOL" or "TOOL(PATTERN)". TOOL may be a group (read, edit).
allow = [
  "run_command(cargo build)",
  "run_command(cargo test *)",
  "run_command(git status)",
  "edit_file(src/**)",
]
ask = [
  "edit(Cargo.toml)",
]
deny = [
  "run_command(git push *)",
  "run_command(rm -rf *)",
  "edit(.git/**)",
]

# Directories outside the workspace the file tools may use. Absolute, or
# starting with ~/. Default: none.
extra_dirs = [
  "~/ws/shared-lib",                              # read and write
  { path = "~/ws/docs", access = "read" },        # read only
]

# Detailed rules: several patterns, exceptions, a message for the model.
[[permissions.rules]]
action  = "deny"
tools   = ["read"]
paths   = [".env*", "secrets/**", "~/.ssh/**"]
except  = [".env.example"]
message = "Secrets are not shown to the model. Ask the user for the value you need."

[[permissions.rules]]
action   = "allow"
tools    = ["run_command"]
commands = ["cargo *", "git diff *", "git log *", "git show *"]
except   = ["cargo publish *", "cargo install *"]
```

### Tool defaults

The values above are the defaults: reading is allowed, changing files and
running commands asks. A tool key may be `allow`, `ask` or `deny`. A tool not
named in `[permissions]` keeps its default. Unknown tool names are a load
error. The tools themselves are described in [`tools.md`](tools.md).

### Tool groups

A rule may name a group instead of a tool; it then applies to every tool of
the group. Groups are only allowed in rules, not as tool keys.

| Group | Tools |
|-------|-------|
| `read` | `read_file`, `list_files`, `search_files` |
| `edit` | `edit_file`, `write_file` |

So `deny = ["read(secrets/**)"]` keeps files below `secrets/` out of reading,
listing and searching alike.

### Rules

A short rule is `TOOL` (every call of that tool) or `TOOL(PATTERN)`, written
in the `allow`, `ask` or `deny` list. A detailed rule is a
`[[permissions.rules]]` table:

| Key | Required | Meaning |
|-----|----------|---------|
| `action` | yes | `allow`, `ask` or `deny`. |
| `tools` | yes | Tool names and groups, at least one. Either only file tools, or only `run_command`. |
| `paths` | no | Path patterns, for file tools. Omitted: every path. |
| `commands` | no | Command patterns, for `run_command`. Omitted: every command. |
| `except` | no | Patterns of the same kind; when one of them matches, the rule does not apply. |
| `message` | no | For `deny`: sent to the model with the denial. For `ask`: shown in the prompt and sent to the model if the user denies. Not allowed for `allow`. |

A rule matches a call when its tool is one of `tools`, one of its patterns
matches (or it has none), and none of `except` matches. The short rule
`deny = ["edit_file(src/**)"]` is the same as a detailed rule with
`action = "deny"`, `tools = ["edit_file"]`, `paths = ["src/**"]`.

#### Path patterns

For `read_file`, `list_files`, `search_files`, `edit_file`, `write_file`.
Patterns match the resolved path (see [`tools.md`](tools.md), "Paths").

| Pattern | Matches |
|---------|---------|
| Without `/`, e.g. `.env*`, `*.pem` | The file or directory name, in any directory (like `.gitignore`). |
| With `/`, e.g. `src/**`, `docs/*.md` | The path relative to the workspace. Never matches a path outside it. |
| Starting with `/` or `~/`, e.g. `~/.ssh/**` | The absolute path; this is how paths in `extra_dirs` are matched. `~` is expanded in patterns. |

Glob syntax: `*` any characters within one path segment, `**` any number of
segments, `?` one character, `[abc]` one of the characters, `{a,b}`
alternatives. `DIR/**` also matches `DIR` itself. Matching is
case-sensitive. A trailing `/` is not allowed. **(assumption)**

#### Command patterns

For `run_command`. A command line is first split into its parts (see
[Commands with several parts](#commands-with-several-parts)); each part is
matched on its own.

- A pattern matches the whole text of a part, with leading and trailing
  space removed and runs of unquoted space taken as one space. Quotes are
  kept as written: `git commit -m "x"` is matched with its quotes.
- `*` matches any characters, including spaces. A pattern that ends in
  ` *` also matches the command without arguments: `cargo test *` matches
  `cargo test` and `cargo test parse --release`. **(assumption)**
- Variable assignments before a command are part of its text:
  `cargo test *` does not match `RUST_LOG=debug cargo test`.
- `git *` matches `git push --force` too; name the subcommands you mean
  (`git diff *`, `git log *`) or add `except` patterns.

#### Commands with several parts

Commands that contain several commands (`;`, `&&`, `||`, `|`, `&`, newline,
`$( )`, backticks, subshells in `( )`) are split into their parts, also
inside the substitutions. Every part is checked on its own, and the call gets
the strictest result of its parts: one denied part denies the whole call, one
part that asks makes it ask. So `cargo test && rm -rf /` is not allowed by
`run_command(cargo test *)`.

Redirections are taken off the part before matching. Redirection to a file
(`> out.txt`, `>> log`, `2> err`) is checked as a `write_file` call on that
path, with the same boundary and rules; `/dev/null` and redirections between
file descriptors (`2>&1`) are always allowed. Redirection from a file
(`< in.txt`) is checked as a `read_file` call.

If a command cannot be split safely (unsupported syntax such as here
documents, `eval`, unbalanced quotes), it asks, and the prompt says why; a
`deny` rule that matches the whole text still denies. **(assumption)**

### Checking order

For a call:

1. Paths outside the workspace and `extra_dirs` are denied, whatever the
   rules say. Paths are resolved first (`..`, symbolic links), so a rule
   cannot be passed by spelling the path differently. `edit_file`,
   `write_file` and redirections into an `extra_dirs` entry with
   `access = "read"` are denied the same way.
2. If any `deny` rule matches: deny.
3. If any `ask` rule matches: ask.
4. If any `allow` rule matches, including rules the user added in the session
   (see below): allow.
5. Otherwise the tool's default action.

Short and detailed rules, and the rules of the user and the project file, are
all checked together; their order in the files does not matter. To make an
exception to a `deny` or `ask` rule, use `except`: an `allow` rule never
overrides them.

The rule that decided is kept with the decision, with its source (file and
line, or "session"). The permission prompt shows why it asks, e.g.
`asks: rule edit(Cargo.toml) (config.toml:24)` or `asks: default for
edit_file`; a denial tells the model which rule denied (see
[`tools.md`](tools.md), "Denied and cancelled calls"). `/permissions` lists
all rules with their source.

### Extra directories

Each entry of `extra_dirs` is a path or a table:

| Key | Required | Meaning |
|-----|----------|---------|
| `path` | yes | Directory, absolute or starting with `~/`. Resolved like tool paths when the configuration is loaded. |
| `access` | no | `write` (default): all file tools, with the same rules as the workspace. `read`: only `read_file`, `list_files`, `search_files`. **(assumption)** |

A plain string is the same as `{ path = "...", access = "write" }`. A
directory that does not exist is skipped with a warning in the log, so one
configuration can be used on several machines. **(assumption)**

`run_command` always starts in the workspace; `extra_dirs` and `access` do
not limit what a command may touch once it runs, only its redirections.
Limiting commands needs a sandbox (open question 5 in
[`readme.md`](readme.md)).

### Session rules

Pressing `a` ("always") in a TUI permission prompt adds an allow rule to the
session. The prompt proposes the narrowest rule, e.g. `run_command(cargo test
parse)` or `edit_file(src/parse.rs)`, and the user may widen it before
confirming, e.g. to `run_command(cargo test *)` or `edit_file(src/**)`. What
each tool proposes is described in [`tools.md`](tools.md), "Permissions".

Session rules are shown with `/permissions` and end with the session; they
are not written to a file. **(assumption)** `deny` and `ask` rules from the
configuration still win over session rules.

### Project files

A project file may only make permissions stricter, because it comes with the
repository:

- It may set tool keys to `ask` or `deny`, and add `ask` and `deny` rules,
  short or detailed, with `except` and `message`.
- It may not set a tool key to `allow`, add `allow` rules, or set
  `extra_dirs`. Such a file is rejected with an error naming the file and key.
- Its rules are added to the user file's rules; for a tool key, the stricter
  value wins (`deny` over `ask` over `allow`).

An `except` in a project file only narrows that file's own rule; it cannot
lift a rule of the user file.

### Who answers

A call that asks opens a question in the session, with everything needed to
decide; any attached client that can answer may answer it: the TUI,
`clai answer` in a terminal, or later a web UI (see
[`sessions.md`](sessions.md), "Questions"). A session without such a client
waits.

In `clai run` (one-shot) the question is asked on the terminal when standard
input is one. Without a terminal, or with `--no-ask`, `ask` is treated as
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

A theme may not set `base` to itself or to another custom theme; without
`base` it starts from `terminal`. A custom theme may not use the name of a
built-in theme. An unknown colour key or value is a load error.

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
| `api_key_cmd`  | no       | Command whose output is the API key, e.g. `pass show anthropic`. Run with `sh -c` once when the provider is first used; it must finish within 30 s. The trailing newline is removed. Empty output or a non-zero exit is an error that shows the command's standard error. **(assumption)** |
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
- an `openai-compatible` provider has no `base_url`;
- `base_url` does not start with `http://` or `https://`;
- a provider or model name is empty or contains `/`;
- a model has no `id` or `context_window`, or a number is out of range:
  `context_window` and `max_output_tokens` at least 1, `timeout_s` 1–86400,
  `max_retries` 0–100, `temperature` 0–2, `tui.max_fps` 1–120;
- `commit.model`, `tui.theme`, `CLAI_MODEL` or `CLAI_THEME` names something
  that is not defined;
- a permission rule names an unknown tool or group, is not of the form
  `TOOL` or `TOOL(PATTERN)`, or has an empty or invalid pattern;
- a detailed rule has no `action` or `tools`, mixes file tools and
  `run_command`, has `paths` for `run_command` or `commands` for file tools,
  or has a `message` with `action = "allow"`;
- a path pattern ends in `/`;
- an `extra_dirs` entry is not absolute or `~/`, or has an unknown `access`.

All errors found are reported, one per line; a TOML syntax error stops at
the first one.

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
