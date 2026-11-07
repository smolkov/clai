//! The settings box (see `spec/tui.md`, "Settings").

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::config::Symbols;
use crate::tui::app::App;
use crate::tui::settings::{Row, SettingsBox, get, rows, shown};
use crate::tui::style::{Colouring, Look};
use crate::tui::wrap::fit;

use super::popup;

/// The box's width, if the screen has room for it.
const WIDTH: u16 = 76;
/// The rows of the box that are not settings: a blank line, the help line, the
/// keys.
const CHROME: usize = 3;

/// What else decides a value, if anything: `⚠ CLAI_THEME=nord`.
fn override_mark(app: &App, settings: &SettingsBox, row: &Row) -> Option<String> {
    let warning = app.settings.look.symbols.warning;
    let theming = &app.settings.theming;
    match row.key {
        "theme" if theming.colouring == Colouring::None => {
            Some(format!("{warning} colours are off (NO_COLOR)"))
        }
        "theme" => theming.theme_env.as_ref().map(|t| {
            if settings.current.theme == settings.saved.theme {
                format!("{warning} CLAI_THEME={t} for this run")
            } else {
                format!("{warning} CLAI_THEME={t} after this run")
            }
        }),
        "symbols" if settings.current.symbols == Symbols::Unicode && !theming.unicode_locale => {
            Some(format!("{warning} the locale is not UTF-8"))
        }
        _ => None,
    }
}

fn keys(settings: &SettingsBox, look: &Look) -> Line<'static> {
    let rule = look.symbols.rule;
    let text = if settings.confirm_close {
        "save changes? [s] save  [n] discard  [Esc] back".to_owned()
    } else if settings.editing.is_some() {
        format!("Enter take {rule} Esc back")
    } else {
        format!(
            "Enter change {rule} Left/Right value {rule} d default {rule} s save {rule} Esc close"
        )
    };
    let style = if settings.confirm_close {
        look.styles.warning
    } else {
        look.styles.dim
    };
    Line::styled(text, style)
}

pub fn draw(app: &App, frame: &mut Frame<'_>, size: Rect) {
    let Some(settings) = &app.settings_box else {
        return;
    };
    let look = &app.settings.look;
    let symbols = &look.symbols;
    let file = app.settings.theming.user_file.display().to_string();
    let home = std::env::var("HOME").unwrap_or_default();
    let file = match file.strip_prefix(&home) {
        Some(rest) if !home.is_empty() => format!("~{rest}"),
        _ => file,
    };
    let title = format!(" Settings {} {file} ", symbols.rule);
    let cells = WIDTH.min(size.width.saturating_sub(4));
    let width = usize::from(cells);
    let rows = rows();
    let wanted = u16::try_from(rows.len() + CHROME).unwrap_or(u16::MAX);
    let area = popup::open(frame, size, &title, cells, wanted, look);
    let room = usize::from(area.height).saturating_sub(CHROME).max(1);
    let first = (settings.selected() + 1).saturating_sub(room);
    let key_width = rows.iter().map(|r| r.key.len()).max().unwrap_or(0);
    let values: Vec<String> = rows
        .iter()
        .map(|r| shown(&get(&settings.current, r.key)))
        .collect();
    let value_width = values.iter().map(|v| v.width()).max().unwrap_or(0).max(8);

    let mut lines = Vec::new();
    let mut cursor = None;
    for (i, (row, value)) in rows.iter().zip(&values).enumerate().skip(first).take(room) {
        let selected = i == settings.selected();
        let pointer = if selected { symbols.pointer } else { " " };
        let changed = if settings.changed(row.key) { "*" } else { " " };
        let value = match (&settings.editing, selected) {
            (Some(editor), true) => {
                let (shown, (_, col)) = editor.rows(value_width);
                let text = shown.first().cloned().unwrap_or_default();
                // After the pointer, the key, a space and the change mark.
                cursor = Some((key_width + 4 + col, lines.len()));
                text
            }
            _ => value.clone(),
        };
        let description =
            override_mark(app, settings, row).unwrap_or_else(|| row.description.to_owned());
        let text = format!(
            "{pointer} {key:<key_width$} {changed}{value:<value_width$}  {description}",
            key = row.key
        );
        let in_file = settings.in_file(row.key) || settings.changed(row.key);
        let style = if selected {
            look.styles.selection
        } else if row.later || !in_file {
            // A default the file does not set, or a key not built yet.
            look.styles.dim
        } else {
            look.styles.text
        };
        lines.push(Line::styled(fit(&text, width, symbols.more), style));
    }
    lines.push(Line::raw(""));
    let help = match (&settings.message, settings.row()) {
        (Some(message), _) => Line::styled(fit(message, width, symbols.more), look.styles.warning),
        (None, Some(row)) => {
            let text = format!("{}: {}", row.key, row.help);
            Line::styled(fit(&text, width, symbols.more), look.styles.dim)
        }
        (None, None) => Line::raw(""),
    };
    lines.resize(usize::from(area.height).saturating_sub(2), Line::raw(""));
    lines.push(help);
    lines.push(keys(settings, look));
    frame.render_widget(Paragraph::new(lines), area);
    if let Some((cell, row)) = cursor
        && let (Ok(cell), Ok(row)) = (u16::try_from(cell), u16::try_from(row))
    {
        frame.set_cursor_position((area.x + cell, area.y + row));
    }
}
