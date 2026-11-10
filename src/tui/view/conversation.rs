//! The conversation area: only the rows that are visible.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::tui::app::App;
use crate::tui::keys::Focus;
use crate::tui::selection::Surface;

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
        // Text selected with the mouse, in reverse colours.
        if let Some(selection) = app
            .selection
            .filter(|s| s.surface == Surface::Conversation && s.dragged)
        {
            let at = app.top + i;
            for x in 0..area.width {
                if selection.contains(at, usize::from(x))
                    && let Some(cell) = buffer.cell_mut((area.x + x, y))
                {
                    cell.modifier.toggle(Modifier::REVERSED);
                }
            }
        }
    }
}
