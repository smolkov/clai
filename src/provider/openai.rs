//! `api = "openai-compatible"`: Chat Completions API with streaming.
//!
//! Used for OpenAI, Ollama, vLLM, llama.cpp and similar servers. Tool calls
//! are collected by index and sent complete when the answer ends, so calls
//! that arrive interleaved need no special case.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use super::error::{ErrorKind, ProviderError};
use super::http::{StatusError, status_kind, status_message};
use super::remote::{Codec, Decode};
use super::sse::SseEvent;
use super::types::{Block, Event, Request, Role, StopReason, parse_arguments};
use crate::config::ApiKey;

pub(crate) struct OpenAi;

impl Codec for OpenAi {
    fn url(&self, base_url: &str) -> String {
        format!("{base_url}/chat/completions")
    }

    fn headers(&self, key: Option<&ApiKey>) -> Vec<(&'static str, String)> {
        key.map(|k| vec![("authorization", format!("Bearer {}", k.expose()))])
            .unwrap_or_default()
    }

    fn encode(&self, request: &Request) -> Value {
        encode(request)
    }

    fn status_error(&self, error: StatusError) -> ProviderError {
        let body: Value = serde_json::from_str(&error.body).unwrap_or(Value::Null);
        let (detail, code) = match body.get("error") {
            // OpenAI: {"error": {"message", "type", "code"}}
            Some(Value::Object(e)) => (
                e.get("message")
                    .and_then(Value::as_str)
                    .unwrap_or(&error.body),
                e.get("code").and_then(Value::as_str),
            ),
            // Ollama and others: {"error": "text"}
            Some(Value::String(s)) => (s.as_str(), None),
            _ => (error.body.as_str(), None),
        };
        let kind = if code == Some("context_length_exceeded") || too_long(detail) {
            ErrorKind::ContextTooLong
        } else {
            status_kind(error.status)
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
    message.contains("context length")
        || message.contains("context window")
        || message.contains("maximum context")
}

/// The API has no error flag for tool results.
const ERROR_PREFIX: &str = "Error: ";

pub(crate) fn encode(request: &Request) -> Value {
    let mut messages = Vec::new();
    if !request.system.is_empty() {
        messages.push(json!({"role": "system", "content": request.system}));
    }
    for message in &request.messages {
        match message.role {
            Role::Assistant => {
                let mut text = String::new();
                let mut calls = Vec::new();
                for block in &message.content {
                    match block {
                        Block::Text { text: t } => text.push_str(t),
                        Block::ToolCall {
                            id,
                            name,
                            arguments,
                        } => {
                            // Invalid arguments are kept as the raw text.
                            let arguments = match arguments {
                                Value::String(raw) => raw.clone(),
                                other => other.to_string(),
                            };
                            calls.push(json!({
                                "id": id,
                                "type": "function",
                                "function": {"name": name, "arguments": arguments},
                            }));
                        }
                        Block::ToolResult { .. } => {}
                    }
                }
                let mut m = Map::new();
                m.insert("role".into(), json!("assistant"));
                m.insert(
                    "content".into(),
                    if text.is_empty() {
                        Value::Null
                    } else {
                        json!(text)
                    },
                );
                if !calls.is_empty() {
                    m.insert("tool_calls".into(), Value::Array(calls));
                }
                messages.push(Value::Object(m));
            }
            Role::User => {
                let mut text = String::new();
                for block in &message.content {
                    match block {
                        Block::ToolResult {
                            call_id,
                            content,
                            is_error,
                        } => {
                            let content = if *is_error {
                                format!("{ERROR_PREFIX}{content}")
                            } else {
                                content.clone()
                            };
                            messages.push(json!({"role": "tool", "tool_call_id": call_id, "content": content}));
                        }
                        Block::Text { text: t } => {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(t);
                        }
                        Block::ToolCall { .. } => {}
                    }
                }
                if !text.is_empty() {
                    messages.push(json!({"role": "user", "content": text}));
                }
            }
        }
    }

    let mut body = Map::new();
    body.insert("model".into(), json!(request.model));
    body.insert("stream".into(), json!(true));
    body.insert("stream_options".into(), json!({"include_usage": true}));
    body.insert("messages".into(), Value::Array(messages));
    if let Some(n) = request.max_output_tokens {
        body.insert("max_tokens".into(), json!(n));
    }
    if let Some(t) = request.temperature {
        body.insert("temperature".into(), json!(t));
    }
    if !request.tools.is_empty() {
        let tools: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {"name": t.name, "description": t.description, "parameters": t.input_schema},
                })
            })
            .collect();
        body.insert("tools".into(), Value::Array(tools));
    }
    Value::Object(body)
}

