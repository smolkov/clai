//! Answering a question with lines of text, as a state machine: the same
//! keys in `clai run` and `clai chat` (see `spec/command.md`, "clai chat").

use crate::agent::{Answer, Question, QuestionKind};

use super::question::has_more;

/// What a line of input leads to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Write this text (may be empty), then ask again with [`Dialog::prompt`].
    Again(String),
    /// The answer is complete.
    Answer(Answer),
    /// Show the whole content in the pager, then ask again.
    View,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Choice,
    Message,
    Rule,
}

/// The state of answering one question.
#[derive(Debug, Clone)]
pub struct Dialog {
    question: Question,
    stage: Stage,
    pager: bool,
}

impl Dialog {
    /// `pager`: whether `v` may show cut content.
    pub fn new(question: Question, pager: bool) -> Dialog {
        Dialog {
            question,
            stage: Stage::Choice,
            pager,
        }
    }

    pub fn question(&self) -> &Question {
        &self.question
    }

    /// The prompt for the next line.
    pub fn prompt(&self) -> &'static str {
        match self.stage {
            Stage::Choice => "> ",
            Stage::Message => "message> ",
            Stage::Rule if self.proposals().is_empty() => "rule> ",
            Stage::Rule => "rule [1]> ",
        }
    }

    /// Back to the first prompt, e.g. after an answer was not accepted.
    pub fn restart(&mut self) {
        self.stage = Stage::Choice;
    }

    fn proposals(&self) -> &[Vec<String>] {
        match &self.question.kind {
            QuestionKind::Permission(p) => &p.proposals,
            QuestionKind::Limit(_) => &[],
        }
    }

    /// Takes one line of input, without its newline.
    pub fn line(&mut self, line: &str) -> Step {
        let line = line.trim();
        match self.stage {
            Stage::Choice => self.choice(line),
            Stage::Message => {
                self.stage = Stage::Choice;
                if line.is_empty() {
                    Step::Again(String::new())
                } else {
                    Step::Answer(Answer::DenyMessage {
                        text: line.to_owned(),
                    })
                }
            }
            Stage::Rule => {
                self.stage = Stage::Choice;
                self.rule(line)
            }
        }
    }

    fn choice(&mut self, line: &str) -> Step {
        let key = line.to_ascii_lowercase();
        if key == "v" && self.pager && has_more(&self.question) {
            return Step::View;
        }
        match &self.question.kind {
            QuestionKind::Permission(_) => match key.as_str() {
                "y" | "allow" => Step::Answer(Answer::Allow),
                "n" | "deny" => Step::Answer(Answer::Deny),
                "c" | "cancel" => Step::Answer(Answer::Cancel),
                "e" => {
                    self.stage = Stage::Message;
                    Step::Again(String::new())
                }
                "a" | "always" => {
                    self.stage = Stage::Rule;
                    let mut list = String::new();
                    for (i, rules) in self.proposals().iter().enumerate() {
                        list.push_str(&format!("  {}  {}\n", i + 1, rules.join(" + ")));
                    }
                    Step::Again(list)
                }
                _ => Step::Again("  answer y, a, n, e or c\n".to_owned()),
            },
            QuestionKind::Limit(_) => match key.as_str() {
                "y" | "continue" => Step::Answer(Answer::Continue),
                "n" | "stop" => Step::Answer(Answer::Stop),
                "c" | "cancel" => Step::Answer(Answer::Cancel),
                _ => Step::Again("  answer y, n or c\n".to_owned()),
            },
        }
    }

    /// A proposal's number, `Enter` for the first, or a typed rule.
    fn rule(&self, line: &str) -> Step {
        let proposals = self.proposals();
        if line.is_empty() {
            return match proposals.first() {
                Some(rules) => Step::Answer(Answer::AllowAlways {
                    rules: rules.clone(),
                }),
                None => Step::Again(String::new()),
            };
        }
        if let Ok(n) = line.parse::<usize>() {
            return match n.checked_sub(1).and_then(|i| proposals.get(i)) {
                Some(rules) => Step::Answer(Answer::AllowAlways {
                    rules: rules.clone(),
                }),
                None => Step::Again(format!("  no rule {n}\n")),
            };
        }
        Step::Answer(Answer::AllowAlways {
            rules: vec![line.to_owned()],
        })
    }
}
