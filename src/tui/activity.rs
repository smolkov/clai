//! The state of the activity line (see `spec/tui.md`, "Activity line",
//! `rfcs/0015-activity-line.md`): when the turn started, what the model
//! does, the verb of the turn, the tokens and the animation frame. Pure: `App`
//! feeds it events and frames, `view::activity` draws it.

use std::time::{Duration, Instant};

use crate::agent::{SessionEvent, State};

/// No event for this long while waiting for the model: the glyph warns.
pub const STALLED_AFTER: Duration = Duration::from_secs(30);
/// Characters of text per output token, for the estimate between `usage`
/// events.
const CHARS_PER_TOKEN: u64 = 4;

/// One verb is chosen for each turn.
pub const VERBS: &[&str] = &[
    "Pondering",
    "Prestidigitating",
    "Untangling",
    "Percolating",
    "Conjuring",
    "Mulling",
    "Ruminating",
    "Cogitating",
    "Deliberating",
    "Brewing",
    "Simmering",
    "Marinating",
    "Noodling",
    "Tinkering",
    "Whittling",
    "Puzzling",
    "Musing",
    "Scheming",
    "Divining",
    "Distilling",
    "Weaving",
    "Spelunking",
    "Rummaging",
    "Sleuthing",
    "Deciphering",
    "Unravelling",
    "Contemplating",
    "Incubating",
    "Fermenting",
    "Kneading",
    "Sculpting",
    "Polishing",
    "Juggling",
    "Herding bits",
    "Wrangling",
    "Calibrating",
    "Synthesizing",
    "Extrapolating",
    "Hypothesizing",
    "Reticulating",
    "Daydreaming",
    "Meandering",
    "Tracing",
    "Sifting",
    "Gathering",
    "Composing",
    "Assembling",
    "Crafting",
    "Mapping",
    "Moseying",
];

/// What the model does in the turn, besides tools (those come from the
/// conversation's running calls).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// The request is sent; no text yet.
    Thinking,
    /// Text deltas arrive.
    Writing,
    /// After a `retry` event, until the next text or tool call.
    Retry {
        attempt: u32,
        max: u32,
        reason: String,
    },
}

#[derive(Debug, Clone)]
pub struct Activity {
    /// The session is in a turn (`working`, or `waiting` within one).
    pub working: bool,
    /// When the turn started; `None` until known after an attach.
    pub started: Option<Instant>,
    /// A `session_status` request for the start is on its way or answered.
    pub asked_start: bool,
    pub phase: Phase,
    /// `Esc` cancelled the turn; it has not ended yet.
    pub cancelling: bool,
    pub verb: &'static str,
    /// Moves the glyph and the shimmer.
    pub frame: usize,
    /// Output tokens of `usage` events in this turn.
    tokens_counted: u64,
    /// Characters of text since the last `usage` event.
    chars_since_usage: u64,
    /// When the last live event came.
    pub last_event: Option<Instant>,
    /// The state of the random number generator for the verbs.
    seed: u64,
}

impl Activity {
    pub fn new(seed: u64) -> Activity {
        Activity {
            working: false,
            started: None,
            asked_start: false,
            phase: Phase::Thinking,
            cancelling: false,
            verb: VERBS.first().copied().unwrap_or("Working"),
            frame: 0,
            tokens_counted: 0,
            chars_since_usage: 0,
            last_event: None,
            seed,
        }
    }

    /// A new seed for the verbs of later turns; the state stays.
    pub fn reseed(&mut self, seed: u64) {
        self.seed = seed;
    }

    /// Forgets the turn before a replay; the generator goes on.
    pub fn reset(&mut self) {
        *self = Activity {
            frame: self.frame,
            ..Activity::new(self.seed)
        };
    }

