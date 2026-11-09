//! The state of the session list (see `spec/tui.md`, "Sessions"): the sessions
//! from the server's last `list`, the selected row, the marks, the filter and
//! what the box asks for. `App` routes keys here and carries out what comes
//! back; `view::sessions` draws it.

mod filter;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};

use crate::protocol::SessionInfo;

use super::editor::{Edit, Editor};
use super::keys;

pub use filter::Filter;

/// What the box is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Browse,
    /// `r`: the new name of the selected session.
    Rename(Editor),
    /// `d`: `y` / `n` for these sessions, in list order.
    ConfirmDelete(Vec<String>),
    /// `/`: the filter text, applied as it is typed.
    Filter(Editor),
    /// `n`, first step: the workspace.
    Workspace(Editor),
    /// `n`, second step: the name; empty for the server's default.
    Name {
        workspace: PathBuf,
        name: Editor,
    },
}

/// What a key in the list asks `App` to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListAction {
    Close,
    /// `Esc` where it leaves the TUI.
    Exit,
    Attach(String),
    Rename {
        session: String,
        name: String,
    },
    /// One `kill` per name.
    Delete(Vec<String>),
    /// Go on to the model picker for a new session.
    New {
        workspace: PathBuf,
        name: Option<String>,
    },
}

/// Deletes sent and not all answered yet.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Deleting {
    open: Vec<String>,
    done: usize,
    /// `docs: no session named docs`.
    failed: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionList {
    pub sessions: Vec<SessionInfo>,
    selected: usize,
    pub mode: Mode,
    /// The id of the attached session, with a `✓` in the list.
    pub current: u64,
    /// The attached session's workspace, the default for a new one: as shown
    /// (`~/ws/project`) and absolute.
    shown: String,
    base: PathBuf,
    /// An error for the box's last line, e.g. a refused name.
    pub message: Option<String>,
    /// A result for the box's last line, e.g. `deleted 5 sessions`.
    pub done: Option<String>,
    /// `Esc` leaves the TUI instead of closing the list: at start without a
    /// session chosen, and after the attached session ended.
    pub exit_on_esc: bool,
    /// Seconds since 1970 at the last `list`, for the age column and the
    /// filter.
    pub now: u64,
    /// Marked sessions, by id.
    pub marked: BTreeSet<u64>,
    /// The filter text; empty shows every session.
    pub filter: String,
    deleting: Option<Deleting>,
}

impl SessionList {
    /// Starts on the attached session.
    pub fn new(sessions: Vec<SessionInfo>, attached: &SessionInfo, now: u64) -> SessionList {
        let selected = sessions
            .iter()
            .position(|s| s.id == attached.id)
            .unwrap_or(0);
        SessionList {
            sessions,
            selected,
            mode: Mode::Browse,
            current: attached.id,
            shown: attached.workspace.clone(),
            base: attached.path.clone(),
            message: None,
            done: None,
            exit_on_esc: false,
            now,
            marked: BTreeSet::new(),
            filter: String::new(),
            deleting: None,
        }
    }

    /// A newer list: the same session stays selected if it is still shown;
    /// marks of sessions gone are dropped.
    pub fn set_sessions(&mut self, sessions: Vec<SessionInfo>, now: u64) {
        let chosen = self.chosen().map(|s| s.id);
        self.sessions = sessions;
        self.now = now;
        let ids: BTreeSet<u64> = self.sessions.iter().map(|s| s.id).collect();
        self.marked.retain(|id| ids.contains(id));
        let at = chosen.and_then(|id| self.sessions.iter().position(|s| s.id == id));
        self.selected = at.unwrap_or(0);
        self.keep_selection_shown();
    }

