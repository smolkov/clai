//! Wrapping styled text to a width in terminal cells. Every produced line is
//! one screen row, so that the heights of entries are exact.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// A piece of text in one style.
pub type Piece = (String, Style);

const TAB: &str = "    ";

/// Wraps `pieces` to `width` cells, at spaces where it can. The first row
/// starts with `first`, the others with `rest`. No text gives one row with
/// only the prefix. Tabs become spaces; other control characters (such as
/// the escape sequences of a command's output) are dropped.
pub fn wrap(pieces: &[Piece], width: usize, first: &Piece, rest: &Piece) -> Vec<Line<'static>> {
    let mut rows: Vec<Vec<(char, Style)>> = Vec::new();
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
            used = row.iter().map(|(c, _)| c.width().unwrap_or(0)).sum();
            after_space = None;
        }
        // A space that would start a wrapped row is dropped.
        if c == ' ' && row.is_empty() && !rows.is_empty() {
            continue;
        }
        row.push((c, style));
        used += w;
        if c == ' ' {
            after_space = Some(row.len());
        }
    }
    rows.push(row);
    rows.into_iter()
        .enumerate()
        .map(|(i, row)| line(if i == 0 { first } else { rest }, &row))
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

    #[test]
    fn fit_cuts_by_width() {
        assert_eq!(fit("abc", 3, "…"), "abc");
        assert_eq!(fit("abcd", 3, "…"), "ab…");
        assert_eq!(fit("日本語", 5, "…"), "日本…");
        assert_eq!(fit("abcd", 2, "..."), "ab");
    }
}
