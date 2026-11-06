//! The session row of the footer: session, workspace, model, position among
//! the sessions, others waiting, context usage, tokens used. The tokens come
//! last, so that they are the first thing cut on a narrow terminal.

use crate::text::tokens;
use crate::tui::app::App;

/// `project · ~/ws/project · anthropic/sonnet · 1/3 ●1 · 12.3k/200k · in 45.1k out 3.2k`
pub fn text(app: &App) -> String {
    let symbols = &app.settings.look.symbols;
    let session = &app.session;
    let mut parts = vec![
        session.name.clone(),
        session.workspace.clone(),
        session.model.clone(),
    ];
    if let Some(place) = app.place {
        let mut part = format!("{}/{}", place.position, place.total);
        if place.waiting > 0 {
            part.push_str(&format!(" {}{}", symbols.dot, place.waiting));
        }
        parts.push(part);
    }
    match app.context_window() {
        Some(window) => parts.push(format!("{}/{}", tokens(app.context_tokens), tokens(window))),
        // A model the server does not list: the size without a limit.
        None if app.context_tokens > 0 => parts.push(tokens(app.context_tokens)),
        None => {}
    }
    if let Some(usage) = app.usage {
        parts.push(format!(
            "in {} out {}",
            tokens(usage.input_tokens),
            tokens(usage.output_tokens)
        ));
    }
    parts.join(&format!(" {} ", symbols.separator))
}
