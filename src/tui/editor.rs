//! The input editor: lines of text and a cursor, counted in characters. An
//! image is one character of its own (RFC 0034), shown as `[Image #N]`, so
//! the cursor passes it and `Backspace` deletes it as one.

use unicode_width::UnicodeWidthChar;

use crate::image::{Image, MAX_PER_MESSAGE, Numbered, placeholder};

/// Images are characters of the Supplementary Private Use Area-A, which no
/// one types: `MARK + N` stands for image `N`.
const MARK: u32 = 0xF_0000;

fn mark(number: u32) -> Option<char> {
    char::from_u32(MARK + number)
}

/// The image number of a character, if it is one.
fn image_of(c: char) -> Option<u32> {
    u32::from(c)
        .checked_sub(MARK)
        .filter(|n| (1..0xFFFE).contains(n))
}

/// What a character shows: an image's placeholder, or itself.
fn shown(c: char) -> (String, usize) {
    match image_of(c) {
        Some(n) => {
            let text = placeholder(n);
            let width = text.len();
            (text, width)
        }
        None => (c.to_string(), c.width().unwrap_or(0)),
    }
}

/// A placeholder on the screen: its row, first cell and width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Marked {
    pub row: usize,
    pub cell: usize,
    pub width: usize,
}

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
    /// The images pasted, by number; those whose character is gone from
    /// the text are not sent.
    images: Vec<Numbered>,
    /// The number of the last image; numbers are not reused until cleared.
    last_image: u32,
}

