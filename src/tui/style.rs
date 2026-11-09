//! Styles and symbols: the themes of `spec/config.md`, "Themes" (`terminal`
//! on the terminal's 16 colours, `dark` and `light` on 256, custom themes over
//! one of them), `NO_COLOR`, Unicode or ASCII.

use indexmap::IndexMap;
use ratatui::style::{Color, Modifier, Style};
use ratatui::symbols::border;

use crate::config::{BuiltinTheme, Colour, Theme};

/// What colours the terminal shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colouring {
    /// `NO_COLOR`, or standard output is not a terminal: attributes only.
    None,
    /// `#rrggbb` is shown as the nearest of the 256 colours.
    Ansi256,
    /// `COLORTERM` is `truecolor` or `24bit`.
    TrueColour,
}

impl Colouring {
    pub fn detect(colour: bool, colorterm: Option<&str>) -> Colouring {
        match colorterm {
            _ if !colour => Colouring::None,
            Some("truecolor" | "24bit") => Colouring::TrueColour,
            _ => Colouring::Ansi256,
        }
    }
}

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
    /// Painted over the whole screen when the theme sets `background`.
    pub background: Option<Color>,
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
            background: None,
        }
    }

    /// The styles of a theme by name: built in, or one of `themes`. An
    /// unknown name is `terminal`, which loading has already ruled out.
    pub fn themed(name: &str, themes: &IndexMap<String, Theme>, colouring: Colouring) -> Styles {
        if colouring == Colouring::None {
            return Styles::new(false);
        }
        let (base, colours): (BuiltinTheme, &[(&str, Colour)]) =
            match (BuiltinTheme::from_name(name), themes.get(name)) {
                (Some(builtin), _) => (builtin, &[]),
                (None, Some(theme)) => (theme.base, &theme.colours),
                (None, None) => (BuiltinTheme::Terminal, &[]),
            };
        let mut styles = match base {
            BuiltinTheme::Terminal => Styles::new(true),
            BuiltinTheme::Dark => Styles::palette(&DARK),
            BuiltinTheme::Light => Styles::palette(&LIGHT),
        };
        for (key, colour) in colours {
            styles.set(key, colour_of(*colour, colouring));
        }
        // A selection by background keeps the theme's text colour.
        if styles.selection.bg.is_some() {
            styles.selection.fg = styles.text.fg;
        }
        styles
    }

    fn palette(p: &Palette) -> Styles {
        let fg = |n: u8| Style::new().fg(Color::Indexed(n));
        let bold = Modifier::BOLD;
        Styles {
            text: fg(p.text),
            dim: fg(p.dim),
            user: fg(p.user).add_modifier(bold),
            accent: fg(p.accent),
            border: fg(p.border),
            selection: Style::new().bg(Color::Indexed(p.selection)),
            success: fg(p.success),
            error: fg(p.error).add_modifier(bold),
            warning: fg(p.warning),
            diff_add: fg(p.diff_add),
            diff_remove: fg(p.diff_remove),
            code: fg(p.code),
            heading: fg(p.accent).add_modifier(bold),
            background: None,
        }
    }

    /// One colour key of a theme over the base's style. A colour replaces
    /// the dim attribute of `dim` and `border`.
    fn set(&mut self, key: &str, colour: Color) {
        let fg = |style: Style| style.fg(colour);
        match key {
            "background" => self.background = Some(colour),
            "text" => self.text = fg(self.text),
            "dim" => self.dim = fg(self.dim).remove_modifier(Modifier::DIM),
            "user" => self.user = fg(self.user),
            "accent" => {
                self.accent = fg(self.accent);
                self.heading = fg(self.heading);
            }
            "border" => self.border = fg(self.border).remove_modifier(Modifier::DIM),
            "selection" => self.selection = Style::new().bg(colour),
            "success" => self.success = fg(self.success),
            "error" => self.error = fg(self.error),
            "warning" => self.warning = fg(self.warning),
            "diff_add" => self.diff_add = fg(self.diff_add),
            "diff_remove" => self.diff_remove = fg(self.diff_remove),
            _ => {}
        }
    }
}

/// The 256-colour indexes of a built-in theme.
struct Palette {
    text: u8,
    dim: u8,
    user: u8,
    accent: u8,
    border: u8,
    selection: u8,
    success: u8,
    error: u8,
    warning: u8,
    diff_add: u8,
    diff_remove: u8,
    code: u8,
}

