//! The agent loop: one turn of a session (see `spec/agent-loop.md`).
//!
//! [`Agent::run_turn`] sends the history to the model, streams the answer,
//! checks and runs its tool calls, asks questions through an [`Asker`], and
//! repeats until the model answers without tool calls. It knows no
//! transport: everything it does goes out as [`SessionEvent`]s.

mod events;
mod history;
mod input;
mod prompt;
mod question;
mod stream;

#[cfg(test)]
mod tests;

use std::collections::BTreeSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio_util::sync::CancellationToken;

use crate::config::{Action, HistorySettings, Limits, Model, Rule};
use crate::provider::{Block, ErrorKind, Provider, ProviderError, Request, Role, StopReason};
use crate::tools::report::{self, Reported};
use crate::tools::{self, Env, Prepared, PreparedCall, ToolOutput, ask_user};

pub use events::{
    CallResult, CallState, EndReason, EventSink, SessionEvent, State, ToolCallEvent, TurnEnd,
    active_skills, retry_text,
};
pub use history::{History, INTERRUPTED, Journal, Record};
pub use input::UserInput;
pub use prompt::{date_of, now_ms, now_secs, system_prompt, today, with_lsp, with_skills};
pub use question::{
    ASK_USER_CHOICES, Answer, AskCall, AskUser, Asker, CallInfo, Choice, Closed, LIMIT_CHOICES,
    LSP_START_CHOICES, Limit, PERMISSION_CHOICES, Permission, PolicyAsker, Question, QuestionKind,
    Reply, SessionRef, TurnInfo,
};

use crate::secrets::Redactor;
use crate::sync::lock;
use stream::{StreamEnd, Streamed};

/// The result for calls not run because the turn stopped at its limit.
const NOT_RUN_LIMIT: &str = "not run: the turn was stopped at its limit";
const CANCELLED: &str = "cancelled by the user";
const NO_ONE: &str = "denied: needs permission, and no one can answer";
const LSP_NOT_STARTED: &str = "language server not started: denied by the user";
/// Added to the result of a command that holds a placeholder (RFC 0030).
const COMMAND_PLACEHOLDER: &str = "\nrun_command does not fill in secrets; use the environment \
                                   variable that holds the value instead";
/// Command output without a newline held back at most, in bytes, before
/// it is sent on with its secrets hidden.
const PROGRESS_HOLD: usize = 4096;
/// The bullet about placeholders, for the system prompt.
const SECRETS_BULLET: &str = "- Secrets in files, output and messages are shown as \
     [secret: KIND #ID]. Never ask for their values. You may write a placeholder into a file \
     you edit and the real value is kept; in commands, use the environment variable instead.\n";

/// The prompt with the bullet about placeholders, before the one about
/// focused changes.
fn with_secrets(system: &str) -> String {
    match system.find("- Keep changes focused") {
        Some(at) => format!(
            "{}{SECRETS_BULLET}{}",
            system.get(..at).unwrap_or(""),
            system.get(at..).unwrap_or("")
        ),
        None => format!("{system}\n{SECRETS_BULLET}"),
    }
}

/// The details of a call with the secrets in their strings hidden.
fn hide_in_details(secrets: &Redactor, details: &tools::Details) -> tools::Details {
    serde_json::to_value(details)
        .ok()
        .map(|value| secrets.redact_json(&value))
        .and_then(|value| serde_json::from_value(value).ok())
        .unwrap_or_else(|| details.clone())
}

/// What a session gives its agent.
pub struct Setup {
    pub provider: Arc<dyn Provider>,
    pub model: Model,
    /// `provider/model`, for the `usage` events; empty when not known.
    pub model_name: String,
    pub limits: Limits,
    /// Which old tool results go to the model as placeholders.
    pub history: HistorySettings,
    /// Permissions, session rules and workspace.
    pub env: Env,
    /// The system prompt; see [`system_prompt`].
    pub system: String,
    /// Someone can answer questions from the model (`ask_user`): a session
    /// on the server, `codeagent run` on a terminal.
    pub can_ask: bool,
    /// Hides secrets from the model (RFC 0030); `None` with `[secrets]
    /// redact = false`.
    pub secrets: Option<Arc<Redactor>>,
}

