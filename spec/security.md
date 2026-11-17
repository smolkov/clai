# Security

What `codeagent` protects, from whom, and how. One place for the threat
model, so that a new feature can be checked against it. Decided in RFC 0010
and RFC 0035; RFC 0030 (secrets) and RFC 0023 (skills) add guards.

## Assets

| Asset | Why it matters |
|-------|----------------|
| Provider keys | Cost money, and name the user at the provider. |
| Other secrets on the machine | Files (`.env`, keys, `~/.ssh`, `~/.aws`) and environment variables (`GITHUB_TOKEN`). |
| The user's code and the conversation | May be confidential; reach the provider by design, nobody else. |
| The workspace and the user's tools | `codeagent` must not leave the repository, git, the shell or the editor in a state that runs code later without the user's consent. |
| The user's terminal and clipboard | Escape sequences can write to the clipboard, change the title, or hide what is shown. |
| API spend | A looping model costs money. |

## Attackers

| Attacker | What it controls |
|----------|------------------|
| A repository | Its files, its project file (`.codeagent/config.toml`), its skills and agent files, its build scripts, what its language server sees. |
| Text the model reads | File contents, command output, answers of language servers, web content in files. Its only lever is what the model then asks for (prompt injection). |
| A file the user is given | An exported session, a theme file, an image. |
| The network | Between `codeagent` and the provider; a provider that redirects. |
| Other local users | The same machine, other accounts. |

Not in the model: the provider itself (it sees what it is sent, by design),
root on the machine, and a user who allows everything (`allow
run_command(*)`, `--yes` in an untrusted repository).

## Entry points and guards

| Source | Trusted | Guard |
|--------|---------|-------|
| User config file | Yes | Validation. A file with `api_key` that others can read gives a warning ([`config.md`](config.md), "Provider keys"). |
| Project config file | No: comes with the repository | May only make permissions stricter; no providers, models, `[tui]`, `[skills]`, `[session]`, `[run_command]`, `[sandbox]` (see [`config.md`](config.md), "Files"). |
| Model output (text, tool calls) | No | Argument checks, workspace boundary, permission rules including the built-in rules (secret files, `.git/`, options that run commands; see [`config.md`](config.md), "Built-in rules"), questions (see [`tools.md`](tools.md)). |
| Language servers | No: they run code of the project (build scripts, macros, plugins) | Started only after the start question, or with `[lsp] start = "allow"` in the user file; a project file may only turn them off. Their answers are data: locations are filtered with the `read_file` rules, text outside the workspace is not shown, and a `workspace/applyEdit` is never applied. A malformed message fails the server, not the session. No provider keys in their environment. |
| Tool output (file contents, command output) | No | Sent to the model as data, and the system prompt says so; never interpreted by `codeagent`, never logged. Secrets hidden (RFC 0030). |
| Child processes (`run_command`, language servers) | No: they run the repository's code | No provider keys in their environment ([`tools.md`](tools.md), "run_command"); optionally the sandbox ([`config.md`](config.md), "Sandbox"). |
| The terminal | — | Plain output (`codeagent run`, `codeagent chat`) removes escape sequences and bidirectional controls from text that comes from outside `codeagent`; the TUI drops control characters. Questions show hidden characters visibly ([`command.md`](command.md), "Terminal output"). |
| Imported session files | No: written by someone else | Session rules are not imported ([`sessions.md`](sessions.md), "Persistence"); image names must be their hash. |
| Clients | Yes, same user | Socket directory mode `0700`, owned by the user, not a link; the server never listens on the network. |
| A later web UI | Only through a bridge the user starts and authenticates | Same protocol and same answer checks as any client (see [`sessions.md`](sessions.md), "Clients"). |
| Provider APIs | For their answers only | HTTPS (or a local address the user configured); keys only to the provider they belong to; redirects are not followed ([`providers.md`](providers.md), "Adapters"). |

Answers are checked by the server, not by the client: a client can only pick
one of the choices of the open question, and an `always` rule must match the
call. A buggy client can therefore not allow more than the user chose.

## Accepted risks

| Risk | Why it stays |
|------|--------------|
| Secrets the model read stay in the saved session until `codeagent kill`. | The journal files are `0600`; RFC 0030 hides known secret shapes before they are saved. Encrypting sessions is out of scope. |
| Without the sandbox, an allowed command may read and write anything the user can, and use the network. | The sandbox is opt-in, because `bwrap` is often missing and user namespaces are off on some systems. |
| Commands that run code by their nature (`make`, `npm run`, interpreters, `awk` with `system`, `sed` with `e`) do whatever the code says. | A rule that allows them allows that; the built-in asks cover only options whose purpose is to run another command. |
| Joined short options (`tar -xIzstd`) and globs (`cat .e*`) are not matched by the built-in rules. | Only plain words are matched; the sandbox is the full answer. |
| `--yes` answers every `ask`, the built-in ones too. | That is what `--yes` means; `deny` rules and the workspace boundary still hold. |
| An active skill's or agent's instructions are trusted as the user's. | The user turns them on; the boxes show where they come from. |

## Checklist for a new feature

An RFC that adds an entry point adds its row above in the same change, and
answers:

1. Which input does it take, and from which attacker?
2. What does it write, and where; can it widen what a later step allows?
3. Which child processes does it start, with which environment?
4. What does it print to a terminal, and is that text from outside?
5. Which limit bounds it (size, count, time, cost)?
6. Can a project file or an imported file turn it on or widen it?

## Findings

Reviews of the code against this model. Each finding names its place.

| RFC | # | Severity | Finding | State |
|-----|---|----------|---------|-------|
| 0010 | 1 | high | No built-in deny for secret files. | closed: built-in rules |
| 0010 | 2 | medium | Read `deny` rules did not reach command arguments (`cat .env`). | closed: argument check |
| 0010 | 3 | medium | No guidance about untrusted content in the system prompt. | closed: two prompt bullets |
| 0010 | 4 | low | Log file and state directory used the default mode. | closed: `0600`, `0700` |
| 0010 | 5 | low | Secrets stay in saved sessions. | accepted (see above) |
| 0035 | 1 | high | Plain output wrote escape sequences from model text, tool lines and questions to the terminal. | closed: safe terminal text |
| 0035 | 2 | high | `codeagent import` kept the session `allow` rules of the file. | closed: rules are not imported |
| 0035 | 3 | medium | `run_command` and language servers inherited the provider keys. | closed: keys removed |
| 0035 | 4 | medium | Provider requests followed redirects with the key and the body. | closed: no redirects |
| 0035 | 5 | medium | No protection for `.git/`: hooks and `core.fsmonitor` could be written. | closed: built-in deny |
| 0035 | 6 | medium | An `allow` rule for `find`, `git`, `rg`, `tar`, `rsync`, `ssh` also allowed their options that run commands. | closed: built-in asks |
| 0035 | 7 | low | Bidirectional and zero-width characters could hide part of a question. | closed: shown visibly |
| 0035 | 8 | low | `api_key` in a file others can read gave no warning. | closed: warning |
| 0035 | 9 | open | Commands were not limited once they ran. | addressed: opt-in sandbox (Linux) |

Checked and sound in RFC 0035: the socket directory, the scratch directory,
the project file's limits, language servers, image names in bundles, the
protocol's line limit and answer checks, TUI rendering, the never-crash
lints, and skills.
