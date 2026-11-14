# Skills

A skill is a directory with a `SKILL.md` file: a name, a description, and
instructions for the model, maybe with scripts and reference files next to
it. The format is the one OpenCode, Claude Code and other agents use, so
skills a user has for them work in `clai` too. The user turns skills on for
a session; an active skill's instructions go into the system prompt, and its
directory becomes readable for the file tools. Decided in RFC 0023.

The model does not choose skills itself: nothing is added to the prompt
unless the user turns it on.

## Format

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

- A skill is a directory below a skills directory that holds a file
  `SKILL.md`, at any depth up to 6 levels: `synced/<id>/pdf/SKILL.md` is a
  skill too, as other agents' sync tools lay them out. Directories are
  searched depth first, in the order of their names. Below a skill nothing
  more is searched, so its own files (`scripts/`, examples) are never
  skills. Directories whose name starts with `.` are passed over. Links are
  followed; a directory reached twice, through a link loop or two links, is
  searched once.
- The file starts with a frontmatter block between two `---` lines. `name`
  and `description` are read from it; other keys (`allowed-tools`, `model`,
  `license`, …), nested tables and lists are read and ignored.
- `name`: 1 to 64 characters, lowercase letters, digits and `-`, not
  starting or ending with `-`. Without `name`, the directory name is used. A
  `name` that differs from the directory name is used, with a warning.
- `description`: required, at most 1 024 characters. Runs of whitespace,
  newlines included, become one space.
- The body, everything after the frontmatter, is the instruction text,
  without blank lines at the start and whitespace at the end.
- Values may be plain (a ` #` starts a comment; indented lines that follow
  continue the value), `'single'` or `"double"` quoted, or a `>` / `|`
  block on the indented lines that follow. A UTF-8 byte order mark and CRLF
  line ends are accepted. No YAML library is used.
- Links are followed for the skill directory and for `SKILL.md`.

A file is skipped with a warning naming the file, and the line where it
helps, when:

- it is larger than 64 KiB, or not UTF-8;
- it does not start with `---`, or the frontmatter has no closing `---`;
- a frontmatter line at the start of a line is not `key: value` (list items
  `- …` are passed over), or a quoted value does not end;
- it has no description, or one that is too long;
- its name breaks the rule above.

A skipped file is not an error: the other skills still load. Warnings show
in `clai skills`, `/skills` of `clai chat`, and the `/skills` box.

## Where skills are found

Directories, highest priority first:

| Order | Directory | Source |
|-------|-----------|--------|
| 1 | each entry of `skills.dirs`, in the order written | `config` |
| 2 | `<workspace>/.opencode/skills` | `project` |
| 3 | `<workspace>/.claude/skills` | `project` |
| 4 | `<workspace>/.agents/skills` | `project` |
| 5 | `~/.config/opencode/skills` | `user` |
| 6 | `~/.claude/skills` | `user` |
| 7 | `~/.agents/skills` | `user` |

- Rows 2 to 7 are the default directories; `skills.default_dirs = false`
  leaves them out ([`config.md`](config.md), "Skills").
- `~/.config` is `$XDG_CONFIG_HOME` when it is set to an absolute path.
  That applies to `opencode` only; `~/.claude` and `~/.agents` are always in
  the home directory.
- The workspace directories are those of the workspace itself; parent
  directories are not searched.
- Within a directory, skills are in the order of their directory names.
- When two skills have the same name, the one in the directory higher in
  the table wins. The other is **shadowed**: listed, with the path of the
  winner, but it cannot be turned on.
- A default directory that does not exist is passed over without a
  warning; a configured one that does not exist gets a warning, because the
  user named it. A directory that cannot be read gets a warning.
- The server finds the skills for a session, with its configuration and
  home directory; `clai skills` finds them itself, the same way.

## Active skills

- The active skills belong to the session, like its model: other sessions
  are not affected, and a new session starts with none (unless
  `--skill` is given, [`command.md`](command.md)).
- A change names the whole new list. Skills that stay keep their place;
  new ones follow, in the order named. Turning on a skill reads its
  `SKILL.md` and keeps the text with the session.
- Later changes of the file are not seen by the session until the skill is
  turned off and on again. The `/skills` box and the status show such a
  skill as **changed on disk**. The prompt stays the same during the
  session, so provider caching keeps working.
- A skill that is no longer found where it was turned on (deleted, its
  directory left the configuration, or another directory now has a skill
  of that name) keeps working from its saved text and is shown as **not
  found any more**. It can be turned off; it cannot be turned on again
  until it is found.
- A change applies from the next model request; a request that runs
  finishes with the old prompt.

### System prompt

Without active skills, the prompt is unchanged. With them, a section
follows the fixed text ([`agent-loop.md`](agent-loop.md), "System prompt"),
one block per skill in the order of activation:

```
The user turned on these skills. Follow their instructions when the task fits them.
Files the instructions name are relative to the skill's directory.

<skill name="release-check" dir="/home/user/ws/team-skills/release-check">
# Release check
…
</skill>
```

`dir` is the directory as found, links not resolved. The skill text is
trusted as an instruction of the user because the user chose it; files
read with tools, also those in a skill directory, stay data.

### Read access

Each active skill's directory is added to the session's directories as
**read only**, like an `extra_dirs` entry with `access = "read"`:
`read_file`, `list_files` and `search_files` may use it, writing there is
denied, and the "Other directories" line of the prompt names it. A
directory inside the workspace or inside an extra directory is not added
again: the access it has there holds. A directory that does not exist is
not added. `run_command` is not changed: a script of a skill runs only if
the rules allow the command.

### Size

The text counts towards the context like the rest of the system prompt. The
status shows each active skill with an estimate of its tokens: its bytes
divided by four, rounded up.

## Saving

A change is the event `skills_changed` in the session log
([`sessions.md`](sessions.md), "Persistence"), with the skills turned on and
their text, and the names turned off. A resumed session replays these
events and continues with the prompt it had, from the saved text; the
directory of a skill that is gone is no longer readable. `clai export` and
`clai import` carry the events; a log of an older `clai` has none, so no
active skills.

## Clients

- TUI: `/skills` opens the selection box, `/skills NAME` turns one skill on
  or off, and after `/skills ` the value list offers the found skills
  ([`tui.md`](tui.md), "Skills"). A change shows the notice
  `skills: + release-check, − pdf`.
- `clai chat`: `/skills` prints the skills, `/skills NAME` turns one on or
  off ([`command.md`](command.md), "clai chat").
- `clai run --skill NAME` and `clai new --skill NAME` start with those
  skills; `clai skills [--json]` lists what the current workspace finds
  ([`command.md`](command.md)).
