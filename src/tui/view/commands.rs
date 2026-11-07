//! The command list, over the last rows of the conversation (see `spec/tui.md`,
//! "Command list").

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph};

use crate::tui::app::App;
use crate::tui::commands::{COMMANDS, Command};
use crate::tui::wrap::fit;

/// The most rows it shows; the selected row stays among them.
const MAX_ROWS: usize = 8;
const LATER: &str = "later version";

fn row(
    app: &App,
    command: &Command,
    selected: bool,
    shown_width: usize,
    width: usize,
) -> Line<'static> {
    let look = &app.settings.look;
    let pointer = if selected { look.symbols.pointer } else { " " };
    let shown = command.shown();
    let description = if command.built {
        command.description
    } else {
        LATER
    };
    let text = format!("{pointer} {shown:<shown_width$}  {description}");
    let style = if selected {
        look.styles.selection
    } else if command.built {
        look.styles.text
    } else {
        look.styles.dim
    };
    Line::styled(fit(&text, width, look.symbols.more), style)
}

/// Draws the command list, or the argument list, with its last row above
/// `bottom` and a line over it. The layout does not change, so the
/// conversation does not jump.
pub fn draw(app: &App, frame: &mut Frame<'_>, conversation: Rect, bottom: u16) {
    if let Some(selected) = app.argument_list {
        arguments(app, frame, conversation, bottom, selected);
    }
    let Some(selected) = app.command_list else {
        return;
    };
    let look = &app.settings.look;
    let width = usize::from(conversation.width);
    let listed = app.listed();
    // The column stays put while the search changes.
    let shown_width = COMMANDS
        .iter()
        .map(|c| c.shown().chars().count())
        .max()
        .unwrap_or(0);
    let room = usize::from(conversation.height).saturating_sub(1);
    let rows = listed.len().clamp(1, MAX_ROWS).min(room);
    let first = (selected + 1).saturating_sub(rows);
    let mut lines = vec![Line::styled(
        look.symbols.rule.repeat(width),
        look.styles.border,
    )];
    if listed.is_empty() {
        lines.push(Line::styled("  no command", look.styles.dim));
    }
    lines.extend(
        listed
            .iter()
            .enumerate()
            .skip(first)
            .take(rows)
            .map(|(i, c)| row(app, c, i == selected, shown_width, width)),
    );
    place(frame, lines, conversation, bottom);
}

/// The values of `/attach ` or `/model `: the value, then its hint.
fn arguments(app: &App, frame: &mut Frame<'_>, conversation: Rect, bottom: u16, selected: usize) {
    let look = &app.settings.look;
    let width = usize::from(conversation.width);
    let values = app.arguments();
    let value_width = values
        .iter()
        .map(|v| v.value.chars().count())
        .max()
        .unwrap_or(0);
    let room = usize::from(conversation.height).saturating_sub(1);
    let rows = values.len().clamp(1, MAX_ROWS).min(room);
    let first = (selected + 1).saturating_sub(rows);
    let mut lines = vec![Line::styled(
        look.symbols.rule.repeat(width),
        look.styles.border,
    )];
    if values.is_empty() {
        lines.push(Line::styled("  no match", look.styles.dim));
    }
    for (i, value) in values.iter().enumerate().skip(first).take(rows) {
        let pointer = if i == selected {
            look.symbols.pointer
        } else {
            " "
        };
        let text = format!(
            "{pointer} {:<value_width$}  {}",
            value.value, value.description
        );
        let style = if i == selected {
            look.styles.selection
        } else {
            look.styles.text
        };
        lines.push(Line::styled(fit(&text, width, look.symbols.more), style));
    }
    place(frame, lines, conversation, bottom);
}

/// Puts the lines over the conversation, ending above `bottom`.
fn place(frame: &mut Frame<'_>, lines: Vec<Line<'static>>, conversation: Rect, bottom: u16) {
    let height = u16::try_from(lines.len()).unwrap_or(0);
    let area = Rect::new(
        conversation.x,
        bottom.saturating_sub(height),
        conversation.width,
        height,
    );
    frame.render_widget(Clear, area);
    frame.render_widget(Paragraph::new(lines), area);
}
