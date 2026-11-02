//! Adapters over real HTTP against a small local test server.

use std::sync::{Arc, Mutex};

use crate::sync::lock;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use indexmap::IndexMap;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::config::{self, KeySource};

const ANTHROPIC_TEXT: &str = include_str!("../../tests/fixtures/anthropic/text.sse");
const OPENAI_TOOLS: &str = include_str!("../../tests/fixtures/openai/tool_calls.sse");

/// What the server sends for one request.
#[derive(Clone)]
struct Reply {
    status: u16,
    headers: Vec<(&'static str, &'static str)>,
    /// Body parts, each after a pause.
    parts: Vec<(Duration, String)>,
    /// Keep the connection open after the parts.
    stall: bool,
    /// Never send the headers.
    silent: bool,
}

impl Reply {
    fn sse(body: &str) -> Reply {
        Reply {
            status: 200,
            headers: vec![("content-type", "text/event-stream")],
            parts: vec![(Duration::ZERO, body.to_owned())],
            stall: false,
            silent: false,
        }
    }

    fn status(status: u16, body: &str) -> Reply {
        Reply {
            status,
            headers: vec![("content-type", "application/json")],
            ..Reply::sse(body)
        }
    }
}

struct Seen {
    path: String,
    headers: Vec<(String, String)>,
    body: Value,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

struct Server {
    url: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Server {
    fn requests(&self) -> usize {
        lock(&self.seen).len()
    }
}

/// Serves `replies` in order, one per connection.
async fn serve(replies: Vec<Reply>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&seen);
    tokio::spawn(async move {
        let mut replies = replies.into_iter();
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let reply = replies
                .next()
                .unwrap_or_else(|| Reply::status(500, "no more replies"));
            let record = Arc::clone(&record);
            tokio::spawn(async move {
                // Read the head, then the body by content-length.
                let mut data = Vec::new();
                let mut buf = [0u8; 4096];
                let head_end = loop {
                    let n = socket.read(&mut buf).await.unwrap();
                    if n == 0 {
                        return;
                    }
                    data.extend_from_slice(&buf[..n]);
                    if let Some(i) = data.windows(4).position(|w| w == b"\r\n\r\n") {
                        break i + 4;
                    }
                };
                let head = String::from_utf8_lossy(&data[..head_end]).into_owned();
                let mut lines = head.lines();
                let path = lines.next().unwrap().split(' ').nth(1).unwrap().to_owned();
                let headers: Vec<(String, String)> = lines
                    .filter_map(|l| l.split_once(':'))
                    .map(|(n, v)| (n.trim().to_ascii_lowercase(), v.trim().to_owned()))
                    .collect();
                let length: usize = headers
                    .iter()
                    .find(|(n, _)| n == "content-length")
                    .map(|(_, v)| v.parse().unwrap())
                    .unwrap_or(0);
                while data.len() < head_end + length {
                    let n = socket.read(&mut buf).await.unwrap();
                    data.extend_from_slice(&buf[..n]);
                }
                let body = serde_json::from_slice(&data[head_end..head_end + length]).unwrap();
                lock(&record).push(Seen {
                    path,
                    headers,
                    body,
                });

                if reply.silent {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                    return;
                }
                let mut head = format!("HTTP/1.1 {} X\r\nconnection: close\r\n", reply.status);
                for (name, value) in &reply.headers {
                    head.push_str(&format!("{name}: {value}\r\n"));
                }
                head.push_str("\r\n");
                if socket.write_all(head.as_bytes()).await.is_err() {
                    return;
                }
                for (pause, part) in &reply.parts {
                    tokio::time::sleep(*pause).await;
                    if socket.write_all(part.as_bytes()).await.is_err() {
                        return;
                    }
                    let _ = socket.flush().await;
                }
                if reply.stall {
                    tokio::time::sleep(Duration::from_secs(60)).await;
                }
                let _ = socket.shutdown().await;
            });
        }
    });
    Server { url, seen }
}

fn provider_config(api: Api, url: &str, key: KeySource) -> config::Provider {
    config::Provider {
        name: "test".into(),
        api,
        base_url: url.into(),
        key,
        timeout: Duration::from_millis(500),
        max_retries: 2,
        models: IndexMap::new(),
    }
}

fn provider(server: &Server, api: Api) -> Arc<dyn Provider> {
    let key = KeySource::Cmd("printf sk-test".into());
    with_retry_base(
        &provider_config(api, &server.url, key),
        Duration::from_millis(10),
    )
    .unwrap()
}

fn request() -> Request {
    Request {
        model: "m-1".into(),
        system: "sys".into(),
        messages: vec![Message::user("hi")],
        tools: Vec::new(),
        max_output_tokens: None,
        temperature: None,
    }
}

