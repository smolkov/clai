//! The state of the theme list (see `spec/tui.md`, "Theme list"): every theme
//! in groups, the filter and the selected row. `App` previews the selected
//! theme and saves the one taken; `view::themes` draws it.

use indexmap::IndexMap;

use crate::config::{BuiltinTheme, Theme, ThemeSource, ThemeWarning, themes::BUNDLED};

use super::editor::{Edit, Editor};

/// Where a theme comes from, in the order of the list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    BuiltIn,
    Bundled,
    Files,
    UserFile,
}

impl Group {
    pub fn heading(self) -> &'static str {
        match self {
            Group::BuiltIn => "built in",
            Group::Bundled => "bundled",
            Group::Files => "theme files",
            Group::UserFile => "user file",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub group: Group,
    /// Where it is from or what it builds on: `base dark`, a file's path.
    pub note: String,
    /// Why a theme file cannot be used.
    pub broken: Option<String>,
}

/// All themes, in the order of the list: built in, bundled (a file of the
/// same name in its place, with its path), theme files by name, tables of the
/// user file in file order, and broken files with their error.
pub fn entries(themes: &IndexMap<String, Theme>, warnings: &[ThemeWarning]) -> Vec<Entry> {
    let mut entries: Vec<Entry> = ["terminal", "dark", "light"]
        .iter()
        .map(|name| Entry {
            name: (*name).to_owned(),
            group: Group::BuiltIn,
            note: String::new(),
            broken: None,
        })
        .collect();
    let base = |theme: &Theme| match theme.base {
        BuiltinTheme::Terminal => "base terminal",
        BuiltinTheme::Dark => "base dark",
        BuiltinTheme::Light => "base light",
    };
    for (name, theme) in themes {
        let bundled = BUNDLED.iter().any(|(b, _)| b == name);
        let (group, note) = match &theme.source {
            ThemeSource::Bundled => (Group::Bundled, base(theme).to_owned()),
            ThemeSource::File(path) if bundled => (Group::Bundled, path.display().to_string()),
            ThemeSource::File(path) => (Group::Files, path.display().to_string()),
            ThemeSource::Table => (Group::UserFile, base(theme).to_owned()),
        };
        entries.push(Entry {
            name: name.clone(),
            group,
            note,
            broken: None,
        });
    }
    for warning in warnings {
        entries.push(Entry {
            name: warning.name.clone(),
            group: Group::Files,
            note: String::new(),
            broken: Some(warning.message.clone()),
        });
    }
    // Stable: the order within a group stays.
    entries.sort_by_key(|e| e.group);
    entries
}

#[derive(Debug, Clone, PartialEq)]
pub struct ThemeList {
    entries: Vec<Entry>,
    pub filter: Editor,
    /// Among the visible entries.
    selected: usize,
    /// The theme in effect when the list opened; `Esc` goes back to it.
    pub original: String,
    /// Opened from the settings box: `Enter` only sets its row.
    pub for_settings: bool,
}

impl ThemeList {
    /// Starts on the theme in effect.
    pub fn new(entries: Vec<Entry>, current: &str, for_settings: bool) -> ThemeList {
        let selected = entries.iter().position(|e| e.name == current).unwrap_or(0);
        ThemeList {
            entries,
            filter: Editor::default(),
            selected,
            original: current.to_owned(),
            for_settings,
        }
    }

    /// Entries read again: the same theme stays selected if it can.
    pub fn set_entries(&mut self, entries: Vec<Entry>) {
        let chosen = self.chosen().map(|e| e.name.clone());
        self.entries = entries;
        let visible = self.visible();
        let at = chosen.and_then(|c| visible.iter().position(|e| e.name == c));
        self.selected = at.unwrap_or(0);
    }

