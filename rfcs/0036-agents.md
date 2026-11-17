# Agents

- RFC: 0036
- Status: draft
- Milestone: none
- Spec: new `spec/agents.md`; [`spec/tui.md`](../spec/tui.md), "Boxes",
  "Sessions", "Keys", "Input editor" (commands);
  [`spec/agent-loop.md`](../spec/agent-loop.md), "System prompt", "Limits";
  [`spec/config.md`](../spec/config.md), "Files", "Permissions";
  [`spec/sessions.md`](../spec/sessions.md), "Persistence";
  [`spec/architecture.md`](../spec/architecture.md), "Protocol";
  [`spec/command.md`](../spec/command.md); [`spec/skills.md`](../spec/skills.md)
  (shared frontmatter reader)

## Summary

An **agent** is a named role for a session: a description, instructions
for the model, and optionally a model, a temperature, a step limit and
stricter permissions. `codeagent` reads agents in OpenCode's format from
`.opencode/agents/` in the workspace and from `~/.config/opencode/agents/`,
so files a user already has for OpenCode work unchanged. It ships two
agents of its own: `build` (today's behaviour) and `plan` (no edits).

Each session runs one agent. In the TUI:

- `/agent` opens a box at the bottom with two tabs. **Agents** lists the
  agents to choose for this session. **Running** lists every session on
  the server with its agent, model, state, current activity and use, and
  `Enter` switches to it.
- `/agent NAME` and `Shift-Tab` switch the agent of the attached session.
- The status row shows the agent. The session list has an `AGENT` column.

`codeagent new`, `run` and `chat` take `--agent NAME`; `codeagent agents`
lists the agents found.

## Motivation

- A user who works in several roles (analysing legacy code, designing,
  implementing) rewrites the same instructions at the start of each
  session, or keeps them in a file and pastes them in. Skills (RFC 0023)
  add knowledge for a task. They do not change who the model is, which
  model runs, or what it may do.
- OpenCode users already have these files. This repository has three, in
  `.opencode/agents/` (`analyst`, `architect`, `developer`), and
  `codeagent` ignores them today.
- Several sessions often run at the same time, one per role or task. The
  session list shows their state, but not what each is doing now, which
  role it has, or what it has cost. To compare, the user has to attach to
  each one.

## Scope

In:

- Finding and reading agent files: OpenCode's current format, and the
  deprecated keys still common in existing files.
- The built-in agents `build` and `plan`.
- One agent per session: choosing, switching, saving and resuming it.
- What an agent changes: the system prompt, the model, the temperature,
  the step limit, the tools offered and the permissions, which an agent can
  only tighten.
- TUI: the `/agent` box with its Agents and Running tabs, `/agent NAME`,
  `Shift-Tab`, the status row and the session list column.
- `codeagent agents`, and `--agent` for `new`, `run` and `chat`.
- Protocol: the `agents` and `set_agent` requests, the `agent_changed`
  event, and new fields in `SessionInfo`. Protocol version 11.

Out:

- **Subagents.** OpenCode lets a session start child sessions with a
  `mode: subagent` agent through a tool. That needs its own design for the
  tool, the permissions between parent and child, the reports and cost
  limits. Here `subagent` agents are listed but cannot be chosen; see open
  question 1. The Running tab is built so that child sessions can appear
  under their parent later.
- Agents in OpenCode's JSON config (`opencode.json`, the `agents` key).
- Claude Code's agent files (`.claude/agents/`); see open question 2.
- Writing agent files from the TUI.
- Per-agent request headers and bodies (OpenCode `request`).

## Design

### Format

An agent is a Markdown file. Its name is the path below the agents
directory without `.md`: `.opencode/agents/analyst.md` is `analyst`, and
`.opencode/agents/team/reviewer.md` is `team/reviewer`.

```markdown
---
description: Reconstructs and explains legacy C/C++ systems
mode: primary
model: anthropic/sonnet
temperature: 0.2
steps: 40
color: "#7aa2f7"
permission:
  edit: deny
  bash:
    "git log*": allow
    "*": ask
---

Act as a forensic code analyst for a legacy Embedded Linux C/C++ codebase.
Do not modify code.
```

The body, everything after the frontmatter, is the agent's instructions.

| Key | Value | Use in `codeagent` |
|-----|-------|--------------------|
| `description` | text, required, at most 1 024 characters | Shown in the box and in `codeagent agents`. |
| `mode` | `primary`, `subagent`, `all`; default `primary` | `primary` and `all` can be chosen. `subagent` is listed, dim, as "subagent: not supported yet". |
| `model` | `provider/model` | A model reference of the user file (`anthropic/sonnet`). Failing that, a provider name and a model `id` (`anthropic/claude-sonnet-5-5`), as OpenCode writes it. A `#variant` suffix is ignored. A model that is not found gives a warning, and the agent runs on the session's model. |
| `temperature` | number 0–2 | Replaces the model's `temperature` while the agent runs. |
| `steps`, `maxSteps` | whole number ≥ 1 | Model requests per turn. The lower of this and `limits.max_requests` applies. |
| `permission` (deprecated in OpenCode) | table | See [Permissions](#permissions). |
| `permissions` | list of `{action, resource, effect}` | See [Permissions](#permissions). |
| `tools` (deprecated in OpenCode) | table `name: false` | The tool is not offered. See [Permissions](#permissions). |
| `hidden` | `true` / `false` | Not listed, and not part of the `Shift-Tab` round; `/agent NAME` still works. |
| `disable`, `disabled` | `true` / `false` | The file is passed over, as if it were not there. |
| `color` | `#rrggbb` | The agent's name in the status row and in the box. Ignored with `NO_COLOR` or a theme without colours. |
| `prompt`, `system`, `top_p`, `request`, `variant` and other keys | — | Read and ignored, with no warning. `prompt` points to a file in OpenCode, and the body serves that purpose here. |

#### Reading the file

The frontmatter reader of skills (`src/skills/parse.rs`) moves to
`src/frontmatter.rs` and is used by both. It gains three things, which
skills accept as well:

- **Nested tables:** indented `key: value` lines below a `key:` line, up
  to three levels deep. They are read as dotted keys, e.g. `permission.edit`
  and `permission.bash.git log*`. A quoted key keeps its spaces and `*`.
- **Lists of tables:** `- key: value` items with indented keys, for
  `permissions`.
- **A wider closing line:** a line of three or more `-` closes the
  frontmatter, and blank lines inside it are passed over. The three files in
  this repository have both. A Markdown formatter turned their `---` into
  a heading underline (`-----------`) and added a blank line after the
  opening `---`. Files like these should still load.

The rules for skipping a file follow those of skills: over 64 KiB, not
UTF-8, no frontmatter, a malformed line, no description, a bad name. Such a
file is skipped with a warning; the other agents still load. Names are
the skill rule plus `/` between parts: lowercase letters, digits, `-`,
`_`, each part 1–64 characters. **(proposal)** `_` is allowed because agent
names come from file names, not from a `name` key.

### Where agents are found

Highest priority first:

| Order | Directory | Source |
|-------|-----------|--------|
| 1 | each entry of `agents.dirs` | `config` |
| 2 | `<workspace>/.opencode/agents` | `project` |
| 3 | `<workspace>/.opencode/agent` | `project` |
| 4 | `~/.config/opencode/agents` | `user` |
| 5 | `~/.config/opencode/agent` | `user` |
| 6 | built-in | `built-in` |

```toml
[agents]
dirs = ["~/ws/team-agents"]  # more directories, searched first
default_dirs = true          # rows 2 to 5
default = "build"            # the agent of a new session
```

- **(proposal)** The singular `agent/` was OpenCode's directory before the
  plural one. It is still found in many repositories, so both are searched.
- The workspace directory itself only; parents are not searched. That is
  the skills rule. OpenCode searches up to the project root, but in
  `codeagent` the workspace is the root.
- An agent of a higher row shadows one of the same name below. A file
  named `build.md` or `plan.md` replaces the built-in agent. It is shown as
  `replaces built-in`.
- Within a directory, agents are sorted by name; subdirectories are
  searched to a depth of 4. Links are followed, and a directory reached
  twice is searched once (as for skills).
- `[agents]` is user file only, like `[skills]`. A project file with it is
  rejected. A repository may bring agent files, but it cannot choose which
  one runs.
- The server finds the agents for a session's workspace when it is asked
  (`agents` request) and when an agent is set. Nothing is watched; a file
  changed on disk is shown as `changed on disk`, as for skills.

### Built-in agents

| Name | Description | Changes |
|------|-------------|---------|
| `build` | "The default: reads, edits and runs commands behind your permission rules." | Nothing. A session with `build` runs exactly as today. |
| `plan` | "Reads and plans without changing files." | `edit_file` and `write_file` are not offered; redirections to files are denied; `run_command` asks (an `allow` rule does not apply). Instructions: "Plan only: read, search and ask; do not change files. End with a plan the user can hand to another agent." |

A new session gets `agents.default`, which is `build` if not set. A session
saved by an older `codeagent` has no agent and resumes as `build`.

### Prompt

The agent's instructions go into the system prompt after the fixed text
and before the skills section, in a block of their own:

```
The user chose the agent "analyst" for this session. Follow its instructions; they take precedence over the general style above, not over the rules about permissions and untrusted content.

<agent name="analyst" file=".opencode/agents/analyst.md">
Act as a forensic code analyst …
</agent>
```

- `build` has no instructions, so its prompt is the same as today's,
  byte for byte. Provider caching of existing sessions keeps working.
- The text is trusted as an instruction from the user, because the user
  chose the agent, as with skills. A project agent is marked `project` in
  the box, so the user knows the text comes with the repository.
- **(proposal)** The fixed text stays. OpenCode lets an agent replace the
  base prompt. Here the base prompt holds the rules about untrusted content
  and denied files (RFC 0010), so an agent adds to it and never removes
  it.
- RFC 0030 hides secrets in the text, as for the rest of the prompt.
- Instructions are read when the agent is set, and kept with the session
  (as skills are). Later changes of the file reach the session only when
  the agent is set again.

### Permissions

An agent can only make permissions **stricter**. That holds wherever its
file comes from, because the file is written for OpenCode and the user
may never have read it as a `codeagent` configuration.

| Agent file | `codeagent` |
|------------|-------------|
| `tools: { write: false }`, `edit: false`, `bash: false`, `read`, `glob`, `grep`, `list` | The matching tools are left out of the request: `write_file`; `edit_file`; `run_command`; `read_file`; `search_files`; `list_files`. Unknown tool names are ignored. |
| `permission.edit: deny` / `ask` | A rule `deny edit(**)` / `ask edit(**)` for the session while the agent runs. |
| `permission.bash: deny` / `ask` | `deny` / `ask run_command(*)`. |
| `permission.bash."git push*": deny` | `deny run_command(git push*)`: the OpenCode pattern is read as a command pattern (words, globs). A pattern that cannot be read gives a warning and is left out. |
| `permission.webfetch`, `websearch`, `skill`, `external_directory`, `doom_loop` | Ignored: `codeagent` has no such tool, or a boundary of its own. |
| `permissions: [{action: shell, resource: "rm *", effect: deny}]` | As above: `shell` → `run_command`, `edit` → `edit`, `read` / `glob` / `grep` → `read`, `subagent` ignored. |
| any `allow` | Ignored, and listed among the agent's warnings: `allow rules of agents are not used; add them to your config file`. |

- The rules have the source `agent analyst` (`Source::Agent`). They are
  checked with the others (deny before ask before allow), so they apply on
  top of the user's and the project's rules. The question says why:
  `asks: rule ask run_command(*) (agent analyst)`.
- Session rules from "always" stay with the session when the agent
  changes. An agent's `deny` and `ask` rules still win over them.
- **(proposal)** OpenCode's "the last matching rule wins" order is not
  kept. In `codeagent` a `deny` always wins, and that is simpler to explain
  than two orders.

### Model and limits

- Setting an agent with a `model` sets the session's model to it (a
  `model_changed` event, as `/model` gives), and a notice says so:
  `agent analyst: model anthropic/sonnet`. Setting an agent without a
  `model` keeps the current one. **(proposal)** The model a user picked
  with `/model` after the agent was set stays until the next agent change.
- `temperature` applies while the agent is set, in place of the model's
  key.
- `steps` lowers `max_requests` for the session while the agent runs. When
  the limit is reached, the usual limit question asks (`spec/agent-loop.md`,
  "Limits"). OpenCode's "last step summarises" is not taken over.

### Switching

- An agent change applies from the next model request; a request that
  runs finishes with the old agent, as for skills. A change while a turn
  runs is allowed.
- Every client gets `agent_changed`, and the TUI shows the notice
  `agent: analyst (was build)`.
- **(proposal)** The history stays. The new agent sees the conversation so
  far, which is the point of switching: plan with `plan`, then build with
  `build` in the same session. Starting fresh is `/new --agent NAME`.
- Saved: `agent_changed` in the session log, with the name, the file, the
  instructions, the model reference and the rules as read. A resumed
  session replays it and runs on the saved agent, even when the file
  changed or is gone (shown as `changed on disk` or `not found any more`,
  as for skills).
- `codeagent export` and `codeagent import` carry the event. Its rules
  are only `deny` and `ask`, so importing them cannot widen anything; see
  RFC 0035.

### TUI

#### Status row and session list

The status row names the agent before the model, in its colour:

```
  project · ~/ws/project · analyst · anthropic/sonnet · 1/3 · 12.3k/200k
```

`build` is shown too. A session always has an agent, and leaving it out
would make `build` look like "none". On a narrow terminal the agent is
left out after the workspace.

The session list gains an `AGENT` column after `NAME`. It is the first
column left out on a narrow terminal, after the time.

#### The agent box

`/agent` (alias `/agents`) opens a box at the bottom, as the skills box and
the model picker do (RFC 0024). It has two tabs, named in its title.
`Tab`, `Shift-Tab`, `Left` and `Right` switch them, as in the status box.

**Agents** tab: the agents for the attached session's workspace.

```
── Agent for "project" │ Running ─────────────────────────────────────────────
Filter: _

.opencode/agents
▶ analyst     Reconstructs and explains legacy C/C++ systems          ✓ current
  architect   Designs and evaluates Embedded Linux C/C++ architecture
  developer   Implements production-grade C/C++ changes for Embedded Linux
built-in
  build       The default: reads, edits and runs commands behind your rules
  plan        Reads and plans without changing files
  ·reviewer   subagent: not supported yet

analyst · project · model: session's (anthropic/sonnet) · tools: all · rules: deny edit(**)
⚠ 1 warning: developer.md: allow rules of agents are not used
Enter select ─ Tab running ─ Esc cancel
──────────────────────────────────────────────────────────────────────────────
```

- Rows are grouped by directory, in order of priority, as in the skills
  box. Shadowed agents are dim and name the one that wins; subagents are
  dim and cannot be chosen.
- The line below the list shows the selected agent: name, source, model,
  the tools left out, its rules, and `changed on disk` when that applies.
- `Enter` sets the selected agent for the session (`set_agent`) and closes
  the box. On the current agent it only closes.
- Typed keys edit the filter (name and description).
- The warning line counts skipped files and ignored keys; the first one is
  shown in full.

**Running** tab: every running session of the server (not the saved
ones), with what it does now.

```
── Agent for "project" │ Running ──────────────────────────────── 3 running ──
  NAME        AGENT      MODEL             STATE       NOW                     TURN    TOKENS   COST
✓ project     analyst    anthropic/sonnet  working     read_file src/io.c      1m 12s  48.1k    $0.21
▶ api-work    developer  anthropic/opus    ● waiting   asks: run_command make  4m 03s  112k     $1.40
  docs        plan       local/qwen        idle        —                       —       9.8k     —

api-work · ~/ws/api · 14 turns · 2 queued · question open for 2m 10s
Enter attach ─ a agent… ─ Tab agents ─ Esc close
──────────────────────────────────────────────────────────────────────────────
```

- `NOW` is the session's activity line (RFC 0015) cut to fit: the running
  tool's summary, `thinking`, `writing`, `asks: <summary>` for an open
  question, or `—` when idle.
- `TURN` is how long the running turn has taken; `TOKENS` and `COST` are
  totals over the session, from the session's statistics (`—` when the
  model has no price).
- The box asks the server every second while this tab is shown, and the
  selection stays on the same session.
- `Enter` attaches to the selected session, as `Enter` in the session list
  does (RFC 0005). That is "switching between running agents". On the
  attached session it only closes.
- `a` opens the Agents tab for the selected session, to change its agent
  without attaching to it first.
- The line below the list shows the selected session's workspace, turns,
  queued messages and how long its question has been open.
- On a narrow terminal `COST`, then `TOKENS`, then `TURN` are left out.

#### Commands and keys

| Command / key | Action |
|---------------|--------|
| `/agent` | Open the box on the Agents tab. |
| `/agent NAME` | Set that agent for the session, without the box. Unknown name: an error that lists the names found. After `/agent `, the value list offers the agents. |
| `/agents running` | Open the box on the Running tab. |
| `Shift-Tab` (input focus) | Next primary agent of the session's workspace, round the end: built-in first, then the found ones, without hidden ones. The notice says which. |
| `Tab` | As today: moves focus between input and conversation. |

**(proposal)** `Shift-Tab` moves the focus today, like `Tab`. Using it for
the agent round in input focus is what OpenCode users expect (OpenCode
uses `Tab`, which is taken here). In conversation focus it still moves the
focus. See open question 3.

The box does not open while a question is open, and a question that opens
closes it, as for the other boxes. Help (`?`) and `/help` list the new
command and key.

### Other clients

- `codeagent agents [--workspace DIR]` prints the agents found, grouped by
  directory, with their mode, model and warnings (like `codeagent skills`).
- `codeagent new --agent NAME`, `codeagent run --agent NAME` and
  `codeagent chat --agent NAME`. An unknown name is exit `2`, with the list
  of names. `codeagent run` without `--agent` uses `agents.default`.
- `codeagent chat`: `/agent` prints the agents, `/agent NAME` sets one,
  `/agents running` prints the Running table once.
- `codeagent ls` gains the `AGENT` column.

### Protocol

Version 11.

| Message | Content |
|---------|---------|
| request `agents` | `session`; answer `Agents(AgentsInfo)`: the agents found (name, description, mode, source, directory, file, model, tools left out, rules, hidden, shadowed by, changed on disk), the session's current agent, and the warnings. |
| request `set_agent` | `session`, `name`. Resumes a saved session, as `set_model` does. Answer: the new `AgentsInfo`. |
| event `agent_changed` | `name`, `from`, `file`, `text`, `model`, `temperature`, `steps`, `tools` (left out), `rules`. In the log, so resume and export carry it. |
| `SessionInfo` | New fields `agent` (name), `activity` (the activity line text or none), `turn_started` (seconds since 1970, or `0` when idle), `tokens` and `cost` (totals). Defaults for an older server: empty, none and `0`. |

`list` already returns every session. The Running tab uses it, so no new
request is needed for running sessions.

### Code

| Place | Change |
|-------|--------|
| `src/frontmatter.rs` | The reader, moved from `src/skills/parse.rs`, with nested tables, lists of tables and the wider closing line. |
| `src/agents/` (new) | `mod.rs` (`Agent`, `Found`, finding and shadowing, the built-in agents), `parse.rs` (keys, model reference, permission mapping), `tests.rs`. |
| `src/config/` | `[agents]`: `dirs`, `default_dirs`, `default`; rejected in a project file. |
| `src/permissions/` | `Source::Agent(name)`; the agent's rules as one more layer of the session's rules, replaced on each change. |
| `src/agent/` | The prompt block; tools left out; `temperature` and `steps`; `agent_changed` replay. |
| `src/session/`, `src/server/requests.rs`, `src/protocol/` | Requests, event, `SessionInfo` fields. |
| `src/tui/` | `view/agents.rs` (the box), `app.rs` (inputs and effects), `commands.rs`, `keys.rs` (`Shift-Tab` in input focus), the status row and the session list column. |
| `src/command/` | `agents.rs`, `--agent` for `new`, `run`, `chat`; `/agent` in `chat`; `ls` column. |

## Dependencies

None. The frontmatter reader stays hand-written: no YAML crate.

## Spec changes

- `spec/agents.md`: new, with format, directories, built-in agents,
  prompt, permissions, model and limits, switching, saving and clients.
  Listed in `spec/readme.md` and `spec/SUMMARY.md`.
- `spec/skills.md`: "Format" points to the shared reader and gains nested
  tables, lists and the wider closing line.
- `spec/config.md`: "Files" gains `[agents]` (user file only); a new
  "Agents" section; "Checking order" names the agent's rules and their
  source.
- `spec/agent-loop.md`: "System prompt" gains the agent block; "Limits"
  gains `steps`.
- `spec/tui.md`: "Boxes" lists the agent box; a new "Agents" section with
  both tabs; "Sessions" gets the `AGENT` column; the status row; "Keys"
  gets `Shift-Tab`; the command list gets `/agent` and `/agents running`.
- `spec/command.md`: `codeagent agents`; `--agent` for `new`, `run` and
  `chat`; `/agent` in `chat`; the `ls` column.
- `spec/architecture.md`, "Protocol": `agents`, `set_agent`,
  `agent_changed`, the `SessionInfo` fields, version 11.
- `spec/sessions.md`, "Persistence": `agent_changed` in the log.
- `spec/security.md` (RFC 0035), if built first: agent files as an entry
  point from the repository; their guard is the user's choice and the
  "only stricter" rule.
- `readme.md`: an "Agents" section with an example file and the keys;
  `todo.md`, `CHANGELOG.md`.

## Testing

- `src/frontmatter.rs`:
  - Nested tables, three levels deep, with quoted keys.
  - Lists of tables.
  - The wider closing line and blank lines in the frontmatter.
  - The three files of `.opencode/agents/` in this repository, copied as
    fixtures, read with the right description and mode.
  - Every existing skills parser test still passes.
- `src/agents/tests.rs`:
  - Finding: the order of directories, `agent/` and `agents/`, nested
    names, shadowing, `build.md` replacing the built-in agent, `disable`,
    `hidden`, a missing configured directory, a link loop.
  - Keys: a model found by reference and by `id`, an unknown model with its
    warning, `#variant` ignored, `steps` and `maxSteps`, `temperature` out
    of range.
  - Permissions: each row of the mapping table; `allow` ignored with the
    warning; a bad command pattern left out with a warning.
- `src/agent/tests.rs`:
  - `build` gives today's prompt byte for byte.
  - `analyst` adds its block before the skills.
  - `plan` leaves out `edit_file` and `write_file`, and `echo x > f` is
    denied.
  - `steps: 2` asks the limit question after two requests.
  - A change during a turn applies from the next request.
- `src/permissions/tests.rs`: an agent's `deny` wins over a session `allow`;
  the reason names `agent analyst`.
- `src/server/tests.rs`:
  - `agents` and `set_agent`; `agent_changed` reaches all clients; the
    model changes with the agent.
  - Resume and export or import keep the agent; a log of protocol 10
    resumes as `build`.
  - `SessionInfo` has the activity and the turn start.
- `src/config/tests.rs`: `[agents]` in a project file is a load error; an
  unknown `default` gives the warning and uses `build`.
- `src/tui/tests.rs`, snapshots:
  - The Agents tab, with groups, a shadowed and a subagent row, and a
    warning.
  - The Running tab with three sessions, and on a narrow terminal.
  - The status row with an agent.
  - `Shift-Tab` going round; `/agent NAME` and an unknown name;
    `Enter` on Running attaching to the other session.
- `src/command/` tests: `codeagent agents`, `--agent` with an unknown name
  (exit `2`), the `ls` column.

## Implementation plan

Each step leaves a working, tested state.

1. **Shared frontmatter reader**
   - Status: todo
   - Files: `src/frontmatter.rs`, `src/skills/parse.rs` (removed),
     `src/skills/mod.rs`, `spec/skills.md`
   - Result: skills read as before; the reader also reads nested tables,
     lists and the wider closing line.
   - Tests: the reader tests; the skills tests unchanged.

2. **Agent files**
   - Status: todo
   - Files: `src/agents/`, `src/config/` (`[agents]`), `spec/agents.md`,
     `spec/config.md`
   - Result: agents are found, read, shadowed and checked; the built-in
     ones exist; nothing uses them yet.
   - Tests: `src/agents/tests.rs`, config tests.

3. **Agents in the session**
   - Status: todo
   - Files: `src/agent/`, `src/permissions/`, `src/session/`,
     `src/server/requests.rs`, `src/protocol/` (version 11),
     `spec/agent-loop.md`, `spec/architecture.md`, `spec/sessions.md`
   - Result: `set_agent` changes the prompt, model, temperature, limit,
     tools and rules; saved, resumed, exported.
   - Tests: agent, permission and server tests.

4. **Command line**
   - Status: todo
   - Files: `src/command/agents.rs`, `src/command/new.rs`, `run.rs`,
     `chat/`, `ls.rs`, `src/cli.rs`, `spec/command.md`
   - Result: `codeagent agents`, `--agent`, `/agent` in `chat`, the `ls`
     column.
   - Tests: command tests.

5. **TUI: Agents tab, command, status row, `Shift-Tab`**
   - Status: todo
   - Files: `src/tui/view/agents.rs`, `app.rs`, `commands.rs`, `keys.rs`,
     `view/status_row.rs`, `view/popup.rs` (help), `spec/tui.md`
   - Result: choosing and switching the agent in the TUI.
   - Tests: snapshots and key tests.

6. **TUI: Running tab and session list column**
   - Status: todo
   - Files: `src/tui/view/agents.rs`, `view/sessions.rs`,
     `src/protocol/` (`SessionInfo` fields), `src/session/` (activity and
     totals in the info), `spec/tui.md`
   - Result: every running session with agent, activity, turn time, tokens
     and cost; `Enter` attaches.
   - Tests: snapshots, the refresh, attaching.

7. **Docs**
   - Status: todo
   - Files: `readme.md`, `todo.md`, `CHANGELOG.md`, `rfcs/readme.md`
   - Result: users know how to write and pick agents.
   - Tests: `cargo fmt`, `clippy -D warnings`, `cargo test`, `ci/check.sh`.

## Alternatives

- **Agents as skills with extra keys.** Skills are additive and several are
  active at once; an agent is one per session and changes the model and
  the rules. Mixing them would make "turn on two agents" a question with
  no good answer. Rejected.
- **A `codeagent` format of its own (`.codeagent/agents/*.toml`).** It
  would be cleaner to check, but users would have to write the same agent
  twice. Rejected: read OpenCode's files, and add `agents.dirs` for more
  places.
- **Let agents widen permissions, at least from the user's own
  directory.** It matches OpenCode. But `~/.config/opencode/agents` was
  written for another program, with its defaults. An `allow bash: "*"`
  there would silently turn off every question in `codeagent`. Rejected.
  `allow` rules belong in `codeagent`'s config.
- **A separate "running agents" box, apart from the session list.** The
  session list is about managing sessions (rename, delete, filter, saved
  ones). The Running tab is about watching what works now. They share the
  data, not the job. A column in the session list alone would not fit
  activity and cost. Kept apart, with `AGENT` added to the list.
- **Start a fresh conversation on each agent change.** Planning and then
  building in one session would be lost. Rejected; `/new --agent` gives a
  fresh start.

## Open questions

1. Subagents: should a later RFC add a `subagent` tool that starts a child
   session with a `mode: subagent` agent, shown under its parent in the
   Running tab? Proposal: yes, as its own RFC, after this one is built.
2. Should `.claude/agents/` (Claude Code's format: `name`, `description`,
   `tools` as a comma list, `model` as `sonnet` / `opus` / `haiku`) be read
   too, as skills read `.claude/skills`? Proposal: yes, in this RFC, as rows
   after the OpenCode ones, if the mapping stays this small.
3. `Shift-Tab` for the agent round in input focus, or another key? It
   changes what `Shift-Tab` does today. Proposal: `Shift-Tab`.
4. Should a project file be allowed to name the default agent for its
   workspace (`[agents] default` in `.codeagent/config.toml`)? It would put
   repository text into the prompt without the user choosing it.
   Proposal: no; a project can only bring files, and the user chooses.
5. Is `build` the right name for the default agent, matching OpenCode, or
   should it be `default`? Proposal: `build`.
