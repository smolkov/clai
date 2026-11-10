//! The entries of the conversation, built from session events, and their
//! lines on the screen. Each entry caches its lines per width; a change or a
//! resize clears only what it has to, so that long sessions scroll without
//! reflowing everything.

use std::collections::HashMap;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;
use unicode_width::UnicodeWidthStr;

use crate::agent::{CallResult, CallState, ToolCallEvent};
use crate::config::ToolPreview;
use crate::text::{main_argument, short_result};
use crate::tools::{Details, group};

use super::highlight;
use super::markdown;
use super::style::Look;
use super::wrap::{Piece, Source, copy_text, fit, wrap_rows};

/// Live output kept for a running command; the result replaces it.
const MAX_OUTPUT: usize = 1 << 20;
/// Cells of a `HH:MM ` prefix.
const STAMP: usize = 6;
/// Characters of the short result in a call's summary line.
const SHORT_RESULT: usize = 24;
/// The last lines of a command's output in its preview.
const COMMAND_TAIL: usize = 6;

/// A drawn row and where it comes from.
pub type Drawn = (Line<'static>, Source);

/// How much of a tool call is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expansion {
    Collapsed,
    /// What `tui.tool_preview` shows of a finished call: the diff, the end
    /// of a command's output, the error.
    Preview,
    /// With the details, long output cut at `tui.tool_output_lines`.
    Expanded,
    /// With the whole result.
    Full,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub state: CallState,
    pub summary: Option<String>,
    pub arguments: Value,
    /// Live output of a running command, so far.
    pub output: String,
    pub result: Option<CallResult>,
    /// From the permission question, if the call asked.
    pub details: Option<Details>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    User(String),
    /// Markdown.
    Text(String),
    Call(Box<ToolCall>),
    Notice(String),
    Error(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub kind: Kind,
    /// `HH:MM`, when `tui.timestamps` is on and the entry came in live.
    stamp: Option<String>,
    /// The rows at the conversation's width; `None` after a change.
    cache: Option<Vec<Drawn>>,
}

/// What a row of the conversation shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Row<'a> {
    /// The entry it belongs to; `None` for queued messages.
    pub entry: Option<usize>,
    /// Whether it is the first row of the entry (not the gap before it).
    pub first: bool,
    pub line: Option<&'a Line<'static>>,
    pub source: Option<&'a Source>,
}

#[derive(Debug)]
pub struct Conversation {
    pub look: Look,
    pub entries: Vec<Entry>,
    /// Messages sent whose `user_message` event has not come yet.
    pub queued: Vec<String>,
    /// Given to new entries; see [`Entry`].
    pub stamp: Option<String>,
    /// `tui.tool_output_lines`; 0 = no limit.
    limit: usize,
    /// `tui.tool_preview`.
    preview: ToolPreview,
    /// What the user chose, by call id; survives a re-attach.
    expanded: HashMap<String, Expansion>,
    /// Entries of tool calls by call id.
    calls: HashMap<String, usize>,
    /// The text entry deltas go to.
    open_text: Option<usize>,
    spinner: usize,
    width: usize,
    /// The first row of each entry, with the gap before it.
    tops: Vec<usize>,
    queued_lines: Vec<Drawn>,
    queued_top: usize,
}

impl Conversation {
    pub fn new(look: Look, limit: usize, preview: ToolPreview) -> Conversation {
        Conversation {
            look,
            entries: Vec::new(),
            queued: Vec::new(),
            stamp: None,
            limit,
            preview,
            expanded: HashMap::new(),
            calls: HashMap::new(),
            open_text: None,
            spinner: 0,
            width: 0,
            tops: Vec::new(),
            queued_lines: Vec::new(),
            queued_top: 0,
        }
    }

    /// Forgets the entries before a replay; what the user chose and the
    /// queued messages stay.
    pub fn clear(&mut self) {
        self.entries.clear();
        self.calls.clear();
        self.open_text = None;
        self.tops.clear();
    }

    /// New settings: every entry is laid out again with them. What the user
    /// expanded or collapsed stays.
    pub fn reconfigure(&mut self, look: Look, limit: usize, preview: ToolPreview) {
        self.look = look;
        self.limit = limit;
        self.preview = preview;
        for entry in &mut self.entries {
            entry.cache = None;
        }
    }

    /// What the user chose to expand, and the queued messages: the part of a
    /// session's view kept while the TUI shows another session.
    pub fn take_choices(&mut self) -> (HashMap<String, Expansion>, Vec<String>) {
        (
            std::mem::take(&mut self.expanded),
            std::mem::take(&mut self.queued),
        )
    }

    pub fn set_choices(&mut self, expanded: HashMap<String, Expansion>, queued: Vec<String>) {
        self.expanded = expanded;
        self.queued = queued;
    }

    fn push(&mut self, kind: Kind) {
        self.open_text = None;
        self.entries.push(Entry {
            kind,
            stamp: self.stamp.clone(),
            cache: None,
        });
    }

    pub fn notice(&mut self, text: impl Into<String>) {
        self.push(Kind::Notice(text.into()));
    }

    pub fn error(&mut self, text: impl Into<String>) {
        self.push(Kind::Error(text.into()));
    }

    /// A `user_message` event: the message is not queued any more.
    pub fn user(&mut self, text: &str) {
        self.unqueue(text);
        self.push(Kind::User(text.to_owned()));
    }

