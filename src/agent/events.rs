//! Session events the agent loop emits (see `spec/agent-loop.md`,
//! "Events").

use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc;

use super::question::{Closed, Question};

/// What a session is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Idle,
    Working,
    Waiting,
    /// Only in the session list: saved on disk, not loaded; any command
    /// that names it resumes it.
    Saved,
}

/// The state of a tool call.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallState {
    /// Streamed by the model, not checked yet.
    Pending,
    /// A permission question is open.
    Waiting,
    Running,
    Done,
    Error,
    Denied,
    Cancelled,
}

impl CallState {
    pub fn is_final(self) -> bool {
        matches!(
            self,
            CallState::Done | CallState::Error | CallState::Denied | CallState::Cancelled
        )
    }
}

/// The result of a tool call, as shown to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CallResult {
    /// The whole result, up to 1 MiB.
    pub text: String,
    pub is_error: bool,
}

/// One change of a tool call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCallEvent {
    /// The call's id from the model.
    pub id: String,
    pub name: String,
    pub state: CallState,
    /// One line, once the call is prepared: `src/main.rs (lines 1–40)`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// `null` until the model has sent them.
    pub arguments: Value,
    /// New output of a running command.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// Set in the final state.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<CallResult>,
}

/// How a turn ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EndReason {
    Done,
    Cancelled,
    Limit,
    Failed,
}

/// The end of a turn, with what it used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnEnd {
    pub reason: EndReason,
    pub requests: u32,
    pub tool_calls: u32,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SessionEvent {
    State {
        state: State,
    },
    UserMessage {
        text: String,
    },
    TextDelta {
        text: String,
    },
    /// The end of a text block, with its whole text.
    TextDone {
        text: String,
    },
    ToolCall(ToolCallEvent),
    Question(Box<Question>),
    QuestionClosed {
        id: String,
        #[serde(flatten)]
        closed: Closed,
    },
    Notice {
        text: String,
    },
    Usage {
        input_tokens: u64,
        output_tokens: u64,
        /// Input plus output tokens of the last request.
        context_tokens: u64,
    },
    Error {
        message: String,
    },
    TurnEnd(TurnEnd),
    /// The session's model changed; applies from the next request.
    ModelChanged {
        /// `provider/model`.
        model: String,
    },
    Renamed {
        name: String,
    },
    /// The session ended: `killed`, `server stopped`, `failed`.
    Ended {
        reason: String,
    },
}

/// Where the loop sends its events. Sending never blocks for long; events
/// for a receiver that is gone are dropped.
#[derive(Clone)]
pub struct EventSink(Target);

#[derive(Clone)]
enum Target {
    Channel(mpsc::UnboundedSender<SessionEvent>),
    /// Called at once, in the loop's task: for output that must stay in
    /// order with what an asker writes (`clai run`).
    Call(Arc<dyn Fn(SessionEvent) + Send + Sync>),
}

impl fmt::Debug for EventSink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("EventSink")
    }
}

impl EventSink {
    pub fn channel() -> (EventSink, mpsc::UnboundedReceiver<SessionEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (EventSink(Target::Channel(tx)), rx)
    }

    /// Calls `f` for each event, as it is emitted. `f` must be quick.
    pub fn call(f: impl Fn(SessionEvent) + Send + Sync + 'static) -> EventSink {
        EventSink(Target::Call(Arc::new(f)))
    }

    pub fn emit(&self, event: SessionEvent) {
        match &self.0 {
            Target::Channel(tx) => {
                let _ = tx.send(event);
            }
            Target::Call(f) => f(event),
        }
    }

    pub fn notice(&self, text: impl Into<String>) {
        self.emit(SessionEvent::Notice { text: text.into() });
    }
}
