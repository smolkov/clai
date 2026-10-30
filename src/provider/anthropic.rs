//! `api = "anthropic"`: Messages API with streaming.

use std::collections::HashMap;

use serde_json::{Map, Value, json};

use super::error::{ErrorKind, ProviderError};
use super::http::{StatusError, status_kind, status_message};
use super::remote::{Codec, Decode};
use super::sse::SseEvent;
use super::types::{Block, Event, Request, Role, StopReason, parse_arguments};
use crate::config::ApiKey;

const VERSION: &str = "2023-06-01";
/// `max_tokens` is required by the API.
const DEFAULT_MAX_TOKENS: u64 = 8192;

pub(crate) struct Anthropic;

impl Codec for Anthropic {
    fn url(&self, base_url: &str) -> String {
        format!("{base_url}/v1/messages")
    }

    fn headers(&self, key: Option<&ApiKey>) -> Vec<(&'static str, String)> {
        let mut headers = vec![("anthropic-version", VERSION.to_owned())];
        if let Some(key) = key {
            headers.push(("x-api-key", key.expose().to_owned()));
        }
        headers
    }

    fn encode(&self, request: &Request) -> Value {
        encode(request)
    }

    fn status_error(&self, error: StatusError) -> ProviderError {
        let body: Value = serde_json::from_str(&error.body).unwrap_or(Value::Null);
        let api = body.get("error");
        let api_type = api.and_then(|e| e.get("type")).and_then(Value::as_str);
        let detail = api
            .and_then(|e| e.get("message"))
            .and_then(Value::as_str)
            .unwrap_or(&error.body);
        let kind = match api_type {
            Some(t) => error_kind(t, detail),
            None => status_kind(error.status),
        };
        let kind = if error.status == 400 && too_long(detail) {
            ErrorKind::ContextTooLong
        } else {
            kind
        };
        ProviderError {
            kind,
            message: status_message(error.status, detail),
            retry_after: error.retry_after,
        }
    }

    fn decoder(&self) -> Box<dyn Decode> {
        Box::new(Decoder::default())
    }
}

fn too_long(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("prompt is too long") || message.contains("context window")
}

/// Error kind for an API error type (`overloaded_error`, …).
fn error_kind(api_type: &str, message: &str) -> ErrorKind {
    match api_type {
        "authentication_error" | "permission_error" => ErrorKind::Auth,
        "rate_limit_error" => ErrorKind::RateLimit,
        "overloaded_error" | "api_error" => ErrorKind::Overloaded,
        "request_too_large" => ErrorKind::ContextTooLong,
        "invalid_request_error" if too_long(message) => ErrorKind::ContextTooLong,
        _ => ErrorKind::InvalidRequest,
    }
}

pub(crate) fn encode(request: &Request) -> Value {
    let mut body = Map::new();
    body.insert("model".into(), json!(request.model));
    body.insert(
        "max_tokens".into(),
        json!(request.max_output_tokens.unwrap_or(DEFAULT_MAX_TOKENS)),
    );
    body.insert("stream".into(), json!(true));
    if !request.system.is_empty() {
        body.insert("system".into(), json!(request.system));
    }
    if let Some(t) = request.temperature {
        body.insert("temperature".into(), json!(t));
    }

    // The API wants user and assistant to alternate: merge neighbours.
    let mut messages: Vec<(Role, Vec<Value>)> = Vec::new();
    for message in &request.messages {
        let blocks: Vec<Value> = message.content.iter().map(encode_block).collect();
        if blocks.is_empty() {
            continue;
        }
        match messages.last_mut() {
            Some((role, content)) if *role == message.role => content.extend(blocks),
            _ => messages.push((message.role, blocks)),
        }
    }
    let messages: Vec<Value> = messages
        .into_iter()
        .map(|(role, content)| {
            let role = match role {
                Role::User => "user",
                Role::Assistant => "assistant",
            };
            json!({"role": role, "content": content})
        })
        .collect();
    body.insert("messages".into(), Value::Array(messages));

    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| json!({"name": t.name, "description": t.description, "input_schema": t.input_schema}))
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }
    Value::Object(body)
}

fn encode_block(block: &Block) -> Value {
    match block {
        Block::Text { text } => json!({"type": "text", "text": text}),
        Block::ToolCall {
            id,
            name,
            arguments,
        } => {
            // Invalid arguments are kept as a string; the API needs an object.
            let input = if arguments.is_object() {
                arguments.clone()
            } else {
                json!({})
            };
            json!({"type": "tool_use", "id": id, "name": name, "input": input})
        }
        Block::ToolResult {
            call_id,
            content,
            is_error,
        } => json!({
            "type": "tool_result",
            "tool_use_id": call_id,
            "content": content,
            "is_error": is_error,
        }),
    }
}

