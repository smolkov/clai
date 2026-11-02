//! A session's conversation as a stream of text for `clai chat`: user
//! messages, model text as it arrives, tool lines, notices.

use std::collections::{HashMap, VecDeque};

use crate::agent::{CallState, SessionEvent};

use super::events::tool_line;

const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const YELLOW: &str = "\x1b[33m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

/// Turns session events into text. Questions are not shown here; the
/// chat shows them with its dialog.
#[derive(Debug, Default)]
pub struct Conversation {
    unicode: bool,
    colour: bool,
    /// Summaries of calls, by id, from their earlier events.
    summaries: HashMap<String, String>,
    /// The last text written did not end with a newline.
    mid_line: bool,
    /// Messages this client sent and whose echo is still to come.
    sent: VecDeque<String>,
}

impl Conversation {
    pub fn new(unicode: bool, colour: bool) -> Conversation {
        Conversation {
            unicode,
            colour,
            ..Conversation::default()
        }
    }

    /// A message this client sent: the terminal shows it already, so its
    /// `user_message` event is not shown again.
    pub fn sent(&mut self, text: &str) {
        self.sent.push_back(text.to_owned());
    }

    /// Whether the cursor is in the middle of a line of model text.
    pub fn mid_line(&self) -> bool {
        self.mid_line
    }

    /// Ends a started line, so that other output starts on its own line.
    pub fn break_line(&mut self) -> &'static str {
        if std::mem::take(&mut self.mid_line) {
            "\n"
        } else {
            ""
        }
    }

    fn paint(&self, colour: &str, text: &str) -> String {
        if self.colour {
            format!("{colour}{text}{RESET}")
        } else {
            text.to_owned()
        }
    }

    /// The text this event adds; `replay` for events from the log.
    pub fn event(&mut self, event: &SessionEvent, replay: bool) -> String {
        match event {
            SessionEvent::UserMessage { text } => {
                if !replay && self.sent.front() == Some(text) {
                    self.sent.pop_front();
                    return String::new();
                }
                let prompt = self.paint(BOLD, ">");
                format!("{}{prompt} {text}\n", self.break_line())
            }
            SessionEvent::TextDelta { text } => {
                if text.is_empty() {
                    return String::new();
                }
                self.mid_line = !text.ends_with('\n');
                text.clone()
            }
            SessionEvent::TextDone { .. } => self.break_line().to_owned(),
            SessionEvent::ToolCall(call) => {
                if let Some(summary) = &call.summary {
                    self.summaries.insert(call.id.clone(), summary.clone());
                }
                if !call.state.is_final() {
                    return String::new();
                }
                let summary = self.summaries.get(&call.id).map(String::as_str);
                let line = tool_line(call, summary, self.unicode);
                let colour = match call.state {
                    CallState::Done => GREEN,
                    CallState::Error => RED,
                    _ => YELLOW,
                };
                format!("{}{}\n", self.break_line(), self.paint(colour, &line))
            }
            SessionEvent::Notice { text } => self.note(text),
            SessionEvent::Error { message } => {
                let line = format!("clai: error: {message}");
                format!("{}{}\n", self.break_line(), self.paint(RED, &line))
            }
            SessionEvent::ModelChanged { model } => self.note(&format!("model is now {model}")),
            SessionEvent::Renamed { name } => self.note(&format!("session renamed to {name}")),
            SessionEvent::Ended { reason } => self.note(&format!("session ended: {reason}")),
            SessionEvent::State { .. }
            | SessionEvent::Question(_)
            | SessionEvent::QuestionClosed { .. }
            | SessionEvent::Usage { .. }
            | SessionEvent::TurnEnd(_) => String::new(),
        }
    }

    /// `clai: <text>`, dimmed.
    pub fn note(&mut self, text: &str) -> String {
        let line = format!("clai: {text}");
        format!("{}{}\n", self.break_line(), self.paint(DIM, &line))
    }
}
