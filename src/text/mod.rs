//! Plain-text output of a session for terminals and scripts: tool lines,
//! notices, questions and answers read as lines. Used by `clai run` and
//! `clai chat`.

mod asker;
mod conversation;
mod dialog;
mod events;
mod question;

#[cfg(test)]
mod tests;

pub use asker::LineAsker;
pub use conversation::Conversation;
pub use dialog::{Dialog, Step};
pub use events::Renderer;
pub use question::{Style, full_content, has_more, render_question, render_question_styled};

use crate::agent::CallState;
use crate::config::Symbols;

/// Whether to use Unicode symbols: `auto` asks the locale.
pub fn unicode(symbols: Symbols, getenv: impl Fn(&str) -> Option<String>) -> bool {
    match symbols {
        Symbols::Unicode => true,
        Symbols::Ascii => false,
        Symbols::Auto => ["LC_ALL", "LC_CTYPE", "LANG"]
            .iter()
            .find_map(|name| getenv(name).filter(|v| !v.is_empty()))
            .is_some_and(|v| {
                let v = v.to_ascii_lowercase();
                v.contains("utf-8") || v.contains("utf8")
            }),
    }
}

/// The marker of a finished tool call.
pub fn marker(state: CallState, unicode: bool) -> &'static str {
    match (state, unicode) {
        (CallState::Done, true) => "✓",
        (CallState::Done, false) => "+",
        (CallState::Error, true) => "✗",
        (CallState::Error, false) => "x",
        (CallState::Denied | CallState::Cancelled, true) => "⊘",
        (CallState::Denied | CallState::Cancelled, false) => "-",
        (_, true) => "…",
        (_, false) => ".",
    }
}

/// Cuts a text to `max` characters, with `…` (or `...`) at the end.
pub fn cut(text: &str, max: usize, unicode: bool) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mark = if unicode { "…" } else { "..." };
    let keep = max.saturating_sub(mark.chars().count());
    let mut out: String = text.chars().take(keep).collect();
    out.push_str(mark);
    out
}