    pub fn unqueue(&mut self, text: &str) {
        if let Some(at) = self.queued.iter().position(|q| q == text) {
            self.queued.remove(at);
        }
    }

    pub fn text_delta(&mut self, text: &str) {
        if let Some(entry) = self.open_text.and_then(|i| self.entries.get_mut(i))
            && let Kind::Text(all) = &mut entry.kind
        {
            all.push_str(text);
            entry.cache = None;
            return;
        }
        self.push(Kind::Text(text.to_owned()));
        self.open_text = self.entries.len().checked_sub(1);
    }

    /// The whole text of the block: the source of truth.
    pub fn text_done(&mut self, text: &str) {
        match self.open_text.take().and_then(|i| self.entries.get_mut(i)) {
            Some(entry) => {
                entry.kind = Kind::Text(text.to_owned());
                entry.cache = None;
            }
            None if !text.is_empty() => self.push(Kind::Text(text.to_owned())),
            None => {}
        }
    }

    /// Ends the text block, if deltas came without their `text_done`.
    pub fn close_text(&mut self) {
        self.open_text = None;
    }

    fn call_mut(&mut self, id: &str) -> Option<&mut Entry> {
        let at = *self.calls.get(id)?;
        self.entries.get_mut(at)
    }

    fn call_state(&self, id: &str) -> Option<CallState> {
        let entry = self.entries.get(*self.calls.get(id)?)?;
        match &entry.kind {
            Kind::Call(call) => Some(call.state),
            _ => None,
        }
    }

    pub fn tool_call(&mut self, event: &ToolCallEvent) {
        // Some models use an id again; a finished call stays as it is then.
        let known = self.call_state(&event.id);
        if known.is_none_or(|state| state.is_final() && !event.state.is_final()) {
            self.push(Kind::Call(Box::new(ToolCall {
                id: event.id.clone(),
                name: event.name.clone(),
                state: event.state,
                summary: None,
                arguments: Value::Null,
                output: String::new(),
                result: None,
                details: None,
            })));
            self.calls
                .insert(event.id.clone(), self.entries.len().saturating_sub(1));
        }
        let Some(entry) = self.call_mut(&event.id) else {
            return;
        };
        entry.cache = None;
        let Kind::Call(call) = &mut entry.kind else {
            return;
        };
        call.state = event.state;
        if event.summary.is_some() {
            call.summary.clone_from(&event.summary);
        }
        if !event.arguments.is_null() {
            call.arguments.clone_from(&event.arguments);
        }
        if let Some(output) = &event.output
            && call.output.len() < MAX_OUTPUT
        {
            call.output.push_str(output);
        }
        if event.result.is_some() {
            call.result.clone_from(&event.result);
        }
        if let Some(details) = &event.details {
            call.details = Some((**details).clone());
        }
    }

    /// What a permission question says about its call.
    pub fn set_details(&mut self, id: &str, name: &str, details: &Details) {
        if !self.calls.contains_key(id) {
            let event = ToolCallEvent {
                details: None,
                id: id.to_owned(),
                name: name.to_owned(),
                state: CallState::Waiting,
                summary: None,
                arguments: Value::Null,
                output: None,
                result: None,
            };
            self.tool_call(&event);
        }
        if let Some(entry) = self.call_mut(id) {
            entry.cache = None;
            if let Kind::Call(call) = &mut entry.kind {
                call.details = Some(details.clone());
            }
        }
    }

    pub fn entry_of_call(&self, id: &str) -> Option<usize> {
        self.calls.get(id).copied()
    }

    /// Whether a tool call is running.
    pub fn running_tool(&self) -> bool {
        self.calls.values().any(|i| {
            matches!(
                self.entries.get(*i).map(|e| &e.kind),
                Some(Kind::Call(call)) if call.state == CallState::Running
            )
        })
    }

    /// Whether a call shows a spinner.
    pub fn spinning(&self) -> bool {
        self.running_calls().next().is_some()
    }

    /// The calls that are pending or running, in the order of the
    /// conversation.
    pub fn running_calls(&self) -> impl Iterator<Item = &ToolCall> {
        let mut at: Vec<usize> = self.calls.values().copied().collect();
        at.sort_unstable();
        at.into_iter()
            .filter_map(|i| match self.entries.get(i).map(|e| &e.kind) {
                Some(Kind::Call(call))
                    if matches!(call.state, CallState::Pending | CallState::Running) =>
                {
                    Some(&**call)
                }
                _ => None,
            })
    }

    /// The next spinner frame; `true` if a spinner is on the screen.
    pub fn tick(&mut self) -> bool {
        self.spinner = self.spinner.wrapping_add(1);
        let mut any = false;
        for at in self.calls.values() {
            if let Some(entry) = self.entries.get_mut(*at)
                && let Kind::Call(call) = &entry.kind
                && matches!(call.state, CallState::Pending | CallState::Running)
            {
                entry.cache = None;
                any = true;
            }
        }
        any
    }

