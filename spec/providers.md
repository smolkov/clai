# Providers

The agent loop talks to every provider through one interface that uses the
same types for all providers. It never sees a provider's own request or
response format. Each API type (`api` in the configuration, see
[`config.md`](config.md)) has a provider adapter that turns the shared types
into its API's format and the API's answers back into the shared types. All
provider differences are handled in the adapter.

```
 agent loop ──Request──▶ ┌──────────────────────┐ ──HTTP──▶ provider API
            ◀──Events─── │ adapter (api = ...)  │ ◀──SSE───
                         └──────────────────────┘
          shared types        encode / decode         provider format
```

Statements marked **(assumption)** need a decision from the user.

## Terms

| Term          | Meaning |
|---------------|---------|
| Shared types  | The request, message, event and error types defined here. They are the same for every provider. |
| Adapter       | The code for one API type. It encodes a `Request` into that API's format, sends it, and decodes the answer into events. |
| Wire format   | A provider's own JSON and streaming format. Only its adapter knows it. |

## Shared types

### Request

| Field               | Type            | Meaning |
|---------------------|-----------------|---------|
| `model`             | text            | Model `id` from the configuration, sent to the API as is. |
| `system`            | text            | System prompt (see `context.md`, planned). May be empty. |
| `messages`          | list of Message | The conversation so far, oldest first. |
| `tools`             | list of ToolSpec | Tools the model may call. May be empty. |
| `max_output_tokens` | number, optional | From the model config; the adapter uses the API's default when it is not set. |
| `temperature`       | number, optional | From the model config; not sent when it is not set. |

### Message

A message has a `role`, `user` or `assistant`, and a list of blocks.

| Block        | Fields                          | Used in |
|--------------|---------------------------------|---------|
| `Text`       | `text`                          | `user`, `assistant` |
| `ToolCall`   | `id`, `name`, `arguments` (JSON object) | `assistant` |
| `ToolResult` | `call_id`, `content` (text), `is_error` | `user` |

- Every `ToolCall` in an assistant message is answered by exactly one
  `ToolResult` with the same id in the next user message, in the same order.
  The agent loop ensures this, also for denied and cancelled calls; the
  adapter may rely on it.
- `id` is opaque text. The agent loop only copies it into `call_id`.

### ToolSpec

| Field          | Meaning |
|----------------|---------|
| `name`         | Tool name, e.g. `read_file`. |
| `description`  | Text for the model about what the tool does. |
| `input_schema` | JSON Schema (an `object` schema) of the tool's arguments. |

Tool specs come from [`tools.md`](tools.md) and are the same for every
provider.

### Events

An answer arrives as a stream of events, in this order:

| Event               | Fields | Meaning |
|---------------------|--------|---------|
| `TextDelta`         | `text` | Next part of the answer text. |
| `ToolCallStart`     | `id`, `name` | The model starts a tool call. |
| `ToolCallArgsDelta` | `id`, `json` | Next part of the call's arguments, as JSON text. |
| `ToolCallEnd`       | `id`, `arguments` | The call is complete; `arguments` is the parsed JSON object. |
| `Usage`             | `input_tokens`, `output_tokens` | Token counts of this request. May come more than once; the last one counts. |
| `Done`              | `stop_reason` | Last event of the stream. |

`stop_reason` is one of:

| Value       | Meaning |
|-------------|---------|
| `EndTurn`   | The model finished its answer. |
| `ToolUse`   | The model stopped to have its tool calls run. |
| `MaxTokens` | The answer was cut at `max_output_tokens`. |
| `Other`     | Any other reason (e.g. a content filter); the adapter logs the provider's own value. |

- Text and tool calls may be mixed. Tool calls do not overlap: a
  `ToolCallStart` is followed by its deltas and its `ToolCallEnd` before the
  next tool call starts. If an API sends several calls interleaved, the
  adapter buffers them and sends them one after another. **(assumption)**
- Arguments that are not valid JSON when the call ends are not a stream
  error: the adapter sends `ToolCallEnd` with the raw text as a JSON string,
  and the agent loop answers the call with a tool error so the model can
  retry. **(assumption)** Empty arguments are `{}`; valid JSON that is not
  an object counts as invalid. **(assumption)**
- A stream that ends without `Done` ends with an error instead.

### Errors

A request or a stream ends with a `ProviderError` instead of `Done` when it
fails.

