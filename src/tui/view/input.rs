//! The input editor on the screen; above the text, the line of the images
//! pasted into it (RFC 0034).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::tui::app::{App, images_line};
use crate::tui::keys::Focus;

/// Cells of `> ` before the text.
const PROMPT: u16 = 2;

/// The line of the images in the input, if it has any.
fn images(app: &App) -> Option<String> {
    let images = app.editor.images();
    (!images.is_empty()).then(|| images_line(&images))
}

/// Rows the text would take in `width` cells, with the images line.
pub fn height(app: &App, width: u16) -> usize {
    let (rows, _) = app.editor.rows(usize::from(width.saturating_sub(PROMPT)));
    rows.len() + usize::from(images(app).is_some())
}

/// The rows of the text: below the images line, if there is one.
fn text_area(app: &App, area: Rect) -> Rect {
    if images(app).is_some() && area.height > 1 {
        Rect::new(area.x, area.y + 1, area.width, area.height - 1)
    } else {
        area
    }
}

/// The first row to show, so that the cursor stays visible.
fn first_row(cursor_row: usize, height: u16) -> usize {
    cursor_row.saturating_sub(usize::from(height).saturating_sub(1))
}

pub fn draw(app: &App, buffer: &mut Buffer, area: Rect) {
    let styles = &app.settings.look.styles;
    if let Some(line) = images(app).filter(|_| area.height > 1) {
        buffer.set_stringn(area.x, area.y, line, usize::from(area.width), styles.dim);
    }
    let area = text_area(app, area);
    let width = usize::from(area.width.saturating_sub(PROMPT));
    let (rows, (cursor_row, _)) = app.editor.rows(width);
    let first = first_row(cursor_row, area.height);
    let dimmed = app.ended.is_some() || app.prompt.is_some() || app.focus != Focus::Input;
    let style = if dimmed { styles.dim } else { styles.text };
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
    // The placeholders of images stand out from the text.
    if !dimmed {
        for marked in app.editor.marked(width) {
            let Some(y) = marked
                .row
                .checked_sub(first)
                .filter(|r| *r < usize::from(area.height))
            else {
                continue;
            };
            let (Ok(y), Ok(x), Ok(w)) = (
                u16::try_from(y),
                u16::try_from(marked.cell),
                u16::try_from(marked.width),
            ) else {
                continue;
            };
            let cells = Rect::new(area.x + PROMPT + x, area.y + y, w, 1).intersection(area);
            buffer.set_style(cells, styles.accent);
        }
    }
}

pub fn cursor(app: &App, area: Rect) -> Option<(u16, u16)> {
    if app.focus != Focus::Input || app.ended.is_some() {
        return None;
    }
    let area = text_area(app, area);
    let width = usize::from(area.width.saturating_sub(PROMPT));
    let (_, (row, cell)) = app.editor.rows(width);
    let y = row.checked_sub(first_row(row, area.height))?;
    Some((
        area.x + PROMPT + u16::try_from(cell).ok()?,
        area.y + u16::try_from(y).ok()?,
    ))
}
