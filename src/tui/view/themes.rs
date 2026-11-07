//! The theme list (see `spec/tui.md`, "Theme list").

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::tui::app::App;
use crate::tui::style::{Colouring, Look};
use crate::tui::themes::ThemeList;
use crate::tui::wrap::fit;

use super::popup;

const WIDTH: u16 = 76;
/// The rows that are not themes: filter, a blank line before and after the
/// list, keys.
const CHROME: usize = 4;
const FILTER: &str = "Filter: ";

/// The list: a heading per group and a row per theme, and which line holds
/// the selected row.
fn list(app: &App, list: &ThemeList, width: usize) -> (Vec<Line<'static>>, usize) {
    let look = &app.settings.look;
    let symbols = &look.symbols;
    let visible = list.visible();
    let name_width = visible.iter().map(|e| e.name.width()).max().unwrap_or(0);
    let mut lines = Vec::new();
    let mut at = 0;
    let mut group = None;
    for (i, entry) in visible.iter().enumerate() {
        if group != Some(entry.group) {
            group = Some(entry.group);
            lines.push(Line::styled(entry.group.heading(), look.styles.heading));
        }
        let selected = i == list.selected();
        let pointer = if selected { symbols.pointer } else { " " };
        let mut marks = Vec::new();
        if entry.name == list.original {
            marks.push(format!("{} current", symbols.done));
        }
        match &entry.broken {
            Some(error) => marks.push(format!("{} {error}", symbols.warning)),
            None if !entry.note.is_empty() => marks.push(entry.note.clone()),
            None => {}
        }
        let text = format!(
            "{pointer} {name:<name_width$}  {}",
            marks.join(", "),
            name = entry.name
        );
        let style = if selected {
            at = lines.len();
            look.styles.selection
        } else if entry.broken.is_some() {
            look.styles.dim
        } else {
            look.styles.text
        };
        lines.push(Line::styled(
            fit(text.trim_end(), width, symbols.more),
            style,
        ));
    }
    (lines, at)
}

fn keys(list: &ThemeList, look: &Look) -> Line<'static> {
    let rule = look.symbols.rule;
    let text = if list.for_settings {
        format!("Enter take {rule} Esc back")
    } else {
        format!(
            "Enter take and save as default {rule} Esc back to {}",
            list.original
        )
    };
    Line::styled(text, look.styles.dim)
}

fn title(app: &App) -> String {
    let theming = &app.settings.theming;
    let warning = app.settings.look.symbols.warning;
    let rule = app.settings.look.symbols.rule;
    if theming.colouring == Colouring::None {
        format!(" Theme {rule} {warning} colours are off (NO_COLOR) ")
    } else if let Some(env) = &theming.theme_env {
        format!(" Theme {rule} {warning} CLAI_THEME={env} for this run ")
    } else {
        " Theme ".to_owned()
    }
}

pub fn draw(app: &App, frame: &mut Frame<'_>, size: Rect) {
    let Some(themes) = &app.theme_list else {
        return;
    };
    let look = &app.settings.look;
    let cells = WIDTH.min(size.width.saturating_sub(4));
    let width = usize::from(cells);
    let (rows, selected) = list(app, themes, width);
    let wanted = u16::try_from(rows.len().max(1) + CHROME).unwrap_or(u16::MAX);
    let area = popup::open(frame, size, &title(app), cells, wanted, look);
    let room = usize::from(area.height).saturating_sub(CHROME).max(1);
    let first = (selected + 1).saturating_sub(room);

    let filter_width = width.saturating_sub(FILTER.len()).max(1);
    let (filter_rows, (cursor_row, cursor_cell)) = themes.filter.rows(filter_width);
    let shown = filter_rows.get(cursor_row).cloned().unwrap_or_default();
    let mut lines = vec![Line::styled(format!("{FILTER}{shown}"), look.styles.text)];
    lines.push(Line::raw(""));
    if rows.is_empty() {
        lines.push(Line::styled("no theme", look.styles.dim));
    }
    lines.extend(rows.into_iter().skip(first).take(room));
    lines.resize(area.height.saturating_sub(1).into(), Line::raw(""));
    lines.push(keys(themes, look));
    frame.render_widget(Paragraph::new(lines), area);
    if let Ok(cell) = u16::try_from(FILTER.len() + cursor_cell) {
        frame.set_cursor_position((area.x + cell, area.y));
    }
}
