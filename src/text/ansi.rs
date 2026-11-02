//! ANSI colours for terminal output; used only when colour is on (see
//! [`super::Terminal`]).

pub const GREEN: &str = "\x1b[32m";
pub const RED: &str = "\x1b[31m";
pub const YELLOW: &str = "\x1b[33m";
pub const CYAN: &str = "\x1b[36m";
pub const DIM: &str = "\x1b[2m";
pub const BOLD: &str = "\x1b[1m";
pub const RESET: &str = "\x1b[0m";

/// `text` in `colour` if `on`, else as it is.
pub fn paint(colour: &str, text: &str, on: bool) -> String {
    if on && !colour.is_empty() {
        format!("{colour}{text}{RESET}")
    } else {
        text.to_owned()
    }
}

/// The colour of a diff line: hunk headers cyan, added green, removed red.
pub fn diff_colour(line: &str) -> &'static str {
    if line.starts_with("@@") {
        CYAN
    } else if line.starts_with('+') {
        GREEN
    } else if line.starts_with('-') {
        RED
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paints_only_when_on() {
        assert_eq!(paint(RED, "x", true), "\x1b[31mx\x1b[0m");
        assert_eq!(paint(RED, "x", false), "x");
        assert_eq!(paint("", "x", true), "x");
        assert_eq!(diff_colour("@@ -1 +1 @@"), CYAN);
        assert_eq!(diff_colour("+a"), GREEN);
        assert_eq!(diff_colour("-a"), RED);
        assert_eq!(diff_colour(" a"), "");
    }
}
