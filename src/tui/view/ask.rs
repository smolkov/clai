//! The box of a question from the model (see `spec/tui.md`, "Questions
//! from the model"): context, the question, its options and the text row;
//! after several questions, the review page.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::agent::{AskUser, QuestionKind};
use crate::tui::app::App;
use crate::tui::ask::Ask;
use crate::tui::markdown;
use crate::tui::prompt::Prompt;
use crate::tui::style::Look;
use crate::tui::wrap::{Piece, plain, wrap};

use super::popup;

/// The box's width, if the screen has room for it.
const WIDTH: u16 = 78;
/// Before an option's description.
const DESCRIPTION: &str = "      ";

fn cells(size: Rect) -> u16 {
    WIDTH.min(size.width.saturating_sub(4))
}

/// The open question from the model, if there is one.
fn open(app: &App) -> Option<(&Prompt, &AskUser, &Ask)> {
    let prompt = app.prompt.as_ref()?;
    let QuestionKind::AskUser(a) = &prompt.question.kind else {
        return None;
    };
    Some((prompt, a, prompt.ask.as_ref()?))
}

fn title(a: &AskUser, ask: &Ask, look: &Look) -> String {
    let rule = look.symbols.rule;
    if ask.reviewing(a) {
        return format!(" Question {rule} review ");
    }
    let header = a.questions.get(ask.page).map_or("", |q| q.header.as_str());
    if a.questions.len() > 1 {
        format!(
            " Question {rule} {}/{} {header} ",
            ask.page + 1,
            a.questions.len()
        )
    } else {
        format!(" Question {rule} {header} ")
    }
}

/// The rows of the box and, while the text row is typed, the cursor's row
/// and cell in them.
fn lines(prompt: &Prompt, a: &AskUser, ask: &Ask, width: usize, look: &Look) -> Body {
    let styles = &look.styles;
    let mut body = Body::default();
    if let Some(error) = &prompt.error {
        body.lines.extend(plain(
            &format!("not accepted: {error}"),
            width,
            styles.error,
        ));
    }
    for text in [&a.text, &a.context].into_iter().flatten() {
        body.lines.extend(markdown::lines(text, width, look));
        body.lines.push(Line::raw(""));
    }
    if ask.reviewing(a) {
        review(&mut body, a, ask, width, look);
    } else {
        question(&mut body, a, ask, width, look);
    }
    body
}

#[derive(Default)]
struct Body {
    lines: Vec<Line<'static>>,
    /// The row of the selected option, to keep it in view.
    selected: usize,
    cursor: Option<(usize, usize)>,
}

fn question(body: &mut Body, a: &AskUser, ask: &Ask, width: usize, look: &Look) {
    let styles = &look.styles;
    let symbols = &look.symbols;
    let Some(q) = a.questions.get(ask.page) else {
        return;
    };
    let Some(draft) = ask.drafts.get(ask.page) else {
        return;
    };
    body.lines.extend(plain(&q.question, width, styles.heading));
    body.lines.push(Line::raw(""));
    let none: Piece = (String::new(), styles.text);
    for (i, option) in q.options.iter().enumerate() {
        let here = ask.cursor == i;
        let chosen = draft.selected.contains(&i);
        let pointer = if here { symbols.pointer } else { " " };
        let mark = match (q.multiple, chosen) {
            (true, true) => "[x] ",
            (true, false) => "[ ] ",
            (false, _) => "",
        };
        let style = if chosen { styles.accent } else { styles.text };
        let lead = format!("{pointer} {} {mark}", i + 1);
        let rest = " ".repeat(lead.width());
        if here {
            body.selected = body.lines.len();
        }
        body.lines.extend(wrap(
            &[(option.label.clone(), style)],
            width,
            &(lead, if here { styles.accent } else { styles.text }),
            &(rest, styles.text),
        ));
        if let Some(d) = &option.description {
            let indent: Piece = (DESCRIPTION.to_owned(), styles.dim);
            body.lines
                .extend(wrap(&[(d.clone(), styles.dim)], width, &indent, &indent));
        }
    }
    // The text row.
    let row = q.options.len();
    let here = ask.cursor == row;
    if here {
        body.selected = body.lines.len();
    }
    let pointer = if here { symbols.pointer } else { " " };
    let lead = format!("{pointer} {} ", row + 1);
    let lead_style = if here { styles.accent } else { styles.text };
    match &ask.editing {
        Some(editor) if here => {
            let prefix = format!("{lead}> ");
            let room = width.saturating_sub(prefix.width()).max(1);
            let (rows, (r, cell)) = editor.rows(room);
            let shown = rows.get(r).cloned().unwrap_or_default();
            body.cursor = Some((body.lines.len(), prefix.width() + cell));
            body.lines.push(Line::from(vec![
                Span::styled(prefix, lead_style),
                Span::styled(shown, styles.text),
            ]));
        }
        _ if !draft.text.trim().is_empty() => {
            let added = if draft.selected.is_empty() {
                "Answer: "
            } else {
                "Added: "
            };
            body.lines.extend(wrap(
                &[
                    (added.to_owned(), styles.dim),
                    (draft.text.trim().to_owned(), styles.accent),
                ],
                width,
                &(lead.clone(), lead_style),
                &(" ".repeat(lead.width()), styles.text),
            ));
        }
        _ => body.lines.extend(wrap(
            &[("Type an answer".to_owned(), styles.dim)],
            width,
            &(lead, lead_style),
            &none,
        )),
    }
    body.lines.push(Line::raw(""));
    let rule = symbols.rule;
    let keys = if ask.editing.is_some() {
        format!("Enter keep {rule} Esc back {rule} Ctrl-C cancel turn")
    } else {
        let pick = if q.multiple {
            "Space tick"
        } else {
            "Up Down choose"
        };
        let tab = if a.questions.len() > 1 {
            format!(" {rule} Tab question")
        } else {
            String::new()
        };
        let enter = if a.questions.len() == 1 && !q.multiple {
            "Enter send"
        } else {
            "Enter next"
        };
        format!("{pick} {rule} {enter}{tab} {rule} Esc skip {rule} Ctrl-C cancel turn")
    };
    keys_line(body, ask, keys, width, look);
}