    fn render(&self) -> Render<'_> {
        Render {
            look: &self.look,
            limit: self.limit,
            preview: self.preview,
            expanded: &self.expanded,
            spinner: self.spinner,
        }
    }

    /// Moves a tool call on: collapsed → expanded → whole result (if cut) →
    /// collapsed; a preview goes to expanded. `false` if the entry is no
    /// tool call.
    pub fn toggle(&mut self, entry: usize) -> bool {
        let Some(Entry {
            kind: Kind::Call(call),
            ..
        }) = self.entries.get(entry)
        else {
            return false;
        };
        let render = self.render();
        let next = match render.expansion(call) {
            Expansion::Collapsed | Expansion::Preview => Expansion::Expanded,
            Expansion::Expanded if render.is_cut(call) => Expansion::Full,
            Expansion::Expanded | Expansion::Full => Expansion::Collapsed,
        };
        self.choose(entry, next);
        true
    }

    fn choose(&mut self, entry: usize, expansion: Expansion) {
        if let Some(entry) = self.entries.get_mut(entry)
            && let Kind::Call(call) = &entry.kind
        {
            self.expanded.insert(call.id.clone(), expansion);
            entry.cache = None;
        }
    }

    /// The entries after the last message of the user: the last turn.
    fn last_turn(&self) -> std::ops::Range<usize> {
        let start = self
            .entries
            .iter()
            .rposition(|e| matches!(e.kind, Kind::User(_)))
            .map_or(0, |i| i + 1);
        start..self.entries.len()
    }

    /// `Ctrl-T`: the calls of the last turn are all collapsed if any shows
    /// more than its summary, else they show their preview (or expand when
    /// they have none). `false` without calls.
    pub fn toggle_last_turn(&mut self) -> bool {
        let calls: Vec<usize> = self
            .last_turn()
            .filter(|i| matches!(self.entries.get(*i).map(|e| &e.kind), Some(Kind::Call(_))))
            .collect();
        if calls.is_empty() {
            return false;
        }
        let render = self.render();
        let shown = |i: &usize| match self.entries.get(*i).map(|e| &e.kind) {
            Some(Kind::Call(call)) => render.expansion(call) != Expansion::Collapsed,
            _ => false,
        };
        let open = calls.iter().any(shown);
        let wanted: Vec<(usize, Expansion)> = calls
            .iter()
            .map(|i| {
                let next = match self.entries.get(*i).map(|e| &e.kind) {
                    _ if open => Expansion::Collapsed,
                    Some(Kind::Call(call)) if render.has_preview(call) => Expansion::Preview,
                    _ => Expansion::Expanded,
                };
                (*i, next)
            })
            .collect();
        for (i, next) in wanted {
            self.choose(i, next);
        }
        true
    }

    /// The text `y` copies of an entry (see `spec/tui.md`, "Copy").
    pub fn entry_text(&self, entry: usize) -> Option<String> {
        Some(match &self.entries.get(entry)?.kind {
            Kind::User(text) | Kind::Text(text) | Kind::Notice(text) | Kind::Error(text) => {
                text.clone()
            }
            Kind::Call(call) => call_text(call),
        })
    }

    /// The code blocks of an answer, as their source.
    pub fn code_blocks(&self, entry: usize) -> Vec<String> {
        match self.entries.get(entry).map(|e| &e.kind) {
            Some(Kind::Text(text)) => markdown::code_blocks(text),
            _ => Vec::new(),
        }
    }

    /// The answers of the model, the last first.
    pub fn answers(&self) -> Vec<usize> {
        self.entries
            .iter()
            .enumerate()
            .rev()
            .filter(|(_, e)| matches!(e.kind, Kind::Text(_)))
            .map(|(i, _)| i)
            .collect()
    }

    /// The diffs of the edits and writes of the last turn, one after the
    /// other, for `/diff`.
    pub fn last_diffs(&self) -> Vec<&ToolCall> {
        self.last_turn()
            .filter_map(|i| match self.entries.get(i).map(|e| &e.kind) {
                Some(Kind::Call(call))
                    if matches!(call.name.as_str(), "edit_file" | "write_file")
                        && call.state == CallState::Done =>
                {
                    Some(&**call)
                }
                _ => None,
            })
            .collect()
    }

    /// The call of an entry.
    pub fn call(&self, entry: usize) -> Option<&ToolCall> {
        match &self.entries.get(entry)?.kind {
            Kind::Call(call) => Some(call),
            _ => None,
        }
    }

    /// The details view of a call: its summary as the title, and all of it.
    pub fn details(&self, entry: usize) -> Option<super::details::Details> {
        let call = self.call(entry)?;
        let render = self.render();
        let (mark, _) = render.state_mark(call);
        let summary = call
            .summary
            .clone()
            .unwrap_or_else(|| main_argument(&call.arguments));
        let summary = summary.lines().next().unwrap_or_default().to_owned();
        let rule = self.look.symbols.rule;
        let title = format!("{} {summary} {rule} {mark}", call.name);
        Some(super::details::Details::new(
            title,
            render.body(call),
            cut_note(call),
        ))
    }

    /// `/diff`: the diffs of the edits and writes of the last turn, each
    /// under its summary; `None` without any.
    pub fn diff_details(&self) -> Option<super::details::Details> {
        let calls = self.last_diffs();
        if calls.is_empty() {
            return None;
        }
        let render = self.render();
        let mut lines: Vec<Vec<Piece>> = Vec::new();
        let mut notes = Vec::new();
        for call in &calls {
            if !lines.is_empty() {
                lines.push(Vec::new());
            }
            let summary = call
                .summary
                .clone()
                .unwrap_or_else(|| main_argument(&call.arguments));
            lines.push(vec![(
                format!("{} {summary}", call.name),
                self.look.styles.heading,
            )]);
            lines.extend(render.body(call));
            notes.extend(cut_note(call));
        }
        let count = calls.len();
        let title = format!(
            "Changes of the last turn {} {count} {}",
            self.look.symbols.rule,
            if count == 1 { "file" } else { "files" }
        );
        notes.dedup();
        let note = (!notes.is_empty()).then(|| notes.join("; "));
        Some(super::details::Details::new(title, lines, note))
    }

    /// The text of rows `from.0..=to.0`, from cell `from.1` of the first up to
    /// cell `to.1` of the last, as the user reads it.
    pub fn copy(&self, from: (usize, usize), to: (usize, usize)) -> String {
        let wanted = to.0.saturating_sub(from.0) + 1;
        let rows = self.rows(from.0, wanted);
        // A selection past the last row takes all of it.
        let to_cell = if rows.len() < wanted {
            usize::MAX
        } else {
            to.1
        };
        copy_text(rows.iter().map(|r| r.line.zip(r.source)), from.1, to_cell)
    }

    /// The messages of the user, oldest first.
    pub fn user_messages(&self) -> Vec<String> {
        self.entries
            .iter()
            .filter_map(|e| match &e.kind {
                Kind::User(text) => Some(text.clone()),
                _ => None,
            })
            .chain(self.queued.iter().cloned())
            .collect()
    }

    /// Brings the lines of all entries up to date for `width` cells and adds
    /// up the rows. Only entries without lines are built.
    pub fn layout(&mut self, width: usize) {
        if width != self.width {
            self.width = width;
            for entry in &mut self.entries {
                entry.cache = None;
            }
        }
        let built: Vec<(usize, Vec<Drawn>)> = {
            let render = self.render();
            self.entries
                .iter()
                .enumerate()
                .filter(|(_, e)| e.cache.is_none())
                .map(|(i, e)| (i, render.entry(e, width)))
                .collect()
        };
        for (i, lines) in built {
            if let Some(entry) = self.entries.get_mut(i) {
                entry.cache = Some(lines);
            }
        }
        self.tops.clear();
        let mut row = 0;
        for i in 0..self.entries.len() {
            self.tops.push(row);
            row += self.gap(i) + self.height(i);
        }
        self.queued_top = row;
        self.queued_lines.clear();
        let dim = self.look.styles.dim;
        for text in &self.queued {
            if !self.entries.is_empty() || !self.queued_lines.is_empty() {
                self.queued_lines.push((Line::default(), Source::default()));
            }
            let pieces = [
                (text.clone(), dim),
                (" (queued)".to_owned(), dim.add_modifier(Modifier::ITALIC)),
            ];
            let first = ("> ".to_owned(), dim);
            let rest = ("  ".to_owned(), dim);
            self.queued_lines
                .extend(wrap_rows(&pieces, width, &first, &rest));
        }
    }

    fn gap(&self, i: usize) -> usize {
        let is_call =
            |i: usize| matches!(self.entries.get(i).map(|e| &e.kind), Some(Kind::Call(_)));
        usize::from(i > 0 && !(is_call(i) && is_call(i - 1)))
    }

    fn height(&self, i: usize) -> usize {
        self.entries
            .get(i)
            .and_then(|e| e.cache.as_ref())
            .map_or(0, Vec::len)
    }

    /// All rows, after [`Conversation::layout`].
    pub fn total(&self) -> usize {
        self.queued_top + self.queued_lines.len()
    }

    /// The first row of an entry (after its gap) and its height.
    pub fn span_of(&self, i: usize) -> Option<(usize, usize)> {
        let top = self.tops.get(i)?;
        Some((top + self.gap(i), self.height(i)))
    }

    /// The entry a row belongs to.
    pub fn entry_at(&self, row: usize) -> Option<usize> {
        self.tops.partition_point(|t| *t <= row).checked_sub(1)
    }

    /// The rows `top..top + height`, as far as there are any. Uses the added
    /// up rows: its cost does not grow with the length of the session.
    pub fn rows(&self, top: usize, height: usize) -> Vec<Row<'_>> {
        let mut rows = Vec::with_capacity(height);
        for row in top..top + height {
            if row >= self.total() {
                break;
            }
            if row >= self.queued_top {
                let queued = self.queued_lines.get(row - self.queued_top);
                rows.push(Row {
                    entry: None,
                    first: false,
                    line: queued.map(|(l, _)| l),
                    source: queued.map(|(_, s)| s),
                });
                continue;
            }
            let Some(i) = self.entry_at(row) else { break };
            let Some((start, _)) = self.span_of(i) else {
                break;
            };
            let drawn = row.checked_sub(start).and_then(|at| {
                self.entries
                    .get(i)
                    .and_then(|e| e.cache.as_ref())
                    .and_then(|lines| lines.get(at))
            });
            rows.push(Row {
                entry: Some(i),
                first: row == start,
                line: drawn.map(|(l, _)| l),
                source: drawn.map(|(_, s)| s),
            });
        }
        rows
    }
}