/// For dark backgrounds (see `rfcs/0006-tui-settings.md`, "Themes").
const DARK: Palette = Palette {
    text: 252,
    dim: 244,
    user: 117,
    accent: 75,
    border: 240,
    selection: 238,
    success: 114,
    error: 203,
    warning: 221,
    diff_add: 114,
    diff_remove: 203,
    code: 180,
};

/// For light backgrounds.
const LIGHT: Palette = Palette {
    text: 235,
    dim: 245,
    user: 25,
    accent: 26,
    border: 250,
    selection: 254,
    success: 28,
    error: 160,
    warning: 136,
    diff_add: 28,
    diff_remove: 160,
    code: 130,
};

const ANSI: [(&str, Color); 16] = [
    ("black", Color::Black),
    ("red", Color::Red),
    ("green", Color::Green),
    ("yellow", Color::Yellow),
    ("blue", Color::Blue),
    ("magenta", Color::Magenta),
    ("cyan", Color::Cyan),
    ("white", Color::Gray),
    ("bright_black", Color::DarkGray),
    ("bright_red", Color::LightRed),
    ("bright_green", Color::LightGreen),
    ("bright_yellow", Color::LightYellow),
    ("bright_blue", Color::LightBlue),
    ("bright_magenta", Color::LightMagenta),
    ("bright_cyan", Color::LightCyan),
    ("bright_white", Color::White),
];

/// A configured colour as the terminal can show it.
pub fn colour_of(colour: Colour, colouring: Colouring) -> Color {
    match colour {
        Colour::Default => Color::Reset,
        Colour::Named(name) => ANSI
            .iter()
            .find(|(n, _)| *n == name)
            .map_or(Color::Reset, |(_, c)| *c),
        Colour::Index(n) => Color::Indexed(n),
        Colour::Rgb(r, g, b) if colouring == Colouring::TrueColour => Color::Rgb(r, g, b),
        Colour::Rgb(r, g, b) => Color::Indexed(nearest_256(r, g, b)),
    }
}

