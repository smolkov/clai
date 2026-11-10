//! Statistics over all saved sessions (see `spec/sessions.md`,
//! "Statistics"): each session's log is counted once into a [`Summary`] by
//! quarter hour in UTC, and the summaries are combined for a range of days in
//! the client's time zone.

use std::collections::{BTreeMap, BTreeSet};

use crate::agent::SessionEvent;
use crate::protocol::{
    DayTokens, GlobalStats, GlobalTotals, ModelTotals, Range, SessionCounts, Stats, TokenTotals,
    TurnCounts,
};

use super::store::Store;

/// The model of tokens that cannot be given to one.
pub const UNKNOWN: &str = "unknown";

/// A quarter hour in milliseconds: the smallest step of a time zone.
const QUARTER_MS: u64 = 900_000;
const DAY_SECS: i64 = 86_400;

/// The use of one model in a slot.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Use {
    pub requests: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl Use {
    fn add(&mut self, other: &Use) {
        self.requests = self.requests.saturating_add(other.requests);
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
    }

    fn is_empty(&self) -> bool {
        *self == Use::default()
    }
}

/// What a session did in a quarter hour, or without a time.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Slot {
    pub turns: TurnCounts,
    pub retries: u64,
    pub errors: u64,
    pub working_ms: u64,
    pub unknown_stretches: u64,
    pub models: BTreeMap<String, Use>,
}

impl Slot {
    fn tokens(&self) -> TokenTotals {
        let mut tokens = TokenTotals::default();
        for used in self.models.values() {
            tokens.input_tokens = tokens.input_tokens.saturating_add(used.input_tokens);
            tokens.output_tokens = tokens.output_tokens.saturating_add(used.output_tokens);
        }
        tokens
    }
}

/// One session's log, counted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    /// By quarter hour in UTC: milliseconds since 1970 / [`QUARTER_MS`]. A
    /// quarter with any event has a slot, so it counts as active.
    pub dated: BTreeMap<u64, Slot>,
    /// Events without a time.
    pub undated: Slot,
    pub has_undated: bool,
    /// A stretch in `working` the log leaves open: since when, if known.
    pub open: Option<Option<u64>>,
    pub last_active: Option<u64>,
}

/// The counts of [`Stats`] that go into a slot, to take their change per
/// event.
#[derive(Clone, Copy)]
struct Snapshot {
    turns: TurnCounts,
    requests: u64,
    retries: u64,
    errors: u64,
    tokens: TokenTotals,
    working_ms: u64,
    unknown_stretches: u64,
}

impl Snapshot {
    fn of(stats: &Stats) -> Snapshot {
        Snapshot {
            turns: stats.turns,
            requests: stats.requests,
            retries: stats.retries,
            errors: stats.errors,
            tokens: stats.tokens,
            working_ms: stats.working_ms,
            unknown_stretches: stats.unknown_stretches,
        }
    }
}

/// Calls `f` with each quarter hour from `from` to `to` (milliseconds) and the
/// milliseconds of it in that stretch.
fn spread(from: u64, to: u64, mut f: impl FnMut(u64, u64)) {
    let mut at = from;
    while at < to {
        let quarter = at / QUARTER_MS;
        let end = quarter.saturating_add(1).saturating_mul(QUARTER_MS).min(to);
        f(quarter, end - at);
        at = end;
    }
}