enum Open {
    Text,
    Tool { id: String, json: String },
    Ignored,
}

#[derive(Default)]
pub(crate) struct Decoder {
    blocks: HashMap<u64, Open>,
    input_tokens: u64,
    output_tokens: u64,
    stop: Option<StopReason>,
    done: bool,
}

fn field<'a>(value: &'a Value, name: &str) -> Result<&'a Value, ProviderError> {
    value
        .get(name)
        .ok_or_else(|| ProviderError::protocol(format!("anthropic: missing `{name}`")))
}

fn text<'a>(value: &'a Value, name: &str) -> Result<&'a str, ProviderError> {
    field(value, name)?
        .as_str()
        .ok_or_else(|| ProviderError::protocol(format!("anthropic: `{name}` is not text")))
}

fn index(value: &Value) -> Result<u64, ProviderError> {
    field(value, "index")?
        .as_u64()
        .ok_or_else(|| ProviderError::protocol("anthropic: `index` is not a number"))
}

fn tokens(usage: &Value, name: &str) -> u64 {
    usage.get(name).and_then(Value::as_u64).unwrap_or(0)
}

impl Decoder {
    fn usage(&self) -> Event {
        Event::Usage {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
        }
    }
}

impl Decode for Decoder {
    fn event(&mut self, sse: SseEvent) -> Result<Vec<Event>, ProviderError> {
        let data: Value = serde_json::from_str(&sse.data).map_err(|e| {
            ProviderError::protocol(format!("anthropic: invalid JSON in event: {e}"))
        })?;
        let kind = data
            .get("type")
            .and_then(Value::as_str)
            .or(sse.event.as_deref())
            .unwrap_or("");
        let mut events = Vec::new();
        match kind {
            "message_start" => {
                if let Some(usage) = data.get("message").and_then(|m| m.get("usage")) {
                    // Cached input counts towards the context as well.
                    self.input_tokens = tokens(usage, "input_tokens")
                        + tokens(usage, "cache_creation_input_tokens")
                        + tokens(usage, "cache_read_input_tokens");
                    self.output_tokens = tokens(usage, "output_tokens");
                    events.push(self.usage());
                }
            }
            "content_block_start" => {
                let index = index(&data)?;
                let block = field(&data, "content_block")?;
                let open = match block.get("type").and_then(Value::as_str) {
                    Some("text") => {
                        if let Some(t) = block.get("text").and_then(Value::as_str)
                            && !t.is_empty()
                        {
                            events.push(Event::TextDelta { text: t.to_owned() });
                        }
                        Open::Text
                    }
                    Some("tool_use") => {
                        let id = text(block, "id")?.to_owned();
                        let name = text(block, "name")?.to_owned();
                        events.push(Event::ToolCallStart {
                            id: id.clone(),
                            name,
                        });
                        Open::Tool {
                            id,
                            json: String::new(),
                        }
                    }
                    other => {
                        tracing::debug!(block_type = ?other, "anthropic: ignored content block");
                        Open::Ignored
                    }
                };
                self.blocks.insert(index, open);
            }
            "content_block_delta" => {
                let index = index(&data)?;
                let delta = field(&data, "delta")?;
                match (
                    delta.get("type").and_then(Value::as_str),
                    self.blocks.get_mut(&index),
                ) {
                    (Some("text_delta"), Some(Open::Text)) => events.push(Event::TextDelta {
                        text: text(delta, "text")?.to_owned(),
                    }),
                    (Some("input_json_delta"), Some(Open::Tool { id, json })) => {
                        let part = text(delta, "partial_json")?;
                        json.push_str(part);
                        events.push(Event::ToolCallArgsDelta {
                            id: id.clone(),
                            json: part.to_owned(),
                        });
                    }
                    (_, None) => {
                        return Err(ProviderError::protocol(format!(
                            "anthropic: delta for unknown block {index}"
                        )));
                    }
                    (other, _) => tracing::debug!(delta_type = ?other, "anthropic: ignored delta"),
                }
            }
            "content_block_stop" => {
                if let Some(Open::Tool { id, json }) = self.blocks.remove(&index(&data)?) {
                    events.push(Event::ToolCallEnd {
                        id,
                        arguments: parse_arguments(&json),
                    });
                }
            }
            "message_delta" => {
                if let Some(reason) = data
                    .get("delta")
                    .and_then(|d| d.get("stop_reason"))
                    .and_then(Value::as_str)
                {
                    self.stop = Some(match reason {
                        "end_turn" | "stop_sequence" => StopReason::EndTurn,
                        "tool_use" => StopReason::ToolUse,
                        "max_tokens" => StopReason::MaxTokens,
                        other => {
                            tracing::info!(stop_reason = other, "anthropic: other stop reason");
                            StopReason::Other
                        }
                    });
                }
                if let Some(usage) = data.get("usage") {
                    if let Some(n) = usage.get("output_tokens").and_then(Value::as_u64) {
                        self.output_tokens = n;
                    }
                    events.push(self.usage());
                }
            }
            "message_stop" => {
                self.done = true;
                events.push(Event::Done {
                    stop_reason: self.stop.unwrap_or(StopReason::Other),
                });
            }
            "error" => {
                let error = data.get("error").unwrap_or(&Value::Null);
                let api_type = error.get("type").and_then(Value::as_str).unwrap_or("");
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("error event without message");
                return Err(ProviderError::new(
                    error_kind(api_type, message),
                    format!("{api_type}: {message}"),
                ));
            }
            "ping" => {}
            other => tracing::debug!(event_type = other, "anthropic: ignored event"),
        }
        Ok(events)
    }

