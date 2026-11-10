//! The state of the settings box (see `spec/tui.md`, "Settings"): the `[tui]`
//! keys as the user file has them, the values shown and applied, and what is
//! being edited. `App` previews `current` and saves the changes; `view::settings`
//! draws it.

use std::collections::BTreeSet;
use std::ops::RangeInclusive;

use crossterm::event::{KeyCode, KeyEvent};

use crate::config::edit::Setting;
use crate::config::{ActivityLine, Symbols, Tui};

use super::editor::{Edit, Editor};
use super::keys;

/// How a row's value changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kind {
    OnOff,
    Choice(&'static [&'static str]),
    Number(RangeInclusive<i64>),
    /// The theme list opens; `Left` / `Right` step through the themes.
    Theme,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    pub key: &'static str,
    pub description: &'static str,
    /// The line under the list for the selected row.
    pub help: &'static str,
    pub kind: Kind,
    /// Not built yet: the value is kept, but changes nothing.
    pub later: bool,
}

const fn row(key: &'static str, description: &'static str, help: &'static str, kind: Kind) -> Row {
    Row {
        key,
        description,
        help,
        kind,
        later: false,
    }
}

/// The keys of `[tui]`, in the order of `spec/config.md`, "TUI".
pub fn rows() -> [Row; 10] {
    [
        row(
            "theme",
            "Colour theme",
            "Enter opens the theme list; Left / Right step through the themes",
            Kind::Theme,
        ),
        row(
            "symbols",
            "Unicode or ASCII symbols",
            "auto: Unicode when the locale is UTF-8; unicode; ascii",
            Kind::Choice(&["auto", "unicode", "ascii"]),
        ),
        row(
            "mouse",
            "Capture the mouse for scrolling",
            "off keeps the terminal's text selection; on scrolls with the wheel",
            Kind::OnOff,
        ),
        row(
            "tool_output_lines",
            "Lines of tool output before it is cut",
            "0 is no limit; 0 to 1000000",
            Kind::Number(0..=1_000_000),
        ),
        row(
            "expand_tool_calls",
            "Keep finished tool calls expanded",
            "Calls that wait for permission are always expanded",
            Kind::OnOff,
        ),
        Row {
            later: true,
            ..row(
                "syntax_highlight",
                "later version",
                "Highlighting of code blocks and diffs comes with a later version",
                Kind::OnOff,
            )
        },
        row(
            "timestamps",
            "Time of each entry",
            "HH:MM before each new entry",
            Kind::OnOff,
        ),
        row(
            "scroll_lines",
            "Lines per mouse-wheel step",
            "1 to 1000",
            Kind::Number(1..=1000),
        ),
        row(
            "max_fps",
            "Redraw limit",
            "Frames per second, 1 to 120",
            Kind::Number(1..=120),
        ),
        row(
            "activity",
            "Line while the session works",
            "playful: a verb per turn; plain: Thinking / Writing; off: no line",
            Kind::Choice(&["playful", "plain", "off"]),
        ),
    ]
}

fn symbols_name(symbols: Symbols) -> &'static str {
    match symbols {
        Symbols::Auto => "auto",
        Symbols::Unicode => "unicode",
        Symbols::Ascii => "ascii",
    }
}

fn activity_name(activity: ActivityLine) -> &'static str {
    match activity {
        ActivityLine::Playful => "playful",
        ActivityLine::Plain => "plain",
        ActivityLine::Off => "off",
    }
}

/// The value of a key, as the file writes it.
pub fn get(tui: &Tui, key: &str) -> Setting {
    match key {
        "theme" => Setting::Text(tui.theme.clone()),
        "symbols" => Setting::Text(symbols_name(tui.symbols).to_owned()),
        "mouse" => Setting::Bool(tui.mouse),
        "tool_output_lines" => Setting::Int(i64::from(tui.tool_output_lines)),
        "expand_tool_calls" => Setting::Bool(tui.expand_tool_calls),
        "syntax_highlight" => Setting::Bool(tui.syntax_highlight),
        "timestamps" => Setting::Bool(tui.timestamps),
        "scroll_lines" => Setting::Int(i64::from(tui.scroll_lines)),
        "activity" => Setting::Text(activity_name(tui.activity).to_owned()),
        _ => Setting::Int(i64::from(tui.max_fps)),
    }
}

