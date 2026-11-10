//! Statistics of a session, counted from its event log (see
//! `spec/sessions.md`, "Statistics"), and the status a `session_status`
//! request returns. The session counts each event as it is published; on
//! resume, and for a saved session, the log is counted again.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::agent::{Answer, CallState, Closed, EndReason, SessionEvent, State};

use super::{ModelInfo, SessionInfo, Status, TokenTotals};

/// Turns by how they ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnCounts {
    pub done: u64,
    pub cancelled: u64,
    pub limit: u64,
    pub failed: u64,
}

impl TurnCounts {
    pub fn total(&self) -> u64 {
        self.done + self.cancelled + self.limit + self.failed
    }
}

/// The calls of one tool by final state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolCounts {
    pub done: u64,
    pub error: u64,
    pub denied: u64,
    pub cancelled: u64,
}

impl ToolCounts {
    pub fn total(&self) -> u64 {
        self.done + self.error + self.denied + self.cancelled
    }

    fn add(&mut self, other: &ToolCounts) {
        self.done += other.done;
        self.error += other.error;
        self.denied += other.denied;
        self.cancelled += other.cancelled;
    }
}

/// Questions by how they closed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnswerCounts {
    pub allow: u64,
    pub always: u64,
    pub deny: u64,
    /// Denied with a message.
    pub explain: u64,
    /// Denied and the turn cancelled.
    pub cancel: u64,
    /// A limit question: go on.
    #[serde(rename = "continue")]
    pub go_on: u64,
    /// A limit question: stop.
    pub stop: u64,
    /// A question from the model: answered.
    #[serde(default)]
    pub reply: u64,
    /// A question from the model: declined.
    #[serde(default)]
    pub decline: u64,
    /// Closed without an answer (no client could answer).
    pub no_one: u64,
    /// Closed because the turn was cancelled.
    pub cancelled: u64,
}

impl AnswerCounts {
    pub fn total(&self) -> u64 {
        self.allow
            + self.always
            + self.deny
            + self.explain
            + self.cancel
            + self.go_on
            + self.stop
            + self.reply
            + self.decline
            + self.no_one
            + self.cancelled
    }
}

/// The turn that runs: its start and its use of the current budget (a
/// `continue` answer to a limit question starts a new one).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunningTurn {
    /// Milliseconds since 1970; `None` in a log without times.
    pub started: Option<u64>,
    pub requests: u64,
    pub tool_calls: u64,
    /// `continue` answers so far.
    pub continued: u64,
    /// Requests counted from `usage` in this turn, over all budgets; the
    /// turn's end adds those that sent none.
    #[serde(default)]
    pub counted: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stats {
    pub tokens: TokenTotals,
    /// Of the last request.
    pub context_tokens: u64,
    pub turns: TurnCounts,
    /// Model requests: from `usage` while a turn runs, and from the count
    /// at its end for requests that reported none.
    pub requests: u64,
    pub retries: u64,
    /// `error` events: failed requests and turns.
    pub errors: u64,
    pub tools: BTreeMap<String, ToolCounts>,
    pub answers: AnswerCounts,
    /// Answers per client name.
    pub by: BTreeMap<String, u64>,
    pub model_changes: u64,
    /// The model before the first change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first_model: Option<String>,
    /// Time in `working`, in milliseconds, over stretches with known times.
    pub working_ms: u64,
    /// Stretches in `working` whose start or end has no time (older logs).
    pub unknown_stretches: u64,
    /// The session is in `working` now.
    pub working: bool,
    /// Since when, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_since: Option<u64>,
    /// The time of the last event, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_active: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub turn: Option<RunningTurn>,
}

