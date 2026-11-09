//! Answering a question with lines of text, as a state machine: the same
//! keys in `clai run` and `clai chat` (see `spec/command.md`, "clai chat").

use crate::agent::{Answer, AskUser, Question, QuestionKind};
use crate::tools::UserAnswer;

use super::question::{ask_block, has_more};

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
    /// A question from the model: all answered, send them?
    Send,
}

/// The state of answering one question.
#[derive(Debug, Clone)]
pub struct Dialog {
    question: Question,
    stage: Stage,
    pager: bool,
    /// Answers so far to a question from the model.
    answers: Vec<UserAnswer>,
}

impl Dialog {
    /// `pager`: whether `v` may show cut content.
    pub fn new(question: Question, pager: bool) -> Dialog {
        Dialog {
            question,
            stage: Stage::Choice,
            pager,
            answers: Vec::new(),
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
            Stage::Send => "send [y/n]> ",
        }
    }

    /// Back to the first prompt, e.g. after an answer was not accepted;
    /// returns the text to show again (the first question from the model).
    pub fn restart(&mut self) -> String {
        self.stage = Stage::Choice;
        self.answers.clear();
        match &self.question.kind {
            QuestionKind::AskUser(a) => ask_block(a, 0),
            _ => String::new(),
        }
    }

    fn proposals(&self) -> &[Vec<String>] {
        match &self.question.kind {
            QuestionKind::Permission(p) => &p.proposals,
            QuestionKind::Limit(_) | QuestionKind::AskUser(_) => &[],
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
            Stage::Send => match line.to_ascii_lowercase().as_str() {
                "y" | "yes" | "" => Step::Answer(Answer::Reply {
                    answers: self.answers.clone(),
                }),
                "n" | "no" => Step::Again(self.restart()),
                "c" | "cancel" => Step::Answer(Answer::Cancel),
                _ => Step::Again("  answer y, n or c\n".to_owned()),
            },
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
            QuestionKind::AskUser(a) => {
                let a = a.clone();
                self.ask_line(&a, line)
            }
        }
    }

