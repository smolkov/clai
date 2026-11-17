# Gemini provider

- RFC: 0032
- Status: done
- Spec: [`spec/providers.md`](../spec/providers.md), "Shared types",
  "Adapters", "Adding an API type"; [`spec/config.md`](../spec/config.md),
  "Providers and models"

## Summary

A third API type, `api = "gemini"`, talks to Google's Gemini API in its own
format: `streamGenerateContent` with server-sent events. A tool call gets
an optional opaque `signature` in the shared types, because Gemini wants
its thought signatures back with every tool call. The example
configuration and the docs gain a `gemini` provider with the current
models.

## Motivation

- Gemini models are among the strong coding models, with a context window
  of about a million tokens. Today `codeagent` cannot use them well.
- Gemini's OpenAI-compatible endpoint (`api = "openai-compatible"`,
  `base_url = ".../v1beta/openai"`) is not enough: Gemini 3 and later
  models hand out a thought signature with each tool call and refuse the
  next request if it is not sent back. The Chat Completions format carries
  it only in a Google extension field the `openai-compatible` adapter does
  not know, so tool use breaks after the first call.

## Scope

In:

- The `gemini` adapter (`src/provider/gemini.rs`): encoding, decoding,
  errors, retries as for the others.
- `signature` on tool calls in the shared types and the saved session.
- `api = "gemini"` in the configuration, with its default `base_url`.
- A `gemini` provider in `src/config/example.toml`, `readme.md` and
  `spec/config.md`.

Out:

- The Interactions API (see "Alternatives").
- Thinking settings (`thinking_level`), thought summaries in the TUI.
- Images, audio, files, Google Search grounding, code execution and other
  Gemini tools.
- Vertex AI (other address, OAuth instead of an API key).
- Explicit context caches (`cachedContents`).
- A model list from the API (`models.list`); models stay in the
  configuration.

## Design

### Endpoint and key

`POST {base_url}/v1beta/models/{id}:streamGenerateContent?alt=sse`.
Default `base_url`: `https://generativelanguage.googleapis.com`. The key goes
in the header `x-goog-api-key`, never in the URL, so it cannot end up in a
log line with the address. **(proposal)** The model `id` is put into the path
as is; an `id` with `/`, `?` or `#` is a load error for a `gemini`
provider. **(proposal)**

### Request

| Shared type | Wire format |
|-------------|-------------|
| `system` | `systemInstruction: {"parts": [{"text"}]}`; left out when empty. |
| `user` / `assistant` | `contents[]` with `role: "user"` / `role: "model"`. Neighbours with the same role are merged, messages without parts left out, as for `anthropic`. |
| `Text` | `{"text"}` part. |
| `ToolCall` | `{"functionCall": {"id", "name", "args"}, "thoughtSignature"}` part; `thoughtSignature` only when the call has a `signature`. Arguments that are not an object (broken JSON kept as a string) are sent as `{}`. |
| `ToolResult` | `{"functionResponse": {"id", "name", "response"}}` part in a `user` message. `name` is taken from the call with the same id; `response` is `{"output": content}`, or `{"error": content}` when `is_error`. **(proposal)** |
| `ToolSpec` | One `tools: [{"functionDeclarations": [...]}]` entry with `name`, `description`, `parametersJsonSchema` (the schema as is). **(proposal; to check: `parameters` takes only an OpenAPI subset)** |
| `max_output_tokens` | `generationConfig.maxOutputTokens`, only when set. |
| `temperature` | `generationConfig.temperature`, only when set. |

The tool results of one turn go into one `user` message, in the order of the
calls, as the API wants for parallel calls.

### Answer

| Wire data | Event |
|-----------|-------|
| `candidates[0].content.parts[].text` (not `thought`) | `TextDelta` |
| `parts[]` with `thought: true` | ignored |
| `parts[].functionCall` | `ToolCallStart`, one `ToolCallArgsDelta` with all arguments, `ToolCallEnd` with the part's `thoughtSignature` |
| `usageMetadata` | `Usage`: `promptTokenCount` is `input_tokens` (it includes cached input), `candidatesTokenCount` + `thoughtsTokenCount` is `output_tokens`, `cachedContentTokenCount` is `cache_read_tokens`. |
| `candidates[0].finishReason` | `Done`: `ToolUse` if the answer has a tool call, else `STOP` → `EndTurn`, `MAX_TOKENS` → `MaxTokens`, else `Other` (logged: `SAFETY`, `RECITATION`, `MALFORMED_FUNCTION_CALL`, …) |
| `promptFeedback.blockReason` without candidates | `Done` with `Other`, logged |
| end of the body after a `finishReason` | the stream ends |

