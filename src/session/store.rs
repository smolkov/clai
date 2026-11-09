//! Sessions on disk (see `spec/sessions.md`, "Persistence"): one directory
//! per session with `session.json` (replaced atomically), and
//! `history.jsonl` and `events.jsonl` (appended, one JSON value per line).

use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::agent::{
    CallResult, CallState, Closed, EndReason, INTERRUPTED, Question, Record, SessionEvent, State,
    ToolCallEvent, TurnEnd, now_ms,
};
use crate::protocol::{SessionInfo, Stats, TokenTotals};

/// The version of the files; a later `clai` reads older ones, never newer.
pub const FORMAT: u32 = 1;

const META: &str = "session.json";
const HISTORY: &str = "history.jsonl";
const EVENTS: &str = "events.jsonl";

/// What `session.json` holds: everything but the history and the log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub format: u32,
    /// The `clai` version that wrote it.
    pub version: String,
    pub name: String,
    pub workspace: PathBuf,
    /// The workspace as shown: `~/ws/rust/clai`.
    pub place: String,
    /// `provider/model`.
    pub model: String,
    /// Seconds since 1970.
    pub created: u64,
    pub updated: u64,
    /// Turns started so far.
    pub turns: u64,
    /// Messages waiting for a turn.
    #[serde(default)]
    pub queued: Vec<String>,
    /// Tokens used so far. A file of an older `clai` has none; the log is
    /// summed when the session resumes.
    #[serde(default)]
    pub usage: TokenTotals,
}

impl Meta {
    pub fn new(name: String, workspace: PathBuf, place: String, model: String, now: u64) -> Meta {
        Meta {
            format: FORMAT,
            version: env!("CARGO_PKG_VERSION").to_owned(),
            name,
            workspace,
            place,
            model,
            created: now,
            updated: now,
            turns: 0,
            queued: Vec::new(),
            usage: TokenTotals::default(),
        }
    }

    /// The session as clients see it; the rest is not in the meta.
    pub fn info(
        &self,
        id: u64,
        state: State,
        queued: usize,
        question: Option<Question>,
    ) -> SessionInfo {
        SessionInfo {
            id,
            name: self.name.clone(),
            workspace: self.place.clone(),
            path: self.workspace.clone(),
            model: self.model.clone(),
            state,
            created: self.created,
            updated: self.updated,
            turns: self.turns,
            queued,
            question,
            usage: self.usage,
        }
    }
}

/// A line of `events.jsonl`: the event's fields and `at`, the time it was
/// logged in milliseconds since 1970. A line of an older `clai` has no
/// `at`; the format stays the same.
#[derive(Debug, Clone, PartialEq)]
pub struct Stamped {
    pub at: Option<u64>,
    pub event: SessionEvent,
}

impl Serialize for Stamped {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut value = serde_json::to_value(&self.event).map_err(serde::ser::Error::custom)?;
        if let (Some(at), serde_json::Value::Object(map)) = (self.at, &mut value) {
            map.insert("at".to_owned(), at.into());
        }
        value.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Stamped {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Stamped, D::Error> {
        let mut value = serde_json::Value::deserialize(deserializer)?;
        let at = match &mut value {
            serde_json::Value::Object(map) => map.remove("at").and_then(|at| at.as_u64()),
            _ => None,
        };
        let event = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        Ok(Stamped { at, event })
    }
}

/// A saved session as one file, for `clai export` and `clai import`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bundle {
    pub format: u32,
    pub meta: Meta,
    pub history: Vec<Record>,
    pub events: Vec<Stamped>,
}

/// A loaded session.
#[derive(Debug, Clone, PartialEq)]
pub struct Loaded {
    pub meta: Meta,
    pub history: Vec<Record>,
    /// The log as clients get it on attach.
    pub events: Vec<SessionEvent>,
    /// Counted from the log, with its times.
    pub stats: Stats,
}

