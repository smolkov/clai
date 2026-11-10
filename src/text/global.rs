//! Statistics over all sessions as rows (see `spec/tui.md`, "Global tab"):
//! the Totals section and the Models table for the status box, and the
//! whole answer as text for `clai chat`.

use crate::protocol::{DayTokens, GlobalStats, ModelTotals};

use super::status::{
    Options, Section, at_least, count, duration, requests_text, row, turns_text, warn,
};
use super::tokens;

/// The cells of the share bar.
pub const SHARE_CELLS: usize = 12;
/// The cells of the longest bar of a day in `clai chat`.
const DAY_BAR: usize = 30;
/// The narrowest and widest model column.
const NAME_MIN: usize = 13;
const NAME_MAX: usize = 28;

/// Year, month and day of a day since 1970 (Howard Hinnant's
/// `civil_from_days`).
pub fn civil(day: i64) -> (i64, u32, u32) {
    let days = day + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (
        y,
        u32::try_from(m).unwrap_or(1),
        u32::try_from(d).unwrap_or(1),
    )
}

/// `2026-11-04`.
pub fn date(day: i64) -> String {
    let (y, m, d) = civil(day);
    format!("{y:04}-{m:02}-{d:02}")
}

/// `11-04`.
pub fn month_day(day: i64) -> String {
    let (_, m, d) = civil(day);
    format!("{m:02}-{d:02}")
}

/// `Wed`.
pub fn weekday(day: i64) -> &'static str {
    // 1970-01-01 was a Thursday.
    match (day + 4).rem_euclid(7) {
        0 => "Sun",
        1 => "Mon",
        2 => "Tue",
        3 => "Wed",
        4 => "Thu",
        5 => "Fri",
        _ => "Sat",
    }
}

/// `2026-11-04 – 2026-11-10`.
fn span(stats: &GlobalStats, unicode: bool) -> String {
    let dash = if unicode { "–" } else { "-" };
    format!("{} {dash} {}", date(stats.first_day), date(stats.last_day))
}

pub fn totals(stats: &GlobalStats, options: &Options<'_>) -> Section {
    let rule = options.rule();
    let t = &stats.totals;
    let s = &stats.sessions;
    let mut sessions = format!("{} active of {}", s.active, s.saved);
    if s.running > 0 {
        sessions.push_str(&format!("{rule}{} running", s.running));
    }
    let exact = t.unknown_stretches == 0;
    let working = if !exact && t.working_ms == 0 {
        "?".to_owned()
    } else {
        format!(
            "{}{}",
            at_least(exact, options.unicode),
            duration(t.working_ms)
        )
    };
    let mut rows = vec![
        row("sessions", sessions),
        row("turns", turns_text(&t.turns)),
        row(
            "requests",
            requests_text(tokens(t.requests), t.retries, t.errors, rule),
        ),
        row(
            "tokens",
            format!(
                "in {}{rule}out {}",
                tokens(t.tokens.input_tokens),
                tokens(t.tokens.output_tokens)
            ),
        ),
        row("working", working),
    ];
    if stats.skipped > 0 {
        rows.push(warn(
            "skipped",
            format!(
                "{} {}",
                count(stats.skipped, "log", "logs"),
                "that cannot be read"
            ),
        ));
    }
    Section {
        title: "Totals".to_owned(),
        head: span(stats, options.unicode),
        rows,
    }
}

/// A row of the Models table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelRow {
    /// Padded to the name column.
    pub name: String,
    /// The numbers, after the name.
    pub columns: String,
    /// The part of all tokens, in percent; `None` for the `all` row.
    pub share: Option<u64>,
    /// No longer configured, or `unknown`: shown dim.
    pub gone: bool,
}

/// The Models table: its head line and rows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelTable {
    pub head: String,
    pub rows: Vec<ModelRow>,
}

fn columns(sessions: u64, requests: u64, input: u64, output: u64) -> String {
    format!(
        "{sessions:>8} {:>9} {:>7} {:>7}",
        tokens(requests),
        tokens(input),
        tokens(output)
    )
}