/// What the lines of an entry depend on.
struct Render<'a> {
    look: &'a Look,
    limit: usize,
    preview: ToolPreview,
    expanded: &'a HashMap<String, Expansion>,
    spinner: usize,
}

impl Render<'_> {
    fn expansion(&self, call: &ToolCall) -> Expansion {
        if let Some(chosen) = self.expanded.get(&call.id) {
            return *chosen;
        }
        if self.preview == ToolPreview::All || call.state == CallState::Waiting {
            Expansion::Expanded
        } else if self.preview == ToolPreview::Changes && self.has_preview(call) {
            Expansion::Preview
        } else {
            Expansion::Collapsed
        }
    }

    /// Whether a call has a preview: a finished edit, write or command, a
    /// command that runs and has output, a failed call.
    fn has_preview(&self, call: &ToolCall) -> bool {
        match call.state {
            CallState::Error => true,
            CallState::Done => matches!(
                call.name.as_str(),
                "edit_file" | "write_file" | "run_command"
            ),
            CallState::Running => call.name == "run_command" && !call.output.is_empty(),
            _ => false,
        }
    }

    /// Whether the expanded call shows only a part of its details. The
    /// change or command of an open question is always shown in full.
    fn is_cut(&self, call: &ToolCall) -> bool {
        self.limit > 0 && call.state != CallState::Waiting && self.body(call).len() > self.limit
    }

    fn entry(&self, entry: &Entry, width: usize) -> Vec<Drawn> {
        let inner = if entry.stamp.is_some() {
            width.saturating_sub(STAMP)
        } else {
            width
        };
        let mut lines = match &entry.kind {
            Kind::User(text) => self.user(text, inner),
            Kind::Text(text) => {
                let lines = markdown::rows(text, inner, self.look);
                if lines.is_empty() {
                    vec![(Line::default(), Source::default())]
                } else {
                    lines
                }
            }
            Kind::Call(call) => self.call(call, inner),
            Kind::Notice(text) => plain_rows(text, inner, self.look.styles.dim),
            Kind::Error(text) => plain_rows(text, inner, self.look.styles.error),
        };
        if let (Some(stamp), Some((first, source))) = (&entry.stamp, lines.first_mut()) {
            first
                .spans
                .insert(0, Span::styled(format!("{stamp} "), self.look.styles.dim));
            source.prefix += STAMP;
            source.end = source.end.saturating_add(STAMP);
        }
        lines
    }

    fn user(&self, text: &str, width: usize) -> Vec<Drawn> {
        let style = self.look.styles.user;
        let prompt = ("> ".to_owned(), style);
        let more = ("  ".to_owned(), style);
        let mut lines = Vec::new();
        for (i, line) in text.split('\n').enumerate() {
            let first = if i == 0 { &prompt } else { &more };
            lines.extend(wrap_rows(&[(line.to_owned(), style)], width, first, &more));
        }
        lines
    }

    fn call(&self, call: &ToolCall, width: usize) -> Vec<Drawn> {
        let expansion = self.expansion(call);
        let mut lines = vec![self.summary(call, expansion, width)];
        let indent = ("  ".to_owned(), self.look.styles.dim);
        let more = |lines: &mut Vec<Drawn>, text: String| {
            for (line, _) in wrap_rows(&[(text, self.look.styles.dim)], width, &indent, &indent) {
                lines.push((line, Source::skipped()));
            }
        };
        let (body, hidden, shown) = match expansion {
            Expansion::Collapsed => return lines,
            Expansion::Preview => self.preview_body(call),
            Expansion::Expanded | Expansion::Full => {
                let body = self.body(call);
                let total = body.len();
                let shown = if expansion == Expansion::Expanded && self.is_cut(call) {
                    self.limit
                } else {
                    total
                };
                (body, None, shown)
            }
        };
        let total = body.len();
        for (i, pieces) in body.into_iter().enumerate().take(shown) {
            let rows = wrap_rows(&pieces, width, &indent, &indent);
            if hidden == Some(i) {
                lines.extend(rows.into_iter().map(|(line, _)| (line, Source::skipped())));
            } else {
                lines.extend(rows);
            }
        }
        if shown < total {
            more(
                &mut lines,
                format!(
                    "{} {} more lines",
                    self.look.symbols.more,
                    group(total - shown)
                ),
            );
        }
        lines
    }

    /// The preview of a call: its pieces, where a `… N lines` row goes (as a
    /// piece of its own) and how many pieces to show.
    fn preview_body(&self, call: &ToolCall) -> (Vec<Vec<Piece>>, Option<usize>, usize) {
        let styles = &self.look.styles;
        let limit = if self.limit == 0 {
            usize::MAX
        } else {
            self.limit
        };
        if call.state == CallState::Error
            || (call.name != "run_command"
                && !matches!(call.name.as_str(), "edit_file" | "write_file"))
        {
            let mut out = Vec::new();
            if let Some(result) = &call.result {
                lines_into(&mut out, &result.text, styles.error);
            }
            let shown = out.len().min(limit);
            return (out, None, shown);
        }
        if call.name == "run_command" {
            let mut out = Vec::new();
            if let Some(command) = command_of(call) {
                self.command(&mut out, &command);
            }
            let output = match &call.result {
                Some(result) => result.text.as_str(),
                None => call.output.as_str(),
            };
            let all: Vec<&str> = output.lines().collect();
            let hidden = all.len().saturating_sub(COMMAND_TAIL);
            if hidden > 0 {
                out.push(vec![(
                    format!("{} {} lines", self.look.symbols.more, group(hidden)),
                    styles.dim,
                )]);
            }
            let marker = (hidden > 0).then(|| out.len() - 1);
            for line in all.iter().skip(hidden) {
                out.push(vec![((*line).to_owned(), styles.text)]);
            }
            let shown = out.len();
            return (out, marker, shown);
        }
        let body = self.body(call);
        let shown = body.len().min(limit);
        (body, None, shown)
    }

    /// `▸ read_file src/parse.rs            ✓ 120 lines`
    fn summary(&self, call: &ToolCall, expansion: Expansion, width: usize) -> Drawn {
        let symbols = &self.look.symbols;
        let styles = &self.look.styles;
        let marker = if expansion == Expansion::Collapsed {
            symbols.collapsed
        } else {
            symbols.expanded
        };
        let (mark, style) = self.state_mark(call);
        let mark = fit(&mark, width / 2, symbols.more);
        let summary = call
            .summary
            .clone()
            .unwrap_or_else(|| main_argument(&call.arguments));
        let name = fit(&call.name, width, symbols.more);
        let head_width = marker.width() + 1 + name.width();
        let room = width.saturating_sub(head_width + mark.width() + 2);
        let summary = fit(
            summary.lines().next().unwrap_or_default(),
            room,
            symbols.more,
        );
        let left = head_width + usize::from(!summary.is_empty()) + summary.width();
        let pad = width.saturating_sub(left + mark.width()).max(1);
        let source = Source {
            end: left,
            ..Source::text(marker.width() + 1)
        };
        let line = Line::from(vec![
            Span::styled(format!("{marker} "), styles.dim),
            Span::styled(name, styles.text.add_modifier(Modifier::BOLD)),
            Span::styled(
                if summary.is_empty() {
                    String::new()
                } else {
                    format!(" {summary}")
                },
                styles.text,
            ),
            Span::raw(" ".repeat(pad)),
            Span::styled(mark, style),
        ]);
        (line, source)
    }

    fn state_mark(&self, call: &ToolCall) -> (String, Style) {
        let symbols = &self.look.symbols;
        let styles = &self.look.styles;
        let short = || {
            let text = call.result.as_ref().map_or("", |r| r.text.as_str());
            let short = short_result(&call.name, call.state, text);
            let cut = fit(&short, SHORT_RESULT, symbols.more);
            if cut.is_empty() {
                String::new()
            } else {
                format!(" {cut}")
            }
        };
        match call.state {
            CallState::Done => (format!("{}{}", symbols.done, short()), styles.success),
            CallState::Error => (format!("{}{}", symbols.failed, short()), styles.error),
            CallState::Denied => (format!("{} denied", symbols.denied), styles.warning),
            CallState::Cancelled => (format!("{} cancelled", symbols.denied), styles.warning),
            CallState::Waiting => (format!("{} ask", symbols.waiting), styles.warning),
            CallState::Pending | CallState::Running => {
                let frames = symbols.spinner;
                let frame = frames
                    .get(self.spinner % frames.len().max(1))
                    .copied()
                    .unwrap_or_default();
                (frame.to_owned(), styles.accent)
            }
        }
    }

    /// The details of an expanded call, the pieces of each line.
    fn body(&self, call: &ToolCall) -> Vec<Vec<Piece>> {
        let styles = &self.look.styles;
        let mut out: Vec<Vec<Piece>> = Vec::new();
        let string = |key: &str| call.arguments.get(key).and_then(Value::as_str);
        match &call.details {
            Some(Details::EditFile { diff, path, .. }) => {
                self.diff(&mut out, &diff.text, Some(&path.resolved));
            }
            Some(Details::WriteFile {
                diff,
                content,
                path,
                ..
            }) => {
                if let Some(diff) = diff {
                    self.diff(&mut out, &diff.text, Some(&path.resolved));
                }
                if let Some(content) = content {
                    self.file(
                        &mut out,
                        &content.text,
                        Some(&path.resolved),
                        styles.diff_add,
                    );
                }
            }
            Some(Details::RunCommand {
                command,
                cwd,
                parts,
                redirects,
                ..
            }) => {
                self.command(&mut out, command);
                out.push(vec![(format!("in {cwd}"), styles.dim)]);
                if call.state == CallState::Waiting {
                    for part in parts {
                        let line = format!("{}  {}", part.text, part.decision.reason);
                        out.push(vec![(line, styles.dim)]);
                    }
                    for r in redirects {
                        let arrow = if r.write { ">" } else { "<" };
                        let line = format!("{arrow} {}  {}", r.target, r.decision.reason);
                        out.push(vec![(line, styles.dim)]);
                    }
                }
            }
            Some(Details::ReadFile {
                path,
                first_line,
                last_line,
                ..
            }) => {
                let line = format!("{}, lines {first_line}–{last_line}", path.resolved);
                out.push(vec![(line, styles.dim)]);
            }
            Some(Details::ListFiles { path, pattern }) => {
                out.push(vec![(
                    format!("{}, pattern {pattern}", path.resolved),
                    styles.dim,
                )]);
            }
            Some(Details::SearchFiles { path, pattern, .. }) => {
                out.push(vec![(
                    format!("{pattern} in {}", path.resolved),
                    styles.dim,
                )]);
            }
            None => match call.name.as_str() {
                "edit_file" => {
                    // The old and new text as one hunk.
                    let mut diff = String::new();
                    for line in string("old_text").unwrap_or_default().lines() {
                        diff.push_str(&format!("-{line}\n"));
                    }
                    for line in string("new_text").unwrap_or_default().lines() {
                        diff.push_str(&format!("+{line}\n"));
                    }
                    self.diff(&mut out, &diff, string("path"));
                }
                "write_file" => {
                    if let Some(content) = string("content") {
                        self.file(&mut out, content, string("path"), styles.diff_add);
                    }
                }
                "run_command" => {
                    if let Some(command) = string("command") {
                        self.command(&mut out, command);
                    }
                }
                _ => {}
            },
        }
        let changes = matches!(call.name.as_str(), "edit_file" | "write_file");
        match &call.result {
            // An edit is its diff; its result only matters when it failed.
            Some(result) if result.is_error || !changes => {
                let style = if result.is_error {
                    styles.error
                } else {
                    styles.text
                };
                lines_into(&mut out, &result.text, style);
            }
            Some(_) => {}
            None => lines_into(&mut out, &call.output, styles.text),
        }
        out
    }

    /// A unified diff without its `---` and `+++` header: the summary names
    /// the file. With a known language, the code of each line is
    /// highlighted, the old and the new side of each hunk apart; the `+` and
    /// `-` keep the diff colours.
    fn diff(&self, out: &mut Vec<Vec<Piece>>, text: &str, path: Option<&str>) {
        let header = |i: usize, line: &str| {
            (i == 0 && line.starts_with("--- ")) || (i == 1 && line.starts_with("+++ "))
        };
        let lines: Vec<&str> = text
            .lines()
            .enumerate()
            .filter(|(i, line)| !header(*i, line))
            .map(|(_, line)| line)
            .collect();
        let grammar = path.and_then(|p| highlight::by_path(p, None));
        let mut at = 0;
        while at < lines.len() {
            // A hunk: its `@@` line, then the lines up to the next one.
            let start = at;
            at += 1;
            while lines.get(at).is_some_and(|l| !l.starts_with("@@")) {
                at += 1;
            }
            let hunk = lines.get(start..at).unwrap_or_default();
            self.hunk(out, hunk, path, grammar);
        }
    }

    fn hunk(
        &self,
        out: &mut Vec<Vec<Piece>>,
        hunk: &[&str],
        path: Option<&str>,
        grammar: Option<&'static syntect::parsing::SyntaxReference>,
    ) {
        let styles = &self.look.styles;
        let side = |keep: char| -> String {
            hunk.iter()
                .filter(|l| l.starts_with(keep) || l.starts_with(' ') || l.is_empty())
                .map(|l| format!("{}\n", l.get(1..).unwrap_or_default()))
                .collect()
        };
        let key = format!("file:{}", path.unwrap_or_default());
        let highlighted =
            |text: String| highlight::highlight(styles.syntax, &key, grammar, &text, styles.text);
        let (old, new) = match grammar {
            Some(_) => (highlighted(side('-')), highlighted(side('+'))),
            None => (None, None),
        };
        let tint = |colour: Style| self.tint(colour);
        let (mut old_at, mut new_at) = (0, 0);
        for line in hunk {
            let (marker, code, style) = match line.chars().next() {
                Some('@') => {
                    out.push(vec![((*line).to_owned(), styles.accent)]);
                    continue;
                }
                Some('+') => {
                    new_at += 1;
                    (
                        "+",
                        new.as_ref().and_then(|n| n.get(new_at - 1)),
                        styles.diff_add,
                    )
                }
                Some('-') => {
                    old_at += 1;
                    (
                        "-",
                        old.as_ref().and_then(|o| o.get(old_at - 1)),
                        styles.diff_remove,
                    )
                }
                Some(' ') | None => {
                    old_at += 1;
                    new_at += 1;
                    (
                        " ",
                        new.as_ref().and_then(|n| n.get(new_at - 1)),
                        styles.text,
                    )
                }
                // `\ No newline at end of file`.
                Some(_) => {
                    out.push(vec![((*line).to_owned(), styles.dim)]);
                    continue;
                }
            };
            match code {
                Some(pieces) => {
                    let background = tint(style);
                    let mut row = vec![(marker.to_owned(), style)];
                    row.extend(pieces.iter().cloned());
                    if let Some(bg) = background.filter(|_| marker != " ") {
                        for (_, s) in &mut row {
                            *s = s.bg(bg);
                        }
                    }
                    out.push(row);
                }
                None => out.push(vec![((*line).to_owned(), style)]),
            }
        }
    }

    /// The background of a changed line: its diff colour mixed into the
    /// theme's background, with true colour and a theme that sets one.
    fn tint(&self, diff: Style) -> Option<ratatui::style::Color> {
        use ratatui::style::Color;
        let styles = &self.look.styles;
        if styles.syntax.colouring != super::style::Colouring::TrueColour {
            return None;
        }
        match (styles.background, diff.fg) {
            (Some(Color::Rgb(r, g, b)), Some(Color::Rgb(dr, dg, db))) => {
                let mix = |a: u8, b: u8| {
                    u8::try_from((u16::from(a) * 82 + u16::from(b) * 18) / 100).unwrap_or(a)
                };
                Some(Color::Rgb(mix(r, dr), mix(g, dg), mix(b, db)))
            }
            _ => None,
        }
    }

    /// The text of a file, highlighted by its name, else in `style`.
    fn file(&self, out: &mut Vec<Vec<Piece>>, text: &str, path: Option<&str>, style: Style) {
        let styles = &self.look.styles;
        let grammar = path.and_then(|p| highlight::by_path(p, text.lines().next()));
        let mut complete = text.to_owned();
        if !complete.ends_with('\n') {
            complete.push('\n');
        }
        let key = format!("file:{}", path.unwrap_or_default());
        match highlight::highlight(styles.syntax, &key, grammar, &complete, styles.text) {
            Some(lines) => out.extend(lines),
            None => lines_into(out, text, style),
        }
    }

    /// `$ command`, the command highlighted as shell.
    fn command(&self, out: &mut Vec<Vec<Piece>>, command: &str) {
        let styles = &self.look.styles;
        let mut text = command.to_owned();
        text.push('\n');
        match highlight::highlight(
            styles.syntax,
            "shell",
            highlight::shell(),
            &text,
            styles.text,
        ) {
            Some(lines) => {
                for (i, pieces) in lines.into_iter().enumerate() {
                    let prompt = if i == 0 { "$ " } else { "  " };
                    let mut row = vec![(prompt.to_owned(), styles.dim)];
                    row.extend(pieces);
                    out.push(row);
                }
            }
            None => lines_into(out, &format!("$ {command}"), styles.text),
        }
    }
}

