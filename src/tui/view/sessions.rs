//! The box of the session list (see `spec/tui.md`, "Sessions").

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::agent::State;
use crate::command::ls::age;
use crate::protocol::SessionInfo;
use crate::tui::app::App;
use crate::tui::editor::Editor;
use crate::tui::sessions::{Mode, SessionList};
use crate::tui::style::{Look, Symbols};
use crate::tui::wrap::fit;

use super::popup;

/// The box's width, if the screen has room for it.
const WIDTH: u16 = 76;
/// The rows of the box that are not sessions: the heading, a blank line, the
/// line of the step or a message, and the keys.
const CHROME: usize = 4;
/// Before the name: the pointer, the mark of the attached session (`✓`), a
/// space each.
const LEAD: usize = 4;

fn state(session: &SessionInfo, symbols: &Symbols) -> String {
    match session.state {
        State::Idle => "idle".to_owned(),
        State::Working => "working".to_owned(),
        State::Waiting => format!("{} waiting", symbols.dot),
        State::Saved => "saved".to_owned(),
    }
}

/// The columns of each session, with the heading first. The age, then the
/// workspace, are left out when the box is too narrow for them.
fn table(list: &SessionList, symbols: &Symbols, width: usize) -> Vec<Vec<String>> {
    let mut rows = vec![vec![
        "NAME".to_owned(),
        "WORKSPACE".to_owned(),
        "MODEL".to_owned(),
        "STATE".to_owned(),
        "AGE".to_owned(),
    ]];
    for session in &list.sessions {
        rows.push(vec![
            session.name.clone(),
            session.workspace.clone(),
            session.model.clone(),
            state(session, symbols),
            age(list.now.saturating_sub(session.created)),
        ]);
    }
    let widths = |rows: &[Vec<String>]| -> usize {
        let columns = rows.first().map_or(0, Vec::len);
        (0..columns)
            .map(|i| {
                rows.iter()
                    .filter_map(|r| r.get(i))
                    .map(|c| c.width())
                    .max()
                    .unwrap_or(0)
            })
            .sum::<usize>()
            + 2 * columns.saturating_sub(1)
            + LEAD
    };
    // Age (the last column), then the workspace (the second).
    for drop in [4, 1] {
        if widths(&rows) <= width {
            break;
        }
        for row in &mut rows {
            if drop < row.len() {
                row.remove(drop);
            }
        }
    }
    rows
}

/// Each row padded to its column.
fn aligned(rows: &[Vec<String>]) -> Vec<String> {
    let columns = rows.first().map_or(0, Vec::len);
    let widths: Vec<usize> = (0..columns)
        .map(|i| {
            rows.iter()
                .filter_map(|r| r.get(i))
                .map(|c| c.width())
                .max()
                .unwrap_or(0)
        })
        .collect();
    rows.iter()
        .map(|row| {
            let cells: Vec<String> = row
                .iter()
                .zip(&widths)
                .map(|(cell, w)| format!("{cell}{}", " ".repeat(w.saturating_sub(cell.width()))))
                .collect();
            cells.join("  ").trim_end().to_owned()
        })
        .collect()
}

fn keys(list: &SessionList, look: &Look) -> Line<'static> {
    let rule = look.symbols.rule;
    let text = match &list.mode {
        Mode::Browse => {
            let esc = if list.exit_on_esc { "exit" } else { "back" };
            if list.sessions.is_empty() {
                format!("n new {rule} Esc {esc}")
            } else {
                format!("Enter attach {rule} n new {rule} r rename {rule} k kill {rule} Esc {esc}")
            }
        }
        Mode::ConfirmKill => format!("y kill {rule} n back"),
        Mode::Rename(_) => format!("Enter rename {rule} Esc back"),
        Mode::Workspace(_) => format!("Enter next {rule} Esc back"),
        Mode::Name { .. } => format!("Enter choose the model {rule} Esc back"),
    };
    Line::styled(text, look.styles.dim)
}

/// The label and the editor of a step that asks for text.
fn asking(list: &SessionList) -> Option<(&'static str, &Editor)> {
    match &list.mode {
        Mode::Rename(editor) => Some(("New name: ", editor)),
        Mode::Workspace(editor) => Some(("Workspace: ", editor)),
        Mode::Name { name, .. } => Some(("Name (empty: from the workspace): ", name)),
        Mode::Browse | Mode::ConfirmKill => None,
    }
}

pub fn draw(app: &App, frame: &mut Frame<'_>, size: Rect) {
    let Some(list) = &app.session_list else {
        return;
    };
    let look = &app.settings.look;
    let symbols = &look.symbols;
    let cells = WIDTH.min(size.width.saturating_sub(4));
    let width = usize::from(cells);
    let rows = aligned(&table(list, symbols, width));
    let count = list.sessions.len().max(1);
    let wanted = u16::try_from(count + CHROME).unwrap_or(u16::MAX);
    let area = popup::open(frame, size, " Sessions ", cells, wanted, look);
    let room = usize::from(area.height).saturating_sub(CHROME).max(1);
    let first = (list.selected() + 1).saturating_sub(room);

    let mut rows = rows.into_iter();
    let heading = rows.next().unwrap_or_default();
    let mut lines = vec![Line::styled(
        fit(
            &format!("{}{heading}", " ".repeat(LEAD)),
            width,
            symbols.more,
        ),
        look.styles.heading,
    )];
    if list.sessions.is_empty() {
        lines.push(Line::styled("no sessions", look.styles.dim));
    }
    for (i, (row, session)) in rows.zip(&list.sessions).enumerate().skip(first).take(room) {
        let selected = i == list.selected();
        let pointer = if selected { symbols.pointer } else { " " };
        let mark = if session.id == list.current {
            symbols.done
        } else {
            " "
        };
        let text = fit(&format!("{pointer} {mark} {row}"), width, symbols.more);
        let style = if selected {
            look.styles.selection
        } else if session.state == State::Waiting {
            look.styles.warning
        } else {
            look.styles.text
        };
        lines.push(Line::styled(text, style));
    }
    let bottom = usize::from(area.height).saturating_sub(2);
    lines.resize(bottom, Line::raw(""));
    let mut cursor = None;
    let step = if let Some((label, editor)) = asking(list) {
        let room = width.saturating_sub(label.width()).max(1);
        let (shown, (row, col)) = editor.rows(room);
        let text = shown.get(row).cloned().unwrap_or_default();
        cursor = Some(label.width() + col);
        Line::styled(format!("{label}{text}"), look.styles.text)
    } else if list.mode == Mode::ConfirmKill {
        let name = list.chosen().map_or("", |s| s.name.as_str());
        Line::styled(
            format!("Kill {name} and delete its files? [y] yes  [n] no"),
            look.styles.warning,
        )
    } else if let Some(message) = &list.message {
        Line::styled(fit(message, width, symbols.more), look.styles.error)
    } else {
        Line::raw("")
    };
    lines.push(step);
    lines.push(keys(list, look));
    frame.render_widget(Paragraph::new(lines), area);
    if let Some(cell) = cursor.and_then(|c| u16::try_from(c).ok()) {
        let y = area.y + area.height.saturating_sub(2);
        frame.set_cursor_position((area.x + cell.min(area.width.saturating_sub(1)), y));
    }
}