#[derive(Default)]
struct ToolBuf {
    id: Option<String>,
    name: String,
    arguments: String,
}

#[derive(Default)]
pub(crate) struct Decoder {
    tools: BTreeMap<u64, ToolBuf>,
    finish_reason: Option<String>,
    done: bool,
}

impl Decoder {
    /// Tool calls in index order, then `Done`.
    fn finish(&mut self) -> Vec<Event> {
        let mut events = Vec::new();
        let has_tools = !self.tools.is_empty();
        for (n, tool) in std::mem::take(&mut self.tools).into_values().enumerate() {
            // Some servers send no id; the agent loop needs one.
            let id = tool
                .id
                .filter(|id| !id.is_empty())
                .unwrap_or_else(|| format!("call_{}", n + 1));
            events.push(Event::ToolCallStart {
                id: id.clone(),
                name: tool.name,
            });
            if !tool.arguments.is_empty() {
                events.push(Event::ToolCallArgsDelta {
                    id: id.clone(),
                    json: tool.arguments.clone(),
                });
            }
            events.push(Event::ToolCallEnd {
                id,
                arguments: parse_arguments(&tool.arguments),
            });
        }
        // Some servers say "stop" although the answer has tool calls.
        let stop_reason = match self.finish_reason.as_deref() {
            _ if has_tools => StopReason::ToolUse,
            Some("stop") => StopReason::EndTurn,
            Some("length") => StopReason::MaxTokens,
            other => {
                tracing::info!(finish_reason = ?other, "openai: other finish reason");
                StopReason::Other
            }
        };
        events.push(Event::Done { stop_reason });
        self.done = true;
        events
    }
}

impl Decode for Decoder {
    fn event(&mut self, sse: SseEvent) -> Result<Vec<Event>, ProviderError> {
        if sse.data.trim() == "[DONE]" {
            return Ok(self.finish());
        }
        let data: Value = serde_json::from_str(&sse.data)
            .map_err(|e| ProviderError::protocol(format!("openai: invalid JSON in chunk: {e}")))?;
        if let Some(error) = data.get("error") {
            let message = match error {
                Value::String(s) => s.clone(),
                other => other
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("error in stream")
                    .to_owned(),
            };
            let kind = if too_long(&message) {
                ErrorKind::ContextTooLong
            } else {
                ErrorKind::Overloaded
            };
            return Err(ProviderError::new(kind, message));
        }

        let mut events = Vec::new();
        if let Some(choice) = data.get("choices").and_then(|c| c.get(0)) {
            if let Some(delta) = choice.get("delta") {
                if let Some(text) = delta.get("content").and_then(Value::as_str)
                    && !text.is_empty()
                {
                    events.push(Event::TextDelta {
                        text: text.to_owned(),
                    });
                }
                for (n, call) in delta
                    .get("tool_calls")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .enumerate()
                {
                    // Without an index, calls in one chunk are told apart by position.
                    let index = call
                        .get("index")
                        .and_then(Value::as_u64)
                        .unwrap_or(n as u64);
                    let buf = self.tools.entry(index).or_default();
                    if let Some(id) = call.get("id").and_then(Value::as_str) {
                        buf.id = Some(id.to_owned());
                    }
                    if let Some(function) = call.get("function") {
                        if let Some(name) = function.get("name").and_then(Value::as_str) {
                            buf.name.push_str(name);
                        }
                        match function.get("arguments") {
                            Some(Value::String(part)) => buf.arguments.push_str(part),
                            // Some servers send the arguments as an object.
                            Some(object @ Value::Object(_)) => {
                                buf.arguments.push_str(&object.to_string())
                            }
                            _ => {}
                        }
                    }
                }
            }
            if let Some(reason) = choice.get("finish_reason").and_then(Value::as_str) {
                self.finish_reason = Some(reason.to_owned());
            }
        }
        if let Some(usage) = data.get("usage").filter(|u| u.is_object()) {
            let n = |name| usage.get(name).and_then(Value::as_u64).unwrap_or(0);
            events.push(Event::Usage {
                input_tokens: n("prompt_tokens"),
                output_tokens: n("completion_tokens"),
            });
        }
        Ok(events)
    }