fn lines_into(out: &mut Vec<Vec<Piece>>, text: &str, style: Style) {
    for line in text.lines() {
        out.push(vec![(line.to_owned(), style)]);
    }
}

fn plain_rows(text: &str, width: usize, style: Style) -> Vec<Drawn> {
    let none = (String::new(), Style::new());
    wrap_rows(&[(text.to_owned(), style)], width, &none, &none)
}

fn command_of(call: &ToolCall) -> Option<String> {
    match &call.details {
        Some(Details::RunCommand { command, .. }) => Some(command.clone()),
        _ => call
            .arguments
            .get("command")
            .and_then(Value::as_str)
            .map(str::to_owned),
    }
}

/// A unified diff as `git apply` takes it: the headers name `a/PATH` and
/// `b/PATH` when the path is in the workspace.
fn git_diff(diff: &str, path: &str) -> String {
    let relative = !path.starts_with('/') && !path.starts_with('~');
    let mut out = String::new();
    for (i, line) in diff.lines().enumerate() {
        match (i, relative) {
            (0, true) if line.starts_with("--- ") => out.push_str(&format!("--- a/{path}")),
            (1, true) if line.starts_with("+++ ") => out.push_str(&format!("+++ b/{path}")),
            _ => out.push_str(line),
        }
        out.push('\n');
    }
    out
}

