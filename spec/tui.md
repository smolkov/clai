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
┌ clai ─ project ─ ~/ws/project ─ anthropic/sonnet ─ 1/3 ●1 ─ 12.3k/200k ─┐
│                                                                          │
│ > fix the failing test                                                   │
│                                                                          │
│ The test fails because `parse` drops the last line. Let me look.         │
│                                                                          │
│ ▸ read_file src/parse.rs                                       ✓ 120 ln │
│ ▾ edit_file src/parse.rs                                       ? ask    │
│   - for i in 0..lines.len() - 1 {                                        │
│   + for i in 0..lines.len() {                                            │
│                                                                          │
├──────────────────────────────────────────────────────────────────────────┤
│ Allow edit of src/parse.rs?  [y] yes  [a] always  [n] no  [e] explain    │
├──────────────────────────────────────────────────────────────────────────┤
│ > _                                                                      │
└ working… 2 requests, 1 tool call ─ Esc cancel ─ Ctrl-D detach ─ ? help ──┘
```

From top to bottom:

| Area           | Content |
|----------------|---------|
| Header         | Program name, session name, workspace path, provider and model (`anthropic/sonnet`), position among all sessions (`1/3`), number of other sessions waiting for an answer (`●1`), context usage (tokens used / limit). |
| Conversation   | Scrollable history of the session: user messages, model text, tool calls. |
| Prompt bar     | Shown only while a question (permission or limit) is open; replaces nothing else. |
| Input          | Multi-line editor for the next user message. |
| Status line    | Current state (idle, waiting for model, running tool, asking), turn counters, key hints. |

The conversation area takes all remaining height. If the terminal is smaller
than 40 × 10 cells, `clai` shows only a "terminal too small" message until it
is resized. **(assumption)**

## Sessions

The TUI shows exactly one session, the attached one. Other sessions keep
running on the server; the header counts the ones waiting for an answer to a
question (see [`sessions.md`](sessions.md), "Background work"), so the user
knows to switch to them.

The session list opens with `Ctrl-S`, `/sessions`, or `clai attach` without a
name:

```
┌ Sessions ─────────────────────────────────────────────────────────────────┐
│   NAME        WORKSPACE           MODEL               STATE       AGE      │
│ ▶ project     ~/ws/project        anthropic/sonnet    working     12 min   │
│   project-2   ~/ws/project        local/qwen          idle        1 h      │
│   docs        ~/ws/docs           anthropic/opus      ● waiting   3 min    │
│                                                                           │
│ Enter attach ─ n new ─ r rename ─ k kill ─ Esc back                       │
└───────────────────────────────────────────────────────────────────────────┘
```

| Key in list | Action |
|-------------|--------|
| `Up` / `Down` | Select a session. |
| `Enter`     | Attach to the selected session (detaches from the current one). |
| `n`         | New session: asks for workspace (default: the current session's) and name, then opens the model picker (preselected: `default_model`), then attaches. |
| `r`         | Rename the selected session. |
| `k`         | Kill the selected session, after a `y`/`n` confirmation. |
| `Esc`       | Close the list, stay on the current session. |

## Model picker

Each session has its own provider and model. The model picker chooses them.
It opens with `Ctrl-O`, `/model` without an argument, or when creating a
session.

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
  model whose context window is smaller than the session's history.
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
  "model: anthropic/sonnet → local/qwen".
- The change applies to this session only. Other sessions and
  `default_model` are not affected.

## Switching sessions

- Each session keeps the TUI's view state while the TUI is attached to
  another one: scroll position, expanded tool calls and unsent input. It is
  restored on switching back. **(assumption)**
- On attach, the TUI loads the session's full history from the server, then
  follows it live. A turn that ran while detached appears as if it had been
  watched.
- If the attached session is killed (by this or another client), the TUI
  shows a notice and opens the session list. If no session is left, it
  offers to start a new one or exit.
- If the connection to the server is lost, the TUI shows "server
  disconnected", retries for 10 s, then exits with an error; the terminal is
  restored. **(assumption)**

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
`clai answer`; the TUI then closes its prompt and shows who answered.

The TUI:

1. Expands the tool call and scrolls it into view, showing the question's
   details: diff or content, command parts with their decisions, working
   directory.
2. Shows the prompt bar with the reason (`asks: default for edit_file`) and
   the choices.
3. Takes keys for the prompt bar only, until the user answers or cancels.

| Key   | Action |
|-------|--------|
| `y`   | Allow this call once. |
| `a`   | Allow this call and add an allow rule for the session. The prompt bar shows the proposed rule (this command, this file); `Tab` widens it step by step (e.g. `cargo test *`, `src/**`), `Enter` confirms, `Esc` goes back. See [`config.md`](config.md), "Session rules". |
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
│ Turn limit reached (50 requests, 120 tool calls). Continue?  [y] yes  [n] stop │
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
- The user may type while a turn runs; the message is sent when the turn ends.
  **(assumption)**
- Lines starting with `/` are commands for `clai`, not messages to the model:

| Command    | Action |
|------------|--------|
| `/help`    | Show keys and commands. |
| `/clear`   | Start a new conversation in the same session. |
| `/model [PROVIDER/MODEL]` | Change the session's provider and model; without an argument, open the model picker. |
| `/permissions` | Show the permission rules in effect: configuration, project file and session rules, with their source. |
| `/sessions` | Open the session list. |
| `/new [NAME]` | Start a new session in the same workspace and attach to it. |
| `/attach NAME` | Attach to another session. |
| `/rename NAME` | Rename the attached session. |
| `/kill`    | End the attached session, after confirmation, and open the session list. |
| `/commit [HINT]` | Generate a commit message for all changes in the workspace and commit them after confirmation; see [Commit](#commit) and [`commit.md`](commit.md). |
| `/detach`, `/quit` | Detach and exit the TUI; the session keeps running. |

### Command list

Typing `/` as the first character of an empty input opens the command list
above the input:

```
├──────────────────────────────────────────────────────────────────────────┤
│ ▶ /model [PROVIDER/MODEL]   Change provider and model                    │
│   /new [NAME]               Start a new session and attach               │
│   /sessions                 Open the session list                        │
├──────────────────────────────────────────────────────────────────────────┤
│ > /m_                                                                    │
```

- The list shows every command with its arguments and a one-line
  description, in the order of the table above.
- Each further character filters the list: commands whose name contains the
  typed text, those that start with it first. The best match is selected.
- If nothing matches, the list shows "no command" and `Enter` sends nothing;
  the input stays so it can be corrected. To send a message that starts with
  `/` to the model, start it with `//`; the first `/` is removed.
  **(assumption)**

| Key in list         | Action |
|---------------------|--------|
| `Up` / `Down`       | Select the previous / next command. |
| `Tab`               | Complete the selected command into the input and close the list, so arguments can be typed. |
| `Enter`             | Run the selected command. If it needs an argument that is missing, complete it like `Tab` instead. |
| `Esc`               | Close the list; the typed text stays. |
| `Backspace` on the `/` | Delete the `/` and close the list. |

After the command name and a space, arguments are completed from current
values where it makes sense: `/model` offers the configured models, `/attach`
the session names. `Tab` completes the selected value.

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
| `Ctrl-N` / `Ctrl-P`  | Attach to the next / previous session in the list. |
| `PgUp` / `PgDn`      | Scroll the conversation. |
| `Ctrl-Home` / `Ctrl-End` | Jump to start / end of the conversation. |
| `Tab`                | Move focus between input and conversation. |
| `Enter` (conversation focus) | Expand or collapse the selected tool call. |
| `Up` / `Down` (conversation focus) | Select the previous / next entry. |
| `?` (conversation focus) | Show help. |

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
  200 ms; older history loads in the background. **(assumption)**
- A session with 10 000 conversation lines scrolls without visible delay.

## Implementation notes

- Library: `ratatui` with the `crossterm` backend. **(assumption)**
- The TUI only displays state and sends user actions to the server; it does
  not call the provider or run tools itself. The agent loop runs in the
  server and does not depend on the UI, so one-shot mode and tests can run
  without a terminal.
- Rendering is tested with `ratatui`'s test backend: given a state, the
  produced screen buffer is compared with an expected one.

## Open questions

1. Is an external editor (`Ctrl-G` opens `$EDITOR` for the input) wanted?
2. Should a `/theme NAME` command switch the theme without editing the
   configuration?
3. Should keys be configurable (`[tui.keys]`)?
