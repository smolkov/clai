//! The prompt bar: an open permission or limit question and the keys that
//! answer it (see `spec/tui.md`, "Permission prompts", "Limit prompt").

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::agent::{Answer, Choice, Question, QuestionKind};

use super::editor::{Edit, Editor};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    /// The choices are shown.
    Choices,
    /// `a`: a proposed rule set is shown; `Tab` steps to the next, wider one.
    Always { proposal: usize },
    /// `e`: a message to the model is typed.
    Explain(Editor),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Prompt {
    pub question: Question,
    pub mode: Mode,
    /// Why the server rejected the last answer.
    pub error: Option<String>,
    /// An answer is on its way; no more until the response.
    pub sending: bool,
}

impl Prompt {
    pub fn new(question: Question) -> Prompt {
        Prompt {
            question,
            mode: Mode::Choices,
            error: None,
            sending: false,
        }
    }

    pub fn id(&self) -> &str {
        &self.question.id
    }

    fn offers(&self, choice: Choice) -> bool {
        self.question.choices.contains(&choice)
    }

    /// The rule sets for "always", narrowest first.
    pub fn proposals(&self) -> &[Vec<String>] {
        match &self.question.kind {
            QuestionKind::Permission(p) => &p.proposals,
            QuestionKind::Limit(_) => &[],
        }
    }

    pub fn can_always(&self) -> bool {
        self.offers(Choice::AllowAlways) && !self.proposals().is_empty()
    }

    fn is_limit(&self) -> bool {
        matches!(self.question.kind, QuestionKind::Limit(_))
    }

    /// The answer the server rejected: back to the choices, with its reason.
    pub fn rejected(&mut self, message: String) {
        self.error = Some(message);
        self.sending = false;
        self.mode = Mode::Choices;
    }

    /// Takes pasted text: only the message to the model, on one line.
    pub fn paste(&mut self, text: &str) {
        if let Mode::Explain(editor) = &mut self.mode {
            editor.paste(&text.replace(['\r', '\n'], " "));
        }
    }

    /// A key of the prompt bar; the answer if it gives one.
    pub fn key(&mut self, key: KeyEvent) -> Option<Answer> {
        if self.sending {
            return None;
        }
        let answer = match self.mode.clone() {
            Mode::Choices => self.choices_key(key),
            Mode::Always { proposal } => self.always_key(key, proposal),
            Mode::Explain(_) => self.explain_key(key),
        };
        if answer.is_some() {
            self.sending = true;
            self.error = None;
        }
        answer
    }

    fn is_interrupt(key: KeyEvent) -> bool {
        key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)
    }

    fn choices_key(&mut self, key: KeyEvent) -> Option<Answer> {
        // `Esc` and `Ctrl-C`: cancels the turn, or stops it at its limit.
        if key.code == KeyCode::Esc || Self::is_interrupt(key) {
            return if self.is_limit() {
                Some(Answer::Stop)
            } else if self.offers(Choice::Cancel) {
                Some(Answer::Cancel)
            } else {
                None
            };
        }
        if !key.modifiers.difference(KeyModifiers::SHIFT).is_empty() {
            return None;
        }
        let KeyCode::Char(c) = key.code else {
            return None;
        };
        match c.to_ascii_lowercase() {
            'y' if self.offers(Choice::Continue) => Some(Answer::Continue),
            'y' if self.offers(Choice::Allow) => Some(Answer::Allow),
            'n' if self.offers(Choice::Stop) => Some(Answer::Stop),
            'n' if self.offers(Choice::Deny) => Some(Answer::Deny),
            'a' if self.can_always() => {
                self.mode = Mode::Always { proposal: 0 };
                None
            }
            'e' if self.offers(Choice::DenyMessage) => {
                self.mode = Mode::Explain(Editor::default());
                None
            }
            _ => None,
        }
    }

    fn always_key(&mut self, key: KeyEvent, proposal: usize) -> Option<Answer> {
        match key.code {
            KeyCode::Esc => self.mode = Mode::Choices,
            KeyCode::Char('c') if Self::is_interrupt(key) => self.mode = Mode::Choices,
            KeyCode::Tab => {
                let count = self.proposals().len().max(1);
                self.mode = Mode::Always {
                    proposal: (proposal + 1) % count,
                };
            }
            KeyCode::Enter => {
                let rules = self.proposals().get(proposal)?.clone();
                return Some(Answer::AllowAlways { rules });
            }
            _ => {}
        }
        None
    }

    fn explain_key(&mut self, key: KeyEvent) -> Option<Answer> {
        if key.code == KeyCode::Esc || Self::is_interrupt(key) {
            self.mode = Mode::Choices;
            return None;
        }
        let Mode::Explain(editor) = &mut self.mode else {
            return None;
        };
        if key.code == KeyCode::Enter {
            let text = editor.text();
            if text.trim().is_empty() {
                return None;
            }
            return Some(Answer::DenyMessage { text });
        }
        if let Some(edit) = super::keys::edit(key)
            && edit != Edit::Newline
        {
            editor.edit(edit);
        }
        None
    }
}