/// The text `y` copies of a tool call.
pub fn call_text(call: &ToolCall) -> String {
    let result = call.result.as_ref().map_or("", |r| r.text.as_str());
    let string = |key: &str| call.arguments.get(key).and_then(Value::as_str);
    match (&call.details, call.name.as_str()) {
        (Some(Details::EditFile { path, diff, .. }), _) if !result_failed(call) => {
            git_diff(&diff.text, &path.resolved)
        }
        (
            Some(Details::WriteFile {
                path,
                diff,
                content,
                ..
            }),
            _,
        ) if !result_failed(call) => match (diff, content) {
            (Some(diff), _) => git_diff(&diff.text, &path.resolved),
            (None, Some(content)) => content.text.clone(),
            (None, None) => result.to_owned(),
        },
        (_, "edit_file") if !result_failed(call) => {
            let mut out = String::new();
            for line in string("old_text").unwrap_or_default().lines() {
                out.push_str(&format!("-{line}\n"));
            }
            for line in string("new_text").unwrap_or_default().lines() {
                out.push_str(&format!("+{line}\n"));
            }
            out
        }
        (_, "write_file") if !result_failed(call) => {
            string("content").unwrap_or_default().to_owned()
        }
        (_, "run_command") => {
            let output = if call.result.is_some() {
                result
            } else {
                call.output.as_str()
            };
            match command_of(call) {
                Some(command) => format!("$ {command}\n{output}"),
                None => output.to_owned(),
            }
        }
        _ => result.to_owned(),
    }
}

fn result_failed(call: &ToolCall) -> bool {
    call.result.as_ref().is_some_and(|r| r.is_error)
}

/// What the details view says when a text of the call was cut.
fn cut_note(call: &ToolCall) -> Option<String> {
    let cut = match &call.details {
        Some(Details::EditFile { diff, .. }) => diff.cut,
        Some(Details::WriteFile { diff, content, .. }) => {
            diff.as_ref().is_some_and(|d| d.cut) || content.as_ref().is_some_and(|c| c.cut)
        }
        _ => false,
    };
    let result_cut = call
        .result
        .as_ref()
        .is_some_and(|r| r.text.len() >= crate::tools::FULL_BYTES);
    match (cut, result_cut) {
        (true, _) => Some("the diff was cut at 1 MiB".to_owned()),
        (_, true) => Some("the output was cut at 1 MiB, as the model got it".to_owned()),
        _ => None,
    }
}
