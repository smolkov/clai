//! The input editor: lines of text and a cursor, counted in characters.

use unicode_width::UnicodeWidthChar;

/// An editing key (see `spec/tui.md`, "Input editor").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edit {
    Char(char),
    Newline,
    Left,
    Right,
    Home,
    End,
    Backspace,
    Delete,
    DeleteWord,
    DeleteToStart,
}

/// Direction in the message history.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Browse {
    Earlier,
    Later,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Editor {
    lines: Vec<String>,
    row: usize,
    /// In characters.
    col: usize,
    /// Which history message is shown; `None` while editing.
    browsing: Option<usize>,
    /// The text put aside while browsing.
    draft: String,
}

impl Default for Editor {
    fn default() -> Self {
        Editor {
            lines: vec![String::new()],
            row: 0,
            col: 0,
            browsing: None,
            draft: String::new(),
        }
    }
}

/// The byte offset of character `col`.
fn offset(line: &str, col: usize) -> usize {
    line.char_indices().nth(col).map_or(line.len(), |(i, _)| i)
}

fn len(line: &str) -> usize {
    line.chars().count()
}

impl Editor {
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    pub fn is_empty(&self) -> bool {
        self.lines.iter().all(String::is_empty)
    }

    pub fn clear(&mut self) {
        *self = Editor::default();
    }

