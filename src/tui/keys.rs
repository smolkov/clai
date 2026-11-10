//! Which key does what, by focus (see `spec/tui.md`, "Keys").

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::editor::Edit;

/// Where keys go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Input,
    Conversation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// `Esc`.
    Cancel,
    /// `Ctrl-C`.
    Interrupt,
    /// `Ctrl-D`.
    Detach,
    PageUp,
    PageDown,
    Start,
    End,
    SwitchFocus,
    Send,
    Edit(Edit),
    Up,
    Down,
    /// `Enter` in the conversation.
    Toggle,
    Help,
    /// `Ctrl-O`.
    ModelPicker,
    /// `Ctrl-S`.
    SessionList,
    /// `Ctrl-N`.
    NextSession,
    /// `Ctrl-P`.
    PreviousSession,
    /// `y` in the conversation: copy the selected entry.
    Copy,
    /// `Y` in the conversation: copy the last code block of the selected
    /// answer.
    CopyCode,
    /// `o` in the conversation: open the selected call in the details view.
    Open,
    /// `Ctrl-Y`: copy the last answer.
    CopyAnswer,
    /// `Ctrl-T`: show or hide the calls of the last turn.
    ToggleCalls,
    Nothing,
}

fn ctrl(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
}

/// The keys that work with any focus and open prompt, because they answer
/// nothing: scrolling, to read what a prompt asks about.
pub fn scrolling(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::PageUp => Some(Action::PageUp),
        KeyCode::PageDown => Some(Action::PageDown),
        KeyCode::Home if ctrl(key) => Some(Action::Start),
        KeyCode::End if ctrl(key) => Some(Action::End),
        _ => None,
    }
}

pub fn is_detach(key: KeyEvent) -> bool {
    ctrl(key) && key.code == KeyCode::Char('d')
}

/// The action of a key when no prompt or dialog takes it.
pub fn action(key: KeyEvent, focus: Focus) -> Action {
    if let Some(action) = scrolling(key) {
        return action;
    }
    match key.code {
        KeyCode::Esc => return Action::Cancel,
        KeyCode::Char('c') if ctrl(key) => return Action::Interrupt,
        KeyCode::Char('d') if ctrl(key) => return Action::Detach,
        KeyCode::Char('o') if ctrl(key) => return Action::ModelPicker,
        KeyCode::Char('s') if ctrl(key) => return Action::SessionList,
        KeyCode::Char('n') if ctrl(key) => return Action::NextSession,
        KeyCode::Char('p') if ctrl(key) => return Action::PreviousSession,
        KeyCode::Char('y') if ctrl(key) => return Action::CopyAnswer,
        KeyCode::Char('t') if ctrl(key) => return Action::ToggleCalls,
        KeyCode::Tab | KeyCode::BackTab => return Action::SwitchFocus,
        _ => {}
    }
    match focus {
        Focus::Conversation => match key.code {
            KeyCode::Up => Action::Up,
            KeyCode::Down => Action::Down,
            KeyCode::Enter => Action::Toggle,
            KeyCode::Char('?') => Action::Help,
            KeyCode::Char('y') => Action::Copy,
            KeyCode::Char('Y') => Action::CopyCode,
            KeyCode::Char('o') => Action::Open,
            _ => Action::Nothing,
        },
        Focus::Input => match key.code {
            KeyCode::Up => Action::Up,
            KeyCode::Down => Action::Down,
            KeyCode::Enter
                if !key
                    .modifiers
                    .intersects(KeyModifiers::SHIFT | KeyModifiers::ALT) =>
            {
                Action::Send
            }
            _ => edit(key).map_or(Action::Nothing, Action::Edit),
        },
    }
}

/// The editing key of a key press.
pub fn edit(key: KeyEvent) -> Option<Edit> {
    Some(match key.code {
        KeyCode::Enter => Edit::Newline,
        KeyCode::Left => Edit::Left,
        KeyCode::Right => Edit::Right,
        KeyCode::Home => Edit::Home,
        KeyCode::End => Edit::End,
        KeyCode::Backspace if ctrl(key) || key.modifiers.contains(KeyModifiers::ALT) => {
            Edit::DeleteWord
        }
        KeyCode::Backspace => Edit::Backspace,
        KeyCode::Delete => Edit::Delete,
        KeyCode::Char('a') if ctrl(key) => Edit::Home,
        KeyCode::Char('e') if ctrl(key) => Edit::End,
        KeyCode::Char('w') if ctrl(key) => Edit::DeleteWord,
        KeyCode::Char('u') if ctrl(key) => Edit::DeleteToStart,
        // `Ctrl-Z` and other control keys do nothing.
        KeyCode::Char(_) if ctrl(key) => return None,
        KeyCode::Char(c) => Edit::Char(c),
        _ => return None,
    })
}
