//! Syntax highlighting of code (see `spec/tui.md`, "Syntax highlighting"):
//! code blocks, diffs, new files and commands. The grammars and themes of
//! `syntect` are loaded once, in the background; until they are there, code
//! is shown in its plain style. Results are cached, and a text that grows
//! (a code block while it streams) goes on from where it stopped.

use std::collections::{HashMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::str::FromStr;
use std::sync::{Arc, Mutex, OnceLock};

use ratatui::style::{Color, Modifier, Style};
use syntect::highlighting::{
    self as sh, FontStyle, HighlightIterator, HighlightState, ScopeSelectors, StyleModifier, Theme,
    ThemeItem, ThemeSet, ThemeSettings,
};
use syntect::parsing::{ParseState, ScopeStack, SyntaxReference, SyntaxSet};

use super::style::{Colouring, nearest_256};
use super::wrap::Piece;

/// More lines than this are not highlighted.
pub const MAX_LINES: usize = 5000;
/// A line longer than this ends the highlighting of its text: grammars get
/// slow on such lines.
pub const MAX_LINE: usize = 2000;
/// Highlighted texts kept.
const CACHE: usize = 512;
/// Texts that may grow, for a prefix match.
const GROWING: usize = 8;

/// Which colours code gets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeTheme {
    /// `tui.syntax_highlight = false`, `NO_COLOR`.
    Off,
    /// `clai`'s own theme on the terminal's 16 colours.
    Ansi,
    /// A theme of `syntect`, by name.
    Named(&'static str),
}

/// What highlighting needs from the look: part of [`super::style::Styles`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Syntax {
    pub theme: CodeTheme,
    pub colouring: Colouring,
}

impl Syntax {
    pub const OFF: Syntax = Syntax {
        theme: CodeTheme::Off,
        colouring: Colouring::None,
    };
}

static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
static THEMES: OnceLock<ThemeSet> = OnceLock::new();
static ANSI: OnceLock<Theme> = OnceLock::new();
static OWN: OnceLock<Vec<(&'static str, Theme)>> = OnceLock::new();

/// Loads the grammars and themes; it takes a moment, so the TUI calls it in
/// the background. Later calls return at once.
pub fn load() {
    SYNTAXES.get_or_init(SyntaxSet::load_defaults_newlines);
    THEMES.get_or_init(ThemeSet::load_defaults);
    ANSI.get_or_init(ansi_theme);
    OWN.get_or_init(own_themes);
}

/// A colour of the `ansi` theme: the alpha 0 marks a terminal colour by its
/// index in `r`, as `bat` does.
const fn terminal(index: u8) -> sh::Color {
    sh::Color {
        r: index,
        g: 0,
        b: 0,
        a: 0,
    }
}

/// The terminal's own foreground.
const DEFAULT: sh::Color = sh::Color {
    r: 0,
    g: 0,
    b: 0,
    a: 1,
};

fn ansi_theme() -> Theme {
    let item = |scopes: &str, colour: sh::Color, font: Option<FontStyle>| {
        ScopeSelectors::from_str(scopes)
            .ok()
            .map(|scope| ThemeItem {
                scope,
                style: StyleModifier {
                    foreground: Some(colour),
                    background: None,
                    font_style: font,
                },
            })
    };
    let items = [
        item(
            "comment, punctuation.definition.comment",
            terminal(8),
            Some(FontStyle::ITALIC),
        ),
        item(
            "string, constant.character, punctuation.definition.string",
            terminal(2),
            None,
        ),
        item(
            "constant.numeric, constant.language, constant.other",
            terminal(3),
            None,
        ),
        item(
            "keyword, storage.modifier, storage.type.function, keyword.operator.word",
            terminal(5),
            None,
        ),
        item("keyword.operator, punctuation", DEFAULT, None),
        item(
            "storage.type, entity.name.type, support.type, entity.name.class",
            terminal(6),
            None,
        ),
        item(
            "entity.name.function, support.function, meta.function-call variable.function",
            terminal(4),
            None,
        ),
        item(
            "entity.name.tag, entity.other.attribute-name, variable.parameter",
            terminal(6),
            None,
        ),
        item("markup.heading", terminal(4), Some(FontStyle::BOLD)),
        item("markup.inserted", terminal(2), None),
        item("markup.deleted", terminal(1), None),
    ];
    Theme {
        name: Some("ansi".to_owned()),
        author: None,
        settings: ThemeSettings {
            foreground: Some(DEFAULT),
            ..ThemeSettings::default()
        },
        scopes: items.into_iter().flatten().collect(),
    }
}

/// The theme of a [`CodeTheme`], once loaded.
fn theme(code: CodeTheme) -> Option<&'static Theme> {
    match code {
        CodeTheme::Off => None,
        CodeTheme::Ansi => ANSI.get(),
        CodeTheme::Named(name) => OWN
            .get()?
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, t)| t)
            .or_else(|| THEMES.get()?.themes.get(name)),
    }
}

