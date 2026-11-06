//! All view state and how it changes: one input in, effects out. No terminal
//! and no network here, so nearly all behaviour is tested without either.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::agent::{Closed, QuestionKind, SessionEvent, State, now_secs};
use crate::protocol::{
    ModelInfo, Notice, Payload, Request, ServerMessage, SessionInfo, TokenTotals,
};

use super::conversation::{Conversation, Kind};
use super::editor::{Browse, Editor};
use super::keys::{self, Action, Focus};
use super::picker::Picker;
use super::prompt::Prompt;
use super::style::Look;
use super::view;

/// `Ctrl-C` twice within this time detaches.
const DOUBLE_INTERRUPT: Duration = Duration::from_secs(1);
/// How long to try to get the connection back.
pub const RECONNECT_FOR: Duration = Duration::from_secs(10);
/// How many ticks between two `list` requests for the session row.
const LIST_TICKS: u32 = 2;

/// The error of a request whose connection was lost: a reconnect follows, so
/// it is no news to the user.
pub const LOST: &str = "the connection to the server was lost";

/// How the TUI ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    Detached {
        name: String,
    },
    Ended {
        name: String,
        reason: String,
    },
    /// The server stayed away.
    Lost,
    /// `SIGTERM` or `SIGHUP`.
    Signal,
}

/// The settings; only the context windows change while the TUI runs.
#[derive(Debug, Clone)]
pub struct Settings {
    pub look: Look,
    /// Who this client is to the server: `tui pts/3`.
    pub client: String,
    pub tool_output_lines: usize,
    pub expand_tool_calls: bool,
    pub scroll_lines: usize,
    /// `tui.timestamps`: the offset of local time from UTC, in seconds.
    pub timestamps: Option<i64>,
    /// Context windows by `provider/model`: from the TUI's own configuration
    /// at first, then from the server's `models` answers.
    pub windows: HashMap<String, u64>,
}

/// A session just attached to.
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    pub session: SessionInfo,
    pub sessions: Vec<SessionInfo>,
    /// The replayed log and what came after it.
    pub backlog: Vec<ServerMessage>,
}

/// One thing that happened.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Key(KeyEvent),
    Paste(String),
    Mouse(MouseEvent),
    Resize,
    Server(ServerMessage),
    /// The answer to a request this client sent.
    Response {
        request: Request,
        result: Result<Payload, String>,
    },
    /// The 1 s timer.
    Tick,
    Disconnected,
    Attached(Box<Attachment>),
    ReconnectFailed,
}

/// What the shell has to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Request(Request),
    Redraw,
    /// Try to connect again, after a short wait.
    Reconnect,
    Exit(Exit),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    Help,
    ConfirmKill,
}

/// Position among all sessions and others waiting for an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    pub position: usize,
    pub total: usize,
    pub waiting: usize,
}

#[derive(Debug)]
pub struct App {
    pub settings: Settings,
    pub session: SessionInfo,
    pub conversation: Conversation,
    pub editor: Editor,
    pub prompt: Option<Prompt>,
    pub dialog: Option<Dialog>,
    pub picker: Option<Picker>,
    /// A `models` request is on its way to open the picker.
    picker_wanted: bool,
    pub focus: Focus,
    /// The selected entry, with focus on the conversation.
    pub selected: Option<usize>,
    /// The first row shown.
    pub top: usize,
    /// New output keeps the view at the end.
    pub follow: bool,
    /// New output came while the user had scrolled up.
    pub new_output: bool,
    pub place: Option<Place>,
    /// Tokens of the last request.
    pub context_tokens: u64,
    /// The sum of the log's `usage` events; `None` until the first. It is not
    /// taken from the session info: the log replayed after it would count twice.
    pub usage: Option<TokenTotals>,
    /// In the current turn.
    pub requests: u32,
    pub tool_calls: u32,
    /// Since when the connection is gone.
    pub lost_since: Option<Instant>,
    /// Why the session ended; any key then exits.
    pub ended: Option<String>,
    /// A short message in the status line, until the next key.
    pub message: Option<String>,
    /// The current time; the shell sets it before each input.
    pub now: Instant,
    last_interrupt: Option<Instant>,
    ticks: u32,
    /// Rows of the conversation area, from the last layout.
    view_height: usize,
    /// All rows, from the last layout.
    total: Option<usize>,
    width: Option<usize>,
    /// The entry to bring into view at the next layout.
    reveal: Option<usize>,
    /// Revealing a question stopped following; resume when it closes.
    resume_follow: bool,
}