/// The nearest colour of the 6×6×6 cube (16–231) and the grey ramp
/// (232–255), by distance in RGB.
pub fn nearest_256(r: u8, g: u8, b: u8) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let distance = |(r2, g2, b2): (u8, u8, u8)| {
        let d = |a: u8, b: u8| (i32::from(a) - i32::from(b)).pow(2);
        d(r, r2) + d(g, g2) + d(b, b2)
    };
    let level = |c: u8| {
        LEVELS
            .iter()
            .enumerate()
            .min_by_key(|(_, l)| (i32::from(**l) - i32::from(c)).abs())
            .map_or(0, |(i, _)| i)
    };
    let (ri, gi, bi) = (level(r), level(g), level(b));
    let at = |i: usize| LEVELS.get(i).copied().unwrap_or(0);
    let cube = (at(ri), at(gi), at(bi));
    let cube_index = 16 + 36 * ri + 6 * gi + bi;
    let grey_step = ((u32::from(r) + u32::from(g) + u32::from(b)) / 3).saturating_sub(3) / 10;
    let grey_step = grey_step.min(23);
    let grey_level = u8::try_from(8 + 10 * grey_step).unwrap_or(238);
    let grey = (grey_level, grey_level, grey_level);
    let index = if distance(grey) < distance(cube) {
        232 + usize::try_from(grey_step).unwrap_or(0)
    } else {
        cube_index
    };
    u8::try_from(index).unwrap_or(231)
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
    /// A marked row of a list.
    pub marked: &'static str,
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
                marked: "■",
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
                marked: "#",
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
    /// The `terminal` theme.
    #[cfg(test)]
    pub fn new(unicode: bool, colour: bool) -> Look {
        Look {
            styles: Styles::new(colour),
            symbols: Symbols::new(unicode),
        }
    }

    pub fn themed(unicode: bool, styles: Styles) -> Look {
        Look {
            styles,
            symbols: Symbols::new(unicode),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ThemeSource;

    fn custom(base: BuiltinTheme, colours: Vec<(&'static str, Colour)>) -> IndexMap<String, Theme> {
        IndexMap::from([(
            "mine".to_owned(),
            Theme {
                base,
                colours,
                code: None,
                source: ThemeSource::Table,
            },
        )])
    }

    #[test]
    fn terminal_is_the_fixed_set_of_before() {
        let none = IndexMap::new();
        assert_eq!(
            Styles::themed("terminal", &none, Colouring::Ansi256),
            Styles::new(true)
        );
        // NO_COLOR: attributes only, whatever the theme.
        assert_eq!(
            Styles::themed("dark", &none, Colouring::None),
            Styles::new(false)
        );
    }

    #[test]
    fn dark_and_light_use_their_palettes() {
        let none = IndexMap::new();
        let dark = Styles::themed("dark", &none, Colouring::Ansi256);
        assert_eq!(dark.text.fg, Some(Color::Indexed(252)));
        assert_eq!(dark.selection.bg, Some(Color::Indexed(238)));
        assert_eq!(dark.selection.fg, Some(Color::Indexed(252)));
        assert!(dark.error.add_modifier.contains(Modifier::BOLD));
        assert_eq!(dark.background, None);
        let light = Styles::themed("light", &none, Colouring::Ansi256);
        assert_eq!(light.accent.fg, Some(Color::Indexed(26)));
        assert_eq!(light.heading.fg, Some(Color::Indexed(26)));
    }

    #[test]
    fn a_custom_theme_changes_only_its_keys() {
        let themes = custom(
            BuiltinTheme::Dark,
            vec![
                ("accent", Colour::Rgb(0xe5, 0xa5, 0x0a)),
                ("dim", Colour::Named("bright_black")),
                ("background", Colour::Rgb(0x28, 0x2c, 0x34)),
            ],
        );
        let mine = Styles::themed("mine", &themes, Colouring::TrueColour);
        let dark = Styles::themed("dark", &themes, Colouring::TrueColour);
        assert_eq!(mine.accent.fg, Some(Color::Rgb(0xe5, 0xa5, 0x0a)));
        assert_eq!(mine.heading.fg, Some(Color::Rgb(0xe5, 0xa5, 0x0a)));
        assert_eq!(mine.dim.fg, Some(Color::DarkGray));
        assert_eq!(mine.background, Some(Color::Rgb(0x28, 0x2c, 0x34)));
        assert_eq!(mine.text, dark.text);
        assert_eq!(mine.success, dark.success);
        // Over `terminal`, a coloured dim is no longer the dim attribute.
        let themes = custom(BuiltinTheme::Terminal, vec![("dim", Colour::Index(244))]);
        let mine = Styles::themed("mine", &themes, Colouring::Ansi256);
        assert!(!mine.dim.add_modifier.contains(Modifier::DIM));
        // A selection colour replaces reversed video.
        let themes = custom(
            BuiltinTheme::Terminal,
            vec![("selection", Colour::Index(238))],
        );
        let mine = Styles::themed("mine", &themes, Colouring::Ansi256);
        assert_eq!(mine.selection.bg, Some(Color::Indexed(238)));
        assert!(!mine.selection.add_modifier.contains(Modifier::REVERSED));
    }

    #[test]
    fn true_colour_or_the_nearest_of_256() {
        let rgb = Colour::Rgb(0x2e, 0x34, 0x40);
        assert_eq!(
            colour_of(rgb, Colouring::TrueColour),
            Color::Rgb(0x2e, 0x34, 0x40)
        );
        // The grey 237 (#3a3a3a) is nearer than any colour of the cube.
        assert_eq!(colour_of(rgb, Colouring::Ansi256), Color::Indexed(237));
        assert_eq!(nearest_256(0, 0, 0), 16);
        assert_eq!(nearest_256(255, 255, 255), 231);
        assert_eq!(nearest_256(255, 0, 0), 196);
        assert_eq!(nearest_256(0x80, 0x80, 0x80), 244);
        assert_eq!(colour_of(Colour::Default, Colouring::Ansi256), Color::Reset);
        assert_eq!(
            colour_of(Colour::Named("bright_white"), Colouring::Ansi256),
            Color::White
        );
        assert_eq!(Colouring::detect(false, Some("truecolor")), Colouring::None);
        assert_eq!(
            Colouring::detect(true, Some("24bit")),
            Colouring::TrueColour
        );
        assert_eq!(Colouring::detect(true, None), Colouring::Ansi256);
    }
}