| Kind             | Meaning | Retried by the adapter |
|------------------|---------|------------------------|
| `Auth`           | Key missing, wrong or not allowed (e.g. HTTP 401, 403). | no |
| `RateLimit`      | Too many requests (HTTP 429); carries `retry_after` when the API sends it. | yes |
| `Overloaded`     | The provider is busy or failing (HTTP 5xx, 529). | yes |
| `ContextTooLong` | The request is larger than the model's context window. | no |
| `InvalidRequest` | The API rejected the request (other HTTP 4xx); carries the API's message. | no |
| `Network`        | Connection failed or broke. | yes |
| `Timeout`        | No response headers within `timeout_s`, or a gap of more than `timeout_s` between two parts of the answer. The total length of an answer is not limited. **(assumption)** | yes |
| `Protocol`       | The answer could not be decoded. | no |
| `Unsupported`    | The request needs something the API type or model cannot do, e.g. tools on a model without tool support. | no |
| `Cancelled`      | The agent loop cancelled the request. | no |

Every error carries a message for the user. API keys are never part of a
message or a log entry.

## Interface

The interface has one operation: send a `Request`, and get either an error
or a stream of events. The agent loop can cancel it at any time (`Esc` in
the TUI); the adapter then closes the connection and ends the stream with
`Cancelled`.

In Rust terms (names may change when it is built):

```rust
trait Provider: Send + Sync {
    /// Sends the request and returns its events as a stream.
    fn stream(&self, request: Request, cancel: CancelToken)
        -> BoxStream<'static, Result<Event, ProviderError>>;
}
```

The server creates one `Provider` for each configured provider, from its
`api` type and keys, when the provider is first used. Sessions that use the
same provider share it.

## Adapters

An adapter:

- encodes the `Request` into its wire format and adds the API key and
  headers;
