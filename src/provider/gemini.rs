//! `api = "gemini"`: Google's Gemini API, `streamGenerateContent` with
//! server-sent events (RFC 0032).
//!
//! Gemini hands out a thought signature with tool calls and wants it back;
//! it travels in the call's `signature`. A call of this adapter always has
//! one (empty when Gemini sent none), so a call without one came from
//! another model and is sent as text, which the API always accepts.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::{Map, Value, json};

use super::error::{ErrorKind, ProviderError};
use super::http::{StatusError, status_kind, status_message};
use super::remote::{Codec, Decode};
use super::sse::SseEvent;
use super::types::{Block, Event, Request, Role, StopReason, image_note};
use crate::config::ApiKey;

/// Ids this adapter makes up for calls without one; never sent back.
const MADE_UP_ID: &str = "gemini_call_";

pub(crate) struct Gemini;

impl Codec for Gemini {
    fn url(&self, base_url: &str, request: &Request) -> String {
        format!(
            "{base_url}/v1beta/models/{}:streamGenerateContent?alt=sse",
            request.model
        )
    }

    fn headers(&self, key: Option<&ApiKey>) -> Vec<(&'static str, String)> {
        key.map(|k| vec![("x-goog-api-key", k.expose().to_owned())])
            .unwrap_or_default()
    }

    fn encode(&self, request: &Request) -> Value {
        encode(request)
    }

    fn status_error(&self, error: StatusError) -> ProviderError {
        let body: Value = serde_json::from_str(&error.body).unwrap_or(Value::Null);
        match body.get("error") {
            Some(e @ Value::Object(_)) => {
                let mut e = api_error(e, Some(error.status));
                e.message = status_message(error.status, &e.message);
                e.retry_after = e.retry_after.or(error.retry_after);
                e
            }
            _ => ProviderError {
                kind: status_kind(error.status),
                message: status_message(error.status, &error.body),
                retry_after: error.retry_after,
            },
        }
    }

    fn decoder(&self) -> Box<dyn Decode> {
        Box::new(Decoder::default())
    }
}

fn too_long(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("exceeds the maximum number of tokens")
        || message.contains("input token count")
        || message.contains("context window")
        || message.contains("too long")
}

/// An error object of the API: `{"code", "message", "status", "details"}`.
fn api_error(error: &Value, http: Option<u16>) -> ProviderError {
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("error without message")
        .to_owned();
    let status = error.get("status").and_then(Value::as_str).unwrap_or("");
    let code = http.or_else(|| {
        error
            .get("code")
            .and_then(Value::as_u64)
            .and_then(|c| u16::try_from(c).ok())
    });
    let kind = match status {
        _ if too_long(&message) => ErrorKind::ContextTooLong,
        _ if message.contains("API key not valid") => ErrorKind::Auth,
        "UNAUTHENTICATED" | "PERMISSION_DENIED" => ErrorKind::Auth,
        "RESOURCE_EXHAUSTED" => ErrorKind::RateLimit,
        "UNAVAILABLE" | "INTERNAL" => ErrorKind::Overloaded,
        "DEADLINE_EXCEEDED" => ErrorKind::Timeout,
        "INVALID_ARGUMENT" | "FAILED_PRECONDITION" | "NOT_FOUND" => ErrorKind::InvalidRequest,
        _ => code.map_or(ErrorKind::InvalidRequest, status_kind),
    };
    let message = if status.is_empty() {
        message
    } else {
        format!("{status}: {message}")
    };
    ProviderError {
        kind,
        message,
        retry_after: retry_delay(error),
    }
}

/// The `retryDelay` of a `RetryInfo` detail: `"30s"`, `"1.5s"`.
fn retry_delay(error: &Value) -> Option<Duration> {
    error
        .get("details")?
        .as_array()?
        .iter()
        .find_map(|d| d.get("retryDelay")?.as_str())
        .and_then(|s| s.strip_suffix('s'))
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|s| s.is_finite() && *s >= 0.0)
        .map(Duration::from_secs_f64)
}

/// How a call of the history goes to the API.
struct CallInfo {
    name: String,
    /// Made by this adapter, so sent as a `functionCall`; else as text.
    native: bool,
}

