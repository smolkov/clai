//! The details view (see `spec/tui.md`, "Details view"): the whole of a
//! tool call, or the diffs of the last turn, over the conversation, to read,
//! search and copy. `App` keeps it; `view::details` draws it.

use ratatui::text::Line;

use super::conversation::Drawn;
use super::wrap::{Piece, Source, copy_text, fit, wrap_rows};

#[derive(Debug, Clone, PartialEq)]
pub struct Details {
    /// `edit_file src/parse.rs ─ ✓ +1 −1`.
    pub title: String,
    /// The text: the pieces of each line.
    pub lines: Vec<Vec<Piece>>,
    /// Said under the text: `the output was cut at 1 MiB`.
    pub note: Option<String>,
    /// The first row shown.
    pub top: usize,
    /// Long lines wrap; without, they are cut and `left` scrolls them.
    pub wrap: bool,
    /// Cells scrolled to the right, without wrap.
    pub left: usize,
    /// The search being typed, after `/`.
    pub typing: Option<String>,
    /// The last search.
    pub search: Option<String>,
    /// The line of the match shown.
    pub found: Option<usize>,
    /// What was not found, to say it.
    pub missed: bool,
    /// The rows at `width`, from [`Details::layout`].
    rows: Vec<Drawn>,
    /// The line of each row.
    row_lines: Vec<usize>,
    width: usize,
    laid_out: Option<(usize, bool, usize)>,
    /// Rows the view shows at once, from the last layout.
    pub height: usize,
}

impl Details {
    pub fn new(title: String, lines: Vec<Vec<Piece>>, note: Option<String>) -> Details {
        Details {
            title,
            lines,
            note,
            top: 0,
            wrap: true,
            left: 0,
            typing: None,
            search: None,
            found: None,
            missed: false,
            rows: Vec::new(),
            row_lines: Vec::new(),
            width: 0,
            laid_out: None,
            height: 1,
        }
    }

    /// The rows for `width` cells, `height` of them shown; built again only
    /// when the width, the wrap or the scroll to the side changed.
    pub fn layout(&mut self, width: usize, height: usize) {
        self.height = height.max(1);
        self.width = width;
        let key = (width, self.wrap, self.left);
        if self.laid_out != Some(key) {
            self.rows.clear();
            self.row_lines.clear();
            let none = (String::new(), ratatui::style::Style::new());
            for (i, pieces) in self.lines.iter().enumerate() {
                if self.wrap {
                    for row in wrap_rows(pieces, width, &none, &none) {
                        self.rows.push(row);
                        self.row_lines.push(i);
                    }
                } else {
                    self.rows
                        .push((shifted(pieces, self.left, width), Source::text(0)));
                    self.row_lines.push(i);
                }
            }
            self.laid_out = Some(key);
        }
        self.top = self.top.min(self.max_top());
    }

    pub fn rows(&self) -> &[Drawn] {
        &self.rows
    }

    /// The line a row shows.
    pub fn line_of(&self, row: usize) -> Option<usize> {
        self.row_lines.get(row).copied()
    }

    pub fn max_top(&self) -> usize {
        self.rows.len().saturating_sub(self.height)
    }

    pub fn scroll(&mut self, by: isize) {
        self.top = self.top.saturating_add_signed(by).min(self.max_top());
    }

    pub fn toggle_wrap(&mut self) {
        self.wrap = !self.wrap;
        self.left = 0;
    }

    pub fn side(&mut self, by: isize) {
        if !self.wrap {
            self.left = self.left.saturating_add_signed(by);
        }
    }

    /// All the text, for `y`.
    pub fn text(&self) -> String {
        let mut out: Vec<String> = self.lines.iter().map(|l| plain(l)).collect();
        while out.last().is_some_and(|l| l.is_empty()) {
            out.pop();
        }
        let mut text = out.join("\n");
        text.push('\n');
        text
    }

    /// The text of rows `from.0..=to.0`, from cell `from.1` to cell `to.1`.
    pub fn copy(&self, from: (usize, usize), to: (usize, usize)) -> String {
        let rows = (from.0..=to.0).map(|row| self.rows.get(row).map(|(l, s)| (l, s)));
        copy_text(rows, from.1, to.1)
    }

    /// Finds the search in the lines after the match shown (or before it,
    /// `back`), wrapping around, and scrolls to it.
    pub fn find(&mut self, back: bool) {
        let Some(query) = self.search.clone().filter(|q| !q.is_empty()) else {
            return;
        };
        let query = query.to_lowercase();
        let count = self.lines.len();
        let start = self
            .found
            .unwrap_or(if back { 0 } else { count.saturating_sub(1) });
        let found = (1..=count).find_map(|step| {
            let i = if back {
                (start + count - step % count) % count.max(1)
            } else {
                (start + step) % count.max(1)
            };
            let line = self.lines.get(i)?;
            plain(line).to_lowercase().contains(&query).then_some(i)
        });
        self.missed = found.is_none();
        if let Some(line) = found {
            self.found = Some(line);
            if let Some(row) = self.row_lines.iter().position(|l| *l == line) {
                self.top = row.min(self.max_top());
            }
        }
    }
}

/// The text of a line's pieces.
fn plain(pieces: &[Piece]) -> String {
    pieces.iter().map(|(t, _)| t.as_str()).collect()
}

/// A line of pieces from cell `left` on, `width` cells of it.
fn shifted(pieces: &[Piece], left: usize, width: usize) -> Line<'static> {
    let mut skip = left;
    let mut room = width;
    let mut spans = Vec::new();
    for (text, style) in pieces {
        let rest: String = text.chars().skip(skip).collect();
        skip = skip.saturating_sub(text.chars().count());
        let shown = fit(&rest, room, "");
        room = room.saturating_sub(unicode_width::UnicodeWidthStr::width(shown.as_str()));
        if !shown.is_empty() {
            spans.push(ratatui::text::Span::styled(shown, *style));
        }
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    fn details(lines: &[&str]) -> Details {
        let lines = lines
            .iter()
            .map(|l| vec![((*l).to_owned(), Style::new())])
            .collect();
        Details::new("t".into(), lines, None)
    }

    #[test]
    fn wraps_and_scrolls_sideways() {
        let mut d = details(&["aaaa bbbb", "c"]);
        d.layout(5, 2);
        assert_eq!(d.rows().len(), 3);
        assert_eq!(d.line_of(1), Some(0));
        assert_eq!(d.max_top(), 1);
        d.toggle_wrap();
        d.layout(5, 2);
        assert_eq!(d.rows().len(), 2);
        d.side(5);
        d.layout(5, 2);
        assert_eq!(d.rows()[0].0.to_string(), "bbbb");
        assert_eq!(d.copy((0, 0), (1, usize::MAX)), "bbbb\n");
    }

    #[test]
    fn search_goes_round() {
        let mut d = details(&["one", "Two", "three", "two again"]);
        d.layout(20, 1);
        d.search = Some("two".into());
        d.find(false);
        assert_eq!((d.found, d.top), (Some(1), 1));
        d.find(false);
        assert_eq!(d.found, Some(3));
        d.find(false);
        assert_eq!(d.found, Some(1));
        d.find(true);
        assert_eq!(d.found, Some(3));
        d.search = Some("zzz".into());
        d.find(false);
        assert!(d.missed);
        assert_eq!(d.found, Some(3));
    }

    #[test]
    fn copies_all() {
        assert_eq!(details(&["a", "b", ""]).text(), "a\nb\n");
    }
}