    /// Servers that close the connection after the last chunk without
    /// `[DONE]`: complete if the finish reason came.
    fn end(&mut self) -> Result<Vec<Event>, ProviderError> {
        if !self.done && self.finish_reason.is_some() {
            return Ok(self.finish());
        }
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
            assert!(decoder.done(), "not done");
            Ok(events)
        };
        let whole = run(fixture.len().max(1));
        assert_eq!(whole, run(1), "byte-wise decoding differs");
        whole
    }

    fn without_usage(events: Vec<Event>) -> Vec<Event> {
        events
            .into_iter()
            .filter(|e| !matches!(e, Event::Usage { .. }))
            .collect()
    }

    #[test]
    fn text_answer() {
        let events = decode(include_str!("../../tests/fixtures/openai/text.sse")).unwrap();
        assert_eq!(
            events,
            [
                Event::TextDelta { text: "Hel".into() },
                Event::TextDelta { text: "lo!".into() },
                Event::Usage {
                    input_tokens: 12,
                    output_tokens: 3
                },
                Event::Done {
                    stop_reason: StopReason::EndTurn
                },
            ]
        );
    }

    #[test]
    fn interleaved_tool_calls() {
        let events = decode(include_str!("../../tests/fixtures/openai/tool_calls.sse")).unwrap();
        assert_eq!(
            without_usage(events),
            [
                Event::ToolCallStart {
                    id: "call_a".into(),
                    name: "read_file".into()
                },
                Event::ToolCallArgsDelta {
                    id: "call_a".into(),
                    json: "{\"path\":\"a.rs\"}".into()
                },
                Event::ToolCallEnd {
                    id: "call_a".into(),
                    arguments: json!({"path": "a.rs"})
                },
                Event::ToolCallStart {
                    id: "call_b".into(),
                    name: "read_file".into()
                },
                Event::ToolCallArgsDelta {
                    id: "call_b".into(),
                    json: "{\"path\":\"b.rs\"}".into()
                },
                Event::ToolCallEnd {
                    id: "call_b".into(),
                    arguments: json!({"path": "b.rs"})
                },
                Event::Done {
                    stop_reason: StopReason::ToolUse
                },
            ]
        );
    }

    #[test]
    fn ollama_quirks() {
        // No ids, whole call in one chunk, "stop" with tool calls, no [DONE].
        let events = decode(include_str!("../../tests/fixtures/openai/ollama.sse")).unwrap();
        assert_eq!(
            without_usage(events),
            [
                Event::TextDelta {
                    text: "Sure.".into()
                },
                Event::ToolCallStart {
                    id: "call_1".into(),
                    name: "list_files".into()
                },
                Event::ToolCallArgsDelta {
                    id: "call_1".into(),
                    json: "{}".into()
                },
                Event::ToolCallEnd {
                    id: "call_1".into(),
                    arguments: json!({})
                },
                Event::ToolCallStart {
                    id: "call_2".into(),
                    name: "read_file".into()
                },
                Event::ToolCallArgsDelta {
                    id: "call_2".into(),
                    json: "{\"path\":\"x\"}".into()
                },
                Event::ToolCallEnd {
                    id: "call_2".into(),
                    arguments: json!({"path": "x"})
                },
                Event::Done {
                    stop_reason: StopReason::ToolUse
                },
            ]
        );
    }

    #[test]
    fn length_and_broken_arguments() {
        let events = decode(include_str!("../../tests/fixtures/openai/length.sse")).unwrap();
        assert!(events.contains(&Event::ToolCallEnd {
            id: "call_x".into(),
            arguments: json!("{\"path\":")
        }));
        // A cut-off tool call is still a tool call.
        assert_eq!(
            events.last(),
            Some(&Event::Done {
                stop_reason: StopReason::ToolUse
            })
        );
    }

    #[test]
    fn error_in_stream() {
        let err = decode(include_str!("../../tests/fixtures/openai/error.sse")).unwrap_err();
        assert_eq!(err.kind, ErrorKind::Overloaded);
        assert_eq!(err.message, "model is overloaded");
    }

    #[test]
    fn cut_stream_is_not_done() {
        let mut decoder = Decoder::default();
        decoder
            .event(SseEvent {
                event: None,
                data: r#"{"choices":[{"delta":{"content":"Hi"}}]}"#.into(),
            })
            .unwrap();
        assert!(decoder.end().unwrap().is_empty());
        assert!(!decoder.done());
    }

    #[test]
    fn encoding() {
        let request = Request {
            model: "qwen3-coder:30b".into(),
            system: "Be brief.".into(),
            messages: vec![
                Message::user("fix it"),
                Message {
                    role: Role::Assistant,
                    content: vec![
                        Block::Text {
                            text: "Looking.".into(),
                        },
                        Block::ToolCall {
                            id: "c1".into(),
                            name: "read_file".into(),
                            arguments: json!({"path": "a"}),
                        },
                        Block::ToolCall {
                            id: "c2".into(),
                            name: "read_file".into(),
                            arguments: json!("{broken"),
                        },
                    ],
                },
                Message {
                    role: Role::User,
                    content: vec![
                        Block::ToolResult {
                            call_id: "c1".into(),
                            content: "fn a() {}".into(),
                            is_error: false,
                        },
                        Block::ToolResult {
                            call_id: "c2".into(),
                            content: "invalid arguments".into(),
                            is_error: true,
                        },
                        Block::Text {
                            text: "and test it".into(),
                        },
                    ],
                },
                Message {
                    role: Role::Assistant,
                    content: vec![Block::ToolCall {
                        id: "c3".into(),
                        name: "list_files".into(),
                        arguments: json!({}),
                    }],
                },
            ],
            tools: vec![ToolSpec {
                name: "read_file".into(),
                description: "Read a file.".into(),
                input_schema: json!({"type": "object"}),
            }],
            max_output_tokens: Some(1000),
            temperature: None,
        };
        assert_eq!(
            encode(&request),
            json!({
                "model": "qwen3-coder:30b",
                "stream": true,
                "stream_options": {"include_usage": true},
                "max_tokens": 1000,
                "messages": [
                    {"role": "system", "content": "Be brief."},
                    {"role": "user", "content": "fix it"},
                    {"role": "assistant", "content": "Looking.", "tool_calls": [
                        {"id": "c1", "type": "function", "function": {"name": "read_file", "arguments": "{\"path\":\"a\"}"}},
                        {"id": "c2", "type": "function", "function": {"name": "read_file", "arguments": "{broken"}}
                    ]},
                    {"role": "tool", "tool_call_id": "c1", "content": "fn a() {}"},
                    {"role": "tool", "tool_call_id": "c2", "content": "Error: invalid arguments"},
                    {"role": "user", "content": "and test it"},
                    {"role": "assistant", "content": null, "tool_calls": [
                        {"id": "c3", "type": "function", "function": {"name": "list_files", "arguments": "{}"}}
                    ]}
                ],
                "tools": [{"type": "function", "function": {"name": "read_file", "description": "Read a file.", "parameters": {"type": "object"}}}]
            })
        );
    }

    #[test]
    fn status_errors() {
        let err = |status, body: &str| {
            OpenAi.status_error(StatusError {
                status,
                retry_after: None,
                body: body.into(),
            })
        };
        let long = err(
            400,
            r#"{"error":{"message":"This model's maximum context length is 32768 tokens","type":"invalid_request_error","code":"context_length_exceeded"}}"#,
        );
        assert_eq!(long.kind, ErrorKind::ContextTooLong);
        let ollama = err(
            404,
            r#"{"error":"model \"qwen\" not found, try pulling it first"}"#,
        );
        assert_eq!(ollama.kind, ErrorKind::InvalidRequest);
        assert_eq!(
            ollama.message,
            "HTTP 404: model \"qwen\" not found, try pulling it first"
        );
        assert_eq!(err(401, "").kind, ErrorKind::Auth);
    }

    #[test]
    fn bearer_only_with_key() {
        assert!(OpenAi.headers(None).is_empty());
    }
}