/// The table of models, `None` when no model was used in the range.
pub fn models(stats: &GlobalStats, unicode: bool) -> Option<ModelTable> {
    if stats.models.is_empty() {
        return None;
    }
    let width = stats
        .models
        .iter()
        .map(|m| m.model.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(NAME_MIN, NAME_MAX);
    let name = |text: &str| {
        let cut = super::cut(text, width, unicode);
        let pad = width.saturating_sub(cut.chars().count());
        format!("{cut}{}", " ".repeat(pad))
    };
    let all_tokens = stats.totals.tokens.input_tokens + stats.totals.tokens.output_tokens;
    let mut rows: Vec<ModelRow> = stats
        .models
        .iter()
        .map(|m: &ModelTotals| ModelRow {
            name: name(&m.model),
            columns: columns(m.sessions, m.requests, m.input_tokens, m.output_tokens),
            share: Some(
                (m.tokens().saturating_mul(100))
                    .checked_div(all_tokens)
                    .unwrap_or(0),
            ),
            gone: !m.configured,
        })
        .collect();
    if stats.models.len() > 1 {
        let t = &stats.totals;
        rows.push(ModelRow {
            name: name("all"),
            columns: columns(
                stats.sessions.active,
                t.requests,
                t.tokens.input_tokens,
                t.tokens.output_tokens,
            ),
            share: None,
            gone: false,
        });
    }
    let head = format!(
        "{:<width$}{:>8} {:>9} {:>7} {:>7}   share",
        "Models",
        "sessions",
        "requests",
        "in",
        "out",
        width = width + 2
    );
    Some(ModelTable { head, rows })
}

/// `████░░░░` or `[##..]`, `cells` wide inside.
pub fn share_bar(percent: u64, cells: usize, unicode: bool) -> String {
    let full = usize::try_from(percent.min(100)).unwrap_or(100) * cells / 100;
    let full = if percent > 0 { full.max(1) } else { 0 };
    if unicode {
        format!("{}{}", "█".repeat(full), "░".repeat(cells - full))
    } else {
        format!("[{}{}]", "#".repeat(full), ".".repeat(cells - full))
    }
}

/// `  12%`.
pub fn percent(share: u64) -> String {
    format!("{share:>3}%")
}

/// The whole answer as text, for `/stats` in `clai chat`.
pub fn text(stats: &GlobalStats, options: &Options<'_>) -> String {
    let mut out = super::status::text(&[totals(stats, options)]);
    out.push('\n');
    match models(stats, options.unicode) {
        Some(table) => {
            out.push_str(table.head.trim_end());
            out.push('\n');
            for r in &table.rows {
                let mut line = format!("  {}{}", r.name, r.columns);
                if let Some(share) = r.share {
                    line.push_str(&format!(
                        "   {} {}",
                        share_bar(share, SHARE_CELLS, options.unicode),
                        percent(share)
                    ));
                }
                out.push_str(line.trim_end());
                out.push('\n');
            }
        }
        None => out.push_str(&super::status::line("Models", "none", true)),
    }
    if stats.models.is_empty() {
        out.push('\n');
    }
    out.push('\n');
    out.push_str("Tokens per day (in + out)\n");
    let max = stats.days.iter().map(DayTokens::total).max().unwrap_or(0);
    for day in &stats.days {
        let total = day.total();
        let cells = usize::try_from(u128::from(total) * DAY_BAR as u128 / u128::from(max.max(1)))
            .unwrap_or(DAY_BAR);
        let cells = if total > 0 { cells.max(1) } else { 0 };
        let bar = if options.unicode { "█" } else { "#" }.repeat(cells);
        let line = format!(
            "  {} {}  {:>6}  {bar}",
            weekday(day.day),
            month_day(day.day),
            tokens(total)
        );
        out.push_str(line.trim_end());
        out.push('\n');
    }
    if stats.undated_tokens > 0 {
        out.push_str(&format!(
            "  {} tokens without a date\n",
            tokens(stats.undated_tokens)
        ));
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::protocol::{GlobalTotals, Range, SessionCounts, TokenTotals, TurnCounts};

    /// 2026-11-10.
    pub(crate) const TODAY: i64 = 20_767;

    /// A week of two models, with an empty day.
    pub(crate) fn week() -> GlobalStats {
        let days = [
            (1_200_000, 40_000),
            (2_300_000, 70_000),
            (0, 0),
            (900_000, 30_000),
            (4_000_000, 120_000),
            (2_000_000, 60_000),
            (500_000, 20_000),
        ];
        let days: Vec<DayTokens> = days
            .iter()
            .enumerate()
            .map(|(i, (input, output))| DayTokens {
                day: TODAY - 6 + i64::try_from(i).unwrap(),
                input_tokens: *input,
                output_tokens: *output,
            })
            .collect();
        let input: u64 = days.iter().map(|d| d.input_tokens).sum();
        let output: u64 = days.iter().map(|d| d.output_tokens).sum();
        GlobalStats {
            range: Range::Week,
            first_day: TODAY - 6,
            last_day: TODAY,
            sessions: SessionCounts {
                active: 12,
                saved: 31,
                running: 4,
            },
            totals: GlobalTotals {
                turns: TurnCounts {
                    done: 80,
                    cancelled: 3,
                    limit: 2,
                    failed: 2,
                },
                requests: 1400,
                retries: 21,
                errors: 3,
                tokens: TokenTotals {
                    input_tokens: input,
                    output_tokens: output,
                },
                working_ms: 24_000_000,
                unknown_stretches: 0,
            },
            models: vec![
                ModelTotals {
                    model: "anthropic/sonnet".into(),
                    configured: true,
                    sessions: 9,
                    requests: 1100,
                    input_tokens: input - 1_000_000,
                    output_tokens: output - 40_000,
                },
                ModelTotals {
                    model: "openai/gpt-old".into(),
                    configured: false,
                    sessions: 4,
                    requests: 300,
                    input_tokens: 1_000_000,
                    output_tokens: 40_000,
                },
            ],
            days,
            undated_tokens: 0,
            skipped: 0,
            now: 0,
        }
    }

    fn options(unicode: bool) -> Options<'static> {
        Options {
            unicode,
            offset: 0,
            me: "clai chat",
        }
    }

    #[test]
    fn dates() {
        assert_eq!(date(TODAY), "2026-11-10");
        assert_eq!(weekday(TODAY), "Tue");
        assert_eq!(month_day(TODAY - 9), "11-01");
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(weekday(0), "Thu");
    }

    #[test]
    fn totals_rows() {
        let section = totals(&week(), &options(true));
        assert_eq!(section.head, "2026-11-04 – 2026-11-10");
        let values: Vec<&str> = section.rows.iter().map(|r| r.value.as_str()).collect();
        assert_eq!(
            values,
            [
                "12 active of 31 ─ 4 running",
                "87: 80 done, 3 cancelled, 2 limit, 2 failed",
                "1.4k ─ 21 retries ─ 3 errors",
                "in 10.9M ─ out 340k",
                "6h 40m"
            ]
        );
        let mut skipped = week();
        skipped.skipped = 1;
        skipped.totals.unknown_stretches = 1;
        let section = totals(&skipped, &options(false));
        assert_eq!(section.rows[4].value, ">= 6h 40m");
        assert_eq!(section.rows[5].value, "1 log that cannot be read");
    }

    #[test]
    fn stats_as_text() {
        assert_eq!(
            text(&week(), &options(false)),
            "\
Totals         2026-11-04 - 2026-11-10
  sessions     12 active of 31 - 4 running
  turns        87: 80 done, 3 cancelled, 2 limit, 2 failed
  requests     1.4k - 21 retries - 3 errors
  tokens       in 10.9M - out 340k
  working      6h 40m

Models            sessions  requests      in     out   share
  anthropic/sonnet       9      1.1k    9.9M    300k   [##########..]  90%
  openai/gpt-old         4       300      1M     40k   [#...........]   9%
  all                   12      1.4k   10.9M    340k

Tokens per day (in + out)
  Wed 11-04    1.2M  #########
  Thu 11-05    2.3M  #################
  Fri 11-06       0
  Sat 11-07    930k  ######
  Sun 11-08    4.1M  ##############################
  Mon 11-09      2M  ###############
  Tue 11-10    520k  ###
"
        );
    }

    #[test]
    fn no_models() {
        let mut stats = week();
        stats.models.clear();
        stats.undated_tokens = 1_200_000;
        let text = text(&stats, &options(true));
        assert!(text.contains("Models         none\n\nTokens"), "{text}");
        assert!(text.ends_with("  1.2M tokens without a date\n"), "{text}");
    }
}
