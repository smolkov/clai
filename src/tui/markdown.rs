//! Markdown as lines for the screen: headings, emphasis, lists, inline code,
//! code blocks, quotes. No syntax highlighting yet. An unfinished block, while
//! the text streams, is shown as if it were closed.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use ratatui::style::{Modifier, Style};
use ratatui::text::Line;

use unicode_width::UnicodeWidthStr;

use super::style::Look;
use super::wrap::{Piece, Source, wrap_rows};

/// `text` as rows of at most `width` cells.
pub fn lines(text: &str, width: usize, look: &Look) -> Vec<Line<'static>> {
    rows(text, width, look)
        .into_iter()
        .map(|(line, _)| line)
        .collect()
}

/// As [`lines`], with where each row comes from: quote marks and
/// indentation are not text, list markers are.
pub fn rows(text: &str, width: usize, look: &Look) -> Vec<(Line<'static>, Source)> {
    let mut r = Renderer::new(width, look);
    for event in Parser::new_ext(text, Options::ENABLE_STRIKETHROUGH) {
        r.event(event);
    }
    r.finish()
}

/// The code blocks of `text`, as their source, in order.
pub fn code_blocks(text: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut open: Option<String> = None;
    for event in Parser::new_ext(text, Options::ENABLE_STRIKETHROUGH) {
        match event {
            Event::Start(Tag::CodeBlock(_)) => open = Some(String::new()),
            Event::Text(text) => {
                if let Some(block) = &mut open {
                    block.push_str(&text);
                }
            }
            Event::End(TagEnd::CodeBlock) => blocks.extend(open.take()),
            _ => {}
        }
    }
    // An unfinished block, while the text streams.
    blocks.extend(open);
    blocks
}

struct Renderer<'a> {
    width: usize,
    look: &'a Look,
    out: Vec<(Line<'static>, Source)>,
    /// The inline text of the block being read.
    pieces: Vec<Piece>,
    /// The inline style: base plus emphasis.
    modifiers: Vec<Modifier>,
    base: Style,
    /// Open lists: the next number, or `None` for bullets.
    lists: Vec<Option<u64>>,
    /// The marker for the first row of the current list item.
    marker: Option<String>,
    quotes: usize,
    in_code: bool,
    /// Targets of open links, shown after their text.
    links: Vec<String>,
}