/// The colours of a syntax theme made from a palette, by role.
struct Palette {
    text: u32,
    comment: u32,
    keyword: u32,
    string: u32,
    number: u32,
    function: u32,
    kind: u32,
    variable: u32,
    operator: u32,
    tag: u32,
}

const fn rgb(hex: u32) -> sh::Color {
    sh::Color {
        r: ((hex >> 16) & 0xff) as u8,
        g: ((hex >> 8) & 0xff) as u8,
        b: (hex & 0xff) as u8,
        a: 0xff,
    }
}

/// One Dark of Atom, Nord and Gruvbox (dark): the syntax colours of the
/// bundled themes of the same name, from their published palettes.
const OWN_PALETTES: [(&str, Palette); 3] = [
    (
        "atom-one-dark",
        Palette {
            text: 0xabb2bf,
            comment: 0x5c6370,
            keyword: 0xc678dd,
            string: 0x98c379,
            number: 0xd19a66,
            function: 0x61afef,
            kind: 0xe5c07b,
            variable: 0xe06c75,
            operator: 0x56b6c2,
            tag: 0xe06c75,
        },
    ),
    (
        "nord",
        Palette {
            text: 0xd8dee9,
            comment: 0x616e88,
            keyword: 0x81a1c1,
            string: 0xa3be8c,
            number: 0xb48ead,
            function: 0x88c0d0,
            kind: 0x8fbcbb,
            variable: 0xd8dee9,
            operator: 0x81a1c1,
            tag: 0x81a1c1,
        },
    ),
    (
        "gruvbox-dark",
        Palette {
            text: 0xebdbb2,
            comment: 0x928374,
            keyword: 0xfb4934,
            string: 0xb8bb26,
            number: 0xd3869b,
            function: 0x8ec07c,
            kind: 0xfabd2f,
            variable: 0x83a598,
            operator: 0xfe8019,
            tag: 0xfb4934,
        },
    ),
];

fn palette_theme(name: &str, p: &Palette) -> Theme {
    let item = |scopes: &str, colour: u32, font: Option<FontStyle>| {
        ScopeSelectors::from_str(scopes)
            .ok()
            .map(|scope| ThemeItem {
                scope,
                style: StyleModifier {
                    foreground: Some(rgb(colour)),
                    background: None,
                    font_style: font,
                },
            })
    };
    let items = [
        item(
            "comment, punctuation.definition.comment",
            p.comment,
            Some(FontStyle::ITALIC),
        ),
        item(
            "string, constant.character, punctuation.definition.string",
            p.string,
            None,
        ),
        item(
            "constant.numeric, constant.language, constant.other",
            p.number,
            None,
        ),
        item(
            "keyword, storage, storage.type.function, keyword.operator.word",
            p.keyword,
            None,
        ),
        item("keyword.operator", p.operator, None),
        item("punctuation", p.text, None),
        item(
            "entity.name.type, entity.name.class, support.type, support.class, storage.type",
            p.kind,
            None,
        ),
        item(
            "entity.name.function, support.function, meta.function-call variable.function",
            p.function,
            None,
        ),
        item(
            "variable.parameter, variable.other.member",
            p.variable,
            None,
        ),
        item("entity.name.tag", p.tag, None),
        item("entity.other.attribute-name", p.number, None),
        item("markup.heading", p.function, Some(FontStyle::BOLD)),
        item("markup.inserted", p.string, None),
        item("markup.deleted", p.tag, None),
    ];
    Theme {
        name: Some(name.to_owned()),
        author: None,
        settings: ThemeSettings {
            foreground: Some(rgb(p.text)),
            ..ThemeSettings::default()
        },
        scopes: items.into_iter().flatten().collect(),
    }
}