/// The directory of one session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Store {
    dir: PathBuf,
}

fn io_err(path: &Path, e: &io::Error) -> String {
    format!("{}: {e}", path.display())
}

impl Store {
    pub fn open(dir: PathBuf) -> Store {
        Store { dir }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// A new, empty directory below `root`, with mode `0700`.
    pub fn create(root: &Path) -> Result<Store, String> {
        DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(root)
            .map_err(|e| io_err(root, &e))?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        for n in 0u32..1000 {
            let dir = root.join(format!("{stamp:x}-{}-{n}", std::process::id()));
            match DirBuilder::new().mode(0o700).create(&dir) {
                Ok(()) => return Ok(Store { dir }),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
                Err(e) => return Err(io_err(&dir, &e)),
            }
        }
        Err(format!("{}: no free directory name", root.display()))
    }

    /// Replaces `session.json`: written to a temporary file, then renamed.
    pub fn write_meta(&self, meta: &Meta) -> Result<(), String> {
        let path = self.dir.join(META);
        let temp = self.dir.join(format!("{META}.tmp"));
        let text = serde_json::to_vec_pretty(meta).map_err(|e| e.to_string())?;
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temp)
            .map_err(|e| io_err(&temp, &e))?;
        file.write_all(&text)
            .and_then(|()| file.sync_all())
            .map_err(|e| io_err(&temp, &e))?;
        fs::rename(&temp, &path).map_err(|e| io_err(&path, &e))
    }

    pub fn read_meta(&self) -> Result<Meta, String> {
        let path = self.dir.join(META);
        let text = fs::read_to_string(&path).map_err(|e| io_err(&path, &e))?;
        let meta: Meta = serde_json::from_str(&text)
            .map_err(|e| format!("{}: {e}", path.display()))
            .or_else(|e| {
                // A newer format may not parse; say that, not the parse error.
                newer(&text).map_or(Err(e), Err)
            })?;
        if meta.format > FORMAT {
            return Err(too_new(meta.format));
        }
        Ok(meta)
    }

    pub fn append_record(&self, record: &Record) -> Result<(), String> {
        append(&self.dir.join(HISTORY), record)
    }

    /// Appends an event to the log with the time now, if it is one that
    /// is kept (see [`kept`]).
    pub fn append_event(&self, event: &SessionEvent) -> Result<(), String> {
        self.append_stamped(&Stamped {
            at: Some(now_ms()),
            event: event.clone(),
        })
    }

    /// Appends an event with its time, if it is kept.
    pub fn append_stamped(&self, stamped: &Stamped) -> Result<(), String> {
        if kept(&stamped.event) {
            append(&self.dir.join(EVENTS), stamped)
        } else {
            Ok(())
        }
    }

    /// Loads the session, repairing a turn that a stopped server left
    /// open; the repair is written to the files too, with the time of the
    /// last event, so that the time the server was away does not count as
    /// work.
    pub fn load(&self) -> Result<Loaded, String> {
        let meta = self.read_meta()?;
        let history: Vec<Record> = read_lines(&self.dir.join(HISTORY))?;
        let mut stamped: Vec<Stamped> = read_lines(&self.dir.join(EVENTS))?;
        let events: Vec<SessionEvent> = stamped.iter().map(|s| s.event.clone()).collect();
        let last = stamped.iter().filter_map(|s| s.at).max();
        for event in repair_events(&events) {
            let repair = Stamped { at: last, event };
            self.append_stamped(&repair)?;
            stamped.push(repair);
        }
        let stats = Stats::of_log(stamped.iter().map(|s| (s.at, &s.event)));
        Ok(Loaded {
            meta,
            history,
            events: expand(stamped.into_iter().map(|s| s.event).collect()),
            stats,
        })
    }

