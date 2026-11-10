//! The state of the status box (see `spec/tui.md`, "Status box"): its two
//! tabs, the last `session_status` answer, the `global_stats` answers by
//! range, the scroll positions, and whether to ask again. `App` asks;
//! `view::status_box` draws it.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use crate::agent::State;
use crate::protocol::{GlobalStats, Range, SessionStatus};

/// From this age on, the title says how old a global answer is.
pub const OLD: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    /// The attached or named session, as `/status` shows it.
    Session,
    /// All sessions of the server, as `/stats` shows them.
    Global,
}

/// A `global_stats` answer and when it came.
#[derive(Debug, Clone, PartialEq)]
pub struct Counted {
    pub stats: Box<GlobalStats>,
    pub at: Instant,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StatusBox {
    pub tab: Tab,
    /// The session shown.
    pub name: String,
    /// `None` until the first answer.
    pub status: Option<Box<SessionStatus>>,
    /// The first line shown of the tab in use.
    pub top: usize,
    /// The largest `top`, from the last frame.
    pub max_top: usize,
    /// The first line of the other tab, kept while it is not shown.
    other_top: usize,
    /// A `session_status` request is on its way.
    pub asking: bool,
    /// An event changed what it shows: ask again at the next tick.
    pub stale: bool,
    /// The range of the Global tab.
    pub range: Range,
    /// The last answer per range.
    pub global: HashMap<Range, Counted>,
    /// The ranges asked for and not answered yet.
    pub global_asking: HashSet<Range>,
    /// The error of a range that has no answer.
    pub global_error: Option<String>,
}

impl StatusBox {
    /// The box on the Session tab; it asks for the session's status.
    pub fn new(name: String) -> StatusBox {
        StatusBox {
            tab: Tab::Session,
            name,
            status: None,
            top: 0,
            max_top: 0,
            other_top: 0,
            asking: true,
            stale: false,
            range: Range::default(),
            global: HashMap::new(),
            global_asking: HashSet::new(),
            global_error: None,
        }
    }

    /// The box on the Global tab with `range`; the session's status is
    /// asked for once its tab is shown.
    pub fn global(name: String, range: Range) -> StatusBox {
        StatusBox {
            tab: Tab::Global,
            asking: false,
            range,
            ..StatusBox::new(name)
        }
    }

    /// Whether to ask for the session's status now: once its tab is shown
    /// without an answer, after an event that changed it, and every tick
    /// while the session works, but never twice at a time.
    pub fn due(&self) -> bool {
        if self.asking || self.tab != Tab::Session {
            return false;
        }
        match &self.status {
            None => true,
            Some(status) => self.stale || status.session.state == State::Working,
        }
    }

    /// Whether to ask for the range shown: it has no answer and none is on
    /// its way.
    pub fn global_due(&self) -> bool {
        self.tab == Tab::Global
            && !self.global.contains_key(&self.range)
            && !self.global_asking.contains(&self.range)
    }

    /// The id of the session shown, once known.
    pub fn id(&self) -> Option<u64> {
        self.status.as_ref().map(|s| s.session.id)
    }

    pub fn scroll(&mut self, by: isize) {
        self.top = self.top.saturating_add_signed(by).min(self.max_top);
    }

    /// Shows the other tab, at the line it was left at.
    pub fn switch(&mut self) {
        self.tab = match self.tab {
            Tab::Session => Tab::Global,
            Tab::Global => Tab::Session,
        };
        std::mem::swap(&mut self.top, &mut self.other_top);
    }

    /// Shows `range` on the Global tab, from its first line.
    pub fn set_range(&mut self, range: Range) {
        if range != self.range {
            self.range = range;
            self.top = 0;
            self.global_error = None;
        }
    }
}
