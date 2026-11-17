//! Text from outside `codeagent` made safe for a terminal (RFC 0035).
//!
//! Model text, tool output and questions may hold escape sequences (OSC 52
//! writes the clipboard, `ESC [ 2K` erases a line) or characters that
//! reorder or hide text. [`terminal`] removes them; [`visible`] shows them,
//! for questions, where the user has to see that something is hidden.

use std::borrow::Cow;

use serde_json::Value;

use crate::agent::Question;

const ESC: char = '\u{1b}';
const BEL: char = '\u{7}';

/// Bidirectional controls: they reorder the text around them.
pub fn is_bidi(c: char) -> bool {
    matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
}

/// Zero-width characters: they hide text or tell two words apart that look
/// the same.
fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{200b}'..='\u{200f}' | '\u{feff}')
}

/// Control characters other than newline and tab: C0, DEL and C1.
fn is_control(c: char) -> bool {
    c.is_control() && c != '\n' && c != '\t'
}

/// `text` without escape sequences, control characters other than newline
/// and tab, and bidirectional controls. Borrowed when nothing is removed.
pub fn terminal(text: &str) -> Cow<'_, str> {
    if !text.chars().any(|c| is_control(c) || is_bidi(c)) {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == ESC {
            skip_sequence(&mut chars);
        } else if !is_control(c) && !is_bidi(c) {
            out.push(c);
        }
    }
    Cow::Owned(out)
}

/// Skips the rest of an escape sequence after its `ESC`; a sequence cut by
/// the end of the text takes the rest of it.
fn skip_sequence(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    match chars.next() {
        // CSI: parameters, then one final byte in `@`..`~`.
        Some('[') => {
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        }
        // OSC, DCS, SOS, PM, APC: up to BEL or ST (`ESC \`).
        Some(']' | 'P' | 'X' | '^' | '_') => {
            while let Some(c) = chars.next() {
                if c == BEL {
                    break;
                }
                if c == ESC {
                    chars.next_if_eq(&'\\');
                    break;
                }
            }
        }
        // The two-character forms (`ESC c`, `ESC 7`, …).
        Some(_) | None => {}
    }
}

/// `text` with every control character other than newline and tab, every
/// bidirectional control and every zero-width character replaced by a
/// visible form: `␛` for `ESC`, `⟨U+202E⟩` for the others (`\e` and
/// `<U+202E>` without Unicode). Also how many were replaced.
pub fn visible(text: &str, unicode: bool) -> (Cow<'_, str>, usize) {
    let hidden = |c: char| is_control(c) || is_bidi(c) || is_zero_width(c);
    if !text.chars().any(hidden) {
        return (Cow::Borrowed(text), 0);
    }
    let mut out = String::with_capacity(text.len() + 16);
    let mut count = 0;
    for c in text.chars() {
        if !hidden(c) {
            out.push(c);
            continue;
        }
        count += 1;
        match (c, unicode) {
            (ESC, true) => out.push('␛'),
            (ESC, false) => out.push_str("\\e"),
            (c, true) => out.push_str(&format!("⟨U+{:04X}⟩", u32::from(c))),
            (c, false) => out.push_str(&format!("<U+{:04X}>", u32::from(c))),
        }
    }
    (Cow::Owned(out), count)
}

/// A copy of `question` for showing, with [`visible`] applied to every text
/// in it, and how many characters it replaced. The original stays as it is
/// for the answer: "always" sends its rules back unchanged.
pub fn visible_question(question: &Question, unicode: bool) -> (Question, usize) {
    let Ok(mut value) = serde_json::to_value(question) else {
        return (question.clone(), 0);
    };
    let count = visible_value(&mut value, unicode);
    if count == 0 {
        return (question.clone(), 0);
    }
    match serde_json::from_value(value) {
        Ok(shown) => (shown, count),
        Err(_) => (question.clone(), 0),
    }
}