pub(crate) fn encode(request: &Request) -> Value {
    // Results need the name of their call, and go as text when it does.
    let mut calls: HashMap<&str, CallInfo> = HashMap::new();
    for message in &request.messages {
        for block in &message.content {
            if let Block::ToolCall {
                id,
                name,
                signature,
                ..
            } = block
            {
                calls.insert(
                    id,
                    CallInfo {
                        name: name.clone(),
                        native: signature.is_some(),
                    },
                );
            }
        }
    }

    // The API wants user and model to alternate: merge neighbours.
    let mut contents: Vec<(Role, Vec<Value>)> = Vec::new();
    for message in &request.messages {
        let parts: Vec<Value> = message
            .content
            .iter()
            .filter_map(|b| encode_block(b, request.images, &calls))
            .collect();
        if parts.is_empty() {
            continue;
        }
        match contents.last_mut() {
            Some((role, existing)) if *role == message.role => existing.extend(parts),
            _ => contents.push((message.role, parts)),
        }
    }
    let contents: Vec<Value> = contents
        .into_iter()
        .map(|(role, parts)| {
            let role = match role {
                Role::User => "user",
                Role::Assistant => "model",
            };
            json!({"role": role, "parts": parts})
        })
        .collect();

    let mut body = Map::new();
    body.insert("contents".into(), Value::Array(contents));
    if !request.system.is_empty() {
        body.insert(
            "systemInstruction".into(),
            json!({"parts": [{"text": request.system}]}),
        );
    }
    if !request.tools.is_empty() {
        let declarations: Vec<Value> = request
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "parameters": openapi_schema(&t.input_schema),
                })
            })
            .collect();
        body.insert(
            "tools".into(),
            json!([{"functionDeclarations": declarations}]),
        );
    }
    let mut config = Map::new();
    if let Some(n) = request.max_output_tokens {
        config.insert("maxOutputTokens".into(), json!(n));
    }
    if let Some(t) = request.temperature {
        config.insert("temperature".into(), json!(t));
    }
    if !config.is_empty() {
        body.insert("generationConfig".into(), Value::Object(config));
    }
    Value::Object(body)
}

fn encode_block(block: &Block, images: bool, calls: &HashMap<&str, CallInfo>) -> Option<Value> {
    Some(match block {
        Block::Text { text } if text.is_empty() => return None,
        Block::Text { text } => json!({"text": text}),
        Block::Image(image) if images && image.loaded() => json!({
            "inlineData": {
                "mimeType": image.media_type.media_type(),
                "data": image.base64(),
            }
        }),
        Block::Image(image) => json!({"text": image_note(image, images)}),
        Block::ToolCall {
            id,
            name,
            arguments,
            signature,
        } => {
            let Some(signature) = signature else {
                // From another model: as text, with its arguments.
                return Some(json!({"text": format!("[tool call {name} {arguments}]")}));
            };
            // Invalid arguments are kept as a string; the API needs an object.
            let args = if arguments.is_object() {
                arguments.clone()
            } else {
                json!({})
            };
            let mut call = Map::new();
            if !id.starts_with(MADE_UP_ID) {
                call.insert("id".into(), json!(id));
            }
            call.insert("name".into(), json!(name));
            call.insert("args".into(), args);
            let mut part = Map::new();
            part.insert("functionCall".into(), Value::Object(call));
            if !signature.is_empty() {
                part.insert("thoughtSignature".into(), json!(signature));
            }
            Value::Object(part)
        }
        Block::ToolResult {
            call_id,
            content,
            is_error,
        } => {
            let (name, native) = calls
                .get(call_id.as_str())
                .map_or(("", false), |c| (c.name.as_str(), c.native));
            if !native {
                let what = if *is_error { "error of" } else { "result of" };
                return Some(json!({"text": format!("[{what} tool call {name}]\n{content}")}));
            }
            let key = if *is_error { "error" } else { "output" };
            let mut response = Map::new();
            if !call_id.starts_with(MADE_UP_ID) {
                response.insert("id".into(), json!(call_id));
            }
            response.insert("name".into(), json!(name));
            response.insert("response".into(), json!({ key: content }));
            json!({"functionResponse": response})
        }
    })
}

/// Schema keys the API's `parameters` (a subset of OpenAPI) accepts.
const SCHEMA_KEYS: &[&str] = &[
    "type",
    "format",
    "title",
    "description",
    "nullable",
    "enum",
    "maxItems",
    "minItems",
    "properties",
    "required",
    "minProperties",
    "maxProperties",
    "minLength",
    "maxLength",
    "pattern",
    "example",
    "anyOf",
    "propertyOrdering",
    "default",
    "items",
    "minimum",
    "maximum",
];

