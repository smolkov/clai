//! The status box (see `spec/tui.md`, "Status box"): the sections of
//! `text::status` with labels dimmed and warnings marked, scrolled when the
//! screen is too small.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::text::status::{LABEL_WIDTH, Options, Section, Tone, sections};
use crate::tui::app::App;
use crate::tui::style::Look;
use crate::tui::wrap::fit;

use super::popup;

/// The box's width, if the screen has room for it.
const WIDTH: u16 = 76;
/// The rows that are not status: a blank line and the keys.
const CHROME: usize = 2;

/// The sections for the open box, if it has an answer.
fn shown(app: &App) -> Option<Vec<Section>> {
    let status = app.status_box.as_ref()?.status.as_ref()?;
    let options = Options {
        unicode: app.settings.look.symbols.unicode,
        offset: app.settings.theming.offset,
        me: &app.settings.client,
    };
    Some(sections(status, &options))
}

/// `label` padded to the value column, then `value` cut to the rest.
fn row(
    label: &str,
    value: &str,
    label_style: ratatui::style::Style,
    value_style: ratatui::style::Style,
    indent: usize,
    width: usize,
    look: &Look,
) -> Line<'static> {
    // Titles and rows put their values in the same column.
    let column = LABEL_WIDTH + 2;
    let label = fit(label, column.saturating_sub(indent + 1), look.symbols.more);
    let padded = format!("{}{label}", " ".repeat(indent));
    let pad = column.saturating_sub(padded.width());
    let value = fit(value, width.saturating_sub(column), look.symbols.more);
    Line::from(vec![
        Span::styled(format!("{padded}{}", " ".repeat(pad)), label_style),
        Span::styled(value, value_style),
    ])
}

fn lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let look = &app.settings.look;
    let styles = &look.styles;
    let Some(sections) = shown(app) else {
        return vec![Line::styled("asking the server…", styles.dim)];
    };
    let mut lines = Vec::new();
    for (i, section) in sections.iter().enumerate() {
        if i > 0 {
            lines.push(Line::raw(""));
        }
        lines.push(row(
            &section.title,
            &section.head,
            styles.heading,
            styles.text,
            0,
            width,
            look,
        ));
        for r in &section.rows {
            let value_style = match r.tone {
                Tone::Normal => styles.text,
                Tone::Warning => styles.warning,
            };
            lines.push(row(
                &r.label,
                &r.value,
                styles.dim,
                value_style,
                2,
                width,
                look,
            ));
        }
    }
    lines
}

fn keys(look: &Look) -> Line<'static> {
    let rule = look.symbols.rule;
    Line::styled(
        format!("Up Down PgUp PgDn scroll {rule} Esc close"),
        look.styles.dim,
    )
}

fn cells(size: Rect) -> u16 {
    WIDTH.min(size.width.saturating_sub(4))
}

/// The rows of status the box shows on a screen of `size`, for `lines`.
fn room(size: Rect, lines: usize) -> usize {
    let wanted = lines.max(1) + CHROME;
    let inner = usize::from(size.height.saturating_sub(2));
    wanted.min(inner).saturating_sub(CHROME).max(1)
}

/// The largest first line, for `App::layout`.
pub fn max_top(app: &App, size: Rect) -> usize {
    if app.status_box.is_none() {
        return 0;
    }
    let count = lines(app, usize::from(cells(size))).len();
    count.saturating_sub(room(size, count))
}

fn title(app: &App) -> String {
    let rule = app.settings.look.symbols.rule;
    match &app.status_box {
        Some(open) => format!(" Status {rule} {} ", open.name),
        None => " Status ".to_owned(),
    }
}

pub fn draw(app: &App, frame: &mut Frame<'_>, size: Rect) {
    let Some(open) = &app.status_box else {
        return;
    };
    let look = &app.settings.look;
    let cells = cells(size);
    let all = lines(app, usize::from(cells));
    let room = room(size, all.len());
    let wanted = u16::try_from(all.len().max(1) + CHROME).unwrap_or(u16::MAX);
    let area = popup::open(frame, size, &title(app), cells, wanted, look);
    let mut shown: Vec<Line<'static>> = all.into_iter().skip(open.top).take(room).collect();
    shown.resize(usize::from(area.height.saturating_sub(1)), Line::raw(""));
    shown.push(keys(look));
    frame.render_widget(Paragraph::new(shown), area);
}
