//! The chart of tokens per day on the Global tab of the status box (see
//! `spec/tui.md`, "Global tab"): one column per day, input and output
//! stacked, with a y axis. Pure on its input. ratatui's `BarChart` neither
//! stacks two series nor draws a y axis, so the cells are drawn here.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use crate::protocol::{DayTokens, GlobalStats, Range};
use crate::text::global::{civil, month_day, weekday};
use crate::text::tokens;
use crate::tui::style::Look;

/// Rows of bars.
const HEIGHT: usize = 8;
/// Eighths of a cell.
const EIGHTHS: u64 = 8;
/// The y labels and the axis: `  4.0M ┤`.
const AXIS: usize = 7;
/// Below this width the chart is left out.
pub const MIN_WIDTH: usize = 40;
/// The widest column, gap included.
const MAX_SLOT: usize = 9;
/// The lower eighth blocks, from none to full.
const BLOCKS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// What a column stands for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Unit {
    Day,
    Week,
    Month,
}

#[derive(Debug, Clone, Copy)]
struct Column {
    /// The first day in it.
    day: i64,
    input: u64,
    output: u64,
}

impl Column {
    fn total(&self) -> u64 {
        self.input.saturating_add(self.output)
    }
}

/// The columns for `room` cells: days, or weeks, or months when the days do
/// not fit; the oldest months go when even those do not.
fn columns(days: &[DayTokens], room: usize) -> (Unit, Vec<Column>) {
    let group = |key: &dyn Fn(i64) -> i64| {
        let mut columns: Vec<(i64, Column)> = Vec::new();
        for d in days {
            let k = key(d.day);
            match columns.last_mut() {
                Some((last, column)) if *last == k => {
                    column.input = column.input.saturating_add(d.input_tokens);
                    column.output = column.output.saturating_add(d.output_tokens);
                }
                _ => columns.push((
                    k,
                    Column {
                        day: d.day,
                        input: d.input_tokens,
                        output: d.output_tokens,
                    },
                )),
            }
        }
        columns.into_iter().map(|(_, c)| c).collect::<Vec<_>>()
    };
    if days.len() <= room {
        return (Unit::Day, group(&|day| day));
    }
    // Weeks start on Monday; 1970-01-01 was a Thursday.
    let weeks = group(&|day| (day + 3).div_euclid(7));
    if weeks.len() <= room {
        return (Unit::Week, weeks);
    }
    let mut months = group(&|day| {
        let (y, m, _) = civil(day);
        y * 12 + i64::from(m)
    });
    let extra = months.len().saturating_sub(room);
    months.drain(..extra);
    (Unit::Month, months)
}

/// The smallest of 1, 2, 4, 5, 8 times a power of ten that is at least `max`:
/// its half is short too.
fn top(max: u64) -> u64 {
    let mut power = 1u64;
    loop {
        for m in [1, 2, 4, 5, 8] {
            let top = power.saturating_mul(m);
            if top >= max {
                return top;
            }
        }
        match power.checked_mul(10) {
            Some(next) => power = next,
            None => return u64::MAX,
        }
    }
}

/// `value` of `top` in eighths of the chart's height, rounded.
fn eighths(value: u64, top: u64) -> u64 {
    let full = u128::from(EIGHTHS) * HEIGHT as u128;
    let scaled =
        (u128::from(value) * full * 2 + u128::from(top.max(1))) / (2 * u128::from(top.max(1)));
    u64::try_from(scaled).unwrap_or(u64::MAX)
}

/// The label of a column, if it gets one.
fn label(unit: Unit, range: Range, index: usize, day: i64) -> Option<String> {
    match (unit, range) {
        (Unit::Day, Range::Week) => Some(weekday(day).to_owned()),
        (Unit::Day, Range::Month) => index.is_multiple_of(7).then(|| month_day(day)),
        (Unit::Day | Unit::Week, _) => Some(month_day(day)),
        (Unit::Month, _) => {
            let (y, m, _) = civil(day);
            Some(format!("{y:04}-{m:02}"))
        }
    }
}

/// How a column is named in the line under the chart.
fn name(unit: Unit, day: i64) -> String {
    match unit {
        Unit::Day => format!("{} {}", weekday(day), month_day(day)),
        Unit::Week => format!("week of {}", month_day(day)),
        Unit::Month => {
            let (y, m, _) = civil(day);
            format!("{y:04}-{m:02}")
        }
    }
}

