//! The questions of a session (see `spec/sessions.md`, "Questions"): the open
//! one, the agent waiting for an answer, answers in the order they came, the
//! one being checked, and why earlier questions closed. No events and no task:
//! the actor calls these methods and publishes.

use std::collections::{HashMap, VecDeque};

use tokio::sync::oneshot;

use crate::agent::{Answer, Closed, Question, Reply};

/// An answer waiting for the agent to ask, or for its verdict.
pub(super) struct Pending {
    pub question: String,
    pub answer: Answer,
    pub by: String,
    pub reply: oneshot::Sender<Result<(), String>>,
}

#[derive(Default)]
pub(super) struct Questions {
    /// The open question, as the clients got it.
    open: Option<Question>,
    /// The agent waits for an answer to this question.
    asking: Option<(String, oneshot::Sender<Reply>)>,
    /// Answers in the order they came; the first is being checked when
    /// `checking` is set.
    answers: VecDeque<Pending>,
    checking: Option<Pending>,
    /// Closed questions and why, for late answers.
    closed: HashMap<String, String>,
}

impl Questions {
    pub fn current(&self) -> Option<&Question> {
        self.open.as_ref()
    }

    /// A question went out to the clients.
    pub fn opened(&mut self, question: Question) {
        self.open = Some(question);
    }

    /// The session was renamed.
    pub fn rename(&mut self, name: &str) {
        if let Some(question) = &mut self.open {
            question.session.name = name.to_owned();
        }
    }

    /// A question closed: the answer being checked gets its verdict, those
    /// waiting are told why they came too late.
    ///
    /// Only what belongs to `id` is touched. The agent's ask and the closing
    /// event reach the actor on different channels, so with two calls in one
    /// answer of the model the agent may already ask the next question when
    /// the close of the previous one comes in; clearing that ask would leave
    /// its answer waiting for ever.
    pub fn closed(&mut self, id: &str, closed: &Closed) {
        if self.open.as_ref().is_some_and(|q| q.id == id) {
            self.open = None;
        }
        if self.asking.as_ref().is_some_and(|(asked, _)| asked == id) {
            self.asking = None;
        }
        let why = match closed {
            Closed::Answered { by, .. } => format!("already answered by {by}"),
            Closed::NoOne => "the question was closed".to_owned(),
            Closed::Cancelled => "the question was closed: the turn was cancelled".to_owned(),
        };
        if let Some(pending) = self.checking.take_if(|p| p.question == id) {
            let counted = matches!(closed, Closed::Answered { by, .. } if *by == pending.by);
            let _ = pending
                .reply
                .send(if counted { Ok(()) } else { Err(why.clone()) });
        }
        let (late, other): (VecDeque<Pending>, VecDeque<Pending>) =
            self.answers.drain(..).partition(|p| p.question == id);
        self.answers = other;
        for pending in late {
            let _ = pending.reply.send(Err(why.clone()));
        }
        self.closed.insert(id.to_owned(), why);
        // An answer to the next question may have waited for this one.
        self.deliver();
    }

    /// An answer from a client of session `session`.
    pub fn answer(&mut self, pending: Pending, session: &str) {
        let open = self.open.as_ref().map(|q| q.id.as_str());
        let asked = self.asking.as_ref().map(|(id, _)| id.as_str());
        if open != Some(pending.question.as_str()) && asked != Some(pending.question.as_str()) {
            let why = self
                .closed
                .get(&pending.question)
                .cloned()
                .unwrap_or_else(|| match open {
                    Some(open) => format!(
                        "{} is not the open question; the open question is {open}",
                        pending.question
                    ),
                    None => format!("{session} has no open question"),
                });
            let _ = pending.reply.send(Err(why));
            return;
        }
        self.answers.push_back(pending);
        self.deliver();
    }

    /// The agent waits for an answer to `question`.
    pub fn ask(&mut self, question: String, reply: oneshot::Sender<Reply>) {
        self.asking = Some((question, reply));
        self.deliver();
    }

    /// The agent did not accept the answer being checked.
    pub fn rejected(&mut self, question: &str, message: String) {
        if let Some(pending) = self.checking.take_if(|p| p.question == question) {
            let _ = pending.reply.send(Err(message));
        }
    }

    /// The session ended: every waiting answer gets `why`.
    pub fn end(&mut self, why: &str) {
        for pending in self.answers.drain(..).chain(self.checking.take()) {
            let _ = pending.reply.send(Err(why.to_owned()));
        }
    }

