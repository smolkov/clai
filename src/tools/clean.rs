//! Cleaning of command output for the model (RFC 0028, "Clean command
//! output"): removes what carries no information and says what it left out.
//! The TUI keeps the raw output.

/// Runs of more than this many identical lines are collapsed.
const SAME_LINES: usize = 3;
/// Runs of more than this many progress lines are collapsed.
const PROGRESS_LINES: usize = 20;
/// Progress lines kept at the start and at the end of a collapsed run.
const PROGRESS_KEEP: usize = 3;
/// First words of progress lines of build and package tools.
const PROGRESS_WORDS: &[&str] = &[
    "Compiling",
    "Checking",
    "Documenting",
    "Downloading",
    "Downloaded",
    "Fresh",
    "Installing",
    "Unpacking",
];

/// The output as the model gets it: without escape sequences, with lines
/// overwritten by `\r` in their last state, and runs collapsed.
pub fn clean(text: &str) -> String {
    let plain = strip_escapes(text);
    let lines: Vec<&str> = plain.split('\n').map(last_state).collect();
    let lines = collapse_progress(&lines);
    let lines = collapse_same(&lines);
    lines.join("\n")
}

/// Removes ANSI escape sequences (CSI, OSC, two-byte escapes) and control
/// characters other than newline, tab and carriage return.
fn strip_escapes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{1b}' => match chars.next() {
                // CSI: parameters and intermediates, then a final byte.
                Some('[') => {
                    for c in chars.by_ref() {
                        if ('\u{40}'..='\u{7e}').contains(&c) {
                            break;
                        }
                    }
                }
                // OSC: ends with BEL or ESC \.
                Some(']') => {
                    while let Some(c) = chars.next() {
                        if c == '\u{7}' {
                            break;
                        }
                        if c == '\u{1b}' {
                            if chars.peek() == Some(&'\\') {
                                chars.next();
                            }
                            break;
                        }
                    }
                }
                _ => {}
            },
            '\n' | '\t' | '\r' => out.push(c),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out
}

/// A line redrawn with `\r` as a terminal shows it at the end: the last
/// part that is not empty.
fn last_state(line: &str) -> &str {
    let line = line.strip_suffix('\r').unwrap_or(line);
    line.rsplit('\r')
        .find(|part| !part.is_empty())
        .unwrap_or("")
}

/// The first word of a progress line, if it is one; lines that mention an
/// error or a warning never are.
fn progress_word(line: &str) -> Option<&'static str> {
    let word = line.split_whitespace().next()?;
    let word = PROGRESS_WORDS.iter().find(|w| **w == word)?;
    let lower = line.to_ascii_lowercase();
    if lower.contains("error") || lower.contains("warning") {
        return None;
    }
    Some(word)
}

fn collapse_progress(lines: &[&str]) -> Vec<String> {
    let mut out = Vec::with_capacity(lines.len());
    let mut i = 0;
    while let Some(line) = lines.get(i) {
        let Some(word) = progress_word(line) else {
            out.push((*line).to_owned());
            i += 1;
            continue;
        };
        let run = lines
            .get(i..)
            .unwrap_or_default()
            .iter()
            .take_while(|l| progress_word(l) == Some(word))
            .count();
        let block = lines.get(i..i + run).unwrap_or_default();
        if run > PROGRESS_LINES {
            out.extend(block.iter().take(PROGRESS_KEEP).map(|l| (*l).to_owned()));
            out.push(format!(
                "… {} more \"{word}\" lines",
                run - 2 * PROGRESS_KEEP
            ));
            out.extend(
                block
                    .iter()
                    .skip(run - PROGRESS_KEEP)
                    .map(|l| (*l).to_owned()),
            );
        } else {
            out.extend(block.iter().map(|l| (*l).to_owned()));
        }
        i += run;
    }
    out
}

fn collapse_same(lines: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(lines.len());
    let mut i = 0;
    while let Some(line) = lines.get(i) {
        let run = lines
            .get(i..)
            .unwrap_or_default()
            .iter()
            .take_while(|l| *l == line)
            .count();
        if run > SAME_LINES {
            out.push(line.clone());
            out.push(format!("… same line {} more times", run - 1));
        } else {
            out.extend(std::iter::repeat_n(line.clone(), run));
        }
        i += run;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_are_removed() {
        assert_eq!(
            clean("\u{1b}[1m\u{1b}[31merror\u{1b}[0m: bad\n\u{1b}]0;title\u{7}done\u{8}"),
            "error: bad\ndone"
        );
        assert_eq!(
            clean("a\u{1b}]8;;http://x\u{1b}\\link\u{1b}]8;;\u{1b}\\b"),
            "alinkb"
        );
        assert_eq!(clean("tab\there"), "tab\there");
    }

    #[test]
    fn carriage_returns_keep_the_last_state() {
        assert_eq!(
            clean("start\n 10%\r 50%\r100%\nend\r\n"),
            "start\n100%\nend\n"
        );
        // A trailing \r alone leaves the line.
        assert_eq!(clean("loading\r"), "loading");
    }

    #[test]
    fn identical_lines_collapse() {
        let text = format!("head\n{}tail", "same\n".repeat(60));
        assert_eq!(clean(&text), "head\nsame\n… same line 59 more times\ntail");
        // Three in a row stay.
        assert_eq!(clean("x\nx\nx"), "x\nx\nx");
    }

    #[test]
    fn progress_runs_collapse() {
        let mut text = String::from("   Updating crates.io index\n");
        for i in 0..30 {
            text.push_str(&format!("   Compiling crate{i} v1.0.{i}\n"));
        }
        text.push_str("error[E0308]: mismatched types\n");
        text.push_str("   Compiling with error in name v1\n");
        text.push_str("    Finished `dev` profile\nexit code 101");
        let cleaned = clean(&text);
        assert_eq!(
            cleaned,
            [
                "   Updating crates.io index",
                "   Compiling crate0 v1.0.0",
                "   Compiling crate1 v1.0.1",
                "   Compiling crate2 v1.0.2",
                "… 24 more \"Compiling\" lines",
                "   Compiling crate27 v1.0.27",
                "   Compiling crate28 v1.0.28",
                "   Compiling crate29 v1.0.29",
                "error[E0308]: mismatched types",
                "   Compiling with error in name v1",
                "    Finished `dev` profile",
                "exit code 101",
            ]
            .join("\n")
        );
        // Twenty stay.
        let short: String = (0..20).map(|i| format!("Compiling c{i}\n")).collect();
        assert_eq!(clean(&short), short);
    }

    #[test]
    fn different_progress_words_are_runs_of_their_own() {
        let mut text = String::new();
        for i in 0..15 {
            text.push_str(&format!("Downloading d{i}\n"));
        }
        for i in 0..15 {
            text.push_str(&format!("Compiling c{i}\n"));
        }
        assert_eq!(clean(&text), text);
    }

    #[test]
    fn plain_output_is_unchanged() {
        let text = "running 3 tests\ntest a ... ok\ntest b ... ok\n\nexit code 0";
        assert_eq!(clean(text), text);
    }
}
