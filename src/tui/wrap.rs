//! Wrapping styled text to a width in terminal cells. Every produced line is
//! one screen row, so that the heights of entries are exact.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// A piece of text in one style.
pub type Piece = (String, Style);

/// Where a drawn row comes from, for copying (see `spec/tui.md`, "Copy"):
/// what of it is text, and whether it goes on the row above.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    /// Cells at the start that are not text: markers, prompts, indentation.
    pub prefix: usize,
    /// Cells after which it is no text any more: the state mark of a call.
    pub end: usize,
    /// It continues the row above: the TUI wrapped the line.
    pub joined: bool,
    /// A space was dropped where the line was wrapped.
    pub space: bool,
    /// Not text at all, such as `… 12 more lines`: never copied.
    pub skip: bool,
}

impl Source {
    /// A row of its own whose text starts after `prefix` cells.
    pub fn text(prefix: usize) -> Source {
        Source {
            prefix,
            end: usize::MAX,
            joined: false,
            space: false,
            skip: false,
        }
    }

    /// A row that is never copied.
    pub fn skipped() -> Source {
        Source {
            skip: true,
            ..Source::text(0)
        }
    }
}

impl Default for Source {
    fn default() -> Source {
        Source::text(0)
    }
}

/// The text of the cells `from..to` of a row, as far as they are text.
pub fn cells(line: &Line<'_>, source: &Source, from: usize, to: usize) -> String {
    let from = from.max(source.prefix);
    let to = to.min(source.end);
    let mut at = 0;
    let mut out = String::new();
    for span in &line.spans {
        for c in span.content.chars() {
            let w = c.width().unwrap_or(0);
            if at >= from && at < to {
                out.push(c);
            }
            at += w;
        }
    }
    out
}

/// Lines and sources of rows `from`..`to` (inclusive rows; cells from the
/// first row's `from.1`, up to the last row's `to.1`) as the user reads them:
/// wrapped rows joined, markers left out, trailing spaces removed. A missing
/// row (`None`) is an empty line.
pub fn copy_text<'a>(
    rows: impl IntoIterator<Item = Option<(&'a Line<'static>, &'a Source)>>,
    from_cell: usize,
    to_cell: usize,
) -> String {
    let rows: Vec<Option<(&Line<'static>, &Source)>> = rows.into_iter().collect();
    let last = rows.len().saturating_sub(1);
    let mut lines: Vec<String> = Vec::new();
    for (i, row) in rows.into_iter().enumerate() {
        let from = if i == 0 { from_cell } else { 0 };
        let to = if i == last { to_cell } else { usize::MAX };
        match row {
            Some((_, source)) if source.skip => {}
            Some((line, source)) => {
                let text = cells(line, source, from, to);
                match lines.last_mut() {
                    Some(previous) if source.joined && i > 0 => {
                        if source.space && !previous.ends_with(' ') {
                            previous.push(' ');
                        }
                        previous.push_str(&text);
                    }
                    _ => lines.push(text),
                }
            }
            None => lines.push(String::new()),
        }
    }
    let lines: Vec<&str> = lines.iter().map(|l| l.trim_end()).collect();
    lines.join("\n")
}

const TAB: &str = "    ";

/// Wraps `pieces` to `width` cells, at spaces where it can. The first row
/// starts with `first`, the others with `rest`. No text gives one row with
/// only the prefix. Tabs become spaces; other control characters (such as
/// the escape sequences of a command's output) are dropped.
pub fn wrap(pieces: &[Piece], width: usize, first: &Piece, rest: &Piece) -> Vec<Line<'static>> {
    wrap_rows(pieces, width, first, rest)
        .into_iter()
        .map(|(line, _)| line)
        .collect()
}

/// As [`wrap`], with where each row comes from: the prefixes are not text,
/// and the rows after the first go on the one above.
pub fn wrap_rows(
    pieces: &[Piece],
    width: usize,
    first: &Piece,
    rest: &Piece,
) -> Vec<(Line<'static>, Source)> {
    let mut rows: Vec<Vec<(char, Style)>> = Vec::new();
    // For each row after the first: whether a space was dropped before it.
    let mut spaces: Vec<bool> = Vec::new();
    let mut dropped = false;
    let mut row: Vec<(char, Style)> = Vec::new();
    let mut used = 0;
    // Where the text after the last space starts in `row`, if any.
    let mut after_space = None;
    let avail = |rows: &Vec<Vec<(char, Style)>>| {
        let prefix = if rows.is_empty() { first } else { rest };
        width.saturating_sub(prefix.0.width()).max(1)
    };
    let chars = pieces.iter().flat_map(|(text, style)| {
        text.chars().flat_map(move |c| {
            let tab = c == '\t';
            let spaces = if tab { TAB.len() } else { 1 };
            std::iter::repeat_n(if tab { ' ' } else { c }, spaces).map(move |c| (c, *style))
        })
    });
    for (c, style) in chars.filter(|(c, _)| !c.is_control()) {
        let w = c.width().unwrap_or(0);
        if used + w > avail(&rows) && !row.is_empty() {
            // A space that does not fit is the break itself.
            let tail = match after_space {
                Some(at) if at < row.len() && c != ' ' => row.split_off(at),
                _ => Vec::new(),
            };
            rows.push(std::mem::replace(&mut row, tail));
            spaces.push(std::mem::take(&mut dropped));
            used = row.iter().map(|(c, _)| c.width().unwrap_or(0)).sum();
            after_space = None;
        }
        // A space that would start a wrapped row is dropped.
        if c == ' ' && row.is_empty() && !rows.is_empty() {
            dropped = true;
            continue;
        }
        row.push((c, style));
        used += w;
        if c == ' ' {
            after_space = Some(row.len());
        }
    }
    rows.push(row);
    spaces.push(dropped);
    rows.into_iter()
        .enumerate()
        .map(|(i, row)| {
            let prefix = if i == 0 { first } else { rest };
            let source = Source {
                joined: i > 0,
                // A space dropped at the start of this row.
                space: i > 0 && spaces.get(i).copied().unwrap_or(false),
                ..Source::text(prefix.0.width())
            };
            (line(prefix, &row), source)
        })
        .collect()
}