impl<'a> Renderer<'a> {
    fn new(width: usize, look: &'a Look) -> Renderer<'a> {
        Renderer {
            width,
            look,
            out: Vec::new(),
            pieces: Vec::new(),
            modifiers: Vec::new(),
            base: look.styles.text,
            lists: Vec::new(),
            marker: None,
            quotes: 0,
            in_code: false,
            links: Vec::new(),
        }
    }

    fn style(&self) -> Style {
        self.modifiers
            .iter()
            .fold(self.base, |style, m| style.add_modifier(*m))
    }

    fn push(&mut self, text: &str) {
        self.pieces.push((text.to_owned(), self.style()));
    }

    /// The prefix of quotes and list indentation: for the rows after the
    /// first, and for the first row without a marker.
    fn indent(&self) -> String {
        let mut out = String::new();
        for _ in 0..self.quotes {
            out.push_str(self.look.symbols.quote);
            out.push(' ');
        }
        for _ in 1..self.lists.len() {
            out.push_str("  ");
        }
        out
    }

    /// Ends the block being read.
    fn flush(&mut self) {
        let marker = self.marker.take();
        if self.pieces.is_empty() {
            if marker.is_none() {
                return;
            }
            self.pieces.push((String::new(), self.base));
        }
        let dim = self.look.styles.dim;
        let indent = self.indent();
        let indent_width = indent.width();
        let (first, rest) = match marker {
            Some(marker) => {
                let rest = " ".repeat(marker.chars().count());
                (
                    (format!("{indent}{marker}"), dim),
                    (format!("{indent}{rest}"), dim),
                )
            }
            None if self.lists.is_empty() => ((indent.clone(), dim), (indent, dim)),
            None => {
                // A second paragraph in a list item lines up with its text.
                let text = format!("{indent}  ");
                ((text.clone(), dim), (text, dim))
            }
        };
        let pieces = std::mem::take(&mut self.pieces);
        let mut rows = wrap_rows(&pieces, self.width, &first, &rest);
        // A list marker is text; the indentation before it is not.
        if let Some((_, source)) = rows.first_mut() {
            source.prefix = indent_width;
        }
        self.out.extend(rows);
    }

    /// A blank row between blocks, unless there is one or nothing yet.
    fn blank(&mut self) {
        if self.out.last().is_some_and(|(l, _)| l.width() > 0) {
            self.out.push((Line::default(), Source::default()));
        }
    }

    fn code(&mut self, text: &str) {
        let style = self.look.styles.code;
        let indent = format!("{}  ", self.indent());
        let prefix = (indent, self.look.styles.dim);
        for line in text.lines() {
            self.out.extend(wrap_rows(
                &[(line.to_owned(), style)],
                self.width,
                &prefix,
                &prefix,
            ));
        }
    }

    fn event(&mut self, event: Event<'_>) {
        match event {
            Event::Start(tag) => self.start(&tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) if self.in_code => self.code(&text),
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => self.push(&text),
            Event::Code(text) => {
                let style = self.look.styles.code;
                self.pieces.push((text.to_string(), style));
            }
            Event::SoftBreak => self.push(" "),
            Event::HardBreak => self.flush(),
            Event::Rule => {
                self.flush();
                self.blank();
                let rule = self.look.symbols.rule.repeat(self.width);
                self.out
                    .push((Line::styled(rule, self.look.styles.dim), Source::skipped()));
                self.blank();
            }
            _ => {}
        }
    }

    fn start(&mut self, tag: &Tag<'_>) {
        match tag {
            Tag::Heading { level, .. } => {
                self.flush();
                self.blank();
                self.base = self.look.styles.heading;
                if *level >= HeadingLevel::H3 {
                    self.base = self.base.remove_modifier(Modifier::BOLD);
                }
            }
            Tag::BlockQuote(_) => {
                self.flush();
                self.quotes += 1;
            }
            Tag::CodeBlock(_) => {
                self.flush();
                self.in_code = true;
            }
            Tag::List(start) => {
                self.flush();
                self.lists.push(*start);
            }
            Tag::Item => {
                self.flush();
                let next = self.lists.last_mut().and_then(Option::as_mut);
                let marker = match next {
                    Some(number) => {
                        let marker = format!("{number}. ");
                        *number += 1;
                        marker
                    }
                    None => format!("{} ", self.look.symbols.bullet),
                };
                self.marker = Some(marker);
            }
            Tag::Emphasis => self.modifiers.push(Modifier::ITALIC),
            Tag::Strong => self.modifiers.push(Modifier::BOLD),
            Tag::Strikethrough => self.modifiers.push(Modifier::CROSSED_OUT),
            Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                self.links.push(dest_url.to_string());
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Heading(_) => {
                self.flush();
                self.base = self.look.styles.text;
                self.blank();
            }
            TagEnd::Paragraph => {
                self.flush();
                self.blank();
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quotes = self.quotes.saturating_sub(1);
                self.blank();
            }
            TagEnd::CodeBlock => {
                self.in_code = false;
                self.blank();
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
                if self.lists.is_empty() {
                    self.blank();
                }
            }
            TagEnd::Item => self.flush(),
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.modifiers.pop();
            }
            TagEnd::Link | TagEnd::Image => {
                if let Some(url) = self.links.pop() {
                    let shown = self.pieces.last().is_some_and(|(text, _)| *text == url);
                    if !url.is_empty() && !shown {
                        let dim = self.look.styles.dim;
                        self.pieces.push((format!(" ({url})"), dim));
                    }
                }
            }
            _ => {}
        }
    }

