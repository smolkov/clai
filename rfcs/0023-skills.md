# Skills

- RFC: 0023
- Status: draft
- Milestone: none; it adds to the system prompt of
  [`spec/agent-loop.md`](../spec/agent-loop.md) and the command list of
  RFC 0004
- Spec: [`spec/agent-loop.md`](../spec/agent-loop.md), "System prompt";
  [`spec/config.md`](../spec/config.md), "Files", "Project files",
  "Reloading"; [`spec/tui.md`](../spec/tui.md), "Input editor",
  "Command list"; [`spec/command.md`](../spec/command.md), "clai run",
  "clai chat"; [`spec/sessions.md`](../spec/sessions.md), "Persistence";
  [`spec/architecture.md`](../spec/architecture.md), "Protocol";
  new [`spec/skills.md`](../spec/skills.md)

## Summary

A skill is a directory with a `SKILL.md` file: a name, a description and
instructions for the model, maybe with scripts and reference files next to
it. This is the format OpenCode, Claude Code and other agents already
use, so skills a user has for them work in `clai` too. `clai` finds skills
in the usual directories of those tools, in the user's home and in the
workspace, and in extra directories named in a new `[skills]` table of the
user file. A configured directory wins when two skills have the same name.
The new command `/skills` opens a selection box: the user ticks the skills
the session should use. An active skill's instructions go into the
system prompt, and its directory becomes readable for the file tools.

## Motivation

- Users keep instructions for recurring work (a release check, a review
  list, how to use a tool of the team) as skills for other agents. Today
  `clai` cannot use them; the user pastes the text into each session.
- Skills are already on disk in known places
  (`~/.claude/skills`, `.agents/skills`, …). Reading them from there
  means one copy for all tools.
- Teams share skills in a directory outside the usual places (a
  shared repository, a mounted drive). That needs a configured path.
- The system prompt is fixed (`spec/agent-loop.md`). Skills are the first
  user-chosen part of it, and a deliberate one: nothing is added unless
  the user picks it.

## Scope

In:

- The skill format `clai` reads, and its checks.
- Where skills are found, in which order, and which one wins on the same
  name.
- `[skills]` in the user file: extra directories, and turning off the
  default ones.
- `/skills` in the TUI: the selection box; `/skills NAME` to toggle one.
- Active skills of a session: in the system prompt, readable directory,
  saved with the session.
- `/skills` in `clai chat`, `clai run --skill NAME`, `clai skills` to
  list what was found.
- The protocol: `skills` and `set_skills`.

Out:

- The model choosing skills by itself (only names and descriptions in the
  prompt, a tool to load one). See "Open questions".
- Fields of other tools' formats beyond `name` and `description`
  (`allowed-tools`, `model`, `license`, …): read and ignored.
- Creating, editing or installing skills from `clai`.
- Skills that change permissions. A script in a skill directory runs
  through `run_command` with the normal rules.
- Commands, agents or other plugin parts of other tools.

## Design

### Skill format

```
~/.claude/skills/
  release-check/
    SKILL.md
    scripts/check-changelog.sh
    reference.md
```

```markdown
---
name: release-check
description: Steps before tagging a release of a Rust crate. Use when the user asks to prepare or check a release.
---

# Release check

1. Run `cargo test` and `cargo clippy` …
```

- A skill is a directory directly below a skills directory that contains
  a file `SKILL.md`. Deeper directories are not searched. **(proposal)**
- The file starts with a frontmatter block between two `---` lines.
  `name` and `description` are read from it; other keys are ignored.
- `name`: 1 to 64 characters, lowercase letters, digits and `-`, not
  starting or ending with `-` (the rule of the common format). Without
  `name`, the directory name is used. A `name` that differs from the
  directory name is used, with a warning in `clai skills`. **(proposal)**
- `description`: required, at most 1 024 characters, one line in the list
  (newlines become spaces).
- The body (everything after the frontmatter) is the instruction text. At
  most 64 KiB for the whole file **(proposal)**; a larger file is skipped
  with a warning, as is a file that is not UTF-8.
