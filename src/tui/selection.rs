//! Text selected with the mouse (see `spec/tui.md`, "Copy"): in the
//! conversation or in the details view, by row and cell. Rows count from the
//! start of what is selected in, not from the screen, so that scrolling
//! keeps the selection on its text.

use ratatui::text::Line;
use unicode_width::UnicodeWidthChar;

/// Where a selection lies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Surface {
    Conversation,
    Details,
}

/// A row and a cell.
pub type Point = (usize, usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub surface: Surface,
    /// Where the button went down.
    pub anchor: Point,
    /// Where the mouse is.
    pub head: Point,
    /// The mouse moved with the button down: a release copies.
    pub dragged: bool,
    /// The conversation followed its end before: it does again once the
    /// selection is cleared.
    pub followed: bool,
}

impl Selection {
    pub fn new(surface: Surface, at: Point) -> Selection {
        Selection {
            surface,
            anchor: at,
            head: at,
            dragged: false,
            followed: false,
        }
    }

    /// The first point and the point after the last cell, in order.
    pub fn range(&self) -> (Point, Point) {
        let (from, to) = if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        };
        (from, (to.0, to.1.saturating_add(1)))
    }

    /// Whether the cell is selected.
    pub fn contains(&self, row: usize, cell: usize) -> bool {
        let ((from_row, from_cell), (to_row, to_cell)) = self.range();
        if row < from_row || row > to_row {
            return false;
        }
        let after_start = row > from_row || cell >= from_cell;
        let before_end = row < to_row || cell < to_cell;
        after_start && before_end
    }
}

/// The cells of the word at `cell` in a row: letters, digits and the
/// characters of paths and names.
pub fn word(line: &Line<'_>, cell: usize) -> Option<(usize, usize)> {
    let mut cells: Vec<(usize, char)> = Vec::new();
    let mut at = 0;
    for span in &line.spans {
        for c in span.content.chars() {
            cells.push((at, c));
            at += c.width().unwrap_or(0);
        }
    }
    if cell >= at {
        return None;
    }
    let in_word = |c: char| c.is_alphanumeric() || "_-./:~@#%+".contains(c);
    let index = cells
        .iter()
        .rposition(|(start, _)| *start <= cell)
        .filter(|i| cells.get(*i).is_some_and(|(_, c)| in_word(*c)))?;
    let mut first = index;
    while first > 0 && cells.get(first - 1).is_some_and(|(_, c)| in_word(*c)) {
        first -= 1;
    }
    let mut last = index;
    while cells.get(last + 1).is_some_and(|(_, c)| in_word(*c)) {
        last += 1;
    }
    // Punctuation after a word is not part of it.
    while last > index && cells.get(last).is_some_and(|(_, c)| ".:".contains(*c)) {
        last -= 1;
    }
    let start = cells.get(first).map(|(s, _)| *s)?;
    let end = cells.get(last).map(|(s, c)| s + c.width().unwrap_or(0))?;
    Some((start, end.saturating_sub(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_range_in_either_direction() {
        let mut s = Selection::new(Surface::Conversation, (3, 5));
        s.head = (1, 2);
        assert_eq!(s.range(), ((1, 2), (3, 6)));
        assert!(s.contains(1, 2));
        assert!(!s.contains(1, 1));
        assert!(s.contains(2, 0));
        assert!(s.contains(3, 5));
        assert!(!s.contains(3, 6));
        assert!(!s.contains(4, 0));
    }

    #[test]
    fn words() {
        let line = Line::raw("see src/parse.rs: here");
        assert_eq!(word(&line, 6), Some((4, 15)));
        assert_eq!(word(&line, 0), Some((0, 2)));
        assert_eq!(word(&line, 3), None);
        assert_eq!(word(&line, 40), None);
    }
}