/// The provider and model of a session, with its `provider/model` name. The
/// session can change them while a turn runs; the change applies from the
/// next request.
#[derive(Clone)]
pub struct ModelHandle(Arc<Mutex<Current>>);

#[derive(Clone)]
struct Current {
    provider: Arc<dyn Provider>,
    model: Model,
    /// `provider/model`; empty when not known.
    name: String,
}

impl ModelHandle {
    pub fn new(provider: Arc<dyn Provider>, model: Model, name: String) -> ModelHandle {
        ModelHandle(Arc::new(Mutex::new(Current {
            provider,
            model,
            name,
        })))
    }

    pub fn get(&self) -> (Arc<dyn Provider>, Model) {
        let current = lock(&self.0);
        (current.provider.clone(), current.model.clone())
    }

    /// The provider, the model and its `provider/model` name, read at once.
    fn get_named(&self) -> (Arc<dyn Provider>, Model, String) {
        let current = lock(&self.0).clone();
        (current.provider, current.model, current.name)
    }

    pub fn set(&self, provider: Arc<dyn Provider>, model: Model, name: String) {
        *lock(&self.0) = Current {
            provider,
            model,
            name,
        };
    }
}

/// The active skills of a session (RFC 0023). The session changes them
/// while a turn may run; the agent takes the change before its next model
/// request.
#[derive(Clone, Default)]
pub struct SkillsHandle(Arc<Mutex<(u64, Vec<crate::skills::Active>)>>);

impl SkillsHandle {
    pub fn set(&self, skills: Vec<crate::skills::Active>) {
        let mut current = lock(&self.0);
        current.0 += 1;
        current.1 = skills;
    }

    pub fn get(&self) -> Vec<crate::skills::Active> {
        lock(&self.0).1.clone()
    }

    /// The skills, if they changed after version `seen`, with the new
    /// version.
    fn changed_since(&self, seen: u64) -> Option<(u64, Vec<crate::skills::Active>)> {
        let current = lock(&self.0);
        (current.0 != seen).then(|| (current.0, current.1.clone()))
    }
}

/// Why the session cancels a turn, if not because of the user: the calls
/// not run and the notice say so. Read once when the turn ends.
#[derive(Clone, Default)]
pub struct CancelReason(Arc<Mutex<Option<String>>>);

impl CancelReason {
    pub fn set(&self, why: impl Into<String>) {
        *lock(&self.0) = Some(why.into());
    }

    fn get(&self) -> Option<String> {
        lock(&self.0).clone()
    }

    fn take(&self) -> Option<String> {
        lock(&self.0).take()
    }
}

/// The state of a session that turns work on.
pub struct Agent {
    cancel_reason: CancelReason,
    model: ModelHandle,
    limits: Limits,
    history_settings: HistorySettings,
    env: Env,
    system: String,
    /// The prompt and the workspace without skills.
    base_system: String,
    base_workspace: Arc<crate::permissions::Workspace>,
    skills: SkillsHandle,
    /// The version of `skills` applied.
    skills_seen: u64,
    can_ask: bool,
    history: History,
    turns: u64,
    /// The last question number; shared by all sessions of a server.
    questions: Arc<AtomicU64>,
    journal: Option<Journal>,
    /// The model gets the `report` tool (`codeagent run --report`).
    offers_report: bool,
    /// The last valid `report` call.
    report: Option<Reported>,
    secrets: Option<Arc<Redactor>>,
    /// Models (`provider/model`) that refused an image in this session; they
    /// get the text note in place of images from then on.
    refused_images: BTreeSet<String>,
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
    cache_read_tokens: u64,
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
    fn not_run(self, why: Option<&str>) -> (String, CallState) {
        if let (Stop::Cancelled, Some(why)) = (self, why) {
            return (format!("not run: {why}"), CallState::Cancelled);
        }
        let (text, state) = self.not_run_text();
        (text.to_owned(), state)
    }