    /// The entries the filter lets through, by name, in any case.
    pub fn visible(&self) -> Vec<&Entry> {
        let filter = self.filter.text().to_lowercase();
        self.entries
            .iter()
            .filter(|e| e.name.to_lowercase().contains(&filter))
            .collect()
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// The selected entry; `None` if the filter matches nothing.
    pub fn chosen(&self) -> Option<&Entry> {
        self.visible().get(self.selected).copied()
    }

    /// The theme to preview: the selected one, unless it is broken.
    pub fn preview(&self) -> Option<&str> {
        self.chosen()
            .filter(|e| e.broken.is_none())
            .map(|e| e.name.as_str())
    }

    pub fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn down(&mut self) {
        let last = self.visible().len().saturating_sub(1);
        self.selected = (self.selected + 1).min(last);
    }

    /// A key that edits the filter; the selection goes to the first match.
    pub fn edit(&mut self, edit: Edit) {
        if edit != Edit::Newline {
            self.filter.edit(edit);
            self.selected = 0;
        }
    }

    pub fn paste(&mut self, text: &str) {
        self.filter.paste(&text.replace(['\r', '\n'], " "));
        self.selected = 0;
    }
}

/// Whether `name` is a theme that can be used now.
pub fn usable(name: &str, themes: &IndexMap<String, Theme>) -> bool {
    BuiltinTheme::from_name(name).is_some() || themes.contains_key(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::themes::file_themes;
    use crate::config::{Colour, ThemeText};

    fn themes() -> IndexMap<String, Theme> {
        let files = [
            ThemeText {
                name: "nord".into(),
                path: "/c/themes/nord.toml".into(),
                text: Ok("base = \"dark\"".into()),
            },
            ThemeText {
                name: "mine".into(),
                path: "/c/themes/mine.toml".into(),
                text: Ok("base = \"light\"".into()),
            },
        ];
        let mut themes = file_themes(&files).0;
        themes.insert(
            "solar".into(),
            Theme {
                base: BuiltinTheme::Dark,
                colours: vec![("user", Colour::Named("cyan"))],
                code: None,
                source: ThemeSource::Table,
            },
        );
        themes
    }

    fn warnings() -> Vec<ThemeWarning> {
        vec![ThemeWarning {
            name: "bad".into(),
            message: "/c/themes/bad.toml:1: base: …".into(),
        }]
    }

    #[test]
    fn groups_in_order() {
        let entries = entries(&themes(), &warnings());
        let rows: Vec<(&str, Group, &str)> = entries
            .iter()
            .map(|e| (e.name.as_str(), e.group, e.note.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                ("terminal", Group::BuiltIn, ""),
                ("dark", Group::BuiltIn, ""),
                ("light", Group::BuiltIn, ""),
                ("atom-one-dark", Group::Bundled, "base dark"),
                ("nord", Group::Bundled, "/c/themes/nord.toml"),
                ("gruvbox-dark", Group::Bundled, "base dark"),
                ("solarized-light", Group::Bundled, "base light"),
                ("mine", Group::Files, "/c/themes/mine.toml"),
                ("bad", Group::Files, ""),
                ("solar", Group::UserFile, "base dark"),
            ]
        );
        assert!(
            entries
                .iter()
                .find(|e| e.name == "bad")
                .unwrap()
                .broken
                .is_some()
        );
    }

    #[test]
    fn the_selection_previews_but_not_a_broken_file() {
        let mut list = ThemeList::new(entries(&themes(), &warnings()), "nord", false);
        assert_eq!(list.preview(), Some("nord"));
        assert_eq!(list.original, "nord");
        for c in "ba".chars() {
            list.edit(Edit::Char(c));
        }
        assert_eq!(list.chosen().unwrap().name, "bad");
        assert_eq!(list.preview(), None);
        for c in "zz".chars() {
            list.edit(Edit::Char(c));
        }
        assert!(list.chosen().is_none());
        // Up and down stop at the ends.
        let mut list = ThemeList::new(entries(&themes(), &[]), "terminal", false);
        list.up();
        assert_eq!(list.preview(), Some("terminal"));
        for _ in 0..20 {
            list.down();
        }
        assert_eq!(list.preview(), Some("solar"));
    }

    #[test]
    fn usable_themes() {
        let themes = themes();
        assert!(usable("dark", &themes));
        assert!(usable("solar", &themes));
        assert!(!usable("bad", &themes));
    }
}