/// One cell of a bar: its text and style.
fn cell(
    input: u64,
    total: u64,
    row: u64,
    look: &Look,
    in_style: Style,
    out_style: Style,
) -> (&'static str, Style) {
    let lower = row * EIGHTHS;
    let cell_in = input.saturating_sub(lower).min(EIGHTHS);
    let cell_total = total.saturating_sub(lower).min(EIGHTHS);
    let block = |n: u64| {
        BLOCKS
            .get(usize::try_from(n).unwrap_or(8))
            .copied()
            .unwrap_or("█")
    };
    if !look.symbols.unicode {
        // Whole cells: `#` input, `+` output.
        let whole = |n: u64| n.div_ceil(EIGHTHS).max(u64::from(n > 0));
        let (input, total) = (whole(input), whole(total).max(whole(input)));
        return if row < input {
            ("#", in_style)
        } else if row < total {
            ("+", out_style)
        } else {
            (" ", Style::new())
        };
    }
    match (cell_in, cell_total) {
        (_, 0) => (" ", Style::new()),
        (EIGHTHS, _) => ("█", in_style),
        (0, n) => (block(n), out_style),
        (n, EIGHTHS) => {
            let style = match out_style.fg {
                Some(colour) => in_style.bg(colour),
                None => in_style,
            };
            (block(n), style)
        }
        // The top of the bar holds both: the larger one colours it.
        (n, all) if n * 2 >= all => (block(all), in_style),
        (_, all) => (block(all), out_style),
    }
}