    #[cfg(test)]
    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.col)
    }

    /// Replaces the text; the cursor goes to the end.
    pub fn set_text(&mut self, text: &str) {
        self.lines = text.split('\n').map(str::to_owned).collect();
        self.row = self.lines.len().saturating_sub(1);
        self.col = self.line().map_or(0, len);
    }

    fn line(&self) -> Option<&str> {
        self.lines.get(self.row).map(String::as_str)
    }

    fn insert(&mut self, c: char) {
        let col = self.col;
        if let Some(line) = self.lines.get_mut(self.row) {
            line.insert(offset(line, col), c);
            self.col += 1;
        }
    }

    fn newline(&mut self) {
        let col = self.col;
        if let Some(line) = self.lines.get_mut(self.row) {
            let tail = line.split_off(offset(line, col));
            self.lines.insert(self.row + 1, tail);
            self.row += 1;
            self.col = 0;
        }
    }

    /// Inserts pasted text as it is; newlines of any kind start a new line.
    pub fn paste(&mut self, text: &str) {
        self.browsing = None;
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        for c in text.chars() {
            match c {
                '\n' => self.newline(),
                c => self.insert(c),
            }
        }
    }

    pub fn edit(&mut self, edit: Edit) {
        self.browsing = None;
        match edit {
            Edit::Char(c) => self.insert(c),
            Edit::Newline => self.newline(),
            Edit::Left => self.left(),
            Edit::Right => self.right(),
            Edit::Home => self.col = 0,
            Edit::End => self.col = self.line().map_or(0, len),
            Edit::Backspace => self.backspace(),
            Edit::Delete => self.delete(),
            Edit::DeleteWord => self.delete_word(),
            Edit::DeleteToStart => self.delete_to_start(),
        }
    }

    fn left(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.col = self.line().map_or(0, len);
        }
    }

    fn right(&mut self) {
        if self.col < self.line().map_or(0, len) {
            self.col += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.col = 0;
        }
    }

    fn backspace(&mut self) {
        let col = self.col;
        if col > 0 {
            if let Some(line) = self.lines.get_mut(self.row) {
                line.remove(offset(line, col - 1));
                self.col -= 1;
            }
        } else if self.row > 0 {
            let tail = self.lines.remove(self.row);
            self.row -= 1;
            if let Some(line) = self.lines.get_mut(self.row) {
                self.col = len(line);
                line.push_str(&tail);
            }
        }
    }

    fn delete(&mut self) {
        let col = self.col;
        let row = self.row;
        let at_end = self.line().is_none_or(|l| col >= len(l));
        if !at_end {
            if let Some(line) = self.lines.get_mut(row) {
                line.remove(offset(line, col));
            }
        } else if row + 1 < self.lines.len() {
            let tail = self.lines.remove(row + 1);
            if let Some(line) = self.lines.get_mut(row) {
                line.push_str(&tail);
            }
        }
    }

    /// Deletes the word before the cursor and the spaces after it; at the
    /// start of a line, joins it to the line before.
    fn delete_word(&mut self) {
        if self.col == 0 {
            self.backspace();
            return;
        }
        let col = self.col;
        if let Some(line) = self.lines.get_mut(self.row) {
            let chars: Vec<char> = line.chars().take(col).collect();
            let word_end = chars.iter().rposition(|c| !c.is_whitespace());
            let start = word_end
                .and_then(|end| {
                    chars
                        .iter()
                        .take(end)
                        .rposition(|c| c.is_whitespace())
                        .map(|i| i + 1)
                })
                .unwrap_or(0);
            let (from, to) = (offset(line, start), offset(line, col));
            line.replace_range(from..to, "");
            self.col = start;
        }
    }

    fn delete_to_start(&mut self) {
        let col = self.col;
        if let Some(line) = self.lines.get_mut(self.row) {
            let to = offset(line, col);
            line.replace_range(..to, "");
            self.col = 0;
        }
    }

    /// `Up`: one line up, or `false` on the first line.
    pub fn up(&mut self) -> bool {
        if self.row == 0 {
            return false;
        }
        self.row -= 1;
        self.col = self.col.min(self.line().map_or(0, len));
        true
    }

    /// `Down`: one line down, or `false` on the last line.
    pub fn down(&mut self) -> bool {
        if self.row + 1 >= self.lines.len() {
            return false;
        }
        self.row += 1;
        self.col = self.col.min(self.line().map_or(0, len));
        true
    }

    /// Shows an earlier or later message of `history` (oldest first). Past
    /// the newest, the text put aside comes back.
    pub fn browse(&mut self, history: &[String], direction: Browse) {
        let next = match (self.browsing, direction) {
            (None, Browse::Later) => return,
            (None, Browse::Earlier) => {
                let Some(last) = history.len().checked_sub(1) else {
                    return;
                };
                self.draft = self.text();
                Some(last)
            }
            (Some(at), Browse::Earlier) => Some(at.saturating_sub(1)),
            (Some(at), Browse::Later) if at + 1 < history.len() => Some(at + 1),
            (Some(_), Browse::Later) => None,
        };
        self.browsing = next;
        match next.and_then(|at| history.get(at)) {
            Some(message) => self.set_text(&message.clone()),
            None => {
                let draft = std::mem::take(&mut self.draft);
                self.set_text(&draft);
                self.browsing = None;
            }
        }
    }

    /// The text as screen rows of `width` cells and the cursor's row and
    /// cell in them.
    pub fn rows(&self, width: usize) -> (Vec<String>, (usize, usize)) {
        let width = width.max(1);
        let mut rows = Vec::new();
        let mut cursor = (0, 0);
        for (r, line) in self.lines.iter().enumerate() {
            let mut row = String::new();
            let mut used = 0;
            for (c, ch) in line.chars().enumerate() {
                let w = ch.width().unwrap_or(0);
                if used + w > width {
                    rows.push(std::mem::take(&mut row));
                    used = 0;
                }
                if r == self.row && c == self.col {
                    cursor = (rows.len(), used);
                }
                row.push(ch);
                used += w;
            }
            if r == self.row && self.col >= len(line) {
                // The cursor after the last character may need its own row.
                if used >= width {
                    rows.push(std::mem::take(&mut row));
                    used = 0;
                }
                cursor = (rows.len(), used);
            }
            rows.push(row);
        }
        (rows, cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> Editor {
        let mut e = Editor::default();
        e.paste(text);
        e
    }

    #[test]
    fn typing_and_newlines() {
        let mut e = Editor::default();
        assert!(e.is_empty());
        e.edit(Edit::Char('a'));
        e.edit(Edit::Newline);
        e.edit(Edit::Char('b'));
        assert_eq!(e.text(), "a\nb");
        assert_eq!(e.cursor(), (1, 1));
        assert!(!e.is_empty());
    }

    #[test]
    fn multi_byte_text() {
        let mut e = typed("äöü");
        e.edit(Edit::Left);
        e.edit(Edit::Char('日'));
        assert_eq!(e.text(), "äö日ü");
        e.edit(Edit::Backspace);
        e.edit(Edit::Backspace);
        assert_eq!(e.text(), "äü");
        e.edit(Edit::Delete);
        assert_eq!(e.text(), "ä");
    }

    #[test]
    fn arrows_cross_lines() {
        let mut e = typed("ab\nc");
        e.edit(Edit::Home);
        e.edit(Edit::Left);
        assert_eq!(e.cursor(), (0, 2));
        e.edit(Edit::Right);
        assert_eq!(e.cursor(), (1, 0));
        e.edit(Edit::End);
        assert_eq!(e.cursor(), (1, 1));
        e.edit(Edit::Right);
        assert_eq!(e.cursor(), (1, 1));
    }

    #[test]
    fn up_and_down_stop_at_the_ends() {
        let mut e = typed("abc\nd");
        assert!(!e.down());
        assert!(e.up());
        assert_eq!(e.cursor(), (0, 1));
        assert!(!e.up());
        e.edit(Edit::End);
        assert!(e.down());
        assert_eq!(e.cursor(), (1, 1));
    }

    #[test]
    fn backspace_and_delete_join_lines() {
        let mut e = typed("ab\ncd");
        e.edit(Edit::Home);
        e.edit(Edit::Backspace);
        assert_eq!(e.text(), "abcd");
        assert_eq!(e.cursor(), (0, 2));
        let mut e = typed("ab\ncd");
        assert!(e.up());
        e.edit(Edit::End);
        e.edit(Edit::Delete);
        assert_eq!(e.text(), "abcd");
    }

    #[test]
    fn delete_word() {
        let mut e = typed("one two  ");
        e.edit(Edit::DeleteWord);
        assert_eq!(e.text(), "one ");
        e.edit(Edit::DeleteWord);
        assert_eq!(e.text(), "");
        let mut e = typed("a\nb");
        e.edit(Edit::Home);
        e.edit(Edit::DeleteWord);
        assert_eq!(e.text(), "ab");
        let mut e = typed("äö üß");
        e.edit(Edit::DeleteWord);
        assert_eq!(e.text(), "äö ");
    }

    #[test]
    fn delete_to_line_start() {
        let mut e = typed("x\none two");
        e.edit(Edit::Left);
        e.edit(Edit::DeleteToStart);
        assert_eq!(e.text(), "x\no");
        assert_eq!(e.cursor(), (1, 0));
    }

    #[test]
    fn paste_with_newlines() {
        let mut e = typed("a");
        e.paste("b\r\nc\rd\ne");
        assert_eq!(e.text(), "ab\nc\nd\ne");
        assert_eq!(e.cursor(), (3, 1));
    }

    #[test]
    fn history_browsing_keeps_the_draft() {
        let history = vec!["old".to_owned(), "new\nline".to_owned()];
        let mut e = typed("draft");
        e.browse(&history, Browse::Later);
        assert_eq!(e.text(), "draft");
        e.browse(&history, Browse::Earlier);
        assert_eq!(e.text(), "new\nline");
        e.browse(&history, Browse::Earlier);
        assert_eq!(e.text(), "old");
        e.browse(&history, Browse::Earlier);
        assert_eq!(e.text(), "old");
        e.browse(&history, Browse::Later);
        e.browse(&history, Browse::Later);
        assert_eq!(e.text(), "draft");
    }

    #[test]
    fn empty_history_changes_nothing() {
        let mut e = typed("draft");
        e.browse(&[], Browse::Earlier);
        assert_eq!(e.text(), "draft");
    }

    #[test]
    fn rows_wrap_and_place_the_cursor() {
        let e = typed("abcde");
        assert_eq!(e.rows(3), (vec!["abc".to_owned(), "de".to_owned()], (1, 2)));
        // A full last row: the cursor gets its own.
        let e = typed("abc");
        assert_eq!(e.rows(3), (vec!["abc".to_owned(), String::new()], (1, 0)));
        let mut e = typed("日本");
        e.edit(Edit::Left);
        assert_eq!(e.rows(10), (vec!["日本".to_owned()], (0, 2)));
    }
}