impl Summary {
    /// Counts a log of `(time, event)`. `model` is the session's model now:
    /// it applies from the start when the log never changes it.
    pub fn of_log<'a>(
        model: &str,
        log: impl IntoIterator<Item = (Option<u64>, &'a SessionEvent)> + Clone,
    ) -> Summary {
        let first_change = log.clone().into_iter().find_map(|(_, event)| match event {
            SessionEvent::ModelChanged { from, .. } => Some(from.clone()),
            _ => None,
        });
        let mut model = match first_change {
            Some(from) if from.is_empty() => UNKNOWN.to_owned(),
            Some(from) => from,
            None => model.to_owned(),
        };
        let mut summary = Summary::default();
        let mut stats = Stats::default();
        for (at, event) in log {
            let before = Snapshot::of(&stats);
            stats.count(event, at);
            let after = Snapshot::of(&stats);
            let user = match event {
                SessionEvent::Usage { model: used, .. } if !used.is_empty() => used.clone(),
                _ => model.clone(),
            };
            let slot = match at {
                Some(at) => summary.dated.entry(at / QUARTER_MS).or_default(),
                None => {
                    summary.has_undated = true;
                    &mut summary.undated
                }
            };
            let turns = &mut slot.turns;
            turns.done += after.turns.done.saturating_sub(before.turns.done);
            turns.cancelled += after.turns.cancelled.saturating_sub(before.turns.cancelled);
            turns.limit += after.turns.limit.saturating_sub(before.turns.limit);
            turns.failed += after.turns.failed.saturating_sub(before.turns.failed);
            slot.retries += after.retries.saturating_sub(before.retries);
            slot.errors += after.errors.saturating_sub(before.errors);
            slot.unknown_stretches += after
                .unknown_stretches
                .saturating_sub(before.unknown_stretches);
            let used = Use {
                requests: after.requests.saturating_sub(before.requests),
                input_tokens: after
                    .tokens
                    .input_tokens
                    .saturating_sub(before.tokens.input_tokens),
                output_tokens: after
                    .tokens
                    .output_tokens
                    .saturating_sub(before.tokens.output_tokens),
            };
            if !used.is_empty() {
                slot.models.entry(user).or_default().add(&used);
            }
            let worked = after.working_ms.saturating_sub(before.working_ms);
            if let (Some(at), true) = (at, worked > 0) {
                spread(at.saturating_sub(worked), at, |quarter, ms| {
                    summary.dated.entry(quarter).or_default().working_ms += ms;
                });
            }
            if let SessionEvent::ModelChanged { model: to, .. } = event {
                model.clone_from(to);
            }
        }
        summary.open = stats.working.then_some(stats.working_since);
        summary.last_active = stats.last_active;
        summary
    }

    /// Counts a saved session's log, with the model of its `session.json`.
    pub fn of_store(store: &Store) -> Result<Summary, String> {
        let meta = store.read_meta()?;
        let log = store.stamped()?;
        Ok(Summary::of_log(
            &meta.model,
            log.iter().map(|s| (s.at, &s.event)),
        ))
    }
}

/// A session for [`combine`].
pub struct Part<'a> {
    pub summary: &'a Summary,
    /// Loaded in the server: a stretch its log leaves open runs until now.
    pub running: bool,
}

/// What `combine` needs besides the summaries.
pub struct Request<'a> {
    pub range: Range,
    /// Seconds east of UTC.
    pub offset: i64,
    /// Milliseconds since 1970.
    pub now: u64,
    /// Whether a `provider/model` is configured now.
    pub configured: &'a dyn Fn(&str) -> bool,
    /// Saved sessions whose log could not be read.
    pub skipped: u64,
}

/// The day of a time in milliseconds, `offset` seconds east of UTC.
fn day_of_ms(ms: u64, offset: i64) -> i64 {
    let secs = i64::try_from(ms / 1000).unwrap_or(i64::MAX);
    secs.saturating_add(offset).div_euclid(DAY_SECS)
}

fn day_of_quarter(quarter: u64, offset: i64) -> i64 {
    day_of_ms(quarter.saturating_mul(QUARTER_MS), offset)
}

fn add_slot(totals: &mut GlobalTotals, slot: &Slot) {
    let turns = &mut totals.turns;
    turns.done += slot.turns.done;
    turns.cancelled += slot.turns.cancelled;
    turns.limit += slot.turns.limit;
    turns.failed += slot.turns.failed;
    totals.retries += slot.retries;
    totals.errors += slot.errors;
    totals.working_ms += slot.working_ms;
    totals.unknown_stretches += slot.unknown_stretches;
    let tokens = slot.tokens();
    totals.tokens.input_tokens += tokens.input_tokens;
    totals.tokens.output_tokens += tokens.output_tokens;
    totals.requests += slot.models.values().map(|u| u.requests).sum::<u64>();
}