async fn collect(provider: &Arc<dyn Provider>) -> Vec<Result<Event, ProviderError>> {
    provider
        .stream(request(), CancellationToken::new())
        .collect()
        .await
}

fn last_error(events: &[Result<Event, ProviderError>]) -> &ProviderError {
    match events.last() {
        Some(Err(e)) => e,
        other => panic!("no error at the end: {other:?}"),
    }
}

#[tokio::test]
async fn anthropic_stream() {
    let server = serve(vec![Reply::sse(ANTHROPIC_TEXT)]).await;
    let events = collect(&provider(&server, Api::Anthropic)).await;
    let text: String = events
        .iter()
        .filter_map(|e| match e {
            Ok(Event::TextDelta { text }) => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(text, "Hello, wörld!");
    assert_eq!(
        events.last(),
        Some(&Ok(Event::Done {
            stop_reason: StopReason::EndTurn
        }))
    );

    let seen = server.seen.lock().unwrap();
    let seen = seen.first().unwrap();
    assert_eq!(seen.path, "/v1/messages");
    assert_eq!(seen.header("x-api-key"), Some("sk-test"));
    assert_eq!(seen.header("anthropic-version"), Some("2023-06-01"));
    assert_eq!(seen.header("content-type"), Some("application/json"));
    assert_eq!(seen.body["model"], json!("m-1"));
    assert_eq!(seen.body["system"], json!("sys"));
}

#[tokio::test]
async fn openai_stream() {
    let server = serve(vec![Reply::sse(OPENAI_TOOLS)]).await;
    let events = collect(&provider(&server, Api::OpenAiCompatible)).await;
    let ends = events
        .iter()
        .filter(|e| matches!(e, Ok(Event::ToolCallEnd { .. })))
        .count();
    assert_eq!(ends, 2);
    let seen = server.seen.lock().unwrap();
    let seen = seen.first().unwrap();
    assert_eq!(seen.path, "/chat/completions");
    assert_eq!(seen.header("authorization"), Some("Bearer sk-test"));
    assert_eq!(
        seen.body["messages"][0],
        json!({"role": "system", "content": "sys"})
    );
}

#[tokio::test]
async fn rate_limit_is_retried() {
    let server = serve(vec![
        Reply {
            headers: vec![("retry-after", "0")],
            ..Reply::status(
                429,
                r#"{"error":{"type":"rate_limit_error","message":"slow down"}}"#,
            )
        },
        Reply::sse(ANTHROPIC_TEXT),
    ])
    .await;
    let events = collect(&provider(&server, Api::Anthropic)).await;
    assert!(events.iter().all(Result::is_ok), "{events:?}");
    assert_eq!(server.requests(), 2);
}

#[tokio::test]
async fn gives_up_after_max_retries() {
    let server = serve(vec![Reply::status(503, "busy"); 5]).await;
    let events = collect(&provider(&server, Api::Anthropic)).await;
    assert_eq!(events.len(), 1);
    let err = last_error(&events);
    assert_eq!(err.kind, ErrorKind::Overloaded);
    assert_eq!(err.message, "HTTP 503: busy");
    assert_eq!(server.requests(), 3, "first try and two retries");
}

#[tokio::test]
async fn auth_is_not_retried() {
    let server = serve(vec![Reply::status(
        401,
        r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error"}}"#,
    )])
    .await;
    let events = collect(&provider(&server, Api::OpenAiCompatible)).await;
    let err = last_error(&events);
    assert_eq!(err.kind, ErrorKind::Auth);
    assert!(err.message.contains("Incorrect API key"), "{}", err.message);
    assert!(!err.message.contains("sk-test"));
    assert_eq!(server.requests(), 1);
}

#[tokio::test]
async fn context_too_long() {
    let server = serve(vec![Reply::status(
        400,
        r#"{"type":"error","error":{"type":"invalid_request_error","message":"prompt is too long: 250000 tokens > 200000 maximum"}}"#,
    )])
    .await;
    let events = collect(&provider(&server, Api::Anthropic)).await;
    assert_eq!(last_error(&events).kind, ErrorKind::ContextTooLong);
}

#[tokio::test]
async fn cut_before_first_event_is_retried() {
    let cut = Reply::sse("event: message_start\ndata: {\"type\":\"mess");
    let server = serve(vec![cut, Reply::sse(ANTHROPIC_TEXT)]).await;
    let events = collect(&provider(&server, Api::Anthropic)).await;
    assert!(events.iter().all(Result::is_ok), "{events:?}");
    assert_eq!(server.requests(), 2);
}

#[tokio::test]
async fn cut_after_an_event_is_not_retried() {
    let half = ANTHROPIC_TEXT
        .split("event: content_block_stop")
        .next()
        .unwrap();
    let server = serve(vec![Reply::sse(half), Reply::sse(ANTHROPIC_TEXT)]).await;
    let events = collect(&provider(&server, Api::Anthropic)).await;
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Ok(Event::TextDelta { .. })))
    );
    assert_eq!(last_error(&events).kind, ErrorKind::Network);
    assert_eq!(server.requests(), 1);
}

