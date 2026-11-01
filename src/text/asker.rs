//! Answering questions with lines of text (see `spec/command.md`,
//! "clai chat").

use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex, PoisonError};

use futures_util::future::BoxFuture;

use crate::agent::{Answer, Asker, Question, QuestionKind, Reply};

use super::question::render_question;

type Input = Arc<Mutex<Box<dyn BufRead + Send>>>;
type Output = Arc<Mutex<Box<dyn Write + Send>>>;

/// Shows each question on `output` and reads the answer from `input`.
/// Reading blocks, so it runs on a blocking thread.
#[derive(Clone)]
pub struct LineAsker {
    input: Input,
    output: Output,
    place: String,
    unicode: bool,
}

/// Who answered, in `question_closed`.
const BY: &str = "terminal";

impl LineAsker {
    pub fn new(
        input: Box<dyn BufRead + Send>,
        output: Box<dyn Write + Send>,
        place: String,
        unicode: bool,
    ) -> LineAsker {
        LineAsker {
            input: Arc::new(Mutex::new(input)),
            output: Arc::new(Mutex::new(output)),
            place,
            unicode,
        }
    }
}

impl Asker for LineAsker {
    fn ask<'a>(&'a self, question: &'a Question) -> BoxFuture<'a, Reply> {
        let this = self.clone();
        let question = question.clone();
        Box::pin(async move {
            tokio::task::spawn_blocking(move || this.dialog(&question))
                .await
                .unwrap_or(Reply::NoOne)
        })
    }

    fn reject(&self, _question: &Question, _by: &str, message: &str) {
        self.write(&format!("  not accepted: {message}\n"));
    }
}

impl LineAsker {
    fn write(&self, text: &str) {
        let mut out = self.output.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = out.write_all(text.as_bytes());
        let _ = out.flush();
    }

    /// A line without its newline; `None` at the end of input.
    fn read(&self, prompt: &str) -> Option<String> {
        self.write(prompt);
        let mut line = String::new();
        let mut input = self.input.lock().unwrap_or_else(PoisonError::into_inner);
        match input.read_line(&mut line) {
            Ok(0) | Err(_) => None,
            Ok(_) => Some(line.trim().to_owned()),
        }
    }

    /// Shows the question and reads until there is an answer.
    fn dialog(&self, question: &Question) -> Reply {
        self.write(&render_question(question, &self.place, self.unicode));
        loop {
            let Some(line) = self.read("> ") else {
                return Reply::NoOne;
            };
            let answer = match &question.kind {
                QuestionKind::Permission(p) => match line.to_ascii_lowercase().as_str() {
                    "y" | "allow" => Some(Answer::Allow),
                    "n" | "deny" => Some(Answer::Deny),
                    "c" | "cancel" => Some(Answer::Cancel),
                    "e" => match self.read("message> ") {
                        None => return Reply::NoOne,
                        Some(text) if text.is_empty() => None,
                        Some(text) => Some(Answer::DenyMessage { text }),
                    },
                    "a" | "always" => match self.rules(&p.proposals) {
                        Rules::Eof => return Reply::NoOne,
                        Rules::Back => None,
                        Rules::Chosen(rules) => Some(Answer::AllowAlways { rules }),
                    },
                    _ => {
                        self.write("  answer y, a, n, e or c\n");
                        None
                    }
                },
                QuestionKind::Limit(_) => match line.to_ascii_lowercase().as_str() {
                    "y" | "continue" => Some(Answer::Continue),
                    "n" | "stop" => Some(Answer::Stop),
                    "c" | "cancel" => Some(Answer::Cancel),
                    _ => {
                        self.write("  answer y, n or c\n");
                        None
                    }
                },
            };
            if let Some(answer) = answer {
                return Reply::Answer {
                    answer,
                    by: BY.to_owned(),
                };
            }
        }
    }

    /// Lists the proposals and reads a number or a typed rule.
    fn rules(&self, proposals: &[Vec<String>]) -> Rules {
        let mut list = String::new();
        for (i, rules) in proposals.iter().enumerate() {
            list.push_str(&format!("  {}  {}\n", i + 1, rules.join(" + ")));
        }
        self.write(&list);
        let prompt = if proposals.is_empty() {
            "rule> ".to_owned()
        } else {
            "rule [1]> ".to_owned()
        };
        let Some(line) = self.read(&prompt) else {
            return Rules::Eof;
        };
        if line.is_empty() {
            return match proposals.first() {
                Some(rules) => Rules::Chosen(rules.clone()),
                None => Rules::Back,
            };
        }
        if let Ok(n) = line.parse::<usize>() {
            return match n.checked_sub(1).and_then(|i| proposals.get(i)) {
                Some(rules) => Rules::Chosen(rules.clone()),
                None => {
                    self.write(&format!("  no rule {n}\n"));
                    Rules::Back
                }
            };
        }
        Rules::Chosen(vec![line])
    }
}

enum Rules {
    Chosen(Vec<String>),
    Back,
    Eof,
}
