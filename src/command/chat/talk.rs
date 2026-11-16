//! `codeagent chat [NAME]`: a conversation with one session in the terminal.
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
  /model [MODEL] --default
                   also save it (or the current one) as default_model
  /status [NAME]   status and statistics of this or another session
  /stats [RANGE]   statistics over all sessions: 7d (default), 30d or all
  /usage           show the tokens used so far
  /sessions        list the sessions; * is this one
  /attach NAME     attach to another session
  /new [NAME] [MODEL]  start a session in this workspace and attach;
                   MODEL is PROVIDER/MODEL, default: default_model
  /models          list the configured models
  /skills [NAME]   list the skills (* active); with a name, turn it on or off
  /lsp             list the language servers
  /lsp restart NAME, /lsp stop NAME
                   restart or stop a language server
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
    /// The last session of the request's workspace, with its model changed
    /// to `model`; without one, a session from the request.
    Continue {
        request: Request,
        model: Option<String>,
    },
}

pub fn talk(ctx: &Context, target: Target) -> Result<()> {
    let Terminal { unicode, colour } = Terminal::detect(symbols(ctx), &ctx.startup);
    current_thread()?.block_on(async {
        let mut client = Client::connect(&Connect::chat(ctx, true)).await?;
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
            Target::Continue { request, model } => {
                crate::command::common::continue_session(&mut client, &ctx.startup, request, model)
                    .await?
            }
        };
        let session = attach(&mut client, &name).await?;
        let mut talk = Talk {
            pager: super::pager_command(ctx),
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

/// What `/stats` prints.
fn stats_text(stats: &crate::protocol::GlobalStats, unicode: bool, offset: i64) -> String {
    let options = crate::text::status::Options {
        unicode,
        offset,
        me: CLIENT_NAME,
    };
    crate::text::global::text(stats, &options)
}

/// The name this client has for the server.
const CLIENT_NAME: &str = "codeagent chat";

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

/// `/sessions`: the table of `codeagent ls`, the attached session marked `*`.
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

/// `/models`: as `codeagent models` prints them.
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
    /// Leave `codeagent chat`; the session keeps running unless it ended.
    Leave,
}

/// The skills as `/skills` prints them: `*` before active ones, a
/// shadowed one with its winner, an active one not found any more, then
/// the warnings.
fn skills_text(info: &crate::protocol::SkillsInfo, workspace: &std::path::Path) -> String {
    let show = |p: &std::path::Path| crate::skills::display(p, workspace, info.home.as_deref());
    let active = |name: &str| info.active.iter().find(|a| a.name == name);
    let mut rows: Vec<Vec<String>> = info
        .active
        .iter()
        .filter(|a| a.missing)
        .map(|a| {
            vec![
                format!("* {}", a.name),
                show(&a.dir),
                "not found any more".to_owned(),
            ]
        })
        .collect();
    for skill in &info.skills {
        let (mark, about) = match (&skill.shadowed_by, active(&skill.name)) {
            (Some(by), _) => (" ", format!("shadowed by {}", show(by))),
            (None, Some(a)) if a.changed_on_disk && !a.missing => {
                ("*", format!("changed on disk; {}", skill.description))
            }
            (None, Some(a)) if !a.missing => ("*", skill.description.clone()),
            (None, _) => (" ", skill.description.clone()),
        };
        rows.push(vec![
            format!("{mark} {}", skill.name),
            show(&skill.dir),
            about,
        ]);
    }
    let mut out = String::new();
    if rows.is_empty() {
        out.push_str("no skills found; searched:\n");
        for dir in &info.dirs {
            out.push_str(&format!("  {}\n", show(dir)));
        }
    }
    for line in crate::output::columns(
        &rows,
        &[crate::output::Align::Left, crate::output::Align::Left],
    ) {
        out.push_str(&line);
        out.push('\n');
    }
    for warning in &info.warnings {
        out.push_str(&format!("warning: {warning}\n"));
    }
    out
}

struct Talk {
    /// The pager for the whole question.
    pager: String,
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
    /// `/skills`: the skills found, `*` before the active ones; `/skills
    /// NAME`: turns that skill on or off. The change prints when its event
    /// comes.
    async fn skills(&mut self, argument: &str) -> Result<()> {
        let info = match self
            .client
            .request(Request::Skills {
                workspace: self.session.path.clone(),
                session: Some(self.session.name.clone()),
            })
            .await?
        {
            Payload::Skills(info) => *info,
            other => return Err(unexpected(&other)),
        };
        if argument.is_empty() {
            let text = skills_text(&info, &self.session.path);
            self.out(&text);
            return Ok(());
        }
        let mut names: Vec<String> = info.active.iter().map(|a| a.name.clone()).collect();
        if names.iter().any(|n| n == argument) {
            names.retain(|n| n != argument);
        } else {
            names.push(argument.to_owned());
        }
        self.client
            .request(Request::SetSkills {
                session: self.session.name.clone(),
                names,
            })
            .await?;
        Ok(())
    }

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

    /// `/new`, `/attach`, `/sessions`, `/models`, `/status`, `/stats`; `None` for other
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
            "skills" => self.skills(argument).await,
            "models" => match self
                .client
                .request(Request::Models {
                    workspace: Some(self.session.path.clone()),
                })
                .await
            {
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
            "stats" => match crate::protocol::Range::parse(argument)
                .or_else(|| argument.is_empty().then(crate::protocol::Range::default))
            {
                Some(range) => {
                    let offset = crate::tui::local_offset();
                    match self
                        .client
                        .request(Request::GlobalStats { range, offset })
                        .await
                    {
                        Ok(Payload::GlobalStats { stats }) => {
                            self.out(&stats_text(&stats, self.unicode, offset));
                            Ok(())
                        }
                        Ok(other) => Err(unexpected(&other)),
                        Err(e) => Err(e),
                    }
                }
                None => Err(Error::Usage("usage: /stats [7d|30d|all]".into())),
            },
            "lsp" => return Some(self.lsp(argument).await),
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
                        skills: Vec::new(),
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
                "detached; {} keeps working; `codeagent chat {}` returns to it",
                self.session.name, self.session.name
            ));
        } else {
            note(&format!(
                "detached; `codeagent chat {}` returns to {}",
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
                pager(
                    &self.pager,
                    full_content(dialog.question()).unwrap_or_default(),
                );
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
                        let again = dialog.restart();
                        self.out(&again);
                        self.dialog = Some(dialog.clone());
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        self.prompt();
        Ok(Flow::Go)
    }

    /// `/lsp`, `/lsp restart NAME`, `/lsp stop NAME`.
    async fn lsp(&mut self, argument: &str) -> Result<()> {
        let usage = || Error::Usage("usage: /lsp [restart NAME | stop NAME]".into());
        let workspace = self.session.path.clone();
        let request = match argument.split_once(char::is_whitespace) {
            None if argument.is_empty() => Request::LspStatus,
            Some((action @ ("restart" | "stop"), name)) => {
                let name = name.trim();
                let servers = match self.client.request(Request::LspStatus).await? {
                    Payload::Lsp { servers } => servers,
                    other => return Err(unexpected(&other)),
                };
                // The one for this workspace first.
                let Some(server) = servers
                    .iter()
                    .filter(|s| s.name == name)
                    .min_by_key(|s| !s.root.starts_with(&workspace))
                else {
                    return Err(Error::Usage(format!(
                        "no language server {name}; /lsp lists them"
                    )));
                };
                let (name, root) = (server.name.clone(), server.root.clone());
                if action == "restart" {
                    Request::LspRestart { name, root }
                } else {
                    Request::LspStop { name, root }
                }
            }
            _ => return Err(usage()),
        };
        match self.client.request(request).await? {
            Payload::Lsp { servers } => {
                self.out(&lsp_text(&servers, &workspace, self.unicode));
                Ok(())
            }
            other => Err(unexpected(&other)),
        }
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
            "model" => match crate::tui::model_argument(argument) {
                (model, true) => return self.model_default(model).await,
                (None, false) => {
                    let text = format!("model: {}", self.session.model);
                    self.note(&text);
                    self.prompt();
                    return Ok(Flow::Go);
                }
                (Some(model), false) => Request::SetModel { session, model },
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

    /// `/model [MODEL] --default`: switches to `MODEL` if given, then saves
    /// it (or the session's model) as `default_model` in the user file the
    /// server reads (RFC 0021).
    async fn model_default(&mut self, model: Option<String>) -> Result<Flow> {
        if let Some(model) = &model {
            let request = Request::SetModel {
                session: self.session.name.clone(),
                model: model.clone(),
            };
            if let Err(e) = self.client.request(request).await {
                self.note(&e.to_string());
                self.prompt();
                return Ok(Flow::Go);
            }
        }
        let workspace = self.session.path.clone();
        let answer = self
            .client
            .request(Request::Models {
                workspace: Some(workspace.clone()),
            })
            .await;
        let path = match answer {
            Ok(Payload::Models { config, .. }) => config,
            Ok(other) => return Err(unexpected(&other)),
            Err(e) => {
                self.note(&e.to_string());
                self.prompt();
                return Ok(Flow::Go);
            }
        };
        let options = crate::config::LoadOptions {
            config_file: Some(path.clone()),
            workspace,
            env: crate::config::Environment::from_process(),
        };
        let target = model.unwrap_or_else(|| self.session.model.clone());
        match crate::config::save_default_model(&options, &path, &target) {
            Ok(saved) => {
                let warning = if self.unicode { "⚠" } else { "!" };
                for line in saved.notes(warning) {
                    self.note(&line);
                }
            }
            Err(e) => self.note(&format!("default model not saved: {e}")),
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

/// The language servers, one line each, with the last error line of a
/// failed one.
pub fn lsp_text(
    servers: &[crate::protocol::LspServerInfo],
    workspace: &std::path::Path,
    unicode: bool,
) -> String {
    if servers.is_empty() {
        return "no language server has run; one starts when the model uses lsp\n".to_owned();
    }
    let dash = if unicode { "—" } else { "-" };
    let roots: Vec<String> = servers
        .iter()
        .map(|s| match s.root.strip_prefix(workspace) {
            Ok(rest) if rest.as_os_str().is_empty() => ".".to_owned(),
            Ok(rest) => rest.display().to_string(),
            Err(_) => s.root.display().to_string(),
        })
        .collect();
    let name_width = servers
        .iter()
        .map(|s| s.name.chars().count())
        .max()
        .unwrap_or(0);
    let root_width = roots.iter().map(|r| r.chars().count()).max().unwrap_or(0);
    let mut out = String::new();
    for (server, root) in servers.iter().zip(&roots) {
        let mut line = format!(
            "{:<name_width$}  {root:<root_width$}  {}",
            server.name,
            server.state_text()
        );
        if !server.sessions.is_empty() {
            line.push_str(&format!(" {dash} {}", server.sessions.join(", ")));
        }
        out.push_str(line.trim_end());
        out.push('\n');
        if server.state == "failed"
            && let Some(last) = server.stderr.last()
        {
            out.push_str(&format!("  {last}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lsp_lists_servers_with_state_and_sessions() {
        use crate::protocol::LspServerInfo;
        let ws = std::path::Path::new("/ws/codeagent");
        assert_eq!(
            lsp_text(&[], ws, true),
            "no language server has run; one starts when the model uses lsp\n"
        );
        let ready = LspServerInfo {
            name: "rust-analyzer".into(),
            root: "/ws/codeagent".into(),
            state: "ready".into(),
            percent: None,
            reason: None,
            sessions: vec!["codeagent".into(), "fix".into()],
            stderr: vec!["ignored".into()],
        };
        let failed = LspServerInfo {
            name: "gopls".into(),
            root: "/ws/go".into(),
            state: "failed".into(),
            reason: Some("exited".into()),
            sessions: Vec::new(),
            stderr: vec!["a".into(), "error: no go.mod".into()],
            ..ready.clone()
        };
        assert_eq!(
            lsp_text(&[ready, failed], ws, false),
            "rust-analyzer  .       ready - codeagent, fix\n\
             gopls          /ws/go  failed: exited\n  error: no go.mod\n"
        );
    }

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
            updated: 0,
            turns: 0,
            queued: 0,
            question: None,
            usage: TokenTotals::default(),
            tmp: None,
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
    fn models_as_codeagent_models_prints_them() {
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
    fn skills_mark_the_active_ones() {
        let skill = |dir: &str, name: &str, shadowed: Option<&str>| crate::protocol::SkillInfo {
            name: name.into(),
            description: format!("About {name}."),
            dir: format!("{dir}/{name}").into(),
            source: crate::skills::Source::User,
            shadowed_by: shadowed.map(Into::into),
        };
        let active = |name: &str, missing: bool| crate::protocol::ActiveSkill {
            name: name.into(),
            dir: format!("/old/{name}").into(),
            tokens: 10,
            changed_on_disk: false,
            missing,
        };
        let info = crate::protocol::SkillsInfo {
            skills: vec![
                skill("/w/.claude/skills", "pdf", None),
                skill("/h/.claude/skills", "pdf", Some("/w/.claude/skills/pdf")),
                skill("/h/.claude/skills", "release", None),
            ],
            dirs: Vec::new(),
            warnings: vec!["x/SKILL.md: no description".into()],
            active: vec![active("gone", true), active("pdf", false)],
            home: Some("/h".into()),
        };
        assert_eq!(
            skills_text(&info, std::path::Path::new("/w")),
            "* gone     /old/gone                 not found any more\n\
             * pdf      .claude/skills/pdf        About pdf.\n\
             \x20 pdf      ~/.claude/skills/pdf      shadowed by .claude/skills/pdf\n\
             \x20 release  ~/.claude/skills/release  About release.\n\
             warning: x/SKILL.md: no description\n"
        );
        let none = crate::protocol::SkillsInfo {
            dirs: vec!["/h/.claude/skills".into()],
            home: Some("/h".into()),
            ..Default::default()
        };
        assert_eq!(
            skills_text(&none, std::path::Path::new("/w")),
            "no skills found; searched:\n  ~/.claude/skills\n"
        );
    }

    #[test]
    fn status_marks_this_client() {
        let mut status = crate::text::status::tests::saved();
        status.session.state = State::Idle;
        status.clients = vec![crate::protocol::ClientInfo {
            name: "codeagent chat".into(),
            answers: true,
        }];
        let text = status_text(&status, true, 0);
        assert!(
            text.contains("clients      1: codeagent chat (this)"),
            "{text}"
        );
        assert!(text.contains("⚠ not configured"), "{text}");
    }

    #[test]
    fn stats_print_totals_models_and_days() {
        let text = stats_text(&crate::text::global::tests::week(), true, 0);
        assert!(
            text.starts_with("Totals         2026-11-04 – 2026-11-10\n"),
            "{text}"
        );
        assert!(text.contains("  anthropic/sonnet       9"), "{text}");
        assert!(text.contains("  Sun 11-08    4.1M  ███"), "{text}");
    }
}
