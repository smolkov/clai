//! All view state and how it changes: one input in, effects out. No terminal
//! and no network here, so nearly all behaviour is tested without either.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;

use crate::agent::{Closed, QuestionKind, SessionEvent, State, now_secs};
use crate::config::edit::{Setting, Stamp};
use crate::config::{Symbols, Theme, ThemeWarning, Tui};
use crate::protocol::{
    ModelInfo, Notice, Payload, Request, ServerMessage, SessionInfo, TokenTotals,
};
use indexmap::IndexMap;

use super::commands::{self, Command};
use super::conversation::{Conversation, Expansion, Kind};
use super::editor::{Browse, Edit, Editor};
use super::keys::{self, Action, Focus};
use super::picker::{Picker, Purpose};
use super::prompt::Prompt;
use super::sessions::{ListAction, SessionList};
use super::settings::{BoxAction, SettingsBox};
use super::status::StatusBox;
use super::style::{Colouring, Look, Styles};
use super::themes::{self, ThemeList};
use super::view;

/// `Ctrl-C` twice within this time detaches.
const DOUBLE_INTERRUPT: Duration = Duration::from_secs(1);
/// How long to try to get the connection back.
pub const RECONNECT_FOR: Duration = Duration::from_secs(10);
/// How many ticks between two `list` requests for the session row.
const LIST_TICKS: u32 = 2;

/// The error of a request whose connection was lost: a reconnect follows, so
/// it is no news to the user.
pub const LOST: &str = "the connection to the server was lost";

/// How the TUI ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Exit {
    Detached {
        name: String,
    },
    Ended {
        name: String,
        reason: String,
    },
    /// The server stayed away.
    Lost,
    /// `SIGTERM` or `SIGHUP`.
    Signal,
}

/// The settings; only the context windows change while the TUI runs.
#[derive(Debug, Clone)]
pub struct Settings {
    pub look: Look,
    /// Who this client is to the server: `tui pts/3`.
    pub client: String,
    pub tool_output_lines: usize,
    pub expand_tool_calls: bool,
    pub scroll_lines: usize,
    /// `tui.timestamps`: the offset of local time from UTC, in seconds.
    pub timestamps: Option<i64>,
    /// Context windows by `provider/model`: from the TUI's own configuration
    /// at first, then from the server's `models` answers.
    pub windows: HashMap<String, u64>,
    /// The `[tui]` settings in effect; the fields above follow from them.
    pub tui: Tui,
    pub theming: Theming,
}

/// What the look is made from besides `[tui]`; read once by the shell.
#[derive(Debug, Clone)]
pub struct Theming {
    /// Bundled themes, theme files and `[themes.NAME]` tables.
    pub themes: IndexMap<String, Theme>,
    /// Broken theme files, left out.
    pub warnings: Vec<ThemeWarning>,
    /// What the user file itself says, for the settings box.
    pub file: FileTui,
    pub colouring: Colouring,
    /// Whether the locale is UTF-8, for `symbols = "auto"`.
    pub unicode_locale: bool,
    /// `CLAI_THEME`: wins over `tui.theme` for this run.
    pub theme_env: Option<String>,
    /// The offset of local time from UTC in seconds, for `tui.timestamps`.
    pub offset: i64,
    /// The user file `[tui]` is read from and saved to.
    pub user_file: PathBuf,
}

/// `[tui]` as the user file has it, without `CLAI_THEME`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FileTui {
    pub tui: Tui,
    /// The keys the file sets; the others have their default.
    pub keys: Vec<String>,
    /// When it was read, so that a save does not overwrite a newer file.
    pub stamp: Option<Stamp>,
}

/// `[tui]` and the themes as loaded again from the user file.
#[derive(Debug, Clone, PartialEq)]
pub struct Reloaded {
    /// With `CLAI_THEME` applied, as at start.
    pub tui: Tui,
    pub themes: IndexMap<String, Theme>,
    pub warnings: Vec<ThemeWarning>,
    pub file: FileTui,
}

impl Settings {
    /// Whether Unicode symbols are shown with these settings.
    pub fn unicode(&self, symbols: Symbols) -> bool {
        match symbols {
            Symbols::Auto => self.theming.unicode_locale,
            Symbols::Unicode => true,
            Symbols::Ascii => false,
        }
    }

    /// The look of `tui` with the themes known now.
    pub fn look_of(&self, tui: &Tui) -> Look {
        let styles = Styles::themed(&tui.theme, &self.theming.themes, self.theming.colouring);
        Look::themed(self.unicode(tui.symbols), styles)
    }
}

/// A session just attached to.
#[derive(Debug, Clone, PartialEq)]
pub struct Attachment {
    pub session: SessionInfo,
    pub sessions: Vec<SessionInfo>,
    /// The replayed log and what came after it.
    pub backlog: Vec<ServerMessage>,
}

/// One thing that happened.
#[derive(Debug, Clone, PartialEq)]
pub enum Input {
    Key(KeyEvent),
    Paste(String),
    Mouse(MouseEvent),
    Resize,
    Server(ServerMessage),
    /// The answer to a request this client sent.
    Response {
        request: Request,
        result: Result<Payload, String>,
    },
    /// The 1 s timer.
    Tick,
    Disconnected,
    /// Attached again after a lost connection, or to another session.
    Attached(Box<Attachment>),
    ReconnectFailed,
    /// The user file loaded again after `config_reloaded`, or why it failed.
    Reloaded(Box<Result<Reloaded, String>>),
    /// A save: the file's new stamp, or why it was not written.
    Saved(Result<Option<Stamp>, String>),
    /// A switch to another session failed; the current one stays.
    SwitchFailed(String),
}

/// What the shell has to do.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Request(Request),
    Redraw,
    /// Try to connect again, after a short wait.
    Reconnect,
    /// Attach to another session on a new connection; the old one is dropped
    /// when it is there, which detaches it.
    Switch(String),
    /// Turn mouse capture on or off.
    Mouse(bool),
    /// Load `[tui]` and the themes from the user file again.
    ReloadConfig,
    /// Write `[tui]` changes into the user file, if it still has `stamp`.
    Save {
        path: PathBuf,
        stamp: Option<Stamp>,
        changes: Vec<(&'static str, Option<Setting>)>,
    },
    Exit(Exit),
}

/// What a save is for, to report its result.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Saving {
    Theme(String),
    /// From the settings box; `close` it once saved.
    Settings {
        close: bool,
    },
}

/// What the TUI keeps of a session while it shows another one (see
/// `spec/tui.md`, "Switching sessions"). The conversation is built again from
/// the log.
#[derive(Debug, Clone)]
struct View {
    top: usize,
    follow: bool,
    focus: Focus,
    selected: Option<usize>,
    expanded: HashMap<String, Expansion>,
    queued: Vec<String>,
    editor: Editor,
}