/// A row of characters as spans, one per run of the same style.
fn line(prefix: &Piece, row: &[(char, Style)]) -> Line<'static> {
    let mut spans = Vec::new();
    if !prefix.0.is_empty() {
        spans.push(Span::styled(prefix.0.clone(), prefix.1));
    }
    let mut text = String::new();
    let mut current = None;
    for (c, style) in row {
        if current.is_some_and(|s| s != *style) {
            spans.push(Span::styled(
                std::mem::take(&mut text),
                current.unwrap_or_default(),
            ));
        }
        current = Some(*style);
        text.push(*c);
    }
    if !text.is_empty() {
        spans.push(Span::styled(text, current.unwrap_or_default()));
    }
    Line::from(spans)
}

/// Wraps plain text of one style, without a prefix.
pub fn plain(text: &str, width: usize, style: Style) -> Vec<Line<'static>> {
    let none = (String::new(), Style::new());
    wrap(&[(text.to_owned(), style)], width, &none, &none)
}

/// The text cut to `width` cells, with `more` at the end if it was too long.
pub fn fit(text: &str, width: usize, more: &str) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    let marked = more.width() <= width;
    let keep = if marked { width - more.width() } else { width };
    let mut used = 0;
    let mut out = String::new();
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w > keep {
            break;
        }
        used += w;
        out.push(c);
    }
    if marked {
        out.push_str(more);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line<'_>]) -> Vec<String> {
        lines.iter().map(ToString::to_string).collect()
    }

    fn wrapped(s: &str, width: usize) -> Vec<String> {
        text(&plain(s, width, Style::new()))
    }

    #[test]
    fn breaks_at_spaces() {
        assert_eq!(wrapped("aaa bbb ccc", 7), ["aaa bbb", "ccc"]);
        assert_eq!(wrapped("aaa bbb ccc", 5), ["aaa ", "bbb ", "ccc"]);
    }

    #[test]
    fn long_words_break_anywhere() {
        assert_eq!(wrapped("abcdefgh", 3), ["abc", "def", "gh"]);
    }

    #[test]
    fn empty_text_is_one_row() {
        assert_eq!(wrapped("", 3), [""]);
    }

    #[test]
    fn wide_characters_take_two_cells() {
        assert_eq!(wrapped("日本語", 4), ["日本", "語"]);
    }

    #[test]
    fn control_characters_are_dropped_and_tabs_are_spaces() {
        assert_eq!(wrapped("a\x1b[31mb\tc", 20), ["a[31mb    c"]);
    }

    #[test]
    fn prefixes_count() {
        let first = ("> ".to_owned(), Style::new());
        let rest = ("  ".to_owned(), Style::new());
        let lines = wrap(&[("aaa bbb".to_owned(), Style::new())], 5, &first, &rest);
        assert_eq!(text(&lines), ["> aaa", "  bbb"]);
    }

    #[test]
    fn styles_stay_with_their_text() {
        let red = Style::new().fg(ratatui::style::Color::Red);
        let lines = wrap(
            &[("a".to_owned(), red), ("b".to_owned(), Style::new())],
            5,
            &(String::new(), Style::new()),
            &(String::new(), Style::new()),
        );
        assert_eq!(lines[0].spans.len(), 2);
        assert_eq!(lines[0].spans[0].style, red);
    }

    fn copied(s: &str, width: usize) -> String {
        let none = (String::new(), Style::new());
        let rows = wrap_rows(&[(s.to_owned(), Style::new())], width, &none, &none);
        copy_text(rows.iter().map(|(l, s)| Some((l, s))), 0, usize::MAX)
    }

    #[test]
    fn wrapped_rows_copy_as_one_line() {
        for width in [3, 5, 7, 20] {
            assert_eq!(copied("aaa bbb ccc", width), "aaa bbb ccc", "{width}");
        }
        assert_eq!(copied("abcdefgh", 3), "abcdefgh");
    }

    #[test]
    fn prefixes_are_not_copied_and_parts_of_rows_are() {
        let first = ("> ".to_owned(), Style::new());
        let rest = ("  ".to_owned(), Style::new());
        let mut rows = wrap_rows(&[("aaa bbb".to_owned(), Style::new())], 5, &first, &rest);
        rows.push((Line::raw("… 3 more"), Source::skipped()));
        rows.extend(wrap_rows(
            &[("ccc  ".to_owned(), Style::new())],
            9,
            &first,
            &rest,
        ));
        let all = |from, to| copy_text(rows.iter().map(|(l, s)| Some((l, s))), from, to);
        assert_eq!(all(0, usize::MAX), "aaa bbb\nccc");
        // From the third cell of the first row to the fourth of the last.
        assert_eq!(all(3, 4), "aa bbb\ncc");
        let with_gap = copy_text(
            [
                Some((&rows[0].0, &rows[0].1)),
                None,
                Some((&rows[3].0, &rows[3].1)),
            ],
            0,
            usize::MAX,
        );
        assert_eq!(with_gap, "aaa\n\nccc");
    }

    #[test]
    fn fit_cuts_by_width() {
        assert_eq!(fit("abc", 3, "…"), "abc");
        assert_eq!(fit("abcd", 3, "…"), "ab…");
        assert_eq!(fit("日本語", 5, "…"), "日本…");
        assert_eq!(fit("abcd", 2, "..."), "ab");
    }
}
