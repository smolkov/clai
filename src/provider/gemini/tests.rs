use super::*;
use crate::image::{Image, fixtures};
use crate::provider::sse::SseParser;
use crate::provider::types::{Message, ToolSpec};

/// Decodes a whole fixture, once at once and once byte by byte; `end`
/// included, as the HTTP layer calls it when the body ends.
fn decode(fixture: &str) -> Result<(Vec<Event>, bool), ProviderError> {
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
        Ok((events, decoder.done()))
    };
    let whole = run(fixture.len().max(1));
    assert_eq!(whole, run(1), "byte-wise decoding differs");
    whole
}

fn fixture(name: &str) -> String {
    let path = format!(
        "{}/tests/fixtures/gemini/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(path).unwrap()
}

fn usage(input: u64, output: u64, cached: u64) -> Event {
    Event::Usage {
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cached,
        cache_write_tokens: 0,
    }
}

fn done(stop_reason: StopReason) -> Event {
    Event::Done { stop_reason }
}

#[test]
fn text_answer() {
    let (events, done_) = decode(&fixture("text.sse")).unwrap();
    assert!(done_);
    assert_eq!(
        events,
        [
            Event::TextDelta {
                text: "Hello".into()
            },
            usage(25, 0, 0),
            Event::TextDelta {
                text: ", wörld!".into()
            },
            // Thinking tokens count as output.
            usage(25, 15, 0),
            done(StopReason::EndTurn),
        ]
    );
}

#[test]
fn tool_calls_keep_their_signature_and_thoughts_are_left_out() {
    let (events, _) = decode(&fixture("tool_calls.sse")).unwrap();
    assert_eq!(
        events,
        [
            Event::TextDelta {
                text: "Let me look.".into()
            },
            Event::ToolCallStart {
                id: "fc_01".into(),
                name: "read_file".into()
            },
            Event::ToolCallArgsDelta {
                id: "fc_01".into(),
                json: r#"{"path":"src/parse.rs"}"#.into()
            },
            Event::ToolCallEnd {
                id: "fc_01".into(),
                arguments: json!({"path": "src/parse.rs"}),
                signature: Some("c2lnLW9uZQ==".into()),
            },
            Event::ToolCallStart {
                id: "fc_02".into(),
                name: "list_files".into()
            },
            Event::ToolCallArgsDelta {
                id: "fc_02".into(),
                json: "{}".into()
            },
            // A parallel call has no signature of its own; it is still
            // marked as a call of this adapter.
            Event::ToolCallEnd {
                id: "fc_02".into(),
                arguments: json!({}),
                signature: Some(String::new()),
            },
            usage(1500, 100, 1000),
            done(StopReason::ToolUse),
        ]
    );
}

#[test]
fn calls_without_id_get_one_and_bad_arguments_become_an_object() {
    let (events, _) = decode(&fixture("no_id.sse")).unwrap();
    let ends: Vec<_> = events
        .iter()
        .filter_map(|e| match e {
            Event::ToolCallEnd { id, arguments, .. } => Some((id.as_str(), arguments.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        ends,
        [
            ("gemini_call_1", json!({"path": "a"})),
            ("gemini_call_2", json!({}))
        ]
    );
    assert_eq!(events.last(), Some(&done(StopReason::ToolUse)));
}

#[test]
fn finish_reasons() {
    let (events, _) = decode(&fixture("max_tokens.sse")).unwrap();
    assert_eq!(events.last(), Some(&done(StopReason::MaxTokens)));
    let (events, _) = decode(&fixture("safety.sse")).unwrap();
    assert_eq!(events.last(), Some(&done(StopReason::Other)));
    let (events, done_) = decode(&fixture("blocked.sse")).unwrap();
    assert!(done_);
    assert_eq!(events, [usage(12, 0, 0), done(StopReason::Other)]);
}

#[test]
fn an_error_chunk_fails_and_a_cut_body_is_not_done() {
    let e = decode(&fixture("error.sse")).unwrap_err();
    assert_eq!(e.kind, ErrorKind::Overloaded);
    assert_eq!(
        e.message,
        "UNAVAILABLE: The model is overloaded. Please try again later."
    );

    let (events, done_) = decode(&fixture("cut.sse")).unwrap();
    assert!(!done_, "the HTTP layer reports the cut");
    assert_eq!(
        events,
        [Event::TextDelta {
            text: "Half an ans".into()
        }]
    );
}

fn call(id: &str, name: &str, arguments: Value, signature: Option<&str>) -> Block {
    Block::ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
        signature: signature.map(str::to_owned),
    }
}

fn result(id: &str, content: &str, is_error: bool) -> Block {
    Block::ToolResult {
        call_id: id.into(),
        content: content.into(),
        is_error,
    }
}

fn request(messages: Vec<Message>) -> Request {
    Request {
        model: "gemini-3.8-flash".into(),
        system: "Be brief.".into(),
        messages,
        tools: vec![ToolSpec {
            name: "read_file".into(),
            description: "Reads a file.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string", "description": "The file."},
                    "lines": {
                        "type": "array",
                        "items": {"type": "object", "properties": {"n": {"type": ["integer", "null"]}},
                                  "additionalProperties": false},
                        "maxItems": 4
                    }
                },
                "required": ["path"],
                "additionalProperties": false,
                "$schema": "http://json-schema.org/draft-07/schema#"
            }),
        }],
        max_output_tokens: Some(1000),
        temperature: Some(0.5),
        images: true,
    }
}