impl Stats {
    /// Counts an event logged at `at` (milliseconds since 1970).
    pub fn count(&mut self, event: &SessionEvent, at: Option<u64>) {
        if let Some(at) = at {
            self.last_active = Some(self.last_active.map_or(at, |last| last.max(at)));
        }
        match event {
            SessionEvent::State { state } => {
                let working = *state == State::Working;
                if working && !self.working {
                    self.working = true;
                    self.working_since = at;
                } else if !working && self.working {
                    self.stop_working(at);
                }
            }
            SessionEvent::UserMessage { .. } => {
                self.turn = Some(RunningTurn {
                    started: at,
                    ..RunningTurn::default()
                });
            }
            SessionEvent::Usage { context_tokens, .. } => {
                self.tokens.count(event);
                self.context_tokens = *context_tokens;
                self.requests += 1;
                if let Some(turn) = &mut self.turn {
                    turn.requests += 1;
                    turn.counted += 1;
                }
            }
            SessionEvent::Retry { .. } => self.retries += 1,
            SessionEvent::Error { .. } => self.errors += 1,
            SessionEvent::ToolCall(call) if call.state.is_final() => {
                let name = if call.name.is_empty() {
                    "(unknown)"
                } else {
                    call.name.as_str()
                };
                let counts = self.tools.entry(name.to_owned()).or_default();
                match call.state {
                    CallState::Done => counts.done += 1,
                    CallState::Error => counts.error += 1,
                    CallState::Denied => counts.denied += 1,
                    _ => counts.cancelled += 1,
                }
                if let Some(turn) = &mut self.turn {
                    turn.tool_calls += 1;
                }
            }
            SessionEvent::QuestionClosed { closed, .. } => self.closed(closed),
            SessionEvent::TurnEnd(end) => {
                match end.reason {
                    EndReason::Done => self.turns.done += 1,
                    EndReason::Cancelled => self.turns.cancelled += 1,
                    EndReason::Limit => self.turns.limit += 1,
                    EndReason::Failed => self.turns.failed += 1,
                }
                let counted = self.turn.take().map_or(0, |t| t.counted);
                self.requests += u64::from(end.requests).saturating_sub(counted);
            }
            SessionEvent::ModelChanged { from, .. } => {
                self.model_changes += 1;
                if self.first_model.is_none() && !from.is_empty() {
                    self.first_model = Some(from.clone());
                }
            }
            SessionEvent::Ended { .. } if self.working => self.stop_working(at),
            _ => {}
        }
    }

    fn closed(&mut self, closed: &Closed) {
        let answers = &mut self.answers;
        match closed {
            Closed::Answered { answer, by } => {
                match answer {
                    Answer::Allow => answers.allow += 1,
                    Answer::AllowAlways { .. } => answers.always += 1,
                    Answer::Deny => answers.deny += 1,
                    Answer::DenyMessage { .. } => answers.explain += 1,
                    Answer::Cancel => answers.cancel += 1,
                    Answer::Continue => {
                        answers.go_on += 1;
                        if let Some(turn) = &mut self.turn {
                            turn.requests = 0;
                            turn.tool_calls = 0;
                            turn.continued += 1;
                        }
                    }
                    Answer::Stop => answers.stop += 1,
                    Answer::Reply { .. } => answers.reply += 1,
                    Answer::Decline => answers.decline += 1,
                }
                *self.by.entry(by.clone()).or_default() += 1;
            }
            Closed::NoOne => answers.no_one += 1,
            Closed::Cancelled => answers.cancelled += 1,
        }
    }

    fn stop_working(&mut self, at: Option<u64>) {
        self.working = false;
        match (self.working_since.take(), at) {
            (Some(since), Some(at)) => self.working_ms += at.saturating_sub(since),
            _ => self.unknown_stretches += 1,
        }
    }

    /// Counts a whole log of `(time, event)`.
    pub fn of_log<'a>(log: impl IntoIterator<Item = (Option<u64>, &'a SessionEvent)>) -> Stats {
        let mut stats = Stats::default();
        for (at, event) in log {
            stats.count(event, at);
        }
        stats
    }

    /// For a session that is not running: a turn or a working stretch the
    /// log leaves open ended with its last event.
    pub fn settle(&mut self) {
        if self.working {
            self.stop_working(self.last_active);
        }
        self.turn = None;
    }

    /// The time in `working` up to `now`, and whether it is exact: `false`
    /// when some stretch has no time.
    pub fn working_time(&self, now: u64) -> (u64, bool) {
        let mut ms = self.working_ms;
        let mut exact = self.unknown_stretches == 0;
        if self.working {
            match self.working_since {
                Some(since) => ms += now.saturating_sub(since),
                None => exact = false,
            }
        }
        (ms, exact)
    }

    /// All tool calls together.
    pub fn all_tools(&self) -> ToolCounts {
        let mut all = ToolCounts::default();
        for counts in self.tools.values() {
            all.add(counts);
        }
        all
    }
}

