//! The conversation area: only the rows that are visible.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;

use crate::tui::app::App;
use crate::tui::keys::Focus;

pub fn draw(app: &App, buffer: &mut Buffer, area: Rect) {
    let selected = (app.focus == Focus::Conversation)
        .then_some(app.selected)
        .flatten();
    let rows = app.conversation.rows(app.top, usize::from(area.height));
    for (i, row) in rows.iter().enumerate() {
        let y = area.y + u16::try_from(i).unwrap_or(0);
        if let Some(line) = row.line {
            buffer.set_line(area.x, y, line, area.width);
        }
        if row.first && row.entry == selected && selected.is_some() {
            let cells = Rect::new(area.x, y, area.width, 1);
            buffer.set_style(cells, app.settings.look.styles.selection);
        }
    }
}