#[test]
fn encodes_a_conversation_with_calls_of_gemini_and_of_another_model() {
    let messages = vec![
        Message::user("Read a"),
        Message {
            role: Role::Assistant,
            content: vec![
                Block::Text {
                    text: "Looking.".into(),
                },
                call("fc_01", "read_file", json!({"path": "a"}), Some("c2ln")),
                call("gemini_call_2", "list_files", json!({}), Some("")),
                call("fc_03", "read_file", json!("{broken"), Some("")),
            ],
        },
        Message {
            role: Role::User,
            content: vec![
                result("fc_01", "fn main() {}", false),
                result("gemini_call_2", "no such directory", true),
                result("fc_03", "bad arguments", true),
            ],
        },
        // A call made by another model before the switch to Gemini.
        Message {
            role: Role::Assistant,
            content: vec![call("toolu_9", "write_file", json!({"path": "b"}), None)],
        },
        Message {
            role: Role::User,
            content: vec![result("toolu_9", "written", false)],
        },
        // Neighbours of the same role are merged.
        Message::user("Thanks"),
    ];
    assert_eq!(
        encode(&request(messages)),
        json!({
            "contents": [
                {"role": "user", "parts": [{"text": "Read a"}]},
                {"role": "model", "parts": [
                    {"text": "Looking."},
                    {"functionCall": {"id": "fc_01", "name": "read_file", "args": {"path": "a"}},
                     "thoughtSignature": "c2ln"},
                    {"functionCall": {"name": "list_files", "args": {}}},
                    {"functionCall": {"id": "fc_03", "name": "read_file", "args": {}}},
                ]},
                {"role": "user", "parts": [
                    {"functionResponse": {"id": "fc_01", "name": "read_file",
                                          "response": {"output": "fn main() {}"}}},
                    {"functionResponse": {"name": "list_files",
                                          "response": {"error": "no such directory"}}},
                    {"functionResponse": {"id": "fc_03", "name": "read_file",
                                          "response": {"error": "bad arguments"}}},
                ]},
                {"role": "model", "parts": [
                    {"text": "[tool call write_file {\"path\":\"b\"}]"},
                ]},
                {"role": "user", "parts": [
                    {"text": "[result of tool call write_file]\nwritten"},
                    {"text": "Thanks"},
                ]},
            ],
            "systemInstruction": {"parts": [{"text": "Be brief."}]},
            "tools": [{"functionDeclarations": [{
                "name": "read_file",
                "description": "Reads a file.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "path": {"type": "string", "description": "The file."},
                        "lines": {
                            "type": "array",
                            "items": {"type": "object",
                                      "properties": {"n": {"type": "integer", "nullable": true}}},
                            "maxItems": 4
                        }
                    },
                    "required": ["path"]
                }
            }]}],
            "generationConfig": {"maxOutputTokens": 1000, "temperature": 0.5},
        })
    );
}

