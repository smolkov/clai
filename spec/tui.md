# TUI

Interactive sessions are shown in a full-screen terminal user interface (TUI).
The TUI is a client of the `clai` server: it attaches to one session at a
time, and can detach, switch to another session or start a new one. Sessions
keep running without a TUI (see `spec/sessions.md`).
One-shot mode (`clai "task"`) and runs where standard output is not a
terminal do not use the TUI; they print plain text (see
[`command.md`](command.md)).

Statements marked **(assumption)** need a decision from the user.

## Goals

- Show the conversation, tool calls and their results as they happen,
  including the model's answer while it streams.
- Make every permission decision visible and quick: the change or command is
  shown in full before the user answers.
- Stay responsive: the screen redraws and keys work while the model answers
  or a command runs.
- Never leave the terminal broken: on exit, error or panic in a dependency,
  the terminal is restored (raw mode off, cursor shown, main screen back).

## Layout

```
> fix the failing test

The test fails because `parse` drops the last line. Let me look.

▸ read_file src/parse.rs                                          ✓ 120 lines
▾ edit_file src/parse.rs                                                ? ask
  - for i in 0..lines.len() - 1 {
  + for i in 0..lines.len() {

──────────────────────────────────────────────────────────────────────────────
edit_file: src/parse.rs  asks: default for edit_file
[y] yes  [a] always  [n] no  [e] explain  [Esc] cancel
──────────────────────────────────────────────────────────────────────────────
> _
──────────────────────────────────────────────────────────────────────────────
  asking · Esc cancel · /help · Ctrl-D detach
  project · ~/ws/project · anthropic/sonnet · 1/3 ●1 · 12.3k/200k · in 45.1k out 3.2k
```

There is no box around the screen: the conversation takes the full width, and
plain lines set off the prompt bar and the input. From top to bottom:

| Area           | Content |
|----------------|---------|
| Conversation   | Scrollable history of the session: user messages, model text, tool calls. |
| Prompt bar     | Shown only while a question (permission or limit) is open; replaces nothing else. |
| Input          | Multi-line editor for the next user message, with a line above and below it. |
| Status row     | Current state (idle, waiting for model, running tool, asking), turn counters, key hints. |
| Session row    | Session name, workspace path, provider and model (`anthropic/sonnet`), position among all sessions (`1/3`), number of other sessions waiting for an answer (`●1`), context usage (tokens used / limit; only the tokens if the server does not list the model, nothing until the first count), tokens the session has used (`in 45.1k out 3.2k`). |

The conversation area takes all remaining height. If the terminal is smaller
than 40 × 10 cells, `clai` shows only a "terminal too small" message until it
is resized.

## Sessions

The TUI shows exactly one session, the attached one. Other sessions keep
running on the server; the session row counts the ones waiting for an answer to a
question (see [`sessions.md`](sessions.md), "Background work"), so the user
knows to switch to them.

The session list opens with `Ctrl-S`, `/sessions`, or `clai attach` without a
name:

```
┌ Sessions ────────────────────────────────────────────────────────────────────┐
│     NAME       WORKSPACE     MODEL             STATE      AGE                │
│   ✓ project    ~/ws/project  anthropic/sonnet  working    12m                │
│ ▶   project-2  ~/ws/project  local/qwen        idle       2h                 │
│     docs       ~/ws/docs     anthropic/opus    ● waiting  3m                 │
│     old        ~/ws/old      anthropic/sonnet  saved      3d                 │
│                                                                              │
│                                                                              │
│ Enter attach ─ n new ─ r rename ─ k kill ─ Esc back                          │
└──────────────────────────────────────────────────────────────────────────────┘
```

- The list holds every session the server knows, running and saved, in the
  server's order, which is also the order of `1/3` and of `Ctrl-N` / `Ctrl-P`.
  `✓` marks the attached session; it is selected when the list opens.
- It asks the server for the sessions when it opens and every 2 s while open;
  the selection stays on the same session.
- On a narrow terminal the age, then the workspace, are left out.
- A question that opens in the attached session closes the list: keys belong to
  the prompt bar then.

