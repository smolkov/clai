//! The details view (see `spec/tui.md`, "Details view"): a box over the
//! conversation with all of a call, the search line or the keys under it.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Clear};

use crate::tui::app::App;
use crate::tui::selection::Surface;
use crate::tui::wrap::fit;

/// The box for a screen of `size`: all of it.
fn frame_area(size: Rect) -> Rect {
    size
}

/// The rows of text: inside the border, after a cell of padding, above the
/// keys and the note.
pub fn area(app: &App, size: Rect) -> Rect {
    let outer = frame_area(size);
    let note = app.details.as_ref().is_some_and(|d| d.note.is_some());
    let below = 1 + u16::from(note);
    Rect::new(
        outer.x + 2,
        outer.y + 1,
        outer.width.saturating_sub(4),
        outer.height.saturating_sub(2 + below),
    )
}

pub fn draw(app: &App, frame: &mut Frame<'_>, size: Rect) {
    let Some(details) = &app.details else {
        return;
    };
    let look = &app.settings.look;
    let styles = &look.styles;
    let rule = look.symbols.rule;
    let outer = frame_area(size);
    let text = area(app, size);
    let title = fit(
        &details.title,
        usize::from(outer.width.saturating_sub(4)),
        look.symbols.more,
    );
    let block = Block::new()
        .borders(Borders::ALL)
        .border_set(look.symbols.border)
        .border_style(styles.border)
        .title(format!(" {title} "));
    frame.render_widget(Clear, outer);
    frame.render_widget(block, outer);
    let selection = app
        .selection
        .filter(|s| s.surface == Surface::Details && s.dragged);
    let buffer = frame.buffer_mut();
    for (i, (line, _)) in details
        .rows()
        .iter()
        .skip(details.top)
        .take(usize::from(text.height))
        .enumerate()
    {
        let y = text.y + u16::try_from(i).unwrap_or(0);
        buffer.set_line(text.x, y, line, text.width);
        let row = details.top + i;
        let found = details.found.is_some() && details.line_of(row) == details.found;
        if found {
            buffer.set_style(Rect::new(text.x, y, text.width, 1), styles.selection);
        }
        if let Some(selection) = selection {
            for x in 0..text.width {
                if selection.contains(row, usize::from(x))
                    && let Some(cell) = buffer.cell_mut((text.x + x, y))
                {
                    cell.modifier.toggle(Modifier::REVERSED);
                }
            }
        }
    }
    let mut y = text.y + text.height;
    if let Some(note) = &details.note {
        let line = Line::styled(
            fit(note, usize::from(text.width), look.symbols.more),
            styles.warning,
        );
        buffer.set_line(text.x, y, &line, text.width);
        y += 1;
    }
    let keys = match (&details.typing, details.missed, &details.search) {
        (Some(typed), _, _) => Line::styled(format!("/{typed}▏"), styles.text),
        (None, true, Some(search)) => Line::styled(format!("not found: {search}"), styles.warning),
        _ => {
            let side = if details.wrap { "" } else { "Left Right " };
            Line::styled(
                format!(
                    "Up Down {side}scroll {rule} / search {rule} n next {rule} y copy {rule} w wrap {rule} Esc close"
                ),
                styles.dim,
            )
        }
    };
    let keys = Line::from(
        keys.spans
            .into_iter()
            .map(|span| {
                let text = fit(&span.content, usize::from(text.width), look.symbols.more);
                ratatui::text::Span::styled(text, span.style)
            })
            .collect::<Vec<_>>(),
    );
    buffer.set_line(text.x, y, &keys, text.width);
}