    /// The indexes of the sessions the filter shows, in list order.
    pub fn visible(&self) -> Vec<usize> {
        let filter = Filter::new(&self.filter);
        self.sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| filter.matches(s, self.now))
            .map(|(i, _)| i)
            .collect()
    }

    /// After the list or the filter changed: a hidden selection moves to the
    /// first session shown.
    fn keep_selection_shown(&mut self) {
        let visible = self.visible();
        if !visible.contains(&self.selected) {
            self.selected = visible.first().copied().unwrap_or(0);
        }
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    /// The selected session, if it is shown.
    pub fn chosen(&self) -> Option<&SessionInfo> {
        self.visible()
            .contains(&self.selected)
            .then(|| self.sessions.get(self.selected))
            .flatten()
    }

    /// The sessions of a delete question, by name, in list order.
    pub fn named(&self, names: &[String]) -> Vec<&SessionInfo> {
        self.sessions
            .iter()
            .filter(|s| names.contains(&s.name))
            .collect()
    }

    /// Takes a key; what `App` must do, if anything.
    pub fn key(&mut self, key: KeyEvent) -> Option<ListAction> {
        self.message = None;
        self.done = None;
        let mode = std::mem::replace(&mut self.mode, Mode::Browse);
        let (mode, action) = match mode {
            // `browse` sets the next mode itself.
            Mode::Browse => return self.browse(key),
            Mode::ConfirmDelete(names) => match key.code {
                KeyCode::Char('y' | 'Y') => {
                    self.marked.clear();
                    self.deleting = Some(Deleting {
                        open: names.clone(),
                        done: 0,
                        failed: Vec::new(),
                    });
                    (Mode::Browse, Some(ListAction::Delete(names)))
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc => (Mode::Browse, None),
                _ => (Mode::ConfirmDelete(names), None),
            },
            Mode::Filter(mut editor) => match key.code {
                KeyCode::Enter | KeyCode::Down => (Mode::Browse, None),
                KeyCode::Esc => {
                    self.filter.clear();
                    self.keep_selection_shown();
                    (Mode::Browse, None)
                }
                _ => {
                    line_edit(&mut editor, key);
                    self.filter = editor.text();
                    self.keep_selection_shown();
                    (Mode::Filter(editor), None)
                }
            },
            Mode::Rename(mut editor) => match key.code {
                KeyCode::Esc => (Mode::Browse, None),
                KeyCode::Enter => {
                    let name = editor.text().trim().to_owned();
                    match self.chosen() {
                        Some(session) if !name.is_empty() && name != session.name => {
                            let session = session.name.clone();
                            (Mode::Browse, Some(ListAction::Rename { session, name }))
                        }
                        _ => (Mode::Browse, None),
                    }
                }
                _ => {
                    line_edit(&mut editor, key);
                    (Mode::Rename(editor), None)
                }
            },
            Mode::Workspace(mut editor) => match key.code {
                KeyCode::Esc => (Mode::Browse, None),
                KeyCode::Enter => {
                    let workspace = resolve(&editor.text(), &self.base, home().as_deref());
                    let name = Editor::default();
                    (Mode::Name { workspace, name }, None)
                }
                _ => {
                    line_edit(&mut editor, key);
                    (Mode::Workspace(editor), None)
                }
            },
            Mode::Name {
                workspace,
                mut name,
            } => match key.code {
                KeyCode::Esc => {
                    let mut editor = Editor::default();
                    editor.set_text(&shown(&workspace, home().as_deref()));
                    (Mode::Workspace(editor), None)
                }
                KeyCode::Enter => {
                    let name = name.text().trim().to_owned();
                    let name = (!name.is_empty()).then_some(name);
                    (Mode::Browse, Some(ListAction::New { workspace, name }))
                }
                _ => {
                    line_edit(&mut name, key);
                    (Mode::Name { workspace, name }, None)
                }
            },
        };
        self.mode = mode;
        action
    }

    /// Moves the selection among the sessions shown.
    fn step(&mut self, forward: bool) {
        let visible = self.visible();
        let at = visible.iter().position(|&i| i == self.selected);
        let next = match (at, forward) {
            (None, _) => visible.first(),
            (Some(at), true) => visible.get(at + 1).or(visible.last()),
            (Some(at), false) => visible.get(at.saturating_sub(1)),
        };
        if let Some(&next) = next {
            self.selected = next;
        }
    }

    fn browse(&mut self, key: KeyEvent) -> Option<ListAction> {
        match key.code {
            KeyCode::Esc if !self.marked.is_empty() => {
                self.marked.clear();
                None
            }
            KeyCode::Esc if !self.filter.is_empty() => {
                self.filter.clear();
                self.keep_selection_shown();
                None
            }
            KeyCode::Esc if self.exit_on_esc => Some(ListAction::Exit),
            KeyCode::Esc => Some(ListAction::Close),
            KeyCode::Up | KeyCode::Char('k') => {
                self.step(false);
                None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.step(true);
                None
            }
            KeyCode::Enter => self.chosen().map(|s| ListAction::Attach(s.name.clone())),
            KeyCode::Char(' ') => {
                if let Some(id) = self.chosen().map(|s| s.id) {
                    if !self.marked.remove(&id) {
                        self.marked.insert(id);
                    }
                    self.step(true);
                }
                None
            }
            KeyCode::Char('a') => {
                let shown: Vec<u64> = self
                    .visible()
                    .into_iter()
                    .filter_map(|i| self.sessions.get(i))
                    .map(|s| s.id)
                    .collect();
                if shown.iter().all(|id| self.marked.contains(id)) {
                    for id in &shown {
                        self.marked.remove(id);
                    }
                } else {
                    self.marked.extend(shown);
                }
                None
            }
            KeyCode::Char('/') => {
                let mut editor = Editor::default();
                editor.set_text(&self.filter);
                self.mode = Mode::Filter(editor);
                None
            }
            KeyCode::Char('n') => {
                let mut editor = Editor::default();
                editor.set_text(&self.shown);
                self.mode = Mode::Workspace(editor);
                None
            }
            KeyCode::Char('r') => {
                if let Some(session) = self.chosen() {
                    let mut editor = Editor::default();
                    editor.set_text(&session.name);
                    self.mode = Mode::Rename(editor);
                }
                None
            }
            KeyCode::Char('d') | KeyCode::Delete => {
                let names: Vec<String> = if self.marked.is_empty() {
                    self.chosen().map(|s| s.name.clone()).into_iter().collect()
                } else {
                    self.sessions
                        .iter()
                        .filter(|s| self.marked.contains(&s.id))
                        .map(|s| s.name.clone())
                        .collect()
                };
                if !names.is_empty() {
                    self.mode = Mode::ConfirmDelete(names);
                }
                None
            }
            _ => None,
        }
    }

    /// A `kill` of `name` sent by the list waits for its answer.
    pub fn is_deleting(&self, name: &str) -> bool {
        self.deleting
            .as_ref()
            .is_some_and(|b| b.open.iter().any(|n| n == name))
    }

    /// The answer to the list's `kill` of `name`; `true` once all are
    /// answered, and the last line then says how it went.
    pub fn deleted(&mut self, name: &str, result: Result<(), String>) -> bool {
        let Some(batch) = &mut self.deleting else {
            return false;
        };
        let Some(at) = batch.open.iter().position(|n| n == name) else {
            return false;
        };
        batch.open.remove(at);
        match result {
            Ok(()) => batch.done += 1,
            Err(message) => batch.failed.push(format!("{name}: {message}")),
        }
        if batch.open.is_empty() {
            let done = match batch.done {
                1 => "deleted 1 session".to_owned(),
                n => format!("deleted {n} sessions"),
            };
            if batch.failed.is_empty() {
                self.done = Some(done);
            } else {
                self.message = Some(format!("{done}; {}", batch.failed.join("; ")));
            }
            self.deleting = None;
            return true;
        }
        false
    }

    /// Pasted text goes to the line being edited, as one line.
    pub fn paste(&mut self, text: &str) {
        let text = text.replace(['\r', '\n'], " ");
        match &mut self.mode {
            Mode::Rename(editor) | Mode::Workspace(editor) | Mode::Name { name: editor, .. } => {
                editor.paste(&text);
            }
            Mode::Filter(editor) => {
                editor.paste(&text);
                self.filter = editor.text();
                self.keep_selection_shown();
            }
            Mode::Browse | Mode::ConfirmDelete(_) => {}
        }
    }
}

/// One line of text: no new lines.
fn line_edit(editor: &mut Editor, key: KeyEvent) {
    if let Some(edit) = keys::edit(key)
        && edit != Edit::Newline
    {
        editor.edit(edit);
    }
}

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

/// The typed workspace as an absolute path: `~` is the home directory, and a
/// relative path is taken from `base`. The server checks that it is a
/// directory; `..` stays for it, since it resolves links.
pub fn resolve(text: &str, base: &Path, home: Option<&Path>) -> PathBuf {
    let text = text.trim();
    match (text.strip_prefix('~'), home) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => {
            home.join(rest.trim_start_matches('/'))
        }
        _ if text.is_empty() => base.to_path_buf(),
        _ => base.join(text),
    }
}

