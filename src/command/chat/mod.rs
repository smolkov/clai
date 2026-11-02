//! `clai chat`: a conversation with a session in the terminal, and
//! answering questions without the TUI (see `spec/command.md`, "clai chat").

mod questions;
mod talk;

use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use clap::{Args, ValueHint};
use clap_complete::ArgValueCompleter;
use serde_json::Value;
use tokio::sync::mpsc;

use crate::agent::{Answer, Question, now_secs};
use crate::client::{Client, Connect};
use crate::command::ls::age;
use crate::complete;
use crate::config::{self, Environment, LoadOptions, Symbols};
use crate::error::{Error, Result};
use crate::output::write_stdout;
use crate::protocol::{Request, SessionInfo};
use crate::runtime::current_thread;

/// Words that make `clai chat NAME CHOICE` the script form.
const CHOICES: [&str; 6] = ["allow", "always", "deny", "cancel", "continue", "stop"];

#[derive(Debug, Args)]
pub struct Chat {
    /// Start a new session (also without NAME); takes DIR instead of NAME.
    #[arg(long, conflicts_with_all = ["list", "questions", "id"])]
    pub new: bool,
    /// With --new: the session name.
    #[arg(long, requires = "new")]
    pub name: Option<String>,
    /// The model, as PROVIDER/MODEL: of a new session, or the model to
    /// switch an existing one to (also to resume a saved session whose
    /// model is gone).
    #[arg(
        long,
        value_name = "PROVIDER/MODEL",
        conflicts_with_all = ["list", "questions", "id"],
        add = ArgValueCompleter::new(complete::models)
    )]
    pub model: Option<String>,
    /// Only answer questions of the named sessions (default: all), without
    /// their conversations.
    #[arg(long, conflicts_with_all = ["list", "id"])]
    pub questions: bool,
    /// List the open questions and exit.
    #[arg(long)]
    pub list: bool,
    /// With --list: print the full questions as JSON.
    #[arg(long, requires = "list")]
    pub json: bool,
    /// Answer only if ID is still the open question.
    #[arg(long, value_name = "ID", conflicts_with = "list")]
    pub id: Option<String>,
    /// The session to chat with (default: a new one), DIR with --new,
    /// sessions with --questions or --list, or NAME CHOICE to answer once:
    /// allow, always RULE, deny [MESSAGE...], cancel, continue, stop.
    #[arg(
        value_name = "NAME",
        value_hint = ValueHint::Other,
        add = ArgValueCompleter::new(complete::sessions)
    )]
    pub words: Vec<String>,
}

/// What `clai chat` was asked to do.
#[derive(Debug, PartialEq)]
enum Mode<'a> {
    /// A conversation with an existing session.
    Attach(&'a str),
    /// A conversation with a new session in this directory.
    New(Option<PathBuf>),
    Questions(&'a [String]),
    List(&'a [String]),
    Script {
        name: &'a str,
        answer: Answer,
    },
}

impl Chat {
    pub fn run(&self, config: Option<&Path>) -> Result<()> {
        match self.mode()? {
            Mode::List(names) => self.list(config, names),
            Mode::Script { name, answer } => script(config, name, answer, self.id.as_deref()),
            Mode::Questions(names) => questions::questions(config, names),
            Mode::Attach(name) => talk::talk(
                config,
                talk::Target::Attach {
                    name: name.to_owned(),
                    model: self.model.clone(),
                },
            ),
            Mode::New(dir) => talk::talk(
                config,
                talk::Target::New(super::new::request(
                    self.name.clone(),
                    self.model.clone(),
                    dir.as_deref(),
                )?),
            ),
        }
    }

    fn mode(&self) -> Result<Mode<'_>> {
        if self.list {
            return Ok(Mode::List(&self.words));
        }
        if self.questions {
            return Ok(Mode::Questions(&self.words));
        }
        match self.words.as_slice() {
            [name, choice, rest @ ..] if !self.new && CHOICES.contains(&choice.as_str()) => {
                Ok(Mode::Script {
                    name,
                    answer: parse_choice(choice, rest)?,
                })
            }
            _ if self.id.is_some() => Err(Error::Usage(
                "--id needs a session and a choice: clai chat NAME CHOICE --id ID".to_owned(),
            )),
            [] => Ok(Mode::New(None)),
            [dir] if self.new => Ok(Mode::New(Some(PathBuf::from(dir)))),
            [name] => Ok(Mode::Attach(name)),
            _ if self.new => Err(Error::Usage(
                "clai chat --new takes at most one DIR".to_owned(),
            )),
            _ => Err(Error::Usage(
                "clai chat talks to one session; use --questions to answer several".to_owned(),
            )),
        }
    }

    fn list(&self, config: Option<&Path>, names: &[String]) -> Result<()> {
        let sessions = current_thread()?.block_on(async {
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
    current_thread()?.block_on(async {
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

/// Reads lines from standard input on a thread, one per request, so that
/// nothing reads the terminal while the pager runs. A line is always asked
/// for otherwise, so that the end of input ends `clai chat` at any time.
pub(crate) struct Lines {
    want: std::sync::mpsc::Sender<()>,
    lines: mpsc::UnboundedReceiver<Option<String>>,
    waiting: bool,
}

impl Lines {
    pub(crate) fn start() -> Lines {
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
    pub(crate) fn want(&mut self) {
        if !self.waiting && self.want.send(()).is_ok() {
            self.waiting = true;
        }
    }

    /// The next line; `None` at the end of input.
    pub(crate) async fn next(&mut self) -> Option<String> {
        if !self.waiting {
            return std::future::pending().await;
        }
        let line = self.lines.recv().await.flatten();
        self.waiting = false;
        line
    }
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
            new: false,
            name: None,
            model: None,
            questions: false,
            list: false,
            json: false,
            id: id.map(str::to_owned),
            words: words.iter().map(|w| (*w).to_owned()).collect(),
        }
    }

    #[test]
    fn modes() {
        assert_eq!(chat(&[], None).mode().unwrap(), Mode::New(None));
        assert_eq!(chat(&["a"], None).mode().unwrap(), Mode::Attach("a"));
        let mut c = chat(&["dir"], None);
        c.new = true;
        assert_eq!(c.mode().unwrap(), Mode::New(Some(PathBuf::from("dir"))));
        // With --new, a second word is not a choice.
        let mut c = chat(&["dir", "allow"], None);
        c.new = true;
        assert!(matches!(c.mode(), Err(Error::Usage(_))));
        let mut c = chat(&["a", "b"], None);
        assert!(matches!(c.mode(), Err(Error::Usage(m)) if m.contains("--questions")));
        c.questions = true;
        assert!(matches!(c.mode().unwrap(), Mode::Questions(w) if w.len() == 2));
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
}