#[tokio::test]
async fn silent_server_times_out() {
    let silent = Reply {
        silent: true,
        ..Reply::sse("")
    };
    let server = serve(vec![silent.clone(), silent.clone(), silent]).await;
    let start = Instant::now();
    let events = collect(&provider(&server, Api::Anthropic)).await;
    let err = last_error(&events);
    assert_eq!(err.kind, ErrorKind::Timeout);
    assert!(
        err.message.contains("waiting for the response"),
        "{}",
        err.message
    );
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn stalled_stream_times_out() {
    let first = ANTHROPIC_TEXT
        .split("event: content_block_start")
        .next()
        .unwrap();
    let server = serve(vec![Reply {
        stall: true,
        ..Reply::sse(first)
    }])
    .await;
    let events = collect(&provider(&server, Api::Anthropic)).await;
    let err = last_error(&events);
    assert_eq!(err.kind, ErrorKind::Timeout);
    assert!(err.message.contains("next part"), "{}", err.message);
}

#[tokio::test]
async fn slow_but_steady_stream_is_fine() {
    // Each gap is below the timeout, the whole answer is longer than it.
    let parts: Vec<(Duration, String)> = ANTHROPIC_TEXT
        .split_inclusive("\n\n")
        .map(|p| (Duration::from_millis(100), p.to_owned()))
        .collect();
    let server = serve(vec![Reply {
        parts,
        ..Reply::sse("")
    }])
    .await;
    let events = collect(&provider(&server, Api::Anthropic)).await;
    assert!(events.iter().all(Result::is_ok), "{events:?}");
}

#[tokio::test]
async fn cancel_during_stream() {
    let first = ANTHROPIC_TEXT
        .split("event: content_block_start")
        .next()
        .unwrap();
    let server = serve(vec![Reply {
        stall: true,
        ..Reply::sse(first)
    }])
    .await;
    let provider = provider(&server, Api::Anthropic);
    let cancel = CancellationToken::new();
    let mut stream = provider.stream(request(), cancel.clone());
    assert!(matches!(stream.next().await, Some(Ok(Event::Usage { .. }))));
    cancel.cancel();
    match stream.next().await {
        Some(Err(e)) => assert_eq!(e.kind, ErrorKind::Cancelled),
        other => panic!("{other:?}"),
    }
    assert!(stream.next().await.is_none());
}

#[tokio::test]
async fn cancel_during_retry_wait() {
    let server = serve(vec![Reply::status(503, "busy"); 3]).await;
    let config = provider_config(Api::Anthropic, &server.url, KeySource::None);
    let provider = with_retry_base(&config, Duration::from_secs(30)).unwrap();
    let cancel = CancellationToken::new();
    let stream = provider.stream(request(), cancel.clone());
    let start = Instant::now();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        cancel.cancel();
    });
    let events: Vec<_> = stream.collect().await;
    assert_eq!(last_error(&events).kind, ErrorKind::Cancelled);
    assert!(start.elapsed() < Duration::from_secs(5));
}

#[tokio::test]
async fn missing_key_is_auth_error() {
    let server = serve(vec![]).await;
    let key = KeySource::Env("CLAI_TEST_SURELY_UNSET_VARIABLE".into());
    let provider = from_config(&provider_config(Api::Anthropic, &server.url, key)).unwrap();
    let events = collect(&provider).await;
    let err = last_error(&events);
    assert_eq!(err.kind, ErrorKind::Auth);
    assert!(err.message.contains("CLAI_TEST_SURELY_UNSET_VARIABLE"));
    assert_eq!(server.requests(), 0);
}

#[tokio::test]
async fn unreachable_server_is_network_error() {
    // A port nobody listens on: bind, note the port, close.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let mut config = provider_config(Api::Anthropic, &url, KeySource::None);
    config.max_retries = 0;
    let provider = from_config(&config).unwrap();
    let events = collect(&provider).await;
    assert_eq!(last_error(&events).kind, ErrorKind::Network);
}

#[test]
fn outside_runtime_is_error_not_panic() {
    let config = provider_config(Api::Anthropic, "http://127.0.0.1:1", KeySource::None);
    let provider = from_config(&config).unwrap();
    let mut stream = provider.stream(request(), CancellationToken::new());
    let first = futures_util::FutureExt::now_or_never(stream.next());
    match first {
        Some(Some(Err(e))) => assert_eq!(e.kind, ErrorKind::Unsupported),
        other => panic!("{other:?}"),
    }
}
