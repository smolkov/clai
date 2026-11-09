//! The status of a session as sections of rows (see `spec/tui.md`, "Status
//! box"): the TUI draws them in a box, `clai chat` prints them as text.

use crate::agent::{State, date_of};
use crate::protocol::{SessionStatus, Stats, ToolCounts};

use super::{short_tokens, tokens};

/// Where the values start: a label is indented by 2 and padded to this.
pub const LABEL_WIDTH: usize = 13;
/// The cells of the context bar.
const BAR: usize = 20;
/// From this share of the context window on, the context row warns.
const WARN_PERCENT: u64 = 80;

/// How a value is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Normal,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Empty for a row that goes on the one above.
    pub label: String,
    pub value: String,
    pub tone: Tone,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub title: String,
    /// Shown after the title, in the value column: a table's column names,
    /// a one-line section.
    pub head: String,
    pub rows: Vec<Row>,
}

/// What the client adds: symbols, its time zone and its own name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options<'a> {
    pub unicode: bool,
    /// Seconds from UTC.
    pub offset: i64,
    /// The client's name, to mark it in the clients row.
    pub me: &'a str,
}

impl Options<'_> {
    fn rule(&self) -> &'static str {
        if self.unicode { " ─ " } else { " - " }
    }

    fn warning(&self) -> &'static str {
        if self.unicode { "⚠" } else { "!" }
    }
}

fn row(label: &str, value: impl Into<String>) -> Row {
    Row {
        label: label.to_owned(),
        value: value.into(),
        tone: Tone::Normal,
    }
}

fn warn(label: &str, value: impl Into<String>) -> Row {
    Row {
        tone: Tone::Warning,
        ..row(label, value)
    }
}

/// `12s`, `4m 12s`, `2h 10m`, `3d 4h`.
pub fn duration(ms: u64) -> String {
    let secs = ms / 1000;
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m {:02}s", secs / 60, secs % 60),
        3600..86_400 => format!("{}h {:02}m", secs / 3600, secs % 3600 / 60),
        _ => format!("{}d {}h", secs / 86_400, secs % 86_400 / 3600),
    }
}