| Key in list | Action |
|-------------|--------|
| `Up` / `Down` | Select a session; they stop at the ends. |
| `Enter`     | Attach to the selected session (detaches from the current one). On the attached session it only closes the list. |
| `n`         | New session: asks for the workspace (default: the attached session's; `~` is the home directory, a relative path starts there) and the name (empty: the default name), then opens the model picker on `default_model`; `Enter` there creates the session and attaches. `Esc` goes back one step; in the picker it creates nothing. |
| `r`         | Rename the selected session, in place; `Enter` renames, `Esc` goes back. A refused name is shown in the list. |
| `k`         | Kill the selected session, after a `y`/`n` confirmation. |
| `Esc`       | Close the list, stay on the current session. Where the list stands for a choice not yet made (after the attached session ended, or for `clai attach` without a name), `Esc` exits the TUI. |

`/new [NAME]` creates a session in the attached session's workspace the same
way: the model picker opens on `default_model`, so `Enter` alone starts the new
session with it. If other sessions work in that workspace, the new session's
conversation starts with the note `clai new` prints.

## Token counters

The session row ends with the tokens the session has used. They are the first
thing cut on a narrow terminal, and left out until the first `usage` event.

- **Input** and **output** are the sums over all requests of the session.
  Input counts the history again with every request, because that is what is
  sent; it is not the context size.
- Anthropic's cached input is included in input, so the counter is not a bill.
- A cancelled request counts what the provider had reported by then. A provider
  that reports nothing adds nothing.

The TUI sums the `usage` events of the log it replays, and starts again on a
reconnect, so a reconnect never counts anything twice.

## Model picker

Each session has its own provider and model. The model picker chooses them.
It opens with `Ctrl-O`, `/model` without an argument, or when creating a
session; then its title is "Model for a new session", it starts on
`default_model`, and it has no `current` or history marks.

```
┌ Model for "project" ──────────────────────────────────────────────────────┐
│ Filter: _                                                                 │
│                                                                           │
│ anthropic                                                                 │
│ ▶ sonnet      claude-sonnet-5-5         200k   ✓ current, default         │
│   opus        claude-opus-5-5           200k                              │
│ local                                                                     │
│   qwen        qwen3-coder:30b            32k   ⚠ history 41k > 32k        │
│                                                                           │
│ Enter select ─ Esc cancel                                                 │
└───────────────────────────────────────────────────────────────────────────┘
```

- The picker lists exactly the providers and models of the configuration
  (`spec/config.md`), grouped by provider, in config order. There is no
  free-text model id: a model that is not configured cannot be chosen.
- Typing filters by provider name, model name and model id.
- Each row shows the model's config name, its `id` and context window. Marks:
  current model of the session, `default_model`, a provider whose API key is
  missing (`⚠ no key`; still selectable, the error comes on first use), and a
  model whose context window is smaller than the session's history
  (`⚠ history 41k > 32k`, from the last context size). Until
  `spec/context.md` exists such a model is only marked.
- The list comes from the server (the `models` request), since its
  configuration and environment are what the requests use. The TUI takes context
  windows from the same answer; it asks when the picker opens and when the
  configuration was reloaded.
- `Enter` selects and closes; on the current model it only closes. `Esc` closes.
  `Up` / `Down` select; other keys edit the filter. With no match the list says
  `no model` and `Enter` does nothing.
- The picker does not open while a question is open, and a question that opens
  closes it: keys belong to the prompt bar then.
- `/model <provider>/<model>` (or `/model <model>` when unambiguous) switches
  without the picker; an unknown name is an error that lists the configured
  models.
- If no model is configured, the picker shows the path of the user config file
  and a short example instead of a list.

Changing the model of a running session:

- The new provider and model apply from the next model request. A request that
  is already running finishes with the old model. **(assumption)**
- The conversation history is kept and converted to the new provider's
  format. If it does not fit the new context window, the oldest parts are
  shortened (see `spec/context.md`); the TUI asks for confirmation first.
- A notice in the conversation records each change, e.g.
  "model: anthropic/sonnet → local/qwen". Choosing the model the session already
  has adds no notice. A log of an older `clai` has no old model; its notice is
  "model: local/qwen".
- The change applies to this session only. Other sessions and
  `default_model` are not affected.

## Switching sessions

- Each session keeps the TUI's view state while the TUI is attached to
  another one: scroll position, expanded tool calls and unsent input. It is
  restored on switching back. **(assumption)**
- On attach, the TUI loads the session's full history from the server, then
  follows it live. A turn that ran while detached appears as if it had been
  watched.
- A switch attaches on a new connection to the server and drops the old one,
  which detaches it. Until the new session is there, the old one stays on
  screen and the status row says `attaching NAME…`; if it fails, the error is
  shown and the old session stays. A question open in the session left stays
  open for other clients and is counted in `●`.
- If the attached session is killed (by this or another client), the TUI
  shows a notice and opens the session list. If no session is left, the list
  says `no sessions` and offers `n` new or `Esc` exit. `Esc` exits with `0`.
  Until the list is there, any key exits. For `clai server stop` the TUI
  opens no list: the notice, the input is disabled, and any key exits with
  `0`; it does not start the server again.
- If the connection to the server is lost, the TUI shows "server
  disconnected" and retries every 500 ms for 10 s, starting the server if
  needed, since sessions are saved. The conversation is rebuilt from the log;
  scroll position, expanded tool calls and unsent input are kept. After 10 s
  it exits with an error; the terminal is restored. **(assumption)**

## Conversation entries

| Entry          | Display |
|----------------|---------|
| User message   | Prefixed with `>`, in the user colour. |
| Model text     | Rendered Markdown: headings, emphasis, lists, inline code, code blocks with syntax highlighting. Streams in as it arrives. |
| Tool call      | One summary line: marker, tool name, main argument, state. |
| Tool details   | Below the summary when expanded: diff for edits, file excerpt for reads, command and output for commands. |
| Error          | Red line with the error message; the session continues. |
| Notice         | Dim line for events such as "turn cancelled" or "limit reached". |

Tool call state is one of: running (spinner), done (`✓` plus a short result,
e.g. line count or exit code), failed (`✗`), denied (`⊘`), waiting for
permission (`?`).

Tool calls are collapsed (`▸`) by default and expanded (`▾`) while they wait
for permission. Long outputs are cut to `tui.tool_output_lines` lines (default
20) in the view, with a "… N more lines" marker; the full text stays
available in the expanded entry. See [`config.md`](config.md), "TUI".

## Permission prompts

When a tool call needs permission, the session sends a permission question
with the call's details, the reason and the proposed rules (see
[`sessions.md`](sessions.md), "Questions", and [`tools.md`](tools.md),
"Questions"). The same question may be answered in another TUI or with
`clai chat`; the TUI then closes its prompt and shows who answered.