    /// A line answering the current question from the model.
    fn ask_line(&mut self, a: &AskUser, line: &str) -> Step {
        let index = self.answers.len();
        let Some(q) = a.questions.get(index) else {
            return Step::Again(String::new());
        };
        match line.to_ascii_lowercase().as_str() {
            "" => return Step::Again(String::new()),
            "s" | "skip" => return Step::Answer(Answer::Decline),
            "c" | "cancel" => return Step::Answer(Answer::Cancel),
            _ => {}
        }
        let answer = match parse_answer(line, q.options.len(), q.multiple) {
            Ok(answer) => answer,
            Err(message) => return Step::Again(format!("  {message}\n")),
        };
        self.answers.push(answer);
        if self.answers.len() < a.questions.len() {
            return Step::Again(ask_block(a, self.answers.len()));
        }
        if a.questions.len() == 1 {
            return Step::Answer(Answer::Reply {
                answers: self.answers.clone(),
            });
        }
        self.stage = Stage::Send;
        Step::Again(review(a, &self.answers))
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

/// `2`, `1,3`, `1 3: text`, or a typed answer.
fn parse_answer(line: &str, options: usize, multiple: bool) -> Result<UserAnswer, String> {
    let (head, tail) = match line.split_once(':') {
        Some((head, tail)) => (head, Some(tail)),
        None => (line, None),
    };
    let is_numbers = head.chars().any(|c| c.is_ascii_digit())
        && head
            .chars()
            .all(|c| c.is_ascii_digit() || c == ',' || c.is_whitespace());
    if !is_numbers {
        return Ok(UserAnswer {
            selected: Vec::new(),
            text: Some(line.trim().to_owned()),
        });
    }
    let mut selected = Vec::new();
    for word in head.split(|c: char| c == ',' || c.is_whitespace()) {
        if word.is_empty() {
            continue;
        }
        let n: usize = word.parse().map_err(|_| format!("no option {word}"))?;
        if n == 0 || n > options {
            return Err(format!("no option {n}; choose 1–{options}"));
        }
        if !selected.contains(&(n - 1)) {
            selected.push(n - 1);
        }
    }
    if !multiple && selected.len() > 1 {
        return Err("choose one option".to_owned());
    }
    let text = tail
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_owned);
    Ok(UserAnswer { selected, text })
}

/// The answers before sending, one line per question.
fn review(a: &AskUser, answers: &[UserAnswer]) -> String {
    let mut out = String::from("Your answers:\n");
    for (i, (q, answer)) in a.questions.iter().zip(answers).enumerate() {
        let mut parts: Vec<String> = answer
            .selected
            .iter()
            .filter_map(|&k| q.options.get(k))
            .map(|o| o.label.clone())
            .collect();
        if let Some(text) = answer.typed() {
            parts.push(format!("\"{text}\""));
        }
        out.push_str(&format!(
            "  {}. {}: {}\n",
            i + 1,
            q.header,
            parts.join(", ")
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::agent::{ASK_USER_CHOICES, AskCall, SessionRef, TurnInfo};
    use crate::tools::ask_user;

    fn question(questions: serde_json::Value) -> Question {
        let arguments = json!({ "questions": questions });
        let asked = ask_user::check(&arguments).unwrap();
        Question {
            id: "q1".into(),
            session: SessionRef::default(),
            opened: 0,
            turn: TurnInfo {
                number: 1,
                requests: 1,
                tool_calls: 1,
            },
            summary: ask_user::summary(&asked.questions),
            choices: ASK_USER_CHOICES.to_vec(),
            kind: QuestionKind::AskUser(Box::new(AskUser {
                call: AskCall {
                    tool: "ask_user".into(),
                    id: "c1".into(),
                    arguments,
                },
                questions: asked.questions,
                context: None,
                text: None,
            })),
        }
    }

    fn two() -> Question {
        question(json!([
            {"question": "Which database?", "header": "Database", "options": [{"label": "SQLite"}, {"label": "Postgres"}]},
            {"question": "Which features?", "header": "Features", "options": [{"label": "Search"}, {"label": "Export"}, {"label": "Import"}], "multiple": true}
        ]))
    }

    fn reply(answers: &[(&[usize], Option<&str>)]) -> Step {
        Step::Answer(Answer::Reply {
            answers: answers
                .iter()
                .map(|(s, t)| UserAnswer {
                    selected: s.to_vec(),
                    text: t.map(str::to_owned),
                })
                .collect(),
        })
    }

    #[test]
    fn one_question_sends_at_once() {
        let q =
            question(json!([{"question": "Which?", "options": [{"label": "A"}, {"label": "B"}]}]));
        assert_eq!(
            Dialog::new(q.clone(), false).line("2"),
            reply(&[(&[1], None)])
        );
        assert_eq!(
            Dialog::new(q.clone(), false).line("Redis please"),
            reply(&[(&[], Some("Redis please"))])
        );
        assert_eq!(
            Dialog::new(q.clone(), false).line("1: but keep it small"),
            reply(&[(&[0], Some("but keep it small"))])
        );
        assert_eq!(
            Dialog::new(q.clone(), false).line("s"),
            Step::Answer(Answer::Decline)
        );
        assert_eq!(
            Dialog::new(q.clone(), false).line("C"),
            Step::Answer(Answer::Cancel)
        );
        let mut d = Dialog::new(q, false);
        assert_eq!(
            d.line("3"),
            Step::Again("  no option 3; choose 1–2\n".into())
        );
        assert_eq!(d.line("1,2"), Step::Again("  choose one option\n".into()));
        assert_eq!(d.line(""), Step::Again(String::new()));
        assert_eq!(d.line("1"), reply(&[(&[0], None)]));
    }

    #[test]
    fn several_questions_are_reviewed() {
        let mut d = Dialog::new(two(), false);
        let Step::Again(next) = d.line("1") else {
            panic!("expected the next question")
        };
        assert!(
            next.starts_with("? Question from the model (2/2): Which features?"),
            "{next}"
        );
        assert_eq!(
            d.line("1, 3 3: keep the old format"),
            Step::Again(
                "Your answers:\n  1. Database: SQLite\n  2. Features: Search, Import, \"keep the old format\"\n"
                    .into()
            )
        );
        assert_eq!(d.prompt(), "send [y/n]> ");
        assert_eq!(
            d.line("y"),
            reply(&[(&[0], None), (&[0, 2], Some("keep the old format"))])
        );

        // `n` starts again at the first question.
        let mut d = Dialog::new(two(), false);
        d.line("2");
        d.line("1");
        let Step::Again(again) = d.line("n") else {
            panic!("expected the first question")
        };
        assert!(
            again.starts_with("? Question from the model (1/2)"),
            "{again}"
        );
        assert_eq!(d.prompt(), "> ");
        d.line("1");
        d.line("2");
        assert_eq!(d.line("y"), reply(&[(&[0], None), (&[1], None)]));
    }
}
