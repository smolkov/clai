//! The state of the session list (see `spec/tui.md`, "Sessions"): the sessions
//! from the server's last `list`, the selected row and what the box asks for.
//! `App` routes keys here and carries out what comes back; `view::sessions`
//! draws it.

use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent};

use crate::protocol::SessionInfo;

use super::editor::{Edit, Editor};
use super::keys;

/// What the box is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mode {
    Browse,
    /// `r`: the new name of the selected session.
    Rename(Editor),
    /// `k`: `y` / `n`.
    ConfirmKill,
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
    Kill(String),
    /// Go on to the model picker for a new session.
    New {
        workspace: PathBuf,
        name: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionList {
    pub sessions: Vec<SessionInfo>,
    selected: usize,
    pub mode: Mode,
    /// The id of the attached session, marked in the list.
    pub current: u64,
    /// The attached session's workspace, the default for a new one: as shown
    /// (`~/ws/project`) and absolute.
    shown: String,
    base: PathBuf,
    /// A message for the box's last line, e.g. a refused name.
    pub message: Option<String>,
    /// `Esc` leaves the TUI instead of closing the list: at start without a
    /// session chosen, and after the attached session ended.
    pub exit_on_esc: bool,
    /// Seconds since 1970 at the last `list`, for the age column.
    pub now: u64,
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
            exit_on_esc: false,
            now,
        }
    }

    /// A newer list: the same session stays selected if it is still there.
    pub fn set_sessions(&mut self, sessions: Vec<SessionInfo>, now: u64) {
        let chosen = self.chosen().map(|s| s.id);
        self.sessions = sessions;
        self.now = now;
        let at = chosen.and_then(|id| self.sessions.iter().position(|s| s.id == id));
        self.selected = at.unwrap_or(0).min(self.sessions.len().saturating_sub(1));
    }

    pub fn selected(&self) -> usize {
        self.selected
    }

    pub fn chosen(&self) -> Option<&SessionInfo> {
        self.sessions.get(self.selected)
    }

    /// Takes a key; what `App` must do, if anything.
    pub fn key(&mut self, key: KeyEvent) -> Option<ListAction> {
        self.message = None;
        let mode = std::mem::replace(&mut self.mode, Mode::Browse);
        let (mode, action) = match mode {
            // `browse` sets the next mode itself.
            Mode::Browse => return self.browse(key),
            Mode::ConfirmKill => match key.code {
                KeyCode::Char('y' | 'Y') => {
                    let name = self.chosen().map(|s| s.name.clone());
                    (Mode::Browse, name.map(ListAction::Kill))
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc => (Mode::Browse, None),
                _ => (Mode::ConfirmKill, None),
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

    fn browse(&mut self, key: KeyEvent) -> Option<ListAction> {
        match key.code {
            KeyCode::Esc if self.exit_on_esc => Some(ListAction::Exit),
            KeyCode::Esc => Some(ListAction::Close),
            KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
                None
            }
            KeyCode::Down => {
                let last = self.sessions.len().saturating_sub(1);
                self.selected = (self.selected + 1).min(last);
                None
            }
            KeyCode::Enter => self.chosen().map(|s| ListAction::Attach(s.name.clone())),
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
            KeyCode::Char('k') => {
                if self.chosen().is_some() {
                    self.mode = Mode::ConfirmKill;
                }
                None
            }
            _ => None,
        }
    }

    /// Pasted text goes to the line being edited, as one line.
    pub fn paste(&mut self, text: &str) {
        let text = text.replace(['\r', '\n'], " ");
        match &mut self.mode {
            Mode::Rename(editor) | Mode::Workspace(editor) | Mode::Name { name: editor, .. } => {
                editor.paste(&text);
            }
            Mode::Browse | Mode::ConfirmKill => {}
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
        assert_eq!(list.key(key(KeyCode::Char('k'))), None);
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
    fn kill_asks_first() {
        let mut list = list();
        list.key(key(KeyCode::Char('k')));
        assert_eq!(list.mode, Mode::ConfirmKill);
        assert_eq!(list.key(key(KeyCode::Char('x'))), None);
        assert_eq!(list.mode, Mode::ConfirmKill);
        assert_eq!(list.key(key(KeyCode::Char('n'))), None);
        assert_eq!(list.mode, Mode::Browse);
        list.key(key(KeyCode::Char('k')));
        assert_eq!(
            list.key(key(KeyCode::Char('y'))),
            Some(ListAction::Kill("b".into()))
        );
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
