//! The box for a question from the model (`ask_user`): which question is
//! shown, what is chosen and typed, and the keys that answer (see
//! `spec/tui.md`, "Questions from the model").

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::agent::{Answer, AskUser};
use crate::tools::UserAnswer;

use super::editor::{Edit, Editor};

/// What is chosen for one question.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Draft {
    /// Option indexes, in the order chosen.
    pub selected: Vec<usize>,
    pub text: String,
}

impl Draft {
    pub fn is_empty(&self) -> bool {
        self.selected.is_empty() && self.text.trim().is_empty()
    }

    fn answer(&self) -> UserAnswer {
        let mut selected = self.selected.clone();
        selected.sort_unstable();
        let text = self.text.trim();
        UserAnswer {
            selected,
            text: (!text.is_empty()).then(|| text.to_owned()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ask {
    /// The question shown; the number of questions is the review page.
    pub page: usize,
    /// The selected row: an option, or the text row after the options.
    pub cursor: usize,
    pub drafts: Vec<Draft>,
    /// The text row is being typed.
    pub editing: Option<Editor>,
    /// `Esc` with something chosen: `y` declines, `n` goes back.
    pub confirm_decline: bool,
    /// The first row shown, when the box is taller than the screen.
    pub top: usize,
}

/// Rows `PgUp` / `PgDn` scroll.
const PAGE: usize = 10;

impl Ask {
    pub fn new(ask: &AskUser) -> Ask {
        Ask {
            page: 0,
            cursor: 0,
            drafts: vec![Draft::default(); ask.questions.len()],
            editing: None,
            confirm_decline: false,
            top: 0,
        }
    }

    pub fn reviewing(&self, ask: &AskUser) -> bool {
        self.page >= ask.questions.len()
    }

    /// The answers as sent.
    pub fn answers(&self) -> Vec<UserAnswer> {
        self.drafts.iter().map(Draft::answer).collect()
    }

    fn go_to(&mut self, page: usize) {
        self.page = page;
        self.cursor = 0;
        self.top = 0;
    }

    /// After a question is answered: the next one, the review page, or
    /// the answer itself for one question with one choice.
    fn advance(&mut self, ask: &AskUser) -> Option<Answer> {
        let count = ask.questions.len();
        if count == 1 && !ask.questions.first().is_some_and(|q| q.multiple) {
            return self.send();
        }
        self.go_to((self.page + 1).min(count));
        None
    }

    fn send(&mut self) -> Option<Answer> {
        if let Some(first) = self.drafts.iter().position(Draft::is_empty) {
            self.go_to(first);
            return None;
        }
        Some(Answer::Reply {
            answers: self.answers(),
        })
    }

    /// Takes pasted text while the text row is typed, on one line.
    pub fn paste(&mut self, text: &str) {
        if let Some(editor) = &mut self.editing {
            editor.paste(&text.replace(['\r', '\n'], " "));
        }
    }

    /// A key of the box; the answer if it gives one.
    pub fn key(&mut self, ask: &AskUser, key: KeyEvent) -> Option<Answer> {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && key.code == KeyCode::Char('c') {
            return Some(Answer::Cancel);
        }
        if self.confirm_decline {
            match key.code {
                KeyCode::Char('y' | 'Y') => return Some(Answer::Decline),
                KeyCode::Char('n' | 'N') | KeyCode::Esc => self.confirm_decline = false,
                _ => {}
            }
            return None;
        }
        if self.editing.is_some() {
            return self.edit_key(ask, key);
        }
        match key.code {
            KeyCode::Esc => {
                if self.drafts.iter().all(Draft::is_empty) {
                    return Some(Answer::Decline);
                }
                self.confirm_decline = true;
                return None;
            }
            KeyCode::PageUp => self.top = self.top.saturating_sub(PAGE),
            KeyCode::PageDown => self.top += PAGE,
            KeyCode::Tab => {
                let last = if ask.questions.len() > 1 {
                    ask.questions.len()
                } else {
                    0
                };
                self.go_to((self.page + 1).min(last));
            }
            KeyCode::BackTab => self.go_to(self.page.saturating_sub(1)),
            _ if self.reviewing(ask) => {
                if key.code == KeyCode::Enter {
                    return self.send();
                }
            }
            _ => return self.question_key(ask, key),
        }
        None
    }

    fn question_key(&mut self, ask: &AskUser, key: KeyEvent) -> Option<Answer> {
        let question = ask.questions.get(self.page)?;
        let options = question.options.len();
        let text_row = options;
        match key.code {
            KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                self.top = 0;
            }
            KeyCode::Down => {
                self.cursor = (self.cursor + 1).min(text_row);
                self.top = 0;
            }
            KeyCode::Char(c @ '1'..='9') if key.modifiers.is_empty() => {
                let row = usize::from(c as u8 - b'1');
                if row <= text_row {
                    self.cursor = row;
                    self.choose(question.multiple, options);
                }
            }
            KeyCode::Char(' ') => self.choose(question.multiple, options),
            KeyCode::Enter if self.cursor == text_row => self.start_editing(),
            KeyCode::Enter => {
                let draft = self.drafts.get_mut(self.page)?;
                if !question.multiple {
                    draft.selected = vec![self.cursor];
                } else if draft.selected.is_empty() {
                    draft.selected.push(self.cursor);
                }
                return self.advance(ask);
            }
            _ => {}
        }
        None
    }

    /// `Space` or a number: tick, select, or start typing on the text row.
    fn choose(&mut self, multiple: bool, options: usize) {
        if self.cursor >= options {
            self.start_editing();
            return;
        }
        let Some(draft) = self.drafts.get_mut(self.page) else {
            return;
        };
        if !multiple {
            draft.selected = vec![self.cursor];
        } else if let Some(at) = draft.selected.iter().position(|&i| i == self.cursor) {
            draft.selected.remove(at);
        } else {
            draft.selected.push(self.cursor);
        }
    }

    fn start_editing(&mut self) {
        let mut editor = Editor::default();
        if let Some(draft) = self.drafts.get(self.page) {
            editor.set_text(&draft.text);
        }
        self.editing = Some(editor);
    }

    fn edit_key(&mut self, ask: &AskUser, key: KeyEvent) -> Option<Answer> {
        let editor = self.editing.as_mut()?;
        match key.code {
            KeyCode::Esc => self.editing = None,
            KeyCode::Enter => {
                let text = editor.text();
                self.editing = None;
                let draft = self.drafts.get_mut(self.page)?;
                draft.text = text;
                if !draft.text.trim().is_empty() {
                    return self.advance(ask);
                }
            }
            _ => {
                if let Some(edit) = super::keys::edit(key)
                    && edit != Edit::Newline
                {
                    editor.edit(edit);
                }
            }
        }
        None
    }
}
