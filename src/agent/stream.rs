//! Collecting a streamed answer (see `spec/agent-loop.md`, "Streaming").

use futures_util::StreamExt;
use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::provider::{Block, ErrorKind, Event, EventStream, ProviderError, StopReason};

use super::events::{CallResult, CallState, EventSink, SessionEvent, ToolCallEvent};

/// A streamed answer.
#[derive(Debug, Default)]
pub struct Streamed {
    /// Text blocks and complete tool calls, in order.
    pub blocks: Vec<Block>,
    pub stop: Option<StopReason>,
    /// The last `Usage` of the request.
    pub usage: Option<(u64, u64)>,
    pub end: StreamEnd,
    /// A tool call was started but did not end; it is left out.
    pub cut_call: bool,
}

#[derive(Debug, Default)]
pub enum StreamEnd {
    #[default]
    Done,
    Cancelled,
    Failed(ProviderError),
}

impl Streamed {
    /// The answer without its tool calls, for an answer that broke off.
    pub fn text_only(&self) -> Vec<Block> {
        self.blocks
            .iter()
            .filter(|b| matches!(b, Block::Text { .. }))
            .cloned()
            .collect()
    }
}

/// A tool call that was started but has not ended.
struct Open {
    id: String,
    name: String,
}

/// Reads the stream to its end, emitting events as they arrive.
pub async fn collect(
    mut stream: EventStream,
    events: &EventSink,
    cancel: &CancellationToken,
) -> Streamed {
    let mut out = Streamed::default();
    let mut text = String::new();
    let mut open: Vec<Open> = Vec::new();
    // Calls that ended, by id: emitted, and dropped again if the answer
    // breaks off.
    let mut ended: Vec<(String, String)> = Vec::new();
    loop {
        let item = tokio::select! {
            biased;
            () = cancel.cancelled() => {
                out.end = StreamEnd::Cancelled;
                break;
            }
            item = stream.next() => item,
        };
        match item {
            None => {
                out.end =
                    StreamEnd::Failed(ProviderError::protocol("the answer ended without Done"));
                break;
            }
            Some(Err(e)) if e.kind == ErrorKind::Cancelled && cancel.is_cancelled() => {
                out.end = StreamEnd::Cancelled;
                break;
            }
            Some(Err(e)) => {
                out.end = StreamEnd::Failed(e);
                break;
            }
            Some(Ok(event)) => match event {
                Event::TextDelta { text: delta } => {
                    if !delta.is_empty() {
                        text.push_str(&delta);
                        events.emit(SessionEvent::TextDelta { text: delta });
                    }
                }
                Event::ToolCallStart { id, name } => {
                    finish_text(&mut text, &mut out.blocks, events);
                    events.emit(call_event(&id, &name, CallState::Pending, Value::Null));
                    open.push(Open { id, name });
                }
                Event::ToolCallArgsDelta { .. } => {}
                Event::ToolCallEnd { id, arguments } => {
                    finish_text(&mut text, &mut out.blocks, events);
                    // A call that did not start (a broken adapter) gets no
                    // name; prepare reports it as an unknown tool.
                    let name = match open.iter().position(|o| o.id == id) {
                        Some(i) => open.remove(i).name,
                        None => String::new(),
                    };
                    events.emit(call_event(
                        &id,
                        &name,
                        CallState::Pending,
                        arguments.clone(),
                    ));
                    ended.push((id.clone(), name.clone()));
                    out.blocks.push(Block::ToolCall {
                        id,
                        name,
                        arguments,
                    });
                }
                Event::Usage {
                    input_tokens,
                    output_tokens,
                } => out.usage = Some((input_tokens, output_tokens)),
                Event::Retry {
                    attempt,
                    max,
                    reason,
                } => events.emit(SessionEvent::Retry {
                    attempt,
                    max,
                    reason,
                }),
                Event::Done { stop_reason } => {
                    out.stop = Some(stop_reason);
                    break;
                }
            },
        }
    }
    finish_text(&mut text, &mut out.blocks, events);

    out.cut_call = !open.is_empty();
    // Calls that will not run get their final state now.
    let broken = !matches!(out.end, StreamEnd::Done);
    let (state, why) = match &out.end {
        StreamEnd::Cancelled => (CallState::Cancelled, "cancelled by the user".to_owned()),
        StreamEnd::Failed(e) => (
            CallState::Error,
            format!("not run: the answer broke off: {e}"),
        ),
        StreamEnd::Done if out.stop == Some(StopReason::MaxTokens) => (
            CallState::Error,
            "not run: the answer was cut at max_output_tokens".to_owned(),
        ),
        StreamEnd::Done => (CallState::Error, "not run: the call did not end".to_owned()),
    };
    let dropped = open
        .iter()
        .map(|o| (o.id.clone(), o.name.clone()))
        .chain(if broken { ended } else { Vec::new() });
    for (id, name) in dropped {
        let mut event = tool_call(&id, &name, state, Value::Null);
        event.result = Some(CallResult {
            text: why.clone(),
            is_error: true,
        });
        events.emit(SessionEvent::ToolCall(event));
    }
    out
}

fn finish_text(text: &mut String, blocks: &mut Vec<Block>, events: &EventSink) {
    if !text.is_empty() {
        let done = std::mem::take(text);
        events.emit(SessionEvent::TextDone { text: done.clone() });
        blocks.push(Block::Text { text: done });
    }
}

fn call_event(id: &str, name: &str, state: CallState, arguments: Value) -> SessionEvent {
    SessionEvent::ToolCall(tool_call(id, name, state, arguments))
}

fn tool_call(id: &str, name: &str, state: CallState, arguments: Value) -> ToolCallEvent {
    ToolCallEvent {
        id: id.to_owned(),
        name: name.to_owned(),
        state,
        summary: None,
        arguments,
        output: None,
        result: None,
    }
}
