# Second security review: threat model, terminal output, imports, child environment

- RFC: 0035
- Status: done
- Milestone: none; it closes gaps that a second review of the code found
- Spec: [`spec/architecture.md`](../spec/architecture.md), "Trust boundaries";
  [`spec/config.md`](../spec/config.md), "Command patterns", "Built-in
  rules", "Project files", "Provider keys";
  [`spec/tools.md`](../spec/tools.md), "run_command";
  [`spec/sessions.md`](../spec/sessions.md), "Persistence";
  [`spec/command.md`](../spec/command.md), "codeagent import",
  "codeagent chat"; [`spec/readme.md`](../spec/readme.md), open question 5
  (sandbox); new: `spec/security.md`

## Summary

RFC 0010 checked the permission engine, the server socket and the saved
sessions. RFC 0030 keeps secrets out of what the model sees. Both hold up.
This review looked at the paths that lead around them: what reaches the
user's terminal, what an imported session file can carry, what child
processes inherit, where HTTP redirects can send a key, and which file
writes or command options run code later, outside the permission check.

The RFC makes eight changes and adds a spec file:

1. A new `spec/security.md`. It sets out the assets, the attackers, the
   entry points and the guard for each, and records what remains open. Every
   later feature adds its row there.
2. Plain-terminal output (`codeagent run`, `chat`, `attach` without the
   TUI, `send`) removes escape sequences from model and tool text.
   Permission questions show hidden characters visibly, in every client.
3. `codeagent import` drops the session `allow` rules a file carries and
   says which ones it dropped.
4. `run_command` and language servers no longer inherit the provider keys
   of `codeagent`.
5. Provider requests do not follow redirects.
6. Built-in `deny` rules protect `.git/` from writes by file tools and
   redirections.
7. Built-in `ask` rules cover command options that run other programs:
   `find -exec`, `git -c`, `rg --pre` and others.
8. A warning appears when a user file that holds `api_key` can be read by
   others.

An optional sandbox for commands, open question 5 of the spec, is designed
here as the last step. It is off by default.

## Motivation

The guards in `spec/architecture.md`, "Trust boundaries", cover what the
model may *do* through tools and what *leaves* through the provider. Some
ways around them are still open:

- **The terminal is an output channel too.** In `codeagent chat`, model
  text and permission questions are written to the terminal as they are.
  A prompt injection in a file can make the model print
  `ESC ] 52 ; c ; <base64> BEL`, which many terminals accept as "write this
  to the clipboard". It can also print `\r` and `ESC [ 2K`, which erase a
  line already on the screen, so a question about `curl x | sh` can be
  made to look like a question about `cargo test`. The TUI already drops
  control characters (`src/tui/wrap.rs`, `wrap`). The plain output does
  not (`src/text/conversation.rs`, `SessionEvent::TextDelta`;
  `src/text/question.rs`, `render_question_styled`).
- **An imported session can grant permissions.** A bundle's `history`
  holds `Record::Rules`. On resume they become session `allow` rules
  (`src/agent/mod.rs`, the `Record::Rules` arm of the history load). A
  file someone else exported, "to move it to another machine", can carry
  `allow run_command(*)`. After the import, the next turn runs any command
  without a question.
- **Allowed commands run the repository's code, with the keys.** The
  child environment of `run_command` is the server's
  (`src/tools/run_command.rs`, `execute`), so it includes
  `ANTHROPIC_API_KEY` and the variables of the other providers. An allowed
  `cargo test` or `npm test`, or `rust-analyzer` running `build.rs`, runs
  code from the repository. That code can read the key and send it
  anywhere. RFC 0030 hides the key in *output*. That does not stop a
  process that has the key from sending it somewhere else.
- **Redirects can carry the key and the conversation away.**
  `reqwest`'s default policy follows up to 10 redirects. On a cross-host
  redirect it drops `Authorization` and cookies, but not `x-api-key`
  (Anthropic) or `x-goog-api-key` (Gemini, RFC 0032). A `307` or `308`
  sends the body again, and the body is the whole conversation. A redirect
  from `https` to `http` is followed too (`src/provider/http.rs`,
  `Http::new`).
