//! `clai chat [NAME]`: a conversation with one session in the terminal.
//! Lines typed are messages; the session's questions are answered inline.

use std::io::{self, Write};
use std::time::{Duration, Instant};

use tokio::signal::unix::{SignalKind, signal};

use crate::agent::{Closed, Question, SessionEvent, State, now_secs};
use crate::client::{Client, Connect, unexpected};
use crate::command::common::create;
use crate::error::{Error, Result};
use crate::output::note;
use crate::protocol::{
    ModelInfo, Notice, Payload, Request, ServerMessage, SessionInfo, TokenTotals,
};
use crate::runtime::current_thread;
use crate::text::sessions::table;
use crate::text::{
    Conversation, Dialog, Step, Style, Terminal, full_content, render_question_styled,
    short_tokens, usage_line,
};

use super::{Lines, pager, symbols};
use crate::command::Context;

/// A second `Ctrl-C` within this time detaches.
const DOUBLE_INTERRUPT: Duration = Duration::from_secs(1);

const HELP: &str = "\
commands:
  /cancel          cancel the running turn
  /model [MODEL]   show or change the model (PROVIDER/MODEL)
  /status [NAME]   status and statistics of this or another session
  /usage           show the tokens used so far
  /sessions        list the sessions; * is this one
  /attach NAME     attach to another session
  /new [NAME] [MODEL]  start a session in this workspace and attach;
                   MODEL is PROVIDER/MODEL, default: default_model
  /models          list the configured models
  /detach          leave; the session keeps running (also Ctrl-D)
  /kill            end the session and leave
  /help            this list
Ctrl-C cancels the running turn; twice when idle detaches.
";

/// Which session to talk to.
pub enum Target {
    /// An existing session; `model` switches it first.
    Attach { name: String, model: Option<String> },
    /// A `new` request.
    New(Request),
}

pub fn talk(ctx: &Context, target: Target) -> Result<()> {
    let Terminal { unicode, colour } = Terminal::detect(symbols(ctx));
    current_thread()?.block_on(async {
        let mut client = Client::connect(&Connect::chat(ctx.config(), true)).await?;
        let name = match target {
            Target::Attach { name, model } => {
                if let Some(model) = model {
                    client
                        .request(Request::SetModel {
                            session: name.clone(),
                            model,
                        })
                        .await?;
                }
                name
            }
            Target::New(request) => create(&mut client, request).await?.name,
        };
        let session = attach(&mut client, &name).await?;
        let mut talk = Talk {
            client,
            lines: Lines::start(),
            conversation: Conversation::new(unicode, colour),
            session,
            usage: TokenTotals::default(),
            context_tokens: 0,
            dialog: None,
            unicode,
            colour,
            interrupted: None,
        };
        talk.replay().await?;
        talk.run().await
    })
}

/// What `/status` prints.
fn status_text(status: &crate::protocol::SessionStatus, unicode: bool, offset: i64) -> String {
    let options = crate::text::status::Options {
        unicode,
        offset,
        me: CLIENT_NAME,
    };
    crate::text::status::text(&crate::text::status::sections(status, &options))
}

/// The name this client has for the server.
const CLIENT_NAME: &str = "clai chat";

/// Attaches and says so on standard error.
async fn attach(client: &mut Client, name: &str) -> Result<SessionInfo> {
    let session = match client
        .request(Request::Attach {
            session: name.to_owned(),
        })
        .await?
    {
        Payload::Session { session } => *session,
        other => return Err(unexpected(&other)),
    };
    note(&format!(
        "attached to {} ({}, {}); /help lists commands, Ctrl-D detaches",
        session.name, session.workspace, session.model
    ));
    Ok(session)
}

/// The words of `/new`: a word with a `/` is the model, another the name.
fn new_words(argument: &str) -> std::result::Result<(Option<String>, Option<String>), String> {
    let (mut name, mut model) = (None, None);
    for word in argument.split_whitespace() {
        let slot = if word.contains('/') {
            &mut model
        } else {
            &mut name
        };
        if slot.is_some() {
            return Err("usage: /new [NAME] [PROVIDER/MODEL]".to_owned());
        }
        *slot = Some(word.to_owned());
    }
    Ok((name, model))
}