impl Default for View {
    fn default() -> Self {
        View {
            top: 0,
            follow: true,
            focus: Focus::Input,
            selected: None,
            expanded: HashMap::new(),
            queued: Vec::new(),
            editor: Editor::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialog {
    Help,
    ConfirmKill,
}

/// Position among all sessions and others waiting for an answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Place {
    pub position: usize,
    pub total: usize,
    pub waiting: usize,
}

#[derive(Debug)]
pub struct App {
    pub settings: Settings,
    pub session: SessionInfo,
    pub conversation: Conversation,
    pub editor: Editor,
    pub prompt: Option<Prompt>,
    pub dialog: Option<Dialog>,
    pub picker: Option<Picker>,
    /// A `models` request is on its way to open the picker for this.
    picker_wanted: Option<Purpose>,
    pub session_list: Option<SessionList>,
    /// A `list` request is on its way to open the session list.
    list_wanted: bool,
    /// The sessions of the last `list`, in the server's order.
    pub sessions: Vec<SessionInfo>,
    /// The models of the last `models` answer.
    pub models: Vec<ModelInfo>,
    /// A switch to this session is on its way.
    pub switching: Option<String>,
    /// The view of each session shown before, by id.
    views: HashMap<u64, View>,
    /// A notice for a session once it is attached, by id.
    greeting: Option<(u64, String)>,
    /// The server is stopping: it is not started again for the session list.
    stopping: bool,
    pub settings_box: Option<SettingsBox>,
    pub theme_list: Option<ThemeList>,
    /// The status box of `/status`.
    pub status_box: Option<StatusBox>,
    /// A theme taken in this run: it wins over `CLAI_THEME` on a reload.
    chosen_theme: Option<String>,
    /// What the save on its way is for.
    saving: Option<Saving>,
    /// The selected row of the open command list. The search is the input's
    /// text, so nothing else needs keeping in step.
    pub command_list: Option<usize>,
    /// The selected row of the open argument list of `/attach` or `/model`.
    pub argument_list: Option<usize>,
    /// A `models` request for the argument list is on its way.
    models_asked: bool,
    pub focus: Focus,
    /// The selected entry, with focus on the conversation.
    pub selected: Option<usize>,
    /// The first row shown.
    pub top: usize,
    /// New output keeps the view at the end.
    pub follow: bool,
    /// New output came while the user had scrolled up.
    pub new_output: bool,
    pub place: Option<Place>,
    /// Tokens of the last request.
    pub context_tokens: u64,
    /// The sum of the log's `usage` events; `None` until the first. It is not
    /// taken from the session info: the log replayed after it would count twice.
    pub usage: Option<TokenTotals>,
    /// In the current turn.
    pub requests: u32,
    pub tool_calls: u32,
    /// Since when the connection is gone.
    pub lost_since: Option<Instant>,
    /// Why the session ended; any key then exits.
    pub ended: Option<String>,
    /// A short message in the status line, until the next key.
    pub message: Option<String>,
    /// The current time; the shell sets it before each input.
    pub now: Instant,
    last_interrupt: Option<Instant>,
    ticks: u32,
    /// Rows of the conversation area, from the last layout.
    view_height: usize,
    /// All rows, from the last layout.
    total: Option<usize>,
    width: Option<usize>,
    /// The entry to bring into view at the next layout.
    reveal: Option<usize>,
    /// Revealing a question stopped following; resume when it closes.
    resume_follow: bool,
}

impl App {
    pub fn new(settings: Settings, attachment: Attachment, now: Instant) -> App {
        let conversation = Conversation::new(
            settings.look,
            settings.tool_output_lines,
            settings.expand_tool_calls,
        );
        let mut app = App {
            settings,
            session: attachment.session.clone(),
            conversation,
            editor: Editor::default(),
            prompt: None,
            dialog: None,
            picker: None,
            picker_wanted: None,
            session_list: None,
            list_wanted: false,
            sessions: Vec::new(),
            models: Vec::new(),
            switching: None,
            views: HashMap::new(),
            greeting: None,
            stopping: false,
            settings_box: None,
            status_box: None,
            theme_list: None,
            chosen_theme: None,
            saving: None,
            command_list: None,
            argument_list: None,
            models_asked: false,
            focus: Focus::Input,
            selected: None,
            top: 0,
            follow: true,
            new_output: false,
            place: None,
            context_tokens: 0,
            usage: None,
            requests: 0,
            tool_calls: 0,
            lost_since: None,
            ended: None,
            message: None,
            now,
            last_interrupt: None,
            ticks: 0,
            view_height: 0,
            total: None,
            width: None,
            reveal: None,
            resume_follow: false,
        };
        app.attached(attachment);
        app
    }

    /// Takes one input.
    pub fn update(&mut self, input: Input) -> Vec<Effect> {
        let asked_before = self.prompt.as_ref().map(|p| p.id().to_owned());
        let mut effects = self.handle(input);
        // A question that opens closes the boxes, so that none hides it. The
        // session list, and the picker for a new session from it, may then
        // be opened over the prompt; they take the keys until they close.
        let asked_now = self.prompt.as_ref().map(|p| p.id());
        if asked_now.is_some() && asked_now != asked_before.as_deref() {
            self.status_box = None;
            let mut restored = self.close_theme_boxes();
            effects.append(&mut restored);
            self.picker = None;
            self.picker_wanted = None;
            self.session_list = None;
            self.list_wanted = false;
            self.command_list = None;
        }
        // An ended session has no model to change; a new session still can
        // be made from the session list.
        if self.ended.is_some() {
            if self
                .picker
                .as_ref()
                .is_some_and(|p| p.purpose == Purpose::Change)
            {
                self.picker = None;
            }
            if self.picker_wanted == Some(Purpose::Change) {
                self.picker_wanted = None;
            }
            self.command_list = None;
        }
        // The picker can open late, when the server answers.
        if self.picker.is_some() || commands::search_text(&self.editor.text()).is_none() {
            self.command_list = None;
        }

        let boxed = self.picker.is_some()
            || self.session_list.is_some()
            || self.settings_box.is_some()
            || self.theme_list.is_some()
            || self.status_box.is_some()
            || self.prompt.is_some()
            || self.dialog.is_some()
            || self.ended.is_some();
        match commands::argument_text(&self.editor.text()) {
            _ if boxed => self.argument_list = None,
            None => self.argument_list = None,
            // The models come with the first answer to `models`.
            Some(("model", _))
                if self.argument_list.is_some() && self.models.is_empty() && !self.models_asked =>
            {
                self.models_asked = true;
                effects.push(Effect::Request(Request::Models));
            }
            Some(_) => {}
        }
        effects
    }

    // Settings and themes.

    /// The themes that can be used, in the order of the theme list.
    fn theme_names(&self) -> Vec<String> {
        themes::entries(&self.settings.theming.themes, &[])
            .into_iter()
            .map(|e| e.name)
            .collect()
    }

    /// The theme in effect without a box open: one taken in this run, then
    /// `CLAI_THEME`, then the file's.
    fn base_theme(&self, file_theme: &str) -> String {
        self.chosen_theme
            .clone()
            .or_else(|| self.settings.theming.theme_env.clone())
            .unwrap_or_else(|| file_theme.to_owned())
    }

    /// The settings the open box shows, as applied: its theme only if the user
    /// changed it there, since `CLAI_THEME` may decide it otherwise.
    fn box_preview(&self) -> Tui {
        let Some(settings) = &self.settings_box else {
            return self.settings.tui.clone();
        };
        let mut tui = settings.current.clone();
        if tui.theme == settings.saved.theme {
            tui.theme = self.base_theme(&settings.saved.theme);
        }
        tui
    }

    /// The settings with another theme, for a preview.
    fn with_theme(&self, theme: &str) -> Tui {
        Tui {
            theme: theme.to_owned(),
            ..self.settings.tui.clone()
        }
    }

    /// A question closes the theme list and the settings box, as it does the
    /// picker; what they previewed is undone.
    fn close_theme_boxes(&mut self) -> Vec<Effect> {
        let mut effects = Vec::new();
        if let Some(list) = self.theme_list.take() {
            effects.extend(self.apply(self.with_theme(&list.original)));
        }
        if self.settings_box.take().is_some() {
            let file = self.settings.theming.file.tui.clone();
            let tui = Tui {
                theme: self.base_theme(&file.theme),
                ..file
            };
            effects.extend(self.apply(tui));
        }
        effects
    }

    /// `/settings`: the box on the file's values.
    fn open_settings(&mut self) {
        let file = &self.settings.theming.file;
        self.settings_box = Some(SettingsBox::new(
            file.tui.clone(),
            file.tui.clone(),
            file.keys.clone(),
            self.theme_names(),
        ));
    }

    /// `/theme`, or `Enter` on the theme row of the settings box.
    fn open_themes(&mut self, for_settings: bool) {
        let current = if for_settings {
            self.box_preview().theme
        } else {
            self.settings.tui.theme.clone()
        };
        let entries = themes::entries(
            &self.settings.theming.themes,
            &self.settings.theming.warnings,
        );
        self.theme_list = Some(ThemeList::new(entries, &current, for_settings));
    }

    /// Takes a theme for this run and saves it as the default.
    fn take_theme(&mut self, name: &str) -> Vec<Effect> {
        let mut effects = self.apply(self.with_theme(name));
        self.chosen_theme = Some(name.to_owned());
        self.saving = Some(Saving::Theme(name.to_owned()));
        effects.push(self.save_effect(vec![("theme", Some(Setting::Text(name.to_owned())))]));
        effects
    }

    fn save_effect(&self, changes: Vec<(&'static str, Option<Setting>)>) -> Effect {
        Effect::Save {
            path: self.settings.theming.user_file.clone(),
            stamp: self.settings.theming.file.stamp,
            changes,
        }
    }

    fn saved(&mut self, result: Result<Option<Stamp>, String>) {
        let saving = self.saving.take();
        let file = &mut self.settings.theming.file;
        match (result, saving) {
            (Ok(stamp), Some(Saving::Theme(name))) => {
                file.stamp = stamp;
                file.tui.theme.clone_from(&name);
                if !file.keys.iter().any(|k| k == "theme") {
                    file.keys.push("theme".to_owned());
                }
                self.message = Some(format!("theme {name} saved as the default"));
            }
            (Ok(stamp), Some(Saving::Settings { close })) => {
                file.stamp = stamp;
                if let Some(settings) = &mut self.settings_box {
                    settings.saved_now();
                    file.tui = settings.saved.clone();
                    file.keys.clone_from(&settings.file_keys);
                }
                if close {
                    self.settings_box = None;
                }
            }
            (Ok(stamp), None) => file.stamp = stamp,
            (Err(message), Some(Saving::Settings { .. })) if self.settings_box.is_some() => {
                if let Some(settings) = &mut self.settings_box {
                    settings.message = Some(message);
                }
            }
            (Err(message), _) => self.conversation.error(format!("not saved: {message}")),
        }
    }

    fn theme_list_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let mut effects = vec![Effect::Redraw];
        let Some(list) = &mut self.theme_list else {
            return effects;
        };
        match key.code {
            KeyCode::Esc => {
                let original = list.original.clone();
                let for_settings = list.for_settings;
                self.theme_list = None;
                let tui = if for_settings {
                    self.box_preview()
                } else {
                    self.with_theme(&original)
                };
                effects.extend(self.apply(tui));
                return effects;
            }
            KeyCode::Enter => {
                let Some(name) = list.preview().map(str::to_owned) else {
                    return effects;
                };
                let for_settings = list.for_settings;
                self.theme_list = None;
                if for_settings {
                    if let Some(settings) = &mut self.settings_box {
                        settings.set_theme(&name);
                    }
                    effects.extend(self.apply(self.box_preview()));
                } else {
                    effects.extend(self.take_theme(&name));
                }
                return effects;
            }
            KeyCode::Up => list.up(),
            KeyCode::Down => list.down(),
            _ => {
                if let Some(edit) = keys::edit(key) {
                    list.edit(edit);
                }
            }
        }
        if let Some(name) = self.theme_list.as_ref().and_then(|l| l.preview())
            && name != self.settings.tui.theme
        {
            let tui = self.with_theme(name);
            effects.extend(self.apply(tui));
        }
        effects
    }

    fn settings_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let mut effects = vec![Effect::Redraw];
        let Some(settings) = &mut self.settings_box else {
            return effects;
        };
        let Some(action) = settings.key(key) else {
            return effects;
        };
        match action {
            BoxAction::Preview => effects.extend(self.apply(self.box_preview())),
            BoxAction::Save { changes, close } => {
                if let Some((_, Some(Setting::Text(theme)))) =
                    changes.iter().find(|(k, _)| *k == "theme")
                {
                    self.chosen_theme = Some(theme.clone());
                }
                self.saving = Some(Saving::Settings { close });
                effects.push(self.save_effect(changes));
            }
            BoxAction::Close { restore } => {
                if restore {
                    effects.extend(self.close_theme_boxes());
                }
                self.settings_box = None;
            }
            BoxAction::Themes => self.open_themes(true),
        }
        effects
    }

    /// `/theme NAME`: takes it at once; an unknown name stays in the input.
    fn theme_command(&mut self, name: &str) -> Option<Vec<Effect>> {
        if themes::usable(name, &self.settings.theming.themes) {
            Some(self.take_theme(name))
        } else {
            self.conversation
                .notice(format!("unknown theme {name}; /theme lists them"));
            None
        }
    }

    fn handle(&mut self, input: Input) -> Vec<Effect> {
        match input {
            Input::Key(key) => self.key(key),
            Input::Paste(text) => self.paste(&text),
            Input::Mouse(mouse) => self.mouse(mouse),
            Input::Resize => vec![Effect::Redraw],
            Input::Server(message) => self.server(message),
            Input::Response { request, result } => self.response(request, result),
            Input::Tick => self.tick(),
            Input::Disconnected => self.disconnected(),
            Input::Attached(attachment) => {
                self.lost_since = None;
                self.attached(*attachment);
                vec![Effect::Redraw]
            }
            Input::ReconnectFailed => self.reconnect_failed(),
            Input::Reloaded(result) => match *result {
                Ok(reloaded) => {
                    self.settings.theming.themes = reloaded.themes;
                    self.settings.theming.warnings = reloaded.warnings;
                    let names = self.theme_names();
                    if let Some(list) = &mut self.theme_list {
                        list.set_entries(themes::entries(
                            &self.settings.theming.themes,
                            &self.settings.theming.warnings,
                        ));
                    }
                    if let Some(settings) = &mut self.settings_box {
                        let file = &reloaded.file;
                        settings.reloaded(file.tui.clone(), file.keys.clone(), names);
                    }
                    self.settings.theming.file = reloaded.file;
                    let mut tui = reloaded.tui;
                    if let Some(theme) = &self.chosen_theme {
                        tui.theme.clone_from(theme);
                    }
                    if self.settings_box.is_some() {
                        tui = self.box_preview();
                    }
                    // An open theme list keeps showing what is selected.
                    if let Some(name) = self.theme_list.as_ref().and_then(|l| l.preview()) {
                        tui.theme = name.to_owned();
                    }
                    let mut effects = self.apply(tui);
                    effects.push(Effect::Redraw);
                    effects
                }
                Err(message) => {
                    // The settings in effect stay.
                    self.message = Some(format!("configuration: {message}"));
                    vec![Effect::Redraw]
                }
            },
            Input::Saved(result) => {
                self.saved(result);
                vec![Effect::Redraw]
            }
            Input::SwitchFailed(message) => {
                self.switching = None;
                self.conversation.error(message);
                vec![Effect::Redraw]
            }
        }
    }

    /// Puts `[tui]` settings into effect: the look, the conversation's
    /// settings, timestamps, scrolling, and the mouse through an effect. The
    /// shell reads `max_fps` from here before each frame.
    pub fn apply(&mut self, tui: Tui) -> Vec<Effect> {
        let look = self.settings.look_of(&tui);
        let limit = usize::try_from(tui.tool_output_lines).unwrap_or(usize::MAX);
        let changed = look != self.settings.look
            || limit != self.settings.tool_output_lines
            || tui.expand_tool_calls != self.settings.expand_tool_calls;
        if changed {
            self.conversation
                .reconfigure(look, limit, tui.expand_tool_calls);
        }
        let mut effects = Vec::new();
        if tui.mouse != self.settings.tui.mouse {
            effects.push(Effect::Mouse(tui.mouse));
        }
        self.settings.look = look;
        self.settings.tool_output_lines = limit;
        self.settings.expand_tool_calls = tui.expand_tool_calls;
        self.settings.scroll_lines = usize::from(tui.scroll_lines);
        self.settings.timestamps = tui.timestamps.then_some(self.settings.theming.offset);
        self.settings.tui = tui;
        effects
    }

    /// The time between two frames, from `tui.max_fps`.
    pub fn frame_time(&self) -> Duration {
        Duration::from_secs(1) / u32::from(self.settings.tui.max_fps.max(1))
    }

    /// The window of the session's model, if it is configured.
    pub fn context_window(&self) -> Option<u64> {
        self.settings.windows.get(&self.session.model).copied()
    }

    // Attach and replay.

    /// Builds the conversation from the log. Scroll position, expanded
    /// calls and unsent input stay, for a re-attach after a lost connection.
    fn attached(&mut self, attachment: Attachment) {
        let Attachment {
            session,
            sessions,
            backlog,
        } = attachment;
        if session.id != self.session.id {
            self.leave_for(session.id);
        }
        self.switching = None;
        self.session = session;
        self.conversation.clear();
        self.conversation.stamp = None;
        self.total = None;
        (self.requests, self.tool_calls) = (0, 0);
        // Summed again from the log, so that a reconnect does not add to it.
        self.context_tokens = 0;
        self.usage = None;
        let earlier = self.prompt.take();
        for message in backlog {
            let _ = self.server(message);
        }
        // What the server says now counts: a log may hold a question that a
        // restart closed without a trace.
        let current = sessions.iter().find(|s| s.id == self.session.id);
        if let Some(current) = current {
            self.session = current.clone();
        }
        let question = current.and_then(|s| s.question.clone());
        let held = self.prompt.take().or(earlier);
        self.prompt = match question {
            Some(q) if q.choices.is_empty() => None,
            Some(q) => match held {
                Some(prompt) if prompt.id() == q.id && !prompt.sending => Some(prompt),
                _ => {
                    self.open_question(&q);
                    Some(Prompt::new(q))
                }
            },
            None => None,
        };
        self.set_place(sessions);
        if let Some((id, text)) = self.greeting.take() {
            if id == self.session.id {
                self.conversation.notice(text);
            } else {
                self.greeting = Some((id, text));
            }
        }
    }

    /// Keeps the view of the session the TUI leaves and takes the one of the
    /// session it goes to, as it was left or a fresh one.
    fn leave_for(&mut self, id: u64) {
        let (expanded, queued) = self.conversation.take_choices();
        let left = View {
            top: self.top,
            follow: self.follow,
            focus: self.focus,
            selected: self.selected,
            expanded,
            queued,
            editor: std::mem::take(&mut self.editor),
        };
        // An ended session has nothing to come back to.
        if self.ended.is_none() {
            self.views.insert(self.session.id, left);
        }
        let view = self.views.remove(&id).unwrap_or_default();
        self.top = view.top;
        self.follow = view.follow;
        self.focus = view.focus;
        self.selected = view.selected;
        self.conversation.set_choices(view.expanded, view.queued);
        self.editor = view.editor;
        self.prompt = None;
        self.dialog = None;
        self.picker = None;
        self.session_list = None;
        self.list_wanted = false;
        self.command_list = None;
        self.status_box = None;
        self.ended = None;
        self.new_output = false;
        self.reveal = None;
        self.resume_follow = false;
        self.width = None;
        self.last_interrupt = None;
    }

    /// The last `list`, for the session row, the session list and switching.
    fn set_place(&mut self, sessions: Vec<SessionInfo>) {
        // Views of sessions that are gone are dropped.
        self.views
            .retain(|id, _| sessions.iter().any(|s| s.id == *id));
        self.sessions = sessions;
        let sessions = &self.sessions;
        let Some(at) = sessions.iter().position(|s| s.id == self.session.id) else {
            return;
        };
        self.place = Some(Place {
            position: at + 1,
            total: sessions.len(),
            waiting: sessions
                .iter()
                .filter(|s| s.id != self.session.id && s.question.is_some())
                .count(),
        });
    }

    // Server messages.

    fn server(&mut self, message: ServerMessage) -> Vec<Effect> {
        match message {
            ServerMessage::Event {
                session,
                replay,
                event,
            } if session == self.session.id => {
                self.conversation.stamp = if replay { None } else { self.stamp() };
                let ended = matches!(event, SessionEvent::Ended { .. });
                self.event(event, replay);
                // The spec's way on: choose another session or make one.
                if ended && !replay && !self.stopping {
                    let mut effects = self.open_sessions();
                    effects.push(Effect::Redraw);
                    return effects;
                }
                vec![Effect::Redraw]
            }
            ServerMessage::Notice { notice } => {
                self.conversation.stamp = self.stamp();
                match notice {
                    Notice::ConfigReloaded => {
                        self.conversation.notice("configuration reloaded");
                        return vec![
                            Effect::Redraw,
                            Effect::Request(Request::Models),
                            Effect::ReloadConfig,
                        ];
                    }
                    Notice::ConfigRejected { errors } => {
                        for error in errors {
                            self.conversation
                                .error(format!("configuration rejected: {error}"));
                        }
                    }
                    Notice::Stopping => {
                        self.stopping = true;
                        self.conversation.notice("the server is stopping");
                    }
                    Notice::SessionCreated { .. } => return Vec::new(),
                }
                vec![Effect::Redraw]
            }
            ServerMessage::Error { message } => {
                self.conversation.error(message);
                vec![Effect::Redraw]
            }
            ServerMessage::Event { .. }
            | ServerMessage::Hello(_)
            | ServerMessage::Response { .. } => Vec::new(),
        }
    }

    fn stamp(&self) -> Option<String> {
        self.settings
            .timestamps
            .map(|offset| clock(now_secs(), offset))
    }

    fn event(&mut self, event: SessionEvent, replay: bool) {
        if !replay
            && let Some(open) = &mut self.status_box
            && open.id() == Some(self.session.id)
        {
            match &event {
                SessionEvent::Renamed { name } => {
                    open.name.clone_from(name);
                    open.stale = true;
                }
                SessionEvent::Ended { .. } => self.status_box = None,
                SessionEvent::TextDelta { .. } => {}
                _ => open.stale = true,
            }
        }
        let conversation = &mut self.conversation;
        match event {
            SessionEvent::State { state } => self.session.state = state,
            SessionEvent::UserMessage { text } => {
                conversation.user(&text);
                (self.requests, self.tool_calls) = (0, 0);
            }
            SessionEvent::TextDelta { text } => conversation.text_delta(&text),
            SessionEvent::TextDone { text } => conversation.text_done(&text),
            SessionEvent::ToolCall(call) => {
                if call.state.is_final() && conversation.entry_of_call(&call.id).is_some() {
                    self.tool_calls += 1;
                }
                conversation.tool_call(&call);
            }
            SessionEvent::Question(question) => {
                self.open_question(&question);
                if !replay && !question.choices.is_empty() {
                    self.prompt = Some(Prompt::new(*question));
                }
            }
            SessionEvent::QuestionClosed { id, closed } => self.question_closed(&id, &closed),
            SessionEvent::Notice { text } => conversation.notice(text),
            SessionEvent::Usage { context_tokens, .. } => {
                self.context_tokens = context_tokens;
                self.usage.get_or_insert_default().count(&event);
                self.requests += 1;
            }
            SessionEvent::Error { message } => conversation.error(message),
            SessionEvent::Retry {
                attempt,
                max,
                reason,
            } => conversation.notice(crate::agent::retry_text(attempt, max, &reason)),
            SessionEvent::TurnEnd(_) => conversation.close_text(),
            SessionEvent::ModelChanged { from, model } => {
                conversation.notice(if from.is_empty() {
                    format!("model: {model}")
                } else {
                    format!("model: {from} → {model}")
                });
                self.session.model = model;
            }
            SessionEvent::Renamed { name } => {
                conversation.notice(format!("session renamed to {name}"));
                self.session.name = name;
            }
            SessionEvent::Ended { reason } => {
                conversation.notice(format!("session ended: {reason}"));
                self.ended = Some(reason);
                self.prompt = None;
                self.dialog = None;
            }
        }
    }

    /// Shows the call of a permission question; a limit question has none.
    fn open_question(&mut self, question: &crate::agent::Question) {
        match &question.kind {
            QuestionKind::Permission(p) => {
                self.conversation
                    .set_details(&p.call.id, p.call.tool.name(), &p.details);
                if !question.choices.is_empty() {
                    self.reveal = self.conversation.entry_of_call(&p.call.id);
                }
            }
            QuestionKind::AskUser(a) if !question.choices.is_empty() => {
                self.reveal = self.conversation.entry_of_call(&a.call.id);
            }
            _ => {}
        }
    }

    fn question_closed(&mut self, id: &str, closed: &Closed) {
        let open = self.prompt.as_ref().is_some_and(|p| p.id() == id);
        if !open {
            return;
        }
        self.prompt = None;
        if std::mem::take(&mut self.resume_follow) {
            self.follow = true;
        }
        match closed {
            Closed::Answered { answer, by } if *by != self.settings.client => {
                let choice = answer.choice().name();
                self.conversation
                    .notice(format!("answered by {by}: {choice}"));
            }
            Closed::Answered { .. } | Closed::Cancelled => {}
            Closed::NoOne => self.conversation.notice("question closed"),
        }
    }

    // Responses.

    fn response(&mut self, request: Request, result: Result<Payload, String>) -> Vec<Effect> {
        let lost = matches!(&result, Err(message) if message == LOST);
        match (request, result) {
            (Request::List, Ok(Payload::Sessions { sessions })) => {
                let before = (self.place, self.sessions.clone());
                self.set_place(sessions);
                let wanted = std::mem::take(&mut self.list_wanted);
                if let Some(list) = &mut self.session_list {
                    list.set_sessions(self.sessions.clone(), now_secs());
                } else if wanted
                    && self.picker.is_none()
                    && self.dialog.is_none()
                    && self.settings_box.is_none()
                    && self.theme_list.is_none()
                {
                    self.show_sessions();
                } else if before == (self.place, self.sessions.clone()) {
                    return Vec::new();
                }
            }
            (Request::New { .. }, Ok(Payload::Session { session })) => {
                return self.created(&session);
            }
            (Request::SessionStatus { session }, result) => {
                let Some(open) = self.status_box.as_mut().filter(|b| b.name == session) else {
                    return Vec::new();
                };
                open.asking = false;
                match result {
                    Ok(Payload::SessionStatus { status }) => {
                        open.name.clone_from(&status.session.name);
                        open.status = Some(status);
                    }
                    // A refresh that fails keeps what is shown; a first
                    // answer that fails closes the box.
                    Err(message) if open.status.is_none() => {
                        self.status_box = None;
                        if !lost {
                            self.conversation.error(message);
                        }
                    }
                    _ => {}
                }
            }
            (Request::Send { text, .. }, Err(message)) => {
                self.conversation.unqueue(&text);
                if self.editor.is_empty() {
                    self.editor.set_text(&text);
                }
                if !lost {
                    self.conversation.error(message);
                }
            }
            (Request::Answer { .. }, Err(message)) => {
                if let Some(prompt) = &mut self.prompt {
                    if lost {
                        prompt.sending = false;
                    } else {
                        prompt.rejected(message);
                    }
                }
            }
            (Request::List, Err(_)) => {
                self.list_wanted = false;
                return Vec::new();
            }
            (Request::Models, Ok(Payload::Models { models, config })) => {
                self.models_asked = false;
                self.models(models, config);
            }
            (Request::Models, Err(message)) => {
                self.models_asked = false;
                // A refresh after a reload fails quietly; the old list stays.
                if self.picker_wanted.take().is_none() || lost {
                    return Vec::new();
                }
                self.conversation.error(message);
            }
            // A refused name or a failed kill is shown where it was asked.
            (_, Err(message)) if !lost && self.session_list.is_some() => {
                if let Some(list) = &mut self.session_list {
                    list.message = Some(message);
                }
            }
            (_, Err(message)) if !lost => self.conversation.error(message),
            _ => return Vec::new(),
        }
        vec![Effect::Redraw]
    }

    /// The server's models: its context windows count from now on, and the
    /// picker opens if it was asked for.
    fn models(&mut self, models: Vec<ModelInfo>, config: PathBuf) {
        self.settings.windows = models
            .iter()
            .map(|m| (m.model.clone(), m.context_window))
            .collect();
        self.models.clone_from(&models);
        let wanted = self.picker_wanted.take();
        if let Some(picker) = &mut self.picker {
            picker.set_models(models, config);
        } else if let Some(purpose) = wanted
            && self.dialog.is_none()
            && self.session_list.is_none()
        {
            self.picker = Some(Picker::new(models, config, &self.session.model, purpose));
        }
    }

    fn open_picker(&mut self, purpose: Purpose) -> Vec<Effect> {
        self.picker_wanted = Some(purpose);
        vec![Effect::Request(Request::Models)]
    }

    /// The model picker for a new session in the attached session's
    /// workspace.
    fn new_session(&mut self, name: Option<String>) -> Vec<Effect> {
        let workspace = self.session.path.clone();
        self.open_picker(Purpose::New { workspace, name })
    }

    /// A new session was made: attach to it, with a note if others work in
    /// the same workspace, as `clai new` prints.
    fn created(&mut self, session: &SessionInfo) -> Vec<Effect> {
        let others: Vec<&str> = self
            .sessions
            .iter()
            .filter(|s| s.path == session.path && s.id != session.id)
            .map(|s| s.name.as_str())
            .collect();
        if !others.is_empty() {
            let verb = if others.len() == 1 { "works" } else { "work" };
            let text = format!(
                "note: {} also {verb} in {}; sessions may change the same files",
                others.join(", "),
                session.workspace
            );
            self.greeting = Some((session.id, text));
        }
        let mut effects = self.switch(&session.name);
        effects.push(Effect::Redraw);
        effects
    }

    // Sessions.

    /// `/status`: the box opens at once and fills with the answer.
    fn open_status(&mut self, name: String) -> Vec<Effect> {
        self.status_box = Some(StatusBox::new(name.clone()));
        vec![Effect::Request(Request::SessionStatus { session: name })]
    }

    /// A key while the status box is open: it scrolls and closes.
    fn status_box_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let effects = vec![Effect::Redraw];
        let Some(open) = &mut self.status_box else {
            return effects;
        };
        let page = isize::try_from(self.view_height.max(2) - 1).unwrap_or(isize::MAX);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => self.status_box = None,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.status_box = None;
            }
            KeyCode::Up => open.scroll(-1),
            KeyCode::Down => open.scroll(1),
            KeyCode::PageUp => open.scroll(-page),
            KeyCode::PageDown => open.scroll(page),
            KeyCode::Home => open.top = 0,
            KeyCode::End => open.top = open.max_top,
            _ => {}
        }
        effects
    }

    /// Asks for the sessions; the answer opens the list.
    fn open_sessions(&mut self) -> Vec<Effect> {
        self.list_wanted = true;
        vec![Effect::Request(Request::List)]
    }

    /// Opens the session list on the last `list`. After the attached session
    /// ended, and at start for `clai attach` without a name, `Esc` leaves.
    pub fn show_sessions(&mut self) {
        let mut list = SessionList::new(self.sessions.clone(), &self.session, now_secs());
        list.exit_on_esc = self.ended.is_some();
        self.session_list = Some(list);
    }

    /// `clai attach` with several sessions: the list over the first one;
    /// `Esc` there leaves.
    pub fn choose_session(&mut self) {
        self.show_sessions();
        if let Some(list) = &mut self.session_list {
            list.exit_on_esc = true;
        }
    }

    /// Attaches to another session; nothing for the attached one, or while a
    /// switch or a reconnect is on its way.
    fn switch(&mut self, name: &str) -> Vec<Effect> {
        if name == self.session.name && self.ended.is_none() {
            return Vec::new();
        }
        if self.lost_since.is_some() {
            self.message = Some("server disconnected".to_owned());
            return Vec::new();
        }
        if self.switching.is_some() {
            return Vec::new();
        }
        self.switching = Some(name.to_owned());
        vec![Effect::Switch(name.to_owned())]
    }

    /// `Ctrl-N` / `Ctrl-P`: the next or previous session of the last `list`,
    /// round the end.
    fn cycle(&mut self, forward: bool) -> Vec<Effect> {
        let count = self.sessions.len();
        let Some(at) = self.sessions.iter().position(|s| s.id == self.session.id) else {
            return Vec::new();
        };
        if count < 2 {
            return Vec::new();
        }
        let next = if forward {
            (at + 1) % count
        } else {
            (at + count - 1) % count
        };
        match self.sessions.get(next) {
            Some(session) => {
                let name = session.name.clone();
                self.switch(&name)
            }
            None => Vec::new(),
        }
    }

    fn session_list_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let mut effects = vec![Effect::Redraw];
        let Some(list) = &mut self.session_list else {
            return effects;
        };
        let Some(action) = list.key(key) else {
            return effects;
        };
        match action {
            ListAction::Close => self.session_list = None,
            ListAction::Exit => return vec![self.leave()],
            ListAction::Attach(name) => {
                if name == self.session.name && self.ended.is_none() {
                    self.session_list = None;
                } else {
                    effects.extend(self.switch(&name));
                }
            }
            ListAction::Rename { session, name } => {
                effects.push(Effect::Request(Request::Rename { session, name }));
            }
            ListAction::Kill(session) => {
                effects.push(Effect::Request(Request::Kill { session }));
            }
            ListAction::New { workspace, name } => {
                self.session_list = None;
                effects.extend(self.open_picker(Purpose::New { workspace, name }));
            }
        }
        effects
    }

    /// Leaving the TUI: detached, or the session had ended.
    fn leave(&self) -> Effect {
        match &self.ended {
            Some(reason) => Effect::Exit(Exit::Ended {
                name: self.session.name.clone(),
                reason: reason.clone(),
            }),
            None => self.detach(),
        }
    }

    // Time and connection.

    fn tick(&mut self) -> Vec<Effect> {
        self.ticks = self.ticks.wrapping_add(1);
        let mut effects = Vec::new();
        // After the session ended only the session list needs it.
        if self.ticks.is_multiple_of(LIST_TICKS)
            && self.lost_since.is_none()
            && (self.ended.is_none() || self.session_list.is_some())
        {
            effects.push(Effect::Request(Request::List));
        }
        if self.conversation.tick() {
            effects.push(Effect::Redraw);
        }
        if self.lost_since.is_none()
            && let Some(open) = &mut self.status_box
            && open.due()
        {
            open.asking = true;
            open.stale = false;
            effects.push(Effect::Request(Request::SessionStatus {
                session: open.name.clone(),
            }));
        }
        effects
    }

    fn disconnected(&mut self) -> Vec<Effect> {
        // A session that ended has nothing to come back to.
        if self.ended.is_some() {
            return vec![Effect::Redraw];
        }
        self.lost_since = Some(self.now);
        vec![Effect::Reconnect, Effect::Redraw]
    }

    fn reconnect_failed(&mut self) -> Vec<Effect> {
        match self.lost_since {
            Some(since) if self.now.duration_since(since) >= RECONNECT_FOR => {
                vec![Effect::Exit(Exit::Lost)]
            }
            Some(_) => vec![Effect::Reconnect],
            None => Vec::new(),
        }
    }

    // Keys.

    fn key(&mut self, key: KeyEvent) -> Vec<Effect> {
        if key.kind == KeyEventKind::Release {
            return Vec::new();
        }
        self.message = None;
        if self.prompt.is_some() {
            // Opened over the prompt with `Ctrl-S`.
            if self.picker.is_some() {
                return self.picker_key(key);
            }
            if self.session_list.is_some() {
                return self.session_list_key(key);
            }
            return self.prompt_key(key);
        }
        if self.status_box.is_some() {
            return self.status_box_key(key);
        }
        if self.theme_list.is_some() {
            return self.theme_list_key(key);
        }
        if self.picker.is_some() {
            return self.picker_key(key);
        }
        if self.settings_box.is_some() {
            return self.settings_key(key);
        }
        if self.session_list.is_some() {
            return self.session_list_key(key);
        }
        if self.ended.is_some() {
            return vec![self.leave()];
        }
        if let Some(dialog) = self.dialog {
            return self.dialog_key(dialog, key);
        }
        let action = keys::action(key, self.focus);
        let mut effects = if self.command_list.is_some() {
            self.command_list_act(action)
        } else if self.argument_list.is_some() {
            self.argument_list_act(action)
        } else {
            self.act(action)
        };
        effects.push(Effect::Redraw);
        effects
    }

    /// A key while a question is open. Besides the prompt's own keys, only
    /// what answers nothing works: scrolling, to read what is asked,
    /// `Ctrl-S` / `Ctrl-N` / `Ctrl-P` to go to another session and `Ctrl-D`
    /// to leave; the question stays open for later or another client.
    fn prompt_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let ask = self.prompt.as_ref().is_some_and(Prompt::is_ask);
        if let Some(action) = keys::scrolling(key).filter(|_| !ask) {
            let mut effects = self.act(action);
            effects.push(Effect::Redraw);
            return effects;
        }
        if let action @ (Action::SessionList | Action::NextSession | Action::PreviousSession) =
            keys::action(key, self.focus)
        {
            let mut effects = self.act(action);
            effects.push(Effect::Redraw);
            return effects;
        }
        let choosing = self
            .prompt
            .as_ref()
            .is_some_and(|p| p.mode == super::prompt::Mode::Choices && !p.is_typing());
        if choosing && keys::is_detach(key) {
            return vec![self.detach()];
        }
        let mut effects = vec![Effect::Redraw];
        if let Some(prompt) = &mut self.prompt
            && let Some(answer) = prompt.key(key)
        {
            effects.push(Effect::Request(Request::Answer {
                session: self.session.name.clone(),
                question: prompt.id().to_owned(),
                answer,
            }));
        }
        effects
    }

    fn picker_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let mut effects = vec![Effect::Redraw];
        let Some(picker) = &mut self.picker else {
            return effects;
        };
        match key.code {
            KeyCode::Esc => self.picker = None,
            KeyCode::Up => picker.up(),
            KeyCode::Down => picker.down(),
            KeyCode::Enter => {
                let Some(model) = picker.chosen().map(|m| m.model.clone()) else {
                    return effects;
                };
                let purpose = picker.purpose.clone();
                self.picker = None;
                match purpose {
                    Purpose::Change if model != self.session.model => {
                        effects.push(Effect::Request(Request::SetModel {
                            session: self.session.name.clone(),
                            model,
                        }));
                    }
                    Purpose::Change => {}
                    Purpose::New { workspace, name } => {
                        effects.push(Effect::Request(Request::New {
                            workspace,
                            name,
                            model: Some(model),
                        }));
                    }
                }
            }
            _ => {
                if let Some(edit) = keys::edit(key) {
                    picker.edit(edit);
                }
            }
        }
        effects
    }

    /// The commands the list shows now.
    pub fn listed(&self) -> Vec<&'static Command> {
        commands::search_text(&self.editor.text()).map_or_else(Vec::new, commands::search)
    }

    /// An action while the command list is open. What the list does not take
    /// works as without it.
    fn command_list_act(&mut self, action: Action) -> Vec<Effect> {
        let listed = self.listed();
        let selected = self.command_list.unwrap_or(0);
        let chosen = listed.get(selected).copied();
        match action {
            Action::Cancel => self.command_list = None,
            Action::Up => self.command_list = Some(selected.saturating_sub(1)),
            Action::Down => {
                let last = listed.len().saturating_sub(1);
                self.command_list = Some((selected + 1).min(last));
            }
            Action::SwitchFocus => {
                if let Some(command) = chosen {
                    self.complete(command);
                }
            }
            Action::Send => match chosen {
                Some(command) if command.built && command.needs_argument => {
                    self.complete(command);
                }
                Some(command) => {
                    self.command_list = None;
                    return self.command(command.name);
                }
                None => {}
            },
            Action::ModelPicker | Action::SessionList => {
                self.command_list = None;
                return self.act(action);
            }
            _ => return self.act(action),
        }
        Vec::new()
    }

    /// Puts the command in the input, for its arguments, and closes the list;
    /// the argument list opens for the commands it completes.
    fn complete(&mut self, command: &Command) {
        self.editor.set_text(&command.completed());
        self.command_list = None;
        if commands::argument_text(&self.editor.text()).is_some() {
            self.argument_list = Some(0);
        }
    }

    /// Changes the input. `/` typed in the empty input opens the command list,
    /// and a new search selects its first row. The argument list opens when
    /// the input becomes `/attach ` or `/model `, and is searched the same way.
    fn edit_input(&mut self, edit: impl FnOnce(&mut Editor), typed_slash: bool) {
        let before = self.editor.text();
        edit(&mut self.editor);
        let after = self.editor.text();
        let opened = before.is_empty() && typed_slash;
        let searched = self.command_list.is_some() && before != after;
        if opened || searched {
            self.command_list = Some(0);
        }
        let arguments = commands::argument_text(&after).is_some();
        let entered = arguments && commands::argument_text(&before).is_none();
        if entered || (arguments && self.argument_list.is_some() && before != after) {
            self.argument_list = Some(0);
        }
    }

    /// The values the argument list shows now.
    pub fn arguments(&self) -> Vec<commands::Value> {
        let text = self.editor.text();
        let Some((command, typed)) = commands::argument_text(&text) else {
            return Vec::new();
        };
        let values = if command == "theme" {
            themes::entries(&self.settings.theming.themes, &[])
                .into_iter()
                .map(|e| commands::Value {
                    description: if e.note.is_empty() {
                        e.group.heading().to_owned()
                    } else {
                        e.note
                    },
                    value: e.name,
                    search_description: false,
                })
                .collect()
        } else if command == "attach" || command == "status" {
            let symbols = &self.settings.look.symbols;
            // `/status` shows the attached session too.
            self.sessions
                .iter()
                .filter(|s| command == "status" || s.id != self.session.id)
                .map(|s| commands::Value {
                    value: s.name.clone(),
                    description: format!(
                        "{} {} {}",
                        s.workspace,
                        symbols.separator,
                        crate::text::sessions::state_name(s.state)
                    ),
                    search_description: false,
                })
                .collect()
        } else {
            self.models
                .iter()
                .map(|m| commands::Value {
                    value: m.model.clone(),
                    description: m.id.clone(),
                    search_description: true,
                })
                .collect()
        };
        commands::search_values(values, typed)
    }

    /// An action while the argument list is open; as for the command list.
    fn argument_list_act(&mut self, action: Action) -> Vec<Effect> {
        let values = self.arguments();
        let selected = self.argument_list.unwrap_or(0);
        let chosen = values.get(selected).map(|v| v.value.clone());
        let fill = |app: &mut App, value: &str| {
            if let Some((command, _)) = commands::argument_text(&app.editor.text()) {
                app.editor.set_text(&format!("/{command} {value}"));
            }
        };
        match action {
            Action::Cancel => self.argument_list = None,
            Action::Up => self.argument_list = Some(selected.saturating_sub(1)),
            Action::Down => {
                let last = values.len().saturating_sub(1);
                self.argument_list = Some((selected + 1).min(last));
            }
            Action::SwitchFocus => {
                if let Some(value) = chosen {
                    fill(self, &value);
                    self.argument_list = None;
                }
            }
            Action::Send => {
                if let Some(value) = chosen {
                    fill(self, &value);
                }
                self.argument_list = None;
                return self.send();
            }
            Action::ModelPicker | Action::SessionList => {
                self.argument_list = None;
                return self.act(action);
            }
            _ => return self.act(action),
        }
        Vec::new()
    }

    fn dialog_key(&mut self, dialog: Dialog, key: KeyEvent) -> Vec<Effect> {
        let mut effects = vec![Effect::Redraw];
        match dialog {
            Dialog::Help => self.dialog = None,
            Dialog::ConfirmKill => match key.code {
                KeyCode::Char('y' | 'Y') => {
                    self.dialog = None;
                    effects.push(Effect::Request(Request::Kill {
                        session: self.session.name.clone(),
                    }));
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc => self.dialog = None,
                _ => {}
            },
        }
        effects
    }

    fn detach(&self) -> Effect {
        Effect::Exit(Exit::Detached {
            name: self.session.name.clone(),
        })
    }

    fn running(&self) -> bool {
        matches!(self.session.state, State::Working | State::Waiting)
    }

    fn cancel(&self) -> Vec<Effect> {
        if self.running() {
            vec![Effect::Request(Request::Cancel {
                session: self.session.name.clone(),
            })]
        } else {
            Vec::new()
        }
    }

    fn act(&mut self, action: Action) -> Vec<Effect> {
        match action {
            Action::Cancel => return self.cancel(),
            Action::Interrupt => return self.interrupt(),
            Action::Detach if self.editor.is_empty() => return vec![self.detach()],
            Action::PageUp => self.scroll_up(self.page()),
            Action::PageDown => self.scroll_down(self.page()),
            Action::Start => {
                self.follow = false;
                self.top = 0;
            }
            Action::End => {
                self.follow = true;
                self.new_output = false;
            }
            Action::SwitchFocus => self.switch_focus(),
            Action::Send => return self.send(),
            Action::Edit(edit) => {
                self.edit_input(|editor| editor.edit(edit), edit == Edit::Char('/'));
            }
            Action::Up => self.up(),
            Action::Down => self.down(),
            Action::Toggle => {
                if let Some(entry) = self.selected
                    && self.conversation.toggle(entry)
                {
                    self.reveal = Some(entry);
                }
            }
            Action::Help => self.dialog = Some(Dialog::Help),
            Action::ModelPicker => return self.open_picker(Purpose::Change),
            Action::SessionList => return self.open_sessions(),
            Action::NextSession => return self.cycle(true),
            Action::PreviousSession => return self.cycle(false),
            Action::Detach | Action::Nothing => {}
        }
        Vec::new()
    }

    /// `Ctrl-C`: twice within 1 s detaches; else it cancels the running turn
    /// or, with none, clears the input.
    fn interrupt(&mut self) -> Vec<Effect> {
        if self
            .last_interrupt
            .is_some_and(|at| self.now.duration_since(at) < DOUBLE_INTERRUPT)
        {
            return vec![self.detach()];
        }
        self.last_interrupt = Some(self.now);
        if self.running() {
            return self.cancel();
        }
        self.editor.clear();
        Vec::new()
    }

    fn switch_focus(&mut self) {
        self.focus = match self.focus {
            Focus::Input => {
                if self.selected.is_none() {
                    self.selected = self.conversation.entries.len().checked_sub(1);
                }
                self.reveal = self.selected;
                Focus::Conversation
            }
            Focus::Conversation => Focus::Input,
        };
    }

    fn up(&mut self) {
        match self.focus {
            Focus::Input => {
                if !self.editor.up() {
                    let history = self.conversation.user_messages();
                    self.editor.browse(&history, Browse::Earlier);
                }
            }
            Focus::Conversation => {
                self.selected = self.selected.map(|s| s.saturating_sub(1));
                self.reveal = self.selected;
            }
        }
    }

    fn down(&mut self) {
        match self.focus {
            Focus::Input => {
                if !self.editor.down() {
                    let history = self.conversation.user_messages();
                    self.editor.browse(&history, Browse::Later);
                }
            }
            Focus::Conversation => {
                let last = self.conversation.entries.len().saturating_sub(1);
                self.selected = self.selected.map(|s| (s + 1).min(last));
                self.reveal = self.selected;
            }
        }
    }

    fn paste(&mut self, text: &str) -> Vec<Effect> {
        if let Some(prompt) = &mut self.prompt {
            prompt.paste(text);
        } else if let Some(list) = &mut self.theme_list {
            list.paste(text);
        } else if let Some(picker) = &mut self.picker {
            picker.paste(text);
        } else if self.settings_box.is_some() {
            // Nothing to paste into.
        } else if let Some(list) = &mut self.session_list {
            list.paste(text);
        } else if self.dialog.is_none() && self.ended.is_none() && self.focus == Focus::Input {
            self.edit_input(|editor| editor.paste(text), false);
        }
        vec![Effect::Redraw]
    }

    fn mouse(&mut self, mouse: MouseEvent) -> Vec<Effect> {
        let step = self.settings.scroll_lines;
        match mouse.kind {
            MouseEventKind::ScrollUp => self.scroll_up(step),
            MouseEventKind::ScrollDown => self.scroll_down(step),
            _ => return Vec::new(),
        }
        vec![Effect::Redraw]
    }

    // Sending and commands.

    fn send(&mut self) -> Vec<Effect> {
        let text = self.editor.text();
        if text.trim().is_empty() {
            return Vec::new();
        }
        if let Some(rest) = text.strip_prefix("//") {
            let message = format!("/{rest}");
            return self.send_message(message);
        }
        if let Some(command) = text.strip_prefix('/') {
            return self.command(command.trim());
        }
        self.send_message(text)
    }

    fn send_message(&mut self, text: String) -> Vec<Effect> {
        if self.lost_since.is_some() {
            self.message = Some("server disconnected; the input is kept".to_owned());
            return Vec::new();
        }
        self.editor.clear();
        self.follow = true;
        self.new_output = false;
        self.conversation.queued.push(text.clone());
        vec![Effect::Request(Request::Send {
            session: self.session.name.clone(),
            text,
        })]
    }

    fn command(&mut self, command: &str) -> Vec<Effect> {
        let (name, argument) = command
            .split_once(char::is_whitespace)
            .map_or((command, ""), |(n, a)| (n, a.trim()));
        let session = self.session.name.clone();
        let effects = match (name, argument) {
            ("help", _) => {
                self.dialog = Some(Dialog::Help);
                Vec::new()
            }
            ("detach" | "quit", _) => {
                self.editor.clear();
                return vec![self.detach()];
            }
            ("model", "") => self.open_picker(Purpose::Change),
            ("model", model) => vec![Effect::Request(Request::SetModel {
                session,
                model: model.to_owned(),
            })],
            ("rename", "") => {
                self.conversation.notice("usage: /rename NAME");
                return Vec::new();
            }
            ("rename", name) => vec![Effect::Request(Request::Rename {
                session,
                name: name.to_owned(),
            })],
            ("kill", _) => {
                self.dialog = Some(Dialog::ConfirmKill);
                Vec::new()
            }
            ("sessions" | "attach", "") => self.open_sessions(),
            ("attach", name) => {
                // A name not in the last list stays in the input, to correct.
                if !self.sessions.iter().any(|s| s.name == name) {
                    self.conversation
                        .notice(format!("no session named {name}; /sessions lists them"));
                    return Vec::new();
                }
                self.switch(name)
            }
            // Theme files may have changed on their own: read them again.
            ("settings", _) => {
                self.open_settings();
                vec![Effect::ReloadConfig]
            }
            ("theme", "") => {
                self.open_themes(false);
                vec![Effect::ReloadConfig]
            }
            ("theme", name) => match self.theme_command(name) {
                Some(effects) => effects,
                // The input stays, so that it can be corrected.
                None => return Vec::new(),
            },
            ("status", "") => self.open_status(session),
            ("status", name) => {
                if !self.sessions.iter().any(|s| s.name == name) && name != session {
                    self.conversation
                        .notice(format!("no session named {name}; /sessions lists them"));
                    return Vec::new();
                }
                self.open_status(name.to_owned())
            }
            ("new", "") => self.new_session(None),
            ("new", name) => self.new_session(Some(name.to_owned())),
            ("clear" | "permissions" | "commit", _) => {
                self.conversation
                    .notice(format!("`/{name}` comes with a later version"));
                Vec::new()
            }
            (other, _) => {
                // The input stays, so that it can be corrected.
                self.conversation.notice(format!(
                    "unknown command /{other}; /help lists them, // sends a message that starts with /"
                ));
                return Vec::new();
            }
        };
        self.editor.clear();
        effects
    }

    // Scrolling.

    fn page(&self) -> usize {
        self.view_height.saturating_sub(1).max(1)
    }

    fn max_top(&self) -> usize {
        self.total.unwrap_or(0).saturating_sub(self.view_height)
    }

    fn scroll_up(&mut self, rows: usize) {
        self.follow = false;
        self.resume_follow = false;
        self.top = self.top.saturating_sub(rows);
    }

    fn scroll_down(&mut self, rows: usize) {
        self.top = self.top.saturating_add(rows);
        if self.top >= self.max_top() {
            self.follow = true;
            self.new_output = false;
        }
    }

    /// Brings the lines and the scroll position up to date for a screen of
    /// `size`. Runs before each frame; drawing then only reads.
    pub fn layout(&mut self, size: Rect) {
        let max_top = view::status_max_top(self, size);
        if let Some(open) = &mut self.status_box {
            open.max_top = max_top;
            open.top = open.top.min(max_top);
        }
        let Some(areas) = view::areas(self, size) else {
            return;
        };
        let width = usize::from(areas.conversation.width);
        self.view_height = usize::from(areas.conversation.height);
        // Keep the entry at the top when the width changes the row numbers.
        let anchor = (self.width.is_some_and(|w| w != width))
            .then(|| self.conversation.entry_at(self.top))
            .flatten();
        self.conversation.layout(width);
        self.width = Some(width);
        if let Some(entry) = anchor
            && let Some((start, _)) = self.conversation.span_of(entry)
        {
            self.top = start;
        }
        let total = self.conversation.total();
        if self.total.is_some_and(|before| total > before) && !self.follow {
            self.new_output = true;
        }
        self.total = Some(total);
        let was_following = self.follow;
        if self.follow {
            self.top = self.max_top();
        }
        if let Some(entry) = self.reveal.take() {
            self.show_entry(entry);
            if was_following && !self.follow && self.prompt.is_some() {
                self.resume_follow = true;
            }
        }
        self.top = self.top.min(self.max_top());
    }

    /// Scrolls as little as needed to show an entry, its start first.
    fn show_entry(&mut self, entry: usize) {
        let Some((start, height)) = self.conversation.span_of(entry) else {
            return;
        };
        let view = self.view_height;
        if start < self.top || height > view {
            self.top = start;
        } else if start + height > self.top + view {
            self.top = start + height - view;
        }
        self.follow = self.top >= self.max_top();
    }

    /// Whether the selected entry is a tool call.
    pub fn selected_is_call(&self) -> bool {
        self.selected
            .and_then(|s| self.conversation.entries.get(s))
            .is_some_and(|e| matches!(e.kind, Kind::Call(_)))
    }
}

/// `HH:MM` of a time in seconds since 1970, `offset` seconds from UTC.
pub fn clock(secs: u64, offset: i64) -> String {
    let secs = i64::try_from(secs).unwrap_or(0).saturating_add(offset);
    let minutes = secs.div_euclid(60);
    let (h, m) = (
        minutes.div_euclid(60).rem_euclid(24),
        minutes.rem_euclid(60),
    );
    format!("{h:02}:{m:02}")
}
