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
mod skills;
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
/// Rows a box at the bottom gets before rows are kept for the conversation
/// above it, on a small screen.
const PANEL_MIN: u16 = 8;

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
    /// A box a command opened, in the place of the input (RFC 0024): its
    /// rows between the rule with its title and the rule below.
    pub panel: Option<Rect>,
    /// The settings on the whole screen (RFC 0024): the rows between the
    /// rule with the title at the top and the rule above the footer.
    pub screen: Option<Rect>,
    /// No rows while a question from the model or a box covers it.
    pub input: Rect,
    pub status: Rect,
    pub info: Rect,
}

/// The box at the bottom: the one that takes the keys, as `App` routes
/// them. Settings and the details view take the whole screen instead.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Panel {
    Status,
    Lsp,
    Themes,
    Picker,
    Skills,
    Sessions,
    Dialog,
}

/// The box at the bottom, if one is open.
pub fn panel(app: &App) -> Option<Panel> {
    if app.status_box.is_some() {
        Some(Panel::Status)
    } else if app.lsp_box.is_some() {
        Some(Panel::Lsp)
    } else if app.theme_list.is_some() {
        Some(Panel::Themes)
    } else if app.picker.is_some() {
        Some(Panel::Picker)
    } else if app.skills_box.is_some() {
        Some(Panel::Skills)
    } else if app.session_list.is_some() {
        Some(Panel::Sessions)
    } else if app.dialog.is_some() {
        Some(Panel::Dialog)
    } else {
        None
    }
}

/// The rows the open box wants for its text at `width` cells.
fn panel_rows(app: &App, panel: Panel, width: usize) -> usize {
    match panel {
        Panel::Status => status_box::wanted(app, width),
        Panel::Lsp => lsp::wanted(app, width),
        Panel::Themes => themes::wanted(app, width),
        Panel::Picker => picker::wanted(app, width),
        Panel::Skills => skills::wanted(app, width),
        Panel::Sessions => sessions::wanted(app, width),
        Panel::Dialog => popup::wanted(app),
    }
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
    // The settings take the screen; a theme list opened from them lies at
    // its bottom.
    let screen = app
        .settings_box
        .is_some()
        .then(|| Rect::new(x, size.y + 1, width, (footer.y - size.y).saturating_sub(2)));
    let keep = KEEP_ABOVE_ASK.min(size.height - MIN_HEIGHT);
    if let Some(open) = panel(app) {
        let rows = u16::try_from(panel_rows(app, open, usize::from(width))).unwrap_or(u16::MAX);
        // The rows between the two rules; the conversation keeps `keep`
        // rows above, unless the box would get fewer than `PANEL_MIN`.
        let free = (footer.y - size.y).saturating_sub(2).max(1);
        let most = free.saturating_sub(keep).max(rows.min(PANEL_MIN)).min(free);
        let height = rows.clamp(1, most.max(1));
        let panel = Rect::new(x, footer.y - 1 - height, width, height);
        let above = if screen.is_some() {
            0
        } else {
            (panel.y - 1).saturating_sub(size.y)
        };
        return Some(Areas {
            conversation: Rect::new(x, size.y, width, above),
            activity: None,
            prompt: None,
            ask: None,
            panel: Some(panel),
            screen,
            input: Rect::new(x, panel.y, width, 0),
            status,
            info,
        });
    }
    if let Some(screen) = screen {
        return Some(Areas {
            conversation: Rect::new(x, size.y, width, 0),
            activity: None,
            prompt: None,
            ask: None,
            panel: None,
            screen: Some(screen),
            input: Rect::new(x, footer.y - 1, width, 0),
            status,
            info,
        });
    }
    if let Some(rows) = ask::height(app, usize::from(width)) {
        // The question takes the rows of the input and the rules around
        // it; 3 rows of the conversation stay above, but not on the
        // smallest screens.
        let most = (footer.y - size.y).saturating_sub(2 + keep).max(1);
        let height = u16::try_from(rows).unwrap_or(most).clamp(1, most);
        let ask = Rect::new(x, footer.y - 1 - height, width, height);
        let conversation = Rect::new(x, size.y, width, (ask.y - 1).saturating_sub(size.y));
        return Some(Areas {
            conversation,
            activity: None,
            prompt: None,
            ask: Some(ask),
            panel: None,
            screen: None,
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
        panel: None,
        screen: None,
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
    if areas.conversation.height > 0 {
        conversation::draw(app, buffer, areas.conversation);
    }
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
    if areas.panel.is_some() || areas.screen.is_some() {
        // A box covers the input.
    } else if let Some(area) = areas.ask {
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
            if areas.panel.is_none()
                && areas.screen.is_none()
                && app.dialog.is_none()
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
    if areas.panel.is_none() && areas.screen.is_none() {
        commands::draw(app, frame, areas.conversation, areas.input.y - 1);
    }
    if let Some(area) = areas.screen {
        settings::draw(app, frame, area);
    }
    if let Some(area) = areas.panel {
        match panel(app) {
            Some(Panel::Status) => status_box::draw(app, frame, area),
            Some(Panel::Lsp) => lsp::draw(app, frame, area),
            Some(Panel::Themes) => themes::draw(app, frame, area),
            Some(Panel::Picker) => picker::draw(app, frame, area),
            Some(Panel::Skills) => skills::draw(app, frame, area),
            Some(Panel::Sessions) => sessions::draw(app, frame, area),
            Some(Panel::Dialog) => popup::draw(app, frame, area),
            None => {}
        }
    }
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

/// The text area of a box at the bottom or on the whole screen, given its
/// rows: clears them and draws the rule with the title above and the rule
/// below, with a title at the right end of the top rule if given.
pub(super) fn open_panel(
    frame: &mut Frame<'_>,
    area: Rect,
    (title, right): (
        ratatui::text::Line<'static>,
        Option<ratatui::text::Line<'static>>,
    ),
    look: &Look,
) -> Rect {
    frame.render_widget(ratatui::widgets::Clear, area);
    let full = Rect::new(
        area.x,
        area.y.saturating_sub(1),
        area.width,
        area.height + 2,
    );
    let buffer = frame.buffer_mut();
    rule(buffer, full, full.y, look);
    rule(buffer, full, area.y + area.height, look);
    let lead = look.symbols.rule.repeat(2);
    let x = area.x + u16::try_from(lead.width()).unwrap_or(2);
    let right_width = right.as_ref().map_or(0, |r| r.width());
    let room = usize::from(area.width)
        .saturating_sub(lead.width() + right_width + 2)
        .max(1);
    let title = ratatui::text::Line::from(
        title
            .spans
            .into_iter()
            .map(|mut s| {
                if s.style == ratatui::style::Style::default() {
                    s.style = look.styles.heading;
                }
                s
            })
            .collect::<Vec<_>>(),
    );
    buffer.set_line(x, full.y, &title, u16::try_from(room).unwrap_or(u16::MAX));
    if let Some(right) = right {
        let width = u16::try_from(right.width()).unwrap_or(0);
        let at = (area.x + area.width).saturating_sub(width + 2);
        buffer.set_line(at, full.y, &right, width);
    }
    area
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