- decodes the streamed answer into events, in the order described in
  [Events](#events);
- maps HTTP statuses and API error bodies to `ProviderError` kinds;
- retries errors marked "yes" in [Errors](#errors), up to `max_retries`
  times with exponential backoff (1 s, 2 s, 4 s, …, at most 30 s), using
  `retry_after` when it is given (at most 60 s). It retries only before the
  first event is sent; once the stream has started, an error ends it and
  the agent loop decides. A stream that breaks before its first event is
  retried as `Network`. **(assumption)** Before each wait it sends a `Retry`
  event with the attempt, the limit and the error, which the loop passes on as
  the session event `retry`; it does not count as the start of the stream.
- reads the API key on the first request and keeps it; if reading fails,
  the request ends with `Auth` and the next request tries again;
- ignores fields it does not know, so new API fields do not break `clai`;
  unknown event types and block types are logged at debug level.
  **(assumption)**
- never panics on any input from the network: anything it cannot decode is
  a `Protocol` error (see [`agent.md`](../agent.md), "Never crash"). A
  single event larger than 16 MiB is a `Protocol` error too.

HTTPS uses the system's trusted certificates plus the bundled Mozilla root
certificates, so it works both behind a company CA and in a minimal
container without CA files. Proxies are taken from `HTTPS_PROXY` and
related variables. **(assumption)**

### anthropic

Messages API, `POST {base_url}/v1/messages` with `stream: true`.
Default `base_url`: `https://api.anthropic.com`.

| Shared type | Wire format |
|-------------|-------------|
| `system` | Top-level `system` field. |
| `Text` | `{"type": "text"}` content block. |
| `ToolCall` | `{"type": "tool_use", "id", "name", "input"}` block in an assistant message. |
| `ToolResult` | `{"type": "tool_result", "tool_use_id", "content", "is_error"}` block in a user message. |
| `ToolSpec` | `tools[]` entry with `name`, `description`, `input_schema`. |
| `max_output_tokens` | `max_tokens` (required by the API; the adapter sends the model's `max_output_tokens`, else 8192). **(assumption)** |
| Key | Header `x-api-key`, plus `anthropic-version: 2023-06-01`. |
| Message order | The API wants `user` and `assistant` to alternate: neighbouring messages with the same role are merged into one. Messages without blocks are left out. **(assumption)** |

| Wire event | Event |
|------------|-------|
| `content_block_delta` with `text_delta` | `TextDelta` |
| `content_block_start` with `tool_use` | `ToolCallStart` |
| `content_block_delta` with `input_json_delta` | `ToolCallArgsDelta` |
| `content_block_stop` of a `tool_use` block | `ToolCallEnd` |
| `message_start.usage`, `message_delta.usage` | `Usage`; `input_tokens` includes cached input (`cache_creation_input_tokens`, `cache_read_input_tokens`). |
| `message_delta.stop_reason`, then `message_stop` | `Done`: `end_turn` → `EndTurn`, `tool_use` → `ToolUse`, `max_tokens` → `MaxTokens`, else `Other` |
| `error` event | `ProviderError` (`overloaded_error` → `Overloaded`, …) |
| `ping`, `thinking` blocks and their deltas | ignored |

### openai-compatible

Chat Completions API, `POST {base_url}/chat/completions` with
`stream: true` and `stream_options.include_usage: true`. Used for OpenAI,
Ollama, vLLM, llama.cpp and similar servers. `base_url` is required.

| Shared type | Wire format |
|-------------|-------------|
| `system` | First message with `role: "system"`. |
| `Text` | `content` of a `user` or `assistant` message. |
| `ToolCall` | `tool_calls[]` entry `{"id", "type": "function", "function": {"name", "arguments"}}` of an assistant message; `arguments` is JSON as text. |
| `ToolResult` | Its own message `{"role": "tool", "tool_call_id", "content"}`. The API has no error flag, so `is_error` results get the prefix `Error: `. **(assumption)** |
| `ToolSpec` | `tools[]` entry `{"type": "function", "function": {"name", "description", "parameters"}}`. |
| `max_output_tokens` | `max_tokens`, only when set. |
| Key | Header `Authorization: Bearer <key>`, only when a key is configured. |

| Wire data | Event |
|-----------|-------|
| `choices[0].delta.content` | `TextDelta` |
| `choices[0].delta.tool_calls[i]` | collected per `index` (by position when there is no `index`) |
| `data: [DONE]`, or the end of the body after a `finish_reason` | for each collected call in index order: `ToolCallStart`, one `ToolCallArgsDelta` with all arguments (if any), `ToolCallEnd`; then `Done` |
| `usage` (last chunk) | `Usage` |
| `finish_reason` | `stop_reason` of `Done`: `ToolUse` if the answer has tool calls, else `stop` → `EndTurn`, `length` → `MaxTokens`, else `Other` |
| chunk with `error` | `ProviderError` (`Overloaded`, or `ContextTooLong` if the message says so) |

Tool calls are collected and sent complete when the answer ends, so calls
that arrive interleaved need no special handling; text still streams. In
this API the arguments of a tool call do not stream. **(assumption)**

- Some servers send no tool-call `id` (e.g. older Ollama versions). The
  adapter then makes one up (`call_1`, `call_2`, … per answer), so the
  agent loop always has an id.
- Some servers end with `finish_reason: "stop"` although the answer has tool
  calls. The adapter reports `ToolUse` whenever the answer contains a tool
  call. **(assumption)**
- Some servers send a whole tool call in one chunk, several calls without
  `index`, arguments as a JSON object instead of text, or no `[DONE]`; all
  of these give the same events.
- Error bodies `{"error": {"message", "code"}}` (OpenAI) and
  `{"error": "text"}` (Ollama) are both understood; `code:
  "context_length_exceeded"` is `ContextTooLong`.

### Adding an API type

A new API type is one new adapter and one new `api` value in the
configuration. The shared types, the interface and the agent loop do not
change. If a new API has a feature the shared types cannot express, the
shared types are extended here first, and the other adapters either support
it or return `Unsupported`.

## Testing

- Each adapter is tested against recorded wire responses (fixture files),
  including errors, missing ids, interleaved tool calls, broken JSON and
  streams cut in the middle. These tests need no network and no key.
- Encoding is tested by comparing the JSON the adapter builds for a given
  `Request` with an expected file.
- The agent loop is tested with a fake provider that replays a scripted list
  of events and errors; it never uses a real adapter.

## Open questions

1. Should reasoning ("thinking") blocks be part of the shared types in the
   first version? They differ between providers and must be sent back
   unchanged in some APIs.
2. Should images (e.g. screenshots given by the user) be supported as input
   in the first version?
3. Should retries live in the adapter, as proposed, or in the agent loop, so
   that a stream that broke halfway can also be repeated?
4. Should prompt caching (Anthropic `cache_control`) be used, and is it
   decided by the adapter alone?
