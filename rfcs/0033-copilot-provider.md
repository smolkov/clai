# GitHub Copilot provider

- RFC: 0033
- Status: draft
- Spec: [`spec/providers.md`](../spec/providers.md), "Adapters";
  [`spec/config.md`](../spec/config.md), "Providers and models";
  [`spec/command.md`](../spec/command.md)

## Summary

A provider type `api = "copilot"` lets a user with a GitHub Copilot
subscription use the models of that subscription (Claude, GPT, Gemini, as
GitHub offers them) in `codeagent`. `codeagent login copilot` signs in once
with GitHub's device flow; the provider then gets short-lived Copilot tokens
by itself and sends requests in the Chat Completions format the
`openai-compatible` adapter already speaks. `codeagent login copilot` also
lists the models the account may use.

**Blocking:** GitHub supports this only for clients it has approved (see
"Is it allowed"). Nothing in this RFC is built before that is settled.

## Motivation

- Many developers already pay for Copilot through their employer and have no
  Anthropic, OpenAI or Google key. For them, Copilot is the only way to use
  `codeagent` with a strong model.
- GitHub opened Copilot subscriptions to a third-party agent in January
  2026 (OpenCode, see the GitHub changelog "GitHub Copilot now supports
  OpenCode", 2026-01-16), so this is a path GitHub has started to allow, not
  only a workaround.

## Is it allowed

What is known (November 2026):

- `api.githubcopilot.com` is not a documented public API. GitHub's official
  REST endpoints for Copilot are for administration and billing.
- GitHub supports Copilot subscriptions in OpenCode with a device login.
  The Copilot SDK (for apps that embed the Copilot agent) needs a
  subscription too.
- Community reports say GitHub warned that sending another client's
  `Copilot-Integration-Id`, or using another app's OAuth client id, can get
  accounts flagged for abuse, and that a third-party OAuth app with the
  `copilot` scope sees a shorter model list than GitHub's own clients.

So `codeagent` does this only as itself:

- its own GitHub OAuth app (client id in the binary; a client id is not a
  secret), registered by the maintainer;
- its own `Copilot-Integration-Id` and `Editor-Version`
  (`codeagent/<version>`), never those of VS Code, the Copilot CLI or
  OpenCode;
- no personal access tokens unless GitHub documents them for third-party
  tools.

If GitHub does not allow `codeagent`'s app to exchange its token for a
Copilot token, this RFC is rejected, and the fallback in "Alternatives"
(GitHub Models through `openai-compatible`) is documented instead.
**(proposal)**

## Scope

In:

- `codeagent login copilot` and `codeagent logout copilot`; `/login
  copilot` in the TUI.
- Storing the GitHub token; getting and refreshing Copilot tokens.
- `api = "copilot"`: the Chat Completions request of `openai-compatible`
  with Copilot's address and headers.
- The model list of the account, shown by `login` and `codeagent models
  --available copilot`.

Out:

- Models that Copilot serves only through the Responses API or the
  Anthropic Messages format (some GPT Codex models): a later change, with
  the `openai-responses` adapter if one comes.
- Copilot's own agent, tools, MCP servers or premium-request accounting.
- GitHub Enterprise Server and data-residency hosts (`*.ghe.com`): later,
  with a `github_host` key.
- Choosing models automatically from the list; the user still writes model
  tables, `login` prints them ready to paste.

## Design

### Login

`codeagent login copilot` (also `/login copilot`, which shows the same lines
in a box):

1. `POST https://github.com/login/device/code` with `codeagent`'s client id
   and scope `read:user` (to check: the scope the app needs).
2. Print `Open https://github.com/login/device and enter ABCD-1234`, copy the
   code with the clipboard setting of the TUI, and open the browser when
   there is a display (`xdg-open`, `open`). **(proposal)**
3. Poll `POST https://github.com/login/oauth/access_token` at the
   `interval` GitHub gives, honouring `slow_down`, until `expires_in`;
   `Ctrl-C` stops.
4. Exchange the GitHub token once for a Copilot token (below), to check that
   the account has Copilot, and fetch the model list.
5. Save the GitHub token; print the account and the models:

   ```
   Signed in to GitHub as octocat; Copilot Business.
   Models (paste into ~/.config/codeagent/config.toml):

   [providers.copilot]
   api = "copilot"

   [providers.copilot.models.claude-sonnet-4-6]
   id             = "claude-sonnet-4.6"
   context_window = 128000
   ...
   ```

   The context window and output limit come from the model list
   (`capabilities.limits`), when it has them.

`codeagent logout copilot` deletes the saved token. Neither command needs the
server.

### Token storage

- The GitHub token is saved in `$XDG_STATE_HOME/codeagent/auth/copilot.json`
  (fallback `~/.local/state/codeagent/auth/`), directory `0700`, file
  `0600`, written atomically, with the account login and the time.
  **(proposal)**
- The provider table may instead name the token with `api_key_env` or
  `api_key_cmd`, for users who keep it in a password store; the file is then
  not read. `api_key` is refused for `copilot`, so the long-lived token is
  never in the config file. **(proposal)**
- The token is a secret of RFC 0030: it is hidden from the model like the
  other provider keys.

### Copilot token

- `GET https://api.github.com/copilot_internal/v2/token` with
  `Authorization: token <github token>` returns a short-lived token, its
  `expires_at`, and `endpoints.api` (the base address for the account; it
  differs for Business and Enterprise).
- The provider keeps it in memory and gets a new one when less than five
  minutes are left, or when a request answers 401 once (then the request is
  sent again; a second 401 is `Auth`). **(proposal)**
- A failed exchange is `Auth` with GitHub's message: no subscription, the
  organisation's policy forbids it, or the app is not allowed. The message
  says to run `codeagent login copilot` when the GitHub token itself is
  refused.
- The Copilot token is never saved or logged.

### Requests

`POST {endpoints.api}/chat/completions`, encoded and decoded exactly as
`openai-compatible` (`src/provider/openai.rs`), with these headers:

| Header | Value |
|--------|-------|
| `Authorization` | `Bearer <copilot token>` |
| `Copilot-Integration-Id` | `codeagent`'s, as GitHub assigns it |
| `Editor-Version` | `codeagent/<version>` |
| `X-Initiator` | `user` for the first request of a turn, `agent` for the requests after tool results **(proposal; to check: premium requests are counted per user turn)** |
| `Openai-Intent` | `conversation-agent` **(to check)** |

The `Remote` wrapper gets a key source that can change between requests
(today it reads the key once). `base_url` is not a key of a `copilot`
provider; the address comes from the token. **(proposal)**

Copilot's Claude models answer in the Chat Completions format; their
prompt cache is used by Copilot without marks, as for `openai-compatible`.

### Model list

`GET {endpoints.api}/models` lists the models with `id`, `name`,
`capabilities` (`limits.max_context_window_tokens`,
`limits.max_output_tokens`, `supports.tool_calls`) and `policy.state`.
`login` and `codeagent models --available copilot` show those with tool
calls and `policy.state` `enabled`; a model the organisation turned off is
listed apart, with the reason. A model in the config that the account
cannot use fails at the first request with `InvalidRequest` and GitHub's
message.

### Configuration

The model ids and limits below only show the form; the real ones come from
the account's model list.

```toml
[providers.copilot]
api = "copilot"

[providers.copilot.models.sonnet]
id             = "claude-sonnet-4.6"
context_window = 128000
```

- `api` takes `copilot`; `base_url` and `api_key` are load errors for it.
- `cache` is ignored.
- `codeagent models` marks a `copilot` provider `no login` when there is no
  saved token and no `api_key_env` / `api_key_cmd`.

## Spec changes

- `spec/providers.md`: a section "copilot": the token exchange, the headers,
  the refresh, the errors.
- `spec/config.md`: the `api` value and its key rules.
- `spec/command.md`: `codeagent login`, `codeagent logout`, `models
  --available`.
- `spec/tui.md`: `/login`.
- `spec/sessions.md` or a new "Files" place: `auth/` in the state directory.
- `readme.md`, `CHANGELOG.md`, `todo.md`.

## Testing

- Device flow against a local test server: pending, `slow_down`, expired
  code, denied, success; the file is written `0600`.
- Token exchange: success with `endpoints.api`; 401 (bad GitHub token), 403
  (no subscription, policy); refresh before `expires_at`; one retry after a
  401 on a request.
- Requests: the headers, the address from the token, `X-Initiator` per
  request; decoding is the `openai-compatible` one and is not tested again.
- Model list: tool support and policy filtering, the printed TOML.
- Config: `api = "copilot"` without key loads; `base_url` and `api_key` are
  refused; `no login` in `codeagent models`.
- Secrets: the GitHub token never appears in logs, errors or requests to the
  model.
- By hand, with a Copilot Business account: login, a session with a Claude
  and a GPT model over several tool rounds, logout.

## Implementation plan

1. **Permission**
   - Status: todo
   - Register `codeagent`'s GitHub OAuth app; ask GitHub whether it may use
     the Copilot token exchange and which `Copilot-Integration-Id` it gets.
     Stop here if not.

2. **Changing keys in `Remote`**
   - Status: todo
   - Files: `src/provider/remote.rs`, `src/provider/http.rs`
   - Result: a provider can get a fresh key before a request and after a
     401; `anthropic` and `openai-compatible` behave as before.
   - Tests: existing provider tests; a key source that changes.

3. **Login and token storage**
   - Status: todo
   - Files: `src/auth/` (new), `src/cli.rs`, `src/command/login.rs`
   - Result: `codeagent login copilot`, `logout`.
   - Tests: device flow, file mode, errors.

4. **Copilot adapter**
   - Status: todo
   - Files: `src/provider/copilot.rs`, `src/provider/mod.rs`,
     `src/config/mod.rs`
   - Result: sessions with `copilot/...` models.
   - Tests: exchange, refresh, headers, config.

5. **Model list, TUI and docs**
   - Status: todo
   - Files: `src/command/models.rs`, `src/tui/commands.rs`, `spec/`,
     `readme.md`, `CHANGELOG.md`, `todo.md`
   - Result: `models --available copilot`, `/login copilot`, docs.
   - Tests: model list, TUI snapshot of the login box.

## Alternatives

- **GitHub Models** (`models.github.ai`, OpenAI-compatible, a token with
  `models:read`). Documented and allowed, works today with
  `api = "openai-compatible"` and no new code; but it is a separate service
  with its own limits and billing, not the Copilot subscription, and has
  fewer models. If this RFC is rejected, `readme.md` shows it as an example.
- **The Copilot SDK.** Official, but it embeds Copilot's whole agent (its
  loop, tools and permissions); `codeagent` would become a front end of
  another agent instead of using a model.
- **Another client's OAuth client id and integration id** (what many
  community tools do). Works today, but impersonates another product and
  risks the user's GitHub account. Not done. **(proposal)**
- **Personal access tokens.** Reported to work only for GitHub's own tools.

## Open questions

1. Who registers the GitHub OAuth app and asks GitHub for approval, and is
   approval needed at all for a non-partner tool? Until this is answered the
   RFC cannot be accepted.
2. Store the GitHub token in a file (proposed) or the system keyring
   (Secret Service, Keychain), which needs a new crate?
3. `X-Initiator` and `Openai-Intent`: needed, and how does GitHub count
   premium requests for an agent's tool rounds? To check with GitHub's
   documentation or answer.
4. Should `login` write the provider and model tables into the user file
   (as `models --set-default` writes there), instead of printing them?
5. Models served only through the Responses API: wait for a separate RFC
   (proposed), or add that format here?
6. GitHub Enterprise Server and `*.ghe.com` in this RFC, or later
   (proposed)?
