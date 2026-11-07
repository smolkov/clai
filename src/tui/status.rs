//! The state of the status box (see `spec/tui.md`, "Status box"): the last
//! `session_status` answer, the scroll position, and whether to ask again.
//! `App` asks; `view::status_box` draws it.

use crate::agent::State;
use crate::protocol::SessionStatus;

#[derive(Debug, Clone, PartialEq)]
pub struct StatusBox {
    /// The session shown.
    pub name: String,
    /// `None` until the first answer.
    pub status: Option<Box<SessionStatus>>,
    /// The first line shown.
    pub top: usize,
    /// The largest `top`, from the last frame.
    pub max_top: usize,
    /// A request is on its way.
    pub asking: bool,
    /// An event changed what it shows: ask again at the next tick.
    pub stale: bool,
}

impl StatusBox {
    pub fn new(name: String) -> StatusBox {
        StatusBox {
            name,
            status: None,
            top: 0,
            max_top: 0,
            asking: true,
            stale: false,
        }
    }

    /// Whether to ask again now: after an event that changed it, and every
    /// tick while the session works, but never twice at a time.
    pub fn due(&self) -> bool {
        if self.asking {
            return false;
        }
        self.stale
            || self
                .status
                .as_ref()
                .is_some_and(|s| s.session.state == State::Working)
    }

    /// The id of the session shown, once known.
    pub fn id(&self) -> Option<u64> {
        self.status.as_ref().map(|s| s.session.id)
    }

    pub fn scroll(&mut self, by: isize) {
        self.top = self.top.saturating_add_signed(by).min(self.max_top);
    }
}