    /// The statistics of a saved session, from its log; the session is
    /// not loaded.
    pub fn stats(&self) -> Result<Stats, String> {
        let stamped: Vec<Stamped> = read_lines(&self.dir.join(EVENTS))?;
        let mut stats = Stats::of_log(stamped.iter().map(|s| (s.at, &s.event)));
        stats.settle();
        Ok(stats)
    }

    /// The size of the session's files in bytes.
    pub fn size(&self) -> u64 {
        [META, HISTORY, EVENTS]
            .iter()
            .filter_map(|name| fs::metadata(self.dir.join(name)).ok())
            .map(|m| m.len())
            .sum()
    }

    pub fn remove(&self) -> Result<(), String> {
        match fs::remove_dir_all(&self.dir) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(io_err(&self.dir, &e)),
        }
    }

    /// The whole session as one value, as it is on disk.
    pub fn bundle(&self) -> Result<Bundle, String> {
        Ok(Bundle {
            format: FORMAT,
            meta: self.read_meta()?,
            history: read_lines(&self.dir.join(HISTORY))?,
            events: read_lines(&self.dir.join(EVENTS))?,
        })
    }

    /// Writes a bundle into this (new) directory, with `meta` in place of
    /// the bundle's.
    pub fn write_bundle(&self, bundle: &Bundle, meta: &Meta) -> Result<(), String> {
        for record in &bundle.history {
            self.append_record(record)?;
        }
        for event in &bundle.events {
            self.append_stamped(event)?;
        }
        self.write_meta(meta)
    }
}

/// Reads a bundle file written by `clai export`.
pub fn read_bundle(path: &Path) -> Result<Bundle, String> {
    let text = fs::read_to_string(path).map_err(|e| io_err(path, &e))?;
    match serde_json::from_str::<Bundle>(&text) {
        Ok(bundle) if bundle.format > FORMAT => Err(too_new(bundle.format)),
        Ok(bundle) => Ok(bundle),
        Err(e) => Err(newer(&text).unwrap_or_else(|| format!("{}: {e}", path.display()))),
    }
}

/// Writes a bundle file, with mode `0600`.
pub fn write_bundle_file(path: &Path, bundle: &Bundle) -> Result<(), String> {
    let text = serde_json::to_vec(bundle).map_err(|e| e.to_string())?;
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| io_err(path, &e))?;
    file.write_all(&text)
        .and_then(|()| file.write_all(b"\n"))
        .map_err(|e| io_err(path, &e))
}

/// The saved sessions below `root`. Directories that cannot be read are
/// logged and left out.
pub fn list(root: &Path) -> Vec<(Store, Meta)> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for entry in entries.flatten() {
        let store = Store::open(entry.path());
        if !store.dir.is_dir() {
            continue;
        }
        match store.read_meta() {
            Ok(meta) => found.push((store, meta)),
            Err(e) => tracing::warn!("saved session left out: {e}"),
        }
    }
    found.sort_by_key(|(_, meta)| meta.created);
    found
}

fn too_new(format: u32) -> String {
    format!("saved by a newer clai (format {format}, this clai reads up to {FORMAT})")
}

/// The error for a file of a newer format, if it is one.
fn newer(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let format = value.get("format")?.as_u64()?;
    let format = u32::try_from(format).unwrap_or(u32::MAX);
    (format > FORMAT).then(|| too_new(format))
}

fn append<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    let mut line = serde_json::to_vec(value).map_err(|e| e.to_string())?;
    line.push(b'\n');
    let mut file = OpenOptions::new()
        .append(true)
        .create(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| io_err(path, &e))?;
    file.write_all(&line).map_err(|e| io_err(path, &e))
}

/// The values of a JSON Lines file; a missing file is empty. A last line
/// cut by a crash (no newline, does not parse) is left out.
fn read_lines<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>, String> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(io_err(path, &e)),
    };
    let complete = text.ends_with('\n');
    let lines: Vec<&str> = text.lines().collect();
    let mut values = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(line) {
            Ok(value) => values.push(value),
            Err(_) if !complete && i + 1 == lines.len() => {
                tracing::warn!(file = %path.display(), "cut last line left out");
            }
            Err(e) => return Err(format!("{}:{}: {e}", path.display(), i + 1)),
        }
    }
    Ok(values)
}