- **Writes that run code later.** Editing asks by default. But "always"
  widens up to `edit(./**)` (`spec/tools.md`, "Permissions"), and that
  rule also covers `.git/hooks/pre-commit` and `.git/config`. If
  `.git/config` gains `core.fsmonitor = "curl … | sh"`, the next
  `git status` runs it, and `git status` is usually allowed. The
  permission engine never sees that command.
- **Command options that run commands.** `spec/config.md` tells users to
  write `ask run_command(find ** -exec)` themselves. Few do. The "always"
  choice for `find . -name '*.rs'` proposes widening to `run_command(find)`,
  and that rule then allows `find . -exec sh -c '…' \;` without a question.
  The same goes for `git -c core.sshCommand=…`, `rg --pre`,
  `tar --to-command` and `rsync -e`.
- **The spec has no threat model in one place.** The knowledge is spread
  over "Trust boundaries", "Project files", RFC 0010 and RFC 0030. A new
  feature has no checklist to go through. For example, RFC 0034 had to work
  out for itself what an image name in a bundle may be.

### Findings

Each finding names its place, so it can be checked. The severity assumes
the default configuration and an attacker who controls text the model reads
(a file, a web page in a file, command output) or a file the user is given.

| # | Severity | Finding | Place |
|---|----------|---------|-------|
| 1 | high | Plain output writes escape sequences from model text, tool lines and questions to the terminal: it can write to the clipboard (OSC 52), change the title, plant hyperlinks (OSC 8), and overwrite what a permission question shows. | `src/text/conversation.rs`, `event`; `src/text/question.rs` |
| 2 | high | `codeagent import` keeps the session `allow` rules in the file; resuming the session applies them. | `src/session/store.rs`, `Bundle`; `src/agent/mod.rs`, history load |
| 3 | medium | `run_command` and language servers inherit the provider keys; code from the repository that an allowed command runs can read them. | `src/tools/run_command.rs`, `execute`; `src/lsp/server.rs` |
| 4 | medium | Provider requests follow redirects: `x-api-key` and the body go to another host, also over `http`. | `src/provider/http.rs`, `Http::new` |
| 5 | medium | No built-in protection for `.git/`: a wide edit rule allows writing hooks and `core.fsmonitor`, which `git` runs later without a question. | `src/config/permissions.rs`, built-in rules |
| 6 | medium | An `allow` rule for `find`, `git`, `rg`, `tar` or `rsync` also allows the options that run any command; "always" proposes exactly such rules. | `src/permissions/check.rs`; `spec/config.md`, "Command patterns" |
| 7 | low | Bidirectional and zero-width characters (U+202A–U+202E, U+2066–U+2069, U+200B–U+200F, U+FEFF) are not control characters, so the TUI shows them. In a permission question they can reorder or hide part of a command or path. | `src/tui/wrap.rs`; `src/text/question.rs` |
| 8 | low | `api_key` in a user file that others can read gives no warning; the spec only advises against it. | `src/config/file.rs`; `spec/config.md`, "Provider keys" |
| 9 | open | Commands are not limited in what they read, write or send once they run (open question 5). | `spec/readme.md` |

Checked and sound, kept as they are:

| Area | Why it holds | Place |
|------|--------------|-------|
| Socket | Directory created `0700`, refused if it belongs to another user or is a link, mode narrowed on every start; no network listener. | `src/server/paths.rs`, `prepare` |
| Scratch directory | Below the runtime directory (`0700`), not in a shared `/tmp`; names from session keys, not from text. | `src/scratch.rs` |
| Project file | Cannot define providers, `[skills]`, `[session]`, `allow` rules, `extra_dirs` or `builtin_except`; may only turn LSP off. | `src/config/file.rs` |
| Language servers | Start only after a question; `workspace/applyEdit` never applied; locations filtered with read rules; size limits on messages. | `src/lsp/` |
| Images in bundles | Names must be `<sha256>.<ext>` and match their bytes; so a name never becomes a path outside `images/`. | `src/session/store.rs` |
| Protocol | Lines at most 16 MiB; answers checked by the server against the open question. | `src/protocol/mod.rs`, `src/server/requests.rs` |
| TUI rendering | Control characters dropped before drawing. | `src/tui/wrap.rs` |
| Never crash | `unsafe` forbidden; `unwrap`, `expect`, `panic`, indexing denied by lints. | `Cargo.toml`, `[lints]` |
| Skills | Only active after the user turns them on; changes on disk do not reach a running prompt. | `spec/skills.md` |