/// `/sessions`: the table of `clai ls`, the attached session marked `*`.
fn sessions_text(sessions: &[SessionInfo], attached: u64, now: u64) -> String {
    let mut out = String::new();
    for (line, session) in table(sessions, now).lines().zip(sessions) {
        let mark = if session.id == attached { '*' } else { ' ' };
        out.push_str(&format!("{mark} {line}\n"));
    }
    if out.is_empty() {
        out.push_str("no sessions\n");
    }
    out
}

/// `/models`: as `clai models` prints them.
fn models_text(models: &[ModelInfo]) -> String {
    let width = |f: fn(&ModelInfo) -> usize| models.iter().map(f).max().unwrap_or(0);
    let w_model = width(|m| m.model.chars().count());
    let w_id = width(|m| m.id.chars().count());
    let w_window = width(|m| short_tokens(m.context_window).len());
    let mut out = String::new();
    for m in models {
        let mut marks = Vec::new();
        if m.default {
            marks.push("default");
        }
        if m.key == "missing" {
            marks.push("no key");
        }
        let line = format!(
            "{:<w_model$}  {:<w_id$}  {:>w_window$}  {}",
            m.model,
            m.id,
            short_tokens(m.context_window),
            marks.join(", ")
        );
        out.push_str(line.trim_end());
        out.push('\n');
    }
    if out.is_empty() {
        out.push_str("no models are configured\n");
    }
    out
}

/// What happens after a line or an event.
enum Flow {
    Go,
    /// Leave `clai chat`; the session keeps running unless it ended.
    Leave,
}

struct Talk {
    client: Client,
    lines: Lines,
    conversation: Conversation,
    /// Name, id, model and state, kept up to date from events.
    session: SessionInfo,
    /// The sum of the log's `usage` events.
    usage: TokenTotals,
    /// Tokens of the last request.
    context_tokens: u64,
    /// The open question being answered.
    dialog: Option<Dialog>,
    unicode: bool,
    colour: bool,
    /// When `Ctrl-C` was last pressed while idle.
    interrupted: Option<Instant>,
}

impl Talk {
    fn out(&self, text: &str) {
        let mut stdout = io::stdout().lock();
        let _ = stdout.write_all(text.as_bytes());
        let _ = stdout.flush();
    }

    fn note(&mut self, text: &str) {
        let line = self.conversation.note(text);
        self.out(&line);
    }

    /// Shows the history the session had before this attach, then the
    /// open question or the prompt.
    async fn replay(&mut self) -> Result<()> {
        // Answered after the whole log, so the backlog holds all of it.
        let sessions = self.client.list().await?;
        if let Some(now) = sessions.into_iter().find(|s| s.id == self.session.id) {
            self.session = now;
        }
        let mut open: Option<Question> = None;
        for message in self.client.take_backlog() {
            match message {
                ServerMessage::Event {
                    session,
                    replay: true,
                    event,
                } if session == self.session.id => match &event {
                    SessionEvent::Usage { .. } => self.count(&event),
                    SessionEvent::Question(q) => open = Some((**q).clone()),
                    SessionEvent::QuestionClosed { id, .. } => {
                        open = open.filter(|q| q.id != *id);
                    }
                    _ => {
                        let text = self.conversation.event(&event, true);
                        self.out(&text);
                    }
                },
                other => {
                    if let Flow::Leave = self.message(other).await? {
                        return Err(Error::Exit(1));
                    }
                }
            }
        }
        match open {
            Some(question) if !question.choices.is_empty() => self.ask(question),
            _ => self.prompt(),
        }
        Ok(())
    }

    /// Attaches to another session and shows its history; the one left
    /// keeps running, and its open question stays for other clients.
    async fn switch_to(&mut self, name: &str) -> Result<()> {
        let left = self.session.name.clone();
        let end = self.conversation.break_line();
        self.out(end);
        let session = attach(&mut self.client, name).await?;
        // Its events still on their way are not this session's, so they are
        // not shown.
        let _ = self.client.request(Request::Detach { session: left }).await;
        self.session = session;
        self.conversation = Conversation::new(self.unicode, self.colour);
        self.usage = TokenTotals::default();
        self.context_tokens = 0;
        self.dialog = None;
        self.interrupted = None;
        self.replay().await
    }