- A function call arrives whole in one chunk; its arguments do not stream.
- A call without `id` gets one made up (`call_1`, `call_2`, … per answer),
  as in `openai-compatible`. A made-up id is not sent back as `id`, only used
  to pair the result. **(proposal)**
- A `thoughtSignature` on a text part, or on an empty part at the end, is
  not kept. The API needs signatures only on function calls; elsewhere they
  are optional. **(proposal)**
- Several calls in one answer are sent one after another in their order.
  Only the first carries a signature; the others have none, and are sent
  back in the same order.

### Signature in the shared types

`Block::ToolCall` and `Event::ToolCallEnd` get `signature: Option<String>`:
opaque text the adapter that made it needs back, unchanged. **(proposal)**

- The `anthropic` and `openai-compatible` adapters never set it and do not
  send it.
- The saved session keeps it; old sessions without the field load with
  `None` (`serde(default)`, left out when `None`).
- Secret hiding (RFC 0030) leaves it alone: it is not text for the model.
- History trimming and compaction keep it with its call.
- A call made by another model (the session switched to Gemini halfway)
  has no signature. The adapter then sends the documented placeholder value
  that turns the check off, if there is one, else the call as text. See
  open question 3.

### Errors

| Wire | Kind |
|------|------|
| 400 `INVALID_ARGUMENT` with "exceeds the maximum number of tokens" or similar | `ContextTooLong` |
| 400 other | `InvalidRequest`, with the API's message |
| 401, 403 `PERMISSION_DENIED`, 400 `API key not valid` | `Auth` |
| 429 `RESOURCE_EXHAUSTED` | `RateLimit`; `retryDelay` of a `RetryInfo` detail as `retry_after` |
| 500, 503 `UNAVAILABLE`, 504 | `Overloaded` |
| `error` object in an SSE chunk | as by its `code` and `status` |

The error body is `{"error": {"code", "message", "status", "details"}}`.

### Configuration

- `api` takes `gemini`; the unknown-API error lists it. The test configs
  that use `gemini` as the unknown name use `bedrock` instead.
- `cache` is ignored: the Gemini API caches a repeated prefix by itself
  (implicit caching) and reports it in `cachedContentTokenCount`.
- Example in `spec/config.md` and `src/config/example.toml`:

  ```toml
  [providers.gemini]
  api         = "gemini"
  api_key_env = "GEMINI_API_KEY"

  [providers.gemini.models.flash]
  id                = "gemini-3.8-flash"
  context_window    = 1048576
  max_output_tokens = 65536

  [providers.gemini.models.pro]
  id                = "gemini-3.1-pro-preview"
  context_window    = 1048576
  max_output_tokens = 65536
  ```

  **(proposal; the ids and limits are to be checked against the model
  pages just before building, see open question 4)**

## Spec changes

- `spec/providers.md`: `signature` in "Message" and "Events"; a new
  section "gemini" next to "anthropic" and "openai-compatible".
- `spec/config.md`: `api` values, default `base_url`, the `id` rule for
  `gemini`, the example.
- `spec/sessions.md` if it lists the fields of a saved block.
- `readme.md`: the provider example; `CHANGELOG.md`, `todo.md`.

## Testing

- Encoding: a `Request` with system prompt, text, two calls (one with a
  signature), their results (one an error) and tools, compared with an
  expected JSON file.
- Decoding, from recorded SSE fixtures in `tests/fixtures/gemini/`: text in
  several chunks; a call with signature; two parallel calls; a call without
  id; thought parts; `MAX_TOKENS`; `SAFETY`; a blocked prompt; usage with
  cache and thinking tokens; an error chunk; a body cut before
  `finishReason`.
- Status errors: 400 context, 400 bad key, 403, 429 with `retryDelay`, 503,
  in `src/provider/tests_http.rs` against the local test server.
- The signature survives a round trip: stream → saved session → loaded →
  encoded request.
- An old saved session without `signature` loads.
- Config: `api = "gemini"` loads without `base_url`; an `id` with `/` is
  refused; the unknown-API error lists `gemini`.
- By hand, with a key: a session with `gemini/flash` that reads and edits a
  file over several tool rounds.

## Implementation plan

1. **Signature in the shared types**
   - Status: done
   - Files: `src/provider/types.rs`, `src/agent/stream.rs`,
     `src/agent/history.rs`, the other adapters, `src/session/store.rs`
   - Result: a tool call carries an optional signature through the loop
     and the saved session; nothing uses it yet.
   - Tests: session round trip, old sessions.