- Frontmatter values may be plain, `'single'` or `"double"` quoted, or a
  `>` / `|` block on the following indented lines. That covers the files
  in use; a file the reader cannot parse is skipped with a warning naming
  the file and line. No YAML crate (see "Dependencies").
- Links are followed for the skill directory and `SKILL.md`, since users
  link skills from a shared checkout into `~/.claude/skills`.

A skill that is skipped is not an error for `clai`: the other skills still
load, and the warnings show in `clai skills` and in the `/skills` box.

### Where skills are found

Directories, in order of priority, highest first:

| Order | Directory | Source |
|-------|-----------|--------|
| 1 | each entry of `skills.dirs`, in the order written | `config` |
| 2 | `<workspace>/.opencode/skills` | `project` |
| 3 | `<workspace>/.claude/skills` | `project` |
| 4 | `<workspace>/.agents/skills` | `project` |
| 5 | `~/.config/opencode/skills` | `user` |
| 6 | `~/.claude/skills` | `user` |
| 7 | `~/.agents/skills` | `user` |

- When two skills have the same name, the one from the directory higher
  in the table wins; the other is **shadowed**: listed, dim, with the
  winner's path, but it cannot be activated.
- Configured directories come first, as asked. Between the default ones,
  the workspace comes before the home directory **(proposal)**, so a
  project can give a skill its own version, as for `.clai/config.toml`
  over the user file. Within each group, the order of the table.
- `~/.config` is `$XDG_CONFIG_HOME` when it is set (`spec/config.md`);
  that applies to `opencode`, not to `~/.claude` and `~/.agents`, which
  those tools always keep in the home directory.
- The workspace directories are those of the workspace itself; parent
  directories are not searched, like `.clai/config.toml`. **(proposal)**
- A directory that does not exist is skipped without a warning; one that
  cannot be read gets a warning.

### Configuration

In the user file:

```toml
[skills]
# More directories with skills, searched before the default ones. Absolute,
# or starting with ~/. A skill here wins over one of the same name elsewhere.
dirs = ["~/ws/team-skills", "/opt/skills"]

# Search the default directories of OpenCode, Claude Code and .agents, in
# the home directory and in the workspace. Default: true.
default_dirs = true
```

- `dirs`: list of paths, absolute or starting with `~/`; a relative path
  is a load error naming the entry. Default: empty. A configured
  directory that does not exist gets a warning (unlike a default one),
  because the user named it.
- `default_dirs = false` searches only `dirs`.
- A project file may not have `[skills]`: a path in it would let a
  repository put files from anywhere on the machine into the prompt and
  make them readable for the model. Such a file is rejected with an error
  naming the file, like `[tui]`. The workspace's own skill directories
  are searched anyway (unless `default_dirs = false`).
- `[skills]` is part of the reload (`spec/config.md`, "Reloading"): a
  changed list applies to the next `skills` request. Sessions keep the
  skills they have active.

### The selection box

`/skills` opens it, like the model picker:

```
┌ Skills for "project" ─────────────────────────────────────────────────────┐
│ Filter: _                                                                 │
│                                                                           │
│ ~/ws/team-skills                                                          │
│ ▶ [x] release-check   Steps before tagging a release of a Rust crate. …   │
│   [ ] api-style       How our HTTP handlers are written and tested.       │
│ .claude/skills                                                            │
│   [ ] migrations      Writing and checking database migrations.           │
│ ~/.claude/skills                                                          │
│   [ ] pdf             Read, fill and merge PDF files.                     │
│       release-check   shadowed by ~/ws/team-skills/release-check          │
│                                                                           │
│ 1 active ─ ⚠ 1 skipped: ~/.agents/skills/old/SKILL.md: no description     │
│ Space toggle ─ Enter apply ─ Esc cancel                                   │
└───────────────────────────────────────────────────────────────────────────┘
```

