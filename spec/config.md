# Configuration

All providers and models `clai` can use are defined in the configuration.
`clai` has no built-in list of providers or models: a model that is not in
the configuration cannot be chosen, neither in the TUI nor on the command
line.

Statements marked **(assumption)** need a decision from the user.

## Files

| File                          | Scope   | May define |
|-------------------------------|---------|------------|
| `~/.config/clai/config.toml`  | User    | Everything: providers, models, defaults, `[limits]`, `[permissions]`, `[commit]`, `[tui]`, `[themes]`. |
| `<workspace>/.clai/config.toml` | Project | Defaults and limits only: `default_model`, `[limits]` (only lower, see [Limits](#limits)), `[permissions]` (restricted, see [Permissions](#permissions)), `[commit]`. Not providers, models, `[tui]` or `[themes]`. |

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
# Action for each tool when no rule matches.
read_file      = "allow"
list_files     = "allow"
search_files   = "allow"
edit_file      = "ask"
write_file     = "ask"
run_command    = "ask"

# Directories outside the workspace the file tools may use. Absolute, or
# starting with ~/. Default: none.
extra_dirs = [
  "~/ws/shared-lib",                              # read and write
  { path = "~/ws/docs", access = "read" },        # read only
]

# Rules, grouped by action. Key: a tool or a group; value: its patterns.
[permissions.allow]
run_command = [
  "ls", "cat", "head", "wc", "grep", "rg",
  "cargo build", "cargo test", "cargo clippy", "cargo fmt",
  "git status", "git diff", "git log", "git show",
]
edit = ["src/**", "tests/**"]

[permissions.ask]
run_command = ["git push", "git commit", "cargo publish"]
edit        = ["Cargo.toml", ".github/**"]

[permissions.deny]
run_command = [
  "rm", "sudo", "curl ** | sh",
  "git push --force", "git push -f", "git reset --hard", "git clean",
]
edit = [".git/**"]
read = [".env*", "secrets/**", "~/.ssh/**"]

# Detailed rules: exceptions and a message for the model.
[[permissions.rules]]
action  = "deny"
tools   = ["read"]
paths   = [".env*"]
except  = [".env.example"]
message = "Secrets are not shown to the model. Ask the user for the value you need."

[[permissions.rules]]
action   = "allow"
tools    = ["run_command"]
commands = ["cargo"]
except   = ["cargo publish", "cargo install", "cargo login"]
```

With these settings `ls -la src` and `cargo test parse --release` run
without asking, `git push origin main` asks, and `rm -rf target`,
`git push --force` and `sudo make install` are denied. A command that no
rule names, e.g. `make`, asks (the `run_command` default).

### Tool defaults

The values above are the defaults: reading is allowed, changing files and
running commands asks. A tool key may be `allow`, `ask` or `deny`. A tool not
named in `[permissions]` keeps its default. Unknown tool names are a load
error. The tools themselves are described in [`tools.md`](tools.md).

`ask_user` (questions from the model, see [`tools.md`](tools.md),
"ask_user") is `allow` or `deny`, default `allow`; `ask` is a load error,
and rules cannot name it. `deny` leaves the tool out of every request. A
project file may set it to `deny`, not to `allow`. Like the other keys, a
change applies to sessions started after it.

### Tool groups

A rule may name a group instead of a tool; it then applies to every tool of
the group. Groups are only allowed in rules, not as tool keys.

| Group | Tools |
|-------|-------|
| `read` | `read_file`, `list_files`, `search_files` |
| `edit` | `edit_file`, `write_file` |

So `read = ["secrets/**"]` in `[permissions.deny]` keeps files below
`secrets/` out of reading, listing and searching alike.

### Rules

Rules are written in one of two forms:

- **By action:** the tables `[permissions.allow]`, `[permissions.ask]` and
  `[permissions.deny]`. Each key is a tool or a group, each value a list of
  patterns. `"**"` as a path pattern, or `"*"` as a command pattern,
  matches every call of the tool.
- **Detailed:** a `[[permissions.rules]]` table, for exceptions and
  messages:

  | Key | Required | Meaning |
  |-----|----------|---------|
  | `action` | yes | `allow`, `ask` or `deny`. |
  | `tools` | yes | Tool names and groups, at least one. Either only file tools, or only `run_command`. |
  | `paths` | no | Path patterns, for file tools. Omitted: every path. |
  | `commands` | no | Command patterns, for `run_command`. Omitted: every command. |
  | `except` | no | Patterns of the same kind; when one of them matches, the rule does not apply. |
  | `message` | no | For `deny`: sent to the model with the denial. For `ask`: shown in the question and sent to the model if the user denies. Not allowed for `allow`. |

A rule matches a call when its tool is one of its tools, one of its patterns
matches (or it has none), and none of `except` matches. Each pattern in the
action tables is a rule of its own: `run_command = ["rm", "sudo"]` in
`[permissions.deny]` is the same as two detailed rules with
`action = "deny"`, `tools = ["run_command"]` and `commands = ["rm"]` or
`["sudo"]`.

Rules are shown, in questions, denials, `/permissions` and logs, in the
notation `ACTION TOOL(PATTERN)`, e.g. `deny run_command(rm)` or
`ask edit(Cargo.toml)`.

#### Path patterns

For `read_file`, `list_files`, `search_files`, `edit_file`, `write_file`.
Patterns match the resolved path (see [`tools.md`](tools.md), "Paths").

| Pattern | Matches |
|---------|---------|
| Without `/`, e.g. `.env*`, `*.pem` | The file or directory name, in any directory (like `.gitignore`). |
| With `/`, e.g. `src/**`, `docs/*.md`, `./Cargo.toml` | The path relative to the workspace; a leading `./` is dropped, so `./Cargo.toml` is only the file in the workspace itself. Never matches a path outside it. |
| Starting with `/` or `~/`, e.g. `~/.ssh/**` | The absolute path; this is how paths in `extra_dirs` are matched. `~` is expanded in patterns. |

Glob syntax: `*` any characters within one path segment, `**` any number of
segments, `?` one character, `[abc]` one of the characters, `{a,b}`
alternatives. `DIR/**` also matches `DIR` itself, and `./**` the whole
workspace. A pattern that matches a directory also matches everything in
it, so `secrets` or `docs/private` cover the files below them. Matching is
case-sensitive. A trailing `/` is not allowed. **(assumption)**

A path in an extra directory is matched by name patterns against its path
below that directory, and by absolute patterns; relative patterns never
match it.

#### Command patterns

For `run_command`. A command line is first split into its parts (see
[Commands with several parts](#commands-with-several-parts)); each part is
matched on its own, word by word. **(assumption)**

- The part is split into words like the shell does: quotes and backslashes
  are removed, so `git commit -m "fix parse"` has the words `git`, `commit`,
  `-m`, `fix parse`. The pattern is split the same way.
- A pattern matches when its words match the **first** words of the part.
  Further words are allowed: `git push` matches `git push`,
  `git push origin main` and `git push --force`; `rm` matches every `rm`
  command. It does not match `git status` or `rmdir`.
- Each pattern word is a glob over one word: `*` any characters, `?` one
  character, `[abc]`, `{a,b}`. So `ls*` matches `ls`, `lsof` and `lsblk`;
  `cargo {build,check}` matches both subcommands; `*` alone matches every
  command.
- The word `**` matches any number of words, also none, so a pattern can
  name words further on: `git ** --force` matches `git push origin --force`;
  `find ** -delete` and `find ** -exec` catch those uses of `find`.
- For `deny` and `ask` rules, the first word also matches by its file name:
  `rm` denies `/bin/rm` and `./rm`. `allow` rules match the command as
  written, so `ls` does not allow a script `./ls` in the repository. A
  pattern whose first word contains `/` matches only that path.
- Variable assignments before a command (`RUST_LOG=debug cargo test`) are
  words too. They are skipped for `deny` and `ask` rules, so `rm` still
  denies `FOO=1 rm -rf x`; an `allow` rule matches only if it names them
  (`RUST_LOG=* cargo test`).
- Wrappers that run another command — `sudo`, `doas`, `env`, `command`,
  `builtin`, `exec`, `nice`, `nohup`, `time`, `timeout`, `xargs`, `watch`,
  `setsid`, `stdbuf`, `ionice` — are looked through for `deny` and `ask`
  rules: every word after the wrapper is tried as the start of the wrapped
  command, so `sudo -u root rm -rf /`, `timeout 10 rm x` and `xargs rm` are
  denied by `rm`, whatever options the wrapper has. An `allow` rule must
  match the part as written, so `ls` does not allow `sudo ls`.
- `sh -c`, `bash -c`, `zsh -c`, `dash -c` and `ksh -c` (also with joined
  flags such as `-ec`) with a literal string: the string is split and
  checked like a command line of its own, and the shell part itself only
  against `deny` and `ask` rules. With anything else (a variable, a
  substitution) the part asks.
- A command name that is only known when the command runs (`$CMD x`,
  `` `which ls` ``) asks, unless a `deny` or `ask` rule matches the part.
- Order matters only among words, not among rules: a more specific `allow`
  never beats a broader `deny` or `ask`. To allow `cargo` except
  `cargo publish`, deny or ask for `cargo publish`, or use `except`.

| Pattern | Matches | Does not match |
|---------|---------|----------------|
| `ls` | `ls`, `ls -la src` | `lsof`, `sudo ls` (for `allow`) |
| `ls*` | `ls`, `lsof -i`, `lsblk` | `cat ls` |
| `rm` | `rm x`, `rm -rf /`; for `deny` and `ask` also `/bin/rm x`, `sudo rm x`, `FOO=1 rm x` | `rmdir x` |
| `git push` | `git push`, `git push -f origin` | `git pull`, `git status` |
| `git ** --force` | `git push --force`, `git push origin main --force` | `git push -f` |
| `cargo {build,test}` | `cargo build --release`, `cargo test parse` | `cargo run` |
| `*` | every command | — |

#### Commands with several parts

Commands that contain several commands (`;`, `&&`, `||`, `|`, `&`, newline,
`$( )`, backticks, subshells in `( )`) are split into their parts, also
inside the substitutions. Every part is checked on its own, and the call gets
the strictest result of its parts: one denied part denies the whole call, one
part that asks makes it ask. So `cargo test && rm -rf /` is not allowed by
`cargo test`, and is denied by `rm`.

A pattern with `|`, like `curl ** | sh`, is the exception: it is matched
against a pipeline as a whole, and only `deny` and `ask` rules may use it.
Its pieces must match consecutive commands of one pipeline, in order, each
like a `deny` rule does (file name, wrappers): `curl ** | sh` matches
`cat x | curl -d @- y | sudo sh`. The result applies to the first command
of the pipeline. **(assumption)**

Shell keywords at the start of a part are skipped, so the commands inside
`if`, `while` and `{ }` are checked: `if test -f x; then rm x; fi` has the
parts `test -f x` and `rm x`. **(assumption)**

Redirections are taken off the part before matching. Redirection to a file
(`> out.txt`, `>> log`, `2> err`, `&> all`) is checked as a `write_file`
call on that path, with the same boundary and rules; `~/` is expanded as the
shell does. `/dev/null`, `/dev/stdout`, `/dev/stderr`, `/dev/stdin`,
`/dev/tty` and redirections between file descriptors (`2>&1`) are always
allowed. Redirection from a file (`< in.txt`) is checked as a `read_file`
call. A redirection whose file is only known when the command runs
(`> $OUT`) asks.

If a command cannot be split safely, it asks, and the question says why; a
`deny` rule that matches the start of the text still denies. Not split
safely are: here documents, `for`, `case`, `select`, `function`, `coproc`,
`eval`, `trap`, function definitions, and unbalanced quotes, parentheses or
backticks. **(assumption)**

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

The built-in rules (see below) are checked with the others, in step 2.

Rules from the action tables and detailed rules, and the rules of the user
and the project file, are all checked together; their order in the files does not matter. To make an
exception to a `deny` or `ask` rule, use `except`: an `allow` rule never
overrides them.

The rule that decided is kept with the decision, with its source (file and
line, "session" or "built-in"). The permission prompt shows why it asks, e.g.
`asks: rule ask edit(Cargo.toml) (config.toml:31)` or `asks: default for
edit_file`; a denial tells the model which rule denied (see
[`tools.md`](tools.md), "Denied and cancelled calls"). `/permissions` lists
all rules with their source, the built-in rules too.

A `run_command` part that would be allowed (by a rule or by the default)
asks instead if one of its plain arguments names a file inside the
workspace or `extra_dirs` that a `deny` rule keeps from reading, e.g.
`cat .env`. A plain argument is a word after the command name that does not
start with `-` and is not built at run time. It asks rather than denies,
because a word is not always a path (`grep .env README.md` searches for
the text). The prompt says `asks: .env is denied for reading by rule deny
read(.env*) (config.toml:12)` and offers no "always": no rule can make the
call allowed. Globs (`cat .e*`), directories (`grep -r KEY .`) and options
(`--file=.env`) are not caught; limiting what commands read needs a
sandbox (open question 5 in [`readme.md`](readme.md)).

### Built-in rules

`clai` ships three `deny` rules for the `read` group, for files whose
content is almost always a secret:

| Rule | Patterns | Except |
|------|----------|--------|
| environment files | `.env`, `.env.*` | `.env.example`, `.env.sample`, `.env.template`, `.env.dist` |
| key files | `*.pem`, `*.key`, `*.p12`, `*.pfx`, `id_rsa`, `id_ecdsa`, `id_ed25519` | |
| credential files | `.netrc`, `.pgpass` | |

The patterns have no `/`, so they match the name in any directory of the
workspace and of `extra_dirs`. Files outside them are denied anyway.

They are `deny` rules like any other: an `allow` rule, a session rule or
`--yes` cannot pass them, and listing and searching leave the files out.
Their source is shown as `built-in`, and the model gets this message with
the denial:

```
this file usually holds secrets and is not shown to the model. Ask the
user for the value you need; the user can add the path to
permissions.builtin_except.
```

The user file can make exceptions with `builtin_except`, a list of path
patterns added to the `except` of every built-in rule:

```toml
[permissions]
builtin_except = ["deploy/certs/*.pem", ".env.test"]
```

It lifts only the built-in rules: a `deny` rule of the user or project
file on the same path still denies.

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
confirming, e.g. to `run_command(cargo test)` or `edit_file(src/**)`. What
each tool proposes is described in [`tools.md`](tools.md), "Permissions".

Session rules are shown with `/permissions` and end with the session; they
are not written to a file. **(assumption)** `deny` and `ask` rules from the
configuration still win over session rules.

### Project files

A project file may only make permissions stricter, because it comes with the
repository:

- It may set tool keys to `ask` or `deny`, and add rules in
  `[permissions.ask]` and `[permissions.deny]` or detailed `ask` and `deny`
  rules, with `except` and `message`.
- It may not set a tool key to `allow`, have a `[permissions.allow]` table,
  add detailed `allow` rules, set `ask_user` to `allow`, or set
  `extra_dirs` or `builtin_except`. Such a file is rejected with an error
  naming the file and key.
- Its rules are added to the user file's rules; for a tool key, the stricter
  value wins (`deny` over `ask` over `allow`).

An `except` in a project file only narrows that file's own rule; it cannot
lift a rule of the user file.

### Who answers

A call that asks opens a question in the session, with everything needed to
decide; any attached client that can answer may answer it: the TUI,
`clai chat` in a terminal, or later a web UI (see
[`sessions.md`](sessions.md), "Questions"). A session without such a client
waits.

In `clai run` (one-shot) the question is asked on the terminal when standard
input is one. Without a terminal, or with `--no-ask`, `ask` is treated as
`deny`. `--yes` turns `ask` into `allow` for that run; `deny` rules and the
workspace boundary still apply. See [`command.md`](command.md).

## Limits

```toml
[limits]
max_requests   = 50     # model requests per turn before the limit question
max_tool_calls = 120    # tool calls per turn before the limit question
```

Both keys are 1–10000. A project file may set them too; for each key the
lower of the user and the project value is used. What they count and what
happens at the limit: [`agent-loop.md`](agent-loop.md), "Limits".
**(assumption)**

## Commit

Settings for commit messages, used by `clai commit` and the TUI command
`/commit` (see [`commit.md`](commit.md)).
Allowed in the user and the project file.

```toml
[commit]
model   = "local/qwen"   # model for commit messages; default: default_model (clai commit), the session model (/commit)
trailer = "Co-Authored-By: clai <clai@localhost>"   # added after the body; default: none
```

## TUI

Settings for the full-screen interface (see [`tui.md`](tui.md)). Only allowed
in the user file. All keys are optional; the values shown are the defaults.

```toml
[tui]
theme             = "terminal"  # terminal | dark | light | a bundled theme | a [themes.NAME] table or themes/NAME.toml
symbols           = "auto"      # auto | unicode | ascii
mouse             = true        # capture the mouse: wheel, drag to copy, click; false keeps the terminal's selection
tool_output_lines = 20          # lines of tool output shown before "… N more lines"; 0 = no limit
tool_preview      = "changes"   # changes | all | none: what finished tool calls show
clipboard         = "auto"      # auto | osc52 | command | off: how copied text reaches the clipboard
syntax_highlight  = true        # colour code blocks, diffs, new files and commands by their language
timestamps        = false       # show the time of each user message and answer
scroll_lines      = 3           # lines per mouse-wheel step
max_fps           = 30          # redraw limit, 1–120
activity          = "playful"   # playful | plain | off: the line while the session works
```

| Key | Meaning |
|-----|---------|
| `theme` | Colour theme, see [Themes](#themes). |
| `symbols` | `auto` uses Unicode symbols (`✓ ✗ ▸ ▾`) when the locale is UTF-8, otherwise ASCII (`+ x > v`). |
| `mouse` | When `true`, the mouse wheel scrolls the conversation, a drag selects and copies text, and a click on a tool call expands it ([`tui.md`](tui.md), "Copy"); the terminal's own selection then needs `Shift` held (in most terminals). |
| `tool_output_lines` | Maximum lines of a tool's output shown in an expanded entry or a preview before it is cut. The full text is shown with `Enter` on the call, and in the details view (`o`). |
| `tool_preview` | What finished tool calls show below their summary ([`tui.md`](tui.md), "Conversation entries"): `changes` the diff of edits and writes, the last lines of commands and the error of failed calls; `all` every call expanded; `none` one line each. Calls waiting for permission are always expanded. |
| `expand_tool_calls` | Deprecated: `true` reads as `tool_preview = "all"`; `tool_preview` wins when both are set. |
| `clipboard` | How copied text reaches the clipboard ([`tui.md`](tui.md), "Copy"): `auto` the terminal's OSC 52 sequence and, when the session is local, `wl-copy`, `xclip`, `xsel` or `pbcopy`; `osc52` only the sequence; `command` only the command; `off` nothing. |
| `syntax_highlight` | Colour code by its language ([`tui.md`](tui.md), "Syntax highlighting"); `false` for slow terminals or plain output. |
| `timestamps` | Prefix entries with `HH:MM`. |
| `scroll_lines` | Mouse-wheel scroll step. |
| `max_fps` | Upper limit for redraws per second. |
| `activity` | The activity line above the input while the session works ([`tui.md`](tui.md), "Activity line"). `playful`: a playful verb per turn with a shimmer; `plain`: `Thinking…` and `Writing…`; `off`: no line (tool call spinners still move). |

### Themes

A theme gives a colour to each part of the interface.

| Theme | Meaning |
|-------|---------|
| `terminal` | Uses only the terminal's 16-colour palette, so it follows the terminal's own theme. Default. |
| `dark` | Built-in theme for dark backgrounds, using 256 colours. |
| `light` | Built-in theme for light backgrounds, using 256 colours. |
| `atom-one-dark`, `nord`, `gruvbox-dark`, `solarized-light` | Bundled with `clai`, see [Theme files](#theme-files). |
| `NAME` | A theme defined as `[themes.NAME]` in the user file, or as a theme file `themes/NAME.toml`. |

`dark` and `light` use the 256 colours:

| Key | `dark` | `light` |
|-----|--------|---------|
| `text` | 252 | 235 |
| `dim` | 244 | 245 |
| `user` | 117 bold | 25 bold |
| `accent` | 75 | 26 |
| `border` | 240 | 250 |
| `selection` | background 238 | background 254 |
| `success`, `diff_add` | 114 | 28 |
| `error`, `diff_remove` | 203 bold | 160 bold |
| `warning` | 221 | 136 |

`terminal` uses the 16 colours, and the dim and reversed attributes for `dim`,
`border` and `selection`; a custom theme that sets one of these uses the
colour instead.

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
| `background` | Painted over the whole screen when set; text without a colour then gets the `text` colour. Without it the terminal's background stays, as for the three built-in themes. |
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
| `code` | The syntax theme for code: `ansi` (on the terminal's 16 colours), `atom-one-dark`, `nord`, `gruvbox-dark` (the syntax colours of the bundled themes of that name), `base16-ocean.dark`, `base16-eighties.dark`, `base16-mocha.dark`, `base16-ocean.light`, `InspiredGitHub`, `Solarized (dark)` or `Solarized (light)`. Without it: `ansi` over `terminal`, `base16-ocean.dark` over `dark`, `InspiredGitHub` over `light`. Another name is an error. |

Colour values: one of the 16 ANSI colour names (`black`, `red`, `green`,
`yellow`, `blue`, `magenta`, `cyan`, `white`, and `bright_` + each of them),
a 256-colour index (`0`–`255`), `"#rrggbb"`, or `"default"` (the terminal's
default foreground or background). In a terminal without true-colour support
(`COLORTERM` is not `truecolor` or `24bit`), `#rrggbb` is shown as the
nearest of the 6×6×6 cube and the grey ramp of the 256 colours.

A theme may not set `base` to itself or to another custom theme; without
`base` it starts from `terminal`. A custom theme may not use the name of a
built-in theme. An unknown colour key or value is a load error.

The environment variable `NO_COLOR` (any value) turns all colours off,
whatever the theme. `CLAI_THEME` overrides `tui.theme` for one run.

### Theme files

A theme can also be a file of its own, so it can be shared and copied without
editing the configuration.

- **Directory:** `themes/` next to the user file, so
  `$XDG_CONFIG_HOME/clai/themes/` by default, or next to the `--config` file.
  Each `NAME.toml` there is the theme `NAME`; a name has letters, digits, `-`
  and `_`. Other files are ignored, and so is a missing directory.
- **Content:** the keys of a `[themes.NAME]` table, at the top level:

  ```toml
  # ~/.config/clai/themes/nord.toml
  base        = "dark"
  background  = "#2e3440"
  text        = "#d8dee9"
  accent      = "#81a1c1"
  ```

- **Bundled:** `atom-one-dark`, `nord`, `gruvbox-dark` and `solarized-light` come
  with `clai`, in this format (`themes/` in the source), so one can be copied
  into the directory and changed.
- **Precedence:** a `[themes.NAME]` table, then a file, then a bundled theme. A
  file `nord.toml` replaces the bundled `nord`, and so does a table. A table and
  a file of the same name are an error. `terminal`, `dark` and `light` cannot be
  replaced.
- `base` is one of `terminal`, `dark`, `light`, as for tables.
- **Errors** name the theme file and line. A broken theme file is a load error
  only if it is the theme in use (`tui.theme` or `CLAI_THEME`); otherwise it is
  left out, and the TUI's theme list shows it with its error. A broken file
  does not fall back to a bundled theme of its name.
- `clai` never writes into the directory.

### Reading and writing `[tui]`

The TUI reads `[tui]`, `[themes]` and the theme files itself: when it starts,
on `config_reloaded`, and when `/settings` or `/theme` opens. A change applies
at once to the running TUI. An invalid change keeps the previous settings and
shows the error in the status line.

The TUI writes the user file only from `/settings` and `/theme` (see
[`tui.md`](tui.md), "Settings"): it sets or removes keys of `[tui]` and keeps
the rest of the file, comments and order included; it checks the new text as
loading would and writes nothing if it fails or if the file changed since it
was read; it replaces the file atomically with its mode kept, through a link to
its target.

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
| `api_key`      | no       | The API key itself. Only for a user file that nobody else can read; prefer `api_key_env` or `api_key_cmd`. Must not be empty. |
| `api_key_env`  | no       | Name of the environment variable that holds the API key. |
| `api_key_cmd`  | no       | Command whose output is the API key, e.g. `pass show anthropic`. Run with `sh -c` once when the provider is first used; it must finish within 30 s. The trailing newline is removed. Empty output or a non-zero exit is an error that shows the command's standard error. **(assumption)** |
| `timeout_s`    | no       | Request timeout in seconds. Default 300. |
| `max_retries`  | no       | Retries for network errors and rate limits. Default 3. |

At most one of `api_key`, `api_key_env` and `api_key_cmd` is set. With none,
requests are sent without a key. Providers, and so `api_key`, are only allowed
in the user file, never in a project file. The key is never written to a log
or shown in the TUI.

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
- a provider has an unknown `api`, more than one of `api_key`, `api_key_env`
  and `api_key_cmd`, or an empty `api_key`;
- an `openai-compatible` provider has no `base_url`;
- `base_url` does not start with `http://` or `https://`;
- a provider or model name is empty or contains `/`;
- a model has no `id` or `context_window`, or a number is out of range:
  `context_window` and `max_output_tokens` at least 1, `timeout_s` 1–86400,
  `max_retries` 0–100, `temperature` 0–2, `tui.max_fps` 1–120,
  `limits.max_requests` and `limits.max_tool_calls` 1–10000;
- `commit.model`, `tui.theme`, `CLAI_MODEL` or `CLAI_THEME` names something
  that is not defined;
- a key in `[permissions.allow]`, `[permissions.ask]` or
  `[permissions.deny]` is not a tool or group, its value is not a list of
  strings, or a pattern is empty or invalid;
- a command pattern has unbalanced quotes, or uses `|` in an `allow` rule;
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
- API keys and other secrets are never written to the log, and neither
  are file contents or command output.
- The log file is created with mode `0600` and its directory with mode
  `0700`; a wider mode of an existing file or directory is narrowed when
  `clai` starts.

## Reloading

The server reads the configuration when it starts and again when the user
file changes. It checks the file's modification time and size every 2 s;
an editor that replaces the file is noticed the same way.

- A valid new configuration applies to new sessions and to the model pickers
  at once; every client gets the notice `config_reloaded`.
- An invalid new configuration is rejected; the server keeps the previous one
  and sends the errors to every client (`config_rejected`), which shows
  them. A server never starts with an invalid configuration.
- `CLAI_MODEL` and `CLAI_THEME` are read by each client, not by the server:
  `clai new` sends its `--model` or `CLAI_MODEL` to the server, which
  checks it.
- A running session keeps its provider and model even if they were removed or
  changed in the configuration; its next model change must pick a model that
  exists. The TUI marks such a model as "not in config".
- The project file is read when a session is created in that workspace.
