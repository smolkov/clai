//! Tool results and their size limits (see `spec/tools.md`, "Result size").

use serde::Serialize;

/// Characters of a result sent to the model.
pub const MODEL_CHARS: usize = 30_000;
const HEAD_CHARS: usize = 20_000;
const TAIL_CHARS: usize = 10_000;
/// Bytes of a result kept for display.
pub const FULL_BYTES: usize = 1 << 20;

/// The result of one tool call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ToolOutput {
    /// What the model gets: at most [`MODEL_CHARS`] characters.
    pub text: String,
    /// What the TUI shows: the whole result, up to [`FULL_BYTES`].
    pub full: String,
    pub is_error: bool,
}

impl ToolOutput {
    pub fn ok(full: impl Into<String>) -> ToolOutput {
        ToolOutput::new(full.into(), false)
    }

    pub fn error(full: impl Into<String>) -> ToolOutput {
        ToolOutput::new(full.into(), true)
    }

    pub fn new(full: String, is_error: bool) -> ToolOutput {
        let full = cap_bytes(full, FULL_BYTES);
        ToolOutput {
            text: cut_for_model(&full),
            full,
            is_error,
        }
    }
}

/// Keeps the first 20 000 and last 10 000 characters of a long text.
pub fn cut_for_model(text: &str) -> String {
    let count = text.chars().count();
    if count <= MODEL_CHARS {
        return text.to_owned();
    }
    let head: String = text.chars().take(HEAD_CHARS).collect();
    let tail: String = text.chars().skip(count - TAIL_CHARS).collect();
    let cut = count - HEAD_CHARS - TAIL_CHARS;
    format!("{head}\n… {} characters cut …\n{tail}", group(cut))
}

/// Keeps a text within `max` bytes: its start and its end, with a mark.
pub fn cap_bytes(text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    let tail_len = max / 16;
    let head_end = floor_char(&text, max - tail_len);
    let tail_start = ceil_char(&text, text.len() - tail_len);
    let cut = tail_start - head_end;
    format!(
        "{}\n… {} bytes cut …\n{}",
        text.get(..head_end).unwrap_or(""),
        group(cut),
        text.get(tail_start..).unwrap_or("")
    )
}

fn floor_char(text: &str, mut i: usize) -> usize {
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_char(text: &str, mut i: usize) -> usize {
    while i < text.len() && !text.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// `12345` as `12 345`, as in the spec's messages.
pub fn group(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(' ');
        }
        out.push(c);
    }
    out
}

/// `1 occurrence`, `3 occurrences`.
pub fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{} {many}", group(n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouping() {
        assert_eq!(group(0), "0");
        assert_eq!(group(999), "999");
        assert_eq!(group(1234), "1 234");
        assert_eq!(group(12345), "12 345");
        assert_eq!(group(1234567), "1 234 567");
    }

    #[test]
    fn model_text_keeps_start_and_end() {
        let text: String = (0..40_000)
            .map(|i| if i % 2 == 0 { 'a' } else { 'é' })
            .collect();
        let cut = cut_for_model(&text);
        assert!(
            cut.contains("\n… 10 000 characters cut …\n"),
            "{}",
            &cut[..100]
        );
        assert_eq!(
            cut.chars().count(),
            30_000 + "\n… 10 000 characters cut …\n".chars().count()
        );
        assert_eq!(cut_for_model("short"), "short");
    }

    #[test]
    fn full_text_is_capped() {
        let text = "x".repeat(FULL_BYTES * 2);
        let capped = cap_bytes(text, FULL_BYTES);
        assert!(capped.len() < FULL_BYTES + 100);
        assert!(capped.contains("bytes cut"));
        let multibyte = "€".repeat(1000);
        let capped = cap_bytes(multibyte, 100);
        assert!(capped.starts_with('€') && capped.ends_with('€'));
    }

    #[test]
    fn output_has_both() {
        let out = ToolOutput::error("boom");
        assert!(out.is_error);
        assert_eq!(out.text, "boom");
        assert_eq!(out.full, "boom");
        assert_eq!(plural(1, "line", "lines"), "1 line");
        assert_eq!(plural(1500, "line", "lines"), "1 500 lines");
    }
}