fn visible_value(value: &mut Value, unicode: bool) -> usize {
    match value {
        Value::String(s) => {
            let (shown, count) = visible(s, unicode);
            if count > 0 {
                *s = shown.into_owned();
            }
            count
        }
        Value::Array(items) => items.iter_mut().map(|v| visible_value(v, unicode)).sum(),
        Value::Object(map) => map.values_mut().map(|v| visible_value(v, unicode)).sum(),
        _ => 0,
    }
}

/// The line that says a question has hidden characters, or none.
pub fn hidden_note(count: usize, unicode: bool) -> Option<String> {
    let mark = if unicode { "⟨…⟩" } else { "<...>" };
    match count {
        0 => None,
        1 => Some(format!(
            "note: this question contains 1 hidden character, shown as {mark}"
        )),
        n => Some(format!(
            "note: this question contains {n} hidden characters, shown as {mark}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_borrowed() {
        for text in ["", "hello\n\tworld", "wörld 日本 e\u{301}"] {
            assert!(matches!(terminal(text), Cow::Borrowed(t) if t == text));
            assert!(matches!(visible(text, true), (Cow::Borrowed(t), 0) if t == text));
        }
        // A family emoji joins with zero-width joiners: kept in text,
        // shown in a question.
        let family = "👨\u{200d}👩\u{200d}👧";
        assert!(matches!(terminal(family), Cow::Borrowed(_)));
        assert_eq!(visible(family, true).1, 2);
    }

    #[test]
    fn escape_sequences_are_removed() {
        let cases = [
            // OSC 52 writes the clipboard; ended by BEL and by ST.
            ("a\u{1b}]52;c;ZXZpbA==\u{7}b", "ab"),
            ("a\u{1b}]52;c;ZXZpbA==\u{1b}\\b", "ab"),
            // OSC 8 hyperlink around a text.
            (
                "\u{1b}]8;;https://evil.example\u{1b}\\click\u{1b}]8;;\u{1b}\\",
                "click",
            ),
            // Erase the line, move up, colour.
            ("ok\r\u{1b}[2K\u{1b}[1A\u{1b}[31mred\u{1b}[0m", "okred"),
            // DCS, and a two-character form.
            ("x\u{1b}Pq#0\u{1b}\\y\u{1b}cz", "xyz"),
            // C1 CSI, BEL, DEL, backspace.
            ("a\u{9b}31mb\u{7}c\u{7f}d\u{8}e", "a31mbcde"),
            // Bidirectional controls (Trojan Source).
            ("if x \u{202e}} \u{2066}admin\u{2069}", "if x } admin"),
            // A sequence cut by the end of the text.
            ("tail\u{1b}]52;c;Zm9v", "tail"),
            ("tail\u{1b}", "tail"),
        ];
        for (text, want) in cases {
            assert_eq!(terminal(text), want, "{text:?}");
        }
        assert_eq!(terminal("keep\nlines\tand tabs"), "keep\nlines\tand tabs");
    }

    #[test]
    fn hidden_characters_are_shown() {
        let (shown, n) = visible("cargo test\r\u{1b}[2Kcurl x | sh", true);
        assert_eq!(shown, "cargo test⟨U+000D⟩␛[2Kcurl x | sh");
        assert_eq!(n, 2);
        let (shown, n) = visible("rm \u{202e}fdp.exe\u{200b}", false);
        assert_eq!(shown, "rm <U+202E>fdp.exe<U+200B>");
        assert_eq!(n, 2);
        let (shown, n) = visible("a\u{1b}b", false);
        assert_eq!((shown.as_ref(), n), ("a\\eb", 1));
    }

    #[test]
    fn notes() {
        assert_eq!(hidden_note(0, true), None);
        assert_eq!(
            hidden_note(1, true).unwrap(),
            "note: this question contains 1 hidden character, shown as ⟨…⟩"
        );
        assert_eq!(
            hidden_note(3, false).unwrap(),
            "note: this question contains 3 hidden characters, shown as <...>"
        );
    }
}