/// A client attached to the session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientInfo {
    /// `tui pts/3`, `clai chat`.
    pub name: String,
    /// Whether it answers questions; a client that only watches does not.
    pub answers: bool,
}

/// An extra directory of the workspace (`permissions.extra_dirs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DirInfo {
    /// As shown: `~/ws/shared-lib`.
    pub path: String,
    pub read_only: bool,
    pub exists: bool,
}

/// The session's model as configured now.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelStatus {
    pub info: ModelInfo,
    /// `anthropic`, `openai-compatible`.
    pub api: String,
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    pub max_retries: u32,
}

/// The limits per turn in effect for the session's workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LimitsInfo {
    pub max_requests: u32,
    pub max_tool_calls: u32,
}

/// The answer to `session_status`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionStatus {
    pub session: SessionInfo,
    pub stats: Stats,
    pub clients: Vec<ClientInfo>,
    /// Where the session is saved, as shown (`~/.local/state/…`), and the
    /// size of its files in bytes; `None` for a session in memory only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    pub extra_dirs: Vec<DirInfo>,
    /// `None` when the model is no longer configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelStatus>,
    /// `None` when the configuration cannot be read for the workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limits: Option<LimitsInfo>,
    /// The server's clock, in milliseconds since 1970, for durations.
    pub now: u64,
    pub server: Status,
}

/// The days a `global_stats` request counts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Range {
    /// The last 7 days, today included.
    #[default]
    #[serde(rename = "7d")]
    Week,
    /// The last 30 days, today included.
    #[serde(rename = "30d")]
    Month,
    #[serde(rename = "all")]
    All,
}

impl Range {
    pub const ALL: [Range; 3] = [Range::Week, Range::Month, Range::All];

    /// `7d`, `30d`, `all`: as typed after `/stats`.
    pub fn name(self) -> &'static str {
        match self {
            Range::Week => "7d",
            Range::Month => "30d",
            Range::All => "all",
        }
    }

    pub fn parse(text: &str) -> Option<Range> {
        Range::ALL.into_iter().find(|r| r.name() == text)
    }

    /// The days it spans, if fixed.
    pub fn days(self) -> Option<i64> {
        match self {
            Range::Week => Some(7),
            Range::Month => Some(30),
            Range::All => None,
        }
    }

    /// The next one, for a key that goes round.
    pub fn next(self) -> Range {
        match self {
            Range::Week => Range::Month,
            Range::Month => Range::All,
            Range::All => Range::Week,
        }
    }
}

/// The sessions of a `global_stats` answer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionCounts {
    /// With at least one event in the range.
    pub active: u64,
    /// All saved sessions.
    pub saved: u64,
    /// Loaded in the server now.
    pub running: u64,
}

/// The totals over all sessions, as in [`Stats`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalTotals {
    pub turns: TurnCounts,
    pub requests: u64,
    pub retries: u64,
    pub errors: u64,
    pub tokens: TokenTotals,
    /// Time in `working`, in milliseconds, over stretches with known times.
    pub working_ms: u64,
    /// Stretches in `working` whose start or end has no time.
    pub unknown_stretches: u64,
}

