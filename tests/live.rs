//! Requests against real APIs. Not run by default:
//!
//! ```sh
//! ANTHROPIC_API_KEY=... cargo test --test live -- --ignored
//! CLAI_LIVE_OPENAI_URL=http://localhost:11434/v1 CLAI_LIVE_OPENAI_MODEL=qwen3-coder:30b \
//!     cargo test --test live -- --ignored
//! ```
//!
//! `CLAI_LIVE_ANTHROPIC_MODEL` picks the Anthropic model (default
//! `claude-haiku-5-5`); `CLAI_LIVE_OPENAI_KEY_ENV` names the variable with
//! the key for the openai-compatible server, if it needs one. A test whose
//! variables are not set prints why and passes.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use indexmap::IndexMap;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use clai::config::{self, Api, KeySource};
use clai::provider::{
    Block, Event, Message, Provider, Request, Role, StopReason, ToolSpec, from_config,
};

fn provider(api: Api, base_url: String, key: KeySource) -> Arc<dyn Provider> {
    from_config(&config::Provider {
        name: "live".into(),
        api,
        base_url,
        key,
        timeout: Duration::from_secs(120),
        max_retries: 2,
        models: IndexMap::new(),
    })
    .unwrap()
}

fn anthropic() -> Option<(Arc<dyn Provider>, String)> {
    if std::env::var("ANTHROPIC_API_KEY").is_err() {
        eprintln!("skipped: ANTHROPIC_API_KEY is not set");
        return None;
    }
    let model = std::env::var("CLAI_LIVE_ANTHROPIC_MODEL").unwrap_or("claude-haiku-5-5".into());
    let key = KeySource::Env("ANTHROPIC_API_KEY".into());
    Some((
        provider(Api::Anthropic, "https://api.anthropic.com".into(), key),
        model,
    ))
}

fn openai() -> Option<(Arc<dyn Provider>, String)> {
    let (Ok(url), Ok(model)) = (
        std::env::var("CLAI_LIVE_OPENAI_URL"),
        std::env::var("CLAI_LIVE_OPENAI_MODEL"),
    ) else {
        eprintln!("skipped: CLAI_LIVE_OPENAI_URL or CLAI_LIVE_OPENAI_MODEL is not set");
        return None;
    };
    let key = std::env::var("CLAI_LIVE_OPENAI_KEY_ENV").map_or(KeySource::None, KeySource::Env);
    Some((provider(Api::OpenAiCompatible, url, key), model))
}

fn request(model: &str, messages: Vec<Message>, tools: Vec<ToolSpec>) -> Request {
    Request {
        model: model.into(),
        system: "You are a test. Answer briefly.".into(),
        messages,
        tools,
        max_output_tokens: Some(256),
        temperature: Some(0.0),
    }
}

async fn run(provider: &Arc<dyn Provider>, request: Request) -> Vec<Event> {
    provider
        .stream(request, CancellationToken::new())
        .map(|e| e.unwrap_or_else(|e| panic!("{:?}: {}", e.kind, e.message)))
        .collect()
        .await
}

async fn text_answer(provider: Arc<dyn Provider>, model: String) {
    let events = run(
        &provider,
        request(&model, vec![Message::user("Say the word: pong")], vec![]),
    )
    .await;
    let text: String = events
        .iter()
        .filter_map(|e| match e {
            Event::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(text.to_lowercase().contains("pong"), "{text}");
    assert_eq!(
        events.last(),
        Some(&Event::Done {
            stop_reason: StopReason::EndTurn
        })
    );
    assert!(events.iter().any(|e| matches!(e, Event::Usage { .. })));
}

async fn tool_round_trip(provider: Arc<dyn Provider>, model: String) {
    let tool = ToolSpec {
        name: "get_number".into(),
        description: "Returns the secret number for a name.".into(),
        input_schema: json!({
            "type": "object",
            "properties": {"name": {"type": "string"}},
            "required": ["name"],
        }),
    };
    let ask =
        Message::user("Use the get_number tool for the name \"blue\", then tell me the number.");
    let events = run(
        &provider,
        request(&model, vec![ask.clone()], vec![tool.clone()]),
    )
    .await;
    let calls: Vec<(String, serde_json::Value)> = events
        .iter()
        .filter_map(|e| match e {
            Event::ToolCallEnd { id, arguments } => Some((id.clone(), arguments.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        events.last(),
        Some(&Event::Done {
            stop_reason: StopReason::ToolUse
        }),
        "{events:?}"
    );
    let (id, arguments) = calls.first().unwrap().clone();
    assert_eq!(arguments["name"], json!("blue"));

    let answer = Message {
        role: Role::Assistant,
        content: vec![Block::ToolCall {
            id: id.clone(),
            name: "get_number".into(),
            arguments,
        }],
    };
    let result = Message {
        role: Role::User,
        content: vec![Block::ToolResult {
            call_id: id,
            content: "4711".into(),
            is_error: false,
        }],
    };
    let events = run(
        &provider,
        request(&model, vec![ask, answer, result], vec![tool]),
    )
    .await;
    let text: String = events
        .iter()
        .filter_map(|e| match e {
            Event::TextDelta { text } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(text.contains("4711"), "{text}");
}

#[tokio::test]
#[ignore = "needs ANTHROPIC_API_KEY and network"]
async fn anthropic_text() {
    if let Some((p, m)) = anthropic() {
        text_answer(p, m).await;
    }
}

#[tokio::test]
#[ignore = "needs ANTHROPIC_API_KEY and network"]
async fn anthropic_tools() {
    if let Some((p, m)) = anthropic() {
        tool_round_trip(p, m).await;
    }
}

#[tokio::test]
#[ignore = "needs CLAI_LIVE_OPENAI_URL and CLAI_LIVE_OPENAI_MODEL"]
async fn openai_text() {
    if let Some((p, m)) = openai() {
        text_answer(p, m).await;
    }
}

#[tokio::test]
#[ignore = "needs CLAI_LIVE_OPENAI_URL and CLAI_LIVE_OPENAI_MODEL"]
async fn openai_tools() {
    if let Some((p, m)) = openai() {
        tool_round_trip(p, m).await;
    }
}

/// TLS and the HTTP stack work: without a key the API answers 401.
#[tokio::test]
#[ignore = "needs network"]
async fn tls_reaches_anthropic() {
    let provider = provider(
        Api::Anthropic,
        "https://api.anthropic.com".into(),
        KeySource::None,
    );
    let events: Vec<_> = provider
        .stream(
            request("claude-haiku-5-5", vec![Message::user("hi")], vec![]),
            CancellationToken::new(),
        )
        .collect()
        .await;
    match events.as_slice() {
        [Err(e)] => assert_eq!(e.kind, clai::provider::ErrorKind::Auth, "{}", e.message),
        other => panic!("{other:?}"),
    }
}