    /// Gives the first waiting answer to the agent, if it is asking and no
    /// other answer is being checked.
    fn deliver(&mut self) {
        if self.checking.is_some() {
            return;
        }
        let Some((asked, _)) = &self.asking else {
            return;
        };
        let Some(position) = self.answers.iter().position(|p| p.question == *asked) else {
            return;
        };
        let (Some(pending), Some((_, reply))) = (self.answers.remove(position), self.asking.take())
        else {
            return;
        };
        let _ = reply.send(Reply::Answer {
            answer: pending.answer.clone(),
            by: pending.by.clone(),
        });
        self.checking = Some(pending);
    }
}

#[cfg(test)]
mod tests {
    use tokio::sync::oneshot::error::TryRecvError;

    use super::*;
    use crate::agent::{LIMIT_CHOICES, Limit, QuestionKind, SessionRef, TurnInfo};

    type Verdict = oneshot::Receiver<Result<(), String>>;

    fn question(id: &str) -> Question {
        Question {
            id: id.to_owned(),
            session: SessionRef::default(),
            opened: 0,
            turn: TurnInfo {
                number: 1,
                requests: 1,
                tool_calls: 0,
            },
            summary: "limit".to_owned(),
            choices: LIMIT_CHOICES.to_vec(),
            kind: QuestionKind::Limit(Limit {
                max_requests: 1,
                max_tool_calls: 1,
                requests: 1,
                tool_calls: 0,
            }),
        }
    }

    fn pending(question: &str, by: &str) -> (Pending, Verdict) {
        let (reply, verdict) = oneshot::channel();
        (
            Pending {
                question: question.to_owned(),
                answer: Answer::Continue,
                by: by.to_owned(),
                reply,
            },
            verdict,
        )
    }

    fn answered(by: &str) -> Closed {
        Closed::Answered {
            answer: Answer::Continue,
            by: by.to_owned(),
        }
    }

    /// An open question the agent is asking; the receiver gets what it hears.
    fn asking(id: &str) -> (Questions, oneshot::Receiver<Reply>) {
        let mut questions = Questions::default();
        questions.opened(question(id));
        let (reply, heard) = oneshot::channel();
        questions.ask(id.to_owned(), reply);
        (questions, heard)
    }

    #[test]
    fn first_answer_wins() {
        let (mut questions, mut heard) = asking("q1");
        let (first, mut first_verdict) = pending("q1", "a");
        let (second, mut second_verdict) = pending("q1", "b");
        questions.answer(first, "s");
        questions.answer(second, "s");
        // The agent hears the first; both still wait for a verdict.
        assert_eq!(
            heard.try_recv().unwrap(),
            Reply::Answer {
                answer: Answer::Continue,
                by: "a".to_owned()
            }
        );
        assert_eq!(first_verdict.try_recv(), Err(TryRecvError::Empty));
        assert_eq!(second_verdict.try_recv(), Err(TryRecvError::Empty));

        questions.closed("q1", &answered("a"));
        assert_eq!(first_verdict.try_recv().unwrap(), Ok(()));
        assert_eq!(
            second_verdict.try_recv().unwrap(),
            Err("already answered by a".to_owned())
        );
        assert!(questions.current().is_none());
    }

    #[test]
    fn a_rejected_answer_lets_the_next_one_through() {
        let (mut questions, mut heard) = asking("q1");
        let (bad, mut bad_verdict) = pending("q1", "a");
        let (good, mut good_verdict) = pending("q1", "b");
        questions.answer(bad, "s");
        questions.answer(good, "s");
        assert!(heard.try_recv().is_ok());

        // Another question's rejection does not touch this answer.
        questions.rejected("q9", "no".to_owned());
        assert_eq!(bad_verdict.try_recv(), Err(TryRecvError::Empty));

        questions.rejected("q1", "not allowed".to_owned());
        assert_eq!(
            bad_verdict.try_recv().unwrap(),
            Err("not allowed".to_owned())
        );
        // The question stays open; the agent asks again and hears the next.
        assert!(questions.current().is_some());
        let (reply, mut heard) = oneshot::channel();
        questions.ask("q1".to_owned(), reply);
        assert_eq!(
            heard.try_recv().unwrap(),
            Reply::Answer {
                answer: Answer::Continue,
                by: "b".to_owned()
            }
        );
        questions.closed("q1", &answered("b"));
        assert_eq!(good_verdict.try_recv().unwrap(), Ok(()));
    }