    fn not_run_text(self) -> (&'static str, CallState) {
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
        let system = match setup.env.lsp.as_deref().filter(|l| l.offered()) {
            Some(lsp) => with_lsp(&setup.system, &lsp.languages()),
            None => setup.system,
        };
        let system = match &setup.secrets {
            Some(secrets) => with_secrets(&secrets.hide(&system)),
            None => system,
        };
        Agent {
            cancel_reason: CancelReason::default(),
            model: ModelHandle::new(setup.provider, setup.model, setup.model_name),
            limits: setup.limits,
            history_settings: setup.history,
            base_workspace: setup.env.workspace.clone(),
            env: setup.env,
            base_system: system.clone(),
            system,
            skills: SkillsHandle::default(),
            skills_seen: 0,
            can_ask: setup.can_ask,
            history: History::default(),
            turns: 0,
            questions: Arc::new(AtomicU64::new(0)),
            journal: None,
            offers_report: false,
            report: None,
            secrets: setup.secrets,
            refused_images: BTreeSet::new(),
        }
    }

    /// Reports every later change of the history and session rules.
    pub fn set_journal(&mut self, journal: Journal) {
        self.journal = Some(journal);
    }

    /// Restores a saved history: messages, full outputs, session rules and
    /// the number of turns. Rules that no longer parse are left out; the
    /// returned texts say which.
    pub fn restore(
        &mut self,
        records: Vec<Record>,
        turns: u64,
        home: Option<&std::path::Path>,
    ) -> Vec<String> {
        let mut problems = Vec::new();
        let mut rules = Vec::new();
        for record in records {
            match record {
                Record::Message(message) => self.history.push(message.role, message.content),
                Record::Full { call_id, text } => {
                    self.history.keep_full(&call_id, "", text);
                }
                Record::Rules { rules: texts } => {
                    for text in texts {
                        match Rule::parse_session(&text, home) {
                            Ok(rule) => rules.push(rule),
                            Err(e) => problems.push(format!("saved rule left out: {e}")),
                        }
                    }
                }
            }
        }
        if !rules.is_empty() {
            let mut session = (*self.env.session).clone();
            session.add(rules);
            self.env.session = Arc::new(session);
        }
        self.turns = turns;
        problems
    }

    /// Gives every tool call without a result the result
    /// [`INTERRUPTED`]; returns whether there were any.
    pub fn repair(&mut self) -> bool {
        let results = self.history.missing_results();
        if results.is_empty() {
            return false;
        }
        self.push(Role::User, results);
        true
    }

    fn journal(&self, record: Record) {
        if let Some(journal) = &self.journal {
            journal(record);
        }
    }

    fn push(&mut self, role: Role, content: Vec<Block>) {
        if content.is_empty() {
            return;
        }
        self.journal(Record::Message(crate::provider::Message {
            role,
            content: content.clone(),
        }));
        self.history.push(role, content);
    }

    /// Numbers questions from a counter shared with other agents, so that
    /// question ids are unique in the server.
    pub fn share_question_ids(&mut self, counter: Arc<AtomicU64>) {
        self.questions = counter;
    }

    /// The handle to give a reason before cancelling a turn.
    pub fn cancel_reason(&self) -> CancelReason {
        self.cancel_reason.clone()
    }

    /// The handle to change the provider and model.
    pub fn model(&self) -> ModelHandle {
        self.model.clone()
    }

    /// What hides secrets from the model in this session.
    pub fn secrets(&self) -> Option<Arc<Redactor>> {
        self.secrets.clone()
    }

    /// The handle to change the active skills.
    pub fn skills(&self) -> SkillsHandle {
        self.skills.clone()
    }

    /// The system prompt the next request sends, with the active skills.
    pub fn system(&mut self) -> &str {
        self.apply_skills();
        &self.system
    }

    /// Takes a change of the active skills: the prompt gets their section,
    /// the workspace their directories, read only.
    fn apply_skills(&mut self) {
        let Some((seen, skills)) = self.skills.changed_since(self.skills_seen) else {
            return;
        };
        self.skills_seen = seen;
        let dirs: Vec<std::path::PathBuf> = skills.iter().map(|s| s.dir.clone()).collect();
        let workspace = if dirs.is_empty() {
            self.base_workspace.clone()
        } else {
            Arc::new(self.base_workspace.with_read_dirs(&dirs))
        };
        self.system = with_skills(&self.base_system, &workspace, &skills);
        if let Some(secrets) = &self.secrets {
            self.system = secrets.hide(&self.system);
        }
        self.env.workspace = workspace;
    }