impl App {
    pub fn new(settings: Settings, attachment: Attachment, now: Instant) -> App {
        let conversation = Conversation::new(
            settings.look,
            settings.tool_output_lines,
            settings.expand_tool_calls,
        );
        let mut app = App {
            settings,
            session: attachment.session.clone(),
            conversation,
            editor: Editor::default(),
            prompt: None,
            dialog: None,
            picker: None,
            picker_wanted: false,
            focus: Focus::Input,
            selected: None,
            top: 0,
            follow: true,
            new_output: false,
            place: None,
            context_tokens: 0,
            usage: None,
            requests: 0,
            tool_calls: 0,
            lost_since: None,
            ended: None,
            message: None,
            now,
            last_interrupt: None,
            ticks: 0,
            view_height: 0,
            total: None,
            width: None,
            reveal: None,
            resume_follow: false,
        };
        app.attached(attachment);
        app
    }

    /// Takes one input.
    pub fn update(&mut self, input: Input) -> Vec<Effect> {
        let effects = self.handle(input);
        // Keys go to an open prompt, so the picker must not hide it: what is
        // typed for the filter would answer the question.
        if self.prompt.is_some() || self.ended.is_some() {
            self.picker = None;
            self.picker_wanted = false;
        }
        effects
    }

    fn handle(&mut self, input: Input) -> Vec<Effect> {
        match input {
            Input::Key(key) => self.key(key),
            Input::Paste(text) => self.paste(&text),
            Input::Mouse(mouse) => self.mouse(mouse),
            Input::Resize => vec![Effect::Redraw],
            Input::Server(message) => self.server(message),
            Input::Response { request, result } => self.response(request, result),
            Input::Tick => self.tick(),
            Input::Disconnected => self.disconnected(),
            Input::Attached(attachment) => {
                self.lost_since = None;
                self.attached(*attachment);
                vec![Effect::Redraw]
            }
            Input::ReconnectFailed => self.reconnect_failed(),
        }
    }

    /// The window of the session's model, if it is configured.
    pub fn context_window(&self) -> Option<u64> {
        self.settings.windows.get(&self.session.model).copied()
    }

    // Attach and replay.

    /// Builds the conversation from the log. Scroll position, expanded
    /// calls and unsent input stay, for a re-attach after a lost connection.
    fn attached(&mut self, attachment: Attachment) {
        let Attachment {
            session,
            sessions,
            backlog,
        } = attachment;
        self.session = session;
        self.conversation.clear();
        self.conversation.stamp = None;
        self.total = None;
        (self.requests, self.tool_calls) = (0, 0);
        // Summed again from the log, so that a reconnect does not add to it.
        self.context_tokens = 0;
        self.usage = None;
        let earlier = self.prompt.take();
        for message in backlog {
            let _ = self.server(message);
        }
        // What the server says now counts: a log may hold a question that a
        // restart closed without a trace.
        let current = sessions.iter().find(|s| s.id == self.session.id);
        if let Some(current) = current {
            self.session = current.clone();
        }
        let question = current.and_then(|s| s.question.clone());
        let held = self.prompt.take().or(earlier);
        self.prompt = match question {
            Some(q) if q.choices.is_empty() => None,
            Some(q) => match held {
                Some(prompt) if prompt.id() == q.id && !prompt.sending => Some(prompt),
                _ => {
                    self.open_question(&q);
                    Some(Prompt::new(q))
                }
            },
            None => None,
        };
        self.set_place(&sessions);
    }

