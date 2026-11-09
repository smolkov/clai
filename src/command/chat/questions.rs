//! `clai chat --questions`: answer the questions of several sessions,
//! without their conversations.

use std::collections::HashMap;
use std::io::{self, Write};

use crate::agent::{Closed, Question, SessionEvent};
use crate::client::{Client, Connect, unexpected};
use crate::error::{Error, Result};
use crate::output::note;
use crate::protocol::{Notice, Payload, Request, ServerMessage, SessionInfo};
use crate::runtime::current_thread;
use crate::text::{Dialog, Step, Style, Terminal, full_content, render_question_styled};

use super::{Lines, pager, symbols};
use crate::command::Context;

/// A question waiting to be shown.
struct Open {
    session: u64,
    question: Question,
}

struct Interactive {
    client: Client,
    lines: Lines,
    /// Open questions not shown yet, oldest first.
    queue: Vec<Open>,
    current: Option<(u64, Dialog)>,
    /// Attached sessions by id: their names.
    sessions: HashMap<u64, String>,
    /// Attach to sessions created later.
    follow: bool,
    said_waiting: bool,
    unicode: bool,
    colour: bool,
}

pub fn questions(ctx: &Context, names: &[String]) -> Result<()> {
    let Terminal { unicode, colour } = Terminal::detect(symbols(ctx));
    current_thread()?.block_on(async {
        let mut client = Client::connect(&Connect::chat(ctx.config(), true)).await?;
        let all = client.list().await?;
        let chosen: Vec<&SessionInfo> = if names.is_empty() {
            all.iter().collect()
        } else {
            let mut chosen = Vec::new();
            for name in names {
                let session = all
                    .iter()
                    .find(|s| &s.name == name)
                    .ok_or_else(|| Error::Failed(format!("no session named {name}")))?;
                chosen.push(session);
            }
            chosen
        };
        let mut chat = Interactive {
            client,
            lines: Lines::start(),
            queue: Vec::new(),
            current: None,
            sessions: HashMap::new(),
            follow: names.is_empty(),
            said_waiting: false,
            unicode,
            colour,
        };
        for session in chosen {
            chat.attach(&session.name).await?;
        }
        chat.run().await
    })
}

impl Interactive {
    async fn attach(&mut self, name: &str) -> Result<()> {
        let session = match self
            .client
            .request(Request::Attach {
                session: name.to_owned(),
            })
            .await?
        {
            Payload::Session { session } => session,
            other => return Err(unexpected(&other)),
        };
        self.sessions.insert(session.id, session.name.clone());
        if let Some(question) = session.question
            && !question.choices.is_empty()
        {
            self.add(session.id, question);
        }
        Ok(())
    }

    async fn run(mut self) -> Result<()> {
        self.show_next();
        loop {
            self.lines.want();
            tokio::select! {
                message = self.client.next() => match message? {
                    Some(message) => {
                        if !self.message(message).await? {
                            return Ok(());
                        }
                    }
                    None => {
                        return Err(Error::Failed("the server closed the connection".into()));
                    }
                },
                line = self.lines.next() => match line {
                    Some(line) => self.line(&line).await?,
                    None => return Ok(()),
                },
            }
        }
    }

    fn out(&self, text: &str) {
        let mut stdout = io::stdout().lock();
        let _ = stdout.write_all(text.as_bytes());
        let _ = stdout.flush();
    }

    fn add(&mut self, session: u64, question: Question) {
        if self.queue.iter().any(|o| o.question.id == question.id) {
            return;
        }
        self.queue.push(Open { session, question });
        self.queue.sort_by_key(|o| o.question.opened);
    }

    fn style(&self, question: &Question) -> Style {
        Style {
            name: question.session.name.clone(),
            place: question.session.workspace.clone(),
            unicode: self.unicode,
            colour: self.colour,
            pager: true,
        }
    }

