//! The agent loop: one turn of a session (see `spec/agent-loop.md`).
//!
//! [`Agent::run_turn`] sends the history to the model, streams the answer,
//! checks and runs its tool calls, asks questions through an [`Asker`], and
//! repeats until the model answers without tool calls. It knows no
//! transport: everything it does goes out as [`SessionEvent`]s.

mod events;
mod history;
mod prompt;
mod question;
mod stream;

#[cfg(test)]
mod tests;

use std::sync::Arc;

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::config::{Action, Limits, Model, Rule};
use crate::provider::{Block, ErrorKind, Provider, ProviderError, Request, Role, StopReason};
use crate::tools::{self, Env, Prepared, PreparedCall, ToolOutput};

pub use events::{
    CallResult, CallState, EndReason, EventSink, SessionEvent, State, ToolCallEvent, TurnEnd,
};
pub use history::History;
pub use prompt::{system_prompt, today};
pub use question::{
    Answer, Asker, CallInfo, Choice, Closed, LIMIT_CHOICES, Limit, PERMISSION_CHOICES, Permission,
    PolicyAsker, Question, QuestionKind, Reply, TurnInfo,
};

use stream::{StreamEnd, Streamed};

/// The result for calls not run because the turn stopped at its limit.
const NOT_RUN_LIMIT: &str = "not run: the turn was stopped at its limit";
const CANCELLED: &str = "cancelled by the user";
const NO_ONE: &str = "denied: needs permission, and no one can answer";

/// What a session gives its agent.
pub struct Setup {
    pub provider: Arc<dyn Provider>,
    pub model: Model,
    pub limits: Limits,
    /// Permissions, session rules and workspace.
    pub env: Env,
    /// The system prompt; see [`system_prompt`].
    pub system: String,
}

/// The state of a session that turns work on.
pub struct Agent {
    provider: Arc<dyn Provider>,
    model: Model,
    limits: Limits,
    env: Env,
    system: String,
    history: History,
    turns: u64,
    questions: u64,
}

/// Requests and tool calls: in the current budget, and in the whole turn.
#[derive(Debug, Default)]
struct Budget {
    requests: u32,
    tool_calls: u32,
    total_requests: u32,
    total_tool_calls: u32,
    input_tokens: u64,
    output_tokens: u64,
}

/// What stops a turn before its model answers without tool calls.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stop {
    Cancelled,
    Limit,
}

impl Stop {
    fn reason(self) -> EndReason {
        match self {
            Stop::Cancelled => EndReason::Cancelled,
            Stop::Limit => EndReason::Limit,
        }
    }

    /// The result for calls not run because of this stop.
    fn not_run(self) -> (&'static str, CallState) {
        match self {
            Stop::Cancelled => (CANCELLED, CallState::Cancelled),
            Stop::Limit => (NOT_RUN_LIMIT, CallState::Cancelled),
        }
    }
}

/// One tool call of an answer.
struct CallIn {
    id: String,
    name: String,
    arguments: Value,
    /// The answer's text before it.
    text: Option<String>,
}