    /// `/new`, `/attach`, `/sessions`, `/models`, `/status`; `None` for other
    /// commands.
    async fn session_command(&mut self, name: &str, argument: &str) -> Option<Result<()>> {
        let result = match name {
            "sessions" => match self.client.list().await {
                Ok(sessions) => {
                    let text = sessions_text(&sessions, self.session.id, now_secs());
                    self.out(&text);
                    Ok(())
                }
                Err(e) => Err(e),
            },
            "models" => match self.client.request(Request::Models).await {
                Ok(Payload::Models { models, .. }) => {
                    self.out(&models_text(&models));
                    Ok(())
                }
                Ok(other) => Err(unexpected(&other)),
                Err(e) => Err(e),
            },
            "status" => {
                let session = if argument.is_empty() {
                    self.session.name.clone()
                } else {
                    argument.to_owned()
                };
                match self
                    .client
                    .request(Request::SessionStatus { session })
                    .await
                {
                    Ok(Payload::SessionStatus { status }) => {
                        let text = status_text(&status, self.unicode, crate::tui::local_offset());
                        self.out(&text);
                        Ok(())
                    }
                    Ok(other) => Err(unexpected(&other)),
                    Err(e) => Err(e),
                }
            }
            "attach" if argument.is_empty() => Err(Error::Usage(
                "usage: /attach NAME; /sessions lists them".into(),
            )),
            "attach" if argument == self.session.name => {
                Err(Error::Usage(format!("already attached to {argument}")))
            }
            "attach" => return Some(self.switch_to(argument).await),
            "new" => match new_words(argument) {
                Ok((name, model)) => {
                    let request = Request::New {
                        workspace: self.session.path.clone(),
                        name,
                        model,
                    };
                    match create(&mut self.client, request).await {
                        Ok(session) => return Some(self.switch_to(&session.name).await),
                        Err(e) => Err(e),
                    }
                }
                Err(usage) => Err(Error::Usage(usage)),
            },
            _ => return None,
        };
        if let Err(e) = result {
            self.note(&e.to_string());
        }
        self.prompt();
        Some(Ok(()))
    }

    async fn run(mut self) -> Result<()> {
        let mut interrupts =
            signal(SignalKind::interrupt()).map_err(Error::io("signal handler"))?;
        loop {
            self.lines.want();
            let flow = tokio::select! {
                message = self.client.next() => match message? {
                    Some(message) => self.message(message).await?,
                    None => return Err(Error::Failed("the server closed the connection".into())),
                },
                line = self.lines.next() => match line {
                    Some(line) => self.line(&line).await?,
                    None => Flow::Leave,
                },
                _ = interrupts.recv() => self.interrupt().await?,
            };
            if let Flow::Leave = flow {
                return self.leave();
            }
        }
    }

    /// Says how to come back, unless the session has ended.
    fn leave(&mut self) -> Result<()> {
        let end = self.conversation.break_line();
        self.out(end);
        if self.session.state != State::Idle || self.dialog.is_some() {
            note(&format!(
                "detached; {} keeps working; `clai chat {}` returns to it",
                self.session.name, self.session.name
            ));
        } else {
            note(&format!(
                "detached; `clai chat {}` returns to {}",
                self.session.name, self.session.name
            ));
        }
        Ok(())
    }

    /// `> ` when the session waits for a message, the question's prompt
    /// when one is open, nothing while it works.
    fn prompt(&mut self) {
        let prompt = match &self.dialog {
            Some(dialog) => match dialog.prompt() {
                "> " => "? ",
                other => other,
            },
            None if self.session.state == State::Idle => "> ",
            None => return,
        };
        let text = format!("{}{prompt}", self.conversation.break_line());
        self.out(&text);
    }

    fn ask(&mut self, question: Question) {
        let style = Style {
            name: self.session.name.clone(),
            place: self.session.workspace.clone(),
            unicode: self.unicode,
            colour: self.colour,
            pager: true,
        };
        let text = format!(
            "{}{}",
            self.conversation.break_line(),
            render_question_styled(&question, &style)
        );
        self.out(&text);
        self.dialog = Some(Dialog::new(question, true));
        self.prompt();
    }

    async fn message(&mut self, message: ServerMessage) -> Result<Flow> {
        match message {
            ServerMessage::Event { session, .. } if session != self.session.id => {}
            ServerMessage::Event { event, replay, .. } => return Ok(self.event(event, replay)),
            ServerMessage::Notice { notice } => match notice {
                Notice::ConfigRejected { errors } => {
                    for error in errors {
                        self.note(&format!("configuration rejected: {error}"));
                    }
                }
                Notice::ConfigReloaded => self.note("configuration reloaded"),
                Notice::Stopping => self.note("the server is stopping"),
                Notice::SessionCreated { .. } => {}
            },
            ServerMessage::Error { message } => return Err(Error::Failed(message)),
            ServerMessage::Hello(_) | ServerMessage::Response { .. } => {}
        }
        Ok(Flow::Go)
    }