2. **Gemini adapter**
   - Status: done
   - Files: `src/provider/gemini.rs`, `src/provider/mod.rs`,
     `src/provider/tests_http.rs`, `tests/fixtures/gemini/`
   - Result: requests are encoded and answers decoded.
   - Tests: encoding, decoding, status errors.

3. **Configuration and docs**
   - Status: done
   - Files: `src/config/mod.rs`, `src/config/example.toml`,
     `src/config/tests.rs`, `tests/configs/`, `spec/`, `readme.md`,
     `CHANGELOG.md`, `todo.md`
   - Result: `api = "gemini"` works end to end.
   - Tests: config tests; the manual session.

## Alternatives

- **The OpenAI-compatible endpoint, no new code.** Works for text, breaks
  tool use on Gemini 3 and later (see "Motivation"). Teaching the
  `openai-compatible` adapter Google's `extra_content` field would need the
  same `signature` in the shared types, and mix one provider's quirks into
  an adapter meant for many.
- **The Interactions API** (`POST /v1beta/interactions`, generally
  available since June 2026; `generateContent` is now called "legacy" but
  "remains fully supported"). It keeps the conversation on Google's servers
  by default (`previous_interaction_id`, kept 55 days on the paid tier).
  `codeagent` keeps history itself and sends it whole, so it would run with
  `store: false` and send all thought blocks back, which is the same work
  as signatures, in a younger format with less documentation. A later
  adapter if `generateContent` goes away.
- **Signature as a generic `extra: serde_json::Value`.** More room for
  future provider data, but untyped; one optional string is all that is
  needed today.

## Open questions

1. `generateContent` (proposed) or the Interactions API?
2. Provider and model names in the example: `gemini/flash` and
   `gemini/pro` (proposed), or the model ids as names?
3. A tool call without a signature from an earlier model in the same
   session: send a placeholder signature if Google documents one, turn the
   call into text, or refuse to switch to Gemini in a session with tool
   calls? Proposed: placeholder if documented, else text.
4. Which models go into the example? Current ids per Google's model page
   (November 2026): stable `gemini-3.8-flash`, `gemini-3.6-flash`,
   `gemini-3.5-flash-lite`, `gemini-3.1-flash-lite`; preview
   `gemini-3.1-pro-preview`, `gemini-3-flash-preview`; the Gemini 4 models
   `gemini-4-carbon`, `gemini-4-argon`, `gemini-4-hydrogen`.
   `gemini-2.5-pro` and `gemini-2.5-flash` are deprecated and redirected
   from 1 December 2026. Proposed: `gemini-3.8-flash` and
   `gemini-3.1-pro-preview`, or a Gemini 4 model if one is meant for
   coding.
5. Show thought summaries in the TUI later (they would need a thinking
   block in the shared types, see `spec/providers.md`, open question 1)?
6. Should `thinking_level` become a model key (`minimal` … `high`) in this
   RFC, or later?

All answered when the RFC was built: `generateContent` (1); `gemini/flash`
and `gemini/hydrogen` (2, 4); calls of another model go as text (3);
thought summaries (5) and `thinking_level` (6) are left for later.

## Changes while building

- **Tool schemas go as `parameters`, not `parametersJsonSchema`.** The API
  reference documents only `parameters`, a subset of OpenAPI, so the
  adapter leaves out the keys outside it (`additionalProperties` is in 10
  of the tool schemas) and turns a type list with `"null"` into
  `nullable`.
- **Calls of another model go as text.** Google documents no placeholder
  signature, so a call without a signature is sent as a text part
  `[tool call NAME ARGUMENTS]`, and its result as text too. To tell the
  calls apart, a call of this adapter always has a signature: Gemini's, or
  an empty one when Gemini sent none (a parallel call, a model without
  thinking).
- **Made-up ids are `gemini_call_N`** and are not sent back, so the
  adapter can tell them from ids of the API.
- **Images are in.** RFC 0034 was built first: images go as `inlineData`,
  and `images` is on by default for `gemini`.
- **`Codec::url` gets the request**, because the model id is part of the
  path.
- **`Done` comes when the body ends**, not with the chunk that has the
  `finishReason`, because the usage may follow in a later chunk.
- **The models in the example** are `gemini-3.8-flash` and
  `gemini-4-hydrogen`, both stable and meant for coding and agents on
  Google's models page in November 2026, each with a 1 048 576-token
  context and 65 536 output tokens.
- The live tests have `gemini_text` and `gemini_tools`
  (`GEMINI_API_KEY`); their answers may now have 2048 tokens, because
  thinking models spend output tokens on thoughts.
- Not tested with a real key: the whole adapter is tested against
  fixtures written from the API reference, not against recorded answers.
  `todo.md` has the manual check.

