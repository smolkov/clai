//! The state of the skills box (see `spec/tui.md`, "Skills"): the skills of
//! the workspace from the server, the ticks, the filter and the selected
//! row. `App` routes keys here; `view::skills` draws it.

use std::path::{Path, PathBuf};

use crate::protocol::{ActiveSkill, SkillInfo, SkillsInfo};

use super::editor::{Edit, Editor};

/// A row of the box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Row<'a> {
    /// The heading of a skills directory.
    Heading(&'a Path),
    /// An active skill that is not found any more.
    Missing(&'a ActiveSkill),
    Skill(&'a SkillInfo),
}

impl<'a> Row<'a> {
    /// Whether `Up`, `Down` and `Space` reach it: shadowed skills and
    /// headings are passed over.
    fn selectable(&self) -> bool {
        match self {
            Row::Heading(_) => false,
            Row::Missing(_) => true,
            Row::Skill(s) => s.shadowed_by.is_none(),
        }
    }

    fn name(&self) -> Option<&'a str> {
        match self {
            Row::Heading(_) => None,
            Row::Missing(a) => Some(&a.name),
            Row::Skill(s) => Some(&s.name),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SkillsBox {
    pub info: SkillsInfo,
    /// The ticked names: the active ones in their order, then those ticked
    /// here.
    ticked: Vec<String>,
    /// The active names when the box opened.
    active: Vec<String>,
    pub filter: Editor,
    /// Among the selectable rows the filter lets through.
    selected: usize,
}

impl SkillsBox {
    /// Opens with the session's active skills ticked.
    pub fn new(info: SkillsInfo) -> SkillsBox {
        let active: Vec<String> = info.active.iter().map(|a| a.name.clone()).collect();
        SkillsBox {
            info,
            ticked: active.clone(),
            active,
            filter: Editor::default(),
            selected: 0,
        }
    }

    /// A newer answer: the ticks stay, the selection stays on its name if it
    /// can.
    pub fn set(&mut self, info: SkillsInfo) {
        let chosen = self.chosen().map(str::to_owned);
        self.info = info;
        let names = self.selectable_names();
        self.selected = chosen
            .and_then(|c| names.iter().position(|n| *n == c))
            .unwrap_or(0);
    }

    /// The rows the filter lets through: missing active skills first, then a
    /// heading per directory and its skills, in order of priority.
    pub fn rows(&self) -> Vec<Row<'_>> {
        let filter = self.filter.text().to_lowercase();
        let matches = |name: &str, description: &str| {
            name.to_lowercase().contains(&filter) || description.to_lowercase().contains(&filter)
        };
        let mut rows: Vec<Row<'_>> = self
            .info
            .active
            .iter()
            .filter(|a| a.missing && matches(&a.name, ""))
            .map(Row::Missing)
            .collect();
        let mut group: Option<&Path> = None;
        for skill in &self.info.skills {
            if !matches(&skill.name, &skill.description) {
                continue;
            }
            let parent = skill.dir.parent().unwrap_or(&skill.dir);
            if group != Some(parent) {
                group = Some(parent);
                rows.push(Row::Heading(parent));
            }
            rows.push(Row::Skill(skill));
        }
        rows
    }

    fn selectable_names(&self) -> Vec<String> {
        self.rows()
            .iter()
            .filter(|r| r.selectable())
            .filter_map(|r| r.name().map(str::to_owned))
            .collect()
    }

    /// The index in [`SkillsBox::rows`] of the selected row.
    pub fn selected_row(&self) -> Option<usize> {
        self.rows()
            .iter()
            .enumerate()
            .filter(|(_, r)| r.selectable())
            .nth(self.selected)
            .map(|(i, _)| i)
    }

    /// The selected skill's name; `None` if the filter matches nothing.
    pub fn chosen(&self) -> Option<&str> {
        self.rows().get(self.selected_row()?)?.name()
    }

    pub fn is_ticked(&self, name: &str) -> bool {
        self.ticked.iter().any(|n| n == name)
    }

    /// The selected row's full description, if any.
    pub fn description(&self) -> Option<&str> {
        let name = self.chosen()?;
        self.info
            .skills
            .iter()
            .find(|s| s.name == name && s.shadowed_by.is_none())
            .map(|s| s.description.as_str())
    }

    pub fn has_skills(&self) -> bool {
        !self.info.skills.is_empty() || !self.info.active.is_empty()
    }

    /// The skills to keep on, for `set_skills`.
    pub fn names(&self) -> Vec<String> {
        self.ticked.clone()
    }

    /// Whether the ticks differ from the active skills.
    pub fn changed(&self) -> bool {
        let mut now = self.ticked.clone();
        let mut then = self.active.clone();
        now.sort();
        then.sort();
        now != then
    }

    /// `Space`: tick or untick the selected skill.
    pub fn toggle(&mut self) {
        let Some(name) = self.chosen().map(str::to_owned) else {
            return;
        };
        if self.is_ticked(&name) {
            self.ticked.retain(|n| *n != name);
        } else {
            self.ticked.push(name);
        }
    }

    pub fn up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn down(&mut self) {
        let last = self.selectable_names().len().saturating_sub(1);
        self.selected = (self.selected + 1).min(last);
    }

    /// A key that edits the filter; the selection goes to the first match.
    pub fn edit(&mut self, edit: Edit) {
        if edit != Edit::Newline {
            self.filter.edit(edit);
            self.selected = 0;
        }
    }

    /// Pasted text goes to the filter, as one line.
    pub fn paste(&mut self, text: &str) {
        self.filter.paste(&text.replace(['\r', '\n'], " "));
        self.selected = 0;
    }

    /// The count of active skills after `Enter`.
    pub fn ticked_count(&self) -> usize {
        self.ticked.len()
    }

    /// The directories searched, for the box without skills.
    pub fn dirs(&self) -> &[PathBuf] {
        &self.info.dirs
    }
}