/// Sets a key; a value of the wrong kind or out of range changes nothing.
pub fn set(tui: &mut Tui, key: &str, value: &Setting) {
    match (key, value) {
        ("theme", Setting::Text(t)) => tui.theme.clone_from(t),
        ("symbols", Setting::Text(t)) => {
            tui.symbols = match t.as_str() {
                "unicode" => Symbols::Unicode,
                "ascii" => Symbols::Ascii,
                _ => Symbols::Auto,
            };
        }
        ("activity", Setting::Text(t)) => {
            tui.activity = match t.as_str() {
                "plain" => ActivityLine::Plain,
                "off" => ActivityLine::Off,
                _ => ActivityLine::Playful,
            };
        }
        ("mouse", Setting::Bool(b)) => tui.mouse = *b,
        ("expand_tool_calls", Setting::Bool(b)) => tui.expand_tool_calls = *b,
        ("syntax_highlight", Setting::Bool(b)) => tui.syntax_highlight = *b,
        ("timestamps", Setting::Bool(b)) => tui.timestamps = *b,
        ("tool_output_lines", Setting::Int(n)) => {
            tui.tool_output_lines = u32::try_from(*n).unwrap_or(tui.tool_output_lines);
        }
        ("scroll_lines", Setting::Int(n)) => {
            tui.scroll_lines = u16::try_from(*n).unwrap_or(tui.scroll_lines);
        }
        ("max_fps", Setting::Int(n)) => tui.max_fps = u16::try_from(*n).unwrap_or(tui.max_fps),
        _ => {}
    }
}

/// A value as the box shows it: `on`, `off`, `20`, `nord`.
pub fn shown(value: &Setting) -> String {
    match value {
        Setting::Bool(true) => "on".to_owned(),
        Setting::Bool(false) => "off".to_owned(),
        Setting::Int(n) => n.to_string(),
        Setting::Text(t) => t.clone(),
    }
}