    /// Takes an event; `live` with the time it came, `None` in a replay.
    pub fn event(&mut self, event: &SessionEvent, live: Option<Instant>) {
        if live.is_some() {
            self.last_event = live;
        }
        match event {
            SessionEvent::State { state } => match state {
                State::Working | State::Waiting if !self.working => self.start(live),
                State::Working | State::Waiting => {}
                State::Idle | State::Saved => self.working = false,
            },
            SessionEvent::TextDelta { text } => {
                self.phase = Phase::Writing;
                let chars = u64::try_from(text.chars().count()).unwrap_or(u64::MAX);
                self.chars_since_usage = self.chars_since_usage.saturating_add(chars);
            }
            SessionEvent::ToolCall(_) => self.phase = Phase::Thinking,
            SessionEvent::Usage { output_tokens, .. } => {
                self.tokens_counted = self.tokens_counted.saturating_add(*output_tokens);
                self.chars_since_usage = 0;
                // The next request comes, or a tool runs.
                if self.phase == Phase::Writing {
                    self.phase = Phase::Thinking;
                }
            }
            SessionEvent::Retry {
                attempt,
                max,
                reason,
            } => {
                self.phase = Phase::Retry {
                    attempt: *attempt,
                    max: *max,
                    reason: reason.clone(),
                };
            }
            SessionEvent::Ended { .. } => self.working = false,
            _ => {}
        }
    }

    /// A new turn, with a new verb.
    fn start(&mut self, at: Option<Instant>) {
        let seed = self.seed;
        let frame = self.frame;
        *self = Activity::new(seed);
        self.frame = frame;
        self.working = true;
        self.started = at;
        self.last_event = at;
        self.verb = self.next_verb();
    }