impl Agent {
    pub fn new(setup: Setup) -> Agent {
        Agent {
            provider: setup.provider,
            model: setup.model,
            limits: setup.limits,
            env: setup.env,
            system: setup.system,
            history: History::default(),
            turns: 0,
            questions: 0,
        }
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    /// The tool environment, with the session rules added so far.
    pub fn env(&self) -> &Env {
        &self.env
    }

    /// Runs one turn for a user message. Cancelling `cancel` ends it as
    /// soon as possible, with the history still valid.
    pub async fn run_turn(
        &mut self,
        message: String,
        events: &EventSink,
        asker: &dyn Asker,
        cancel: CancellationToken,
    ) -> TurnEnd {
        self.turns += 1;
        let mut budget = Budget::default();
        events.emit(SessionEvent::State {
            state: State::Working,
        });
        events.emit(SessionEvent::UserMessage {
            text: message.clone(),
        });
        self.history
            .push(Role::User, vec![Block::Text { text: message }]);
        let reason = self.turn(&mut budget, events, asker, &cancel).await;
        match reason {
            EndReason::Cancelled => events.notice("turn cancelled"),
            EndReason::Limit => events.notice("limit reached"),
            EndReason::Done | EndReason::Failed => {}
        }
        let end = TurnEnd {
            reason,
            requests: budget.total_requests,
            tool_calls: budget.total_tool_calls,
            input_tokens: budget.input_tokens,
            output_tokens: budget.output_tokens,
        };
        events.emit(SessionEvent::TurnEnd(end));
        events.emit(SessionEvent::State { state: State::Idle });
        end
    }

    async fn turn(
        &mut self,
        budget: &mut Budget,
        events: &EventSink,
        asker: &dyn Asker,
        cancel: &CancellationToken,
    ) -> EndReason {
        loop {
            if cancel.is_cancelled() {
                return EndReason::Cancelled;
            }
            if budget.requests >= self.limits.max_requests
                && let Some(stop) = self.limit(budget, events, asker, cancel).await
            {
                return stop.reason();
            }
            budget.requests += 1;
            budget.total_requests += 1;

            let request = Request {
                model: self.model.id.clone(),
                system: self.system.clone(),
                messages: self.history.messages().to_vec(),
                tools: tools::specs(),
                max_output_tokens: self.model.max_output_tokens,
                temperature: self.model.temperature,
            };
            let stream = self.provider.stream(request, cancel.child_token());
            let answer = stream::collect(stream, events, cancel).await;
            if let Some((input, output)) = answer.usage {
                budget.input_tokens += input;
                budget.output_tokens += output;
                events.emit(SessionEvent::Usage {
                    input_tokens: input,
                    output_tokens: output,
                    context_tokens: input + output,
                });
            }
            match &answer.end {
                StreamEnd::Done => {}
                StreamEnd::Cancelled => {
                    self.history.push(Role::Assistant, answer.text_only());
                    return EndReason::Cancelled;
                }
                StreamEnd::Failed(e) => {
                    self.history.push(Role::Assistant, answer.text_only());
                    events.emit(SessionEvent::Error {
                        message: self.error_message(e),
                    });
                    return EndReason::Failed;
                }
            }
            if answer.blocks.is_empty() {
                events.notice(if answer.cut_call {
                    "answer cut at max_output_tokens"
                } else {
                    "the model gave an empty answer"
                });
                return EndReason::Done;
            }
            let calls = calls_of(&answer);
            self.history.push(Role::Assistant, answer.blocks.clone());
            if calls.is_empty() {
                match answer.stop {
                    Some(StopReason::MaxTokens) => events.notice("answer cut at max_output_tokens"),
                    Some(StopReason::Other) => events.notice("the model stopped: other reason"),
                    _ => {}
                }
                return EndReason::Done;
            }
            if answer.cut_call {
                events.notice("answer cut at max_output_tokens; the last tool call is left out");
            }
            let (results, stop) = self.run_calls(calls, budget, events, asker, cancel).await;
            self.history.push(Role::User, results);
            if let Some(stop) = stop {
                return stop.reason();
            }
        }
    }

    fn error_message(&self, e: &ProviderError) -> String {
        match e.kind {
            ErrorKind::ContextTooLong => format!(
                "the conversation is too long for {} ({} tokens); start a new session",
                self.model.display_name,
                tools::group(usize::try_from(self.model.context_window).unwrap_or(usize::MAX))
            ),
            _ => e.message.clone(),
        }
    }

    /// Runs the calls of one answer in order; returns their results and
    /// why the turn stops, if it does.
    async fn run_calls(
        &mut self,
        calls: Vec<CallIn>,
        budget: &mut Budget,
        events: &EventSink,
        asker: &dyn Asker,
        cancel: &CancellationToken,
    ) -> (Vec<Block>, Option<Stop>) {
        let mut results = Vec::new();
        let mut stop = None;
        for call in calls {
            if stop.is_none() && cancel.is_cancelled() {
                stop = Some(Stop::Cancelled);
            }
            if stop.is_none() && budget.tool_calls >= self.limits.max_tool_calls {
                stop = self.limit(budget, events, asker, cancel).await;
            }
            let (output, state) = match stop {
                Some(stop) => {
                    let (text, state) = stop.not_run();
                    (ToolOutput::error(text), state)
                }
                None => {
                    budget.tool_calls += 1;
                    budget.total_tool_calls += 1;
                    let (output, state, call_stop) =
                        self.one_call(&call, budget, events, asker, cancel).await;
                    stop = call_stop;
                    (output, state)
                }
            };
            events.emit(SessionEvent::ToolCall(ToolCallEvent {
                id: call.id.clone(),
                name: call.name.clone(),
                state,
                summary: None,
                arguments: call.arguments.clone(),
                output: None,
                result: Some(CallResult {
                    text: output.full.clone(),
                    is_error: output.is_error,
                }),
            }));
            self.history.keep_full(&call.id, &output.text, output.full);
            results.push(Block::ToolResult {
                call_id: call.id,
                content: output.text,
                is_error: output.is_error,
            });
        }
        (results, stop)
    }

    /// Prepares, asks about and runs one call.
    async fn one_call(
        &mut self,
        call: &CallIn,
        budget: &Budget,
        events: &EventSink,
        asker: &dyn Asker,
        cancel: &CancellationToken,
    ) -> (ToolOutput, CallState, Option<Stop>) {
        let prepared = match tools::prepare(&self.env, &call.name, &call.arguments) {
            Prepared::Failed(output) => return (output, CallState::Error, None),
            Prepared::Denied(output) => return (output, CallState::Denied, None),
            Prepared::Ready(prepared) => prepared,
        };
        let summary = Some(prepared.summary.clone());
        if prepared.action() == Action::Ask {
            self.emit_call(events, call, CallState::Waiting, summary.clone(), None);
            match self
                .ask_permission(call, &prepared, budget, events, asker, cancel)
                .await
            {
                Decided::Run => {}
                Decided::Result(output, state) => return (output, state, None),
                Decided::Stop(stop) => {
                    let (text, state) = stop.not_run();
                    return (ToolOutput::error(text), state, Some(stop));
                }
            }
        }
        self.emit_call(events, call, CallState::Running, summary, None);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let env = self.env.clone();
        let run = prepared.run_with_progress(&env, cancel.clone(), Some(tx));
        let forward = async {
            while let Some(text) = rx.recv().await {
                self.emit_call(events, call, CallState::Running, None, Some(text));
            }
        };
        let (output, ()) = tokio::join!(run, forward);
        if cancel.is_cancelled() {
            return (output, CallState::Cancelled, Some(Stop::Cancelled));
        }
        let state = if output.is_error {
            CallState::Error
        } else {
            CallState::Done
        };
        (output, state, None)
    }

    fn emit_call(
        &self,
        events: &EventSink,
        call: &CallIn,
        state: CallState,
        summary: Option<String>,
        output: Option<String>,
    ) {
        events.emit(SessionEvent::ToolCall(ToolCallEvent {
            id: call.id.clone(),
            name: call.name.clone(),
            state,
            summary,
            arguments: call.arguments.clone(),
            output,
            result: None,
        }));
    }

    /// Opens a permission question and waits for a valid answer.
    async fn ask_permission(
        &mut self,
        call: &CallIn,
        prepared: &PreparedCall,
        budget: &Budget,
        events: &EventSink,
        asker: &dyn Asker,
        cancel: &CancellationToken,
    ) -> Decided {
        let question = self.question(
            budget,
            format!("{}: {}", prepared.tool, prepared.summary),
            &PERMISSION_CHOICES,
            QuestionKind::Permission(Box::new(Permission {
                call: CallInfo {
                    tool: prepared.tool,
                    id: call.id.clone(),
                    arguments: call.arguments.clone(),
                },
                details: prepared.details.clone(),
                reason: prepared.decision.reason.clone(),
                reason_text: prepared.decision.reason.to_string(),
                proposals: prepared.proposals.clone(),
                text: call.text.clone(),
            })),
        );
        let validate = |answer: &Answer| -> Result<Option<Vec<Rule>>, String> {
            match answer {
                Answer::AllowAlways { rules } => prepared.accept_rules(&self.env, rules).map(Some),
                Answer::DenyMessage { text } if text.trim().is_empty() => {
                    Err("the message is empty".into())
                }
                _ => Ok(None),
            }
        };
        let Some((reply, rules)) = wait(&question, events, asker, cancel, validate).await else {
            return Decided::Stop(Stop::Cancelled);
        };
        let answer = match reply {
            Reply::NoOne => return Decided::Result(ToolOutput::error(NO_ONE), CallState::Denied),
            Reply::Answer { answer, .. } => answer,
        };
        match answer {
            Answer::Allow => Decided::Run,
            Answer::AllowAlways { .. } => {
                let rules = rules.unwrap_or_default();
                let texts: Vec<String> = rules.iter().map(ToString::to_string).collect();
                let mut session = (*self.env.session).clone();
                session.add(rules);
                self.env.session = Arc::new(session);
                events.notice(format!("rules added: {}", texts.join(", ")));
                Decided::Run
            }
            Answer::Deny => {
                Decided::Result(ToolOutput::error("denied by the user"), CallState::Denied)
            }
            Answer::DenyMessage { text } => Decided::Result(
                ToolOutput::error(format!("denied by the user: {}", text.trim())),
                CallState::Denied,
            ),
            Answer::Cancel => Decided::Stop(Stop::Cancelled),
            // Not among the choices; `wait` rejects them.
            Answer::Continue | Answer::Stop => Decided::Stop(Stop::Cancelled),
        }
    }

    /// Asks the limit question; `None` means continue with a new budget.
    async fn limit(
        &mut self,
        budget: &mut Budget,
        events: &EventSink,
        asker: &dyn Asker,
        cancel: &CancellationToken,
    ) -> Option<Stop> {
        let limit = Limit {
            max_requests: self.limits.max_requests,
            max_tool_calls: self.limits.max_tool_calls,
            requests: budget.requests,
            tool_calls: budget.tool_calls,
        };
        let question = self.question(
            budget,
            format!(
                "turn limit reached ({} requests, {} tool calls)",
                budget.requests, budget.tool_calls
            ),
            &LIMIT_CHOICES,
            QuestionKind::Limit(limit),
        );
        let reply = wait(&question, events, asker, cancel, |_| Ok(())).await;
        match reply {
            None => Some(Stop::Cancelled),
            Some((Reply::NoOne, ())) => Some(Stop::Limit),
            Some((Reply::Answer { answer, .. }, ())) => match answer {
                Answer::Continue => {
                    budget.requests = 0;
                    budget.tool_calls = 0;
                    None
                }
                Answer::Cancel => Some(Stop::Cancelled),
                _ => Some(Stop::Limit),
            },
        }
    }

    fn question(
        &mut self,
        budget: &Budget,
        summary: String,
        choices: &[Choice],
        kind: QuestionKind,
    ) -> Question {
        self.questions += 1;
        Question {
            id: format!("q{}", self.questions),
            opened: prompt::now_secs(),
            turn: TurnInfo {
                number: self.turns,
                requests: budget.total_requests,
                tool_calls: budget.total_tool_calls,
            },
            summary,
            choices: choices.to_vec(),
            kind,
        }
    }
}

/// What the answer to a permission question decided.
enum Decided {
    Run,
    Result(ToolOutput, CallState),
    Stop(Stop),
}

/// Emits a question and waits until it has a valid answer, the asker says
/// no one can answer, or the turn is cancelled (`None`). `validate` checks
/// what the choice alone cannot.
async fn wait<T>(
    question: &Question,
    events: &EventSink,
    asker: &dyn Asker,
    cancel: &CancellationToken,
    validate: impl Fn(&Answer) -> Result<T, String>,
) -> Option<(Reply, T)>
where
    T: Default,
{
    events.emit(SessionEvent::State {
        state: State::Waiting,
    });
    events.emit(SessionEvent::Question(Box::new(question.clone())));
    let result = loop {
        let reply = tokio::select! {
            biased;
            () = cancel.cancelled() => break None,
            reply = asker.ask(question) => reply,
        };
        let checked = match &reply {
            Reply::NoOne => Ok(T::default()),
            Reply::Answer { answer, by } => {
                let checked = if question.choices.contains(&answer.choice()) {
                    validate(answer)
                } else {
                    Err(format!(
                        "`{}` is not an answer to this question",
                        choice_name(answer.choice())
                    ))
                };
                if let Err(message) = &checked {
                    asker.reject(question, by, message);
                }
                checked
            }
        };
        if let Ok(value) = checked {
            break Some((reply, value));
        }
    };
    let closed = match &result {
        None => Closed::Cancelled,
        Some((Reply::NoOne, _)) => Closed::NoOne,
        Some((Reply::Answer { answer, by }, _)) => Closed::Answered {
            answer: answer.clone(),
            by: by.clone(),
        },
    };
    events.emit(SessionEvent::QuestionClosed {
        id: question.id.clone(),
        closed,
    });
    events.emit(SessionEvent::State {
        state: State::Working,
    });
    result
}

fn choice_name(choice: Choice) -> String {
    serde_json::to_value(choice)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// The tool calls of an answer, each with the text before it.
fn calls_of(answer: &Streamed) -> Vec<CallIn> {
    let mut text = String::new();
    let mut calls = Vec::new();
    for block in &answer.blocks {
        match block {
            Block::Text { text: t } => {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(t);
            }
            Block::ToolCall {
                id,
                name,
                arguments,
            } => calls.push(CallIn {
                id: id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
                text: (!text.is_empty()).then(|| text.clone()),
            }),
            Block::ToolResult { .. } => {}
        }
    }
    calls
}