| Key | Action |
|-----|--------|
| `Up` / `Down` | Select the previous / next skill; shadowed rows are passed over. |
| `Space` | Tick or untick the selected skill. |
| `Enter` | Apply: the ticked skills become the session's active skills, the box closes. |
| `Esc` | Close without changes. |
| other keys | Edit the filter (name and description, any case). |

- Rows are grouped by directory, in the order of priority; a group
  header shows the directory with `~` for the home directory and relative
  to the workspace for project ones. Empty directories have no header.
- The box opens with the session's active skills ticked. An active skill
  that is no longer found (deleted, or its directory left the config) is
  listed at the top as `[x] name  not found any more`; unticking it is the
  only change possible for it.
- `Space` is free for the tick for the same reason as in the model picker
  (RFC 0021): names have no spaces. Pasted text still goes into the
  filter.
- The description is cut to the box width with `…`; the selected row's
  full description shows in the line above the footer when it does not
  fit. **(proposal)**
- Warnings (skipped files, missing configured directories) are counted in
  the status line of the box, the first one in full.
- With no skills found, the box lists the searched directories and a
  short example of a `SKILL.md` instead.
- `Enter` with no change only closes. A change adds a notice to the
  conversation: `skills: + release-check, − pdf`.
- The box does not open while a question is open, and a question that
  opens closes it, as for the model picker.
- In ASCII mode the ticks stay `[x]` / `[ ]`; separators follow
  `tui.symbols`.

`/skills NAME` toggles one skill without the box and adds the same
notice; an unknown name is an error that lists the found names. After
`/skills `, the value list of the command list (RFC 0004) offers the
found skills, active ones marked. The command table gets the row:

| Command | Action |
|---------|--------|
| `/skills [NAME]` | Choose the session's skills; with a name, turn that skill on or off. |

### Active skills in a session

- The active skills belong to the session, like its model: other sessions
  are not affected, and a new session starts with none. **(proposal)**
- When a skill is activated, the server reads its `SKILL.md` and keeps
  the text with the session. Later changes of the file are not seen by
  that session until the skill is turned off and on again; `/skills`
  marks such a skill `changed on disk`. The prompt stays the same during
  a session, so provider caching keeps working. **(proposal)**
- The system prompt gets a section after the fixed text, one block per
  active skill in the order of activation:

  ```
  The user turned on these skills. Follow their instructions when the task fits them.
  Files the instructions name are relative to the skill's directory.

  <skill name="release-check" dir="/home/user/ws/team-skills/release-check">
  # Release check
  …
  </skill>
  ```

  Without active skills the prompt is unchanged. A change applies from the
  next model request; a request that runs finishes with the old prompt.
- Each active skill's directory is added to the session's directories as
  **read only**, like an entry of `extra_dirs`: `read_file`, `list_files`
  and `search_files` may use it, and the "Other directories" line of the
  prompt names it. Writing there needs an `extra_dirs` entry with write
  access, as now. `run_command` is not changed: a script of the skill
  runs only if the rules allow the command. **(proposal)**
- The skills' text counts towards the context like the rest of the
  system prompt; the status box shows the active skills and their size in
  tokens (estimated).
- The skill text is trusted as the user's instruction because the user
  chose it; the line of the prompt that says file contents are data still
  holds for files read with tools, including files in the skill
  directory.

### Saving and resuming

- Activation and deactivation are events in the session log
  (`spec/sessions.md`, "Persistence"): `skill_on` with name, directory
  and text, `skill_off` with name. A resumed session rebuilds its active
  skills from the log, with the saved text, so it continues with the
  prompt it had. A skill whose directory is gone keeps working from the
  saved text, but its directory is no longer readable, and the box says
  `not found any more`.
- `clai export` includes the events; `clai import` reads them. A log of
  an older `clai` has none: no active skills.

### Other clients

- `clai chat`: `/skills` prints the found skills with their source and
  marks (`*` active, `shadowed by …`), and `/skills NAME` toggles one.
- `clai run --skill NAME` (repeatable) and `clai new --skill NAME`
  start the session with those skills; an unknown name is a usage error
  (exit `2`) that lists the found names.
