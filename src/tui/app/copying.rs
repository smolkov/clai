//! Copying and the details view (see `spec/tui.md`, "Copy" and "Details
//! view"): the copy keys and commands, the mouse selection, clicks on calls,
//! and the keys of the details view.

use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::config::Clipboard;
use crate::tui::keys::Focus;
use crate::tui::selection::{self, Selection, Surface};

use super::{App, Effect};

/// How long `copied 12 lines` stays in the footer.
pub const FLASH: Duration = Duration::from_secs(2);
/// Clicks closer than this count as a double or triple click.
const MULTI_CLICK: Duration = Duration::from_millis(400);

impl App {
    /// Puts a message in the footer that goes after [`FLASH`].
    pub(super) fn flash(&mut self, text: impl Into<String>) {
        self.message = Some(text.into());
        self.message_at = Some(self.now);
    }

    /// Clears a message of [`App::flash`] that is old enough; `true` if one
    /// went.
    pub(super) fn expire_message(&mut self) -> bool {
        match self.message_at {
            Some(at) if self.now.duration_since(at) >= FLASH => {
                self.message = None;
                self.message_at = None;
                true
            }
            _ => false,
        }
    }

    /// Copies `text`, and says so in the footer.
    pub(super) fn copy_out(&mut self, text: String) -> Vec<Effect> {
        if text.trim().is_empty() {
            self.flash("nothing to copy");
            return Vec::new();
        }
        if self.settings.tui.clipboard == Clipboard::Off {
            self.flash("copying is off (tui.clipboard)");
            return Vec::new();
        }
        let lines = text.trim_end_matches('\n').lines().count().max(1);
        self.flash(if lines == 1 {
            "copied 1 line".to_owned()
        } else {
            format!("copied {lines} lines")
        });
        vec![Effect::Copy(text)]
    }

    /// `y`: the selected entry.
    pub(super) fn copy_selected(&mut self) -> Vec<Effect> {
        match self.selected.and_then(|i| self.conversation.entry_text(i)) {
            Some(text) => self.copy_out(text),
            None => Vec::new(),
        }
    }

    /// `Y`: the last code block of the selected answer, or of the last one.
    pub(super) fn copy_code_of_selected(&mut self) -> Vec<Effect> {
        let entry = self
            .selected
            .filter(|i| !self.conversation.code_blocks(*i).is_empty())
            .or_else(|| self.conversation.answers().first().copied());
        let block = entry.and_then(|i| self.conversation.code_blocks(i).pop());
        match block {
            Some(block) => self.copy_out(block),
            None => {
                self.flash("no code block");
                Vec::new()
            }
        }
    }

    /// `/copy [N|code]`, `Ctrl-Y`: an answer, `back` before the last, or the
    /// last code block of the last answer.
    pub(super) fn copy_command(&mut self, argument: &str) -> Option<Vec<Effect>> {
        let answers = self.conversation.answers();
        if argument == "code" {
            let block = answers
                .first()
                .and_then(|i| self.conversation.code_blocks(*i).pop());
            return Some(match block {
                Some(block) => self.copy_out(block),
                None => {
                    self.conversation
                        .notice("the last answer has no code block");
                    Vec::new()
                }
            });
        }
        let back = if argument.is_empty() {
            1
        } else {
            match argument.parse::<usize>() {
                Ok(n) if n >= 1 => n,
                _ => return None,
            }
        };
        let text = answers
            .get(back - 1)
            .and_then(|i| self.conversation.entry_text(*i));
        Some(match text {
            Some(text) => self.copy_out(text),
            None => {
                self.conversation.notice(if answers.is_empty() {
                    "no answer to copy yet".to_owned()
                } else {
                    format!("only {} answers", answers.len())
                });
                Vec::new()
            }
        })
    }

    /// `o`, or a click on `… N more lines`: the call in the details view.
    pub(super) fn open_details(&mut self, entry: usize) -> bool {
        match self.conversation.details(entry) {
            Some(details) => {
                self.clear_selection();
                self.details = Some(details);
                true
            }
            None => false,
        }
    }

    /// `/diff`.
    pub(super) fn open_diff(&mut self) {
        match self.conversation.diff_details() {
            Some(details) => {
                self.clear_selection();
                self.details = Some(details);
            }
            None => self.conversation.notice("the last turn changed no file"),
        }
    }