The TUI:

1. Expands the tool call and scrolls it into view, showing the question's
   details: diff or content, command parts with their decisions, working
   directory.
2. Shows the prompt bar with the reason (`asks: default for edit_file`) and
   the choices.
3. Takes keys for the prompt bar only, until the user answers or cancels.
   What answers nothing still works: `PgUp` / `PgDn`, `Ctrl-Home` / `Ctrl-End`
   and the mouse wheel scroll, to read what is asked; `Ctrl-N` / `Ctrl-P`
   attach to another session and `Ctrl-S` opens the session list over the
   prompt, which then takes the keys until it closes; `Ctrl-D` detaches. The
   question stays open, for later or for another client: attaching to the
   session again shows the prompt again. `Ctrl-C` acts like `Esc`.

| Key   | Action |
|-------|--------|
| `y`   | Allow this call once. |
| `a`   | Allow this call and add an allow rule for the session. The prompt bar shows the proposed rule (this command, this file); `Tab` widens it step by step (e.g. `cargo test`, `src/**`), `Enter` confirms, `Esc` goes back. See [`config.md`](config.md), "Session rules". |
| `n`   | Deny; the model is told the call was denied. |
| `e`   | Deny and type a short message to the model explaining why or what to do instead. |
| `Esc` | Deny and cancel the turn. |