- `clai skills [--json]` lists what the current workspace would find:
  name, description, path, source, shadowed by, and the warnings. It
  needs no server; it reads the config and the directories itself.
  **(proposal)**

### Protocol

- `skills { workspace }` answers the found skills for that workspace, with
  `name`, `description`, `dir`, `source` (`config`, `project`, `user`),
  `shadowed_by`, and `warnings`. The server finds them, because its
  configuration and home directory are what sessions use.
- `set_skills { session, names }` sets the session's active skills (the
  whole list, so the box and `/skills NAME` share one request). The
  answer and the event `skills_changed { session, active }` carry the
  active list with `changed_on_disk` per skill; every attached client
  updates.
- `status` gains `skills`: active names and their token estimate.
- New requests and fields only; old clients ignore them. As in RFC 0021,
  no version change is needed if old servers answer an unknown request
  with an error the TUI shows as "the server is too old for skills".

### Code

- `src/skills/mod.rs`: `Skill`, `Source`, `find(config, workspace) ->
  Found { skills, warnings }`, the priority and shadowing.
- `src/skills/parse.rs`: frontmatter and body, the checks.
- `src/config`: `[skills]` with `dirs` and `default_dirs`; project file
  check.
- `src/agent/prompt.rs`: `system_prompt` takes the active skills.
- `src/permissions`: the session's workspace gains the skill directories
  as read-only extra directories.
- `src/session`: `skill_on` / `skill_off` events and replay.
- `src/protocol`, `src/server/requests.rs`: the two requests, the event.
- `src/tui/skills.rs`, `src/tui/view/skills.rs`: the box;
  `src/tui/commands.rs`: `/skills`.
- `src/command/skills.rs`, `src/command/chat/`, `src/command/run.rs`.

## Dependencies

None. The frontmatter is a few `key: value` lines; a YAML crate
(`serde_yaml` is no longer maintained, the others are large) is more than
the two keys need. If files in use turn out to need more YAML, this is
the place to revisit.

## Spec changes

- New `spec/skills.md`: format, checks, directories and priority,
  shadowing, what activation does, saving. Linked from `spec/readme.md`
  and `SUMMARY.md`.
- `spec/config.md`: `[skills]` in the files table (user file only), its
  keys, the project file rejection, reloading.
- `spec/agent-loop.md`, "System prompt": the skills section; the
  "Other directories" line includes skill directories.
- `spec/tui.md`: `/skills` in the command table; a "Skills" section with
  the box; the value list after `/skills `; the status box line.
- `spec/command.md`: `clai skills`; `--skill` for `clai run` and
  `clai new`; `/skills` in `clai chat`.
- `spec/sessions.md`, "Persistence": `skill_on`, `skill_off`.
- `spec/architecture.md`: `src/skills/` in the project tree; the
  requests and the event.

## Testing

- `src/skills/` unit tests: frontmatter variants (plain, quoted, `>` and
  `|` blocks, CRLF, missing end line), missing `name` (directory name
  used), bad `name`, missing or long `description`, too large, not UTF-8,
  links to a directory and to `SKILL.md`; priority over all seven default
  directories and two configured ones; shadowing; `default_dirs = false`;
  `XDG_CONFIG_HOME`; missing default dir (no warning) and missing
  configured dir (warning).
- `src/config/tests.rs`: `[skills]` parsing, relative path rejected,
  project file with `[skills]` rejected.
- `src/agent/tests.rs`: prompt without skills unchanged (snapshot), with
  two skills in activation order.
- `src/permissions`: a skill directory is readable, not writable;
  `run_command` rules unchanged.
- `src/session` / `src/server/tests.rs`: `set_skills` writes the events,
  resume restores the text even after the file changed or was removed;
  `changed_on_disk`; `skills_changed` reaches a second client.
- `src/tui/tests.rs`: `Space` ticks, `Enter` sends `set_skills` and the
  notice, `Esc` sends nothing, shadowed rows are skipped, filter, `not
  found any more`, `/skills NAME` and unknown name. Snapshots: the box,
  no skills found, ASCII.