/// What a key in the box asks `App` to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoxAction {
    /// `current` changed: apply it.
    Preview,
    /// Save these changes; `close` once saved.
    Save {
        changes: Vec<(&'static str, Option<Setting>)>,
        close: bool,
    },
    /// Close; `restore` puts the file's values back first.
    Close { restore: bool },
    /// Open the theme list for the `theme` row.
    Themes,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsBox {
    /// The file's values (defaults for keys it does not set).
    pub saved: Tui,
    /// The values shown and applied.
    pub current: Tui,
    /// The keys the file sets.
    pub file_keys: Vec<String>,
    /// Keys set back to their default with `d`: removed from the file.
    reset: BTreeSet<&'static str>,
    selected: usize,
    /// A number being typed.
    pub editing: Option<Editor>,
    /// `Esc` with changes asks first.
    pub confirm_close: bool,
    pub message: Option<String>,
    /// The themes `Left` / `Right` step through.
    themes: Vec<String>,
}

impl SettingsBox {
    pub fn new(saved: Tui, current: Tui, file_keys: Vec<String>, themes: Vec<String>) -> Self {
        SettingsBox {
            saved,
            current,
            file_keys,
            reset: BTreeSet::new(),
            selected: 0,
            editing: None,
            confirm_close: false,
            message: None,
            themes,
        }
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn row(&self) -> Option<Row> {
        rows().get(self.selected).cloned()
    }

    /// Whether the file sets the key.
    pub fn in_file(&self, key: &str) -> bool {
        self.file_keys.iter().any(|k| k == key)
    }

    /// Whether a key differs from the file, or is to be removed from it.
    pub fn changed(&self, key: &str) -> bool {
        get(&self.current, key) != get(&self.saved, key)
            || (self.reset.contains(key) && self.in_file(key))
    }

    /// What `s` writes: removed keys, then the changed ones.
    pub fn changes(&self) -> Vec<(&'static str, Option<Setting>)> {
        rows()
            .iter()
            .filter(|r| self.changed(r.key))
            .map(|r| {
                let value = (!self.reset.contains(r.key)).then(|| get(&self.current, r.key));
                (r.key, value)
            })
            .collect()
    }

    /// After a save: the file has the values shown.
    pub fn saved_now(&mut self) {
        for (key, value) in self.changes() {
            match value {
                Some(_) if !self.in_file(key) => self.file_keys.push(key.to_owned()),
                None => self.file_keys.retain(|k| k != key),
                Some(_) => {}
            }
        }
        self.saved = self.current.clone();
        self.reset.clear();
        self.message = Some("saved".to_owned());
    }

    /// The file changed meanwhile: keys not changed here follow it.
    pub fn reloaded(&mut self, file: Tui, keys: Vec<String>, themes: Vec<String>) {
        for row in rows() {
            if !self.changed(row.key) {
                set(&mut self.current, row.key, &get(&file, row.key));
            }
        }
        self.saved = file;
        self.file_keys = keys;
        self.themes = themes;
    }

    /// Sets the selected theme from the theme list.
    pub fn set_theme(&mut self, name: &str) {
        self.current.theme = name.to_owned();
        self.reset.remove("theme");
    }

    fn set_current(&mut self, key: &'static str, value: &Setting) {
        set(&mut self.current, key, value);
        self.reset.remove(key);
    }

    /// One step of the selected row's value; `forward` or back.
    fn step(&mut self, row: &Row, forward: bool) -> Option<BoxAction> {
        let value = get(&self.current, row.key);
        let next = match (&row.kind, &value) {
            (Kind::OnOff, Setting::Bool(b)) => Setting::Bool(!b),
            (Kind::Choice(values), Setting::Text(t)) => {
                Setting::Text(cycle(values.iter().copied(), t, forward))
            }
            (Kind::Theme, Setting::Text(t)) => {
                Setting::Text(cycle(self.themes.iter().map(String::as_str), t, forward))
            }
            (Kind::Number(range), Setting::Int(n)) => {
                let n = if forward {
                    n.saturating_add(1)
                } else {
                    n.saturating_sub(1)
                };
                Setting::Int(n.clamp(*range.start(), *range.end()))
            }
            _ => return None,
        };
        if next == value {
            return None;
        }
        self.set_current(row.key, &next);
        Some(BoxAction::Preview)
    }

    /// Takes a key; what `App` must do, if anything.
    pub fn key(&mut self, key: KeyEvent) -> Option<BoxAction> {
        let row = self.row()?;
        if self.confirm_close {
            return match key.code {
                KeyCode::Char('s') => {
                    self.confirm_close = false;
                    Some(BoxAction::Save {
                        changes: self.changes(),
                        close: true,
                    })
                }
                KeyCode::Char('n') => Some(BoxAction::Close { restore: true }),
                KeyCode::Esc => {
                    self.confirm_close = false;
                    None
                }
                _ => None,
            };
        }
        if let Some(mut editor) = self.editing.take() {
            return self.edit_number(&row, &mut editor, key);
        }
        self.message = None;
        match key.code {
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            KeyCode::Down => {
                self.selected = (self.selected + 1).min(rows().len() - 1);
                None
            }
            KeyCode::Left => self.step(&row, false),
            KeyCode::Right => self.step(&row, true),
            KeyCode::Enter | KeyCode::Char(' ') => match row.kind {
                Kind::Theme => Some(BoxAction::Themes),
                Kind::Number(_) => {
                    let mut editor = Editor::default();
                    editor.set_text(&shown(&get(&self.current, row.key)));
                    self.editing = Some(editor);
                    None
                }
                Kind::OnOff | Kind::Choice(_) => self.step(&row, true),
            },
            KeyCode::Char('d') => {
                let default = get(&Tui::default(), row.key);
                let was = get(&self.current, row.key);
                set(&mut self.current, row.key, &default);
                self.reset.insert(row.key);
                (was != default).then_some(BoxAction::Preview)
            }
            KeyCode::Char('s') => {
                let changes = self.changes();
                if changes.is_empty() {
                    self.message = Some("nothing to save".to_owned());
                    None
                } else {
                    Some(BoxAction::Save {
                        changes,
                        close: false,
                    })
                }
            }
            KeyCode::Esc if self.changes().is_empty() => Some(BoxAction::Close { restore: false }),
            KeyCode::Esc => {
                self.confirm_close = true;
                None
            }
            _ => None,
        }
    }

    fn edit_number(&mut self, row: &Row, editor: &mut Editor, key: KeyEvent) -> Option<BoxAction> {
        let Kind::Number(range) = &row.kind else {
            return None;
        };
        match key.code {
            KeyCode::Esc => {
                self.message = None;
                None
            }
            KeyCode::Enter => match editor.text().trim().parse::<i64>() {
                Ok(n) if range.contains(&n) => {
                    self.message = None;
                    let changed = get(&self.current, row.key) != Setting::Int(n);
                    self.set_current(row.key, &Setting::Int(n));
                    changed.then_some(BoxAction::Preview)
                }
                _ => {
                    self.message = Some(format!(
                        "{} is not in {}–{}",
                        editor.text().trim(),
                        range.start(),
                        range.end()
                    ));
                    self.editing = Some(editor.clone());
                    None
                }
            },
            _ => {
                if let Some(edit) = keys::edit(key)
                    && (matches!(edit, Edit::Char(c) if c.is_ascii_digit())
                        || !matches!(edit, Edit::Char(_) | Edit::Newline))
                {
                    editor.edit(edit);
                }
                self.editing = Some(editor.clone());
                None
            }
        }
    }
}

/// The value after (or before) `current` among `values`, round the end; the
/// first if `current` is not among them.
fn cycle<'a>(values: impl Iterator<Item = &'a str>, current: &str, forward: bool) -> String {
    let values: Vec<&str> = values.collect();
    let Some(at) = values.iter().position(|v| *v == current) else {
        return values
            .first()
            .map_or_else(|| current.to_owned(), |v| (*v).to_owned());
    };
    let count = values.len();
    let next = if forward {
        (at + 1) % count
    } else {
        (at + count - 1) % count
    };
    values
        .get(next)
        .map_or_else(|| current.to_owned(), |v| (*v).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn settings() -> SettingsBox {
        let saved = Tui {
            mouse: false,
            ..Tui::default()
        };
        let themes = ["terminal", "dark", "light", "nord"]
            .map(String::from)
            .to_vec();
        SettingsBox::new(saved.clone(), saved, vec!["mouse".into()], themes)
    }

    fn select(settings: &mut SettingsBox, key_name: &str) {
        for _ in 0..rows().len() {
            settings.key(key(KeyCode::Up));
        }
        for _ in 0..rows().len() {
            if settings.row().unwrap().key == key_name {
                return;
            }
            settings.key(key(KeyCode::Down));
        }
        panic!("no row {key_name}");
    }

    #[test]
    fn on_off_and_choices_step_round() {
        let mut s = settings();
        select(&mut s, "mouse");
        assert_eq!(s.key(key(KeyCode::Enter)), Some(BoxAction::Preview));
        assert!(s.current.mouse);
        assert_eq!(s.changes(), [("mouse", Some(Setting::Bool(true)))]);
        s.key(key(KeyCode::Char(' ')));
        assert!(s.changes().is_empty());
        select(&mut s, "symbols");
        s.key(key(KeyCode::Up));
        s.key(key(KeyCode::Left));
        assert_eq!(s.current.theme, "nord");
        s.key(key(KeyCode::Down));
        s.key(key(KeyCode::Left));
        assert_eq!(s.current.symbols, Symbols::Ascii);
        s.key(key(KeyCode::Right));
        assert_eq!(s.current.symbols, Symbols::Auto);
    }

    #[test]
    fn the_activity_line_steps_through_its_choices() {
        let mut s = settings();
        select(&mut s, "activity");
        assert_eq!(s.key(key(KeyCode::Enter)), Some(BoxAction::Preview));
        assert_eq!(s.current.activity, ActivityLine::Plain);
        assert_eq!(
            s.changes(),
            [("activity", Some(Setting::Text("plain".into())))]
        );
        s.key(key(KeyCode::Right));
        assert_eq!(s.current.activity, ActivityLine::Off);
        s.key(key(KeyCode::Right));
        assert_eq!(s.current.activity, ActivityLine::Playful);
        assert!(s.changes().is_empty());
    }

    #[test]
    fn numbers_step_within_the_range_and_are_typed() {
        let mut s = settings();
        select(&mut s, "max_fps");
        s.key(key(KeyCode::Right));
        assert_eq!(s.current.max_fps, 31);
        s.key(key(KeyCode::Enter));
        assert!(s.editing.is_some());
        s.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        for c in "5x00".chars() {
            s.key(key(KeyCode::Char(c)));
        }
        // Letters are not taken; 500 is out of range and stays.
        assert_eq!(s.editing.as_ref().unwrap().text(), "500");
        assert_eq!(s.key(key(KeyCode::Enter)), None);
        assert_eq!(s.message.as_deref(), Some("500 is not in 1–120"));
        assert!(s.editing.is_some());
        s.key(key(KeyCode::Backspace));
        assert_eq!(s.key(key(KeyCode::Enter)), Some(BoxAction::Preview));
        assert_eq!(s.current.max_fps, 50);
        assert!(s.editing.is_none());
        // Esc leaves the number as it was.
        s.key(key(KeyCode::Enter));
        s.key(key(KeyCode::Char('9')));
        s.key(key(KeyCode::Esc));
        assert_eq!(s.current.max_fps, 50);
        select(&mut s, "max_fps");
        for _ in 0..200 {
            s.key(key(KeyCode::Right));
        }
        assert_eq!(s.current.max_fps, 120);
    }

    #[test]
    fn d_removes_the_key_from_the_file() {
        let mut s = settings();
        select(&mut s, "mouse");
        assert_eq!(s.key(key(KeyCode::Char('d'))), Some(BoxAction::Preview));
        assert!(s.current.mouse);
        assert_eq!(s.changes(), [("mouse", None)]);
        // A key the file does not set: nothing to remove.
        select(&mut s, "timestamps");
        assert_eq!(s.key(key(KeyCode::Char('d'))), None);
        assert_eq!(s.changes(), [("mouse", None)]);
        s.saved_now();
        assert!(s.changes().is_empty());
        assert!(s.file_keys.is_empty());
        assert_eq!(s.message.as_deref(), Some("saved"));
    }

    #[test]
    fn save_and_close() {
        let mut s = settings();
        assert_eq!(s.key(key(KeyCode::Char('s'))), None);
        assert_eq!(s.message.as_deref(), Some("nothing to save"));
        assert_eq!(
            s.key(key(KeyCode::Esc)),
            Some(BoxAction::Close { restore: false })
        );
        select(&mut s, "timestamps");
        s.key(key(KeyCode::Enter));
        assert_eq!(
            s.key(key(KeyCode::Char('s'))),
            Some(BoxAction::Save {
                changes: vec![("timestamps", Some(Setting::Bool(true)))],
                close: false
            })
        );
        // Esc with changes asks: back, discard, or save and close.
        assert_eq!(s.key(key(KeyCode::Esc)), None);
        assert!(s.confirm_close);
        assert_eq!(s.key(key(KeyCode::Esc)), None);
        assert!(!s.confirm_close);
        s.key(key(KeyCode::Esc));
        assert_eq!(
            s.key(key(KeyCode::Char('n'))),
            Some(BoxAction::Close { restore: true })
        );
        let mut s2 = s.clone();
        s2.confirm_close = true;
        assert!(matches!(
            s2.key(key(KeyCode::Char('s'))),
            Some(BoxAction::Save { close: true, .. })
        ));
        // The theme row opens the theme list.
        s.confirm_close = false;
        select(&mut s, "theme");
        assert_eq!(s.key(key(KeyCode::Enter)), Some(BoxAction::Themes));
        s.set_theme("light");
        assert_eq!(
            s.changes(),
            [
                ("theme", Some(Setting::Text("light".into()))),
                ("timestamps", Some(Setting::Bool(true)))
            ]
        );
    }

    #[test]
    fn a_reload_keeps_what_was_changed_here() {
        let mut s = settings();
        select(&mut s, "timestamps");
        s.key(key(KeyCode::Enter));
        let file = Tui {
            mouse: true,
            max_fps: 60,
            ..Tui::default()
        };
        s.reloaded(file, vec!["mouse".into(), "max_fps".into()], Vec::new());
        assert!(s.current.timestamps);
        assert!(s.current.mouse);
        assert_eq!(s.current.max_fps, 60);
        assert_eq!(s.changes(), [("timestamps", Some(Setting::Bool(true)))]);
    }

    #[test]
    fn values_as_shown() {
        let tui = Tui::default();
        let shown: Vec<String> = rows().iter().map(|r| shown(&get(&tui, r.key))).collect();
        assert_eq!(
            shown,
            [
                "terminal", "auto", "on", "20", "off", "on", "off", "3", "30", "playful"
            ]
        );
        let mut tui = Tui::default();
        set(&mut tui, "mouse", &Setting::Int(1));
        assert!(tui.mouse, "a value of the wrong kind changes nothing");
        assert_eq!(cycle(["a", "b"].into_iter(), "x", true), "a");
        assert_eq!(cycle(std::iter::empty(), "x", true), "x");
    }
}