#[test]
fn images_go_inline_or_as_a_note() {
    let image = Image::from_bytes(fixtures::png(2, 2)).unwrap();
    let message = Message {
        role: Role::User,
        content: vec![
            Block::Text {
                text: "See [Image #1]".into(),
            },
            Block::Image(image.clone()),
        ],
    };
    let mut req = request(vec![message]);
    let body = encode(&req);
    assert_eq!(
        body["contents"][0]["parts"][1],
        json!({"inlineData": {"mimeType": "image/png", "data": image.base64()}})
    );
    req.images = false;
    let body = encode(&req);
    assert_eq!(
        body["contents"][0]["parts"][1],
        json!({"text": "[image omitted: this model does not read images]"})
    );
}

#[test]
fn a_minimal_request_has_no_optional_fields() {
    let mut req = request(vec![Message::user("hi")]);
    req.system.clear();
    req.tools.clear();
    req.max_output_tokens = None;
    req.temperature = None;
    assert_eq!(
        encode(&req),
        json!({"contents": [{"role": "user", "parts": [{"text": "hi"}]}]})
    );
}

fn status(status: u16, body: &str) -> ProviderError {
    Gemini.status_error(StatusError {
        status,
        retry_after: None,
        body: body.into(),
    })
}

#[test]
fn status_errors() {
    let e = status(
        400,
        r#"{"error": {"code": 400, "message": "The input token count (1200000) exceeds the maximum number of tokens allowed (1048576).", "status": "INVALID_ARGUMENT"}}"#,
    );
    assert_eq!(e.kind, ErrorKind::ContextTooLong);

    let e = status(
        400,
        r#"{"error": {"code": 400, "message": "API key not valid. Please pass a valid API key.", "status": "INVALID_ARGUMENT"}}"#,
    );
    assert_eq!(e.kind, ErrorKind::Auth);
    assert_eq!(
        e.message,
        "HTTP 400: INVALID_ARGUMENT: API key not valid. Please pass a valid API key."
    );

    let e = status(
        403,
        r#"{"error": {"code": 403, "message": "Permission denied.", "status": "PERMISSION_DENIED"}}"#,
    );
    assert_eq!(e.kind, ErrorKind::Auth);

    let e = status(
        429,
        r#"{"error": {"code": 429, "message": "Quota exceeded.", "status": "RESOURCE_EXHAUSTED",
            "details": [{"@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "12.5s"}]}}"#,
    );
    assert_eq!(e.kind, ErrorKind::RateLimit);
    assert_eq!(e.retry_after, Some(Duration::from_millis(12_500)));

    let e = status(
        400,
        r#"{"error": {"code": 400, "message": "Invalid value at 'contents'", "status": "INVALID_ARGUMENT"}}"#,
    );
    assert_eq!(e.kind, ErrorKind::InvalidRequest);

    let e = status(503, "upstream connect error");
    assert_eq!(e.kind, ErrorKind::Overloaded);
    assert_eq!(e.message, "HTTP 503: upstream connect error");
}

#[test]
fn the_model_goes_into_the_path_and_the_key_into_a_header() {
    let req = request(vec![Message::user("hi")]);
    assert_eq!(
        Gemini.url("https://generativelanguage.googleapis.com", &req),
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-3.8-flash:streamGenerateContent?alt=sse"
    );
}
