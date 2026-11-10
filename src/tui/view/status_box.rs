//! The status box (see `spec/tui.md`, "Status box"): two tabs in the title;
//! on the Session tab the sections of `text::status`, on the Global tab those
//! of `text::global` and the chart of tokens per day. Labels are dimmed and
//! warnings marked; the tab scrolls when the screen is too small.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::protocol::{GlobalStats, Range};
use crate::text::global::{self, SHARE_CELLS, percent, share_bar};
use crate::text::status::{LABEL_WIDTH, Options, Section, Tone, sections};
use crate::tui::app::App;
use crate::tui::status::{OLD, Tab};
use crate::tui::style::Look;
use crate::tui::wrap::fit;

use super::{day_chart, popup};

/// The box's width, if the screen has room for it.
const WIDTH: u16 = 76;
/// The rows that are not status: a blank line and the keys.
const CHROME: usize = 2;

fn options(app: &App) -> Options<'_> {
    Options {
        unicode: app.settings.look.symbols.unicode,
        offset: app.settings.theming.offset,
        me: &app.settings.client,
    }
}

/// The sections for the open box, if it has an answer.
fn shown(app: &App) -> Option<Vec<Section>> {
    let status = app.status_box.as_ref()?.status.as_ref()?;
    Some(sections(status, &options(app)))
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
    if app
        .status_box
        .as_ref()
        .is_some_and(|open| open.tab == Tab::Global)
    {
        return global_lines(app, width);
    }
    let Some(sections) = shown(app) else {
        return vec![Line::styled("asking the server…", styles.dim)];
    };
    section_lines(&sections, width, look)
}

/// Sections as rows, with a blank line between them.
fn section_lines(sections: &[Section], width: usize, look: &Look) -> Vec<Line<'static>> {
    let styles = &look.styles;
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

/// The Global tab: Totals, Models and the chart.
fn global_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let look = &app.settings.look;
    let styles = &look.styles;
    let Some(open) = &app.status_box else {
        return Vec::new();
    };
    let Some(counted) = open.global.get(&open.range) else {
        return vec![match &open.global_error {
            Some(error) => Line::styled(error.clone(), styles.error),
            None => Line::styled("counting the logs of all sessions…", styles.dim),
        }];
    };
    let stats: &GlobalStats = &counted.stats;
    let mut lines = section_lines(&[global::totals(stats, &options(app))], width, look);
    lines.push(Line::raw(""));
    lines.extend(model_lines(stats, width, look));
    lines.push(Line::raw(""));
    lines.extend(day_chart::lines(stats, width, look));
    lines
}

/// The Models table, with the share as a bar in the `user` colour.
fn model_lines(stats: &GlobalStats, width: usize, look: &Look) -> Vec<Line<'static>> {
    let styles = &look.styles;
    let Some(table) = global::models(stats, look.symbols.unicode) else {
        return vec![row(
            "Models",
            "none",
            styles.heading,
            styles.text,
            0,
            width,
            look,
        )];
    };
    let (title, columns) = table.head.split_at("Models".len());
    let mut lines = vec![Line::from(vec![
        Span::styled(title.to_owned(), styles.heading),
        Span::styled(columns.to_owned(), styles.dim),
    ])];
    for r in table.rows {
        let name_style = if r.gone { styles.dim } else { styles.text };
        let mut spans = vec![
            Span::styled(format!("  {}", r.name), name_style),
            Span::styled(r.columns, styles.text),
        ];
        if let Some(share) = r.share {
            spans.push(Span::raw("   "));
            spans.push(Span::styled(
                share_bar(share, SHARE_CELLS, look.symbols.unicode),
                styles.user,
            ));
            spans.push(Span::styled(format!(" {}", percent(share)), styles.text));
        }
        lines.push(Line::from(spans));
    }
    lines
}

fn keys(app: &App) -> Line<'static> {
    let look = &app.settings.look;
    let rule = look.symbols.rule;
    let global = app
        .status_box
        .as_ref()
        .is_some_and(|open| open.tab == Tab::Global);
    let text = if global {
        format!(
            "Tab session {rule} 7 3 a range {rule} R count again {rule} Up Down scroll {rule} Esc close"
        )
    } else {
        format!("Tab global {rule} Up Down PgUp PgDn scroll {rule} Esc close")
    };
    Line::styled(text, look.styles.dim)
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

/// `7 days`, `30 days`, `all`.
fn range_name(range: Range) -> &'static str {
    match range {
        Range::Week => "7 days",
        Range::Month => "30 days",
        Range::All => "all",
    }
}

/// The tabs, the one in use bold, and on the Global tab the age of an old
/// answer; at the right, on the Global tab, the ranges. A box of `cells`
/// too narrow for both names only the range in use, after the tabs.
fn title(app: &App, cells: u16) -> (Line<'static>, Option<Line<'static>>) {
    let look = &app.settings.look;
    let styles = &look.styles;
    let rule = look.symbols.rule;
    let bar = if look.symbols.unicode { "│" } else { "|" };
    let Some(open) = &app.status_box else {
        return (Line::raw(" Status "), None);
    };
    let tab = |text: String, on: bool| {
        if on {
            Span::styled(text, styles.heading)
        } else {
            Span::styled(text, styles.dim)
        }
    };
    let global = open.tab == Tab::Global;
    let mut spans = vec![
        Span::raw(format!(" Status {rule} ")),
        tab(open.name.clone(), !global),
        Span::raw(format!(" {bar} ")),
        tab("Global".to_owned(), global),
    ];
    if global
        && let Some(counted) = open.global.get(&open.range)
        && let Some(age) = app.now.checked_duration_since(counted.at)
        && age >= OLD
    {
        spans.push(Span::styled(
            format!(
                " {rule} {} old",
                crate::text::status::duration(u64::try_from(age.as_millis()).unwrap_or(u64::MAX))
            ),
            styles.dim,
        ));
    }
    let left = Line::from(spans.clone());
    spans.push(Span::raw(" "));
    let right = global.then(|| {
        let mut spans = vec![Span::raw(" ")];
        for (i, range) in Range::ALL.into_iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(format!(" {bar} ")));
            }
            spans.push(tab(range_name(range).to_owned(), range == open.range));
        }
        spans.push(Span::raw(" "));
        Line::from(spans)
    });
    let wanted = left.width() + right.as_ref().map_or(0, Line::width) + 4;
    match right {
        Some(_) if wanted > usize::from(cells) => {
            let mut spans = left.spans;
            spans.push(Span::raw(format!(" {rule} ")));
            spans.push(Span::styled(range_name(open.range), styles.heading));
            spans.push(Span::raw(" "));
            (Line::from(spans), None)
        }
        right => {
            let mut spans = left.spans;
            spans.push(Span::raw(" "));
            (Line::from(spans), right)
        }
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
    let area = popup::open_titled(frame, size, title(app, cells), cells, wanted, look);
    let mut shown: Vec<Line<'static>> = all.into_iter().skip(open.top).take(room).collect();
    shown.resize(usize::from(area.height.saturating_sub(1)), Line::raw(""));
    shown.push(keys(app));
    frame.render_widget(Paragraph::new(shown), area);
}