Edits are shown as a unified diff with added lines green and removed lines
red. Commands are shown exactly as they will run, with the working directory.

## Limit prompt

When a turn reaches its limit of model requests or tool calls, the session
waits (see [`sessions.md`](sessions.md), "Background work") and the TUI shows
the prompt bar:

```
Turn limit reached (50 requests, 120 tool calls). Continue?
[y] yes  [n] stop
```

| Key   | Action |
|-------|--------|
| `y`   | Continue the turn with a new budget of the same size. |
| `n`   | Stop the turn; a "limit reached" notice is added. |
| `Esc` | Same as `n`. |

The prompt takes keys only for the prompt bar, as for permission prompts.

## Input editor

- `Enter` sends the message; `Shift-Enter` or `Alt-Enter` inserts a newline.
  **(assumption: terminals that cannot report `Shift-Enter` use `Alt-Enter`)**
- Standard line editing: arrows, `Home`/`End`, `Ctrl-A`/`Ctrl-E`,
  `Ctrl-W` delete word, `Ctrl-U` delete to line start.
- `Up`/`Down` on the first/last line of the input browse previous messages of
  this session.
- Pasted text is inserted as is, including newlines (bracketed paste); it is
  not sent until `Enter`.
- The user may type and send while a turn runs. The message goes to the server
  at once, which queues it until the turn ends (other clients see the queue
  too); the TUI shows it as "queued" until the turn that takes it starts.
- Lines starting with `/` are commands for `clai`, not messages to the model:

| Command    | Action |
|------------|--------|
| `/help`    | Show keys and commands. |
| `/clear`   | Start a new conversation in the same session. |
| `/model [PROVIDER/MODEL]` | Change the session's provider and model; without an argument, open the model picker. |
| `/permissions` | Show the permission rules in effect: configuration, project file and session rules, with their source. |
| `/sessions` | Open the session list. |
| `/new [NAME]` | Start a new session in the same workspace and attach to it; the model picker opens on `default_model` first. |
| `/attach [NAME]` | Attach to another session; without a name, open the session list. A name that is not in the list stays in the input with a notice. |
| `/rename NAME` | Rename the attached session. |
| `/settings` | Open the settings box: the `[tui]` settings, changed with a preview and saved into the user file. See [Settings](#settings). |
| `/theme [NAME]` | Open the theme list; with a name, switch to that theme. Either way the theme is saved as the default. See [Theme list](#theme-list). |
| `/kill`    | End the attached session, after confirmation, and open the session list. |
| `/commit [HINT]` | Generate a commit message for all changes in the workspace and commit them after confirmation; see [Commit](#commit) and [`commit.md`](commit.md). |
| `/detach`, `/quit` | Detach and exit the TUI; the session keeps running. |

### Command list

Typing `/` into the empty input, with focus on the input, opens the command
list above the input. Pasted text and a message from the history do not open it.

```
──────────────────────────────────────────────────────────────────────────────
▶ /model [PROVIDER/MODEL]  Change the model; without one, choose it
  /permissions             later version
  /rename NAME             Rename the session
  /commit [HINT]           later version
──────────────────────────────────────────────────────────────────────────────
> /m_
──────────────────────────────────────────────────────────────────────────────
```

- The list shows every command with its arguments and a one-line
  description, in the order of the table above. A command that is not built yet
  is dim and says `later version`; `Enter` on it says so too.
- It lies over the last rows of the conversation and shows at most 8 rows; the
  selected row stays visible. The layout does not change, so the conversation
  does not jump.
- Each further character searches the list: commands with a name that contains
  the typed text, in any case, those that start with it first, each in table
  order. The best match is selected. `/quit` shares a row with `/detach`, and
  either name finds it.
- If nothing matches, the list shows "no command" and `Enter` sends nothing;
  the input stays so it can be corrected. To send a message that starts with
  `/` to the model, start it with `//`; the first `/` is removed, and the second
  `/` closes the list. **(assumption)**
- A space or a new line closes the list too: arguments follow.
- A question that opens, and `Ctrl-O`, close the list and keep the text. Keys
  belong to the prompt bar or the picker then.
- A closed list opens again only from an empty input. With it closed, `Enter`
  runs the text as it stands; an unknown command gets a notice and stays in the
  input, and `/rename` without a name gets its usage.

| Key in list         | Action |
|---------------------|--------|
| `Up` / `Down`       | Select the previous / next command; they stop at the ends. |
| `Tab`               | Complete the selected command into the input and close the list, so arguments can be typed. With no match, nothing. Focus does not change. |
| `Enter`             | Run the selected command. If it needs an argument, complete it like `Tab` instead. |
| `Esc`               | Close the list and nothing else: the typed text stays and a running turn goes on. A second `Esc` cancels the turn. |
| `Backspace` on the `/` | Delete the `/` and close the list. |

Other keys work as without the list: the conversation scrolls, and `Ctrl-C`
cancels the turn or clears the input, which closes the list.

Every box the user can open has a command in the table above. The prompt bar has
none: it opens when the agent asks a question.

After `/attach `, `/model ` and `/theme ` (typed, or completed with `Tab`), a
list of values lies in the same place: `/attach` offers the other sessions,
with their workspace and state, `/model` the server's models, with their id,
`/theme` the themes, with where they come from. Typing searches them like
commands, by name, and models also by id. `Up` / `Down` select, `Tab` completes
the selected value, `Enter` runs the command with it, and with no match
(`no match`) `Enter` runs the text as typed. `Esc` closes the list until the
input no longer is such a command. A second word closes it.

While the command list is open, `Up`/`Down` select in the list instead of
browsing the message history.

### Commit

`/commit [HINT]` commits all changes in the session's workspace with a
message written by the model. Nothing is committed without the user's
confirmation. Which changes are included and how the message is generated is
described in [`commit.md`](commit.md); `clai commit` on the command line uses
the same rules but only prints the message.

Steps:

1. Collect the changes of the workspace and generate the message (see
   [`commit.md`](commit.md)). If there is nothing to commit or the directory
   cannot be committed, a notice explains why and nothing else happens.
   The request and answer are not added to the conversation history; the
   conversation only gets a notice. **(assumption)**
2. Confirm: the message is shown in a dialog together with the list of files
   that will be committed:

   ```
   ┌ Commit 3 files in ~/ws/project ──────────────────────────────────────────┐
   │ Fix last-line loss in parse                                              │
   │                                                                          │
   │ The loop stopped one line early, so the last line of every input was     │
   │ dropped. Iterate over all lines and add a test for single-line input.    │
   │                                                                          │
   │ M src/parse.rs   A tests/parse_single_line.rs   M Cargo.lock             │
   ├──────────────────────────────────────────────────────────────────────────┤
   │ Enter commit ─ e edit ─ r regenerate ─ d diff ─ Esc cancel               │
   └──────────────────────────────────────────────────────────────────────────┘
   ```

   | Key     | Action |
   |---------|--------|
   | `Enter` | Commit with the shown message. |
   | `e`     | Edit the message in the dialog (multi-line editor, same keys as the input). |
   | `r`     | Generate a new message. |
   | `d`     | Show the full diff that will be committed. |
   | `Esc`   | Cancel; nothing is staged or committed. |

3. Commit: stage the changes of the workspace (`git add -A -- <workspace>`)
   and commit only them (`git commit -F - -- <workspace>`). Changes already
   staged outside the workspace stay staged but are not committed. Git hooks
   run as usual; `clai` never passes `--no-verify`.
4. Result: the conversation gets a notice with the short hash and subject. If
   git fails (hook rejects, no `user.name`), its output is shown and the
   dialog opens again with the message, so the user can retry or cancel.

`/commit` never pushes, amends, or changes branches. If files change while
the dialog is open, `Enter` first shows "changes differ from the generated
message" and offers `r` regenerate or `Enter` commit anyway.

With a remote provider, the dialog header names the provider that received
the diff.

## Keys

| Key                  | Action |
|----------------------|--------|
| `Esc` / `Ctrl-C`     | Cancel the running turn. With no running turn, `Ctrl-C` clears the input. |
| `Ctrl-C` twice within 1 s, or `Ctrl-D` with empty input | Detach and exit the TUI; the session keeps running. |
| `Ctrl-S`             | Open the session list. |
| `Ctrl-O`             | Open the model picker for the attached session. |
| `Ctrl-N` / `Ctrl-P`  | Attach to the next / previous session in the list, round the end. With one session, nothing. |
| `PgUp` / `PgDn`      | Scroll the conversation. |
| `Ctrl-Home` / `Ctrl-End` | Jump to start / end of the conversation. |
| `Tab`                | Move focus between input and conversation. |
| `Enter` (conversation focus) | Expand or collapse the selected tool call. |
| `Up` / `Down` (conversation focus) | Select the previous / next entry. |
| `?` (conversation focus) | Show help. |

`Ctrl-Z` does nothing for now: the TUI does not suspend.

While new output arrives, the view follows the end of the conversation,
unless the user scrolled up; then it stays put and the status line shows
"↓ new output". `Ctrl-End` resumes following.

Mouse wheel scrolls the conversation. Mouse capture can be turned off with
`tui.mouse = false` so the terminal's own text selection works (see
[`config.md`](config.md), "TUI").

## Cancellation and exit

- Cancelling a turn stops the model request and kills a running command
  (process group, `SIGTERM`, then `SIGKILL` after 2 s). Tool calls already
  done stay done; a notice is added to the conversation.
- Exiting the TUI only detaches; a running turn continues on the server. To
  stop it, cancel the turn or kill the session.
- On exit, the terminal is restored before anything else is printed. A panic
  hook and signal handlers (`SIGTERM`, `SIGHUP`) restore it too.

## Settings

`/settings` opens a box with the `[tui]` keys of the user file
([`config.md`](config.md), "TUI"):

```
┌ Settings ─ ~/.config/clai/config.toml ───────────────────────────────────────┐
│   theme              terminal  Colour theme                                  │
│   symbols            auto      Unicode or ASCII symbols                      │
│ ▶ mouse             *on        Capture the mouse for scrolling               │
│   tool_output_lines  20        Lines of tool output before it is cut         │
│   expand_tool_calls  off       Keep finished tool calls expanded             │
│   syntax_highlight   on        later version                                 │
│   timestamps         off       Time of each entry                            │
│   scroll_lines       3         Lines per mouse-wheel step                    │
│   max_fps            30        Redraw limit                                  │
│                                                                              │
│ mouse: off keeps the terminal's text selection; on scrolls with the wheel    │
│ Enter change ─ Left/Right value ─ d default ─ s save ─ Esc close             │
└──────────────────────────────────────────────────────────────────────────────┘
```

- One row per key, with the value and a description; the line under the list
  explains the selected key. A key the file does not set shows its default,
  dim. `*` marks a value changed and not saved.
- The `theme` row says when something else decides it: `⚠ CLAI_THEME=nord for
  this run`, or `⚠ colours are off (NO_COLOR)`. `symbols = unicode` in a locale
  that is not UTF-8 is marked too, and still applied.
- **Preview:** a change applies to this TUI at once; other TUIs see it only once
  it is saved.
- The title names the file `s` writes: the user file of this TUI (`--config` or
  the default path).

| Key | Action |
|-----|--------|
| `Up` / `Down` | Select a row. |
| `Left` / `Right` | Previous / next value: on/off, one of a list, one step of a number, the themes in the order of the theme list. |
| `Enter`, `Space` | On/off: toggle. List: next value. Number: type it in the row; `Enter` takes it, `Esc` goes back, a value out of range says so and stays. Theme: open the [theme list](#theme-list); its `Enter` only sets the row. |
| `d` | Back to the default; the key is removed from the file on save. |
| `s` | Save all changes. |
| `Esc` | Close. With unsaved changes it asks `save changes? [s] save  [n] discard  [Esc] back`; discarding puts the file's values back. |

Saving (also for the theme list):

- The keys are set or removed in `[tui]` of the user file; the table is added if
  it is missing. Everything else stays as it is, comments and order included.
- The new text is loaded like at start first. If that fails, nothing is written
  and the box shows the error.
- If the file changed since it was read, nothing is written either:
  `the file changed since it was read; open it again`.
- The file is replaced atomically with its mode kept; a link is written through
  to its target. The server notices it and sends `config_reloaded`; every TUI
  then applies the new `[tui]`.

Opening the box or the theme list reads the user file and the theme files
again, so a theme file edited on its own is listed. A question that opens
closes the box and the theme list and undoes their preview, as for the model
picker.

## Theme list

`/theme` opens a list of all themes ([`config.md`](config.md), "Themes"):

```
┌ Theme ───────────────────────────────────────────────────────────────────────┐
│ Filter:                                                                      │
│                                                                              │
│ built in                                                                     │
│   terminal         ✓ current                                                 │
│ ▶ dark                                                                       │
│   light                                                                      │
│ bundled                                                                      │
│   atom-one-dark    base dark                                                 │
│   nord             base dark                                                 │
│   gruvbox-dark     base dark                                                 │
│   solarized-light  base light                                                │
│ theme files                                                                  │
│   bad              ⚠ themes/bad.toml:1: base: …                              │
│                                                                              │
│ Enter take and save as default ─ Esc back to terminal                        │
└──────────────────────────────────────────────────────────────────────────────┘
```

- Groups: built in, bundled (a theme file that replaces one shows its path
  there), theme files by name, then the `[themes.NAME]` tables of the user file
  in file order. A broken theme file is listed dim with its error and cannot
  be taken. Typing filters by name; `Up` / `Down` select.
- **Preview:** the selected theme applies at once, on the real conversation.
  With the selection on a broken file or no match, the last theme shown stays.
  `Esc` goes back to the theme in effect when the list opened.
- `Enter` takes the theme and saves it into the user file as `tui.theme`, the
  default theme: every TUI started from then on uses it, and open ones switch
  to it on `config_reloaded`. Nothing is saved before `Enter`. Opened from the
  settings box, `Enter` only sets the row there.
- `/theme NAME` does the same without the list; an unknown name is a notice
  that names `/theme`, and the input stays. After `/theme ` the argument list
  offers the theme names.
- With `CLAI_THEME` set, the title says so. A theme taken in the list wins over
  it for the rest of the run, and is saved for later runs.
- With `NO_COLOR` the title says that colours are off.

## Appearance

- Colours come from the theme set in `tui.theme`: `terminal` (default, the
  terminal's 16-colour palette, so it follows the terminal's own theme),
  the built-in `dark` and `light`, or a custom theme. `NO_COLOR` turns
  colours off. Themes and all other TUI settings: [`config.md`](config.md),
  "TUI".
- Unicode symbols (`✓ ✗ ▸ ▾`) fall back to ASCII (`+ x > v`) when the locale
  is not UTF-8, or always with `tui.symbols = "ascii"`.
- Resizing the terminal re-wraps all text; no content is lost.

## Performance

- Redraw at most 30 times per second; streaming text is batched between
  frames.
- Input never waits for the network or a tool: the UI runs on its own task and
  receives events from the server over the client connection.
- Switching to a session with a long history shows its latest page within
  200 ms; older history loads in the background. **(assumption)** The first
  version replays the whole log before its first frame; paged loading is built
  if that turns out to be too slow.
- A session with 10 000 conversation lines scrolls without visible delay.

## Implementation notes

- Library: `ratatui` with the `crossterm` backend.
- The TUI only displays state and sends user actions to the server; it does
  not call the provider or run tools itself. The agent loop runs in the
  server and does not depend on the UI, so one-shot mode and tests can run
  without a terminal.
- Rendering is tested with `ratatui`'s test backend: given a state, the
  produced screen buffer is compared with an expected one.

## Open questions

1. Is an external editor (`Ctrl-G` opens `$EDITOR` for the input) wanted?
2. Answered ([RFC 0006](../rfcs/0006-tui-settings.md)): `/theme [NAME]`
   switches the theme and saves it as the default; see [Theme list](#theme-list).
3. Should keys be configurable (`[tui.keys]`)?
