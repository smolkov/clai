//! Questions the loop asks, their answers, and who answers them (see
//! `spec/sessions.md`, "Questions").

use futures_util::future::BoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::Tool;
use crate::permissions::{Proposal, Reason};
use crate::tools::Details;

/// The answers a question accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Choice {
    Allow,
    AllowAlways,
    Deny,
    DenyMessage,
    Cancel,
    Continue,
    Stop,
}

pub const PERMISSION_CHOICES: [Choice; 5] = [
    Choice::Allow,
    Choice::AllowAlways,
    Choice::Deny,
    Choice::DenyMessage,
    Choice::Cancel,
];
pub const LIMIT_CHOICES: [Choice; 3] = [Choice::Continue, Choice::Stop, Choice::Cancel];

/// Requests and tool calls a turn has used so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TurnInfo {
    /// Number of the turn in the session, from 1.
    pub number: u64,
    pub requests: u32,
    pub tool_calls: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Question {
    /// `q1`, `q2`, … in the session.
    pub id: String,
    /// Seconds since 1970 when it opened.
    pub opened: u64,
    pub turn: TurnInfo,
    /// One line: `run_command: cargo test parse`.
    pub summary: String,
    pub choices: Vec<Choice>,
    #[serde(flatten)]
    pub kind: QuestionKind,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QuestionKind {
    Permission(Box<Permission>),
    Limit(Limit),
}

/// A tool call that asks.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Permission {
    pub call: CallInfo,
    pub details: Details,
    pub reason: Reason,
    /// `reason` as text: `default for edit_file`.
    pub reason_text: String,
    /// Rule sets for "always", narrowest first.
    pub proposals: Vec<Proposal>,
    /// The model's text in the same answer before the call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CallInfo {
    pub tool: Tool,
    /// The call's id from the model.
    pub id: String,
    /// Exactly as the model sent them.
    pub arguments: Value,
}

/// A turn that used up its budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Limit {
    pub max_requests: u32,
    pub max_tool_calls: u32,
    /// Counts in the budget that is used up.
    pub requests: u32,
    pub tool_calls: u32,
}

/// An answer, as a client sends it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "choice", rename_all = "snake_case")]
pub enum Answer {
    Allow,
    /// Rules in the form of `[permissions.allow]` entries:
    /// `run_command(cargo test)`.
    AllowAlways {
        rules: Vec<String>,
    },
    Deny,
    DenyMessage {
        text: String,
    },
    Cancel,
    Continue,
    Stop,
}

impl Answer {
    pub fn choice(&self) -> Choice {
        match self {
            Answer::Allow => Choice::Allow,
            Answer::AllowAlways { .. } => Choice::AllowAlways,
            Answer::Deny => Choice::Deny,
            Answer::DenyMessage { .. } => Choice::DenyMessage,
            Answer::Cancel => Choice::Cancel,
            Answer::Continue => Choice::Continue,
            Answer::Stop => Choice::Stop,
        }
    }
}

/// What an [`Asker`] gives back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// An answer, and who gave it (`tui pts/3`, `clai chat`, `--yes`).
    Answer { answer: Answer, by: String },
    /// No one can answer: a permission question counts as denied, a limit
    /// question as "stop".
    NoOne,
}

/// How a question was closed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "closed", rename_all = "snake_case")]
pub enum Closed {
    Answered { answer: Answer, by: String },
    NoOne,
    Cancelled,
}

/// Gets decisions for the loop: the session asks its clients, `clai run`
/// asks on the terminal or answers by a policy.
pub trait Asker: Send + Sync {
    /// Waits for an answer. The loop stops waiting when the turn is
    /// cancelled, so this need not watch for that.
    fn ask<'a>(&'a self, question: &'a Question) -> BoxFuture<'a, Reply>;

    /// The answer was not accepted; the question stays open and is asked
    /// again.
    fn reject(&self, question: &Question, by: &str, message: &str);
}

/// Answers without asking anyone (`clai run` without a terminal, `--yes`,
/// `--no-ask`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyAsker {
    /// Permission questions count as denied.
    Deny,
    /// Permission questions are allowed (`--yes`).
    Allow,
}

impl Asker for PolicyAsker {
    fn ask<'a>(&'a self, question: &'a Question) -> BoxFuture<'a, Reply> {
        let reply = match (&question.kind, self) {
            (QuestionKind::Permission(_), PolicyAsker::Allow) => Reply::Answer {
                answer: Answer::Allow,
                by: "--yes".into(),
            },
            _ => Reply::NoOne,
        };
        Box::pin(async move { reply })
    }

    fn reject(&self, _question: &Question, _by: &str, message: &str) {
        tracing::warn!("policy answer rejected: {message}");
    }
}