/// `1 message`, `2 messages`.
fn count(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// `1.2 MB`, `340 kB`, `512 B`.
fn bytes(n: u64) -> String {
    match n {
        0..1000 => format!("{n} B"),
        1000..1_000_000 => format!("{} kB", n / 1000),
        _ => format!("{}.{} MB", n / 1_000_000, n % 1_000_000 / 100_000),
    }
}

/// `2026-11-07 17:46` of a time in seconds since 1970, `offset` seconds
/// from UTC.
pub fn local_time(secs: u64, offset: i64) -> String {
    let local = i64::try_from(secs)
        .unwrap_or(0)
        .saturating_add(offset)
        .max(0);
    let local = u64::try_from(local).unwrap_or(0);
    let minutes = local / 60;
    format!(
        "{} {:02}:{:02}",
        date_of(local),
        minutes / 60 % 24,
        minutes % 60
    )
}

/// `[######......]` or `██████░░░░░░`.
fn bar(percent: u64, unicode: bool) -> String {
    let full = usize::try_from(percent.min(100)).unwrap_or(100) * BAR / 100;
    if unicode {
        format!("{}{}", "█".repeat(full), "░".repeat(BAR - full))
    } else {
        format!("[{}{}]", "#".repeat(full), ".".repeat(BAR - full))
    }
}

/// The parts that are not zero, joined: `20 done, 1 cancelled`.
fn parts(counts: &[(u64, &str)], join: &str) -> String {
    counts
        .iter()
        .filter(|(n, _)| *n > 0)
        .map(|(n, name)| format!("{n} {name}"))
        .collect::<Vec<_>>()
        .join(join)
}

pub fn sections(status: &SessionStatus, options: &Options<'_>) -> Vec<Section> {
    let mut sections = vec![session(status, options), model(status, options)];
    if let Some(turn) = this_turn(status) {
        sections.push(turn);
    }
    sections.push(totals(status, options));
    sections.push(tools(&status.stats));
    sections.push(questions(&status.stats, options));
    sections.push(server(status, options));
    sections
}

fn session(status: &SessionStatus, options: &Options<'_>) -> Section {
    let info = &status.session;
    let now_secs = status.now / 1000;
    let mut rows = vec![row("name", format!("{}  (id {})", info.name, info.id))];
    let state = match (info.state, &info.question) {
        (State::Waiting, Some(q)) => format!("waiting: {}", q.summary),
        (State::Idle, _) => "idle".to_owned(),
        (State::Working, _) => "working".to_owned(),
        (State::Waiting, None) => "waiting".to_owned(),
        (State::Saved, _) => "saved, not loaded".to_owned(),
    };
    rows.push(row("state", state));
    rows.push(row("workspace", info.workspace.clone()));
    for (i, dir) in status.extra_dirs.iter().enumerate() {
        let label = if i == 0 { "extra dirs" } else { "" };
        let mut value = dir.path.clone();
        if dir.read_only {
            value.push_str("  (read only)");
        }
        if dir.exists {
            rows.push(row(label, value));
        } else {
            rows.push(warn(
                label,
                format!("{value}  {} missing", options.warning()),
            ));
        }
    }
    rows.push(row(
        "created",
        format!(
            "{}  ({} ago)",
            local_time(info.created, options.offset),
            duration(now_secs.saturating_sub(info.created).saturating_mul(1000))
        ),
    ));
    if let Some(last) = status.stats.last_active {
        rows.push(row(
            "last active",
            format!("{} ago", duration(status.now.saturating_sub(last))),
        ));
    }
    if info.state != State::Saved {
        let names: Vec<String> = status
            .clients
            .iter()
            .map(|c| {
                let mut name = c.name.clone();
                if c.name == options.me {
                    name.push_str(" (this)");
                }
                if !c.answers {
                    name.push_str(" (watching)");
                }
                name
            })
            .collect();
        let value = if names.is_empty() {
            "none".to_owned()
        } else {
            format!("{}: {}", names.len(), names.join(", "))
        };
        rows.push(row("clients", value));
    }
    if info.queued > 0 {
        rows.push(row(
            "queued",
            count(
                u64::try_from(info.queued).unwrap_or(u64::MAX),
                "message",
                "messages",
            ),
        ));
    }
    match (&status.dir, status.size) {
        (Some(dir), size) => {
            let mut value = dir.clone();
            if let Some(size) = size {
                value.push_str(&format!("  ({})", bytes(size)));
            }
            rows.push(row("saved in", value));
        }
        (None, _) => rows.push(warn("saved in", "memory only; not saved")),
    }
    Section {
        title: "Session".to_owned(),
        head: String::new(),
        rows,
    }
}

fn model(status: &SessionStatus, options: &Options<'_>) -> Section {
    let stats = &status.stats;
    let rule = options.rule();
    let mut rows = Vec::new();
    match &status.model {
        Some(model) => {
            rows.push(row(
                "model",
                format!("{}  ({})", model.info.model, model.info.id),
            ));
            rows.push(row("api", format!("{}  {}", model.api, model.base_url)));
            let key = match model.info.key.as_str() {
                "none" => warn("key", "none"),
                "missing" => warn("key", "missing: the variable is not set"),
                "config" => row("key", "set in the config file"),
                "env" => row("key", "set in the environment"),
                "command" => row("key", "from a command (not run)"),
                other => row("key", other),
            };
            rows.push(key);
            let window = model.info.context_window;
            if let Some(percent) = stats.context_tokens.saturating_mul(100).checked_div(window) {
                let value = format!(
                    "{} / {}  {}  {percent}%",
                    tokens(stats.context_tokens),
                    short_tokens(window),
                    bar(percent, options.unicode)
                );
                rows.push(if percent >= WARN_PERCENT {
                    warn("context", value)
                } else {
                    row("context", value)
                });
            }
            let temperature = model
                .temperature
                .map_or_else(|| "default".to_owned(), |t| t.to_string());
            let output = model
                .max_output_tokens
                .map_or_else(|| "default".to_owned(), short_tokens);
            rows.push(row(
                "settings",
                format!(
                    "temperature {temperature}{rule}max output {output}{rule}max_retries {}",
                    model.max_retries
                ),
            ));
        }
        None => {
            rows.push(warn(
                "model",
                format!(
                    "{}  {} not configured",
                    status.session.model,
                    options.warning()
                ),
            ));
            rows.push(row("context", tokens(stats.context_tokens)));
        }
    }
    if stats.model_changes > 0 {
        let mut value = stats.model_changes.to_string();
        if let Some(first) = &stats.first_model {
            value.push_str(&format!("; first {first}"));
        }
        rows.push(row("changes", value));
    }
    Section {
        title: "Model".to_owned(),
        head: String::new(),
        rows,
    }
}

fn this_turn(status: &SessionStatus) -> Option<Section> {
    let turn = status
        .stats
        .turn
        .filter(|_| status.session.state != State::Saved)?;
    let of = |n: u64, limit: Option<u32>| match limit {
        Some(limit) => format!("{n} / {limit}"),
        None => n.to_string(),
    };
    let mut rows = vec![
        row(
            "requests",
            of(turn.requests, status.limits.map(|l| l.max_requests)),
        ),
        row(
            "tool calls",
            of(turn.tool_calls, status.limits.map(|l| l.max_tool_calls)),
        ),
    ];
    if turn.continued > 0 {
        rows.push(row(
            "continued",
            format!(
                "{}; the counts start again",
                count(turn.continued, "time", "times")
            ),
        ));
    }
    if let Some(started) = turn.started {
        rows.push(row("running", duration(status.now.saturating_sub(started))));
    }
    Some(Section {
        title: "This turn".to_owned(),
        head: String::new(),
        rows,
    })
}

fn totals(status: &SessionStatus, options: &Options<'_>) -> Section {
    let stats = &status.stats;
    let rule = options.rule();
    let turns = &stats.turns;
    let mut turn_value = turns.total().to_string();
    let detail = parts(
        &[
            (turns.done, "done"),
            (turns.cancelled, "cancelled"),
            (turns.limit, "limit"),
            (turns.failed, "failed"),
        ],
        ", ",
    );
    if !detail.is_empty() {
        turn_value.push_str(&format!(": {detail}"));
    }
    let mut requests = stats.requests.to_string();
    let more = parts(
        &[
            (
                stats.retries,
                if stats.retries == 1 {
                    "retry"
                } else {
                    "retries"
                },
            ),
            (
                stats.errors,
                if stats.errors == 1 { "error" } else { "errors" },
            ),
        ],
        rule,
    );
    if !more.is_empty() {
        requests.push_str(rule);
        requests.push_str(&more);
    }
    let (working, exact) = stats.working_time(status.now);
    let age = status
        .now
        .saturating_sub(status.session.created.saturating_mul(1000));
    let working = if !exact && working == 0 {
        "?".to_owned()
    } else {
        let at_least = if exact { "" } else { "≥ " };
        let at_least = if options.unicode || exact {
            at_least
        } else {
            ">= "
        };
        format!("{at_least}{} of {}", duration(working), duration(age))
    };
    Section {
        title: "Totals".to_owned(),
        head: String::new(),
        rows: vec![
            row("turns", turn_value),
            row("requests", requests),
            row(
                "tokens",
                format!(
                    "in {}{rule}out {}",
                    tokens(stats.tokens.input_tokens),
                    tokens(stats.tokens.output_tokens)
                ),
            ),
            row("working", working),
        ],
    }
}

fn tool_columns(counts: &ToolCounts) -> String {
    format!(
        "{:>5} {:>5} {:>6} {:>7} {:>10}",
        counts.total(),
        counts.done,
        counts.error,
        counts.denied,
        counts.cancelled
    )
}

fn tools(stats: &Stats) -> Section {
    let mut tools: Vec<(&String, &ToolCounts)> = stats.tools.iter().collect();
    // Most used first; by name among equals.
    tools.sort_by(|a, b| b.1.total().cmp(&a.1.total()).then(a.0.cmp(b.0)));
    let mut rows: Vec<Row> = tools
        .iter()
        .map(|(name, counts)| row(name, tool_columns(counts)))
        .collect();
    if rows.is_empty() {
        return Section {
            title: "Tool calls".to_owned(),
            head: "none".to_owned(),
            rows,
        };
    }
    if rows.len() > 1 {
        rows.push(row("all", tool_columns(&stats.all_tools())));
    }
    Section {
        title: "Tool calls".to_owned(),
        head: format!(
            "{:>5} {:>5} {:>6} {:>7} {:>10}",
            "calls", "done", "error", "denied", "cancelled"
        ),
        rows,
    }
}

fn questions(stats: &Stats, options: &Options<'_>) -> Section {
    let rule = options.rule();
    let a = &stats.answers;
    let total = a.total();
    if total == 0 {
        return Section {
            title: "Questions".to_owned(),
            head: "none".to_owned(),
            rows: Vec::new(),
        };
    }
    let head = format!(
        "{total}: {}",
        parts(
            &[
                (a.allow, "yes"),
                (a.always, "always"),
                (a.deny, "no"),
                (a.explain, "explain"),
                (a.cancel, "cancel"),
                (a.go_on, "continue"),
                (a.stop, "stop"),
                (a.reply, "answered"),
                (a.decline, "declined"),
                (a.no_one, "closed"),
                (a.cancelled, "cancelled"),
            ],
            rule,
        )
    );
    let mut rows = Vec::new();
    if !stats.by.is_empty() {
        let mut by: Vec<(&String, &u64)> = stats.by.iter().collect();
        by.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        let by: Vec<String> = by.iter().map(|(name, n)| format!("{name} {n}")).collect();
        rows.push(row("", format!("by {}", by.join(rule))));
    }
    Section {
        title: "Questions".to_owned(),
        head,
        rows,
    }
}

fn server(status: &SessionStatus, options: &Options<'_>) -> Section {
    let rule = options.rule();
    let s = &status.server;
    Section {
        title: "Server".to_owned(),
        head: format!(
            "{}{rule}pid {}{rule}{}, {} running{rule}{}",
            s.version,
            s.pid,
            count(
                u64::try_from(s.sessions).unwrap_or(u64::MAX),
                "session",
                "sessions"
            ),
            s.running,
            count(
                u64::try_from(s.clients).unwrap_or(u64::MAX),
                "client",
                "clients"
            ),
        ),
        rows: Vec::new(),
    }
}

/// A title line or a row, padded so that values line up.
pub fn line(label: &str, value: &str, title: bool) -> String {
    let text = if title {
        format!("{label:<width$}{value}", width = LABEL_WIDTH + 2)
    } else {
        format!("  {label:<LABEL_WIDTH$}{value}")
    };
    text.trim_end().to_owned()
}

/// The sections as text, with a blank line between them, for `clai chat`.
pub fn text(sections: &[Section]) -> String {
    let mut out = String::new();
    for (i, section) in sections.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&line(&section.title, &section.head, true));
        out.push('\n');
        for row in &section.rows {
            out.push_str(&line(&row.label, &row.value, false));
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    use crate::protocol::{SessionInfo, Stats, Status, TokenTotals};

    /// A saved session whose model is gone.
    pub(crate) fn saved() -> SessionStatus {
        let mut stats = Stats::default();
        stats.count(
            &crate::agent::SessionEvent::Usage {
                input_tokens: 1200,
                output_tokens: 30,
                context_tokens: 1230,
            },
            None,
        );
        SessionStatus {
            session: SessionInfo {
                id: 3,
                name: "docs".into(),
                workspace: "~/ws/docs".into(),
                path: "/home/u/ws/docs".into(),
                model: "fake/gone".into(),
                state: State::Saved,
                created: 1_793_491_200,
                updated: 0,
                turns: 0,
                queued: 0,
                question: None,
                usage: TokenTotals::default(),
            },
            stats,
            clients: Vec::new(),
            dir: None,
            size: None,
            extra_dirs: Vec::new(),
            model: None,
            limits: None,
            now: 1_793_491_200_000 + 90_000,
            server: Status {
                pid: 7,
                sessions: 1,
                running: 0,
                busy: 0,
                clients: 1,
                version: "0.4.0".into(),
            },
        }
    }

    #[test]
    fn a_saved_session_as_text() {
        let options = Options {
            unicode: false,
            offset: 0,
            me: "clai chat",
        };
        let text = text(&sections(&saved(), &options));
        assert_eq!(
            text,
            "\
Session
  name         docs  (id 3)
  state        saved, not loaded
  workspace    ~/ws/docs
  created      2026-11-01 00:00  (1m 30s ago)
  saved in     memory only; not saved

Model
  model        fake/gone  ! not configured
  context      1.2k

Totals
  turns        0
  requests     1
  tokens       in 1.2k - out 30
  working      0s of 1m 30s

Tool calls     none

Questions      none

Server         0.4.0 - pid 7 - 1 session, 0 running - 1 client
"
        );
    }

    #[test]
    fn durations() {
        assert_eq!(duration(12_400), "12s");
        assert_eq!(duration(252_000), "4m 12s");
        assert_eq!(duration(7_800_000), "2h 10m");
        assert_eq!(duration(273_600_000), "3d 4h");
        assert_eq!(local_time(1_793_491_200 + 3_660, 3_600), "2026-11-01 02:01");
    }

    #[test]
    fn sizes_and_bars() {
        assert_eq!(bytes(512), "512 B");
        assert_eq!(bytes(34_000), "34 kB");
        assert_eq!(bytes(1_234_567), "1.2 MB");
        assert_eq!(bar(24, false), "[####................]");
        assert_eq!(bar(150, true), "█".repeat(20));
    }
}