    fn finish(mut self) -> Vec<(Line<'static>, Source)> {
        self.flush();
        while self.out.last().is_some_and(|(l, _)| l.width() == 0) {
            self.out.pop();
        }
        self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    fn look() -> Look {
        Look::new(true, true)
    }

    fn text(md: &str, width: usize) -> Vec<String> {
        lines(md, width, &look())
            .iter()
            .map(ToString::to_string)
            .collect()
    }

    /// The style of the span with exactly this text.
    fn style_of(md: &str, wanted: &str) -> Style {
        lines(md, 80, &look())
            .iter()
            .flat_map(|l| l.spans.iter())
            .find(|s| s.content == wanted)
            .map_or_else(|| panic!("no span {wanted:?}"), |s| s.style)
    }

    #[test]
    fn headings_are_styled_and_separated() {
        assert_eq!(text("# Title\nbody", 80), ["Title", "", "body"]);
        assert_eq!(style_of("# Title", "Title"), look().styles.heading);
    }

    #[test]
    fn emphasis() {
        assert!(
            style_of("a *b* c", "b")
                .add_modifier
                .contains(Modifier::ITALIC)
        );
        assert!(
            style_of("a **b** c", "b")
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert!(
            style_of("***b***", "b")
                .add_modifier
                .contains(Modifier::BOLD | Modifier::ITALIC)
        );
    }

    #[test]
    fn inline_code() {
        assert_eq!(text("use `parse` here", 80), ["use parse here"]);
        assert_eq!(
            style_of("use `parse` here", "parse").fg,
            Some(Color::Yellow)
        );
    }

    #[test]
    fn lists() {
        assert_eq!(text("- a\n- b", 80), ["• a", "• b"]);
        assert_eq!(text("3. a\n4. b", 80), ["3. a", "4. b"]);
        assert_eq!(text("- a\n  - b\n- c", 80), ["• a", "  • b", "• c"]);
    }

    #[test]
    fn list_items_wrap_under_their_text() {
        assert_eq!(text("- aaa bbb", 6), ["• aaa ", "  bbb"]);
    }

    #[test]
    fn paragraphs_are_separated() {
        assert_eq!(text("a\nb\n\nc", 80), ["a b", "", "c"]);
    }

    #[test]
    fn code_blocks_keep_their_lines() {
        assert_eq!(
            text("```rust\nfn a() {\n    b\n}\n```\nafter", 80),
            ["  fn a() {", "      b", "  }", "", "after"]
        );
        assert_eq!(style_of("```\nx\n```", "x").fg, Some(Color::Yellow));
    }

    #[test]
    fn unfinished_blocks_while_streaming() {
        assert_eq!(text("```\nfn a", 80), ["  fn a"]);
        assert_eq!(text("a **b", 80), ["a **b"]);
        assert_eq!(text("- a\n- ", 80), ["• a", "• "]);
        assert_eq!(text("# ", 80), Vec::<String>::new());
    }

    #[test]
    fn quotes_links_and_rules() {
        assert_eq!(text("> a", 80), ["│ a"]);
        assert_eq!(text("[a](http://b)", 80), ["a (http://b)"]);
        assert_eq!(text("<http://b>", 80), ["http://b"]);
        assert_eq!(text("a\n\n---\n\nb", 3), ["a", "", "───", "", "b"]);
    }

    fn copied(md: &str, width: usize) -> String {
        let rows = rows(md, width, &look());
        crate::tui::wrap::copy_text(rows.iter().map(|(l, s)| Some((l, s))), 0, usize::MAX)
    }

    #[test]
    fn copies_as_read() {
        assert_eq!(copied("> quoted text here", 8), "quoted text here");
        assert_eq!(copied("- aaa bbb\n- c", 6), "• aaa bbb\n• c");
        assert_eq!(
            copied("a\n\n```\nfn a() {\n    b\n}\n```", 80),
            "a\n\nfn a() {\n    b\n}"
        );
        assert_eq!(copied("a\n\n---\n\nb", 3), "a\n\n\nb");
    }

    #[test]
    fn code_blocks_as_source() {
        assert_eq!(
            code_blocks("x\n```rust\nfn a() {}\n```\n\n    b\n\n```\nopen"),
            ["fn a() {}\n", "b\n", "open"]
        );
    }

    #[test]
    fn no_more_than_the_width() {
        for line in lines("# long heading text\n\n- item with long text", 6, &look()) {
            assert!(line.width() <= 6, "{line}");
        }
    }
}
