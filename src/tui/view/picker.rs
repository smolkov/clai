//! The box of the model picker (see `spec/tui.md`, "Model picker").

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::protocol::ModelInfo;
use crate::text::short_tokens;
use crate::tui::app::App;
use crate::tui::picker::{Picker, Purpose};
use crate::tui::style::Look;
use crate::tui::wrap::fit;

use super::popup;

/// The box's width, if the screen has room for it.
const WIDTH: u16 = 76;
/// The rows of the box that are not models: filter, two blank lines, keys.
const CHROME: usize = 4;
const FILTER: &str = "Filter: ";

const NO_MODELS: &str = r#"[providers.local]
api      = "openai-compatible"
base_url = "http://localhost:11434/v1"

[providers.local.models.qwen]
id             = "qwen3-coder:30b"
context_window = 32768"#;

/// The marks of a row: `✓ current, default`, `⚠ no key`, `⚠ history 41k > 32k`.
fn marks(app: &App, model: &ModelInfo) -> Vec<String> {
    let symbols = &app.settings.look.symbols;
    let change = app
        .picker
        .as_ref()
        .is_some_and(|p| p.purpose == Purpose::Change);
    let mut marks = Vec::new();
    if change && model.model == app.session.model {
        marks.push(format!("{} current", symbols.done));
    }
    if model.default {
        marks.push("default".to_owned());
    }
    if model.key == "missing" {
        marks.push(format!("{} no key", symbols.warning));
    }
    if change && app.context_tokens > model.context_window {
        marks.push(format!(
            "{} history {} > {}",
            symbols.warning,
            short_tokens(app.context_tokens),
            short_tokens(model.context_window)
        ));
    }
    marks
}

/// The list: a heading per provider and a row per model, and which line holds
/// the selected row.
fn list(app: &App, picker: &Picker, width: usize) -> (Vec<Line<'static>>, usize) {
    let look = &app.settings.look;
    let visible = picker.visible();
    let column = |cells: fn(&ModelInfo) -> usize| visible.iter().map(|m| cells(m)).max();
    let w_name = column(|m| m.name.width()).unwrap_or(0);
    let w_id = column(|m| m.id.width()).unwrap_or(0);
    let w_window = column(|m| short_tokens(m.context_window).len()).unwrap_or(0);
    let mut lines = Vec::new();
    let mut at = 0;
    let mut provider = None;
    for (i, model) in visible.iter().enumerate() {
        if provider != Some(&model.provider) {
            provider = Some(&model.provider);
            let heading = fit(&model.provider, width, look.symbols.more);
            lines.push(Line::styled(heading, look.styles.heading));
        }
        let selected = i == picker.selected();
        let pointer = if selected { look.symbols.pointer } else { " " };
        let marks = marks(app, model);
        let warned = marks.iter().any(|m| m.starts_with(look.symbols.warning));
        let window = short_tokens(model.context_window);
        let row = format!(
            "{pointer} {name:<w_name$}  {id:<w_id$}  {window:>w_window$}  {marks}",
            name = model.name,
            id = model.id,
            marks = marks.join(", ")
        );
        let style = if selected {
            at = lines.len();
            look.styles.selection
        } else if warned {
            look.styles.warning
        } else {
            look.styles.text
        };
        let row = fit(row.trim_end(), width, look.symbols.more);
        lines.push(Line::styled(row, style));
    }
    (lines, at)
}

fn keys(look: &Look) -> Line<'static> {
    let text = format!("Enter select {} Esc cancel", look.symbols.rule);
    Line::styled(text, look.styles.dim)
}

fn no_models(picker: &Picker, look: &Look) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::raw("No model is configured. Add one to"),
        Line::raw(picker.config.display().to_string()),
        Line::raw(""),
    ];
    lines.extend(NO_MODELS.lines().map(|l| Line::raw(l.to_owned())));
    lines.push(Line::raw(""));
    lines.push(Line::styled("Esc close", look.styles.dim));
    lines
}

pub fn draw(app: &App, frame: &mut Frame<'_>, size: Rect) {
    let Some(picker) = &app.picker else {
        return;
    };
    let look = &app.settings.look;
    let title = match &picker.purpose {
        Purpose::Change => format!(" Model for \"{}\" ", app.session.name),
        Purpose::New { .. } => " Model for a new session ".to_owned(),
    };
    let cells = WIDTH.min(size.width.saturating_sub(4));
    let width = usize::from(cells);
    if !picker.has_models() {
        let lines = no_models(picker, look);
        let rows = u16::try_from(lines.len()).unwrap_or(u16::MAX);
        let area = popup::open(frame, size, &title, cells, rows, look);
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }
    let (rows, selected) = list(app, picker, width);
    let wanted = u16::try_from(rows.len().max(1) + CHROME).unwrap_or(u16::MAX);
    let area = popup::open(frame, size, &title, cells, wanted, look);
    let room = usize::from(area.height).saturating_sub(CHROME).max(1);
    let first = (selected + 1).saturating_sub(room);

    let filter_width = width.saturating_sub(FILTER.len()).max(1);
    let (filter_rows, (cursor_row, cursor_cell)) = picker.filter.rows(filter_width);
    let shown = filter_rows.get(cursor_row).cloned().unwrap_or_default();
    let mut lines = vec![Line::styled(format!("{FILTER}{shown}"), look.styles.text)];
    lines.push(Line::raw(""));
    if rows.is_empty() {
        lines.push(Line::styled("no model", look.styles.dim));
    }
    lines.extend(rows.into_iter().skip(first).take(room));
    lines.resize(area.height.saturating_sub(1).into(), Line::raw(""));
    lines.push(keys(look));
    frame.render_widget(Paragraph::new(lines), area);
    if let Ok(cell) = u16::try_from(FILTER.len() + cursor_cell) {
        frame.set_cursor_position((area.x + cell, area.y));
    }
}
