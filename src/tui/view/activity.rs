//! The activity line above the input while the session works (see
//! `spec/tui.md`, "Activity line"):
//! `✻ Prestidigitating… (14s · ↓ 1.2k tokens · Esc cancel)`.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

use crate::config::ActivityLine;
use crate::text::main_argument;
use crate::text::status::duration;
use crate::tui::activity::Phase;
use crate::tui::app::App;
use crate::tui::conversation::ToolCall;
use crate::tui::keys::Focus;
use crate::tui::style::{Colouring, Look};
use crate::tui::wrap::fit;

/// Characters of the brighter band of the shimmer.
const BAND: usize = 3;
/// Frames for one pass of the band, about two seconds.
const SWEEP: usize = 17;

/// What the line says, before it is cut to a width.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parts {
    /// The glyph of this frame.
    pub glyph: &'static str,
    /// No event for a while: the glyph warns.
    pub stalled: bool,
    /// `Prestidigitating…`, `Running cargo test…`.
    pub text: String,
    /// The text is the turn's verb, which shimmers.
    pub verb: bool,
    /// In brackets, dropped from the right when there is no room.
    pub details: Vec<String>,
}

pub fn parts(app: &App) -> Parts {
    let look = &app.settings.look;
    let symbols = &look.symbols;
    let activity = &app.activity;
    let frames = symbols.activity;
    let glyph = frames
        .get(activity.frame % frames.len().max(1))
        .copied()
        .unwrap_or_default();
    let more = symbols.more;
    let playful = app.settings.tui.activity == ActivityLine::Playful;
    let calls: Vec<&ToolCall> = app.conversation.running_calls().collect();
    let (text, verb) = if activity.cancelling {
        (format!("Cancelling{more}"), false)
    } else if let Some(first) = calls.first() {
        let others = calls.len() - 1;
        let text = if others > 0 {
            format!("{}{more} +{others}", tool_text(first))
        } else {
            format!("{}{more}", tool_text(first))
        };
        (text, false)
    } else {
        match &activity.phase {
            Phase::Retry {
                attempt,
                max,
                reason,
            } => (format!("Retrying ({attempt}/{max}): {reason}{more}"), false),
            _ if playful => (format!("{}{more}", activity.verb), true),
            Phase::Writing => (format!("Writing{more}"), false),
            Phase::Thinking => (format!("Thinking{more}"), false),
        }
    };
    let mut details = Vec::new();
    if let Some(elapsed) = activity.elapsed(app.now) {
        details.push(duration(
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
        ));
    }
    let tokens = activity.tokens();
    if tokens > 0 {
        details.push(format!(
            "{} {} tokens",
            symbols.new_output,
            crate::text::tokens(tokens)
        ));
    }
    if app.focus == Focus::Input && app.editor.is_empty() && !activity.cancelling {
        details.push("Esc cancel".to_owned());
    }
    Parts {
        glyph,
        stalled: calls.is_empty() && activity.stalled(app.now),
        text,
        verb,
        details,
    }
}

/// `Running cargo test`, `Reading src/parse.rs`: the tool and its main
/// argument, as on the call's summary line.
fn tool_text(call: &ToolCall) -> String {
    let argument = call
        .summary
        .clone()
        .unwrap_or_else(|| main_argument(&call.arguments));
    let argument = argument
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned();
    let verb = match call.name.as_str() {
        "run_command" => "Running",
        "read_file" => "Reading",
        "edit_file" => "Editing",
        "write_file" => "Writing",
        "list_files" => "Listing",
        "search_files" => "Searching",
        "ask_user" => "Asking",
        _ => {
            return if argument.is_empty() {
                format!("Running {}", call.name)
            } else {
                format!("Running {} {argument}", call.name)
            };
        }
    };
    if argument.is_empty() {
        verb.to_owned()
    } else {
        format!("{verb} {argument}")
    }
}

/// The parts cut to `width` cells: the details go from the right first, then
/// the text is cut. The glyph always stays.
pub fn fitted(parts: &Parts, width: usize, look: &Look) -> (String, Option<String>) {
    let separator = format!(" {} ", look.symbols.separator);
    let head = parts.glyph.width() + 1;
    let mut details = parts.details.clone();
    loop {
        let bracket = (!details.is_empty()).then(|| format!(" ({})", details.join(&separator)));
        let used = head + parts.text.width() + bracket.as_ref().map_or(0, |b| b.width());
        if used <= width || details.is_empty() {
            let room = width.saturating_sub(head + bracket.as_ref().map_or(0, |b| b.width()));
            return (fit(&parts.text, room, look.symbols.more), bracket);
        }
        details.pop();
    }
}

pub fn line(app: &App, width: usize) -> Line<'static> {
    let look = &app.settings.look;
    let styles = &look.styles;
    let parts = parts(app);
    let (text, bracket) = fitted(&parts, width, look);
    let glyph_style = if parts.stalled {
        styles.warning
    } else {
        styles.accent
    };
    let mut spans = vec![Span::styled(format!("{} ", parts.glyph), glyph_style)];
    let colour = app.settings.theming.colouring != Colouring::None;
    let playful = app.settings.tui.activity == ActivityLine::Playful;
    if parts.verb && colour && playful {
        spans.extend(shimmer(&text, app.activity.frame, styles.dim, styles.text));
    } else if parts.verb && colour {
        spans.push(Span::styled(text, styles.dim));
    } else {
        spans.push(Span::styled(text, styles.text));
    }
    if let Some(bracket) = bracket {
        spans.push(Span::styled(bracket, styles.dim));
    }
    Line::from(spans)
}

/// The text in `base` with a band of [`BAND`] characters in `bright` that
/// moves from left to right, once per [`SWEEP`] frames.
fn shimmer(text: &str, frame: usize, base: Style, bright: Style) -> Vec<Span<'static>> {
    let count = text.chars().count();
    // The band enters from the left and leaves at the right.
    let end = (frame % SWEEP) * (count + BAND) / SWEEP;
    let start = end.saturating_sub(BAND);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut run_bright = false;
    for (i, c) in text.chars().enumerate() {
        let lit = i >= start && i < end;
        if lit != run_bright && !run.is_empty() {
            let style = if run_bright { bright } else { base };
            spans.push(Span::styled(std::mem::take(&mut run), style));
        }
        run_bright = lit;
        run.push(c);
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, if run_bright { bright } else { base }));
    }
    spans
}

pub fn draw(app: &App, buffer: &mut Buffer, area: Rect) {
    let line = line(app, usize::from(area.width));
    buffer.set_line(area.x, area.y, &line, area.width);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_band_moves_through_the_text() {
        let base = Style::new();
        let bright = Style::new().add_modifier(ratatui::style::Modifier::BOLD);
        let lit = |frame| -> String {
            shimmer("Pondering…", frame, base, bright)
                .into_iter()
                .filter(|s| s.style == bright)
                .map(|s| s.content.into_owned())
                .collect()
        };
        assert_eq!(lit(0), "");
        assert_eq!(lit(5), "Pon");
        assert_eq!(lit(9), "der");
        assert_eq!(lit(SWEEP - 1), "…");
        assert_eq!(lit(SWEEP), "");
        let all: String = shimmer("Pondering…", 7, base, bright)
            .into_iter()
            .map(|s| s.content.into_owned())
            .collect();
        assert_eq!(all, "Pondering…");
    }
}