/// The use of one model.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelTotals {
    /// `provider/model`, or `unknown`.
    pub model: String,
    /// In the server's configuration now.
    pub configured: bool,
    pub sessions: u64,
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl ModelTotals {
    pub fn tokens(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

/// The tokens of one day.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DayTokens {
    /// Days since 1970 in the client's time zone.
    pub day: i64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl DayTokens {
    pub fn total(&self) -> u64 {
        self.input_tokens.saturating_add(self.output_tokens)
    }
}

/// The answer to `global_stats`: statistics over the logs of all saved
/// sessions (see `spec/sessions.md`, "Statistics").
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalStats {
    pub range: Range,
    /// The first and the last day, as days since 1970 in the client's time
    /// zone; the last is today.
    pub first_day: i64,
    pub last_day: i64,
    pub sessions: SessionCounts,
    pub totals: GlobalTotals,
    /// Most tokens first.
    pub models: Vec<ModelTotals>,
    /// One per day of the range, empty days included.
    pub days: Vec<DayTokens>,
    /// Tokens of events without a time; counted in `all` only.
    pub undated_tokens: u64,
    /// Logs that could not be read.
    pub skipped: u64,
    /// The server's clock, in milliseconds since 1970.
    pub now: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{ToolCallEvent, TurnEnd};
    use serde_json::Value;

    fn state(state: State) -> SessionEvent {
        SessionEvent::State { state }
    }

    fn call(name: &str, state: CallState) -> SessionEvent {
        SessionEvent::ToolCall(ToolCallEvent {
            id: "c".into(),
            name: name.into(),
            state,
            summary: None,
            arguments: Value::Null,
            output: None,
            result: None,
        })
    }

    fn usage(input: u64, output: u64) -> SessionEvent {
        SessionEvent::Usage {
            input_tokens: input,
            output_tokens: output,
            context_tokens: input + output,
            model: String::new(),
        }
    }

    fn end(reason: EndReason) -> SessionEvent {
        SessionEvent::TurnEnd(TurnEnd {
            reason,
            requests: 0,
            tool_calls: 0,
            input_tokens: 0,
            output_tokens: 0,
        })
    }

    fn answered(answer: Answer, by: &str) -> SessionEvent {
        SessionEvent::QuestionClosed {
            id: "q".into(),
            closed: Closed::Answered {
                answer,
                by: by.into(),
            },
        }
    }

    fn user() -> SessionEvent {
        SessionEvent::UserMessage { text: "hi".into() }
    }

    #[test]
    fn counts_a_log() {
        let log = [
            (Some(1_000), state(State::Working)),
            (Some(1_000), user()),
            (Some(2_000), usage(100, 10)),
            (Some(2_100), call("read_file", CallState::Pending)),
            (Some(2_200), call("read_file", CallState::Done)),
            (Some(2_300), state(State::Waiting)),
            (Some(4_000), answered(Answer::Allow, "tui pts/3")),
            (Some(4_000), state(State::Working)),
            (Some(4_500), call("edit_file", CallState::Denied)),
            (
                Some(4_600),
                SessionEvent::Retry {
                    attempt: 1,
                    max: 3,
                    reason: "HTTP 503".into(),
                },
            ),
            (Some(5_000), usage(200, 20)),
            (Some(5_000), end(EndReason::Done)),
            (Some(5_000), state(State::Idle)),
            (
                Some(6_000),
                SessionEvent::ModelChanged {
                    from: "a/one".into(),
                    model: "a/two".into(),
                },
            ),
            (
                Some(7_000),
                SessionEvent::ModelChanged {
                    from: "a/two".into(),
                    model: "a/three".into(),
                },
            ),
        ];
        let stats = Stats::of_log(log.iter().map(|(at, e)| (*at, e)));
        assert_eq!(
            stats.tokens,
            TokenTotals {
                input_tokens: 300,
                output_tokens: 30
            }
        );
        assert_eq!(stats.context_tokens, 220);
        assert_eq!(stats.requests, 2);
        assert_eq!(stats.retries, 1);
        assert_eq!(stats.turns.done, 1);
        assert_eq!(stats.tools["read_file"].done, 1);
        assert_eq!(stats.tools["edit_file"].denied, 1);
        assert_eq!(stats.all_tools().total(), 2);
        assert_eq!(stats.answers.allow, 1);
        assert_eq!(stats.by["tui pts/3"], 1);
        let more = [
            answered(Answer::Reply { answers: vec![] }, "tui"),
            answered(Answer::Decline, "tui"),
        ];
        let model = Stats::of_log(more.iter().map(|e| (None, e)));
        assert_eq!((model.answers.reply, model.answers.decline), (1, 1));
        assert_eq!(model.answers.total(), 2);
        // Working 1.0–2.3 s and 4.0–5.0 s; waiting does not count.
        assert_eq!(stats.working_time(9_000), (2_300, true));
        assert_eq!(stats.model_changes, 2);
        assert_eq!(stats.first_model.as_deref(), Some("a/one"));
        assert_eq!(stats.last_active, Some(7_000));
        assert!(stats.turn.is_none());
    }

    #[test]
    fn the_running_turn_and_a_new_budget() {
        let mut stats = Stats::default();
        stats.count(&state(State::Working), Some(10));
        stats.count(&user(), Some(10));
        stats.count(&usage(1, 1), Some(20));
        stats.count(&call("run_command", CallState::Error), Some(30));
        let turn = stats.turn.unwrap();
        assert_eq!(
            (turn.started, turn.requests, turn.tool_calls),
            (Some(10), 1, 1)
        );
        assert_eq!(stats.working_time(110), (100, true));
        stats.count(&answered(Answer::Continue, "chat"), Some(40));
        let turn = stats.turn.unwrap();
        assert_eq!((turn.requests, turn.tool_calls, turn.continued), (0, 0, 1));
        assert_eq!(stats.answers.go_on, 1);
    }

    #[test]
    fn requests_without_usage_count_at_the_end_of_the_turn() {
        let mut stats = Stats::default();
        stats.count(&user(), None);
        stats.count(&usage(1, 1), None);
        stats.count(
            &SessionEvent::TurnEnd(TurnEnd {
                reason: EndReason::Done,
                requests: 3,
                tool_calls: 0,
                input_tokens: 0,
                output_tokens: 0,
            }),
            None,
        );
        assert_eq!(stats.requests, 3);
    }

    #[test]
    fn a_log_without_times() {
        let log = [
            (None, state(State::Working)),
            (None, user()),
            (None, end(EndReason::Cancelled)),
            (None, state(State::Idle)),
            (Some(50), state(State::Working)),
            (Some(80), state(State::Idle)),
        ];
        let stats = Stats::of_log(log.iter().map(|(at, e)| (*at, e)));
        assert_eq!(stats.working_time(100), (30, false));
        assert_eq!(stats.turns.cancelled, 1);
    }

    #[test]
    fn a_saved_log_cut_in_a_turn_settles() {
        let log = [
            (Some(10), state(State::Working)),
            (Some(10), user()),
            (Some(70), usage(1, 1)),
        ];
        let mut stats = Stats::of_log(log.iter().map(|(at, e)| (*at, e)));
        stats.settle();
        assert!(stats.turn.is_none());
        assert!(!stats.working);
        assert_eq!(stats.working_time(1_000), (60, true));
    }

    #[test]
    fn closed_without_answer() {
        let mut stats = Stats::default();
        for closed in [Closed::NoOne, Closed::Cancelled] {
            stats.count(
                &SessionEvent::QuestionClosed {
                    id: "q".into(),
                    closed,
                },
                None,
            );
        }
        stats.count(
            &answered(Answer::DenyMessage { text: "no".into() }, "a"),
            None,
        );
        assert_eq!(
            (
                stats.answers.no_one,
                stats.answers.cancelled,
                stats.answers.explain
            ),
            (1, 1, 1)
        );
        assert_eq!(stats.answers.total(), 3);
        assert!(stats.by.contains_key("a"));
    }

    #[test]
    fn round_trip() {
        let mut stats = Stats::default();
        stats.count(&answered(Answer::Continue, "a"), Some(1));
        let json = serde_json::to_value(&stats).unwrap();
        assert_eq!(json["answers"]["continue"], 1);
        let back: Stats = serde_json::from_value(json).unwrap();
        assert_eq!(back, stats);
    }
}
