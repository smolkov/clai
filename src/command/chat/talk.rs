//! `clai chat [NAME]`: a conversation with one session in the terminal.
//! Lines typed are messages; the session's questions are answered inline.

use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use tokio::signal::unix::{SignalKind, signal};

use crate::agent::{Closed, Question, SessionEvent, State};
use crate::client::{Client, Connect, runtime, unexpected};
use crate::command::new::create;
use crate::error::{Error, Result};
use crate::protocol::{Notice, Payload, Request, ServerMessage, SessionInfo};
use crate::text::{
    Conversation, Dialog, Step, Style, full_content, render_question_styled, unicode,
};

use super::{Lines, choice_name, pager, symbols};

/// A second `Ctrl-C` within this time detaches.
const DOUBLE_INTERRUPT: Duration = Duration::from_secs(1);

const HELP: &str = "\
commands:
  /cancel          cancel the running turn
  /model [MODEL]   show or change the model (PROVIDER/MODEL)
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

pub fn talk(config: Option<&Path>, target: Target) -> Result<()> {
    let unicode = unicode(symbols(config), |name| std::env::var(name).ok());
    let colour = io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    runtime()?.block_on(async {
        let mut client = Client::connect(&Connect::chat(config, true)).await?;
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
        let session = match client
            .request(Request::Attach {
                session: name.clone(),
            })
            .await?
        {
            Payload::Session { session } => session,
            other => return Err(unexpected(&other)),
        };
        let _ = writeln!(
            io::stderr(),
            "clai: attached to {} ({}, {}); /help lists commands, Ctrl-D detaches",
            session.name,
            session.workspace,
            session.model
        );
        let mut talk = Talk {
            client,
            lines: Lines::start(),
            conversation: Conversation::new(unicode, colour),
            session: *session,
            dialog: None,
            unicode,
            colour,
            interrupted: None,
        };
        talk.replay().await?;
        talk.run().await
    })
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

    async fn run(mut self) -> Result<()> {
        let mut interrupts = signal(SignalKind::interrupt()).map_err(|source| Error::Io {
            context: "signal handler".to_owned(),
            source,
        })?;
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
            let _ = writeln!(
                io::stderr(),
                "clai: detached; {} keeps working; `clai chat {}` returns to it",
                self.session.name,
                self.session.name
            );
        } else {
            let _ = writeln!(
                io::stderr(),
                "clai: detached; `clai chat {}` returns to {}",
                self.session.name,
                self.session.name
            );
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

    fn event(&mut self, event: SessionEvent, replay: bool) -> Flow {
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
                            format!("answered in {by}: {}", choice_name(answer))
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
            SessionEvent::ModelChanged { model } => {
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
