//! The session row of the footer: session, workspace, model, position among
//! the sessions, others waiting, context usage.

use crate::tui::app::App;

/// `project · ~/ws/project · anthropic/sonnet · 1/3 ●1 · 12.3k/200k`
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
    if let Some(window) = app.context_window() {
        parts.push(format!("{}/{}", tokens(app.context_tokens), tokens(window)));
    }
    parts.join(&format!(" {} ", symbols.separator))
}

/// `950`, `12.3k`, `200k`, `1.2M`.
pub fn tokens(n: u64) -> String {
    let short = |value: u64, unit: u64, name: &str| {
        let tenths = value * 10 / unit;
        if tenths.is_multiple_of(10) {
            format!("{}{name}", tenths / 10)
        } else {
            format!("{}.{}{name}", tenths / 10, tenths % 10)
        }
    };
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => short(n, 1_000, "k"),
        _ => short(n, 1_000_000, "M"),
    }
}

#[cfg(test)]
mod tests {
    use super::tokens;

    #[test]
    fn token_counts() {
        assert_eq!(tokens(950), "950");
        assert_eq!(tokens(12_345), "12.3k");
        assert_eq!(tokens(200_000), "200k");
        assert_eq!(tokens(1_250_000), "1.2M");
    }
}