/// The section: a heading with the legend, the chart, the x labels and a
/// line on the highest day and the mean.
pub fn lines(stats: &GlobalStats, width: usize, look: &Look) -> Vec<Line<'static>> {
    let styles = &look.styles;
    let unicode = look.symbols.unicode;
    let (in_style, out_style) = (styles.user, styles.accent);
    let (in_mark, out_mark) = if unicode { ("█", "█") } else { ("#", "+") };
    let mut lines = vec![Line::from(vec![
        Span::styled("Tokens per day  ", styles.heading),
        Span::styled(in_mark, in_style),
        Span::styled(" in  ", styles.dim),
        Span::styled(out_mark, out_style),
        Span::styled(" out", styles.dim),
    ])];
    let undated = (stats.undated_tokens > 0).then(|| {
        Line::styled(
            format!("  {} tokens without a date", tokens(stats.undated_tokens)),
            styles.dim,
        )
    });
    let max_day = stats.days.iter().map(DayTokens::total).max().unwrap_or(0);
    if max_day == 0 {
        lines.push(Line::styled("  no tokens in this range", styles.dim));
        lines.extend(undated);
        return lines;
    }
    if width < MIN_WIDTH {
        lines.push(Line::styled(
            "  the box is too narrow for the chart",
            styles.dim,
        ));
        lines.extend(undated);
        return lines;
    }
    let room = width - AXIS;
    let (unit, columns) = columns(&stats.days, room);
    let slot = (room / columns.len().max(1)).clamp(1, MAX_SLOT);
    let bar = if slot >= 2 { slot - 1 } else { 1 };
    let max = columns.iter().map(Column::total).max().unwrap_or(0);
    let top = top(max);
    let heights: Vec<(u64, u64)> = columns
        .iter()
        .map(|c| {
            let total = eighths(c.total(), top).max(u64::from(c.total() > 0));
            (eighths(c.input, top).min(total), total)
        })
        .collect();
    let (tick, plain, corner, rule) = if unicode {
        ("┤", "│", "┼", "─")
    } else {
        ("+", "|", "+", "-")
    };
    for row in (0..HEIGHT).rev() {
        let label = if row == HEIGHT - 1 {
            tokens(top)
        } else if row == HEIGHT / 2 - 1 {
            tokens(top / 2)
        } else {
            String::new()
        };
        let axis = if label.is_empty() { plain } else { tick };
        let mut spans = vec![Span::styled(format!("{label:>5} {axis}"), styles.dim)];
        for (input, total) in &heights {
            let (text, style) = cell(
                *input,
                *total,
                u64::try_from(row).unwrap_or(0),
                look,
                in_style,
                out_style,
            );
            spans.push(Span::styled(text.repeat(bar), style));
            spans.push(Span::raw(" ".repeat(slot - bar)));
        }
        lines.push(Line::from(spans));
    }
    lines.push(Line::styled(
        format!("{:>5} {corner}{}", "0", rule.repeat(columns.len() * slot)),
        styles.dim,
    ));
    let mut labels = " ".repeat(AXIS);
    let mut free = 0;
    for (i, column) in columns.iter().enumerate() {
        let at = i * slot;
        if at < free {
            continue;
        }
        if let Some(text) = label(unit, stats.range, i, column.day) {
            if at + text.len() > room {
                break;
            }
            labels.push_str(&" ".repeat(at.saturating_sub(labels.len() - AXIS)));
            labels.push_str(&text);
            free = at + text.len() + 1;
        }
    }
    lines.push(Line::styled(labels, styles.dim));
    let total: u64 = stats.days.iter().map(DayTokens::total).sum();
    let days = u64::try_from(stats.days.len()).unwrap_or(1).max(1);
    let rule = look.symbols.rule;
    let mut summary = String::from("max ");
    if let Some(highest) = columns.iter().rfind(|c| c.total() == max) {
        summary.push_str(&format!(
            "{} {} {}",
            tokens(max),
            if unit == Unit::Day { "on" } else { "in" },
            name(unit, highest.day)
        ));
    }
    // The mean is per day, whatever a column is.
    summary.push_str(&format!(" {rule} avg {}", tokens(total / days)));
    match unit {
        Unit::Day => {}
        Unit::Week => summary.push_str(&format!(" {rule} a column is a week")),
        Unit::Month => summary.push_str(&format!(" {rule} a column is a month")),
    }
    lines.push(Line::styled(
        format!("{}{summary}", " ".repeat(AXIS)),
        styles.dim,
    ));
    lines.extend(undated);
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tops_have_short_halves() {
        assert_eq!(top(0), 1);
        assert_eq!(top(3), 4);
        assert_eq!(top(4_100_000), 5_000_000);
        assert_eq!(top(9_000), 10_000);
        assert_eq!(top(1_600), 2_000);
    }

    fn days(n: i64, first: i64) -> Vec<DayTokens> {
        (0..n)
            .map(|i| DayTokens {
                day: first + i,
                input_tokens: 1,
                output_tokens: 0,
            })
            .collect()
    }

    #[test]
    fn days_become_weeks_then_months() {
        // 2026-11-10 was a Tuesday.
        let (unit, cols) = columns(&days(30, 20_767 - 29), 33);
        assert_eq!((unit, cols.len()), (Unit::Day, 30));
        let (unit, cols) = columns(&days(60, 20_767 - 59), 33);
        assert_eq!(unit, Unit::Week);
        assert_eq!(cols.len(), 10);
        // The first week is cut by the range; the rest start on Monday.
        assert_eq!(weekday(cols[1].day), "Mon");
        assert_eq!(cols.iter().map(|c| c.input).sum::<u64>(), 60);
        let (unit, cols) = columns(&days(800, 20_767 - 799), 20);
        assert_eq!(unit, Unit::Month);
        assert_eq!(cols.len(), 20);
        assert_eq!(cols.last().unwrap().day, 20_767 - 9);
    }

    fn text(stats: &GlobalStats, width: usize, unicode: bool) -> String {
        let look = Look::new(unicode, false);
        lines(stats, width, &look)
            .iter()
            .map(|line| {
                let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
                format!("{}\n", text.trim_end())
            })
            .collect()
    }

    /// `n` days up to 2026-11-10, with tokens that rise and fall.
    fn stats(range: Range, n: i64) -> GlobalStats {
        let today = 20_767;
        let days = (0..n)
            .map(|i| {
                let wave = u64::try_from((i * 37) % 11).unwrap();
                DayTokens {
                    day: today - n + 1 + i,
                    input_tokens: wave * 300_000,
                    output_tokens: wave * 90_000,
                }
            })
            .collect();
        GlobalStats {
            range,
            first_day: today - n + 1,
            last_day: today,
            days,
            ..GlobalStats::default()
        }
    }

    #[test]
    fn chart_of_30_days() {
        insta::assert_snapshot!(text(&stats(Range::Month, 30), 72, true));
    }

    #[test]
    fn chart_of_all_time_in_weeks_and_months() {
        insta::assert_snapshot!("weeks", text(&stats(Range::All, 200), 72, true));
        let mut months = stats(Range::All, 900);
        months.undated_tokens = 1_200_000;
        insta::assert_snapshot!("months", text(&months, 72, false));
    }

    #[test]
    fn chart_without_tokens() {
        let mut empty = stats(Range::Week, 7);
        for day in &mut empty.days {
            *day = DayTokens {
                day: day.day,
                ..DayTokens::default()
            };
        }
        assert_eq!(
            text(&empty, 72, true),
            "Tokens per day  █ in  █ out\n  no tokens in this range\n"
        );
    }
}
