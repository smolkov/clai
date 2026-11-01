//! A question as text, in the layout of `spec/command.md`, "clai chat".

use std::fmt::Write as _;

use crate::agent::{Choice, Permission, Question, QuestionKind};
use crate::config::Action;
use crate::permissions::Decision;
use crate::tools::{Details, PathDetails, group};

use super::cut;

/// New file content shown before `… N more lines`.
const CONTENT_LINES: usize = 40;
/// Width of the label column.
const LABEL: usize = 9;

/// The question and its choices; `place` is the workspace as shown
/// (`~/ws/rust/clai`).
pub fn render_question(question: &Question, place: &str, unicode: bool) -> String {
    let rule = if unicode { "──" } else { "--" };
    let mut out = format!(
        "{rule} clai {} {place} {} {} {rule}\n",
        dot(unicode),
        dot(unicode),
        question.id
    );
    match &question.kind {
        QuestionKind::Permission(p) => permission(&mut out, question, p, unicode),
        QuestionKind::Limit(_) => {
            let _ = writeln!(out, "{}", question.summary);
        }
    }
    out.push_str(&choices(&question.choices));
    out.push('\n');
    out
}

fn dot(unicode: bool) -> &'static str {
    if unicode { "·" } else { "-" }
}

fn label(out: &mut String, name: &str, text: &str) {
    let _ = writeln!(out, "  {name:<LABEL$}{text}");
}

fn permission(out: &mut String, question: &Question, p: &Permission, unicode: bool) {
    let _ = writeln!(out, "{}", question.summary);
    match &p.details {
        Details::RunCommand {
            cwd,
            timeout_s,
            parts,
            redirects,
            ..
        } => {
            label(out, "in", &format!("{cwd}, timeout {timeout_s} s"));
            let width = parts
                .iter()
                .map(|p| p.text.chars().count())
                .max()
                .unwrap_or(0);
            for (i, part) in parts.iter().enumerate() {
                let text = format!("{:<width$}  {}", part.text, decision(&part.decision));
                label(out, &format!("part {}", i + 1), &text);
            }
            for r in redirects {
                let arrow = if r.write { ">" } else { "<" };
                let path = if r.path.is_empty() {
                    &r.target
                } else {
                    &r.path
                };
                label(
                    out,
                    if r.write { "writes" } else { "reads" },
                    &format!("{arrow} {path}  {}", decision(&r.decision)),
                );
            }
        }
        Details::ReadFile {
            path,
            first_line,
            last_line,
            lines,
            ..
        } => {
            let of = lines
                .map(|n| format!(" of {}", group(n)))
                .unwrap_or_default();
            label(
                out,
                "file",
                &format!("{}, lines {first_line}–{last_line}{of}", where_(path)),
            );
            label(out, "asks", &p.reason_text);
        }
        Details::ListFiles { path, pattern } => {
            label(out, "in", &format!("{}, pattern {pattern}", where_(path)));
            label(out, "asks", &p.reason_text);
        }
        Details::SearchFiles {
            path,
            pattern,
            ignore_case,
            glob,
        } => {
            let mut text = format!("{pattern} in {}", where_(path));
            if *ignore_case {
                text.push_str(", ignoring case");
            }
            if let Some(glob) = glob {
                let _ = write!(text, ", files {glob}");
            }
            label(out, "search", &text);
            label(out, "asks", &p.reason_text);
        }
        Details::EditFile {
            path,
            diff,
            occurrences,
            ..
        } => {
            let times = if *occurrences == 1 {
                String::new()
            } else {
                format!(", {occurrences} occurrences")
            };
            label(out, "file", &format!("{}{times}", where_(path)));
            label(out, "asks", &p.reason_text);
            block(out, &diff.text);
        }
        Details::WriteFile {
            path,
            exists,
            content,
            diff,
            lines,
            created_dirs,
        } => {
            let what = if *exists { "replace" } else { "new file" };
            label(
                out,
                what,
                &format!("{} ({} lines)", where_(path), group(*lines)),
            );
            if !created_dirs.is_empty() {
                label(out, "creates", &created_dirs.join(", "));
            }
            label(out, "asks", &p.reason_text);
            if let Some(diff) = diff {
                block(out, &diff.text);
            }
            if let Some(content) = content {
                let all: Vec<&str> = content.text.lines().collect();
                let shown = all.iter().take(CONTENT_LINES).copied().collect::<Vec<_>>();
                block(out, &shown.join("\n"));
                if all.len() > CONTENT_LINES {
                    let more = if unicode { "…" } else { "..." };
                    let _ = writeln!(
                        out,
                        "{more} {} more lines",
                        group(all.len() - CONTENT_LINES)
                    );
                }
            }
        }
    }
    if let Some(text) = &p.text {
        let first = text
            .lines()
            .find(|l| !l.trim().is_empty())
            .unwrap_or_default();
        label(out, "model", &format!("\"{}\"", cut(first, 200, unicode)));
    }
}

/// `src/a.rs`, with `(extra dir)` when it is not in the workspace.
fn where_(path: &PathDetails) -> String {
    match path.area {
        "workspace" => path.resolved.clone(),
        "extra_dir" => format!("{} (extra dir)", path.resolved),
        _ => format!("{} (outside)", path.resolved),
    }
}

/// `allow  allow run_command(cargo build)  config.toml:28`
fn decision(decision: &Decision) -> String {
    let action = match decision.action {
        Action::Allow => "allow",
        Action::Ask => "ask  ",
        Action::Deny => "deny ",
    };
    format!("{action}  {}", decision.reason)
}

fn block(out: &mut String, text: &str) {
    for line in text.lines() {
        let _ = writeln!(out, "    {line}");
    }
}

/// `[y] allow  [a] always  …`
pub(super) fn choices(choices: &[Choice]) -> String {
    choices
        .iter()
        .map(|c| match c {
            Choice::Allow => "[y] allow",
            Choice::AllowAlways => "[a] always",
            Choice::Deny => "[n] deny",
            Choice::DenyMessage => "[e] deny with message",
            Choice::Cancel => "[c] cancel turn",
            Choice::Continue => "[y] continue",
            Choice::Stop => "[n] stop",
        })
        .collect::<Vec<_>>()
        .join("  ")
}