    fn end(&mut self) -> Result<Vec<Event>, ProviderError> {
        Ok(Vec::new())
    }

    fn done(&self) -> bool {
        self.done
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::sse::SseParser;
    use crate::provider::types::{Message, ToolSpec};

    /// Decodes a whole fixture, once at once and once byte by byte.
    fn decode(fixture: &str) -> Result<Vec<Event>, ProviderError> {
        let run = |chunk_size: usize| {
            let mut parser = SseParser::new();
            let mut decoder = Decoder::default();
            let mut events = Vec::new();
            for chunk in fixture.as_bytes().chunks(chunk_size) {
                for sse in parser.push(chunk)? {
                    events.extend(decoder.event(sse)?);
                }
            }
            events.extend(decoder.end()?);
            Ok(events)
        };
        let whole = run(fixture.len().max(1));
        assert_eq!(whole, run(1), "byte-wise decoding differs");
        whole
    }

    #[test]
    fn text_answer() {
        let events = decode(include_str!("../../tests/fixtures/anthropic/text.sse")).unwrap();
        assert_eq!(
            events,
            [
                Event::Usage {
                    input_tokens: 25,
                    output_tokens: 1
                },
                Event::TextDelta {
                    text: "Hello".into()
                },
                Event::TextDelta {
                    text: ", wörld!".into()
                },
                Event::Usage {
                    input_tokens: 25,
                    output_tokens: 15
                },
                Event::Done {
                    stop_reason: StopReason::EndTurn
                },
            ]
        );
    }

    #[test]
    fn tool_use() {
        let events = decode(include_str!("../../tests/fixtures/anthropic/tool_use.sse")).unwrap();
        let tool_events: Vec<_> = events
            .iter()
            .filter(|e| !matches!(e, Event::Usage { .. }))
            .cloned()
            .collect();
        assert_eq!(
            tool_events,
            [
                Event::TextDelta {
                    text: "Let me look.".into()
                },
                Event::ToolCallStart {
                    id: "toolu_01".into(),
                    name: "read_file".into()
                },
                Event::ToolCallArgsDelta {
                    id: "toolu_01".into(),
                    json: "{\"path\": ".into()
                },
                Event::ToolCallArgsDelta {
                    id: "toolu_01".into(),
                    json: "\"src/parse.rs\"}".into()
                },
                Event::ToolCallEnd {
                    id: "toolu_01".into(),
                    arguments: json!({"path": "src/parse.rs"})
                },
                Event::ToolCallStart {
                    id: "toolu_02".into(),
                    name: "list_files".into()
                },
                Event::ToolCallEnd {
                    id: "toolu_02".into(),
                    arguments: json!({})
                },
                Event::Done {
                    stop_reason: StopReason::ToolUse
                },
            ]
        );
        // Cached input is counted.
        assert!(events.contains(&Event::Usage {
            input_tokens: 1500,
            output_tokens: 3
        }));
    }

    #[test]
    fn broken_arguments_are_kept_as_text() {
        let events = decode(include_str!(
            "../../tests/fixtures/anthropic/broken_args.sse"
        ))
        .unwrap();
        assert!(events.contains(&Event::ToolCallEnd {
            id: "toolu_09".into(),
            arguments: json!("{\"path\": \"a")
        }));
    }

    #[test]
    fn error_event() {
        let err = decode(include_str!("../../tests/fixtures/anthropic/error.sse")).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Overloaded);
        assert_eq!(err.message, "overloaded_error: Overloaded");
    }

