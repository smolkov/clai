# clai specification

`clai` is an AI coding agent for the terminal, written in Rust. The user
describes a task in plain language. `clai` sends it to a large language model
(LLM), which answers with text and tool calls (read a file, edit a file, run a
command). `clai` runs the tool calls and returns the results to the model
until the task is done.

This file is the start of the specification and its index. Each topic has its
own file in `spec/`, listed in [Topic files](#topic-files). When this file and a topic file disagree, the topic file
is wrong until the user decides otherwise.

> Status: draft. Statements marked **(assumption)** were filled in without a
> decision from the user. They must be confirmed or replaced before they are
> built. See [Open questions](#open-questions).

## Goals

- Work on a local project from the terminal: answer questions about the code,
  change it, run its build and tests.
- Keep the user in control: show every change and command, and ask before
  anything that writes or executes, unless the user allowed it beforehand.
- Work in the background: sessions keep working without a TUI and wait for
  the user only when a decision is needed (see [`sessions.md`](sessions.md),
  "Background work").
- Never crash: errors are reported and the session continues (see
  [`agent.md`](../agent.md), "Never crash").
- Start fast and ship as a single static binary with no runtime dependencies.
- Be predictable: the same configuration and input give the same tool
  behaviour; only the model's answers vary.

## Non-goals

- No graphical or web interface; terminal only in the first version.
- No IDE plugin in the first version.
- No hosting or training of models; `clai` only calls provider APIs in the first version.
- No multi-user mode and no network server. The local background server that
  holds sessions ([`sessions.md`](sessions.md)) is reachable only by its user.

## Terms

| Term          | Meaning |
|---------------|---------|
| Session       | One conversation in one workspace, with its history and settings. Sessions live in the server and keep running when no TUI is attached; several can run at once. |
| Server        | Background `clai` process of the user that holds and runs all sessions. |
| Attach / detach | Connect a TUI to a session, or disconnect it without stopping the session. |
| Turn          | One user message and everything `clai` does until it waits for the user again. |
| Provider      | A service that hosts LLMs and exposes an API, e.g. Anthropic or a local Ollama. Defined in the configuration. |
| Adapter       | The code for one provider API type (`api` in the configuration). It turns the shared request and event types into the API's own format and back. See [`providers.md`](providers.md). |
| Model         | An LLM of a provider, defined in the configuration and referred to as `<provider>/<model>`. Each session uses one model, which can be changed in the TUI. |
| TUI           | The full-screen terminal interface of an interactive session. |
| Tool          | An action the model may ask `clai` to run, e.g. `read_file`, `edit_file`, `run_command`. |
| Tool call     | One request from the model to run a tool with given arguments. |
| Permission    | The rule that decides whether a tool call runs, is asked about, or is denied. |
| Workspace     | The directory a session was created in; its file tools are limited to it. |
| Context       | Everything sent to the model in a request: system prompt, history, tool results. |

## Usage overview

```sh
clai                          # new session in the current directory, attach the TUI
clai "fix the failing test"   # one-shot: run one task, print the result, exit
clai attach [NAME]            # attach to a running session
clai ls                       # list sessions
clai models                   # list configured providers and models
clai commit                   # print a commit message for the changes here
```

All commands, options and exit codes: [`command.md`](command.md).

An interactive session runs as a full-screen terminal user interface (TUI):
conversation, tool calls and permission prompts on one screen, with an input
editor at the bottom. `Esc` or `Ctrl-C` cancels the current turn; `Ctrl-D`
at an empty input detaches and exits the TUI while the session keeps running.
Several sessions can run at once; the TUI shows one and can switch between
them. Details: [`tui.md`](tui.md), [`sessions.md`](sessions.md). One-shot
mode, and output that is not a terminal, use plain text instead.

## Behaviour summary

- **Agent loop:** send context to the model, stream the answer to the
  attached TUIs, run any tool calls, append their results, repeat until the model
  answers without tool calls or the user stops it at a limit.
- **Background work:** a session keeps working whether a TUI is attached or
  not. It waits for the user only for a decision: a permission question or a
  limit question. Details: [`sessions.md`](sessions.md).
- **Limits:** after a configurable number of model requests and tool calls
  the turn pauses and asks the user whether to continue, so a confused model
  cannot loop forever.
- **Tools:** read file, list or search files, edit file (exact-text replace),
  write new file, run shell command with a timeout. Every tool returns
  either a result or an error message to the model; a tool error never ends
  the session.
- **Permissions:** each tool call is allowed, asked about or denied by
  per-tool defaults and `allow`/`ask`/`deny` rules in the configuration.
  By default reading inside the workspace is allowed; writing files and
  running commands ask the user, showing the diff or the command. Access
  outside the workspace is denied. Details: [`config.md`](config.md),
  "Permissions".
- **Configuration:** read from `~/.config/clai/config.toml`, then
  `<workspace>/.clai/config.toml`, then environment variables, then
  command-line options; later sources win. All providers and models are
  defined in the user config; `clai` has no built-in list. API keys come
  from an environment variable or a command and are never written to a file
  or log. Details: [`config.md`](config.md).
- **Providers and models:** each session uses one configured provider and
  model, chosen when it is created and changeable in the TUI.
- **Errors:** network and API errors are retried with backoff a few times,
  then shown to the user; the session stays open.

## Topic files

Each topic has its own file. Links point to written files; planned files are
created when work on the topic starts. [`SUMMARY.md`](SUMMARY.md) is the table
of contents in [mdBook](https://rust-lang.github.io/mdBook/) format; planned
files appear there as draft chapters (empty links).

When a spec file is added, renamed or removed, update this table and
`SUMMARY.md` in the same change.

| File | Status | Topic |
|------|--------|-------|
| [`readme.md`](readme.md) | draft | This file: purpose, goals, terms, index, open questions. |
| [`SUMMARY.md`](SUMMARY.md) | — | Table of contents for mdBook. |
| [`command.md`](command.md) | draft | All command-line commands: syntax, options, output, exit codes. |
| [`tui.md`](tui.md) | draft | Full-screen interface: layout, session list, model picker, keys, permission prompts, input editor. |
| [`sessions.md`](sessions.md) | draft | Server, several sessions, attach/detach, session commands. |
| [`commit.md`](commit.md) | draft | Commit messages: `clai commit` (prints to stdout) and the TUI `/commit`. |
| [`config.md`](config.md) | draft | Config files, precedence, providers and models, tool permissions, TUI settings and themes, validation, reloading. |
| `agent-loop.md` | planned | Request/response cycle, streaming, limits, cancellation. |
| `tools.md` | planned | Each tool: arguments, results, errors, limits. |
| [`providers.md`](providers.md) | draft | Agent–provider interface: shared request, message, event and error types; adapters per API type that encode and decode the provider's format; retries. |
| `context.md` | planned | System prompt, project instructions, history size and truncation. |

Related files outside `spec/`: [`agent.md`](../agent.md) (rules for the
coding agent working on this repository).

## Open questions

1. **Provider APIs:** are `anthropic` and `openai-compatible` the right API
   types to support first ([`config.md`](config.md))?
2. **First version:** which of interactive mode (TUI) and one-shot mode come
   first?
3. **Permissions:** should "always allow" answers from the TUI be saveable
   to the user configuration?
4. **Platforms:** Linux only, or also macOS and Windows?
5. **Sandbox:** should commands run in a sandbox, or only behind the
   permission prompt?
