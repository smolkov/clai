//! Answering questions with lines of text (see `spec/command.md`,
//! "clai chat").

use std::io::{BufRead, Write};
use std::sync::{Arc, Mutex, PoisonError};

use futures_util::future::BoxFuture;

use crate::agent::{Asker, Question, Reply};

use super::dialog::{Dialog, Step};
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
        let mut dialog = Dialog::new(question.clone(), false);
        loop {
            let Some(line) = self.read(dialog.prompt()) else {
                return Reply::NoOne;
            };
            match dialog.line(&line) {
                Step::Again(text) => self.write(&text),
                Step::Answer(answer) => {
                    return Reply::Answer {
                        answer,
                        by: BY.to_owned(),
                    };
                }
                // Not offered without a pager.
                Step::View => {}
            }
        }
    }
}