    fn set_place(&mut self, sessions: &[SessionInfo]) {
        let Some(at) = sessions.iter().position(|s| s.id == self.session.id) else {
            return;
        };
        self.place = Some(Place {
            position: at + 1,
            total: sessions.len(),
            waiting: sessions
                .iter()
                .filter(|s| s.id != self.session.id && s.question.is_some())
                .count(),
        });
    }

    // Server messages.

    fn server(&mut self, message: ServerMessage) -> Vec<Effect> {
        match message {
            ServerMessage::Event {
                session,
                replay,
                event,
            } if session == self.session.id => {
                self.conversation.stamp = if replay { None } else { self.stamp() };
                self.event(event, replay);
                vec![Effect::Redraw]
            }
            ServerMessage::Notice { notice } => {
                self.conversation.stamp = self.stamp();
                match notice {
                    Notice::ConfigReloaded => {
                        self.conversation.notice("configuration reloaded");
                        return vec![Effect::Redraw, Effect::Request(Request::Models)];
                    }
                    Notice::ConfigRejected { errors } => {
                        for error in errors {
                            self.conversation
                                .error(format!("configuration rejected: {error}"));
                        }
                    }
                    Notice::Stopping => self.conversation.notice("the server is stopping"),
                    Notice::SessionCreated { .. } => return Vec::new(),
                }
                vec![Effect::Redraw]
            }
            ServerMessage::Error { message } => {
                self.conversation.error(message);
                vec![Effect::Redraw]
            }
            ServerMessage::Event { .. }
            | ServerMessage::Hello(_)
            | ServerMessage::Response { .. } => Vec::new(),
        }
    }

    fn stamp(&self) -> Option<String> {
        self.settings
            .timestamps
            .map(|offset| clock(now_secs(), offset))
    }

    fn event(&mut self, event: SessionEvent, replay: bool) {
        let conversation = &mut self.conversation;
        match event {
            SessionEvent::State { state } => self.session.state = state,
            SessionEvent::UserMessage { text } => {
                conversation.user(&text);
                (self.requests, self.tool_calls) = (0, 0);
            }
            SessionEvent::TextDelta { text } => conversation.text_delta(&text),
            SessionEvent::TextDone { text } => conversation.text_done(&text),
            SessionEvent::ToolCall(call) => {
                if call.state.is_final() && conversation.entry_of_call(&call.id).is_some() {
                    self.tool_calls += 1;
                }
                conversation.tool_call(&call);
            }
            SessionEvent::Question(question) => {
                self.open_question(&question);
                if !replay && !question.choices.is_empty() {
                    self.prompt = Some(Prompt::new(*question));
                }
            }
            SessionEvent::QuestionClosed { id, closed } => self.question_closed(&id, &closed),
            SessionEvent::Notice { text } => conversation.notice(text),
            SessionEvent::Usage { context_tokens, .. } => {
                self.context_tokens = context_tokens;
                self.usage.get_or_insert_default().count(&event);
                self.requests += 1;
            }
            SessionEvent::Error { message } => conversation.error(message),
            SessionEvent::TurnEnd(_) => conversation.close_text(),
            SessionEvent::ModelChanged { from, model } => {
                conversation.notice(if from.is_empty() {
                    format!("model: {model}")
                } else {
                    format!("model: {from} → {model}")
                });
                self.session.model = model;
            }
            SessionEvent::Renamed { name } => {
                conversation.notice(format!("session renamed to {name}"));
                self.session.name = name;
            }
            SessionEvent::Ended { reason } => {
                conversation.notice(format!("session ended: {reason}"));
                self.ended = Some(reason);
                self.prompt = None;
                self.dialog = None;
            }
        }
    }

    /// Shows the call of a permission question; a limit question has none.
    fn open_question(&mut self, question: &crate::agent::Question) {
        if let QuestionKind::Permission(p) = &question.kind {
            self.conversation
                .set_details(&p.call.id, p.call.tool.name(), &p.details);
            if !question.choices.is_empty() {
                self.reveal = self.conversation.entry_of_call(&p.call.id);
            }
        }
    }