impl Default for Editor {
    fn default() -> Self {
        Editor {
            lines: vec![String::new()],
            row: 0,
            col: 0,
            browsing: None,
            draft: String::new(),
            images: Vec::new(),
            last_image: 0,
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
    /// The text with each image as its placeholder `[Image #N]`.
    pub fn text(&self) -> String {
        self.raw()
            .chars()
            .map(|c| image_of(c).map_or_else(|| c.to_string(), placeholder))
            .collect()
    }

    /// The text with images as their characters.
    fn raw(&self) -> String {
        self.lines.join("\n")
    }

    /// The numbers of the images in the text, in order.
    pub fn numbers(&self) -> Vec<u32> {
        self.lines
            .iter()
            .flat_map(|l| l.chars())
            .filter_map(image_of)
            .collect()
    }

    /// The images in the text, in order.
    pub fn images(&self) -> Vec<Numbered> {
        self.numbers()
            .iter()
            .filter_map(|n| self.images.iter().find(|i| i.number == *n))
            .cloned()
            .collect()
    }

    /// Puts an image at the cursor; its number, or why not.
    pub fn insert_image(&mut self, image: Image) -> Result<u32, String> {
        if self.numbers().len() >= MAX_PER_MESSAGE {
            return Err(format!("a message takes at most {MAX_PER_MESSAGE} images"));
        }
        let number = self.last_image + 1;
        let c = mark(number).ok_or("too many images")?;
        self.browsing = None;
        self.last_image = number;
        self.images.push(Numbered { number, image });
        self.insert(c);
        Ok(number)
    }

    /// Turns the placeholders of `images` in the text back into images,
    /// for a message from the history whose images are still known. Not
    /// done when a number is taken by another image.
    pub fn adopt_images(&mut self, images: &[Numbered]) {
        let taken = images.iter().any(|new| {
            self.images
                .iter()
                .any(|old| old.number == new.number && old.image.hash != new.image.hash)
        });
        if taken {
            return;
        }
        let mut raw = self.raw();
        for numbered in images {
            if let Some(c) = mark(numbered.number) {
                raw = raw.replace(&placeholder(numbered.number), &c.to_string());
            }
            if !self.images.iter().any(|i| i.number == numbered.number) {
                self.images.push(numbered.clone());
            }
            self.last_image = self.last_image.max(numbered.number);
        }
        let browsing = self.browsing;
        let draft = std::mem::take(&mut self.draft);
        self.set_text(&raw);
        self.browsing = browsing;
        self.draft = draft;
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
                // Images stay in the text put aside.
                self.draft = self.raw();
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
        let (rows, cursor, _) = self.layout(width);
        (rows, cursor)
    }

    /// Where the placeholders of images are in [`Editor::rows`].
    pub fn marked(&self, width: usize) -> Vec<Marked> {
        self.layout(width).2
    }

    fn layout(&self, width: usize) -> (Vec<String>, (usize, usize), Vec<Marked>) {
        let width = width.max(1);
        let mut rows = Vec::new();
        let mut cursor = (0, 0);
        let mut marked = Vec::new();
        for (r, line) in self.lines.iter().enumerate() {
            let mut row = String::new();
            let mut used = 0;
            for (c, ch) in line.chars().enumerate() {
                let (text, w) = shown(ch);
                if used + w > width && used > 0 {
                    rows.push(std::mem::take(&mut row));
                    used = 0;
                }
                if r == self.row && c == self.col {
                    cursor = (rows.len(), used);
                }
                if image_of(ch).is_some() {
                    marked.push(Marked {
                        row: rows.len(),
                        cell: used,
                        width: w,
                    });
                }
                row.push_str(&text);
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
        (rows, cursor, marked)
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

    fn png(width: u32) -> Image {
        Image::from_bytes(crate::image::fixtures::png(width, 1)).unwrap()
    }

    #[test]
    fn images_are_one_character_shown_as_their_placeholder() {
        let mut e = typed("see ");
        assert_eq!(e.insert_image(png(1)), Ok(1));
        e.paste(" and ");
        assert_eq!(e.insert_image(png(2)), Ok(2));
        assert_eq!(e.text(), "see [Image #1] and [Image #2]");
        assert_eq!(e.numbers(), [1, 2]);
        // Backspace removes the whole image; its number is not reused.
        e.edit(Edit::Backspace);
        assert_eq!(e.text(), "see [Image #1] and ");
        assert_eq!(e.insert_image(png(3)), Ok(3));
        assert_eq!(
            e.images().iter().map(|i| i.image.width).collect::<Vec<_>>(),
            [1, 3]
        );
        // The cursor passes it in one step.
        e.edit(Edit::Home);
        for _ in 0..4 {
            e.edit(Edit::Right);
        }
        e.edit(Edit::Delete);
        assert_eq!(e.text(), "see  and [Image #3]");
        assert_eq!(e.numbers(), [3]);
        e.clear();
        assert_eq!(e.insert_image(png(1)), Ok(1));
    }

    #[test]
    fn at_most_ten_images() {
        let mut e = Editor::default();
        for _ in 0..10 {
            e.insert_image(png(1)).unwrap();
        }
        assert!(e.insert_image(png(1)).is_err());
        e.edit(Edit::Backspace);
        assert!(e.insert_image(png(1)).is_ok());
    }

    #[test]
    fn images_wrap_whole_and_are_marked() {
        let mut e = typed("abcdef");
        e.insert_image(png(1)).unwrap();
        e.edit(Edit::Left);
        e.edit(Edit::Right);
        let (rows, cursor) = e.rows(12);
        assert_eq!(rows, ["abcdef", "[Image #1]"]);
        assert_eq!(cursor, (1, 10));
        assert_eq!(
            e.marked(12),
            [Marked {
                row: 1,
                cell: 0,
                width: 10
            }]
        );
        // Narrower than the placeholder: it takes a row of its own.
        let (rows, _) = e.rows(4);
        assert_eq!(rows, ["abcd", "ef", "[Image #1]", ""]);
    }

    #[test]
    fn images_survive_browsing_and_come_back_from_history() {
        let mut e = typed("draft ");
        e.insert_image(png(1)).unwrap();
        let old = Numbered {
            number: 4,
            image: png(4),
        };
        let history = vec!["old [Image #4]".to_owned()];
        e.browse(&history, Browse::Earlier);
        e.adopt_images(std::slice::from_ref(&old));
        assert_eq!(e.numbers(), [4]);
        assert_eq!(e.text(), "old [Image #4]");
        e.browse(&history, Browse::Later);
        assert_eq!(e.numbers(), [1]);
        assert_eq!(e.text(), "draft [Image #1]");
        // A number taken by another image: the history stays text.
        let mut e = Editor::default();
        e.insert_image(png(9)).unwrap();
        e.set_text("old [Image #1]");
        e.adopt_images(&[Numbered {
            number: 1,
            image: png(4),
        }]);
        assert!(e.numbers().is_empty());
    }
}