fn own_themes() -> Vec<(&'static str, Theme)> {
    OWN_PALETTES
        .iter()
        .map(|(name, palette)| (*name, palette_theme(name, palette)))
        .collect()
}

/// The grammar of a fence's info string: `rust`, `rs`, `py`, `sh`.
pub fn by_token(token: &str) -> Option<&'static SyntaxReference> {
    let syntaxes = SYNTAXES.get()?;
    let token = token
        .split([' ', ',', '{'])
        .next()
        .unwrap_or_default()
        .trim();
    if token.is_empty() {
        return None;
    }
    let lower = token.to_lowercase();
    let alias = match lower.as_str() {
        "shell" | "bash" | "zsh" | "console" | "shellsession" => "sh",
        "golang" => "go",
        "js" | "javascript" | "jsx" | "node" => "js",
        "ts" | "typescript" | "tsx" => "js",
        "c++" | "cpp" => "cpp",
        "py" | "python3" => "py",
        "yml" => "yaml",
        "jsonc" | "json5" => "json",
        other => other,
    };
    syntaxes
        .find_syntax_by_token(alias)
        .or_else(|| syntaxes.find_syntax_by_extension(alias))
}

/// The grammar of a file, by its name, else by its first line
/// (`#!/bin/sh`).
pub fn by_path(path: &str, first_line: Option<&str>) -> Option<&'static SyntaxReference> {
    let syntaxes = SYNTAXES.get()?;
    let name = path.rsplit('/').next().unwrap_or(path);
    let by_name = match name {
        "Makefile" | "makefile" | "GNUmakefile" => syntaxes.find_syntax_by_extension("make"),
        "Dockerfile" => syntaxes.find_syntax_by_name("Dockerfile"),
        _ => None,
    };
    let extension = name.rsplit_once('.').map(|(_, e)| e);
    by_name
        .or_else(|| extension.and_then(|e| syntaxes.find_syntax_by_extension(e)))
        .or_else(|| extension.and_then(by_token))
        .or_else(|| first_line.and_then(|l| syntaxes.find_syntax_by_first_line(l)))
}

/// The shell, for commands.
pub fn shell() -> Option<&'static SyntaxReference> {
    SYNTAXES.get()?.find_syntax_by_extension("sh")
}

/// A colour of a theme as the terminal shows it.
fn colour(c: sh::Color, colouring: Colouring) -> Option<Color> {
    match c.a {
        0 => Some(match c.r {
            0 => Color::Black,
            1 => Color::Red,
            2 => Color::Green,
            3 => Color::Yellow,
            4 => Color::Blue,
            5 => Color::Magenta,
            6 => Color::Cyan,
            7 => Color::Gray,
            8 => Color::DarkGray,
            n => Color::Indexed(n),
        }),
        1 => None,
        _ if colouring == Colouring::TrueColour => Some(Color::Rgb(c.r, c.g, c.b)),
        _ => Some(Color::Indexed(nearest_256(c.r, c.g, c.b))),
    }
}

fn style_of(s: sh::Style, base: Style, colouring: Colouring) -> Style {
    let mut style = base;
    if let Some(fg) = colour(s.foreground, colouring) {
        style = style.fg(fg);
    } else {
        style.fg = None;
    }
    if s.font_style.contains(FontStyle::BOLD) {
        style = style.add_modifier(Modifier::BOLD);
    }
    if s.font_style.contains(FontStyle::ITALIC) {
        style = style.add_modifier(Modifier::ITALIC);
    }
    style
}