    fn question_closed(&mut self, id: &str, closed: &Closed) {
        let open = self.prompt.as_ref().is_some_and(|p| p.id() == id);
        if !open {
            return;
        }
        self.prompt = None;
        if std::mem::take(&mut self.resume_follow) {
            self.follow = true;
        }
        match closed {
            Closed::Answered { answer, by } if *by != self.settings.client => {
                let choice = answer.choice().name();
                self.conversation
                    .notice(format!("answered by {by}: {choice}"));
            }
            Closed::Answered { .. } | Closed::Cancelled => {}
            Closed::NoOne => self.conversation.notice("question closed"),
        }
    }

    // Responses.

    fn response(&mut self, request: Request, result: Result<Payload, String>) -> Vec<Effect> {
        let lost = matches!(&result, Err(message) if message == LOST);
        match (request, result) {
            (Request::List, Ok(Payload::Sessions { sessions })) => {
                let before = self.place;
                self.set_place(&sessions);
                if before == self.place {
                    return Vec::new();
                }
            }
            (Request::Send { text, .. }, Err(message)) => {
                self.conversation.unqueue(&text);
                if self.editor.is_empty() {
                    self.editor.set_text(&text);
                }
                if !lost {
                    self.conversation.error(message);
                }
            }
            (Request::Answer { .. }, Err(message)) => {
                if let Some(prompt) = &mut self.prompt {
                    if lost {
                        prompt.sending = false;
                    } else {
                        prompt.rejected(message);
                    }
                }
            }
            (Request::List, Err(_)) => return Vec::new(),
            (Request::Models, Ok(Payload::Models { models, config })) => {
                self.models(models, config);
            }
            (Request::Models, Err(message)) => {
                // A refresh after a reload fails quietly; the old list stays.
                if !std::mem::take(&mut self.picker_wanted) || lost {
                    return Vec::new();
                }
                self.conversation.error(message);
            }
            (_, Err(message)) if !lost => self.conversation.error(message),
            _ => return Vec::new(),
        }
        vec![Effect::Redraw]
    }

    /// The server's models: its context windows count from now on, and the
    /// picker opens if it was asked for.
    fn models(&mut self, models: Vec<ModelInfo>, config: PathBuf) {
        self.settings.windows = models
            .iter()
            .map(|m| (m.model.clone(), m.context_window))
            .collect();
        let wanted = std::mem::take(&mut self.picker_wanted);
        if let Some(picker) = &mut self.picker {
            picker.set_models(models, config);
        } else if wanted && self.dialog.is_none() {
            self.picker = Some(Picker::new(models, config, &self.session.model));
        }
    }

    fn open_picker(&mut self) -> Vec<Effect> {
        self.picker_wanted = true;
        vec![Effect::Request(Request::Models)]
    }

    // Time and connection.

    fn tick(&mut self) -> Vec<Effect> {
        self.ticks = self.ticks.wrapping_add(1);
        let mut effects = Vec::new();
        if self.ticks.is_multiple_of(LIST_TICKS)
            && self.lost_since.is_none()
            && self.ended.is_none()
        {
            effects.push(Effect::Request(Request::List));
        }
        if self.conversation.tick() {
            effects.push(Effect::Redraw);
        }
        effects
    }

    fn disconnected(&mut self) -> Vec<Effect> {
        // A session that ended has nothing to come back to.
        if self.ended.is_some() {
            return vec![Effect::Redraw];
        }
        self.lost_since = Some(self.now);
        vec![Effect::Reconnect, Effect::Redraw]
    }

    fn reconnect_failed(&mut self) -> Vec<Effect> {
        match self.lost_since {
            Some(since) if self.now.duration_since(since) >= RECONNECT_FOR => {
                vec![Effect::Exit(Exit::Lost)]
            }
            Some(_) => vec![Effect::Reconnect],
            None => Vec::new(),
        }
    }

    // Keys.

