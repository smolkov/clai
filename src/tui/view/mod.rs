//! Drawing only: reads [`App`], writes to the frame. No box around the screen:
//! the conversation takes the full width, plain lines set off the prompt bar
//! and the input, and two footer rows hold the status and the session (see
//! `spec/tui.md`, "Layout").

mod activity;
mod ask;
mod commands;
mod conversation;
mod day_chart;
mod details;
mod info;
mod input;
mod lsp;
mod picker;
mod popup;
pub mod prompt;
mod sessions;
mod settings;
mod status;
mod status_box;
mod themes;

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::Color;
use ratatui::widgets::{Paragraph, Wrap};

use super::app::{ACTIVITY_MIN_HEIGHT, App};
use super::style::Look;
use super::wrap::fit;
use unicode_width::UnicodeWidthStr;

/// The smallest terminal that shows more than a message.
pub const MIN_WIDTH: u16 = 40;
pub const MIN_HEIGHT: u16 = 10;

/// The status row and the session row.
const FOOTER: u16 = 2;
/// Cells before the footer text, in line with the text of the input.
const FOOTER_INDENT: u16 = 2;
/// Rows of the conversation kept above a tall question from the model.
const KEEP_ABOVE_ASK: u16 = 3;

/// Where the parts of the screen go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Areas {
    pub conversation: Rect,
    /// The activity line, with an empty row above it.
    pub activity: Option<Rect>,
    pub prompt: Option<Rect>,
    /// A question from the model, in the place of the input (RFC 0022):
    /// its rows between the rule with its title and the rule below.
    pub ask: Option<Rect>,
    /// No rows while a question from the model covers it.
    pub input: Rect,
    pub status: Rect,
    pub info: Rect,
}

/// How far the status box can scroll on a screen of `size`.
pub fn status_max_top(app: &App, size: Rect) -> usize {
    status_box::max_top(app, size)
}

/// How far the question from the model can scroll in `area`.
pub fn ask_max_top(app: &App, area: Rect) -> usize {
    ask::max_top(app, area)
}

/// Where the details view shows its text on a screen of `size`.
pub fn details_area(app: &App, size: Rect) -> Rect {
    details::area(app, size)
}

/// The areas for a screen of `size`; `None` if it is too small.
pub fn areas(app: &App, size: Rect) -> Option<Areas> {
    if size.width < MIN_WIDTH || size.height < MIN_HEIGHT {
        return None;
    }
    let width = size.width;
    let x = size.x;
    let footer = Rect::new(
        x + FOOTER_INDENT,
        size.y + size.height - FOOTER,
        width - FOOTER_INDENT,
        FOOTER,
    );
    let status = Rect {
        height: 1,
        ..footer
    };
    let info = Rect {
        y: footer.y + 1,
        height: 1,
        ..footer
    };
    if let Some(rows) = ask::height(app, usize::from(width)) {
        // The question takes the rows of the input and the rules around
        // it; 3 rows of the conversation stay above, but not on the
        // smallest screens.
        let keep = KEEP_ABOVE_ASK.min(size.height - MIN_HEIGHT);
        let most = (footer.y - size.y).saturating_sub(2 + keep).max(1);
        let height = u16::try_from(rows).unwrap_or(most).clamp(1, most);
        let ask = Rect::new(x, footer.y - 1 - height, width, height);
        let conversation = Rect::new(x, size.y, width, (ask.y - 1).saturating_sub(size.y));
        return Some(Areas {
            conversation,
            activity: None,
            prompt: None,
            ask: Some(ask),
            input: Rect::new(x, ask.y, width, 0),
            status,
            info,
        });
    }
    // At most a quarter of the height each, so that the conversation stays.
    let most = (size.height / 4).max(2);
    let input_rows = u16::try_from(input::height(app, width)).unwrap_or(most);
    let input_height = input_rows.clamp(1, most);
    // The line below the input lies between it and the footer.
    let input = Rect::new(x, footer.y - 1 - input_height, width, input_height);
    // The line above the input, and above the prompt bar if there is one.
    let mut end = input.y - 1;
    // A question from the model is a box over the screen, not a bar.
    let prompt = app.prompt.as_ref().filter(|p| !p.is_ask()).map(|p| {
        let rows = prompt::lines(p, usize::from(width), &app.settings.look).len();
        let height = u16::try_from(rows).unwrap_or(most).clamp(1, most);
        end -= height + 1;
        Rect::new(x, end + 1, width, height)
    });
    // The last row of the conversation's place, with an empty row above.
    let activity = (app.activity_on() && size.height >= ACTIVITY_MIN_HEIGHT).then(|| {
        end -= 2;
        Rect::new(x, end + 1, width, 1)
    });
    let conversation = Rect::new(x, size.y, width, end.saturating_sub(size.y));
    Some(Areas {
        conversation,
        activity,
        prompt,
        ask: None,
        input,
        status,
        info,
    })
}

