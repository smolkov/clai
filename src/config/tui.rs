//! `[tui]` and `[themes]`: checked here, turned into colours by the TUI.

use std::ops::RangeInclusive;

use indexmap::IndexMap;
use serde::Deserialize;
use toml::Spanned;

use super::errors::{ConfigError, FileCtx};
use super::file::{ThemeFile, TuiFile};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Symbols {
    #[default]
    Auto,
    Unicode,
    Ascii,
}

/// `tui.activity`: the activity line while the session works (see
/// `spec/tui.md`, "Activity line").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivityLine {
    /// A playful verb per turn, with a shimmer.
    #[default]
    Playful,
    /// `Thinking…` and `Writing…`, no shimmer.
    Plain,
    /// No line.
    Off,
}

/// `tui.tool_preview`: what finished tool calls show below their summary
/// (see `spec/tui.md`, "Conversation entries").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolPreview {
    /// Edits, writes and commands show a preview; failed calls their error.
    #[default]
    Changes,
    /// Every call is expanded.
    All,
    /// Every finished call is one line.
    None,
}

impl ToolPreview {
    pub fn name(self) -> &'static str {
        match self {
            ToolPreview::Changes => "changes",
            ToolPreview::All => "all",
            ToolPreview::None => "none",
        }
    }
}

/// `tui.clipboard`: how copied text reaches the clipboard (see
/// `spec/tui.md`, "Copy").
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Clipboard {
    /// OSC 52, and a clipboard command when the session is local.
    #[default]
    Auto,
    /// Only the terminal's OSC 52 sequence.
    Osc52,
    /// Only a clipboard command: `wl-copy`, `xclip`, `xsel`, `pbcopy`.
    Command,
    /// Nothing is copied.
    Off,
}

impl Clipboard {
    pub fn name(self) -> &'static str {
        match self {
            Clipboard::Auto => "auto",
            Clipboard::Osc52 => "osc52",
            Clipboard::Command => "command",
            Clipboard::Off => "off",
        }
    }
}

/// TUI settings, with the defaults of `spec/config.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tui {
    pub theme: String,
    pub symbols: Symbols,
    pub mouse: bool,
    /// 0 = no limit.
    pub tool_output_lines: u32,
    /// `tool_preview`; `expand_tool_calls = true` reads as `all`.
    pub tool_preview: ToolPreview,
    pub clipboard: Clipboard,
    pub syntax_highlight: bool,
    pub timestamps: bool,
    pub scroll_lines: u16,
    pub max_fps: u16,
    pub activity: ActivityLine,
}