## Scope

In:

- `spec/security.md`, the threat model, with a findings table for this
  review and RFC 0010.
- Findings 1 to 8, as designed below.
- The sandbox (finding 9) as an opt-in for `run_command` on Linux, using
  `bwrap` when it is on `PATH`.

Out:

- A sandbox for language servers. They need the build tools of the
  project and its caches; a later RFC can reuse the sandbox step.
- A sandbox on macOS. `sandbox-exec` is deprecated, and nothing else is
  available without a new crate or `unsafe`.
- Landlock or seccomp directly. That needs system calls, so it needs either
  `unsafe` (forbidden) or a new crate. `bwrap` already uses them, and it is
  in the package repositories.
- Protection against a user who allows `run_command(*)` or uses `--yes`
  in an untrusted repository. The spec already says what those mean;
  `spec/security.md` says it again in one place.
- Encrypting the saved sessions (RFC 0010, finding 5, still accepted).

## Design

### The threat model in the spec

A new file, `spec/security.md`, listed in `spec/readme.md` and
`spec/SUMMARY.md`:

- **Assets:** provider keys; other secrets on the machine (files,
  variables); the user's code and the conversation; the integrity of the
  workspace and of the user's tools (git, shell, editor); the user's
  terminal and clipboard; API spend.
- **Attackers:**
  - A **repository** the user opens: its files, its project file, its
    skills, its build scripts and the language server's view of it.
  - **Text the model reads:** files, command output, answers of language
    servers, web content inside files. This is prompt injection; its only
    lever is what the model asks for.
  - **A file the user is given:** an exported session, a theme file, an
    image.
  - **The network** between `codeagent` and the provider, or a provider
    that redirects.
  - **Other local users** on the same machine.
- **Not in the model:** the provider itself, which sees what it is sent
  by design; root on the machine; a user who allows everything.
- **Entry points and guards:** one table, the "Trust boundaries" of
  `architecture.md` moved here and extended with rows for imported
  sessions, child processes, the terminal and HTTP redirects.
  `architecture.md` keeps a short paragraph and a link.
- **Accepted risks:** what is known and not fixed, with the reason. For
  example: secrets stay in saved sessions; commands may read anything when
  the sandbox is off; `--yes` answers every `ask`.
- **Checklist for a new feature:** which input it takes from which
  attacker, what it writes and where, which child processes it starts and
  with which environment, what it prints to a terminal, and which limit
  bounds it. An RFC that adds an entry point adds its row in the same
  change.
- **Findings:** the table of this RFC and that of RFC 0010, kept up to date.

### Safe terminal text

One function, `text::safe::terminal(&str) -> Cow<str>`, used for every
piece of text from outside `codeagent` before it is written to a terminal
by the plain clients:

- It removes `ESC` and the sequence that follows it (CSI, OSC up to `BEL`
  or `ESC \`, DCS, the two-byte forms), and the C1 controls U+0080–U+009F.
- It removes the C0 controls other than `\n` and `\t`. A lone `\r`
  becomes nothing, as in `tools::clean`.
- It removes the bidirectional controls U+202A–U+202E and U+2066–U+2069.

`codeagent` keeps its own colours. They are added after `terminal` runs,
by `text::ansi::paint`, so they still work.

Permission questions, and the details of an `ask_user` question, show
hidden characters instead of removing them. The user has to see that the
command contains something. `text::safe::visible(&str)` turns each
character from the sets above, and U+200B–U+200F and U+FEFF, into a
visible form: `␛` for `ESC`, `⟨U+202E⟩` for the others, or `\e` and
`<U+202E>` when the terminal does not show Unicode. If it changed anything,
the question gets a line: `note: the command contains 3 hidden characters,
shown as ⟨…⟩`. The TUI uses the same function for questions. Elsewhere it
keeps dropping control characters as it does today.

**(proposal)** Model text keeps zero-width characters outside questions.
They matter in some scripts and code, and outside a question they cannot
change a decision.

### Imports carry no permissions

`Store::import` drops every `Record::Rules` from the bundle's history
before it writes the session. `codeagent import` prints one line per rule
it dropped:

```
codeagent: imported as fix-parser-2
codeagent: left out the session rule allow run_command(cargo test) (rules are not imported)
```

The workspace in the bundle stays. It is where the session runs, and
resuming one whose workspace does not exist already fails with a message.
The workspace's own project file and the user's file decide the
permissions again.

**(proposal)** No `--keep-rules` option. A user who wants the rules back
can grant them again with "always". It is one click each, and the user
sees each rule as it is granted.

### No provider keys in child processes

When `run_command` starts a command, and when a language server starts,
these variables are removed from the child's environment:

- the `api_key_env` of every configured provider;
- the provider variables known to RFC 0030 (`ANTHROPIC_API_KEY`,
  `OPENAI_API_KEY`, `GEMINI_API_KEY`, `GOOGLE_API_KEY`, `OPENROUTER_API_KEY`
  and the rest of its list of provider keys).

Other secret variables (`GITHUB_TOKEN`, `AWS_*`, `[secrets] values_env`)
stay. Real tasks need them (`gh pr view`, `aws s3 ls`), and RFC 0030 hides
their values in the output.

A user who builds an application against a provider's API, and whose tests
need its key, keeps the key with a user-file key:

```toml
[run_command]
keep_env = ["OPENAI_API_KEY"]
```

The `[run_command]` table is new. It is allowed in the user file only; a
project file with it is rejected, like `[skills]`. `keep_env` holds names
of variables, and a name that is not a valid variable name is a load error.
The startup context (RFC 0026) already holds the environment, so the
filtered map is built once per configuration and passed to the tool.

### No redirects to providers

`Http::new` sets `redirect(reqwest::redirect::Policy::none())`. A `3xx`
answer becomes an error of kind `InvalidRequest`, not retried:
`the provider answered with a redirect to https://other.example/…; set
base_url to the final address`. The error names the target's scheme and
host. It never names the query string, which can hold a token.

### Built-in protection for `.git`

A built-in `deny` rule for the `edit` group joins the built-in rules of
RFC 0010:

| Rule | Patterns | Except |
|------|----------|--------|
| git directory | `.git/**`, `**/.git/**` | |

It covers `edit_file`, `write_file` and redirections (`echo x >
.git/hooks/pre-commit`), in the workspace and in `extra_dirs`. Reading
`.git/` stays allowed. The model may still run `git` commands, which change
`.git/` the way `git` means them to, behind their own rules. The way out is
`builtin_except`, as for the read rules, e.g. `builtin_except =
[".git/info/exclude"]`.

The refusal the model sees: `denied by permission rule: deny
edit(.git/**): files in .git/ change how git runs; use git commands
instead, or ask the user.`

**(proposal)** `.envrc` (direnv runs it on `cd`) and editor task files
(`.vscode/tasks.json`) are not protected. Changing them is normal project
work, and they run only after the user's own action; see open questions.

### Built-in asks for options that run commands

Built-in `ask` rules, checked in step 3 of "Checking order" with the
user's `ask` rules. An `allow` rule cannot pass them; `--yes` answers them
like any `ask`. "Always" is not offered for them, as for the argument check
of RFC 0010:

| Command | Patterns |
|---------|----------|
| `find` | `find ** -exec`, `find ** -execdir`, `find ** -ok`, `find ** -okdir` |
| `git` | `git ** -c`, `git ** --config-env*`, `git config`, `git ** --upload-pack*`, `git ** --receive-pack*`, `git ** --exec*`, `git ** -x` (with `rebase`) |
| `rg` | `rg ** --pre*` |
| `tar` | `tar ** --to-command*`, `tar ** --use-compress-program*`, `tar ** -I`, `tar ** --checkpoint-action*` |
| `rsync` | `rsync ** -e`, `rsync ** --rsh*` |
| `ssh` | `ssh ** -o`, `ssh ** -F` |

The reason in the question: `asks: built-in rule ask run_command(find **
-exec): this option runs another command`. A user who wants one of them
allowed says so with `builtin_except` (rules are matched by their pattern
text there, e.g. `builtin_except = ["git config"]`).

Known limits, pinned by tests so that a change is noticed:

- Joined short options are not caught (`tar -xIzstd`). Only the plain
  word is matched.
- `awk 'BEGIN{system("…")}'`, `sed 's/x/y/e'`, `make`, `npm run` and every
  interpreter run code by their nature. A user who allows them allows
  that. `spec/security.md` says so.

**(proposal)** `git ** -x` is matched only after `rebase`
(`git rebase ** -x`), because `-x` means other things for other
subcommands.

### Warning for a readable key

When the user file sets `api_key` and its mode allows others to read it
(`mode & 0o077 != 0`), loading it adds a warning:
`config.toml: api_key is readable by other users (mode 0644); run chmod
600 ~/.config/codeagent/config.toml`. `codeagent run` prints it on standard
error, and the TUI shows it as a notice once per start. It is not an error,
so a setup that works keeps working. Saving the file through `/settings`
already keeps its mode. It does not narrow the mode silently, because the
file is the user's.

### Sandbox for commands (opt-in)

```toml
[sandbox]
enabled = false        # true: run_command runs inside bwrap
network = true         # false: no network inside the sandbox
hide = ["~/.ssh", "~/.aws", "~/.gnupg", "~/.config/gh"]   # added to the built-in list
```

User file only. When `enabled = true`, and on Linux with `bwrap` on
`PATH`, a command runs as:

```
bwrap --die-with-parent --new-session --unshare-pid [--unshare-net]
      --ro-bind / /  --dev /dev  --proc /proc
      --bind <workspace> <workspace>  --bind <scratch> <scratch>
      [--bind <extra_dir> …  or --ro-bind for access = "read"]
      --tmpfs <each hidden dir>
      -- sh -c <command>
```

- The whole file system can be read, and only the workspace, the scratch
  directory and the `write` extra directories can be written. So the
  write boundary of the file tools now also holds for commands.
- Hidden directories are empty inside the sandbox. The built-in list is
  `~/.config/codeagent`, `~/.local/state/codeagent`, the runtime directory,
  `~/.ssh`, `~/.gnupg`, `~/.aws`, `~/.netrc`; `hide` adds more.
- `network = false` adds `--unshare-net`.
- `--new-session` stops the `TIOCSTI` terminal injection; the process group
  handling of `run_command` is unchanged (`bwrap` is the group leader).
- If `bwrap` is missing or fails to start (for example, user namespaces are
  turned off), the call is an error: `sandbox: bwrap not found; install
  bubblewrap or set sandbox.enabled = false`. It never runs the command
  without the sandbox.
- The question shows `in sandbox` (or `in sandbox, no network`) after the
  working directory, so the user knows what an "allow" means.
- On other systems `enabled = true` is a load error naming the platform.

The permission rules stay as they are. The sandbox limits what an allowed
command can do; it does not replace the question.

## Dependencies

None. `bwrap` is an external program, used only when the user turns the
sandbox on, the way `wl-paste` and `xclip` are used for the clipboard.

## Spec changes

- `spec/security.md`: new, as in "The threat model in the spec".
- `spec/readme.md`, `spec/SUMMARY.md`: list it. Open question 5
  (sandbox) is answered: opt-in, `bwrap`, Linux.
- `spec/architecture.md`, "Trust boundaries": moves to `security.md`,
  with a short paragraph and a link left in its place.
- `spec/config.md`:
  - "Built-in rules": the `.git` rule and the command-option asks;
    `builtin_except` also names command patterns.
  - "Command patterns": the built-in asks replace the advice to write
    `find ** -exec` by hand.
  - "Files": `[run_command]` and `[sandbox]`, user file only.
  - New sections "run_command" (`keep_env`) and "Sandbox".
  - "Provider keys": the mode warning.
- `spec/tools.md`, "run_command": the environment without provider keys;
  the sandbox; "Permissions": no "always" for built-in asks.
- `spec/providers.md`: redirects are not followed.
- `spec/sessions.md`, "Persistence", and `spec/command.md`, "codeagent
  import": rules are not imported.
- `spec/command.md`, "codeagent chat", "codeagent run": output is made
  safe for the terminal; questions show hidden characters.
- `spec/tui.md`: questions show hidden characters.
- `readme.md`: "Permissions" names the new built-in rules; a "Security"
  section links to `spec/security.md` and explains the sandbox.
- `todo.md`, `CHANGELOG.md`.

## Testing

- `src/text/safe.rs` (new), unit tests:
  - `terminal` removes CSI, OSC 52 ending in `BEL` and in `ESC \`, OSC 8,
    DCS, C1, `\r` and bidirectional controls. It keeps `\n`, `\t`, emoji,
    CJK and combining characters. Text without anything to remove comes
    back borrowed.
  - `visible` marks each set, in Unicode and in ASCII form, and counts
    them.
- `src/text/tests.rs`: a `TextDelta` with OSC 52 prints without it; a
  permission question for a command with `\r\x1b[2K` shows `␛` and the
  note; colours of `codeagent` still appear when colour is on.
- `src/tui/tests.rs`: a snapshot of a permission question with U+202E in
  the command.
- `src/session/store.rs`, `src/command/import.rs`: a bundle with
  `Record::Rules` imports without them, prints the dropped rules, and the
  resumed session asks for the command the rule allowed.
- `src/tools/tests.rs`:
  - `run_command "printenv ANTHROPIC_API_KEY"`, with the variable set and
    named by a provider, prints nothing and exits `1`.
  - With `keep_env`, it prints the value; RFC 0030 hides it in the result.
  - `GITHUB_TOKEN` is still passed.
- `src/lsp/tests.rs`: the fake server's environment has no provider key.
- `src/provider/tests_http.rs`: a `307` to another local port is an error
  naming the host and leaving out the query; the second server gets no
  request.
- `src/permissions/tests.rs`:
  - `write_file .git/hooks/pre-commit`, `edit_file sub/.git/config` and
    `echo x > .git/config` are denied with source `built-in`.
    `read_file .git/config` is allowed. `builtin_except` lifts the rule.
  - With `allow run_command(find)`, `find . -name x` is allowed and
    `find . -exec rm {} \;` asks, without "always". The same holds for
    each row of the table. `tar -xIzstd` is allowed (known limit, pinned).
  - `builtin_except = ["git config"]` lifts that ask only.
- `src/config/tests.rs`: `[run_command]` and `[sandbox]` in a project file
  are load errors; a bad `keep_env` name is a load error; `api_key` in a
  `0644` file gives the warning, and in a `0600` file it does not.
- `src/tools/tests.rs`, sandbox (skipped when `bwrap` is not on `PATH` or
  cannot create a namespace, and the skip says why):
  - With the sandbox on, `touch ~/x` fails and `touch x` works.
  - `ls ~/.ssh` is empty.
  - With `network = false`, `getent hosts localhost` still works, while a
    connection to a test server on the host's loopback fails.
  - A missing `bwrap` (`PATH` without it) gives the error and runs nothing.
- `ci/check.sh` gains a check that no code outside `src/text/safe.rs` and
  the TUI writes event text to standard output without going through
  `safe::terminal`. It is a `grep` for `print!` in `src/command/` and
  `src/text/`, with an allow list.

## Implementation plan

Each step leaves a working, tested state. The order puts the high findings
first.

1. **Threat model**
   - Status: done
   - Files: `spec/security.md`, `spec/readme.md`, `spec/SUMMARY.md`,
     `spec/architecture.md`
   - Result: the spec has one place for assets, attackers, guards,
     accepted risks and the checklist. The findings table shows this
     RFC's findings as "open".
   - Tests: `mdbook build` of `spec/` (the existing CI step).

2. **Safe terminal text**
   - Status: done
   - Files: `src/text/safe.rs`, `src/text/mod.rs`,
     `src/text/conversation.rs`, `src/text/question.rs`,
     `src/tui/view/` (questions), `src/command/chat/`, `src/command/run.rs`,
     `src/command/send.rs`, `ci/check.sh`, `spec/command.md`, `spec/tui.md`
   - Result: findings 1 and 7 closed.
   - Tests: the `safe`, text and TUI tests above.

3. **Imports without rules**
   - Status: done
   - Files: `src/session/store.rs`, `src/command/import.rs`,
     `spec/sessions.md`, `spec/command.md`
   - Result: finding 2 closed.
   - Tests: the import tests above.

4. **No redirects**
   - Status: done
   - Files: `src/provider/http.rs`, `src/provider/tests_http.rs`,
     `spec/providers.md`
   - Result: finding 4 closed.
   - Tests: the redirect test.

5. **Child environment**
   - Status: done
   - Files: `src/config/file.rs`, `src/config/mod.rs` (`[run_command]`),
     `src/tools/run_command.rs`, `src/lsp/server.rs`, `src/secrets/mod.rs`
     (the shared list of provider variables), `spec/config.md`,
     `spec/tools.md`
   - Result: finding 3 closed.
   - Tests: the `printenv`, `keep_env`, LSP and config tests.

6. **Built-in rules for `.git` and command options**
   - Status: done
   - Files: `src/config/permissions.rs`, `src/permissions/check.rs`,
     `src/permissions/tests.rs`, `src/tools/tests.rs`, `spec/config.md`,
     `spec/tools.md`
   - Result: findings 5 and 6 closed. If a question's reason changes shape,
     the protocol version goes up.
   - Tests: the permission tests above; the existing tests with
     `.git/**` fixtures still pass.

7. **Readable key warning**
   - Status: done
   - Files: `src/config/file.rs`, `src/config/tests.rs`, `spec/config.md`
   - Result: finding 8 closed.
   - Tests: the mode tests.

8. **Sandbox**
   - Status: done
   - Files: `src/sandbox.rs` (new: building the `bwrap` arguments),
     `src/tools/run_command.rs`, `src/config/` (`[sandbox]`), the question
     details, `spec/config.md`, `spec/tools.md`, `spec/readme.md`
   - Result: finding 9 addressed for `run_command` on Linux, opt-in.
   - Tests: the sandbox tests above, plus unit tests of the argument list
     for each combination of settings.

9. **Docs**
   - Status: done
   - Files: `readme.md`, `todo.md`, `CHANGELOG.md`, `spec/security.md`
     (findings marked closed), `rfcs/readme.md`
   - Result: users know about the built-in rules, `keep_env` and the
     sandbox.
   - Tests: `cargo fmt`, `clippy -D warnings`, `cargo test`, `ci/check.sh`.

## Alternatives

- **Strip every escape sequence in the TUI and plain output, also in
  questions.** It is simpler, but the user would never learn that a
  command hides something. Rejected for questions, used everywhere else.
- **Ask on import, rule by rule.** `codeagent import` may run without a
  terminal, and a list of rules from a stranger is exactly the kind of
  question users wave through. Rejected for dropping them.
- **Remove every secret variable from child processes.** It breaks `gh`,
  `aws` and `docker login`, which are normal tasks. Rejected. Provider
  keys belong to `codeagent` itself, so those are removed.
- **Follow redirects to the same host only.** Same-host redirects to
  providers are rare, and the error tells the user what to set. Not worth
  the extra rule.
- **Deny instead of ask for command options.** `find -exec` and `git -c`
  have real uses in tasks. Rejected for `ask`, as in RFC 0010.
- **A sandbox through Landlock with a crate.** It needs a new dependency,
  and either `unsafe` or a crate that wraps it. `bwrap` does more (mount
  and network namespaces), is packaged everywhere and runs as a separate
  program. Rejected for now.
- **Sandbox on by default.** `bwrap` is often not installed, and user
  namespaces are off on some systems. A default that fails on many
  machines teaches users to turn it off. Rejected for opt-in, with a
  readme section.

## Open questions

1. Should `.envrc` and editor task files (`.vscode/tasks.json`,
   `.idea/workspace.xml`) get a built-in `ask` for writes, like `.git` gets
   a `deny`? Proposal: no. See the design.
2. Is the list of command options complete enough? Candidates left out:
   `git ** --ext-diff` (it needs a configured diff driver, which the
   `.git` rule now protects) and `rsync ** --rsync-path*`.
3. Should the sandbox also apply to `api_key_cmd`, the clipboard programs
   and `codeagent commit`'s `git`? Proposal: no. They are commands the user
   configured, not commands the model asked for.
4. Should step 8 (sandbox) be its own RFC, so that steps 1 to 7 can be
   accepted and built first? Proposal: keep it here and build it last;
   split it if it is still being discussed when steps 1 to 7 are done.
5. Should `--yes` refuse the built-in asks (command options), as it
   refuses a `deny`? It would make `codeagent run --yes` in CI safer
   against injected `find -exec`, at the cost of tasks that need it.

All answered as proposed when the RFC was built: no protection for
`.envrc` and editor task files (1); the list of options stays (2); the
sandbox is for `run_command` only (3); it is built in this RFC (4);
`--yes` answers the built-in asks (5).

## Changes while building

- **The question note** reads `note: this question contains N hidden
  characters, shown as ⟨…⟩`, for every kind of question, not only
  commands. It counts each hidden character in every text of the
  question; a command shown in the summary and in the details counts
  twice.
- **Questions are made visible through their JSON form**: every string of
  a copy of the question goes through `visible`, so new detail kinds are
  covered without code. The TUI keeps the original for the answer, since
  "always" sends the proposed rule back as it is.
- **The TUI drops bidirectional controls everywhere**, not only in
  questions, as it drops control characters.
- **`codeagent import` gets its own answer**, `imported`, with the session
  and the rules left out; the protocol version is now `11`.
- **`.git` itself is protected**, besides `.git/**` and `**/.git/**`: in a
  submodule or worktree `.git` is a file that names the git directory.
- **`git ** --exec*` and `git ** -x` became `git rebase ** --exec*` and
  `git rebase ** -x`**, since `-x` and `--exec-path` mean other things
  elsewhere.
- **`builtin_except` names a command pattern by its text**
  (`"git config"`); any other entry is still a path pattern.
- **The sandbox makes `/tmp` a private, empty directory**, besides hiding
  the listed places. It hides the state and runtime directories as found
  (`$XDG_STATE_HOME`, `$XDG_RUNTIME_DIR`), so the server's socket is out of
  reach too; the scratch directory in the runtime directory is bound again.
  A hidden file (`~/.netrc`) is covered with an empty file.
- **The question says `in sandbox`** through a new optional field of the
  command details, `sandbox`, in the plain question and in the TUI's tool
  details.
- **The key warning is shown by `codeagent run` and the TUI**;
  `codeagent chat` does not load the configuration, so it does not show
  it.
- **The provider key list** is fixed in `src/child.rs` (`PROVIDER_KEYS`):
  RFC 0030 has no list of provider variables, only name endings.
- **Left out:** the `ci/check.sh` check for `print!` in `src/command/`
  (in "Testing"); a `grep` cannot tell model text from `codeagent`'s own,
  and the renderers are covered by tests. It is in `todo.md`.
- The sandbox tests run where `bwrap` can make namespaces and say so when
  they skip; on the machine this was built on they ran.