/// The statistics of a range over the given sessions.
pub fn combine(parts: &[Part<'_>], request: &Request<'_>) -> GlobalStats {
    let offset = request.offset;
    let today = day_of_ms(request.now, offset);
    let first_day = match request.range.days() {
        Some(days) => today - (days - 1),
        None => parts
            .iter()
            .filter_map(|p| p.summary.dated.keys().next())
            .map(|q| day_of_quarter(*q, offset))
            .min()
            .unwrap_or(today)
            .min(today),
    };
    let all = request.range == Range::All;
    let in_range = |day: i64| all || (first_day..=today).contains(&day);
    let mut totals = GlobalTotals::default();
    let mut models: BTreeMap<String, ModelTotals> = BTreeMap::new();
    let mut days: BTreeMap<i64, DayTokens> = BTreeMap::new();
    let mut sessions = SessionCounts::default();
    let mut undated_tokens = 0u64;
    for part in parts {
        let summary = part.summary;
        sessions.saved += 1;
        if part.running {
            sessions.running += 1;
        }
        let mut active = false;
        let mut used: BTreeSet<&str> = BTreeSet::new();
        let mut take = |slot: &'_ Slot, day: Option<i64>, totals: &mut GlobalTotals| {
            add_slot(totals, slot);
            for (name, model) in &slot.models {
                let entry = models.entry(name.clone()).or_default();
                entry.requests += model.requests;
                entry.input_tokens += model.input_tokens;
                entry.output_tokens += model.output_tokens;
            }
            if let Some(day) = day {
                let tokens = slot.tokens();
                let entry = days.entry(day).or_default();
                entry.input_tokens += tokens.input_tokens;
                entry.output_tokens += tokens.output_tokens;
            }
        };
        for (quarter, slot) in &summary.dated {
            let day = day_of_quarter(*quarter, offset);
            if in_range(day) {
                active = true;
                take(slot, Some(day), &mut totals);
                used.extend(slot.models.keys().map(String::as_str));
            }
        }
        if all && summary.has_undated {
            active = true;
            take(&summary.undated, None, &mut totals);
            used.extend(summary.undated.models.keys().map(String::as_str));
            let tokens = summary.undated.tokens();
            undated_tokens += tokens.input_tokens + tokens.output_tokens;
        }
        match summary.open {
            Some(Some(since)) => {
                let end = if part.running {
                    request.now
                } else {
                    summary.last_active.unwrap_or(since)
                };
                spread(since, end, |quarter, ms| {
                    if in_range(day_of_quarter(quarter, offset)) {
                        active = true;
                        totals.working_ms += ms;
                    }
                });
            }
            Some(None) if active => totals.unknown_stretches += 1,
            _ => {}
        }
        if active {
            sessions.active += 1;
            for name in used {
                if let Some(entry) = models.get_mut(name) {
                    entry.sessions += 1;
                }
            }
        }
    }
    // A log that cannot be read is still a saved session.
    sessions.saved += request.skipped;
    let mut models: Vec<ModelTotals> = models
        .into_iter()
        .map(|(name, mut totals)| {
            totals.configured = name != UNKNOWN && (request.configured)(&name);
            totals.model = name;
            totals
        })
        .collect();
    models.sort_by(|a, b| b.tokens().cmp(&a.tokens()).then(a.model.cmp(&b.model)));
    let days = (first_day..=today)
        .map(|day| DayTokens {
            day,
            ..days.get(&day).copied().unwrap_or_default()
        })
        .collect();
    GlobalStats {
        range: request.range,
        first_day,
        last_day: today,
        sessions,
        totals,
        models,
        days,
        undated_tokens,
        skipped: request.skipped,
        now: request.now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::{EndReason, State, TurnEnd};

    const HOUR: u64 = 3_600_000;
    const DAY: u64 = 24 * HOUR;
    /// 2026-11-10 00:00 UTC.
    const TODAY: u64 = 20_767 * DAY;

    fn usage(input: u64, output: u64, model: &str) -> SessionEvent {
        SessionEvent::Usage {
            input_tokens: input,
            output_tokens: output,
            context_tokens: input + output,
            model: model.into(),
        }
    }

    fn changed(from: &str, to: &str) -> SessionEvent {
        SessionEvent::ModelChanged {
            from: from.into(),
            model: to.into(),
        }
    }

    fn state(state: State) -> SessionEvent {
        SessionEvent::State { state }
    }

    fn end() -> SessionEvent {
        SessionEvent::TurnEnd(TurnEnd {
            reason: EndReason::Done,
            requests: 1,
            tool_calls: 0,
            input_tokens: 0,
            output_tokens: 0,
        })
    }

    fn summary(model: &str, log: &[(Option<u64>, SessionEvent)]) -> Summary {
        Summary::of_log(model, log.iter().map(|(at, e)| (*at, e)))
    }

    fn global(parts: &[(&Summary, bool)], range: Range, offset: i64, now: u64) -> GlobalStats {
        let parts: Vec<Part<'_>> = parts
            .iter()
            .map(|(summary, running)| Part {
                summary,
                running: *running,
            })
            .collect();
        combine(
            &parts,
            &Request {
                range,
                offset,
                now,
                configured: &|name| name != "a/gone",
                skipped: 0,
            },
        )
    }

    fn model<'a>(stats: &'a GlobalStats, name: &str) -> &'a ModelTotals {
        stats.models.iter().find(|m| m.model == name).unwrap()
    }

    #[test]
    fn a_model_change_in_the_middle_splits_the_tokens() {
        let log = [
            (
                Some(TODAY + HOUR),
                SessionEvent::UserMessage { text: "hi".into() },
            ),
            (Some(TODAY + HOUR), usage(100, 10, "")),
            (Some(TODAY + HOUR), changed("a/one", "a/two")),
            (Some(TODAY + 2 * HOUR), usage(200, 20, "")),
            (Some(TODAY + 2 * HOUR), end()),
        ];
        let one = summary("a/two", &log);
        let stats = global(&[(&one, false)], Range::Week, 0, TODAY + 3 * HOUR);
        assert_eq!(stats.totals.tokens.input_tokens, 300);
        assert_eq!(stats.totals.requests, 2);
        assert_eq!(stats.totals.turns.done, 1);
        assert_eq!(model(&stats, "a/one").input_tokens, 100);
        assert_eq!(model(&stats, "a/two").input_tokens, 200);
        // Most tokens first.
        assert_eq!(stats.models[0].model, "a/two");
        assert_eq!(model(&stats, "a/one").sessions, 1);
        assert_eq!(stats.sessions.active, 1);
    }

    #[test]
    fn a_log_that_never_changes_the_model_uses_the_session_model() {
        let one = summary("a/gone", &[(Some(TODAY), usage(5, 5, ""))]);
        let stats = global(&[(&one, false)], Range::All, 0, TODAY);
        assert_eq!(stats.models.len(), 1);
        assert_eq!(stats.models[0].model, "a/gone");
        assert!(!stats.models[0].configured);
    }

    #[test]
    fn an_old_change_without_from_gives_the_tokens_before_it_to_unknown() {
        let log = [
            (Some(TODAY), usage(7, 0, "")),
            (Some(TODAY), changed("", "a/two")),
            (Some(TODAY), usage(9, 0, "")),
        ];
        let stats = global(&[(&summary("a/two", &log), false)], Range::All, 0, TODAY);
        assert_eq!(model(&stats, UNKNOWN).input_tokens, 7);
        assert!(!model(&stats, UNKNOWN).configured);
        assert_eq!(model(&stats, "a/two").input_tokens, 9);
    }

    #[test]
    fn the_model_of_a_usage_event_wins() {
        let log = [
            (Some(TODAY), usage(7, 0, "a/one")),
            (Some(TODAY), changed("a/one", "a/two")),
        ];
        let stats = global(&[(&summary("a/two", &log), false)], Range::All, 0, TODAY);
        assert_eq!(stats.models.len(), 1);
        assert_eq!(model(&stats, "a/one").input_tokens, 7);
    }

    #[test]
    fn days_around_midnight_follow_the_offset() {
        // 23:30 UTC on the day before.
        let late = TODAY - HOUR / 2;
        let one = summary("a/one", &[(Some(late), usage(10, 0, ""))]);
        let east = global(&[(&one, false)], Range::Week, 3_600, TODAY + HOUR);
        assert_eq!(east.days.last().unwrap().input_tokens, 10);
        let utc = global(&[(&one, false)], Range::Week, 0, TODAY + HOUR);
        assert_eq!(utc.days[5].input_tokens, 10);
        assert_eq!(utc.days[6].input_tokens, 0);
        // 00:30 UTC is still the day before at UTC-1.
        let early = summary("a/one", &[(Some(TODAY + HOUR / 2), usage(3, 0, ""))]);
        let west = global(&[(&early, false)], Range::Week, -3_600, TODAY + 2 * HOUR);
        assert_eq!(west.days[5].input_tokens, 3);
        assert_eq!(west.days.len(), 7);
    }

    #[test]
    fn the_range_counts_today_and_the_days_before() {
        let log = [
            (Some(TODAY - 6 * DAY), usage(1, 0, "")),
            (Some(TODAY - 7 * DAY), usage(2, 0, "")),
            (None, usage(4, 0, "")),
        ];
        let one = summary("a/one", &log);
        let week = global(&[(&one, false)], Range::Week, 0, TODAY + HOUR);
        assert_eq!(week.totals.tokens.input_tokens, 1);
        assert_eq!(week.first_day, week.last_day - 6);
        assert_eq!(week.days.len(), 7);
        assert_eq!(week.undated_tokens, 0);
        let all = global(&[(&one, false)], Range::All, 0, TODAY + HOUR);
        assert_eq!(all.totals.tokens.input_tokens, 7);
        assert_eq!(all.undated_tokens, 4);
        assert_eq!(all.days.len(), 8);
        // Undated tokens are not in a day.
        assert_eq!(all.days.iter().map(DayTokens::total).sum::<u64>(), 3);
        let month = global(&[(&one, false)], Range::Month, 0, TODAY + HOUR);
        assert_eq!(month.days.len(), 30);
        assert_eq!(month.totals.tokens.input_tokens, 3);
    }

    #[test]
    fn working_time_is_split_by_day_and_open_stretches_run_until_now() {
        let log = [
            (Some(TODAY - HOUR), state(State::Working)),
            (Some(TODAY + HOUR), state(State::Idle)),
            (Some(TODAY + 2 * HOUR), state(State::Working)),
        ];
        let one = summary("a/one", &log);
        let utc = |running| global(&[(&one, running)], Range::Week, 0, TODAY + 3 * HOUR);
        assert_eq!(utc(true).totals.working_ms, 3 * HOUR);
        assert_eq!(utc(true).sessions.running, 1);
        // A saved session's open stretch ends with its last event.
        assert_eq!(utc(false).totals.working_ms, 2 * HOUR);
        // Of yesterday's hour, nothing is in a range that starts today.
        let mut parts = global(&[(&one, false)], Range::Week, 0, TODAY + 6 * DAY);
        assert_eq!(parts.totals.working_ms, HOUR);
        parts = global(&[(&one, false)], Range::Week, 0, TODAY + 7 * DAY);
        assert_eq!(parts.totals.working_ms, 0);
        assert_eq!(parts.sessions.active, 0);
        assert_eq!(parts.sessions.saved, 1);
    }

    #[test]
    fn sessions_are_counted_once_each() {
        let one = summary("a/one", &[(Some(TODAY), usage(1, 1, ""))]);
        let two = summary(
            "a/one",
            &[
                (Some(TODAY), usage(1, 1, "")),
                (Some(TODAY), usage(1, 1, "")),
            ],
        );
        let old = summary("a/one", &[(Some(TODAY - 20 * DAY), usage(1, 1, ""))]);
        let stats = global(
            &[(&one, false), (&two, true), (&old, false)],
            Range::Week,
            0,
            TODAY,
        );
        assert_eq!(
            stats.sessions,
            SessionCounts {
                active: 2,
                saved: 3,
                running: 1
            }
        );
        assert_eq!(model(&stats, "a/one").sessions, 2);
        assert_eq!(model(&stats, "a/one").requests, 3);
    }

    #[test]
    fn no_sessions() {
        let stats = global(&[], Range::All, 0, TODAY);
        assert_eq!((stats.first_day, stats.last_day), (20_767, 20_767));
        assert_eq!(stats.days.len(), 1);
        assert!(stats.models.is_empty());
    }
}