/// Whether an event is saved: streamed pieces are not, because the whole
/// text (`text_done`) and the final state of a call carry the same.
pub fn kept(event: &SessionEvent) -> bool {
    match event {
        SessionEvent::TextDelta { .. } => false,
        SessionEvent::ToolCall(call) => call.output.is_none(),
        _ => true,
    }
}

/// The saved log as clients expect it: each `text_done` after the text as
/// one `text_delta`.
fn expand(events: Vec<SessionEvent>) -> Vec<SessionEvent> {
    let mut out = Vec::with_capacity(events.len());
    for event in events {
        if let SessionEvent::TextDone { text } = &event {
            out.push(SessionEvent::TextDelta { text: text.clone() });
        }
        out.push(event);
    }
    out
}

/// Events that close a turn the log leaves open: its question, its calls,
/// the turn itself.
pub fn repair_events(events: &[SessionEvent]) -> Vec<SessionEvent> {
    let mut in_turn = false;
    let mut question: Option<String> = None;
    let mut calls: Vec<ToolCallEvent> = Vec::new();
    for event in events {
        match event {
            SessionEvent::UserMessage { .. } => in_turn = true,
            SessionEvent::TurnEnd(_) => {
                in_turn = false;
                question = None;
                calls.clear();
            }
            SessionEvent::Question(q) => question = Some(q.id.clone()),
            SessionEvent::QuestionClosed { id, .. } => {
                if question.as_ref() == Some(id) {
                    question = None;
                }
            }
            SessionEvent::ToolCall(call) => {
                calls.retain(|c| c.id != call.id);
                if !call.state.is_final() {
                    calls.push(call.clone());
                }
            }
            _ => {}
        }
    }
    if !in_turn {
        return Vec::new();
    }
    let mut repair = Vec::new();
    if let Some(id) = question {
        repair.push(SessionEvent::QuestionClosed {
            id,
            closed: Closed::Cancelled,
        });
    }
    for call in calls {
        repair.push(SessionEvent::ToolCall(ToolCallEvent {
            state: CallState::Cancelled,
            output: None,
            result: Some(CallResult {
                text: INTERRUPTED.to_owned(),
                is_error: true,
            }),
            ..call
        }));
    }
    repair.push(SessionEvent::Notice {
        text: "the server stopped during this turn; it was interrupted".to_owned(),
    });
    repair.push(SessionEvent::TurnEnd(TurnEnd {
        reason: EndReason::Cancelled,
        requests: 0,
        tool_calls: 0,
        input_tokens: 0,
        output_tokens: 0,
    }));
    repair.push(SessionEvent::State { state: State::Idle });
    repair
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use serde_json::json;

    use super::*;
    use crate::provider::{Block, Message, Role};

    fn meta() -> Meta {
        Meta::new(
            "s".into(),
            PathBuf::from("/ws"),
            "~/ws".into(),
            "a/b".into(),
            100,
        )
    }

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn write_and_load() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::create(&tmp.path().join("sessions")).unwrap();
        let mut m = meta();
        m.queued = vec!["later".into()];
        store.write_meta(&m).unwrap();
        let records = vec![
            Record::Message(Message::user("hi")),
            Record::Message(Message {
                role: Role::Assistant,
                content: vec![
                    Block::Text { text: "ok".into() },
                    Block::ToolCall {
                        id: "c1".into(),
                        name: "read_file".into(),
                        arguments: json!({"path": "a"}),
                    },
                ],
            }),
            Record::Message(Message {
                role: Role::User,
                content: vec![Block::ToolResult {
                    call_id: "c1".into(),
                    content: "x".into(),
                    is_error: false,
                }],
            }),
            Record::Full {
                call_id: "c1".into(),
                text: "xx".into(),
            },
            Record::Rules {
                rules: vec!["allow run_command(ls)".into()],
            },
        ];
        for r in &records {
            store.append_record(r).unwrap();
        }
        for e in [
            SessionEvent::UserMessage { text: "hi".into() },
            SessionEvent::TextDelta { text: "o".into() },
            SessionEvent::TextDelta { text: "k".into() },
            SessionEvent::TextDone { text: "ok".into() },
            SessionEvent::TurnEnd(TurnEnd {
                reason: EndReason::Done,
                requests: 1,
                tool_calls: 0,
                input_tokens: 1,
                output_tokens: 1,
            }),
        ] {
            store.append_event(&e).unwrap();
        }
        let loaded = store.load().unwrap();
        assert_eq!(loaded.meta, m);
        assert_eq!(loaded.history, records);
        // Deltas are not saved; the whole text comes back as one.
        assert_eq!(
            loaded.events[..3],
            [
                SessionEvent::UserMessage { text: "hi".into() },
                SessionEvent::TextDelta { text: "ok".into() },
                SessionEvent::TextDone { text: "ok".into() },
            ]
        );
        assert_eq!(loaded.events.len(), 4);
        assert_eq!(mode(store.dir()), 0o700);
        for file in [META, HISTORY, EVENTS] {
            assert_eq!(mode(&store.dir().join(file)), 0o600, "{file}");
        }
        assert_eq!(list(&tmp.path().join("sessions")).len(), 1);
        store.remove().unwrap();
        assert!(list(&tmp.path().join("sessions")).is_empty());
    }

    #[test]
    fn a_meta_without_totals_reads_as_zero() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::create(tmp.path()).unwrap();
        let mut old = serde_json::to_value(meta()).unwrap();
        old.as_object_mut().unwrap().remove("usage");
        fs::write(store.dir().join(META), old.to_string()).unwrap();
        assert_eq!(store.read_meta().unwrap().usage, TokenTotals::default());
        assert_eq!(FORMAT, 1);
    }

    #[test]
    fn cut_last_line_and_newer_format() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::create(tmp.path()).unwrap();
        store.write_meta(&meta()).unwrap();
        store
            .append_record(&Record::Message(Message::user("a")))
            .unwrap();
        let path = store.dir().join(HISTORY);
        let mut text = fs::read_to_string(&path).unwrap();
        text.push_str("{\"record\":\"mess");
        fs::write(&path, &text).unwrap();
        assert_eq!(store.load().unwrap().history.len(), 1);
        // A broken line in the middle is an error.
        fs::write(&path, "{broken\n{\"record\":\"rules\",\"rules\":[]}\n").unwrap();
        assert!(store.load().unwrap_err().contains(":1:"));

        let mut newer = serde_json::to_value(meta()).unwrap();
        newer["format"] = json!(FORMAT + 1);
        newer["extra"] = json!({"new": true});
        fs::write(store.dir().join(META), newer.to_string()).unwrap();
        let e = store.load().unwrap_err();
        assert!(e.contains("saved by a newer clai"), "{e}");
    }

    #[test]
    fn open_turn_is_closed() {
        let call = |state| {
            SessionEvent::ToolCall(ToolCallEvent {
                id: "c1".into(),
                name: "run_command".into(),
                state,
                summary: Some("sleep 9".into()),
                arguments: json!({"command": "sleep 9"}),
                output: None,
                result: None,
            })
        };
        let events = vec![
            SessionEvent::State {
                state: State::Working,
            },
            SessionEvent::UserMessage { text: "go".into() },
            call(CallState::Waiting),
        ];
        let repair = repair_events(&events);
        let SessionEvent::ToolCall(closed) = &repair[0] else {
            panic!("{repair:?}");
        };
        assert_eq!(closed.state, CallState::Cancelled);
        assert_eq!(closed.summary.as_deref(), Some("sleep 9"));
        assert!(matches!(
            repair.last(),
            Some(SessionEvent::State { state: State::Idle })
        ));
        assert!(repair.iter().any(|e| matches!(
            e,
            SessionEvent::TurnEnd(TurnEnd {
                reason: EndReason::Cancelled,
                ..
            })
        )));

        // A finished turn needs nothing.
        let mut done = events.clone();
        done.push(call(CallState::Done));
        done.push(SessionEvent::TurnEnd(TurnEnd {
            reason: EndReason::Done,
            requests: 1,
            tool_calls: 1,
            input_tokens: 0,
            output_tokens: 0,
        }));
        assert!(repair_events(&done).is_empty());
    }

    #[test]
    fn bundles() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::create(tmp.path()).unwrap();
        store.write_meta(&meta()).unwrap();
        store
            .append_record(&Record::Message(Message::user("a")))
            .unwrap();
        let bundle = store.bundle().unwrap();
        let file = tmp.path().join("s.json");
        write_bundle_file(&file, &bundle).unwrap();
        assert_eq!(mode(&file), 0o600);
        let read = read_bundle(&file).unwrap();
        assert_eq!(read, bundle);
        let copy = Store::create(tmp.path()).unwrap();
        let mut renamed = read.meta.clone();
        renamed.name = "t".into();
        copy.write_bundle(&read, &renamed).unwrap();
        let loaded = copy.load().unwrap();
        assert_eq!(loaded.meta.name, "t");
        assert_eq!(loaded.history, bundle.history);
    }

    #[test]
    fn events_carry_their_time() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::create(&tmp.path().join("sessions")).unwrap();
        store.write_meta(&meta()).unwrap();
        // A line of an older `clai`, without `at`, then lines with it.
        fs::write(
            store.dir().join(EVENTS),
            "{\"event\":\"state\",\"state\":\"working\"}\n",
        )
        .unwrap();
        store
            .append_stamped(&Stamped {
                at: Some(1_000),
                event: SessionEvent::UserMessage { text: "hi".into() },
            })
            .unwrap();
        store
            .append_stamped(&Stamped {
                at: Some(3_000),
                event: SessionEvent::Usage {
                    input_tokens: 5,
                    output_tokens: 1,
                    context_tokens: 6,
                },
            })
            .unwrap();
        let line = fs::read_to_string(store.dir().join(EVENTS)).unwrap();
        assert!(
            line.lines().nth(1).unwrap().contains("\"at\":1000"),
            "{line}"
        );

        // Clients get the events without times; the open turn is closed at
        // the time of the last event.
        let loaded = store.load().unwrap();
        assert_eq!(
            loaded.events[1],
            SessionEvent::UserMessage { text: "hi".into() }
        );
        assert_eq!(loaded.stats.requests, 1);
        assert_eq!(loaded.stats.turns.cancelled, 1);
        assert_eq!(loaded.stats.last_active, Some(3_000));
        assert_eq!(loaded.stats.working_time(9_000), (0, false));
        let stamped: Vec<Stamped> = read_lines(&store.dir().join(EVENTS)).unwrap();
        assert_eq!(stamped[0].at, None);
        assert!(stamped[3..].iter().all(|s| s.at == Some(3_000)));

        // A saved session's statistics, without loading it.
        let stats = store.stats().unwrap();
        assert_eq!(stats.requests, 1);
        assert!(stats.turn.is_none());
        assert!(store.size() > 0);

        // A bundle keeps the times.
        let bundle = store.bundle().unwrap();
        assert_eq!(bundle.events[1].at, Some(1_000));
        let text = serde_json::to_string(&bundle).unwrap();
        let back: Bundle = serde_json::from_str(&text).unwrap();
        assert_eq!(back, bundle);
    }
}