    fn count(&mut self, event: &SessionEvent) {
        if let SessionEvent::Usage { context_tokens, .. } = event {
            self.usage.count(event);
            self.context_tokens = *context_tokens;
        }
    }

    fn event(&mut self, event: SessionEvent, replay: bool) -> Flow {
        self.count(&event);
        if replay {
            let text = self.conversation.event(&event, true);
            self.out(&text);
            return Flow::Go;
        }
        match &event {
            SessionEvent::Question(question) if !question.choices.is_empty() => {
                self.ask((**question).clone());
            }
            SessionEvent::QuestionClosed { id, closed } => {
                if self.dialog.as_ref().is_some_and(|d| d.question().id == *id) {
                    self.dialog = None;
                    let text = match closed {
                        Closed::Answered { answer, by } => {
                            format!("answered in {by}: {}", answer.choice().name())
                        }
                        Closed::Cancelled => "question closed: the turn was cancelled".to_owned(),
                        Closed::NoOne => "question closed".to_owned(),
                    };
                    self.note(&text);
                    self.prompt();
                }
            }
            SessionEvent::State { state } => {
                self.session.state = *state;
                if *state == State::Idle {
                    self.prompt();
                }
            }
            SessionEvent::ModelChanged { model, .. } => {
                self.session.model.clone_from(model);
                let text = self.conversation.event(&event, false);
                self.out(&text);
            }
            SessionEvent::Renamed { name } => {
                self.session.name.clone_from(name);
                let text = self.conversation.event(&event, false);
                self.out(&text);
            }
            SessionEvent::Ended { .. } => {
                self.session.state = State::Idle;
                self.dialog = None;
                let text = self.conversation.event(&event, false);
                self.out(&text);
                return Flow::Leave;
            }
            _ => {
                let text = self.conversation.event(&event, false);
                self.out(&text);
            }
        }
        Flow::Go
    }

    async fn line(&mut self, line: &str) -> Result<Flow> {
        if let Some(mut dialog) = self.dialog.take() {
            return self.answer_line(&mut dialog, line).await;
        }
        let text = line.trim();
        if text.is_empty() {
            self.prompt();
            return Ok(Flow::Go);
        }
        if let Some(command) = text.strip_prefix('/') {
            return self.command(command).await;
        }
        self.conversation.sent(text);
        self.session.state = State::Working;
        let request = Request::Send {
            session: self.session.name.clone(),
            text: text.to_owned(),
        };
        if let Err(e) = self.client.request(request).await {
            self.note(&e.to_string());
            self.prompt();
        }
        Ok(Flow::Go)
    }