/// A path as the user typed it: `~/…` below the home directory.
fn shown(path: &Path, home: Option<&Path>) -> String {
    match home.and_then(|h| path.strip_prefix(h).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::State;
    use crossterm::event::KeyModifiers;

    fn info(id: u64, name: &str) -> SessionInfo {
        SessionInfo {
            id,
            name: name.to_owned(),
            workspace: "~/ws/project".to_owned(),
            path: "/home/u/ws/project".into(),
            model: "anthropic/sonnet".to_owned(),
            state: State::Idle,
            created: 0,
            updated: 0,
            turns: 0,
            queued: 0,
            question: None,
            usage: Default::default(),
        }
    }

    fn list() -> SessionList {
        let sessions = vec![info(1, "a"), info(2, "b"), info(3, "c")];
        SessionList::new(sessions, &info(2, "b"), 0)
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn typed(list: &mut SessionList, text: &str) {
        for c in text.chars() {
            assert_eq!(list.key(key(KeyCode::Char(c))), None);
        }
    }

    #[test]
    fn starts_on_the_attached_session_and_stops_at_the_ends() {
        let mut list = list();
        assert_eq!(list.chosen().unwrap().name, "b");
        list.key(key(KeyCode::Up));
        list.key(key(KeyCode::Up));
        assert_eq!(list.chosen().unwrap().name, "a");
        for _ in 0..3 {
            list.key(key(KeyCode::Down));
        }
        assert_eq!(list.chosen().unwrap().name, "c");
        assert_eq!(
            list.key(key(KeyCode::Enter)),
            Some(ListAction::Attach("c".into()))
        );
    }

    #[test]
    fn j_and_k_move_like_down_and_up() {
        let mut list = list();
        list.key(key(KeyCode::Char('j')));
        assert_eq!(list.chosen().unwrap().name, "c");
        list.key(key(KeyCode::Char('j')));
        assert_eq!(list.chosen().unwrap().name, "c");
        list.key(key(KeyCode::Char('k')));
        list.key(key(KeyCode::Char('k')));
        list.key(key(KeyCode::Char('k')));
        assert_eq!(list.chosen().unwrap().name, "a");
        assert_eq!(list.mode, Mode::Browse);
        // While a name or the filter is typed, they are letters.
        list.key(key(KeyCode::Char('/')));
        typed(&mut list, "jk");
        assert_eq!(list.filter, "jk");
    }

    #[test]
    fn a_refresh_keeps_the_selected_session_by_id() {
        let mut list = list();
        list.key(key(KeyCode::Down));
        list.set_sessions(vec![info(3, "renamed"), info(1, "a")], 9);
        assert_eq!(list.chosen().unwrap().name, "renamed");
        assert_eq!(list.now, 9);
        // Gone: the first row; no rows: nothing.
        list.set_sessions(vec![info(1, "a")], 9);
        assert_eq!(list.chosen().unwrap().name, "a");
        list.set_sessions(Vec::new(), 9);
        assert!(list.chosen().is_none());
        assert_eq!(list.key(key(KeyCode::Enter)), None);
        assert_eq!(list.key(key(KeyCode::Char('d'))), None);
        assert_eq!(list.mode, Mode::Browse);
    }

    #[test]
    fn esc_closes_or_exits() {
        let mut list = list();
        assert_eq!(list.key(key(KeyCode::Esc)), Some(ListAction::Close));
        list.exit_on_esc = true;
        assert_eq!(list.key(key(KeyCode::Esc)), Some(ListAction::Exit));
    }

    #[test]
    fn rename_edits_the_name_in_place() {
        let mut list = list();
        list.key(key(KeyCode::Char('r')));
        assert!(matches!(&list.mode, Mode::Rename(e) if e.text() == "b"));
        typed(&mut list, "2");
        assert_eq!(
            list.key(key(KeyCode::Enter)),
            Some(ListAction::Rename {
                session: "b".into(),
                name: "b2".into()
            })
        );
        assert_eq!(list.mode, Mode::Browse);
        // The same name or none sends nothing; Esc goes back.
        list.key(key(KeyCode::Char('r')));
        assert_eq!(list.key(key(KeyCode::Enter)), None);
        list.key(key(KeyCode::Char('r')));
        assert_eq!(list.key(key(KeyCode::Esc)), None);
        assert_eq!(list.mode, Mode::Browse);
    }

    #[test]
    fn delete_asks_first() {
        for code in [KeyCode::Char('d'), KeyCode::Delete] {
            let mut list = list();
            list.key(key(code));
            assert_eq!(list.mode, Mode::ConfirmDelete(vec!["b".into()]));
            assert_eq!(list.key(key(KeyCode::Char('x'))), None);
            assert!(matches!(list.mode, Mode::ConfirmDelete(_)));
            assert_eq!(list.key(key(KeyCode::Char('n'))), None);
            assert_eq!(list.mode, Mode::Browse);
            list.key(key(code));
            assert_eq!(
                list.key(key(KeyCode::Char('y'))),
                Some(ListAction::Delete(vec!["b".into()]))
            );
            assert!(list.is_deleting("b"));
        }
    }

    #[test]
    fn space_marks_and_moves_on() {
        let mut list = list();
        list.key(key(KeyCode::Up));
        list.key(key(KeyCode::Char(' ')));
        assert_eq!(list.chosen().unwrap().name, "b");
        list.key(key(KeyCode::Char(' ')));
        assert_eq!(list.marked, BTreeSet::from([1, 2]));
        // Again on a marked one: unmarked.
        list.key(key(KeyCode::Up));
        list.key(key(KeyCode::Char(' ')));
        assert_eq!(list.marked, BTreeSet::from([1]));
    }

    #[test]
    fn a_marks_all_shown_then_unmarks() {
        let mut list = list();
        list.key(key(KeyCode::Char('a')));
        assert_eq!(list.marked, BTreeSet::from([1, 2, 3]));
        list.key(key(KeyCode::Char('a')));
        assert!(list.marked.is_empty());
        list.sessions[2].workspace = "~/ws/docs".into();
        list.filter = "docs".into();
        list.key(key(KeyCode::Char('a')));
        assert_eq!(list.marked, BTreeSet::from([3]));
    }

    #[test]
    fn marks_survive_a_refresh_and_drop_for_sessions_gone() {
        let mut list = list();
        list.key(key(KeyCode::Char('a')));
        list.set_sessions(vec![info(3, "c"), info(1, "a")], 5);
        assert_eq!(list.marked, BTreeSet::from([1, 3]));
    }

    #[test]
    fn delete_takes_the_marked_sessions_in_list_order() {
        let mut list = list();
        list.key(key(KeyCode::Down));
        list.key(key(KeyCode::Char(' ')));
        list.key(key(KeyCode::Up));
        list.key(key(KeyCode::Up));
        list.key(key(KeyCode::Char(' ')));
        list.key(key(KeyCode::Char('d')));
        assert_eq!(list.mode, Mode::ConfirmDelete(vec!["a".into(), "c".into()]));
        // `n` keeps the marks; `y` clears them.
        list.key(key(KeyCode::Char('n')));
        assert_eq!(list.marked.len(), 2);
        list.key(key(KeyCode::Char('d')));
        assert_eq!(
            list.key(key(KeyCode::Char('y'))),
            Some(ListAction::Delete(vec!["a".into(), "c".into()]))
        );
        assert!(list.marked.is_empty());
    }

    #[test]
    fn the_result_line_waits_for_all_answers() {
        let mut list = list();
        list.key(key(KeyCode::Char('a')));
        list.key(key(KeyCode::Char('d')));
        list.key(key(KeyCode::Char('y')));
        assert!(!list.deleted("other", Ok(())));
        assert!(!list.deleted("a", Ok(())));
        assert_eq!(list.done, None);
        assert!(!list.deleted("b", Err("no session named b".into())));
        assert!(list.deleted("c", Ok(())));
        assert_eq!(
            list.message.as_deref(),
            Some("deleted 2 sessions; b: no session named b")
        );
        assert!(!list.is_deleting("a"));
        // All well: the result, not an error.
        list.key(key(KeyCode::Char('d')));
        list.key(key(KeyCode::Char('y')));
        assert!(list.deleted("b", Ok(())));
        assert_eq!(list.done.as_deref(), Some("deleted 1 session"));
        assert_eq!(list.message, None);
    }

    #[test]
    fn the_filter_hides_rows_as_it_is_typed() {
        let mut list = list();
        list.sessions[2].workspace = "~/ws/docs".into();
        list.key(key(KeyCode::Char('/')));
        typed(&mut list, "docs");
        assert_eq!(list.filter, "docs");
        assert_eq!(list.visible(), [2]);
        // The selection moves to the first shown.
        assert_eq!(list.chosen().unwrap().name, "c");
        assert_eq!(list.key(key(KeyCode::Enter)), None);
        assert_eq!(list.mode, Mode::Browse);
        assert_eq!(list.filter, "docs");
        // Up and Down stay among the shown rows.
        list.key(key(KeyCode::Up));
        assert_eq!(list.chosen().unwrap().name, "c");
        // `/` again starts with the text; Esc there empties it.
        list.key(key(KeyCode::Char('/')));
        assert!(matches!(&list.mode, Mode::Filter(e) if e.text() == "docs"));
        list.key(key(KeyCode::Esc));
        assert_eq!(list.filter, "");
        assert_eq!(list.visible().len(), 3);
    }

    #[test]
    fn nothing_shown_nothing_to_act_on() {
        let mut list = list();
        list.filter = "nothing".into();
        assert!(list.chosen().is_none());
        for code in [
            KeyCode::Enter,
            KeyCode::Char('d'),
            KeyCode::Char('r'),
            KeyCode::Char(' '),
        ] {
            assert_eq!(list.key(key(code)), None);
            assert_eq!(list.mode, Mode::Browse);
        }
        assert!(list.marked.is_empty());
    }

    #[test]
    fn esc_clears_marks_then_the_filter_then_closes() {
        let mut list = list();
        list.key(key(KeyCode::Char(' ')));
        list.filter = "b".into();
        assert_eq!(list.key(key(KeyCode::Esc)), None);
        assert!(list.marked.is_empty());
        assert_eq!(list.filter, "b");
        assert_eq!(list.key(key(KeyCode::Esc)), None);
        assert_eq!(list.filter, "");
        assert_eq!(list.key(key(KeyCode::Esc)), Some(ListAction::Close));
    }

    #[test]
    fn new_asks_for_workspace_then_name() {
        let mut list = list();
        list.key(key(KeyCode::Char('n')));
        assert!(matches!(&list.mode, Mode::Workspace(e) if e.text() == "~/ws/project"));
        // Replace it with a relative path.
        list.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        typed(&mut list, "docs");
        assert_eq!(list.key(key(KeyCode::Enter)), None);
        let Mode::Name { workspace, .. } = &list.mode else {
            panic!("{:?}", list.mode);
        };
        assert_eq!(workspace, Path::new("/home/u/ws/project/docs"));
        // Esc goes back one step, Enter with no name takes the default.
        assert_eq!(list.key(key(KeyCode::Esc)), None);
        assert!(matches!(&list.mode, Mode::Workspace(_)));
        list.key(key(KeyCode::Enter));
        typed(&mut list, "d2");
        assert_eq!(
            list.key(key(KeyCode::Enter)),
            Some(ListAction::New {
                workspace: "/home/u/ws/project/docs".into(),
                name: Some("d2".into())
            })
        );
        list.key(key(KeyCode::Char('n')));
        list.key(key(KeyCode::Enter));
        assert!(matches!(
            list.key(key(KeyCode::Enter)),
            Some(ListAction::New { name: None, .. })
        ));
    }

    #[test]
    fn paste_goes_to_the_line_being_edited() {
        let mut list = list();
        list.paste("ignored");
        list.key(key(KeyCode::Char('r')));
        list.paste("x\ny");
        assert!(matches!(&list.mode, Mode::Rename(e) if e.text() == "bx y"));
    }

    #[test]
    fn workspace_paths() {
        let base = Path::new("/ws/project");
        let home = Some(Path::new("/home/u"));
        assert_eq!(resolve("", base, home), base);
        assert_eq!(resolve("~", base, home), Path::new("/home/u"));
        assert_eq!(resolve("~/ws/x", base, home), Path::new("/home/u/ws/x"));
        assert_eq!(resolve(" /tmp ", base, home), Path::new("/tmp"));
        assert_eq!(
            resolve("../docs", base, home),
            Path::new("/ws/project/../docs")
        );
        // `~user` is not expanded.
        assert_eq!(resolve("~x", base, home), Path::new("/ws/project/~x"));
        assert_eq!(resolve("~", base, None), Path::new("/ws/project/~"));
        assert_eq!(shown(Path::new("/home/u/ws"), home), "~/ws");
        assert_eq!(shown(Path::new("/home/u"), home), "~");
        assert_eq!(shown(Path::new("/tmp"), home), "/tmp");
    }
}