    #[test]
    fn a_late_answer_hears_why() {
        let (mut questions, _heard) = asking("q1");
        questions.closed("q1", &Closed::Cancelled);
        let (late, mut verdict) = pending("q1", "a");
        questions.answer(late, "s");
        assert_eq!(
            verdict.try_recv().unwrap(),
            Err("the question was closed: the turn was cancelled".to_owned())
        );
    }

    #[test]
    fn a_wrong_question_is_named() {
        let (mut questions, _heard) = asking("q2");
        let (wrong, mut verdict) = pending("q1", "a");
        questions.answer(wrong, "s");
        assert_eq!(
            verdict.try_recv().unwrap(),
            Err("q1 is not the open question; the open question is q2".to_owned())
        );

        let mut none = Questions::default();
        let (wrong, mut verdict) = pending("q1", "a");
        none.answer(wrong, "s");
        assert_eq!(
            verdict.try_recv().unwrap(),
            Err("s has no open question".to_owned())
        );
    }

    #[test]
    fn answers_wait_for_the_agent_to_ask() {
        let mut questions = Questions::default();
        questions.opened(question("q1"));
        let (early, mut verdict) = pending("q1", "a");
        questions.answer(early, "s");
        assert_eq!(verdict.try_recv(), Err(TryRecvError::Empty));

        let (reply, mut heard) = oneshot::channel();
        questions.ask("q1".to_owned(), reply);
        assert_eq!(
            heard.try_recv().unwrap(),
            Reply::Answer {
                answer: Answer::Continue,
                by: "a".to_owned()
            }
        );
    }

    /// Two calls in one answer of the model: the agent asks the second
    /// question before the actor sees the first one close. The close must not
    /// forget that ask, or the answer to the second question never arrives.
    #[test]
    fn the_next_ask_survives_the_close_of_the_previous_question() {
        let (mut questions, mut heard) = asking("q1");
        let (first, mut first_verdict) = pending("q1", "tui");
        questions.answer(first, "s");
        assert!(heard.try_recv().is_ok());
        let (reply, mut heard_next) = oneshot::channel();
        questions.ask("q2".to_owned(), reply);
        questions.closed("q1", &answered("tui"));
        questions.opened(question("q2"));
        assert_eq!(first_verdict.try_recv().unwrap(), Ok(()));

        let (next, mut next_verdict) = pending("q2", "tui");
        questions.answer(next, "s");
        assert_eq!(
            heard_next.try_recv().unwrap(),
            Reply::Answer {
                answer: Answer::Continue,
                by: "tui".to_owned()
            }
        );
        questions.closed("q2", &answered("tui"));
        assert_eq!(next_verdict.try_recv().unwrap(), Ok(()));
    }

    /// The answer to the next question comes while the previous answer is
    /// still being checked: it waits, is not told it came too late, and goes
    /// to the agent once the previous question closed.
    #[test]
    fn an_answer_to_the_next_question_waits_for_the_close() {
        let (mut questions, mut heard) = asking("q1");
        let (first, _first_verdict) = pending("q1", "tui");
        questions.answer(first, "s");
        assert!(heard.try_recv().is_ok());
        let (reply, mut heard_next) = oneshot::channel();
        questions.ask("q2".to_owned(), reply);
        let (next, mut next_verdict) = pending("q2", "chat");
        questions.answer(next, "s");
        assert_eq!(heard_next.try_recv(), Err(TryRecvError::Empty));

        questions.closed("q1", &answered("tui"));
        assert_eq!(next_verdict.try_recv(), Err(TryRecvError::Empty));
        assert_eq!(
            heard_next.try_recv().unwrap(),
            Reply::Answer {
                answer: Answer::Continue,
                by: "chat".to_owned()
            }
        );
    }

    #[test]
    fn a_rename_reaches_the_open_question() {
        let (mut questions, _heard) = asking("q1");
        questions.rename("new");
        assert_eq!(questions.current().unwrap().session.name, "new");
    }

    #[test]
    fn the_end_answers_everyone() {
        let (mut questions, _heard) = asking("q1");
        let (checked, mut checked_verdict) = pending("q1", "a");
        let (waiting, mut waiting_verdict) = pending("q1", "b");
        questions.answer(checked, "s");
        questions.answer(waiting, "s");
        questions.end("gone");
        assert_eq!(checked_verdict.try_recv().unwrap(), Err("gone".to_owned()));
        assert_eq!(waiting_verdict.try_recv().unwrap(), Err("gone".to_owned()));
    }
}