    async fn answer_line(&mut self, dialog: &mut Dialog, line: &str) -> Result<Flow> {
        match dialog.line(line) {
            Step::Again(text) => {
                self.out(&text);
                self.dialog = Some(dialog.clone());
            }
            Step::View => {
                pager(full_content(dialog.question()).unwrap_or_default());
                self.dialog = Some(dialog.clone());
            }
            Step::Answer(answer) => {
                let request = Request::Answer {
                    session: self.session.name.clone(),
                    question: dialog.question().id.clone(),
                    answer,
                };
                match self.client.request(request).await {
                    Ok(_) => return Ok(Flow::Go),
                    Err(Error::Failed(message) | Error::Usage(message)) => {
                        self.out(&format!("  not accepted: {message}\n"));
                        dialog.restart();
                        self.dialog = Some(dialog.clone());
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        self.prompt();
        Ok(Flow::Go)
    }

    async fn command(&mut self, command: &str) -> Result<Flow> {
        let (name, argument) = command
            .split_once(char::is_whitespace)
            .map_or((command, ""), |(n, a)| (n, a.trim()));
        if let Some(result) = self.session_command(name, argument).await {
            return match result {
                Ok(()) => Ok(Flow::Go),
                // The session left is still attached; the others are lost.
                Err(e @ (Error::Failed(_) | Error::Usage(_))) => {
                    self.note(&e.to_string());
                    self.prompt();
                    Ok(Flow::Go)
                }
                Err(e) => Err(e),
            };
        }
        let session = self.session.name.clone();
        let request = match name {
            "help" => {
                self.out(HELP);
                self.prompt();
                return Ok(Flow::Go);
            }
            "detach" | "quit" | "exit" => return Ok(Flow::Leave),
            "cancel" => Request::Cancel { session },
            "kill" => Request::Kill { session },
            "model" if argument.is_empty() => {
                let text = format!("model: {}", self.session.model);
                self.note(&text);
                self.prompt();
                return Ok(Flow::Go);
            }
            "model" => Request::SetModel {
                session,
                model: argument.to_owned(),
            },
            "usage" => {
                let text = usage_line(self.usage, self.context_tokens);
                self.note(&text);
                self.prompt();
                return Ok(Flow::Go);
            }
            other => {
                self.note(&format!("unknown command /{other}; /help lists them"));
                self.prompt();
                return Ok(Flow::Go);
            }
        };
        let kill = matches!(request, Request::Kill { .. });
        match self.client.request(request).await {
            Ok(_) if kill => {
                self.session.state = State::Idle;
                self.note(&format!("session {} ended", self.session.name));
                return Ok(Flow::Leave);
            }
            Ok(_) => {}
            Err(e) => self.note(&e.to_string()),
        }
        self.prompt();
        Ok(Flow::Go)
    }

    /// `Ctrl-C`: cancels a running turn; when idle, twice detaches.
    async fn interrupt(&mut self) -> Result<Flow> {
        if self.session.state != State::Idle {
            self.interrupted = None;
            let request = Request::Cancel {
                session: self.session.name.clone(),
            };
            if let Err(e) = self.client.request(request).await {
                self.note(&e.to_string());
            }
            return Ok(Flow::Go);
        }
        if self
            .interrupted
            .is_some_and(|at| at.elapsed() < DOUBLE_INTERRUPT)
        {
            return Ok(Flow::Leave);
        }
        self.interrupted = Some(Instant::now());
        self.note("Ctrl-C again or Ctrl-D to detach");
        self.prompt();
        Ok(Flow::Go)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_takes_a_name_and_a_model_in_any_order() {
        assert_eq!(new_words(""), Ok((None, None)));
        assert_eq!(new_words("spike"), Ok((Some("spike".into()), None)));
        assert_eq!(
            new_words("local/qwen"),
            Ok((None, Some("local/qwen".into())))
        );
        let both = Ok((Some("spike".into()), Some("local/qwen".into())));
        assert_eq!(new_words("spike local/qwen"), both);
        assert_eq!(new_words(" local/qwen  spike "), both);
        for wrong in ["a b", "x/y z/w", "a b/c d"] {
            assert!(new_words(wrong).is_err(), "{wrong}");
        }
    }

    fn info(id: u64, name: &str) -> SessionInfo {
        SessionInfo {
            id,
            name: name.into(),
            workspace: "~/ws".into(),
            path: "/home/u/ws".into(),
            model: "test/m".into(),
            state: State::Idle,
            created: 0,
            queued: 0,
            question: None,
            usage: TokenTotals::default(),
        }
    }

    #[test]
    fn the_session_table_marks_this_one() {
        let text = sessions_text(&[info(1, "a"), info(2, "bb")], 2, 120);
        assert_eq!(
            text,
            "  a   ~/ws  test/m  idle  2m\n* bb  ~/ws  test/m  idle  2m\n"
        );
        assert_eq!(sessions_text(&[], 1, 0), "no sessions\n");
    }

    #[test]
    fn models_as_clai_models_prints_them() {
        let model = |model: &str, id: &str, default: bool, key: &str| ModelInfo {
            model: model.into(),
            provider: String::new(),
            name: String::new(),
            id: id.into(),
            context_window: 32_768,
            default,
            key: key.into(),
        };
        let text = models_text(&[
            model("anthropic/sonnet", "claude-sonnet-5-5", true, "missing"),
            model("local/qwen", "qwen3", false, "env"),
        ]);
        assert_eq!(
            text,
            "anthropic/sonnet  claude-sonnet-5-5  32k  default, no key\n\
             local/qwen        qwen3              32k\n"
        );
        assert_eq!(models_text(&[]), "no models are configured\n");
    }

    #[test]
    fn status_marks_this_client() {
        let mut status = crate::text::status::tests::saved();
        status.session.state = State::Idle;
        status.clients = vec![crate::protocol::ClientInfo {
            name: "clai chat".into(),
            answers: true,
        }];
        let text = status_text(&status, true, 0);
        assert!(text.contains("clients      1: clai chat (this)"), "{text}");
        assert!(text.contains("⚠ not configured"), "{text}");
    }
}