/// `schema` as the API's `parameters` take it: keys outside the subset
/// (`additionalProperties`, `$schema`, …) left out, at every level; a type
/// list with `"null"` becomes the type and `nullable`.
fn openapi_schema(schema: &Value) -> Value {
    let Value::Object(map) = schema else {
        return schema.clone();
    };
    let mut out = Map::new();
    for (key, value) in map {
        if !SCHEMA_KEYS.contains(&key.as_str()) {
            continue;
        }
        let value = match (key.as_str(), value) {
            ("properties", Value::Object(props)) => Value::Object(
                props
                    .iter()
                    .map(|(k, v)| (k.clone(), openapi_schema(v)))
                    .collect(),
            ),
            ("items", v) => openapi_schema(v),
            ("anyOf", Value::Array(list)) => {
                Value::Array(list.iter().map(openapi_schema).collect())
            }
            ("type", Value::Array(types)) => {
                let named: Vec<&Value> = types.iter().filter(|t| *t != "null").collect();
                if named.len() < types.len() {
                    out.insert("nullable".into(), json!(true));
                }
                named.first().map_or(json!("string"), |t| (*t).clone())
            }
            (_, v) => v.clone(),
        };
        out.insert(key.clone(), value);
    }
    Value::Object(out)
}

#[derive(Default)]
pub(crate) struct Decoder {
    /// Calls of this answer, for made-up ids and the stop reason.
    calls: u32,
    finish: Option<StopReason>,
    done: bool,
}

fn tokens(usage: &Value, name: &str) -> u64 {
    usage.get(name).and_then(Value::as_u64).unwrap_or(0)
}

impl Decoder {
    fn part(&mut self, part: &Value, events: &mut Vec<Event>) {
        if part.get("thought").and_then(Value::as_bool) == Some(true) {
            return;
        }
        if let Some(call) = part.get("functionCall") {
            self.calls += 1;
            let id = call
                .get("id")
                .and_then(Value::as_str)
                .filter(|id| !id.is_empty())
                .map_or_else(|| format!("{MADE_UP_ID}{}", self.calls), str::to_owned);
            let name = call
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            let arguments = match call.get("args") {
                Some(a @ Value::Object(_)) => a.clone(),
                _ => json!({}),
            };
            let signature = part
                .get("thoughtSignature")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned();
            events.push(Event::ToolCallStart {
                id: id.clone(),
                name,
            });
            events.push(Event::ToolCallArgsDelta {
                id: id.clone(),
                json: arguments.to_string(),
            });
            events.push(Event::ToolCallEnd {
                id,
                arguments,
                signature: Some(signature),
            });
        } else if let Some(text) = part.get("text").and_then(Value::as_str)
            && !text.is_empty()
        {
            events.push(Event::TextDelta {
                text: text.to_owned(),
            });
        }
    }
}

impl Decode for Decoder {
    fn event(&mut self, sse: SseEvent) -> Result<Vec<Event>, ProviderError> {
        let data: Value = serde_json::from_str(&sse.data)
            .map_err(|e| ProviderError::protocol(format!("gemini: invalid JSON in event: {e}")))?;
        if let Some(error) = data.get("error") {
            return Err(api_error(error, None));
        }
        let mut events = Vec::new();
        let candidate = data
            .get("candidates")
            .and_then(Value::as_array)
            .and_then(|c| c.first());
        if let Some(candidate) = candidate {
            let parts = candidate
                .get("content")
                .and_then(|c| c.get("parts"))
                .and_then(Value::as_array);
            for part in parts.into_iter().flatten() {
                self.part(part, &mut events);
            }
            if let Some(reason) = candidate.get("finishReason").and_then(Value::as_str) {
                self.finish = Some(match reason {
                    "STOP" => StopReason::EndTurn,
                    "MAX_TOKENS" => StopReason::MaxTokens,
                    other => {
                        tracing::info!(finish_reason = other, "gemini: other finish reason");
                        StopReason::Other
                    }
                });
            }
        } else if let Some(reason) = data
            .get("promptFeedback")
            .and_then(|f| f.get("blockReason"))
            .and_then(Value::as_str)
        {
            tracing::info!(block_reason = reason, "gemini: prompt blocked");
            self.finish = Some(StopReason::Other);
        }
        if let Some(usage) = data.get("usageMetadata") {
            events.push(Event::Usage {
                input_tokens: tokens(usage, "promptTokenCount"),
                output_tokens: tokens(usage, "candidatesTokenCount")
                    + tokens(usage, "thoughtsTokenCount"),
                cache_read_tokens: tokens(usage, "cachedContentTokenCount"),
                cache_write_tokens: 0,
            });
        }
        Ok(events)
    }

    /// The API ends the body after the last chunk; `Done` follows when one
    /// had a finish reason.
    fn end(&mut self) -> Result<Vec<Event>, ProviderError> {
        let Some(finish) = self.finish else {
            return Ok(Vec::new());
        };
        self.done = true;
        let stop_reason = if self.calls > 0 && finish == StopReason::EndTurn {
            StopReason::ToolUse
        } else {
            finish
        };
        Ok(vec![Event::Done { stop_reason }])
    }

    fn done(&self) -> bool {
        self.done
    }
}

#[cfg(test)]
mod tests;
