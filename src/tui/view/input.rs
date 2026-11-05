//! The input editor on the screen.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::tui::app::App;
use crate::tui::keys::Focus;

/// Cells of `> ` before the text.
const PROMPT: u16 = 2;

/// Rows the text would take in `width` cells.
pub fn height(app: &App, width: u16) -> usize {
    let (rows, _) = app.editor.rows(usize::from(width.saturating_sub(PROMPT)));
    rows.len()
}

/// The first row to show, so that the cursor stays visible.
fn first_row(cursor_row: usize, height: u16) -> usize {
    cursor_row.saturating_sub(usize::from(height).saturating_sub(1))
}

pub fn draw(app: &App, buffer: &mut Buffer, area: Rect) {
    let styles = &app.settings.look.styles;
    let width = usize::from(area.width.saturating_sub(PROMPT));
    let (rows, (cursor_row, _)) = app.editor.rows(width);
    let first = first_row(cursor_row, area.height);
    let style = if app.ended.is_some() || app.prompt.is_some() || app.focus != Focus::Input {
        styles.dim
    } else {
        styles.text
    };
    for (i, row) in rows
        .iter()
        .skip(first)
        .take(usize::from(area.height))
        .enumerate()
    {
        let y = area.y + u16::try_from(i).unwrap_or(0);
        if first + i == 0 {
            buffer.set_string(area.x, y, ">", style);
        }
        buffer.set_stringn(area.x + PROMPT, y, row, width, style);
    }
}

pub fn cursor(app: &App, area: Rect) -> Option<(u16, u16)> {
    if app.focus != Focus::Input || app.ended.is_some() {
        return None;
    }
    let width = usize::from(area.width.saturating_sub(PROMPT));
    let (_, (row, cell)) = app.editor.rows(width);
    let y = row.checked_sub(first_row(row, area.height))?;
    Some((
        area.x + PROMPT + u16::try_from(cell).ok()?,
        area.y + u16::try_from(y).ok()?,
    ))
}