/// A text highlighted so far: its lines, and the states after its last
/// complete line, to go on from.
struct Done {
    language: String,
    syntax: Syntax,
    /// The complete lines highlighted, with their newlines.
    text: String,
    lines: Arc<Vec<Vec<Piece>>>,
    parse: ParseState,
    highlight: HighlightState,
}

#[derive(Default)]
struct Cache {
    done: HashMap<u64, (Arc<Done>, u64)>,
    /// The keys of the last texts that may grow, newest last.
    growing: VecDeque<u64>,
    clock: u64,
    /// Lines highlighted since the start, for tests.
    count: u64,
}

static CACHE_STATE: OnceLock<Mutex<Cache>> = OnceLock::new();

fn cache() -> std::sync::MutexGuard<'static, Cache> {
    CACHE_STATE
        .get_or_init(|| Mutex::new(Cache::default()))
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn key(language: &str, syntax: Syntax, text: &str) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    language.hash(&mut hasher);
    format!("{:?}", syntax).hash(&mut hasher);
    text.hash(&mut hasher);
    hasher.finish()
}

/// Lines highlighted so far, for tests of the cache.
#[cfg(test)]
pub fn count() -> u64 {
    cache().count
}

/// Highlights `text` in the grammar `language` (a key of the caller:
/// a fence token or a path), one list of pieces per line. Pieces over
/// `base` take the theme's colours. `None` when it is not highlighted: off,
/// not loaded yet, no grammar, or too long. A last line without its newline
/// is in `base`, until it is complete.
pub fn highlight(
    syntax: Syntax,
    language: &str,
    grammar: Option<&'static SyntaxReference>,
    text: &str,
    base: Style,
) -> Option<Vec<Vec<Piece>>> {
    let theme = theme(syntax.theme)?;
    let grammar = grammar?;
    let syntaxes = SYNTAXES.get()?;
    if text.lines().count() > MAX_LINES {
        return None;
    }
    let complete = match text.rfind('\n') {
        Some(at) => text.get(..=at).unwrap_or_default(),
        None => "",
    };
    let open = text.get(complete.len()..).unwrap_or_default();
    let done = done(syntax, language, grammar, complete, theme, syntaxes, base)?;
    let mut lines = (*done.lines).clone();
    if !open.is_empty() {
        lines.push(vec![(open.to_owned(), base)]);
    }
    Some(lines)
}

