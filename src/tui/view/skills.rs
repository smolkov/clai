//! The skills box (see `spec/tui.md`, "Skills").

use std::path::Path;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::tui::app::App;
use crate::tui::skills::{Row, SkillsBox};
use crate::tui::style::Look;
use crate::tui::wrap::fit;

use super::popup;

/// The box's width, if the screen has room for it.
const WIDTH: u16 = 78;
/// The rows that are not skills: filter and a blank line; the full
/// description, the status line and the keys.
const CHROME: usize = 5;
const FILTER: &str = "Filter: ";

const EXAMPLE: &str = "---
name: release-check
description: Steps before a release. Use when asked to release.
---

# Release check

1. Run the tests.";

/// A path as the box shows it: relative to the workspace, `~/…`, or as it
/// is.
fn shown(app: &App, skills: &SkillsBox, path: &Path) -> String {
    crate::skills::display(path, &app.session.path, skills.info.home.as_deref())
}

/// The rows, and which line holds the selected one.
fn list(app: &App, skills: &SkillsBox, width: usize) -> (Vec<Line<'static>>, usize) {
    let look = &app.settings.look;
    let more = look.symbols.more;
    let rows = skills.rows();
    let selected = skills.selected_row();
    let name_width = rows
        .iter()
        .filter_map(|r| match r {
            Row::Missing(a) => Some(a.name.width()),
            Row::Skill(s) => Some(s.name.width()),
            Row::Heading(_) => None,
        })
        .max()
        .unwrap_or(0);
    let mut lines = Vec::new();
    let mut at = 0;
    for (i, row) in rows.iter().enumerate() {
        let is_selected = selected == Some(i);
        let pointer = if is_selected {
            look.symbols.pointer
        } else {
            " "
        };
        let tick = |name: &str| if skills.is_ticked(name) { "[x]" } else { "[ ]" };
        let (text, style) = match row {
            Row::Heading(dir) => {
                lines.push(Line::styled(
                    fit(&shown(app, skills, dir), width, more),
                    look.styles.heading,
                ));
                continue;
            }
            Row::Missing(active) => (
                format!(
                    "{pointer} {} {:<name_width$}   not found any more",
                    tick(&active.name),
                    active.name
                ),
                look.styles.warning,
            ),
            Row::Skill(skill) => match &skill.shadowed_by {
                Some(by) => (
                    format!(
                        "      {:<name_width$}   shadowed by {}",
                        skill.name,
                        shown(app, skills, by)
                    ),
                    look.styles.dim,
                ),
                None => {
                    let changed = skills
                        .info
                        .active
                        .iter()
                        .any(|a| a.name == skill.name && a.changed_on_disk);
                    let about = if changed {
                        format!(
                            "changed on disk {} {}",
                            look.symbols.separator, skill.description
                        )
                    } else {
                        skill.description.clone()
                    };
                    let style = if skills.is_ticked(&skill.name) {
                        look.styles.accent
                    } else {
                        look.styles.text
                    };
                    (
                        format!(
                            "{pointer} {} {:<name_width$}   {about}",
                            tick(&skill.name),
                            skill.name
                        ),
                        style,
                    )
                }
            },
        };
        let style = if is_selected {
            at = lines.len();
            look.styles.selection
        } else {
            style
        };
        lines.push(Line::styled(fit(text.trim_end(), width, more), style));
    }
    (lines, at)
}

/// The line under the list: the count of active skills, and the warnings.
fn status(skills: &SkillsBox, width: usize, look: &Look) -> Line<'static> {
    let rule = look.symbols.rule;
    let mut text = format!("{} active", skills.ticked_count());
    let warnings = &skills.info.warnings;
    if let Some(first) = warnings.first() {
        text.push_str(&format!(
            " {rule} {} {} skipped: {first}",
            look.symbols.warning,
            warnings.len()
        ));
    }
    let style = if warnings.is_empty() {
        look.styles.dim
    } else {
        look.styles.warning
    };
    Line::styled(fit(&text, width, look.symbols.more), style)
}

fn keys(look: &Look) -> Line<'static> {
    let rule = look.symbols.rule;
    let text = format!("Space toggle {rule} Enter apply {rule} Esc cancel");
    Line::styled(text, look.styles.dim)
}

/// The box without skills: where they were looked for, and an example.
fn no_skills(app: &App, skills: &SkillsBox, width: usize, look: &Look) -> Vec<Line<'static>> {
    let more = look.symbols.more;
    let mut lines = vec![Line::raw(
        "No skills found. Put a directory with a SKILL.md in one of:",
    )];
    for dir in skills.dirs() {
        lines.push(Line::raw(fit(
            &format!("  {}", shown(app, skills, dir)),
            width,
            more,
        )));
    }
    lines.push(Line::raw(""));
    lines.extend(
        EXAMPLE
            .lines()
            .map(|l| Line::styled(l.to_owned(), look.styles.dim)),
    );
    for warning in &skills.info.warnings {
        lines.push(Line::styled(
            fit(&format!("{} {warning}", look.symbols.warning), width, more),
            look.styles.warning,
        ));
    }
    lines.push(Line::raw(""));
    lines.push(Line::styled("Esc close", look.styles.dim));
    lines
}

pub fn draw(app: &App, frame: &mut Frame<'_>, size: Rect) {
    let Some(skills) = &app.skills_box else {
        return;
    };
    let look = &app.settings.look;
    let title = format!(" Skills for \"{}\" ", app.session.name);
    let cells = WIDTH.min(size.width.saturating_sub(4));
    let width = usize::from(cells);
    if !skills.has_skills() {
        let lines = no_skills(app, skills, width, look);
        let rows = u16::try_from(lines.len()).unwrap_or(u16::MAX);
        let area = popup::open(frame, size, &title, cells, rows, look);
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }
    let (rows, selected) = list(app, skills, width);
    let wanted = u16::try_from(rows.len().max(1) + CHROME).unwrap_or(u16::MAX);
    let area = popup::open(frame, size, &title, cells, wanted, look);
    let room = usize::from(area.height).saturating_sub(CHROME).max(1);
    let first = (selected + 1).saturating_sub(room);

    let filter_width = width.saturating_sub(FILTER.len()).max(1);
    let (filter_rows, (cursor_row, cursor_cell)) = skills.filter.rows(filter_width);
    let typed = filter_rows.get(cursor_row).cloned().unwrap_or_default();
    let mut lines = vec![Line::styled(format!("{FILTER}{typed}"), look.styles.text)];
    lines.push(Line::raw(""));
    if rows.is_empty() {
        lines.push(Line::styled("no skill", look.styles.dim));
    }
    lines.extend(rows.into_iter().skip(first).take(room));
    // Under the list: the selected row's description when the row cuts it,
    // the status line and the keys.
    lines.resize(area.height.saturating_sub(3).into(), Line::raw(""));
    let full = skills.description().filter(|d| {
        let name = skills.chosen().unwrap_or("");
        // `▶ [x] ` before the name, three spaces after the padded name.
        6 + name.width() + 3 + d.width() > width
    });
    lines.push(match full {
        Some(text) => Line::styled(fit(text, width, look.symbols.more), look.styles.dim),
        None => Line::raw(""),
    });
    lines.push(status(skills, width, look));
    lines.push(keys(look));
    frame.render_widget(Paragraph::new(lines), area);
    if let Ok(cell) = u16::try_from(FILTER.len() + cursor_cell) {
        frame.set_cursor_position((area.x + cell, area.y));
    }
}
