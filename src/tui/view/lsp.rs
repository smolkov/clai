//! The box of `/lsp` (see `spec/tui.md`, "Language servers").

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::tui::app::App;
use crate::tui::lsp::{STDERR_SHOWN, shown_root};
use crate::tui::wrap::fit;

/// The rows of the box, before the keys line.
pub fn lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let Some(open) = &app.lsp_box else {
        return Vec::new();
    };
    let look = &app.settings.look;
    let symbols = &look.symbols;
    let styles = &look.styles;
    let mut out = Vec::new();
    let Some(servers) = &open.servers else {
        out.push(Line::styled("asking the server …", styles.dim));
        return out;
    };
    if servers.is_empty() {
        out.push(Line::styled(
            "No language server has run. One starts when the model uses lsp.",
            styles.dim,
        ));
    }
    let workspace = &app.session.path;
    let roots: Vec<String> = servers
        .iter()
        .map(|s| shown_root(&s.root, workspace))
        .collect();
    let name_width = servers.iter().map(|s| s.name.width()).max().unwrap_or(0);
    let root_width = roots.iter().map(|r| r.width()).max().unwrap_or(0).min(40);
    for (i, (server, root)) in servers.iter().zip(&roots).enumerate() {
        let selected = i == open.selected;
        let pointer = if selected { symbols.pointer } else { " " };
        let root = fit(root, root_width, symbols.more);
        let mut text = format!(
            "{pointer} {name:<name_width$}  {root:<root_width$}  {}",
            server.state_text(),
            name = server.name,
        );
        if !server.sessions.is_empty() {
            text.push_str(&format!(
                "  {} {}",
                symbols.rule,
                server.sessions.join(", ")
            ));
        }
        let style = if selected {
            styles.selection
        } else if server.state == "failed" {
            styles.error
        } else {
            styles.text
        };
        out.push(Line::styled(fit(&text, width, symbols.more), style));
    }
    if let Some(server) = open.selected() {
        let tail: Vec<&String> = server
            .stderr
            .iter()
            .rev()
            .take(STDERR_SHOWN)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        if !tail.is_empty() {
            out.push(Line::raw(""));
            out.push(Line::styled(
                format!("Last lines of {}:", server.name),
                styles.heading,
            ));
            for line in tail {
                out.push(Line::styled(fit(line, width, symbols.more), styles.dim));
            }
        }
    }
    if let Some(error) = &open.error {
        out.push(Line::raw(""));
        out.push(Line::styled(fit(error, width, symbols.more), styles.error));
    }
    out
}

/// The rows the box wants at `width` cells (RFC 0024): its lines, a blank
/// line and the keys.
pub fn wanted(app: &App, width: usize) -> usize {
    if app.lsp_box.is_none() {
        return 0;
    }
    lines(app, width).len() + 2
}

pub fn draw(app: &App, frame: &mut Frame<'_>, area: Rect) {
    if app.lsp_box.is_none() {
        return;
    }
    let look = &app.settings.look;
    let width = usize::from(area.width);
    let mut rows = lines(app, width);
    let rule = look.symbols.rule;
    let keys = format!("r restart {rule} k stop {rule} Up Down choose {rule} Esc close");
    let area = super::open_panel(frame, area, (Line::raw(" Language servers "), None), look);
    // The keys stay the last row when the lines do not all fit.
    rows.truncate(usize::from(area.height).saturating_sub(2));
    rows.push(Line::raw(""));
    rows.push(Line::styled(keys, look.styles.dim));
    frame.render_widget(Paragraph::new(rows), area);
}
