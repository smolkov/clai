# Tools

Tools are the actions the model may ask `clai` to run: read, list and
search files, change files, run commands. The model sends a tool call with
JSON arguments; `clai` checks the permission (see [`config.md`](config.md),
"Permissions"), runs the tool, and sends the result back to the model.

| Tool | Does | Default permission |
|------|------|--------------------|
| [`read_file`](#read_file) | Read lines of a text file. | `allow` |
| [`list_files`](#list_files) | List files and directories by glob. | `allow` |
| [`search_files`](#search_files) | Search file contents by regex. | `allow` |
| [`edit_file`](#edit_file) | Replace exact text in a file. | `ask` |
| [`write_file`](#write_file) | Create a file, or replace one. | `ask` |
| [`run_command`](#run_command) | Run a shell command in the workspace. | `ask` |

Statements marked **(assumption)** need a decision from the user.

## Common rules

### Calls and results

- Every tool call gets exactly one result: text for the model, and a flag
  whether it is an error (`is_error`, see [`providers.md`](providers.md)).
- A tool never ends the turn or the session. Anything that goes wrong is an
  error result whose message says what was wrong and, where it helps, what
  to do instead, so the model can correct the call.
- Arguments are checked against the tool's schema first. Missing or wrong
  arguments, unknown argument names, and arguments that were not valid JSON
  are an error result that names the problem; the tool does not run.
- An unknown tool name is an error result that lists the tools.
- Several tool calls in one answer run one after another, in the order the
  model sent them (see [`agent-loop.md`](agent-loop.md), "Tool calls").
  **(assumption)**

### Paths

- A path is relative to the workspace, or absolute. `~` is not expanded.
- Before anything is read or written, the path is resolved: `.` and `..`
  are removed and symbolic links are followed (for a file that does not
  exist yet, its nearest existing parent directory is resolved).
- The resolved path must lie inside the workspace or one of the
  `extra_dirs`; otherwise the call is denied without asking, whatever the
  rules say (see [`config.md`](config.md), "Checking order").
- Results show paths relative to the workspace when they are inside it,
  otherwise absolute.
- Permission patterns of the file tools match the resolved path (see
  [`config.md`](config.md), "Path patterns").

### Text files

- Files are read as UTF-8. Bytes that are not valid UTF-8 are shown as `�`;
  such a file can be read and searched, but `edit_file` refuses it, because
  it could not write the other bytes back unchanged. **(assumption)**
- A file with a NUL byte in its first 8 KiB counts as binary. `read_file`
  and `edit_file` refuse it; `search_files` skips it. **(assumption)**
- Written files are UTF-8. `edit_file` keeps all text outside the replaced
  part byte for byte, including line endings and a missing final newline.

### Result size

At most 30 000 characters of a result go to the model. A longer result keeps
its first 20 000 and last 10 000 characters, with a line
`… 12 345 characters cut …` in between. The TUI shows the full result, kept
up to 1 MiB per call. **(assumption)**

### Permissions

Before a tool runs, its call is checked as described in
[`config.md`](config.md), "Permissions". This table says what each tool
checks, and which rule `a` ("always") proposes in the TUI prompt.

| Tool | Checked before it runs | Checked while it runs | `a` proposes, then wider |
|------|------------------------|-----------------------|--------------------------|
| `read_file` | `path` | — | `read_file(PATH)`, `read_file(DIR/**)` |
| `list_files` | `path` | each entry, with the `list_files` rules | `list_files(PATH/**)` |
| `search_files` | `path` | each file, with the `read_file` rules | `search_files(PATH/**)` |
| `edit_file` | `path` | — | `edit_file(PATH)`, `edit_file(DIR/**)`, `edit(DIR/**)` |
| `write_file` | `path` | — | `write_file(PATH)`, `write_file(DIR/**)`, `edit(DIR/**)` |
| `run_command` | each part of the command; redirection targets as `write_file` / `read_file` | — | each part not yet allowed, with all its words; then fewer words, down to the first word, e.g. `run_command(cargo test parse)`, `run_command(cargo test)`, `run_command(cargo)` |

`DIR` is the directory of `PATH`, and wider steps go up one directory at a
time, up to the workspace (`./**`); a file in the workspace itself is
proposed as `./NAME`, so the rule does not match that name in other
directories. A path in an extra directory is proposed as an absolute
pattern, up to that directory. Glob characters in names are put in brackets
(`a[*]b.rs`), so a proposed rule matches only what it names. `a` never
proposes a bare tool rule (`run_command`); the user may type one.

For `run_command`, one proposal is a set of rules added together: one for
each part that asks, and a `write_file` or `read_file` rule for each
redirection that asks. There is no proposal when an `ask` rule decided or a
part cannot be checked (see [`config.md`](config.md), "Commands with
several parts"), because no allow rule would change the answer.
**(assumption)**

Checks while a tool runs keep a listing or search from showing what reading
would not:

- `list_files` leaves out entries a `deny` rule of `list_files` (or the
  `read` group) matches, and does not descend into such directories. Entries
  that would ask are listed.
- `search_files` skips files whose `read_file` check is `deny` or `ask`, and
  does not ask for each of them. The result ends with a line
  `(3 files skipped by permission rules; read them with read_file to be asked)`,
  also when nothing matched. Directories that a `read_file` `deny` rule
  matches are not entered; their files are not counted. A symbolic link
  to a file is checked where it points, including the workspace boundary.
- Entries left out this way are not counted in the 1000-entry and 200-match
  limits.

A call that asks opens a permission question in the session (see below).

### Questions

When a call asks, the tool prepares the `details` of the permission question
(see [`sessions.md`](sessions.md), "Questions") before anything is changed or
run. The details are the same for every client: the TUI, `clai chat`,
`clai run` on a terminal, or a web UI.

Every question about a file tool has, for each path: the path as the model
wrote it, the resolved path (relative to the workspace, or absolute), and
whether it is in the workspace or in an `extra_dirs` entry.

| Tool | Details |
|------|---------|
| `read_file` | Line range to be read; file size and number of lines. |
| `list_files` | Start directory, glob. |
| `search_files` | Regular expression, `ignore_case`, start path, glob. |
| `edit_file` | Unified diff with 3 lines of context, changed line range, number of occurrences replaced; file size and modification time, used to check again before writing. |
| `write_file` | Whether the file exists; for a new file the full content and its line count, for `overwrite` a unified diff against the old content; directories that will be created. |
| `run_command` | The command line exactly as it will run; working directory; `timeout_s`; the environment variables `clai` adds; the parts of the command, each with its own decision (`allow`, `ask`, `deny`) and the rule that decided; redirection targets with their resolved paths. |

For `run_command`, the parts that ask are marked, so the user sees which part
of `cargo build && ./deploy.sh` needs the answer. The proposals for "always"
cover only those parts.

The question is built from the state at the moment it opens. A file that
does not exist or cannot be read gives an error result at once, without a
question; a file that is too large for a diff shows its size and the changed
line range instead. **(assumption)**

### Denied and cancelled calls

| Case | Result for the model (error) |
|------|------------------------------|
| Denied by a `deny` rule | `denied by permission rule: deny run_command(git push --force)`; with the rule's `message`: `denied by permission rule: deny read(.env*): Secrets are not shown …` |
| Denied by a built-in rule (see [`config.md`](config.md), "Built-in rules") | `denied by permission rule: deny read(.env, .env.*): this file usually holds secrets and is not shown to the model. …` |
| Denied by a tool default | `denied: run_command is not allowed` |
| One part of a command denied | `denied by permission rule: deny run_command(rm), for the part "rm -rf target"` |
| Path outside the workspace | `denied: /etc/passwd is outside the workspace` |
| Write into a read-only extra directory | `denied: ~/ws/docs is read-only` |
| Denied by the user (`n`) | `denied by the user` |
| Denied with a message (`e`) | `denied by the user: <message>` |
| `ask` without a client that can answer (`clai run` without a terminal) | `denied: needs permission, and no one can answer` |
| Turn cancelled before or while the call ran | `cancelled by the user` (with the output so far, for `run_command`, `list_files` and `search_files`) |

### Cancellation

`run_command` stops as described below. `list_files` and `search_files`
check for cancellation between files and return what they found so far
with `cancelled by the user`. `read_file`, `edit_file` and `write_file` are
short and finish; their result is still recorded. **(assumption)**

### Display

Each call is one line in the TUI conversation (see [`tui.md`](tui.md),
"Conversation entries"): marker, tool name, main argument, state. The
"Shown" line of each tool below says what the main argument and the expanded
details are.

## read_file

Reads lines of a text file.

```json
{
  "path":   "src/parse.rs",
  "offset": 1,
  "limit":  2000
}
```

| Argument | Required | Meaning |
|----------|----------|---------|
| `path` | yes | File to read. |
| `offset` | no | First line, counted from 1. Default 1. |
| `limit` | no | Most lines to return, 1–2000. Default 2000. |

Result: the lines, each with its number, right-aligned to six places, and an
arrow:

```
     1→use std::fs;
     2→
     3→fn parse(input: &str) -> Vec<Line> {
```

- Lines longer than 2000 characters are cut, with `… (line cut)` at the end.
- If the file has more lines after the returned ones, a last line says
  `… 1 234 more lines; use offset 2001 to read on`.
- An empty file gives `(empty file)`; an offset after the last line gives
  `(file has 120 lines)`.

Errors: file not found (with up to three similar names in the same
directory: the same name ignoring case, or at most two letters different,
added or missing; closest first), path is a directory (use `list_files`),
binary file, permission denied by the operating system.

Shown: path and line range; expanded: the returned lines.

Permission: the path (see [Permissions](#permissions)).

## list_files

Lists files and directories that match a glob.

```json
{
  "path":    "src",
  "pattern": "**/*.rs"
}
```

| Argument | Required | Meaning |
|----------|----------|---------|
| `path` | no | Directory to start in. Default: the workspace. |
| `pattern` | no | Glob, relative to `path`: `*` within one directory, `**` across directories, `?` one character, `{a,b}` alternatives. Default `*`: the entries of `path` itself. |

Result: matching paths, relative to the workspace, one per line, sorted;
directories end with `/`.

- Entries ignored by `.gitignore`, `.ignore` and the global git excludes
  are left out, also outside git repositories. `.git` is never listed.
  Other hidden files are listed. **(assumption)**
- Symbolic links are listed but not followed into directories.
- At most 1000 entries; if there are more, a last line says
  `… 4 321 more entries; use a narrower path or pattern`.
- No match gives `(no files match)`.

Errors: `path` not found or not a directory, invalid glob.

Shown: path and pattern; expanded: the list.

Permission: `path`, then each entry (see [Permissions](#permissions)).

## search_files

Searches the contents of text files for a regular expression.

```json
{
  "pattern":     "fn parse\\(",
  "path":        "src",
  "glob":        "*.rs",
  "ignore_case": false
}
```

| Argument | Required | Meaning |
|----------|----------|---------|
| `pattern` | yes | Regular expression (Rust `regex` syntax), matched per line. |
| `path` | no | File or directory to search. Default: the workspace. |
| `glob` | no | Only files whose name or path below `path` matches this glob. |
| `ignore_case` | no | Case-insensitive search. Default `false`. |

Result: one line per matching line, `path:line: text`, files in sorted
order:

```
src/parse.rs:3: fn parse(input: &str) -> Vec<Line> {
src/main.rs:17:     let lines = parse(&text);
```

- The same files are searched as `list_files` would list: `.gitignore` is
  respected, `.git` is skipped. Binary files are skipped.
- Matching lines are cut to 300 characters around the match.
- At most 200 matches; if there are more, a last line says
  `… more matches in 12 files; use a narrower pattern, path or glob`.
- No match gives `(no matches)`.

Errors: invalid regular expression (with the parser's message), `path` not
found, invalid glob.

Shown: pattern and path; expanded: the matches.

Permission: `path`, then each file with the `read_file` rules (see
[Permissions](#permissions)).

## edit_file

Replaces exact text in an existing file.

```json
{
  "path":        "src/parse.rs",
  "old_text":    "for i in 0..lines.len() - 1 {",
  "new_text":    "for i in 0..lines.len() {",
  "replace_all": false
}
```

| Argument | Required | Meaning |
|----------|----------|---------|
| `path` | yes | File to change. It must exist. |
| `old_text` | yes | Text to replace, exactly as in the file, including indentation. Not empty. |
| `new_text` | yes | Replacement; may be empty to delete `old_text`. Must differ from `old_text`. |
| `replace_all` | no | Replace every occurrence. Default `false`: `old_text` must occur exactly once. |

- Matching is exact: no trimming, no case or whitespace folding. Line
  numbers from `read_file` (`  12→`) are not part of the text.
- In a file with CRLF line endings, `\n` in `old_text` and `new_text`
  stands for `\r\n`, so the model does not need to know the line endings.
  **(assumption)**
- The file does not need to be read first; the exact match already fails
  if the file changed in that place. **(assumption)**
- The permission question shows the change as a unified diff. The file is
  checked again just before writing; if it changed since the question was
  opened, the call fails with "file changed while waiting for permission".
- The file is written to a temporary file in the same directory and then
  renamed over the original, so it is never left half written. Its
  permissions are kept.

Result: `edited src/parse.rs: replaced 1 occurrence (line 12)`, or with
`replace_all`, `replaced 3 occurrences (lines 12, 40, 41)`.

Errors:

| Case | Message |
|------|---------|
| `old_text` not found | `old_text not found in src/parse.rs`; if a match that differs only in whitespace exists, it is quoted with its line number: the first place where the same number of lines, with runs of whitespace taken as one space, equals `old_text` (or, for one line, contains it). |
| Found more than once, without `replace_all` | `old_text occurs 3 times (lines 12, 40, 41); add surrounding text to make it unique, or set replace_all` |
| `old_text` empty, or equal to `new_text` | says so; to create a file, use `write_file` |
| File not found, binary, or not valid UTF-8 | says so |

Shown: path and changed line range; expanded: the diff.

Permission: the path (see [Permissions](#permissions)).

## write_file

Creates a file, or replaces one completely.

```json
{
  "path":      "tests/parse_single_line.rs",
  "content":   "#[test]\nfn single_line() { … }\n",
  "overwrite": false
}
```

| Argument | Required | Meaning |
|----------|----------|---------|
| `path` | yes | File to write. |
| `content` | yes | The whole new content. |
| `overwrite` | no | Allow replacing an existing file. Default `false`. **(assumption)** |

- Missing parent directories are created.
- Without `overwrite`, an existing file is an error: `src/a.rs exists; use
  edit_file to change it, or set overwrite`.
- The permission question shows the new content, or for `overwrite` a unified
  diff against the old content.
- Written like `edit_file`: temporary file, then rename; the permissions of
  a replaced file are kept, a new file gets the default permissions.

Result: `created tests/parse_single_line.rs (12 lines)` or
`replaced src/a.rs (40 lines, was 38)`.

Errors: path is a directory, existing file without `overwrite`, a parent
path is a file (`src is a file, not a directory`), permission denied by the
operating system.

Shown: path and line count; expanded: content or diff.

Permission: the path (see [Permissions](#permissions)).

## run_command

Runs a shell command in the workspace and returns its output.

```json
{
  "command":   "cargo test parse",
  "timeout_s": 120
}
```

| Argument | Required | Meaning |
|----------|----------|---------|
| `command` | yes | Command line, run with `sh -c`. |
| `timeout_s` | no | Seconds before the command is stopped, 1–600. Default 120. **(assumption)** |

- The command runs in the workspace directory. Each call starts there again;
  a `cd` does not carry over to the next call.
- Standard input is empty (`/dev/null`); a command that waits for input
  gets end of file instead of hanging.
- The environment is the server's, plus `PAGER=cat`, `GIT_PAGER=cat`,
  `GIT_TERMINAL_PROMPT=0` and `TERM=dumb`, so no pager or password prompt
  waits for input. **(assumption)**
- The command runs in its own process group. On timeout or when the turn is
  cancelled, the group gets `SIGTERM`, and `SIGKILL` 2 s later (see
  [`tui.md`](tui.md), "Cancellation and exit"). Background processes the
  command started are stopped with it.
- When the shell exits, processes it left in the background get 0.5 s to
  finish writing output, then the whole group is stopped. So
  `sleep 30 & echo hi` returns at once; a server started with `&` does not
  outlive the call. **(assumption)**
- Standard output and standard error are collected together, in the order
  they were written.
- The permission question shows the command exactly as it will run, with the
  working directory. How commands with `;`, `&&`, pipes and the like are
  matched against rules: [`config.md`](config.md), "Commands with several parts".

Result: the output, then a last line `exit code 0`. Output cut by the size
limit keeps its start and end (see [Result size](#result-size)), which
keeps both the first error and the final summary of a build.

- A non-zero exit code makes the result an error, so the TUI marks it `✗`;
  the output is the same.
- A timeout gives the output so far and `stopped after 120 s (timeout)`,
  as an error.
- A command ended by a signal gives `killed by signal 9`, as an error.
- A command that cannot be started (no `sh`) is an error.

Shown: the command; expanded: command, working directory and output.

Permission: each part of the command line, and the targets of its
redirections (see [Permissions](#permissions) and [`config.md`](config.md),
"Commands with several parts").

## Open questions

1. Should `read_file` also show images (screenshots) to models that accept
   them? See [`providers.md`](providers.md), open question 2.
2. Should `run_command` keep a shell across calls, so `cd` and exported
   variables carry over?
3. Should commands run in a sandbox (no network, writes only inside the
   workspace)? See open question 5 in [`readme.md`](readme.md).
4. Is a tool for fetching web pages wanted?
5. Should rules be able to look at more than the path or command, e.g.
   allow `write_file` only for new files (`overwrite = false`), or limit
   `timeout_s`?