    fn key(&mut self, key: KeyEvent) -> Vec<Effect> {
        if key.kind == KeyEventKind::Release {
            return Vec::new();
        }
        self.message = None;
        if let Some(reason) = &self.ended {
            return vec![Effect::Exit(Exit::Ended {
                name: self.session.name.clone(),
                reason: reason.clone(),
            })];
        }
        if self.prompt.is_some() {
            return self.prompt_key(key);
        }
        if self.picker.is_some() {
            return self.picker_key(key);
        }
        if let Some(dialog) = self.dialog {
            return self.dialog_key(dialog, key);
        }
        let action = keys::action(key, self.focus);
        let mut effects = self.act(action);
        effects.push(Effect::Redraw);
        effects
    }

    /// A key while a question is open. Besides the prompt's own keys, only
    /// what answers nothing works: scrolling, to read what is asked, and
    /// `Ctrl-D` to leave the question for another client.
    fn prompt_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        if let Some(action) = keys::scrolling(key) {
            let mut effects = self.act(action);
            effects.push(Effect::Redraw);
            return effects;
        }
        let choosing = self
            .prompt
            .as_ref()
            .is_some_and(|p| p.mode == super::prompt::Mode::Choices);
        if choosing && keys::is_detach(key) {
            return vec![self.detach()];
        }
        let mut effects = vec![Effect::Redraw];
        if let Some(prompt) = &mut self.prompt
            && let Some(answer) = prompt.key(key)
        {
            effects.push(Effect::Request(Request::Answer {
                session: self.session.name.clone(),
                question: prompt.id().to_owned(),
                answer,
            }));
        }
        effects
    }

    fn picker_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let mut effects = vec![Effect::Redraw];
        let Some(picker) = &mut self.picker else {
            return effects;
        };
        match key.code {
            KeyCode::Esc => self.picker = None,
            KeyCode::Up => picker.up(),
            KeyCode::Down => picker.down(),
            KeyCode::Enter => {
                let Some(model) = picker.chosen().map(|m| m.model.clone()) else {
                    return effects;
                };
                self.picker = None;
                if model != self.session.model {
                    effects.push(Effect::Request(Request::SetModel {
                        session: self.session.name.clone(),
                        model,
                    }));
                }
            }
            _ => {
                if let Some(edit) = keys::edit(key) {
                    picker.edit(edit);
                }
            }
        }
        effects
    }

    fn dialog_key(&mut self, dialog: Dialog, key: KeyEvent) -> Vec<Effect> {
        let mut effects = vec![Effect::Redraw];
        match dialog {
            Dialog::Help => self.dialog = None,
            Dialog::ConfirmKill => match key.code {
                KeyCode::Char('y' | 'Y') => {
                    self.dialog = None;
                    effects.push(Effect::Request(Request::Kill {
                        session: self.session.name.clone(),
                    }));
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc => self.dialog = None,
                _ => {}
            },
        }
        effects
    }

    fn detach(&self) -> Effect {
        Effect::Exit(Exit::Detached {
            name: self.session.name.clone(),
        })
    }

    fn running(&self) -> bool {
        matches!(self.session.state, State::Working | State::Waiting)
    }

    fn cancel(&self) -> Vec<Effect> {
        if self.running() {
            vec![Effect::Request(Request::Cancel {
                session: self.session.name.clone(),
            })]
        } else {
            Vec::new()
        }
    }

    fn act(&mut self, action: Action) -> Vec<Effect> {
        match action {
            Action::Cancel => return self.cancel(),
            Action::Interrupt => return self.interrupt(),
            Action::Detach if self.editor.is_empty() => return vec![self.detach()],
            Action::PageUp => self.scroll_up(self.page()),
            Action::PageDown => self.scroll_down(self.page()),
            Action::Start => {
                self.follow = false;
                self.top = 0;
            }
            Action::End => {
                self.follow = true;
                self.new_output = false;
            }
            Action::SwitchFocus => self.switch_focus(),
            Action::Send => return self.send(),
            Action::Edit(edit) => self.editor.edit(edit),
            Action::Up => self.up(),
            Action::Down => self.down(),
            Action::Toggle => {
                if let Some(entry) = self.selected
                    && self.conversation.toggle(entry)
                {
                    self.reveal = Some(entry);
                }
            }
            Action::Help => self.dialog = Some(Dialog::Help),
            Action::ModelPicker => return self.open_picker(),
            Action::Detach | Action::Nothing => {}
        }
        Vec::new()
    }

    /// `Ctrl-C`: twice within 1 s detaches; else it cancels the running turn
    /// or, with none, clears the input.
    fn interrupt(&mut self) -> Vec<Effect> {
        if self
            .last_interrupt
            .is_some_and(|at| self.now.duration_since(at) < DOUBLE_INTERRUPT)
        {
            return vec![self.detach()];
        }
        self.last_interrupt = Some(self.now);
        if self.running() {
            return self.cancel();
        }
        self.editor.clear();
        Vec::new()
    }

    fn switch_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Input => {
                if self.selected.is_none() {
                    self.selected = self.conversation.entries.len().checked_sub(1);
                }
                self.reveal = self.selected;
                Focus::Conversation
            }
            Focus::Conversation => Focus::Input,
        };
    }

    fn up(&mut self) {
        match self.focus {
            Focus::Input => {
                if !self.editor.up() {
                    let history = self.conversation.user_messages();
                    self.editor.browse(&history, Browse::Earlier);
                }
            }
            Focus::Conversation => {
                self.selected = self.selected.map(|s| s.saturating_sub(1));
                self.reveal = self.selected;
            }
        }
    }

    fn down(&mut self) {
        match self.focus {
            Focus::Input => {
                if !self.editor.down() {
                    let history = self.conversation.user_messages();
                    self.editor.browse(&history, Browse::Later);
                }
            }
            Focus::Conversation => {
                let last = self.conversation.entries.len().saturating_sub(1);
                self.selected = self.selected.map(|s| (s + 1).min(last));
                self.reveal = self.selected;
            }
        }
    }

    fn paste(&mut self, text: &str) -> Vec<Effect> {
        if let Some(prompt) = &mut self.prompt {
            prompt.paste(text);
        } else if let Some(picker) = &mut self.picker {
            picker.paste(text);
        } else if self.dialog.is_none() && self.ended.is_none() && self.focus == Focus::Input {
            self.editor.paste(text);
        }
        vec![Effect::Redraw]
    }

    fn mouse(&mut self, mouse: MouseEvent) -> Vec<Effect> {
        let step = self.settings.scroll_lines;
        match mouse.kind {
            MouseEventKind::ScrollUp => self.scroll_up(step),
            MouseEventKind::ScrollDown => self.scroll_down(step),
            _ => return Vec::new(),
        }
        vec![Effect::Redraw]
    }

    // Sending and commands.

    fn send(&mut self) -> Vec<Effect> {
        let text = self.editor.text();
        if text.trim().is_empty() {
            return Vec::new();
        }
        if let Some(rest) = text.strip_prefix("//") {
            let message = format!("/{rest}");
            return self.send_message(message);
        }
        if let Some(command) = text.strip_prefix('/') {
            return self.command(command.trim());
        }
        self.send_message(text)
    }

    fn send_message(&mut self, text: String) -> Vec<Effect> {
        if self.lost_since.is_some() {
            self.message = Some("server disconnected; the input is kept".to_owned());
            return Vec::new();
        }
        self.editor.clear();
        self.follow = true;
        self.new_output = false;
        self.conversation.queued.push(text.clone());
        vec![Effect::Request(Request::Send {
            session: self.session.name.clone(),
            text,
        })]
    }

    fn command(&mut self, command: &str) -> Vec<Effect> {
        let (name, argument) = command
            .split_once(char::is_whitespace)
            .map_or((command, ""), |(n, a)| (n, a.trim()));
        let session = self.session.name.clone();
        let effects = match (name, argument) {
            ("help", _) => {
                self.dialog = Some(Dialog::Help);
                Vec::new()
            }
            ("detach" | "quit", _) => {
                self.editor.clear();
                return vec![self.detach()];
            }
            ("model", "") => self.open_picker(),
            ("model", model) => vec![Effect::Request(Request::SetModel {
                session,
                model: model.to_owned(),
            })],
            ("rename", "") => {
                self.conversation.notice("usage: /rename NAME");
                return Vec::new();
            }
            ("rename", name) => vec![Effect::Request(Request::Rename {
                session,
                name: name.to_owned(),
            })],
            ("kill", _) => {
                self.dialog = Some(Dialog::ConfirmKill);
                Vec::new()
            }
            ("clear" | "permissions" | "sessions" | "new" | "attach" | "commit", _) => {
                self.conversation
                    .notice(format!("`/{name}` comes with a later version"));
                Vec::new()
            }
            (other, _) => {
                // The input stays, so that it can be corrected.
                self.conversation.notice(format!(
                    "unknown command /{other}; /help lists them, // sends a message that starts with /"
                ));
                return Vec::new();
            }
        };
        self.editor.clear();
        effects
    }

    // Scrolling.

    fn page(&self) -> usize {
        self.view_height.saturating_sub(1).max(1)
    }

    fn max_top(&self) -> usize {
        self.total.unwrap_or(0).saturating_sub(self.view_height)
    }

    fn scroll_up(&mut self, rows: usize) {
        self.follow = false;
        self.resume_follow = false;
        self.top = self.top.saturating_sub(rows);
    }

    fn scroll_down(&mut self, rows: usize) {
        self.top = self.top.saturating_add(rows);
        if self.top >= self.max_top() {
            self.follow = true;
            self.new_output = false;
        }
    }

    /// Brings the lines and the scroll position up to date for a screen of
    /// `size`. Runs before each frame; drawing then only reads.
    pub fn layout(&mut self, size: Rect) {
        let Some(areas) = view::areas(self, size) else {
            return;
        };
        let width = usize::from(areas.conversation.width);
        self.view_height = usize::from(areas.conversation.height);
        // Keep the entry at the top when the width changes the row numbers.
        let anchor = (self.width.is_some_and(|w| w != width))
            .then(|| self.conversation.entry_at(self.top))
            .flatten();
        self.conversation.layout(width);
        self.width = Some(width);
        if let Some(entry) = anchor
            && let Some((start, _)) = self.conversation.span_of(entry)
        {
            self.top = start;
        }
        let total = self.conversation.total();
        if self.total.is_some_and(|before| total > before) && !self.follow {
            self.new_output = true;
        }
        self.total = Some(total);
        let was_following = self.follow;
        if self.follow {
            self.top = self.max_top();
        }
        if let Some(entry) = self.reveal.take() {
            self.show_entry(entry);
            if was_following && !self.follow && self.prompt.is_some() {
                self.resume_follow = true;
            }
        }
        self.top = self.top.min(self.max_top());
    }

    /// Scrolls as little as needed to show an entry, its start first.
    fn show_entry(&mut self, entry: usize) {
        let Some((start, height)) = self.conversation.span_of(entry) else {
            return;
        };
        let view = self.view_height;
        if start < self.top || height > view {
            self.top = start;
        } else if start + height > self.top + view {
            self.top = start + height - view;
        }
        self.follow = self.top >= self.max_top();
    }

    /// Whether the selected entry is a tool call.
    pub fn selected_is_call(&self) -> bool {
        self.selected
            .and_then(|s| self.conversation.entries.get(s))
            .is_some_and(|e| matches!(e.kind, Kind::Call(_)))
    }
}

/// `HH:MM` of a time in seconds since 1970, `offset` seconds from UTC.
pub fn clock(secs: u64, offset: i64) -> String {
    let secs = i64::try_from(secs).unwrap_or(0).saturating_add(offset);
    let minutes = secs.div_euclid(60);
    let (h, m) = (
        minutes.div_euclid(60).rem_euclid(24),
        minutes.rem_euclid(60),
    );
    format!("{h:02}:{m:02}")
}