impl Default for Tui {
    fn default() -> Self {
        Tui {
            theme: "terminal".to_owned(),
            symbols: Symbols::Auto,
            mouse: true,
            tool_output_lines: 20,
            tool_preview: ToolPreview::Changes,
            clipboard: Clipboard::Auto,
            syntax_highlight: true,
            timestamps: false,
            scroll_lines: 3,
            max_fps: 30,
            activity: ActivityLine::Playful,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinTheme {
    Terminal,
    Dark,
    Light,
}

impl BuiltinTheme {
    pub fn from_name(name: &str) -> Option<BuiltinTheme> {
        match name {
            "terminal" => Some(BuiltinTheme::Terminal),
            "dark" => Some(BuiltinTheme::Dark),
            "light" => Some(BuiltinTheme::Light),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colour {
    /// The terminal's default foreground or background.
    Default,
    /// One of the 16 ANSI colours, by name.
    Named(&'static str),
    Index(u8),
    Rgb(u8, u8, u8),
}

const ANSI_NAMES: [&str; 16] = [
    "black",
    "red",
    "green",
    "yellow",
    "blue",
    "magenta",
    "cyan",
    "white",
    "bright_black",
    "bright_red",
    "bright_green",
    "bright_yellow",
    "bright_blue",
    "bright_magenta",
    "bright_cyan",
    "bright_white",
];

/// A colour value: ANSI name, `default`, `#rrggbb`, or an integer 0–255.
pub fn parse_colour(value: &toml::Value) -> Result<Colour, String> {
    match value {
        toml::Value::Integer(n) => u8::try_from(*n)
            .map(Colour::Index)
            .map_err(|_| format!("colour index {n} is not in 0–255")),
        toml::Value::String(s) if s == "default" => Ok(Colour::Default),
        toml::Value::String(s) => {
            if let Some(name) = ANSI_NAMES.iter().find(|n| **n == s) {
                return Ok(Colour::Named(name));
            }
            parse_hex(s).ok_or_else(|| {
                format!("unknown colour `{s}`; use an ANSI colour name, \"default\", \"#rrggbb\" or 0–255")
            })
        }
        other => Err(format!(
            "colour must be a string or an integer, not {}",
            other.type_str()
        )),
    }
}

fn parse_hex(s: &str) -> Option<Colour> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 || !hex.is_ascii() {
        return None;
    }
    let byte = |range| u8::from_str_radix(hex.get(range)?, 16).ok();
    Some(Colour::Rgb(byte(0..2)?, byte(2..4)?, byte(4..6)?))
}

/// The names the `code` key of a theme takes: `clai`'s own (`ansi`, and those
/// of the bundled themes) and the themes `syntect` bundles.
pub const CODE_THEMES: [&str; 11] = [
    "ansi",
    "atom-one-dark",
    "nord",
    "gruvbox-dark",
    "base16-ocean.dark",
    "base16-eighties.dark",
    "base16-mocha.dark",
    "base16-ocean.light",
    "InspiredGitHub",
    "Solarized (dark)",
    "Solarized (light)",
];

/// Where a custom theme comes from (see `spec/config.md`, "Themes").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemeSource {
    /// A `[themes.NAME]` table of the user file.
    Table,
    /// `themes/NAME.toml` next to the user file.
    File(std::path::PathBuf),
    /// Built into `clai`.
    Bundled,
}

/// A custom theme: a built-in base with some colours changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub base: BuiltinTheme,
    pub colours: Vec<(&'static str, Colour)>,
    /// Name of the syntax highlighting theme.
    pub code: Option<String>,
    pub source: ThemeSource,
}

pub(crate) fn build_themes(
    ctx: FileCtx<'_>,
    themes: IndexMap<String, Spanned<ThemeFile>>,
    errors: &mut Vec<ConfigError>,
) -> IndexMap<String, Theme> {
    let mut built = IndexMap::new();
    for (name, spanned) in themes {
        let span = spanned.span();
        let file = spanned.into_inner();
        let key = format!("themes.{name}");
        if BuiltinTheme::from_name(&name).is_some() {
            errors.push(ctx.error(
                Some(span.clone()),
                Some(key.clone()),
                "a custom theme may not use the name of a built-in theme",
            ));
        }
        if let Some(theme) = build_theme(ctx, &format!("{key}."), file, ThemeSource::Table, errors)
        {
            built.insert(name, theme);
        }
    }
    built
}

/// One theme from its keys; `prefix` comes before each key in errors:
/// `themes.solar.` for a table, nothing for a theme file.
pub(crate) fn build_theme(
    ctx: FileCtx<'_>,
    prefix: &str,
    file: ThemeFile,
    source: ThemeSource,
    errors: &mut Vec<ConfigError>,
) -> Option<Theme> {
    let base = match &file.base {
        None => BuiltinTheme::Terminal,
        Some(base) => match BuiltinTheme::from_name(base.get_ref()) {
            Some(b) => b,
            None => {
                errors.push(ctx.error(
                    Some(base.span()),
                    Some(format!("{prefix}base")),
                    format!(
                        "base `{}` is not a built-in theme (terminal, dark, light)",
                        base.get_ref()
                    ),
                ));
                return None;
            }
        },
    };
    let mut colours = Vec::new();
    for (colour_key, value) in colour_keys(&file) {
        let Some(value) = value else { continue };
        match parse_colour(value.get_ref()) {
            Ok(colour) => colours.push((colour_key, colour)),
            Err(message) => errors.push(ctx.error(
                Some(value.span()),
                Some(format!("{prefix}{colour_key}")),
                message,
            )),
        }
    }
    if let Some(code) = &file.code
        && !CODE_THEMES.contains(&code.as_str())
    {
        errors.push(ctx.error(
            None,
            Some(format!("{prefix}code")),
            format!(
                "unknown syntax theme `{code}`; one of: {}",
                CODE_THEMES.join(", ")
            ),
        ));
    }
    Some(Theme {
        base,
        colours,
        code: file.code,
        source,
    })
}

fn colour_keys(file: &ThemeFile) -> [(&'static str, &Option<Spanned<toml::Value>>); 12] {
    [
        ("background", &file.background),
        ("text", &file.text),
        ("dim", &file.dim),
        ("user", &file.user),
        ("accent", &file.accent),
        ("border", &file.border),
        ("selection", &file.selection),
        ("success", &file.success),
        ("error", &file.error),
        ("warning", &file.warning),
        ("diff_add", &file.diff_add),
        ("diff_remove", &file.diff_remove),
    ]
}

/// Whether `name` is a built-in or a configured theme.
pub fn theme_exists(name: &str, themes: &IndexMap<String, Theme>) -> bool {
    BuiltinTheme::from_name(name).is_some() || themes.contains_key(name)
}

pub(crate) fn build_tui(
    ctx: FileCtx<'_>,
    file: Option<TuiFile>,
    themes: &IndexMap<String, Theme>,
    // Broken theme files in use: their errors say enough already.
    broken: &[String],
    errors: &mut Vec<ConfigError>,
) -> Tui {
    let mut tui = Tui::default();
    let Some(file) = file else {
        return tui;
    };
    if let Some(theme) = file.theme {
        if theme_exists(theme.get_ref(), themes) || broken.contains(theme.get_ref()) {
            tui.theme = theme.into_inner();
        } else {
            errors.push(ctx.error(
                Some(theme.span()),
                Some("tui.theme".into()),
                format!("unknown theme `{}`", theme.get_ref()),
            ));
        }
    }
    tui.symbols = file.symbols.unwrap_or(tui.symbols);
    tui.mouse = file.mouse.unwrap_or(tui.mouse);
    // `expand_tool_calls` is deprecated: `true` reads as `all`, and
    // `tool_preview` wins when both are set.
    tui.tool_preview = match (file.tool_preview, file.expand_tool_calls) {
        (Some(preview), _) => preview,
        (None, Some(true)) => ToolPreview::All,
        (None, _) => tui.tool_preview,
    };
    tui.clipboard = file.clipboard.unwrap_or(tui.clipboard);
    tui.syntax_highlight = file.syntax_highlight.unwrap_or(tui.syntax_highlight);
    tui.timestamps = file.timestamps.unwrap_or(tui.timestamps);
    tui.activity = file.activity.unwrap_or(tui.activity);
    let mut int = |value: &Option<Spanned<i64>>, key: &str, range: RangeInclusive<i64>| {
        let value = value.as_ref()?;
        let n = *value.get_ref();
        if range.contains(&n) {
            Some(n)
        } else {
            errors.push(ctx.error(
                Some(value.span()),
                Some(format!("tui.{key}")),
                format!("{n} is not in {}–{}", range.start(), range.end()),
            ));
            None
        }
    };
    if let Some(n) = int(&file.tool_output_lines, "tool_output_lines", 0..=1_000_000) {
        tui.tool_output_lines = u32::try_from(n).unwrap_or(tui.tool_output_lines);
    }
    if let Some(n) = int(&file.scroll_lines, "scroll_lines", 1..=1000) {
        tui.scroll_lines = u16::try_from(n).unwrap_or(tui.scroll_lines);
    }
    if let Some(n) = int(&file.max_fps, "max_fps", 1..=120) {
        tui.max_fps = u16::try_from(n).unwrap_or(tui.max_fps);
    }
    tui
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours() {
        let s = |s: &str| toml::Value::String(s.into());
        assert_eq!(parse_colour(&s("cyan")), Ok(Colour::Named("cyan")));
        assert_eq!(
            parse_colour(&s("bright_red")),
            Ok(Colour::Named("bright_red"))
        );
        assert_eq!(parse_colour(&s("default")), Ok(Colour::Default));
        assert_eq!(
            parse_colour(&s("#e5a50A")),
            Ok(Colour::Rgb(0xe5, 0xa5, 0x0a))
        );
        assert_eq!(
            parse_colour(&toml::Value::Integer(208)),
            Ok(Colour::Index(208))
        );
        assert!(parse_colour(&toml::Value::Integer(256)).is_err());
        assert!(parse_colour(&s("#12345")).is_err());
        assert!(parse_colour(&s("#12345g")).is_err());
        assert!(parse_colour(&s("#ééé")).is_err());
        assert!(parse_colour(&s("purple")).is_err());
        assert!(parse_colour(&toml::Value::Boolean(true)).is_err());
    }
}