- `tests/skills.rs`: `clai skills --json` in a temporary home and
  workspace; `clai run --skill NAME` with the mock provider puts the text
  into the request's system prompt; unknown `--skill` exits `2`.

## Implementation plan

1. **Finding and reading skills**
   - Status: todo
   - Files: `src/skills/mod.rs`, `src/skills/parse.rs`, `src/lib.rs`,
     `src/config/` (`[skills]`), `src/config/tests.rs`
   - Result: `skills::find` returns the skills of a workspace with
     priority, shadowing and warnings; the config has `[skills]`.
   - Tests: the skills and config unit tests.

2. **`clai skills`**
   - Status: todo
   - Files: `src/cli.rs`, `src/command/skills.rs`, `tests/skills.rs`
   - Result: the user can see what is found and why something is
     skipped, before any session uses it.
   - Tests: `tests/skills.rs` for the listing.

3. **Active skills in the session and the prompt**
   - Status: todo
   - Files: `src/agent/prompt.rs`, `src/agent/system_prompt.md`,
     `src/permissions/`, `src/session/`, `src/protocol/`,
     `src/server/requests.rs`, tests
   - Result: `skills` and `set_skills`; the prompt section; read access to
     skill directories; the log events and resume.
   - Tests: agent, permissions, session and server tests.

4. **The selection box and `/skills`**
   - Status: todo
   - Files: `src/tui/skills.rs`, `src/tui/view/skills.rs`,
     `src/tui/app.rs`, `src/tui/commands.rs`, `src/tui/status.rs`,
     `src/tui/tests.rs`, snapshots
   - Result: `/skills` and `/skills NAME` in the TUI; the status box shows
     active skills.
   - Tests: TUI tests and snapshots.

5. **`clai chat`, `clai run`, `clai new`**
   - Status: todo
   - Files: `src/command/chat/`, `src/command/run.rs`,
     `src/command/new.rs`, `tests/skills.rs`
   - Result: `/skills` in `clai chat`; `--skill NAME`.
   - Tests: `tests/skills.rs` with the mock provider.

6. **Spec and readme**
   - Status: todo
   - Files: the spec files above, `readme.md`, `todo.md`
   - Result: the spec and readme describe skills.
   - Tests: `ci/check.sh`, `ci/test.sh`.

## Alternatives

- **The model loads skills itself** (the way the format is used by Claude
  Code and OpenCode): names and descriptions of all skills in the prompt,
  and a tool that loads one when the task fits. Less work for the user,
  but every session pays for the list, a project skill from an untrusted
  repository could be pulled in without the user choosing it, and the
  fixed tool list of `spec/agent-loop.md` changes. It can come later on
  top of this RFC (see "Open questions").
- **Skill text as a message in the history** instead of the system
  prompt. No prompt change, but turning a skill off cannot remove it, and
  trimming old history (RFC 0008) could drop it.
- **Read `SKILL.md` on every request.** Always fresh, but an edit in
  another window changes a running session's prompt behind its back and
  breaks caching.
- **Configured directories last, defaults first.** Matches "more specific
  wins" less well; the user asked for configured directories to win.
- **A `clai`-only directory** (`~/.config/clai/skills`). One more place
  for the same files; `skills.dirs` covers it for those who want it.
- **Active skills per workspace, remembered across sessions.** Handy for
  a skill used every day in one project, but it adds state outside the
  config files. A `[skills] active = [...]` default could do it later.

## Open questions

1. Priority between the default directories: workspace before home
   (proposal), or home before workspace?
2. Should `~/.config/clai/skills` and `<workspace>/.clai/skills` be
   default directories too, before the other tools' ones?
3. Should a new session start with no skills (proposal), or with a
   configured default list (`[skills] active = [...]`)?
4. Should the model later be able to see all skills and load one itself
   (a `load_skill` tool, asking the user like other calls)?
5. Read access to an active skill's directory (proposal), or nothing
   beyond the `SKILL.md` text?
6. `clai skills` as its own command (proposal), or only in `clai chat`
   and the TUI?
