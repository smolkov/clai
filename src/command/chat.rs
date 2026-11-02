//! `clai chat`: answer questions of sessions in the terminal (see
//! `spec/command.md`, "clai chat").

use std::collections::HashMap;
use std::io::{self, BufRead, IsTerminal, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use clap::Args;
use clap_complete::ArgValueCompleter;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::agent::{Answer, Closed, Question, SessionEvent, now_secs};
use crate::client::{Client, Connect, runtime, unexpected};
use crate::command::ls::age;
use crate::complete;
use crate::config::{self, Environment, LoadOptions, Symbols};
use crate::error::{Error, Result};
use crate::output::write_stdout;
use crate::protocol::{Notice, Payload, Request, ServerMessage, SessionInfo};
use crate::text::{Dialog, Step, Style, full_content, render_question_styled, unicode};

/// Words that make `clai chat NAME CHOICE` the script form.
const CHOICES: [&str; 6] = ["allow", "always", "deny", "cancel", "continue", "stop"];

#[derive(Debug, Args)]
pub struct Chat {
    /// List the open questions and exit.
    #[arg(long)]
    pub list: bool,
    /// With --list: print the full questions as JSON.
    #[arg(long, requires = "list")]
    pub json: bool,
    /// Answer only if ID is still the open question.
    #[arg(long, value_name = "ID", conflicts_with = "list")]
    pub id: Option<String>,
    /// Sessions to answer (default: all), or NAME CHOICE to answer once:
    /// allow, always RULE, deny [MESSAGE...], cancel, continue, stop.
    #[arg(value_name = "NAME", add = ArgValueCompleter::new(complete::sessions))]
    pub words: Vec<String>,
}

/// What `clai chat` was asked to do.
#[derive(Debug, PartialEq)]
enum Mode<'a> {
    Interactive(&'a [String]),
    List(&'a [String]),
    Script { name: &'a str, answer: Answer },
}

impl Chat {
    pub fn run(&self, config: Option<&Path>) -> Result<()> {
        match self.mode()? {
            Mode::List(names) => self.list(config, names),
            Mode::Script { name, answer } => script(config, name, answer, self.id.as_deref()),
            Mode::Interactive(names) => interactive(config, names),
        }
    }

    fn mode(&self) -> Result<Mode<'_>> {
        if self.list {
            return Ok(Mode::List(&self.words));
        }
        match self.words.as_slice() {
            [name, choice, rest @ ..] if CHOICES.contains(&choice.as_str()) => Ok(Mode::Script {
                name,
                answer: parse_choice(choice, rest)?,
            }),
            _ if self.id.is_some() => Err(Error::Usage(
                "--id needs a session and a choice: clai chat NAME CHOICE --id ID".to_owned(),
            )),
            words => Ok(Mode::Interactive(words)),
        }
    }

    fn list(&self, config: Option<&Path>, names: &[String]) -> Result<()> {
        let sessions = runtime()?.block_on(async {
            let mut client = Client::connect(&Connect::chat(config, false)).await?;
            client.list().await
        })?;
        let unknown: Vec<&String> = names
            .iter()
            .filter(|n| !sessions.iter().any(|s| &s.name == *n))
            .collect();
        let questions: Vec<(&SessionInfo, &Question)> = sessions
            .iter()
            .filter(|s| names.is_empty() || names.contains(&s.name))
            .filter_map(|s| s.question.as_ref().map(|q| (s, q)))
            .collect();
        let text = if self.json {
            let list: Vec<Value> = questions
                .iter()
                .filter_map(|(_, q)| serde_json::to_value(q).ok())
                .collect();
            format!("{}\n", Value::Array(list))
        } else {
            list_lines(&questions, now_secs())
        };
        write_stdout(&text)?;
        if unknown.is_empty() {
            return Ok(());
        }
        for name in unknown {
            let _ = writeln!(io::stderr(), "clai: no session named {name}");
        }
        Err(Error::Exit(1))
    }
}

/// `clai chat NAME CHOICE ...` as an answer.
fn parse_choice(choice: &str, rest: &[String]) -> Result<Answer> {
    let text = rest.join(" ");
    let text = text.trim();
    let no_more = |answer: Answer| {
        if text.is_empty() {
            Ok(answer)
        } else {
            Err(Error::Usage(format!("`{choice}` takes no more words")))
        }
    };
    match choice {
        "allow" => no_more(Answer::Allow),
        "cancel" => no_more(Answer::Cancel),
        "continue" => no_more(Answer::Continue),
        "stop" => no_more(Answer::Stop),
        "deny" if text.is_empty() => Ok(Answer::Deny),
        "deny" => Ok(Answer::DenyMessage {
            text: text.to_owned(),
        }),
        "always" if text.is_empty() => Err(Error::Usage(
            "`always` needs a rule, e.g. clai chat NAME always 'run_command(cargo test)'"
                .to_owned(),
        )),
        "always" => Ok(Answer::AllowAlways {
            rules: vec![text.to_owned()],
        }),
        other => Err(Error::Usage(format!("unknown choice `{other}`"))),
    }
}

/// `session  id  age  summary`, aligned.
fn list_lines(questions: &[(&SessionInfo, &Question)], now: u64) -> String {
    let rows: Vec<[String; 4]> = questions
        .iter()
        .map(|(s, q)| {
            [
                s.name.clone(),
                q.id.clone(),
                age(now.saturating_sub(q.opened)),
                q.summary.clone(),
            ]
        })
        .collect();
    let width = |i: usize| {
        rows.iter()
            .filter_map(|r| r.get(i))
            .map(|c| c.chars().count())
            .max()
            .unwrap_or(0)
    };
    let (w0, w1, w2) = (width(0), width(1), width(2));
    rows.iter()
        .map(|[name, id, age, summary]| format!("{name:<w0$}  {id:<w1$}  {age:<w2$}  {summary}\n"))
        .collect()
}

fn script(config: Option<&Path>, name: &str, answer: Answer, id: Option<&str>) -> Result<()> {
    runtime()?.block_on(async {
        let mut client = Client::connect(&Connect::chat(config, false)).await?;
        let sessions = client.list().await?;
        let session = sessions
            .iter()
            .find(|s| s.name == name)
            .ok_or_else(|| Error::Failed(format!("no session named {name}")))?;
        let question = session
            .question
            .as_ref()
            .ok_or_else(|| Error::Failed(format!("{name} has no open question")))?;
        if let Some(id) = id
            && id != question.id
        {
            return Err(Error::Failed(format!(
                "the open question of {name} is {}, not {id}",
                question.id
            )));
        }
        client
            .request(Request::Answer {
                session: name.to_owned(),
                question: question.id.clone(),
                answer,
            })
            .await
            .map(|_| ())
    })
}

/// The name of a choice as written in answers: `allow`, `deny_message`.
fn choice_name(answer: &Answer) -> String {
    serde_json::to_value(answer.choice())
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Reads lines from standard input on a thread, one per request, so that
/// nothing reads the terminal while the pager runs. A line is always asked
/// for otherwise, so that the end of input ends `clai chat` at any time.
struct Lines {
    want: std::sync::mpsc::Sender<()>,
    lines: mpsc::UnboundedReceiver<Option<String>>,
    waiting: bool,
}

impl Lines {
    fn start() -> Lines {
        let (want, want_rx) = std::sync::mpsc::channel::<()>();
        let (tx, lines) = mpsc::unbounded_channel();
        std::thread::spawn(move || {
            let stdin = io::stdin();
            while want_rx.recv().is_ok() {
                let mut line = String::new();
                let line = match stdin.lock().read_line(&mut line) {
                    Ok(0) | Err(_) => None,
                    Ok(_) => Some(line),
                };
                let end = line.is_none();
                if tx.send(line).is_err() || end {
                    return;
                }
            }
        });
        Lines {
            want,
            lines,
            waiting: false,
        }
    }

    /// Asks for the next line, unless one is already asked for.
    fn want(&mut self) {
        if !self.waiting && self.want.send(()).is_ok() {
            self.waiting = true;
        }
    }

    /// The next line; `None` at the end of input.
    async fn next(&mut self) -> Option<String> {
        if !self.waiting {
            return std::future::pending().await;
        }
        let line = self.lines.recv().await.flatten();
        self.waiting = false;
        line
    }
}

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

fn interactive(config: Option<&Path>, names: &[String]) -> Result<()> {
    let unicode = unicode(symbols(config), |name| std::env::var(name).ok());
    let colour = io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    runtime()?.block_on(async {
        let mut client = Client::connect(&Connect::chat(config, true)).await?;
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

/// `tui.symbols` from the configuration, if it loads.
fn symbols(config: Option<&Path>) -> Symbols {
    let options = LoadOptions {
        config_file: config.map(Path::to_path_buf),
        workspace: std::env::current_dir().unwrap_or_default(),
        env: Environment::from_process(),
    };
    config::load(&options).map_or(Symbols::Auto, |c| c.tui.symbols)
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
                    let _ = writeln!(io::stderr(), "clai: the server is stopping");
                    return Ok(false);
                }
                Notice::ConfigRejected { errors } => {
                    for error in errors {
                        let _ = writeln!(io::stderr(), "clai: configuration rejected: {error}");
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
                            format!("\nanswered in {by}: {}\n", choice_name(&answer))
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
                        dialog.restart();
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

/// Shows `text` through `$PAGER` (default `less`).
fn pager(text: &str) {
    let pager = std::env::var("PAGER")
        .ok()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| "less".to_owned());
    let child = Command::new("sh")
        .args(["-c", &pager])
        .stdin(Stdio::piped())
        .spawn();
    match child {
        Ok(mut child) => {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(text.as_bytes());
            }
            let _ = child.wait();
        }
        Err(e) => {
            let _ = writeln!(io::stderr(), "clai: pager {pager}: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chat(words: &[&str], id: Option<&str>) -> Chat {
        Chat {
            list: false,
            json: false,
            id: id.map(str::to_owned),
            words: words.iter().map(|w| (*w).to_owned()).collect(),
        }
    }

    #[test]
    fn modes() {
        let c = chat(&[], None);
        assert_eq!(c.mode().unwrap(), Mode::Interactive(&[]));
        let c = chat(&["a", "b"], None);
        assert!(matches!(c.mode().unwrap(), Mode::Interactive(w) if w.len() == 2));
        let c = chat(&["a", "allow"], Some("q3"));
        assert_eq!(
            c.mode().unwrap(),
            Mode::Script {
                name: "a",
                answer: Answer::Allow
            }
        );
        let c = chat(&["a", "deny", "use", "b.txt"], None);
        assert_eq!(
            c.mode().unwrap(),
            Mode::Script {
                name: "a",
                answer: Answer::DenyMessage {
                    text: "use b.txt".into()
                }
            }
        );
        let c = chat(&["a", "always", "run_command(cargo test)"], None);
        assert_eq!(
            c.mode().unwrap(),
            Mode::Script {
                name: "a",
                answer: Answer::AllowAlways {
                    rules: vec!["run_command(cargo test)".into()]
                }
            }
        );
        for (words, id) in [
            (&["a", "always"][..], None),
            (&["a", "allow", "x"][..], None),
            (&["a"][..], Some("q1")),
        ] {
            assert!(
                matches!(chat(words, id).mode(), Err(Error::Usage(_))),
                "{words:?}"
            );
        }
    }

    #[test]
    fn choice_names() {
        assert_eq!(choice_name(&Answer::Allow), "allow");
        assert_eq!(
            choice_name(&Answer::DenyMessage { text: "x".into() }),
            "deny_message"
        );
    }
}