    /// A key while the details view is open.
    pub(super) fn details_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let mut effects = vec![Effect::Redraw];
        let Some(details) = &mut self.details else {
            return effects;
        };
        if let Some(typed) = &mut details.typing {
            match key.code {
                KeyCode::Esc => details.typing = None,
                KeyCode::Enter => {
                    details.search = details.typing.take();
                    details.found = None;
                    details.find(false);
                }
                KeyCode::Backspace => {
                    typed.pop();
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    typed.push(c);
                }
                _ => {}
            }
            return effects;
        }
        let page = isize::try_from(details.height.saturating_sub(1).max(1)).unwrap_or(1);
        details.missed = false;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.details = None,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.details = None;
            }
            KeyCode::Up | KeyCode::Char('k') => details.scroll(-1),
            KeyCode::Down | KeyCode::Char('j') => details.scroll(1),
            KeyCode::PageUp => details.scroll(-page),
            KeyCode::PageDown | KeyCode::Char(' ') => details.scroll(page),
            KeyCode::Home => details.top = 0,
            KeyCode::End => details.top = details.max_top(),
            KeyCode::Left => details.side(-8),
            KeyCode::Right => details.side(8),
            KeyCode::Char('w') => details.toggle_wrap(),
            KeyCode::Char('/') => details.typing = Some(String::new()),
            KeyCode::Char('n') => details.find(false),
            KeyCode::Char('N') => details.find(true),
            KeyCode::Char('y') => {
                let text = details.text();
                effects.extend(self.copy_out(text));
            }
            _ => {}
        }
        effects
    }

    /// Ends the selection; the conversation follows its end again if it did
    /// before.
    pub(super) fn clear_selection(&mut self) {
        if let Some(selection) = self.selection.take()
            && selection.followed
        {
            self.follow = true;
        }
    }

    /// The surface, row and cell of a screen position, if it is on the
    /// conversation or the open details view. Rows count from the start of
    /// the surface.
    fn point(&self, column: u16, row: u16) -> Option<(Surface, (usize, usize))> {
        let inside = |area: Rect| {
            column >= area.x
                && column < area.x + area.width
                && row >= area.y
                && row < area.y + area.height
        };
        let at = |area: Rect, top: usize| {
            (
                top + usize::from(row - area.y),
                usize::from(column - area.x),
            )
        };
        match &self.details {
            Some(details) => inside(self.details_area)
                .then(|| (Surface::Details, at(self.details_area, details.top))),
            None => inside(self.conversation_area)
                .then(|| (Surface::Conversation, at(self.conversation_area, self.top))),
        }
    }

    /// While a drag goes above or below the surface, it scrolls; the row
    /// under the mouse is then the first or last one shown.
    fn drag_point(&mut self, column: u16, row: u16) -> Option<(usize, usize)> {
        let (area, surface) = match &self.details {
            Some(_) => (self.details_area, Surface::Details),
            None => (self.conversation_area, Surface::Conversation),
        };
        if area.height == 0 {
            return None;
        }
        let cell = usize::from(
            column
                .saturating_sub(area.x)
                .min(area.width.saturating_sub(1)),
        );
        let above = row < area.y;
        let below = row >= area.y + area.height;
        match surface {
            Surface::Details => {
                let details = self.details.as_mut()?;
                if above {
                    details.scroll(-1);
                } else if below {
                    details.scroll(1);
                }
                let shown = usize::from(row.clamp(area.y, area.y + area.height - 1) - area.y);
                Some((details.top + shown, cell))
            }
            Surface::Conversation => {
                if above {
                    self.scroll_up(1);
                } else if below {
                    self.scroll_down(1);
                }
                let shown = usize::from(row.clamp(area.y, area.y + area.height - 1) - area.y);
                Some((self.top + shown, cell))
            }
        }
    }

    /// The left button: a drag selects, a release copies what was dragged
    /// over; a click toggles a call or opens its details; a double click
    /// selects a word, a triple click a line.
    pub(super) fn mouse_button(&mut self, mouse: MouseEvent) -> Vec<Effect> {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.clear_selection();
                let Some((surface, at)) = self.point(mouse.column, mouse.row) else {
                    return vec![Effect::Redraw];
                };
                let clicks = match self.last_click {
                    Some((when, x, y, n))
                        if self.now.duration_since(when) < MULTI_CLICK
                            && (x, y) == (mouse.column, mouse.row) =>
                    {
                        n % 3 + 1
                    }
                    _ => 1,
                };
                self.last_click = Some((self.now, mouse.column, mouse.row, clicks));
                if clicks > 1 {
                    return self.select_by_clicks(surface, at, clicks);
                }
                self.selection = Some(Selection::new(surface, at));
                vec![Effect::Redraw]
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.selection.is_none() {
                    return Vec::new();
                }
                let Some(at) = self.drag_point(mouse.column, mouse.row) else {
                    return Vec::new();
                };
                let follow = self.follow;
                let Some(selection) = &mut self.selection else {
                    return Vec::new();
                };
                if !selection.dragged && selection.surface == Surface::Conversation {
                    // The text does not move under the mouse.
                    selection.followed = follow;
                }
                selection.dragged = true;
                selection.head = at;
                if selection.surface == Surface::Conversation {
                    self.follow = false;
                }
                vec![Effect::Redraw]
            }
            MouseEventKind::Up(MouseButton::Left) => {
                let Some(selection) = self.selection else {
                    return Vec::new();
                };
                if selection.dragged {
                    let (from, to) = selection.range();
                    let text = self.selected_text(selection.surface, from, to);
                    let mut effects = self.copy_out(text);
                    effects.push(Effect::Redraw);
                    return effects;
                }
                self.selection = None;
                if selection.surface == Surface::Conversation {
                    self.click(selection.anchor.0);
                }
                vec![Effect::Redraw]
            }
            _ => Vec::new(),
        }
    }

    fn selected_text(&self, surface: Surface, from: (usize, usize), to: (usize, usize)) -> String {
        match surface {
            Surface::Conversation => self.conversation.copy(from, to),
            Surface::Details => self
                .details
                .as_ref()
                .map_or_else(String::new, |d| d.copy(from, to)),
        }
    }

    /// A click on a row of the conversation: a call's summary toggles it,
    /// its `… N more lines` opens its details. The focus stays.
    fn click(&mut self, row: usize) {
        let Some(found) = self.conversation.rows(row, 1).first().copied() else {
            return;
        };
        let Some(entry) = found.entry else {
            return;
        };
        if self.conversation.call(entry).is_none() {
            return;
        }
        if found.first {
            if self.conversation.toggle(entry) {
                self.reveal = Some(entry);
            }
            if self.focus == Focus::Conversation {
                self.selected = Some(entry);
            }
        } else if found.source.is_some_and(|s| s.skip) {
            self.open_details(entry);
        }
    }

    /// A double click selects the word under the mouse, a triple click the
    /// line with its wrapped rows; either is copied at once.
    fn select_by_clicks(
        &mut self,
        surface: Surface,
        (row, cell): (usize, usize),
        clicks: u8,
    ) -> Vec<Effect> {
        let rows: Vec<(ratatui::text::Line<'static>, crate::tui::wrap::Source)> = match surface {
            Surface::Conversation => {
                let start = row.saturating_sub(200);
                self.conversation
                    .rows(start, row - start + 200)
                    .iter()
                    .map(|r| {
                        (
                            r.line.cloned().unwrap_or_default(),
                            r.source.copied().unwrap_or_default(),
                        )
                    })
                    .collect()
            }
            Surface::Details => self
                .details
                .as_ref()
                .map(|d| d.rows().to_vec())
                .unwrap_or_default(),
        };
        let base = match surface {
            Surface::Conversation => row.saturating_sub(200),
            Surface::Details => 0,
        };
        let Some(index) = row.checked_sub(base) else {
            return Vec::new();
        };
        let Some((line, _)) = rows.get(index) else {
            return Vec::new();
        };
        let (from, to) = if clicks == 2 {
            let Some((first, last)) = selection::word(line, cell) else {
                return vec![Effect::Redraw];
            };
            ((row, first), (row, last))
        } else {
            let mut first = index;
            while first > 0 && rows.get(first).is_some_and(|(_, s)| s.joined) {
                first -= 1;
            }
            let mut last = index;
            while rows.get(last + 1).is_some_and(|(_, s)| s.joined) {
                last += 1;
            }
            ((base + first, 0), (base + last, usize::MAX - 1))
        };
        let mut selection = Selection::new(surface, from);
        selection.head = to;
        selection.dragged = true;
        self.selection = Some(selection);
        let (start, end) = selection.range();
        let text = self.selected_text(surface, start, end);
        let mut effects = self.copy_out(text);
        effects.push(Effect::Redraw);
        effects
    }

    /// Any key ends a selection; `true` if there was one.
    pub(super) fn key_clears_selection(&mut self) -> bool {
        if self.selection.is_some() {
            self.clear_selection();
            return true;
        }
        false
    }
}