    /// Shows the oldest open question, or says that there is none.
    fn show_next(&mut self) {
        if self.current.is_some() {
            return;
        }
        if self.queue.is_empty() {
            if !self.said_waiting {
                let _ = writeln!(io::stderr(), "waiting for questions (Ctrl-C to stop)");
                self.said_waiting = true;
            }
            return;
        }
        let open = self.queue.remove(0);
        self.said_waiting = false;
        let text = render_question_styled(&open.question, &self.style(&open.question));
        let dialog = Dialog::new(open.question, true);
        self.out(&text);
        self.out(dialog.prompt());
        self.current = Some((open.session, dialog));
    }

    /// Handles a message from the server; `false` ends `clai chat`.
    async fn message(&mut self, message: ServerMessage) -> Result<bool> {
        match message {
            // The log of an attached session: its open question came with
            // the attach response.
            ServerMessage::Event { replay: true, .. } => {}
            ServerMessage::Event { session, event, .. } => self.event(session, event),
            ServerMessage::Notice { notice } => match notice {
                Notice::SessionCreated { session } if self.follow => {
                    self.attach(&session.name).await?;
                }
                Notice::Stopping => {
                    note("the server is stopping");
                    return Ok(false);
                }
                Notice::ConfigRejected { errors } => {
                    for error in errors {
                        note(&format!("configuration rejected: {error}"));
                    }
                }
                Notice::SessionCreated { .. } | Notice::ConfigReloaded => {}
            },
            ServerMessage::Error { message } => return Err(Error::Failed(message)),
            ServerMessage::Hello(_) | ServerMessage::Response { .. } => {}
        }
        Ok(true)
    }

    fn event(&mut self, session: u64, event: SessionEvent) {
        match event {
            SessionEvent::Question(question) if !question.choices.is_empty() => {
                self.add(session, *question);
                self.show_next();
            }
            SessionEvent::QuestionClosed { id, closed } => {
                self.queue.retain(|o| o.question.id != id);
                let shown = self
                    .current
                    .as_ref()
                    .is_some_and(|(_, d)| d.question().id == id);
                if shown {
                    let text = match closed {
                        Closed::Answered { answer, by } => {
                            format!("\nanswered in {by}: {}\n", answer.choice().name())
                        }
                        Closed::Cancelled => "\nclosed: the turn was cancelled\n".to_owned(),
                        Closed::NoOne => "\nclosed\n".to_owned(),
                    };
                    self.out(&text);
                    self.current = None;
                    self.show_next();
                }
            }
            SessionEvent::Renamed { name } => {
                self.sessions.insert(session, name.clone());
                for open in &mut self.queue {
                    if open.session == session {
                        open.question.session.name.clone_from(&name);
                    }
                }
            }
            SessionEvent::Ended { reason } => {
                let name = self.sessions.remove(&session).unwrap_or_default();
                self.queue.retain(|o| o.session != session);
                if self.current.as_ref().is_some_and(|(s, _)| *s == session) {
                    self.out(&format!("\nsession {name} ended: {reason}\n"));
                    self.current = None;
                    self.show_next();
                }
            }
            _ => {}
        }
    }

    async fn line(&mut self, line: &str) -> Result<()> {
        let Some((session, mut dialog)) = self.current.take() else {
            // No question shown; nothing to answer.
            return Ok(());
        };
        match dialog.line(line) {
            Step::Again(text) => {
                self.out(&text);
                self.out(dialog.prompt());
                self.current = Some((session, dialog));
            }
            Step::View => {
                pager(full_content(dialog.question()).unwrap_or_default());
                self.out(dialog.prompt());
                self.current = Some((session, dialog));
            }
            Step::Answer(answer) => {
                let name = self.sessions.get(&session).cloned().unwrap_or_default();
                let request = Request::Answer {
                    session: name,
                    question: dialog.question().id.clone(),
                    answer,
                };
                match self.client.request(request).await {
                    Ok(_) => self.show_next(),
                    Err(Error::Failed(message)) | Err(Error::Usage(message)) => {
                        self.out(&format!("  not accepted: {message}\n"));
                        let again = dialog.restart();
                        self.out(&again);
                        self.out(dialog.prompt());
                        self.current = Some((session, dialog));
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        Ok(())
    }
}
