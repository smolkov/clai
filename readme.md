# codeagent

`codeagent` is an AI coding agent for the terminal, written in Rust. You describe
a task in plain language; `codeagent` sends it to a large language model, runs the
tool calls the model asks for (read and edit files, run commands, ask a
language server) behind permission rules, and shows every change and command.

> Status: in development. The TUI (`codeagent`, `codeagent new`, `codeagent attach`), `codeagent run`
> (one task, without the TUI), sessions in the background server, saved and
> resumed (`ls`, `send`, `chat`, `kill`, `export`, `import`, `server`),
> `codeagent models`, `codeagent skills`, `codeagent commit` and `codeagent completion` work. The TUI `/commit`
> dialog and context management come next. See [`todo.md`](todo.md).

## Install

Releases are on the
[GitHub releases page](https://github.com/smolkov/codeagent/releases): static
Linux binaries (x86_64, aarch64), macOS binaries, Debian packages, and
`SHA256SUMS`. The archives hold `codeagent`, the readme, the licences and the
shell completions in `completions/`.

```sh
v=0.3.1 t=x86_64-unknown-linux-musl
curl -LO https://github.com/smolkov/codeagent/releases/download/v$v/codeagent-$v-$t.tar.gz
curl -LO https://github.com/smolkov/codeagent/releases/download/v$v/SHA256SUMS
sha256sum -c --ignore-missing SHA256SUMS
tar xf codeagent-$v-$t.tar.gz
install codeagent-$v-$t/codeagent ~/.local/bin/
```

The build provenance of each file can be checked with
`gh attestation verify FILE --repo smolkov/codeagent`.

On Debian and Ubuntu (the package needs no other package and installs the
shell completions):

```sh
sudo apt install ./codeagent_0.3.1-1_amd64.deb
```

From crates.io, built from source (needs Rust 1.88 or newer and a C
compiler, for the crypto library):

```sh
cargo install codeagent --locked
```

## Build

The toolchain is pinned in `rust-toolchain.toml`; `rustup` installs it on
the first build.

```sh
cargo build --release
./target/release/codeagent --help
```

`ci/` holds the scripts the GitHub and GitLab pipelines run; they run
locally too:

| Script | Does |
|--------|------|
| `ci/check.sh` | `cargo fmt --check`, `clippy -D warnings`. |
| `ci/test.sh` | `cargo test`, and the tests of the scripts. |
| `ci/crate.sh [--allow-dirty]` | Packs the crate as crates.io gets it; builds and tests it. |
| `ci/build.sh TARGET` | Release build; Linux targets are musl and checked static. Needs `musl-tools`, or `cargo-zigbuild` for another architecture. |
| `ci/package.sh TARGET` | `dist/codeagent-VERSION-TARGET.tar.gz`. |
| `ci/deb.sh TARGET` | `dist/codeagent_VERSION-1_ARCH.deb` with `cargo-deb`; `lintian` if installed. |
| `ci/deb-test.sh DEB` | Installs, checks and removes the package; only in a throw-away container. |

## Shell completion

```sh
echo 'source <(codeagent completion bash)' >> ~/.bashrc   # bash
echo 'source <(codeagent completion zsh)'  >> ~/.zshrc    # zsh
echo 'codeagent completion fish | source'  >> ~/.config/fish/config.fish
```

Tab then completes commands, options, and the configured models for
`--model`. `elvish` and `powershell` work too; see
[`spec/command.md`](spec/command.md), "codeagent completion".

## Commands

| Command | Purpose |
|---------|---------|
| `codeagent` | New session in the current directory, with the TUI; `codeagent -c` continues the last one there. |
| `codeagent TASK...`, `codeagent run TASK...` | Run one task without the TUI and exit. |
| `codeagent new`, `codeagent attach`, `codeagent ls`, `codeagent kill`, `codeagent send` | Manage sessions in the background server. |
| `codeagent chat` | Chat with a session in the terminal, or answer its questions. |
| `codeagent models` | List configured providers and models. |
| `codeagent skills` | List the skills found for the current directory. |
| `codeagent commit` | Print a commit message for the staged changes (`-a`: all changes of tracked files): `codeagent commit -a \| git commit -a -F -`. |
| `codeagent export`, `codeagent import` | Write a session to a file, add one from a file. |
| `codeagent server` | Run, stop or check the background server. |
| `codeagent completion [SHELL]` | Print the shell code for Tab completion. |

All options and exit codes: [`spec/command.md`](spec/command.md).

## The TUI

`codeagent` creates a session in the current directory and opens the full-screen
interface; `codeagent new [DIR]` does the same for `DIR`, and `codeagent attach NAME`
returns to a session (without a name and with several sessions, it opens the
session list). Leaving only detaches: the session keeps running.

The model's answers start with `●`; tool calls are one line each, with
short names (`read`, `ls`, `search`, `edit`, `write`, `run`) and their
result.

| Key | Action |
|-----|--------|
| `Enter` | Send the message; also while a turn runs (it is queued). |
| `Alt-Enter`, `Shift-Enter` | New line. `Shift-Enter` needs a terminal that reports it. |
| `Up` / `Down` | Earlier messages of the session. |
| `Esc`, `Ctrl-C` | Cancel the running turn; without one, `Ctrl-C` clears the input. |
| `Ctrl-C` twice, `Ctrl-D` on empty input | Detach. |
| `PgUp` / `PgDn`, mouse wheel | Scroll. `Ctrl-Home` / `Ctrl-End`: start / end, and follow again. |
| `Ctrl-O` | Choose the model: type to filter, `Up` / `Down` select, `Enter` changes it, `Esc` closes. |
| `Ctrl-S` | Session list: `Enter` attach, `n` new (workspace, name, then the model), `r` rename, `k` kill, `Esc` close. |
| `Ctrl-N` / `Ctrl-P` | Attach to the next / previous session. |
| `Tab` | Switch to the conversation: `Up` / `Down` select, `Enter` expands a tool call, `o` opens it in a view with all of it (`/` search, `y` copy, `w` wrap), `y` copies the entry, `Y` its code block, `?` shows help. |
| `Ctrl-Y` / `Ctrl-T` | Copy the last answer / show or hide the tool calls of the last turn. |
| Mouse | Drag to select and copy (wrapped lines joined, markers left out), double click a word, triple click a line; click a tool call for a short look, double click it for the whole result. `Shift`-drag uses the terminal's selection. |

A tool call that needs permission opens a prompt: `y` yes, `a` always (`Tab`
widens the rule, `Enter` confirms), `n` no, `e` no with a message to the model,
`Esc` no and cancel the turn. A turn at its limit asks `y` continue, `n` stop.

Commands: `/help`, `/model [PROVIDER/MODEL]` (without one, choose it as with
`Ctrl-O`; with `--default` also make it the default of new sessions), `/sessions`, `/new [NAME]` (a session in the same workspace; the model
picker starts on `default_model`, so `Enter` takes it), `/attach [NAME]`,
`/rename NAME`, `/status [NAME]`, `/stats [7d|30d|all]`, `/lsp` (the language servers; `r` restart, `k` stop), `/skills [NAME]` (choose the session's skills; with a name, turn one on or off), `/settings`, `/theme [NAME]`, `/kill`, `/copy [N|code]`
(the last answer, the one N back, or its last code block), `/diff` (the changes of the last turn), `/detach`, `/quit`. `/` in the empty input lists them:
type to search, `Up` / `Down` select, `Enter` runs, `Tab` completes, `Esc` closes
the list and keeps the text. After `/attach `, `/model ` and `/skills ` a list
offers the session names, models and skills the same way. The list also shows commands of later
versions, dimmed. A message that starts with `/` is written `//…`.

Switching keeps each session's unsent input, scroll position and expanded tool
calls. When the attached session ends, the session list opens.

`/status` shows the attached session, `/status NAME` another one, running or
saved: state, workspace, clients, where it is saved; the model with its key,
context use and settings; the running turn against `[limits]`; and statistics
over the whole session: turns, requests, retries, errors, tokens, time spent
working, tool calls per tool, and questions by answer and client. It updates
while open; `Esc` closes it.

`/stats` opens the same box on its Global tab (`Tab` switches): statistics over
all sessions of the server for the last 7 days, 30 days or all time (`7`, `3`,
`a`): sessions, turns, requests, tokens and working time, a table per model
with its share of the tokens, and a chart of tokens per day. `R` counts again.

`/settings` shows the `[tui]` settings: change one with `Enter` or `Left` /
`Right` and see it at once, `d` sets the default, `s` saves into the config file
(comments and order stay), `Esc` closes. `/theme` lists the themes and previews
the selected one; `Enter` takes it and saves it as the default, `/theme NAME`
does so directly. Besides `terminal`, `dark` and `light`, `codeagent` brings
`atom-one-dark`, `nord`, `gruvbox-dark` and `solarized-light`. Your own themes go
in the config file as `[themes.NAME]` or as files in `~/.config/codeagent/themes/`,
e.g. `themes/mine.toml`:

```toml
base       = "dark"       # terminal | dark | light
background = "#282c34"    # optional: paints the whole screen
text       = "#abb2bf"
accent     = "#61afef"
```

The session row ends with the tokens the session has used, `in 45.1k out 3.2k`.
Input counts the history again with each request, so this is neither the context
size nor a bill.

Without a terminal the TUI exits with `2`; use `codeagent run` or `codeagent chat`.
`[tui]` in the configuration has `symbols`, `mouse` (off: the terminal's own
text selection works), `tool_output_lines`, `tool_preview` (`changes`: diffs and
the end of command output under finished calls; `all`; `none`), `clipboard`
(`auto`: OSC 52, plus `wl-copy` / `xclip` / `xsel` / `pbcopy` when local;
`osc52`; `command`; `off`), `timestamps`, `scroll_lines`, `max_fps` and
`activity`; `syntax_highlight` colours code blocks, diffs, new files and
commands by their language, with the syntax theme a theme's `code` key names
(`ansi` on the terminal's 16 colours, `atom-one-dark`, `nord`, `gruvbox-dark`,
`base16-ocean.dark`, `InspiredGitHub`, `Solarized (dark)`, …; each bundled theme
uses the syntax colours of its own scheme); `NO_COLOR` turns colours off. `expand_tool_calls = true`
still works and means `tool_preview = "all"`.

While the session works, a line above the input shows what it does, since
when, and the output tokens of the turn:
`✻ Prestidigitating… (14s · ↓ 1.2k tokens · Esc cancel)`. The glyph turns to
the warning colour when the model has been quiet for 30 s.
`tui.activity = "plain"` says `Thinking…` instead of a playful verb, `"off"`
hides the line.

## Running a task

```sh
codeagent fix the failing test in tests/parse.rs
codeagent run --model local/qwen "explain src/main.rs"
codeagent - < task.txt                               # task from standard input
codeagent --yes -C ~/ws/app "run the tests and fix what fails"
```

The final answer goes to standard output, one line per tool call to
standard error. When `codeagent` runs in a terminal it asks before writing files
or running commands that the rules do not allow; in scripts such calls are
denied unless `--yes` is given. `deny` rules always apply. Exit codes: `0`
done, `1` failed or stopped, `2` usage or configuration error, `130`
`Ctrl-C`.

`--report` prints a report instead of the answer: the status, findings
with a severity and a place, the changed files, the commands and every
tool call. `--report=json` prints it as JSON for scripts. Without words
the task comes from a pipe:

```sh
codeagent run --report "analyse this repository for security problems"
cat issue.md | codeagent run --report=json > report.json
```

## Sessions in the background

Sessions run in a background server, one per user, which the first session
command starts and which exits after 60 s without sessions and clients. A
session keeps working while no one watches it, and waits only when it needs
a decision: a permission or limit question, or a question from the model.

When a choice is yours (which design, which database), the model can ask
with the `ask_user` tool: one to four questions with options, answered in
the TUI in the place of the input (what you typed is kept) or as numbered lines in `codeagent chat` and `codeagent run`; you
can always type your own answer instead. The turn goes on with the answer.
`ask_user = "deny"` in `[permissions]` turns it off.

```sh
codeagent new -d --name api ~/ws/api       # start a session, print its name
codeagent attach api                       # open it in the TUI
codeagent -c                               # go on with this directory's last session
codeagent send -c "and the docs too"       # send to it without attaching
codeagent send api "run the tests and fix what fails"
codeagent ls                               # name, workspace, model, state, age
codeagent chat                             # new session here, chat in the terminal
codeagent chat api                         # chat with a running session
codeagent chat --questions                 # only answer questions, of all sessions
codeagent chat --list                      # open questions, one per line
codeagent chat api allow --id q3           # answer from a script
codeagent kill api                         # end it and delete its saved files
codeagent server status                    # running, pid, sessions
codeagent server stop                      # stop the server; sessions stay saved
codeagent export api api.json              # write a session to one file
codeagent import api.json                  # add it again (e.g. on another machine)
```

`codeagent chat` is a plain-text chat: lines you type go to the model, its text
streams back with one line per tool call. `Ctrl-C` cancels the running
turn, `Ctrl-D` detaches (the session keeps working), `/help` lists the
commands (`/cancel`, `/model`, `/usage`, `/status [NAME]`, `/stats [RANGE]`, `/sessions`, `/attach NAME`,
`/new [NAME] [PROVIDER/MODEL]`, `/models`, `/detach`, `/kill`). It shows each question
with the diff or command and reads the answer as a line: `y` allow, `a` always (then a rule), `n` deny, `e` deny
with a message, `c` cancel the turn; for a limit question `y` continue,
`n` stop; for a question from the model an option's number (`1,3: text`
for several, with a text), your own text, `s` skip or `c` cancel.

Sessions are saved as they go, in `~/.local/state/codeagent/sessions/`
(`$XDG_STATE_HOME`), so they survive a server stop, a crash or a reboot.
`codeagent ls` shows them as `saved`, and `codeagent chat NAME` (or `send`) resumes
one with its whole conversation and the rules you allowed. A turn that was
running is interrupted, not continued. If its model is gone from the
configuration, resume with `codeagent chat NAME --model PROVIDER/MODEL`. The
files hold the conversation, including file contents and command output,
so they are readable only by you; API keys are never saved. The
server reloads `config.toml` when it changes; new sessions use the new
file. Tab completion knows session names.

The socket is `$XDG_RUNTIME_DIR/codeagent/server.sock` (or
`/tmp/codeagent-$UID/server.sock`), in a directory only you can read. Details:
[`spec/sessions.md`](spec/sessions.md).

## Configuration

Providers and models are defined in `~/.config/codeagent/config.toml`
(`$XDG_CONFIG_HOME/codeagent/config.toml`); `codeagent` has no built-in list. A
minimal file:

```toml
default_model = "anthropic/sonnet"

[providers.anthropic]
api         = "anthropic"
api_key_env = "ANTHROPIC_API_KEY"

[providers.anthropic.models.sonnet]
id             = "claude-sonnet-5-5"
context_window = 200000
```

Check it with `codeagent models`, which lists every configured model and marks
the default and providers whose key variable is not set.

To change the default model of new sessions, tick `Space` "Also make it the
default" in the model picker (`Ctrl-O`) before `Enter`, use
`/model NAME --default`, or run `codeagent models --set-default NAME`. All three
write `default_model` into the user file, comments kept; the next `codeagent` start
uses it.

A project can add `.codeagent/config.toml` to set `default_model`, `[commit]`, lower `[limits]` and
stricter `[permissions]`. `CODEAGENT_MODEL` overrides `default_model` and
`CODEAGENT_THEME` overrides the theme. All keys: [`spec/config.md`](spec/config.md). Sample files to start from
(minimal, Ollama, every key, read-only, for scripts) are in
[`tests/configs/`](tests/configs/readme.md).

## Permissions

Every tool call of the model is allowed, asked about or denied. By default
reading is allowed, and changing files and running commands ask. Rules in
`[permissions]` change that, grouped by action; each key is a tool or a
group (`read`, `edit`):

```toml
[permissions.allow]
run_command = ["ls", "cat", "cargo build", "cargo test", "git status", "git diff"]
edit        = ["src/**", "tests/**"]

[permissions.ask]
run_command = ["git push"]

[permissions.deny]
run_command = ["rm", "sudo", "git push --force"]
read        = [".env*", "secrets/**"]
```

Command patterns match the first words of each command: `git push` also
matches `git push origin main`, and `rm` denies every `rm`, also through
`sudo`, `xargs` or `sh -c`. A command with several parts (`a && b`, pipes,
`$( )`) is checked part by part, and redirections like `> file` are checked
as writes. Nothing outside the workspace (and `extra_dirs`) is ever read or
written. Details, exceptions (`except`) and messages for the model:
[`spec/config.md`](spec/config.md), "Permissions".

Out of the box, `codeagent` keeps well-known secret files from the model:
`.env` and `.env.*` (not `.env.example` and the like), key files (`*.pem`,
`*.key`, `*.p12`, `*.pfx`, `id_rsa`, …), `.netrc` and `.pgpass`. They are
not read, listed or searched, and an allowed command that names one, such
as `cat .env`, asks first. No `allow` rule or `--yes` passes them; to let
the model read one, add it to `builtin_except` in your own config file:

```toml
[permissions]
builtin_except = ["deploy/certs/*.pem"]
```

## Language servers

With a language server the model finds definitions, references, types and
callers of a symbol instead of guessing with a regex, and sees at once when
an edit broke the build. The `lsp` tool asks the server for `definition`,
`type_definition`, `implementation`, `references`, `hover`,
`incoming_calls`, `outgoing_calls`, `document_symbols`,
`workspace_symbols` and `diagnostics`. After `edit_file` and `write_file`,
the errors a running server reports for the changed file are added to the
result.

These servers are used when their program is on `PATH`: rust-analyzer
(`.rs`), gopls (`.go`), clangd (C and C++), pyright (`.py`) and
typescript-language-server (`.ts`, `.js`). `codeagent` installs none; with
rustup, `rustup component add rust-analyzer`. Add or change servers in the
user file:

```toml
[lsp]
start = "ask"                 # ask, allow or deny

[lsp.servers.rust-analyzer]   # keys left out come from the built-in one
settings = { checkOnSave = false }

[lsp.servers.ruff]            # serves .py instead of pyright
command    = ["ruff", "server"]
extensions = ["py"]
roots      = ["pyproject.toml"]
```

**Starting a server asks.** A language server runs code of the project
(rust-analyzer runs build scripts and procedural macros), as `cargo build`
does, so the first `lsp` call of a session for a server asks:

```
Start language server rust-analyzer in ~/ws/codeagent?
command: rust-analyzer  It may run build scripts and macros of the project.
[y] yes  [n] no  [e] explain  [Esc] cancel
```

`y` lasts for the session; `n` for the turn. `start = "allow"` (or
`codeagent run --yes`) skips the question, `start = "deny"` or `enabled = false`
turns the tool off. A project's `.codeagent/config.toml` can only turn it off.
Sessions on the same project share one server; it stops after 10 minutes
without use. `/lsp` shows the servers, their state and last error lines,
and restarts or stops one. Details: [`spec/tools.md`](spec/tools.md),
"lsp", and [`spec/config.md`](spec/config.md), "LSP".

## Skills

A skill is a directory with a `SKILL.md`: a name, a description and
instructions for the model, maybe with scripts next to it. It is the format
of OpenCode and Claude Code, so skills you have for them work here too.

```markdown
---
name: release-check
description: Steps before tagging a release of a Rust crate.
---

# Release check

1. Run `cargo test` and `cargo clippy` …
```

`codeagent` finds skills in the directories of your user file's `[skills] dirs`,
then in `.opencode/skills`, `.claude/skills` and `.agents/skills` of the
workspace, then in `~/.config/opencode/skills`, `~/.claude/skills` and
`~/.agents/skills`, also in subdirectories (`synced/<id>/<skill>/`). When
two skills have the same name, the earlier
directory wins. `codeagent skills` lists what it finds, and why a file is
skipped.

```toml
[skills]
dirs = ["~/ws/team-skills"]   # searched first
default_dirs = true           # false: only dirs
```

Nothing is used until you turn it on for a session: `/skills` opens a box
(`Space` ticks, `Enter` applies), `/skills NAME` turns one on or off, and
`codeagent run --skill NAME` or `codeagent new --skill NAME` start with it. An active
skill's instructions go into the system prompt, and its directory is
readable (not writable) for the model; its scripts run only as your
command rules allow. The session keeps the text it read, also after a
restart; `/skills` marks a skill whose file changed since. A project's
`.codeagent/config.toml` cannot set `[skills]`. Details:
[`spec/skills.md`](spec/skills.md).

## Limits

A turn pauses and asks whether to continue after 50 model requests or 120
tool calls, so a confused model cannot loop forever. Change it with:

```toml
[limits]
max_requests   = 50
max_tool_calls = 120
```

A project file may only lower these values. Details:
[`spec/agent-loop.md`](spec/agent-loop.md), "Limits".

## Logging

`codeagent` logs to `~/.local/state/codeagent/codeagent.log` (or `$XDG_STATE_HOME/codeagent/codeagent.log`).
Set `CODEAGENT_LOG=debug` for more detail.

## Release

A tag `vX.Y.Z` releases; the pipelines do the rest (RFC 0020):

1. Move the entries under `Unreleased` in `CHANGELOG.md` into a new section
   `## X.Y.Z - YYYY-MM-DD`.
2. Set `version = "X.Y.Z"` in `Cargo.toml`, run `cargo build` (updates
   `Cargo.lock`), and commit.
3. `git tag -a vX.Y.Z -m vX.Y.Z`, then push the commit and the tag, to
   GitHub and to the GitLab mirror.
4. GitHub checks, builds all targets, packs, tests the Debian packages,
   and publishes the release with the notes, `SHA256SUMS` and provenance;
   then `cargo publish`. GitLab builds the Linux files, uploads them to its
   package registry and creates its release.

A version with `-` (`0.2.0-rc.1`) is a pre-release: released on GitHub as
such, not published to crates.io.

crates.io, once:

- The first version is published by hand: `ci/crate.sh`, then
  `cargo publish --locked` with your crates.io token. Publishing cannot be
  undone; a version can only be yanked.
- Then, in the crate's settings on crates.io, add a trusted publisher:
  repository `smolkov/codeagent`, workflow `release.yml`, environment
  `crates-io`. Create the `crates-io` environment in the GitHub repository
  (with yourself as required reviewer, if each publish should wait for
  you). Later releases publish without a stored token.

## Specification

The behaviour is specified in [`spec/`](spec/readme.md); the build order is in
[`implementation.md`](implementation.md).
