//! Styles and symbols: one fixed set on the terminal's 16 colours (what the
//! `terminal` theme will be), `NO_COLOR`, Unicode or ASCII.

use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;

/// All styles the TUI uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Styles {
    pub text: Style,
    pub dim: Style,
    pub user: Style,
    pub accent: Style,
    pub border: Style,
    pub selection: Style,
    pub success: Style,
    pub error: Style,
    pub warning: Style,
    pub diff_add: Style,
    pub diff_remove: Style,
    pub code: Style,
    pub heading: Style,
}

impl Styles {
    /// With `colour` off (`NO_COLOR`) only text attributes are left.
    pub fn new(colour: bool) -> Styles {
        let fg = |colour_on: Color| {
            if colour {
                Style::new().fg(colour_on)
            } else {
                Style::new()
            }
        };
        let dim = Style::new().add_modifier(Modifier::DIM);
        Styles {
            text: Style::new(),
            dim,
            user: fg(Color::Cyan).add_modifier(Modifier::BOLD),
            accent: fg(Color::Blue),
            border: dim,
            selection: Style::new().add_modifier(Modifier::REVERSED),
            success: fg(Color::Green),
            error: fg(Color::Red).add_modifier(Modifier::BOLD),
            warning: fg(Color::Yellow),
            diff_add: fg(Color::Green),
            diff_remove: fg(Color::Red),
            code: fg(Color::Yellow),
            heading: fg(Color::Blue).add_modifier(Modifier::BOLD),
        }
    }
}

/// All symbols the TUI uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Symbols {
    pub unicode: bool,
    pub collapsed: &'static str,
    pub expanded: &'static str,
    pub done: &'static str,
    pub failed: &'static str,
    pub denied: &'static str,
    pub waiting: &'static str,
    pub spinner: &'static [&'static str],
    pub more: &'static str,
    pub bullet: &'static str,
    pub quote: &'static str,
    pub new_output: &'static str,
    pub dot: &'static str,
    /// The selected row of a list.
    pub pointer: &'static str,
    /// A mark that needs attention.
    pub warning: &'static str,
    /// Plain lines across the screen and in Markdown.
    pub rule: &'static str,
    /// Between the parts of a footer row.
    pub separator: &'static str,
    /// Of popups.
    pub border: border::Set<'static>,
}

const ASCII_BORDER: border::Set<'static> = border::Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

impl Symbols {
    pub fn new(unicode: bool) -> Symbols {
        if unicode {
            Symbols {
                unicode,
                collapsed: "▸",
                expanded: "▾",
                done: "✓",
                failed: "✗",
                denied: "⊘",
                waiting: "?",
                spinner: &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
                more: "…",
                bullet: "•",
                quote: "│",
                new_output: "↓",
                dot: "●",
                pointer: "▶",
                warning: "⚠",
                rule: "─",
                separator: "·",
                border: border::PLAIN,
            }
        } else {
            Symbols {
                unicode,
                collapsed: ">",
                expanded: "v",
                done: "+",
                failed: "x",
                denied: "-",
                waiting: "?",
                spinner: &["|", "/", "-", "\\"],
                more: "...",
                bullet: "*",
                quote: "|",
                new_output: "v",
                dot: "*",
                pointer: ">",
                warning: "!",
                rule: "-",
                separator: "-",
                border: ASCII_BORDER,
            }
        }
    }
}

/// How things look.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Look {
    pub styles: Styles,
    pub symbols: Symbols,
}

impl Look {
    pub fn new(unicode: bool, colour: bool) -> Look {
        Look {
            styles: Styles::new(colour),
            symbols: Symbols::new(unicode),
        }
    }
}