fn review(body: &mut Body, a: &AskUser, ask: &Ask, width: usize, look: &Look) {
    let styles = &look.styles;
    body.lines
        .extend(plain("Your answers:", width, styles.heading));
    body.lines.push(Line::raw(""));
    let header_width = a
        .questions
        .iter()
        .map(|q| q.header.width())
        .max()
        .unwrap_or(0);
    for (i, (q, draft)) in a.questions.iter().zip(&ask.drafts).enumerate() {
        let lead = format!("{} {:<header_width$}  ", i + 1, q.header);
        let rest = " ".repeat(lead.width());
        let mut pieces: Vec<Piece> = Vec::new();
        let labels: Vec<&str> = draft
            .selected
            .iter()
            .filter_map(|&k| q.options.get(k))
            .map(|o| o.label.as_str())
            .collect();
        if !labels.is_empty() {
            pieces.push((labels.join(", "), styles.accent));
        }
        let text = draft.text.trim();
        if !text.is_empty() {
            let sep = if labels.is_empty() { "" } else { "; " };
            pieces.push((format!("{sep}\"{text}\""), styles.text));
        }
        if pieces.is_empty() {
            pieces.push(("no answer yet".to_owned(), styles.warning));
        }
        body.lines.extend(wrap(
            &pieces,
            width,
            &(lead, styles.dim),
            &(rest, styles.dim),
        ));
    }
    body.lines.push(Line::raw(""));
    let rule = look.symbols.rule;
    keys_line(
        body,
        ask,
        format!("Enter send {rule} Shift-Tab question {rule} Esc skip {rule} Ctrl-C cancel turn"),
        width,
        look,
    );
}

/// The keys, or the question before declining.
fn keys_line(body: &mut Body, ask: &Ask, keys: String, width: usize, look: &Look) {
    if ask.confirm_decline {
        body.lines.extend(plain(
            "Skip the questions and drop what you chose? [y] yes  [n] no",
            width,
            look.styles.warning,
        ));
    } else {
        body.lines.extend(plain(&keys, width, look.styles.dim));
    }
}

pub fn draw(app: &App, frame: &mut Frame<'_>, size: Rect) {
    let Some((prompt, a, ask)) = open(app) else {
        return;
    };
    let look = &app.settings.look;
    let cells = cells(size);
    let width = usize::from(cells);
    let body = lines(prompt, a, ask, width, look);
    let wanted = u16::try_from(body.lines.len()).unwrap_or(u16::MAX);
    let area = popup::open(frame, size, &title(a, ask, look), cells, wanted, look);
    let room = usize::from(area.height).max(1);
    let max_top = body.lines.len().saturating_sub(room);
    // Scrolled by `PgUp` / `PgDn`; otherwise the selected row is in view.
    let top = if ask.top > 0 {
        ask.top.min(max_top)
    } else {
        (body.selected + 1).saturating_sub(room).min(max_top)
    };
    let shown: Vec<Line<'static>> = body.lines.into_iter().skip(top).take(room).collect();
    frame.render_widget(Paragraph::new(shown).style(Style::default()), area);
    if let Some((row, cell)) = body.cursor
        && row >= top
        && row < top + room
    {
        let x = area.x + u16::try_from(cell).unwrap_or(0);
        let y = area.y + u16::try_from(row - top).unwrap_or(0);
        frame.set_cursor_position((x, y));
    }
}
