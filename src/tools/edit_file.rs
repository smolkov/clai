//! `edit_file` (see `spec/tools.md`, "edit_file").

use std::fs;
use std::time::UNIX_EPOCH;

use serde::Deserialize;
use similar::TextDiff;

use crate::config::{Action, Tool};
use crate::permissions::{Resolved, path_proposals};

use super::output::{ToolOutput, plural};
use super::question::{Details, PathDetails, Text};
use super::read_file::file_error;
use super::text::{is_binary, line_at, write_atomic};
use super::{Call, Env, Failure, Op as AnyOp};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Args {
    path: String,
    old_text: String,
    new_text: String,
    replace_all: Option<bool>,
}

#[derive(Debug)]
pub struct Op {
    pub path: Resolved,
    /// The file as it was when the call was prepared.
    original: Vec<u8>,
    new_content: String,
    result: String,
}

pub(super) fn prepare(env: &Env, args: Result<Args, Failure>) -> Result<Call, Failure> {
    let args = args?;
    if args.old_text.is_empty() {
        return Err("old_text is empty; to create a file, use write_file"
            .to_owned()
            .into());
    }
    if args.old_text == args.new_text {
        return Err("new_text is the same as old_text; nothing would change"
            .to_owned()
            .into());
    }
    let path = env.workspace.resolve(&args.path);
    let decision = env.checker().check_path(Tool::EditFile, &path);
    if decision.action == Action::Deny {
        return Err(Failure::Denied(decision.denial()));
    }
    let shown = path.display();
    let meta = fs::metadata(&path.abs).map_err(|e| file_error(&path, e))?;
    if meta.is_dir() {
        return Err(format!("{shown} is a directory").into());
    }
    let original = fs::read(&path.abs).map_err(|e| file_error(&path, e))?;
    if is_binary(&original) {
        return Err(format!("{shown} is a binary file").into());
    }
    let Ok(content) = String::from_utf8(original.clone()) else {
        return Err(format!(
            "{shown} is not valid UTF-8; edit_file cannot change it without changing other bytes"
        )
        .into());
    };
    let crlf = content.contains("\r\n");
    let (old, new) = if crlf {
        (to_crlf(&args.old_text), to_crlf(&args.new_text))
    } else {
        (args.old_text.clone(), args.new_text.clone())
    };
    let starts: Vec<usize> = content.match_indices(&old).map(|(i, _)| i).collect();
    let lines: Vec<usize> = starts.iter().map(|&i| line_at(&content, i)).collect();
    if starts.is_empty() {
        return Err(not_found(&shown, &content, &old).into());
    }
    let replace_all = args.replace_all.unwrap_or(false);
    if starts.len() > 1 && !replace_all {
        return Err(format!(
            "old_text occurs {} times (lines {}); add surrounding text to make it unique, or set replace_all",
            starts.len(),
            join(&lines)
        )
        .into());
    }
    let new_content = content.replace(&old, &new);
    // Changed lines in the new file: from the first replacement to the end
    // of the last one.
    let shift = new.len() as isize - old.len() as isize;
    let last_start =
        starts.last().copied().unwrap_or(0) as isize + shift * (starts.len() as isize - 1);
    let last_end = (last_start.max(0) as usize + new.len()).min(new_content.len());
    let first_line = lines.first().copied().unwrap_or(1);
    let last_line = line_at(
        &new_content,
        last_end.saturating_sub(usize::from(new.ends_with('\n'))),
    )
    .max(first_line);
    let diff = TextDiff::from_lines(content.as_str(), new_content.as_str())
        .unified_diff()
        .context_radius(3)
        .header(&shown, &shown)
        .to_string();
    let result = if starts.len() == 1 {
        format!("edited {shown}: replaced 1 occurrence (line {first_line})")
    } else {
        format!(
            "edited {shown}: replaced {} (lines {})",
            plural(starts.len(), "occurrence", "occurrences"),
            join(&lines)
        )
    };
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs());
    Ok(Call {
        summary: line_range(&shown, first_line, last_line),
        details: Details::EditFile {
            path: PathDetails::new(&args.path, &path),
            diff: Text::new(diff),
            first_line,
            last_line,
            occurrences: starts.len(),
            size: meta.len(),
            modified,
        },
        proposals: path_proposals(Tool::EditFile, &path),
        decision,
        denial: None,
        op: AnyOp::Edit(Op {
            path,
            original,
            new_content,
            result,
        }),
    })
}

pub(super) fn line_range(shown: &str, first: usize, last: usize) -> String {
    if first == last {
        format!("{shown} (line {first})")
    } else {
        format!("{shown} (lines {first}–{last})")
    }
}

fn join(lines: &[usize]) -> String {
    lines
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// `\n` that is not part of `\r\n` becomes `\r\n`.
fn to_crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// The "not found" message, quoting a match that differs only in
/// whitespace, if there is one.
fn not_found(shown: &str, content: &str, old: &str) -> String {
    let base = format!("old_text not found in {shown}");
    let wanted = squash(old);
    if wanted.is_empty() {
        return base;
    }
    let lines: Vec<&str> = content.lines().collect();
    let n = old.lines().count().max(1);
    for start in 0..lines.len() {
        let end = (start + n).min(lines.len());
        let Some(window) = lines.get(start..end) else {
            break;
        };
        let candidate = window.join("\n");
        let hit = if n == 1 {
            squash(&candidate).contains(&wanted)
        } else {
            squash(&candidate) == wanted
        };
        if hit {
            return format!(
                "{base}; this text at line {} differs only in whitespace:\n{candidate}",
                start + 1
            );
        }
    }
    base
}

/// Runs of whitespace as one space, trimmed.
fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

impl Op {
    pub fn run(self) -> ToolOutput {
        let shown = self.path.display();
        match fs::read(&self.path.abs) {
            Ok(now) if now == self.original => {}
            _ => return ToolOutput::error("file changed while waiting for permission"),
        }
        let keep = fs::metadata(&self.path.abs).ok().map(|m| m.permissions());
        match write_atomic(&self.path.abs, self.new_content.as_bytes(), keep) {
            Ok(()) => ToolOutput::ok(self.result),
            Err(e) => ToolOutput::error(format!("{shown}: {e}")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crlf() {
        assert_eq!(to_crlf("a\nb\r\nc"), "a\r\nb\r\nc");
    }

    #[test]
    fn whitespace_hint() {
        let content = "fn a() {\n    let x  =  1;\n}\n";
        let msg = not_found("a.rs", content, "let x = 1;");
        assert!(
            msg.contains("at line 2 differs only in whitespace"),
            "{msg}"
        );
        let msg = not_found("a.rs", content, "fn a() {\nlet x = 1;");
        assert!(msg.contains("at line 1"), "{msg}");
        assert_eq!(
            not_found("a.rs", content, "nothing"),
            "old_text not found in a.rs"
        );
    }
}