/// The complete lines of a text, highlighted: from the cache, by going on
/// from a text it starts with, or from the start.
fn done(
    syntax: Syntax,
    language: &str,
    grammar: &'static SyntaxReference,
    text: &str,
    theme: &'static Theme,
    syntaxes: &'static SyntaxSet,
    base: Style,
) -> Option<Arc<Done>> {
    let id = key(language, syntax, text);
    let start = {
        let mut cache = cache();
        cache.clock += 1;
        let clock = cache.clock;
        if let Some((done, used)) = cache.done.get_mut(&id) {
            *used = clock;
            return Some(done.clone());
        }
        cache
            .growing
            .iter()
            .rev()
            .filter_map(|k| cache.done.get(k).map(|(d, _)| d.clone()))
            .find(|d| d.language == language && d.syntax == syntax && text.starts_with(&d.text))
    };
    let highlighter = sh::Highlighter::new(theme);
    let (mut lines, mut parse, mut state, from) = match &start {
        Some(d) => (
            (*d.lines).clone(),
            d.parse.clone(),
            d.highlight.clone(),
            d.text.len(),
        ),
        None => (
            Vec::new(),
            ParseState::new(grammar),
            HighlightState::new(&highlighter, ScopeStack::new()),
            0,
        ),
    };
    let mut counted = 0;
    for line in text.get(from..).unwrap_or_default().split_inclusive('\n') {
        if line.len() > MAX_LINE {
            return None;
        }
        let ops = parse.parse_line(line, syntaxes).ok()?;
        let pieces: Vec<Piece> = HighlightIterator::new(&mut state, &ops, line, &highlighter)
            .map(|(style, part)| {
                (
                    part.trim_end_matches('\n').to_owned(),
                    style_of(style, base, syntax.colouring),
                )
            })
            .filter(|(part, _)| !part.is_empty())
            .collect();
        lines.push(pieces);
        counted += 1;
    }
    let done = Arc::new(Done {
        language: language.to_owned(),
        syntax,
        text: text.to_owned(),
        lines: Arc::new(lines),
        parse,
        highlight: state,
    });
    let mut cache = cache();
    cache.count += counted;
    let clock = cache.clock;
    cache.done.insert(id, (done.clone(), clock));
    cache.growing.retain(|k| *k != id);
    cache.growing.push_back(id);
    while cache.growing.len() > GROWING {
        cache.growing.pop_front();
    }
    if cache.done.len() > CACHE
        && let Some(oldest) = cache
            .done
            .iter()
            .min_by_key(|(_, (_, used))| *used)
            .map(|(k, _)| *k)
    {
        cache.done.remove(&oldest);
    }
    Some(done)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> Syntax {
        load();
        Syntax {
            theme: CodeTheme::Ansi,
            colouring: Colouring::Ansi256,
        }
    }

    fn rust(text: &str) -> Vec<Vec<Piece>> {
        highlight(on(), "rust", by_token("rust"), text, Style::new()).unwrap()
    }

    fn style_of_word(lines: &[Vec<Piece>], word: &str) -> Style {
        lines
            .iter()
            .flatten()
            .find(|(text, _)| text.contains(word))
            .map(|(_, s)| *s)
            .unwrap_or_else(|| panic!("no {word} in {lines:?}"))
    }

    #[test]
    fn rust_keywords_strings_and_comments() {
        let lines = rust("fn main() {\n    let s = \"hi\"; // note\n}\n");
        assert_eq!(lines.len(), 3);
        assert_eq!(style_of_word(&lines, "fn").fg, Some(Color::Magenta));
        assert_eq!(style_of_word(&lines, "hi").fg, Some(Color::Green));
        let comment = style_of_word(&lines, "note");
        assert_eq!(comment.fg, Some(Color::DarkGray));
        assert!(comment.add_modifier.contains(Modifier::ITALIC));
        // The text is all there, in order.
        let joined: Vec<String> = lines
            .iter()
            .map(|l| l.iter().map(|(t, _)| t.as_str()).collect())
            .collect();
        assert_eq!(joined, ["fn main() {", "    let s = \"hi\"; // note", "}"]);
    }

    #[test]
    fn a_string_over_lines_keeps_its_colour() {
        let lines = rust("let s = \"one\ntwo\";\n");
        assert_eq!(style_of_word(&lines, "two").fg, Some(Color::Green));
    }

    #[test]
    fn plain_when_off_unknown_or_too_long() {
        assert!(
            highlight(
                Syntax::OFF,
                "rust",
                by_token("rust"),
                "fn a() {}\n",
                Style::new()
            )
            .is_none()
        );
        assert!(by_token("no-such-language").is_none());
        assert!(highlight(on(), "x", None, "fn a() {}\n", Style::new()).is_none());
        let long = format!("let a = \"{}\";\n", "x".repeat(MAX_LINE));
        assert!(highlight(on(), "rust", by_token("rust"), &long, Style::new()).is_none());
        let many = "a\n".repeat(MAX_LINES + 1);
        assert!(highlight(on(), "rust", by_token("rust"), &many, Style::new()).is_none());
    }

    #[test]
    fn the_open_line_stays_plain_until_it_is_complete() {
        let base = Style::new().fg(Color::Yellow);
        let lines = highlight(on(), "rust", by_token("rust"), "fn a() {}\nlet", base).unwrap();
        assert_eq!(lines.last().unwrap(), &vec![("let".to_owned(), base)]);
    }

    #[test]
    fn languages_by_token_and_path() {
        on();
        for token in [
            "rust", "rs", "py", "python", "sh", "bash", "js", "json", "yaml", "go", "c",
        ] {
            assert!(by_token(token).is_some(), "{token}");
        }
        assert_eq!(
            by_token("rust {.ignore}").map(|s| s.name.as_str()),
            Some("Rust")
        );
        assert_eq!(
            by_path("src/main.rs", None).map(|s| s.name.as_str()),
            Some("Rust")
        );
        assert_eq!(
            by_path("Makefile", None).map(|s| s.name.as_str()),
            Some("Makefile")
        );
        assert!(by_path("script", Some("#!/bin/bash")).is_some());
        assert!(by_path("notes", Some("hello")).is_none());
        assert!(shell().is_some());
    }

    #[test]
    fn a_growing_text_goes_on_from_where_it_stopped() {
        let syntax = on();
        let text: String = (0..50)
            .map(|i| format!("let x{i} = {i}; // grow\n"))
            .collect();
        let whole = highlight(syntax, "grow", by_token("rust"), &text, Style::new()).unwrap();
        // The same text again: from the cache.
        let before = count();
        let again = highlight(syntax, "grow", by_token("rust"), &text, Style::new()).unwrap();
        assert_eq!(again, whole);
        assert_eq!(count(), before);
        // Line by line, as a block streams: each line once.
        let mut grown = String::new();
        let first = count();
        let mut last = Vec::new();
        for line in text.split_inclusive('\n') {
            grown.push_str(line);
            last = highlight(syntax, "grown", by_token("rust"), &grown, Style::new()).unwrap();
        }
        assert_eq!(last, whole);
        // Other tests may highlight at the same time; at most a little more.
        assert!(count() - first < 50 + 20, "{}", count() - first);
    }

    #[test]
    fn named_themes_and_true_colour() {
        load();
        let syntax = Syntax {
            theme: CodeTheme::Named("base16-ocean.dark"),
            colouring: Colouring::TrueColour,
        };
        let lines = highlight(
            syntax,
            "rust",
            by_token("rust"),
            "fn a() {}\n",
            Style::new(),
        )
        .unwrap();
        assert!(matches!(
            style_of_word(&lines, "fn").fg,
            Some(Color::Rgb(..))
        ));
        let indexed = Syntax {
            colouring: Colouring::Ansi256,
            ..syntax
        };
        let lines = highlight(
            indexed,
            "rust",
            by_token("rust"),
            "fn a() {}\n",
            Style::new(),
        )
        .unwrap();
        assert!(matches!(
            style_of_word(&lines, "fn").fg,
            Some(Color::Indexed(_))
        ));
        // The syntax themes of the bundled themes.
        for name in ["atom-one-dark", "nord", "gruvbox-dark"] {
            let own = Syntax {
                theme: CodeTheme::Named(name),
                ..syntax
            };
            let lines =
                highlight(own, "rust", by_token("rust"), "fn a() {}\n", Style::new()).unwrap();
            assert!(
                matches!(style_of_word(&lines, "fn").fg, Some(Color::Rgb(..))),
                "{name}"
            );
        }
        let one_dark = Syntax {
            theme: CodeTheme::Named("atom-one-dark"),
            ..syntax
        };
        let lines = highlight(
            one_dark,
            "rust",
            by_token("rust"),
            "fn a() {}\n",
            Style::new(),
        )
        .unwrap();
        assert_eq!(
            style_of_word(&lines, "fn").fg,
            Some(Color::Rgb(0xc6, 0x78, 0xdd))
        );
        let unknown = Syntax {
            theme: CodeTheme::Named("no such theme"),
            ..syntax
        };
        assert!(
            highlight(
                unknown,
                "rust",
                by_token("rust"),
                "fn a() {}\n",
                Style::new()
            )
            .is_none()
        );
    }
}
