//! The prompt bar: what is asked and which keys answer.

use ratatui::layout::Rect;
use ratatui::text::{Line, Span};

use crate::agent::{Choice, QuestionKind};
use crate::tools::Details;
use crate::tui::prompt::{Mode, Prompt};
use crate::tui::style::Look;
use crate::tui::wrap::{Piece, plain, wrap};

const EXPLAIN: &str = "Message to the model: ";

/// The rows of the bar in `width` cells.
pub fn lines(prompt: &Prompt, width: usize, look: &Look) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    if let Some(error) = &prompt.error {
        out.extend(plain(
            &format!("not accepted: {error}"),
            width,
            look.styles.error,
        ));
    }
    match &prompt.mode {
        Mode::Choices => choices(&mut out, prompt, width, look),
        Mode::Always { proposal } => always(&mut out, prompt, *proposal, width, look),
        Mode::Explain(editor) => {
            // A long message scrolls in its one row, with the cursor.
            let (rows, (row, _)) = editor.rows(width.saturating_sub(EXPLAIN.len()));
            let shown = rows.get(row).cloned().unwrap_or_default();
            out.push(Line::from(vec![
                Span::styled(EXPLAIN, look.styles.accent),
                Span::styled(shown, look.styles.text),
            ]));
        }
    }
    out
}

/// The cursor while a message to the model is typed.
pub fn cursor(prompt: &Prompt, area: Rect) -> Option<(u16, u16)> {
    let Mode::Explain(editor) = &prompt.mode else {
        return None;
    };
    let width = usize::from(area.width);
    let (_, (_, cell)) = editor.rows(width.saturating_sub(EXPLAIN.len()));
    let error_rows = prompt.error.as_ref().map_or(0, |e| {
        plain(&format!("not accepted: {e}"), width, Default::default()).len()
    });
    Some((
        area.x + u16::try_from(EXPLAIN.len() + cell).ok()?,
        area.y + u16::try_from(error_rows).ok()?,
    ))
}

fn keys_piece(text: String, look: &Look) -> Piece {
    (text, look.styles.accent)
}

fn choices(out: &mut Vec<Line<'static>>, prompt: &Prompt, width: usize, look: &Look) {
    let question = &prompt.question;
    let none: Piece = (String::new(), look.styles.text);
    let mut head: Vec<Piece> = Vec::new();
    let mut keys = Vec::new();
    match &question.kind {
        QuestionKind::Permission(p) => {
            if let Details::LspStart { command, .. } = &p.details {
                // `Start language server rust-analyzer in ~/ws/clai?`
                let mut summary = question.summary.clone();
                if let Some(first) = summary.get(..1) {
                    summary = format!(
                        "{}{}?",
                        first.to_uppercase(),
                        summary.get(1..).unwrap_or("")
                    );
                }
                out.extend(wrap(&[(summary, look.styles.text)], width, &none, &none));
                head.push((format!("command: {}  ", command.join(" ")), look.styles.dim));
                head.push((
                    "It may run build scripts and macros of the project.".to_owned(),
                    look.styles.warning,
                ));
            } else {
                head.push((format!("{}  ", question.summary), look.styles.text));
                head.push((format!("asks: {}", p.reason_text), look.styles.dim));
            }
            for choice in &question.choices {
                let label = match choice {
                    Choice::Allow => "[y] yes",
                    Choice::AllowAlways if prompt.can_always() => "[a] always",
                    Choice::Deny => "[n] no",
                    Choice::DenyMessage => "[e] explain",
                    Choice::Cancel => "[Esc] cancel",
                    _ => continue,
                };
                keys.push(label);
            }
        }
        QuestionKind::Limit(limit) => {
            head.push((
                format!(
                    "Turn limit reached ({} requests, {} tool calls). Continue?",
                    limit.requests, limit.tool_calls
                ),
                look.styles.text,
            ));
            keys.extend(["[y] yes", "[n] stop"]);
        }
        // Drawn as a box (`view::ask`); the bar is not shown.
        QuestionKind::AskUser(_) => {
            head.push((question.summary.clone(), look.styles.text));
        }
    }
    out.extend(wrap(&head, width, &none, &none));
    let keys = keys_piece(keys.join("  "), look);
    out.extend(wrap(&[keys], width, &none, &none));
}

fn always(
    out: &mut Vec<Line<'static>>,
    prompt: &Prompt,
    proposal: usize,
    width: usize,
    look: &Look,
) {
    let none: Piece = (String::new(), look.styles.text);
    let proposals = prompt.proposals();
    let rules = proposals
        .get(proposal)
        .map(|rules| rules.join(", "))
        .unwrap_or_default();
    let head = [
        ("Always allow: ".to_owned(), look.styles.dim),
        (rules, look.styles.text),
    ];
    out.extend(wrap(&head, width, &none, &none));
    let keys = if proposals.len() > 1 {
        format!(
            "({}/{})  [Tab] wider  [Enter] confirm  [Esc] back",
            proposal + 1,
            proposals.len()
        )
    } else {
        "[Enter] confirm  [Esc] back".to_owned()
    };
    out.extend(wrap(&[keys_piece(keys, look)], width, &none, &none));
}