    /// splitmix64: small, and enough to vary a word.
    fn next_verb(&mut self) -> &'static str {
        self.seed = self.seed.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        let count = u64::try_from(VERBS.len()).unwrap_or(1).max(1);
        let at = usize::try_from(z % count).unwrap_or(0);
        VERBS.get(at).copied().unwrap_or("Working")
    }

    /// The session works, as the attach found it; the start is not known.
    pub fn attached_working(&mut self, now: Instant) {
        if !self.working {
            self.start(None);
        }
        self.started = None;
        self.asked_start = false;
        self.last_event = Some(now);
    }

    /// The start from `session_status`, if the turn still runs.
    pub fn set_start(&mut self, started: Instant) {
        if self.working {
            self.started = Some(started);
        }
    }

    /// Whether a `session_status` request should ask for the start.
    pub fn wants_start(&self) -> bool {
        self.working && self.started.is_none() && !self.asked_start
    }

    /// `Esc` or `Ctrl-C` cancelled the turn.
    pub fn cancel(&mut self) {
        if self.working {
            self.cancelling = true;
        }
    }

    /// The next animation frame.
    pub fn frame(&mut self) {
        self.frame = self.frame.wrapping_add(1);
    }

    /// Output tokens of the turn: counted, and estimated since the last
    /// count.
    pub fn tokens(&self) -> u64 {
        self.tokens_counted
            .saturating_add(self.chars_since_usage / CHARS_PER_TOKEN)
    }

    /// The time since the turn started.
    pub fn elapsed(&self, now: Instant) -> Option<Duration> {
        self.started.map(|s| now.saturating_duration_since(s))
    }

    /// No event for [`STALLED_AFTER`] while the model is waited for.
    pub fn stalled(&self, now: Instant) -> bool {
        !self.cancelling
            && self.phase == Phase::Thinking
            && self
                .last_event
                .is_some_and(|at| now.saturating_duration_since(at) >= STALLED_AFTER)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{CallState, ToolCallEvent};

    fn state(state: State) -> SessionEvent {
        SessionEvent::State { state }
    }

    fn delta(text: &str) -> SessionEvent {
        SessionEvent::TextDelta { text: text.into() }
    }

    fn usage(output_tokens: u64) -> SessionEvent {
        SessionEvent::Usage {
            input_tokens: 10,
            output_tokens,
            context_tokens: 10 + output_tokens,
            model: String::new(),
        }
    }

    fn tool() -> SessionEvent {
        SessionEvent::ToolCall(ToolCallEvent {
            details: None,
            id: "c1".into(),
            name: "read_file".into(),
            state: CallState::Running,
            summary: None,
            arguments: serde_json::Value::Null,
            output: None,
            result: None,
        })
    }

    fn working(now: Instant) -> Activity {
        let mut a = Activity::new(7);
        a.event(&state(State::Working), Some(now));
        a
    }

    #[test]
    fn the_phases_follow_the_events() {
        let now = Instant::now();
        let mut a = working(now);
        assert!(a.working);
        assert_eq!(a.started, Some(now));
        assert_eq!(a.phase, Phase::Thinking);
        a.event(&delta("Let me look."), Some(now));
        assert_eq!(a.phase, Phase::Writing);
        a.event(&tool(), Some(now));
        assert_eq!(a.phase, Phase::Thinking);
        a.event(
            &SessionEvent::Retry {
                attempt: 2,
                max: 3,
                reason: "rate limited".into(),
            },
            Some(now),
        );
        assert!(matches!(a.phase, Phase::Retry { attempt: 2, .. }));
        a.event(&delta("ok"), Some(now));
        assert_eq!(a.phase, Phase::Writing);
        a.cancel();
        assert!(a.cancelling);
        // A question within the turn keeps it.
        a.event(&state(State::Waiting), Some(now));
        assert!(a.working && a.cancelling);
        a.event(&state(State::Idle), Some(now));
        assert!(!a.working);
        // A new turn starts afresh.
        a.event(&state(State::Working), Some(now));
        assert!(!a.cancelling);
        assert_eq!(a.phase, Phase::Thinking);
        assert_eq!(a.tokens(), 0);
    }

    #[test]
    fn cancel_outside_a_turn_does_nothing() {
        let mut a = Activity::new(1);
        a.cancel();
        assert!(!a.cancelling);
    }

    #[test]
    fn tokens_are_estimated_until_counted() {
        let now = Instant::now();
        let mut a = working(now);
        assert_eq!(a.tokens(), 0);
        a.event(&delta(&"x".repeat(4100)), Some(now));
        assert_eq!(a.tokens(), 1025);
        a.event(&usage(1200), Some(now));
        assert_eq!(a.tokens(), 1200);
        a.event(&delta("abcdefgh"), Some(now));
        assert_eq!(a.tokens(), 1202);
        a.event(&usage(300), Some(now));
        assert_eq!(a.tokens(), 1500);
    }

    #[test]
    fn a_quiet_model_stalls_after_thirty_seconds() {
        let now = Instant::now();
        let mut a = working(now);
        assert!(!a.stalled(now + Duration::from_secs(29)));
        assert!(a.stalled(now + STALLED_AFTER));
        // An event resets it; text arriving is no stall.
        let later = now + Duration::from_secs(40);
        a.event(&delta("a"), Some(later));
        assert!(!a.stalled(later + STALLED_AFTER));
        a.event(&usage(1), Some(later));
        assert!(a.stalled(later + STALLED_AFTER));
        // Replayed events are no sign of life.
        a.event(&delta("b"), None);
        a.event(&usage(1), None);
        assert!(a.stalled(later + STALLED_AFTER));
    }

    #[test]
    fn the_same_seed_gives_the_same_verbs() {
        let now = Instant::now();
        let turns = |seed| {
            let mut a = Activity::new(seed);
            (0..5)
                .map(|_| {
                    a.event(&state(State::Working), Some(now));
                    a.event(&state(State::Idle), Some(now));
                    a.verb
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(turns(42), turns(42));
        let verbs = turns(42);
        // A new turn gives a new choice, not always the same word.
        assert!(verbs.iter().any(|v| *v != verbs[0]), "{verbs:?}");
        assert!(verbs.iter().all(|v| VERBS.contains(v)));
    }

    #[test]
    fn the_verb_stays_for_the_turn() {
        let now = Instant::now();
        let mut a = working(now);
        let verb = a.verb;
        a.event(&delta("a"), Some(now));
        a.event(&state(State::Waiting), Some(now));
        a.event(&state(State::Working), Some(now));
        assert_eq!(a.verb, verb);
    }

    #[test]
    fn an_attach_to_a_working_session_asks_for_the_start() {
        let now = Instant::now();
        let mut a = Activity::new(3);
        // The replay ran the turn's start without times.
        a.event(&state(State::Working), None);
        assert_eq!(a.started, None);
        a.attached_working(now);
        assert!(a.wants_start());
        a.asked_start = true;
        assert!(!a.wants_start());
        let start = now.checked_sub(Duration::from_secs(14)).unwrap_or(now);
        a.set_start(start);
        assert_eq!(a.elapsed(now), Some(now - start));
    }
}