    /// Offers the `report` tool from the next request on (RFC 0027).
    pub fn offer_report(&mut self) {
        self.offers_report = true;
    }

    /// What the last valid `report` call handed in.
    pub fn report(&self) -> Option<&Reported> {
        self.report.as_ref()
    }

    #[cfg(test)]
    pub(crate) fn set_history_settings(&mut self, settings: HistorySettings) {
        self.history_settings = settings;
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
        message: impl Into<UserInput>,
        events: &EventSink,
        asker: &dyn Asker,
        cancel: CancellationToken,
    ) -> TurnEnd {
        let UserInput { text, images } = message.into();
        self.turns += 1;
        if let Some(lsp) = &self.env.lsp {
            lsp.new_turn();
        }
        // Made at the first turn, again when it went missing (RFC 0029).
        if let Some(scratch) = self.env.workspace.scratch()
            && let Err(e) = scratch.prepare()
        {
            tracing::warn!("scratch directory: {e}");
            events.emit(SessionEvent::Notice {
                text: format!("cannot make the scratch directory: {e}"),
            });
        }
        let mut budget = Budget::default();
        events.emit(SessionEvent::State {
            state: State::Working,
        });
        let input = UserInput {
            text: self.hide_in_message(text, events),
            images,
        };
        let had_images = has_images(self.history.messages());
        events.emit(SessionEvent::UserMessage {
            text: input.text.clone(),
            images: input.images.clone(),
        });
        let (_, _, model_name) = self.model.get_named();
        if !input.images.is_empty() && self.refused_images.contains(&model_name) {
            events.notice(format!(
                "{model_name} refused images earlier in this session; they are sent as notes"
            ));
        } else if !input.images.is_empty() && !had_images {
            // Secret hiding works on text only; said once per session.
            events.notice("images are sent as they are; secrets in them are not hidden");
        }
        self.push(Role::User, input.blocks());
        let reason = self.turn(&mut budget, events, asker, &cancel).await;
        match reason {
            EndReason::Cancelled => match self.cancel_reason.take() {
                Some(why) => events.notice(format!("turn cancelled: {why}")),
                None => events.notice("turn cancelled"),
            },
            EndReason::Limit => events.notice("limit reached"),
            EndReason::Done | EndReason::Failed => {}
        }
        let end = TurnEnd {
            reason,
            requests: budget.total_requests,
            tool_calls: budget.total_tool_calls,
            input_tokens: budget.input_tokens,
            output_tokens: budget.output_tokens,
            cache_read_tokens: budget.cache_read_tokens,
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
        // Tools and system prompt are fixed for the turn, so its requests
        // share their start and the provider can cache it (RFC 0028).
        self.apply_skills();
        let system = self.system.clone();
        let tools = self.tool_specs();
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

            let (provider, model, model_name) = self.model.get_named();
            let images = model.images && !self.refused_images.contains(&model_name);
            let messages = self.history.for_request(&self.history_settings);
            let sent_images = images && has_images(&messages);
            let request = Request {
                model: model.id.clone(),
                system: system.clone(),
                messages,
                tools: tools.clone(),
                max_output_tokens: model.max_output_tokens,
                temperature: model.temperature,
                images,
            };
            let stream = provider.stream(request, cancel.child_token());
            let answer = stream::collect(stream, events, cancel).await;
            if let Some((input, output, cache_read, cache_write)) = answer.usage {
                budget.input_tokens += input;
                budget.output_tokens += output;
                budget.cache_read_tokens += cache_read;
                events.emit(SessionEvent::Usage {
                    input_tokens: input,
                    output_tokens: output,
                    cache_read_tokens: cache_read,
                    cache_write_tokens: cache_write,
                    context_tokens: input + output,
                    model: model_name.clone(),
                });
            }
            match &answer.end {
                StreamEnd::Done => {}
                StreamEnd::Cancelled => {
                    self.push(Role::Assistant, answer.text_only());
                    return EndReason::Cancelled;
                }
                StreamEnd::Failed(e) => {
                    self.push(Role::Assistant, answer.text_only());
                    let mut message = error_message(&model, &model_name, e);
                    if sent_images && refuses_images(e) {
                        message.push_str(&refused_images_hint(&model_name));
                        self.refused_images.insert(model_name);
                    }
                    events.emit(SessionEvent::Error { message });
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
            self.push(Role::Assistant, answer.blocks.clone());
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
            self.push(Role::User, results);
            if let Some(stop) = stop {
                return stop.reason();
            }
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
            let (mut output, state) = match stop {
                Some(stop) => {
                    let (text, state) = stop.not_run(self.cancel_reason.get().as_deref());
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
            // Nothing of a secret goes to the model or into the log.
            if let Some(secrets) = &self.secrets {
                output.text = secrets.hide(&output.text);
                output.full = secrets.hide(&output.full);
            }
            events.emit(SessionEvent::ToolCall(ToolCallEvent {
                details: None,
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
            // A repeated read of an unchanged file points to the earlier
            // result the model still has (RFC 0028).
            if call.name == "read_file"
                && !output.is_error
                && let Some(earlier) = self.history.unchanged_read(
                    &call.arguments,
                    &output.text,
                    &self.history_settings,
                )
            {
                output.text = history::unchanged_result(&earlier);
            }
            if self
                .history
                .keep_full(&call.id, &output.text, output.full.clone())
            {
                self.journal(Record::Full {
                    call_id: call.id.clone(),
                    text: output.full,
                });
            }
            results.push(Block::ToolResult {
                call_id: call.id,
                content: output.text,
                is_error: output.is_error,
            });
        }
        (results, stop)
    }

    /// Whether the model gets `ask_user`: someone can answer and the user
    /// did not turn it off.
    fn offers_ask_user(&self) -> bool {
        self.can_ask && !self.env.permissions.ask_user_denied
    }

    /// The tools sent with a request.
    fn tool_specs(&self) -> Vec<crate::provider::ToolSpec> {
        let mut specs = tools::specs();
        if self.env.lsp.as_deref().is_some_and(|l| l.offered()) {
            specs.push(tools::lsp::spec());
        }
        if self.offers_ask_user() {
            specs.push(ask_user::spec());
        }
        if self.offers_report {
            specs.push(report::spec());
        }
        specs
    }

    /// Opens a question from the model and turns the answer into the
    /// call's result.
    async fn ask_user(
        &mut self,
        call: &CallIn,
        budget: &Budget,
        events: &EventSink,
        asker: &dyn Asker,
        cancel: &CancellationToken,
    ) -> (ToolOutput, CallState, Option<Stop>) {
        if !self.offers_ask_user() {
            return (
                ToolOutput::error(ask_user::NOT_AVAILABLE),
                CallState::Denied,
                None,
            );
        }
        let asked = match ask_user::check(&call.arguments) {
            Ok(asked) => asked,
            Err(e) => return (ToolOutput::error(e), CallState::Error, None),
        };
        let headers: Vec<&str> = asked.questions.iter().map(|q| q.header.as_str()).collect();
        self.emit_call(
            events,
            call,
            CallState::Waiting,
            Some(headers.join(", ")),
            None,
            None,
        );
        let question = self.question(
            budget,
            ask_user::summary(&asked.questions),
            &ASK_USER_CHOICES,
            QuestionKind::AskUser(Box::new(AskUser {
                call: AskCall {
                    tool: ask_user::NAME.into(),
                    id: call.id.clone(),
                    arguments: call.arguments.clone(),
                },
                questions: asked.questions.clone(),
                context: asked.context,
                text: call.text.clone(),
            })),
        );
        let validate = |answer: &Answer| match answer {
            Answer::Reply { answers } => ask_user::check_answers(&asked.questions, answers),
            _ => Ok(()),
        };
        let stopped = |agent: &Agent| {
            let (text, state) = Stop::Cancelled.not_run(agent.cancel_reason.get().as_deref());
            (ToolOutput::error(text), state, Some(Stop::Cancelled))
        };
        let Some((reply, ())) = wait(&question, events, asker, cancel, validate).await else {
            return stopped(self);
        };
        match reply {
            Reply::NoOne => (
                ToolOutput::error(ask_user::NOT_AVAILABLE),
                CallState::Denied,
                None,
            ),
            Reply::Answer { answer, .. } => match answer {
                Answer::Reply { answers } => (
                    ToolOutput::ok(ask_user::result_text(&asked.questions, &answers)),
                    CallState::Done,
                    None,
                ),
                Answer::Decline => (ToolOutput::ok(ask_user::DECLINED), CallState::Done, None),
                // `cancel`, and choices `wait` rejects.
                _ => stopped(self),
            },
        }
    }

    /// Keeps the report of a `report` call; not permission-checked.
    fn report_call(
        &mut self,
        call: &CallIn,
        events: &EventSink,
    ) -> (ToolOutput, CallState, Option<Stop>) {
        if !self.offers_report {
            return (
                ToolOutput::error(report::NOT_AVAILABLE),
                CallState::Error,
                None,
            );
        }
        match report::check(&call.arguments) {
            Ok(reported) => {
                let summary = Some(reported.status.text().to_owned());
                self.emit_call(events, call, CallState::Running, summary, None, None);
                let text = report::result_text(&reported);
                self.report = Some(reported);
                (ToolOutput::ok(text), CallState::Done, None)
            }
            Err(e) => (ToolOutput::error(e), CallState::Error, None),
        }
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
        if call.name == ask_user::NAME {
            return self.ask_user(call, budget, events, asker, cancel).await;
        }
        if call.name == report::NAME {
            return self.report_call(call, events);
        }
        let arguments = match self.restore_secrets(call) {
            Ok(arguments) => arguments,
            Err(text) => return (ToolOutput::error(text), CallState::Error, None),
        };
        let mut prepared = match tools::prepare(
            &self.env,
            &call.name,
            arguments.as_ref().unwrap_or(&call.arguments),
        ) {
            Prepared::Failed(output) => return (output, CallState::Error, None),
            Prepared::Denied(output) => return (output, CallState::Denied, None),
            Prepared::Ready(prepared) => prepared,
        };
        if let Some(secrets) = &self.secrets {
            prepared.details = hide_in_details(secrets, &prepared.details);
        }
        let summary = Some(prepared.summary.clone());
        if prepared.action() == Action::Ask {
            self.emit_call(
                events,
                call,
                CallState::Waiting,
                summary.clone(),
                None,
                Some(&prepared.details),
            );
            match self
                .ask_permission(call, &prepared, budget, events, asker, cancel)
                .await
            {
                Decided::Run => {}
                Decided::Result(output, state) => return (output, state, None),
                Decided::Stop(stop) => {
                    let (text, state) = stop.not_run(self.cancel_reason.get().as_deref());
                    return (ToolOutput::error(text), state, Some(stop));
                }
            }
        }
        match prepared.lsp_start(&self.env) {
            tools::lsp::Start::Ready => {}
            tools::lsp::Start::Denied => {
                return (ToolOutput::error(LSP_NOT_STARTED), CallState::Denied, None);
            }
            tools::lsp::Start::Ask(details) => {
                self.emit_call(
                    events,
                    call,
                    CallState::Waiting,
                    summary.clone(),
                    None,
                    Some(&prepared.details),
                );
                match self
                    .ask_lsp_start(call, &prepared, *details, budget, events, asker, cancel)
                    .await
                {
                    Decided::Run => {}
                    Decided::Result(output, state) => return (output, state, None),
                    Decided::Stop(stop) => {
                        let (text, state) = stop.not_run(self.cancel_reason.get().as_deref());
                        return (ToolOutput::error(text), state, Some(stop));
                    }
                }
            }
        }
        // A question gave the details already.
        let details = (prepared.action() != Action::Ask).then_some(&prepared.details);
        self.emit_call(events, call, CallState::Running, summary, None, details);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let env = self.env.clone();
        let run = prepared.run_with_progress(&env, cancel.clone(), Some(tx));
        let forward = async {
            // Whole lines only, so a secret is not split between two parts.
            let mut pending = String::new();
            while let Some(text) = rx.recv().await {
                let text = match &self.secrets {
                    Some(secrets) => {
                        pending.push_str(&text);
                        let cut = match pending.rfind('\n') {
                            Some(at) => at + 1,
                            None if pending.len() > PROGRESS_HOLD => pending.len(),
                            None => continue,
                        };
                        let rest = pending.split_off(cut);
                        secrets.hide(&std::mem::replace(&mut pending, rest))
                    }
                    None => text,
                };
                self.emit_call(events, call, CallState::Running, None, Some(text), None);
            }
            if let Some(secrets) = &self.secrets
                && !pending.is_empty()
            {
                let text = secrets.hide(&pending);
                self.emit_call(events, call, CallState::Running, None, Some(text), None);
            }
        };
        let (mut output, ()) = tokio::join!(run, forward);
        if call.name == "run_command"
            && self.secrets.is_some()
            && call
                .arguments
                .get("command")
                .and_then(Value::as_str)
                .is_some_and(|c| crate::secrets::count(c) > 0)
        {
            output.text.push_str(COMMAND_PLACEHOLDER);
            output.full.push_str(COMMAND_PLACEHOLDER);
        }
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

    /// The user's message with its secrets hidden, and a notice that says
    /// which.
    fn hide_in_message(&self, message: String, events: &EventSink) -> String {
        let Some(secrets) = &self.secrets else {
            return message;
        };
        let redacted = secrets.redact(&message);
        if redacted.hidden.is_empty() {
            return message;
        }
        let mut names: Vec<&str> = redacted
            .hidden
            .iter()
            .map(|p| {
                p.strip_prefix("[secret: ")
                    .and_then(|p| p.strip_suffix(']'))
                    .unwrap_or(p)
            })
            .collect();
        names.dedup();
        events.emit(SessionEvent::Notice {
            text: format!(
                "hidden from the model in your message: {}",
                names.join(", ")
            ),
        });
        redacted.text
    }

    /// The arguments of `edit_file` and `write_file` with the values of the
    /// placeholders the session has seen put back, for files in the
    /// workspace and the writable extra directories (RFC 0030). `None`
    /// when nothing changes; an error for an `old_text` with a placeholder
    /// whose value is not known.
    fn restore_secrets(&self, call: &CallIn) -> Result<Option<Value>, String> {
        let Some(secrets) = &self.secrets else {
            return Ok(None);
        };
        let fields: &[&str] = match call.name.as_str() {
            "edit_file" => &["old_text", "new_text"],
            "write_file" => &["content"],
            _ => return Ok(None),
        };
        let Some(path) = call.arguments.get("path").and_then(Value::as_str) else {
            return Ok(None);
        };
        let resolved = self.env.workspace.resolve(path);
        if resolved.is_outside() || self.env.workspace.in_scratch(&resolved) {
            return Ok(None);
        }
        let mut arguments = call.arguments.clone();
        let mut changed = false;
        for field in fields {
            let Some(text) = arguments.get(*field).and_then(Value::as_str) else {
                continue;
            };
            let restored = secrets.restore(text);
            if *field == "old_text"
                && let Some(first) = restored.unknown.first()
            {
                return Err(format!(
                    "old_text contains {first}, whose value is not known in this session; \
                     read the file again"
                ));
            }
            if restored.text != text
                && let Some(slot) = arguments.get_mut(*field)
            {
                *slot = Value::String(restored.text);
                changed = true;
            }
        }
        Ok(changed.then_some(arguments))
    }

    fn emit_call(
        &self,
        events: &EventSink,
        call: &CallIn,
        state: CallState,
        summary: Option<String>,
        output: Option<String>,
        details: Option<&tools::Details>,
    ) {
        events.emit(SessionEvent::ToolCall(ToolCallEvent {
            details: details.map(|d| Box::new(d.clone())),
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
                self.journal(Record::Rules {
                    rules: texts.clone(),
                });
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
            Answer::Continue | Answer::Stop | Answer::Reply { .. } | Answer::Decline => {
                Decided::Stop(Stop::Cancelled)
            }
        }
    }

    /// Asks whether the language server of an `lsp` call may start (RFC
    /// 0019): it may run code of the project. "Yes" lasts for the session.
    #[allow(clippy::too_many_arguments)]
    async fn ask_lsp_start(
        &mut self,
        call: &CallIn,
        prepared: &PreparedCall,
        details: tools::Details,
        budget: &Budget,
        events: &EventSink,
        asker: &dyn Asker,
        cancel: &CancellationToken,
    ) -> Decided {
        let question = self.question(
            budget,
            prepared.lsp_start_summary(&self.env),
            &LSP_START_CHOICES,
            QuestionKind::Permission(Box::new(Permission {
                call: CallInfo {
                    tool: prepared.tool,
                    id: call.id.clone(),
                    arguments: call.arguments.clone(),
                },
                details,
                reason: crate::permissions::Reason::Default {
                    tool: crate::config::Tool::Lsp,
                },
                reason_text: "starting a language server ([lsp] start = \"ask\")".into(),
                proposals: Vec::new(),
                text: call.text.clone(),
            })),
        );
        let validate = |answer: &Answer| -> Result<(), String> {
            match answer {
                Answer::DenyMessage { text } if text.trim().is_empty() => {
                    Err("the message is empty".into())
                }
                _ => Ok(()),
            }
        };
        let Some((reply, ())) = wait(&question, events, asker, cancel, validate).await else {
            return Decided::Stop(Stop::Cancelled);
        };
        let answer = match reply {
            Reply::NoOne => return Decided::Result(ToolOutput::error(NO_ONE), CallState::Denied),
            Reply::Answer { answer, .. } => answer,
        };
        match answer {
            Answer::Allow | Answer::AllowAlways { .. } => {
                prepared.lsp_started(&self.env, true);
                Decided::Run
            }
            Answer::Deny => {
                prepared.lsp_started(&self.env, false);
                Decided::Result(ToolOutput::error(LSP_NOT_STARTED), CallState::Denied)
            }
            Answer::DenyMessage { text } => {
                prepared.lsp_started(&self.env, false);
                Decided::Result(
                    ToolOutput::error(format!("{LSP_NOT_STARTED}: {}", text.trim())),
                    CallState::Denied,
                )
            }
            _ => Decided::Stop(Stop::Cancelled),
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
        let number = self.questions.fetch_add(1, Ordering::Relaxed) + 1;
        Question {
            id: format!("q{number}"),
            session: SessionRef::default(),
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

/// Whether any of `messages` holds an image.
fn has_images(messages: &[crate::provider::Message]) -> bool {
    messages
        .iter()
        .any(|m| m.content.iter().any(|b| matches!(b, Block::Image(_))))
}

/// Whether `e` is the API refusing the images of a request: a rejected
/// request whose message speaks of images (RFC 0034).
fn refuses_images(e: &ProviderError) -> bool {
    matches!(e.kind, ErrorKind::InvalidRequest | ErrorKind::Unsupported)
        && e.message.to_lowercase().contains("image")
}

/// What follows the error of a model that refused an image.
fn refused_images_hint(model_name: &str) -> String {
    let table = match model_name.split_once('/') {
        Some((provider, model)) => format!("[providers.{provider}.models.{model}]"),
        None => "its model table".to_owned(),
    };
    format!(
        "\n{model_name} refused the image; this session sends images to it as notes from now \
         on. If the model does not read images, set `images = false` in {table}."
    )
}

fn error_message(model: &Model, model_name: &str, e: &ProviderError) -> String {
    match e.kind {
        // Retried already (see `spec/providers.md`, "Adapters").
        ErrorKind::RateLimit | ErrorKind::Overloaded => {
            let provider = model_name.split_once('/').map_or(model_name, |(p, _)| p);
            format!(
                "{}\nthe provider is busy; send the message again later, or raise max_retries \
                 in [providers.{provider}]",
                e.message
            )
        }
        ErrorKind::ContextTooLong => format!(
            "the conversation is too long for {} ({} tokens); start a new session",
            model.display_name,
            tools::group(usize::try_from(model.context_window).unwrap_or(usize::MAX))
        ),
        _ => e.message.clone(),
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
                        answer.choice().name()
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
                ..
            } => calls.push(CallIn {
                id: id.clone(),
                name: name.clone(),
                arguments: arguments.clone(),
                text: (!text.is_empty()).then(|| text.clone()),
            }),
            Block::ToolResult { .. } | Block::Image(_) => {}
        }
    }
    calls
}