/// Draws the whole screen, after [`App::layout`].
pub fn draw(app: &App, frame: &mut Frame<'_>) {
    let size = frame.area();
    let Some(areas) = areas(app, size) else {
        too_small(frame, size);
        return;
    };
    let look = &app.settings.look;
    let buffer = frame.buffer_mut();
    conversation::draw(app, buffer, areas.conversation);
    if let Some(area) = areas.activity {
        activity::draw(app, buffer, area);
    }
    if let (Some(area), Some(p)) = (areas.prompt, &app.prompt) {
        rule(buffer, size, area.y - 1, look);
        for (i, line) in prompt::lines(p, usize::from(area.width), look)
            .iter()
            .take(usize::from(area.height))
            .enumerate()
        {
            let y = area.y + u16::try_from(i).unwrap_or(0);
            buffer.set_line(area.x, y, line, area.width);
        }
    }
    if let Some(area) = areas.ask {
        ask::draw(app, frame, area);
    } else {
        let buffer = frame.buffer_mut();
        rule(buffer, size, areas.input.y - 1, look);
        input::draw(app, buffer, areas.input);
        rule(buffer, size, areas.input.y + areas.input.height, look);
    }
    let buffer = frame.buffer_mut();
    footer_row(buffer, areas.status, &status::text(app), look);
    footer_row(buffer, areas.info, &info::text(app), look);
    match cursor(app, &areas) {
        Some((x, y))
            if app.dialog.is_none()
                && !app.prompt.as_ref().is_some_and(|p| p.is_ask())
                && app.picker.is_none()
                && app.session_list.is_none()
                && app.settings_box.is_none()
                && app.status_box.is_none()
                && app.lsp_box.is_none()
                && app.details.is_none()
                && app.theme_list.is_none() =>
        {
            frame.set_cursor_position((x, y));
        }
        _ => {}
    }
    commands::draw(app, frame, areas.conversation, areas.input.y - 1);
    popup::draw(app, frame, size);
    sessions::draw(app, frame, size);
    picker::draw(app, frame, size);
    settings::draw(app, frame, size);
    themes::draw(app, frame, size);
    status_box::draw(app, frame, size);
    lsp::draw(app, frame, size);
    details::draw(app, frame, size);
    paint(frame.buffer_mut(), look);
}

/// A theme with `background` paints every cell the drawing left to the
/// terminal, and gives the theme's text colour to text that has none, so it
/// stays readable on that background.
fn paint(buffer: &mut Buffer, look: &Look) {
    let Some(background) = look.styles.background else {
        return;
    };
    let text = look.styles.text.fg;
    for cell in &mut buffer.content {
        if cell.bg == Color::Reset {
            cell.bg = background;
        }
        if cell.fg == Color::Reset
            && let Some(text) = text
        {
            cell.fg = text;
        }
    }
}

fn cursor(app: &App, areas: &Areas) -> Option<(u16, u16)> {
    if let (Some(prompt), Some(area)) = (&app.prompt, areas.prompt) {
        return prompt::cursor(prompt, area);
    }
    input::cursor(app, areas.input)
}

fn too_small(frame: &mut Frame<'_>, size: Rect) {
    let text = format!("terminal too small (at least {MIN_WIDTH} x {MIN_HEIGHT})");
    let paragraph = Paragraph::new(text)
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: true });
    let y = size.height / 2;
    let area = Rect::new(size.x, size.y + y, size.width, size.height - y);
    frame.render_widget(paragraph, area);
}

/// A plain line across the screen with a title near its start:
/// `── Question ─ 1/2 Database ──────`.
fn titled_rule(buffer: &mut Buffer, size: Rect, y: u16, title: &str, look: &Look) {
    rule(buffer, size, y, look);
    let lead = look.symbols.rule.repeat(2);
    let x = size.x + u16::try_from(lead.width()).unwrap_or(2);
    let room = usize::from(size.width).saturating_sub(lead.width());
    buffer.set_string(
        x,
        y,
        fit(title, room, look.symbols.more),
        look.styles.heading,
    );
}

/// A plain line across the screen.
fn rule(buffer: &mut Buffer, size: Rect, y: u16, look: &Look) {
    let line = look.symbols.rule.repeat(usize::from(size.width));
    buffer.set_string(size.x, y, line, look.styles.border);
}

/// A dim row of the footer, cut to fit.
fn footer_row(buffer: &mut Buffer, area: Rect, text: &str, look: &Look) {
    let text = fit(text, usize::from(area.width), look.symbols.more);
    buffer.set_string(area.x, area.y, text, look.styles.dim);
}