    #[test]
    fn cut_stream_is_not_done() {
        let fixture = include_str!("../../tests/fixtures/anthropic/text.sse");
        let cut = fixture.get(..fixture.len() / 2).unwrap();
        let mut parser = SseParser::new();
        let mut decoder = Decoder::default();
        for sse in parser.push(cut.as_bytes()).unwrap() {
            decoder.event(sse).unwrap();
        }
        assert!(!decoder.done());
    }

    #[test]
    fn thinking_and_unknown_events_are_ignored() {
        let events = decode(include_str!("../../tests/fixtures/anthropic/thinking.sse")).unwrap();
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, Event::TextDelta { .. }))
                .count(),
            1
        );
    }

    #[test]
    fn invalid_json_is_protocol_error() {
        let err = Decoder::default()
            .event(SseEvent {
                event: None,
                data: "{nope".into(),
            })
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
    }

    #[test]
    fn encoding() {
        let request = Request {
            model: "claude-sonnet-5-5".into(),
            system: "Be brief.".into(),
            messages: vec![
                Message::user("fix it"),
                Message::user("please"),
                Message {
                    role: Role::Assistant,
                    content: vec![
                        Block::Text {
                            text: "Looking.".into(),
                        },
                        Block::ToolCall {
                            id: "t1".into(),
                            name: "read_file".into(),
                            arguments: json!({"path": "a"}),
                        },
                        Block::ToolCall {
                            id: "t2".into(),
                            name: "read_file".into(),
                            arguments: json!("{broken"),
                        },
                    ],
                },
                Message {
                    role: Role::User,
                    content: vec![
                        Block::ToolResult {
                            call_id: "t1".into(),
                            content: "fn a() {}".into(),
                            is_error: false,
                        },
                        Block::ToolResult {
                            call_id: "t2".into(),
                            content: "invalid arguments".into(),
                            is_error: true,
                        },
                    ],
                },
                Message {
                    role: Role::User,
                    content: vec![],
                },
            ],
            tools: vec![ToolSpec {
                name: "read_file".into(),
                description: "Read a file.".into(),
                input_schema: json!({"type": "object", "properties": {"path": {"type": "string"}}}),
            }],
            max_output_tokens: None,
            temperature: Some(0.2),
        };
        assert_eq!(
            encode(&request),
            json!({
                "model": "claude-sonnet-5-5",
                "max_tokens": 8192,
                "stream": true,
                "system": "Be brief.",
                "temperature": 0.2,
                "messages": [
                    {"role": "user", "content": [
                        {"type": "text", "text": "fix it"},
                        {"type": "text", "text": "please"}
                    ]},
                    {"role": "assistant", "content": [
                        {"type": "text", "text": "Looking."},
                        {"type": "tool_use", "id": "t1", "name": "read_file", "input": {"path": "a"}},
                        {"type": "tool_use", "id": "t2", "name": "read_file", "input": {}}
                    ]},
                    {"role": "user", "content": [
                        {"type": "tool_result", "tool_use_id": "t1", "content": "fn a() {}", "is_error": false},
                        {"type": "tool_result", "tool_use_id": "t2", "content": "invalid arguments", "is_error": true}
                    ]}
                ],
                "tools": [{
                    "name": "read_file",
                    "description": "Read a file.",
                    "input_schema": {"type": "object", "properties": {"path": {"type": "string"}}}
                }]
            })
        );
    }

    #[test]
    fn status_errors() {
        let err = |status, body: &str| {
            Anthropic.status_error(StatusError {
                status,
                retry_after: None,
                body: body.into(),
            })
        };
        let auth = err(
            401,
            r#"{"type":"error","error":{"type":"authentication_error","message":"invalid x-api-key"}}"#,
        );
        assert_eq!(auth.kind, ErrorKind::Auth);
        assert_eq!(auth.message, "HTTP 401: invalid x-api-key");
        let long = err(
            400,
            r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 210000 tokens > 200000 maximum"}}"#,
        );
        assert_eq!(long.kind, ErrorKind::ContextTooLong);
        assert_eq!(err(529, "").kind, ErrorKind::Overloaded);
        assert_eq!(
            err(502, "<html>bad gateway</html>").message,
            "HTTP 502: <html>bad gateway</html>"
        );
    }
}
