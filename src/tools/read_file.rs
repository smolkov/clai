//! `read_file` (see `spec/tools.md`, "read_file").

use std::fs;
use std::io::{self, BufRead, BufReader};

use serde::Deserialize;

use crate::config::Tool;
use crate::permissions::{Resolved, path_proposals};

use super::output::{ToolOutput, group};
use super::question::{Details, PathDetails};
use super::text::{file_is_binary, similar_names};
use super::{Call, Env, Failure, Op as AnyOp, in_range};

const MAX_LINES: u64 = 2000;
const MAX_LINE_CHARS: usize = 2000;
/// Files up to this size get their lines counted for the question.
const COUNT_LIMIT: u64 = 10 << 20;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Args {
    path: String,
    offset: Option<u64>,
    limit: Option<u64>,
}

#[derive(Debug)]
pub struct Op {
    pub path: Resolved,
    offset: u64,
    limit: u64,
}

pub(super) fn prepare(env: &Env, args: Result<Args, Failure>) -> Result<Call, Failure> {
    let args = args?;
    let offset = in_range("offset", args.offset, 1, u64::MAX, 1)?;
    let limit = in_range("limit", args.limit, 1, MAX_LINES, MAX_LINES)?;
    let path = env.workspace.resolve(&args.path);
    let decision = env.checker().check_path(Tool::ReadFile, &path);
    if decision.action == crate::config::Action::Deny {
        return Err(Failure::Denied(decision.denial()));
    }
    let shown = path.display();
    let meta = fs::metadata(&path.abs).map_err(|e| file_error(&path, e))?;
    if meta.is_dir() {
        return Err(format!("{shown} is a directory; use list_files").into());
    }
    if file_is_binary(&path.abs).map_err(|e| file_error(&path, e))? {
        return Err(format!("{shown} is a binary file").into());
    }
    let lines = (meta.len() <= COUNT_LIMIT)
        .then(|| count_lines(&path))
        .flatten();
    let mut last = offset.saturating_add(limit - 1);
    if let Some(n) = lines {
        last = last.min(n as u64).max(offset);
    }
    Ok(Call {
        summary: format!("{shown} (lines {offset}–{last})"),
        details: Details::ReadFile {
            path: PathDetails::new(&args.path, &path),
            first_line: offset,
            last_line: last,
            size: meta.len(),
            lines,
        },
        proposals: path_proposals(Tool::ReadFile, &path),
        decision,
        denial: None,
        op: AnyOp::Read(Op {
            path,
            offset,
            limit,
        }),
    })
}

/// "Not found" with similar names, or the system's message.
pub(super) fn file_error(path: &Resolved, e: io::Error) -> Failure {
    let shown = path.display();
    if e.kind() == io::ErrorKind::NotFound {
        let similar = similar_names(&path.abs);
        if similar.is_empty() {
            return Failure::Error(format!("file not found: {shown}"));
        }
        return Failure::Error(format!(
            "file not found: {shown}; similar names: {}",
            similar.join(", ")
        ));
    }
    Failure::Error(format!("{shown}: {e}"))
}

fn count_lines(path: &Resolved) -> Option<usize> {
    let bytes = fs::read(&path.abs).ok()?;
    let newlines = bytes.iter().filter(|&&b| b == b'\n').count();
    let unterminated = usize::from(bytes.last().is_some_and(|&b| b != b'\n'));
    Some(newlines + unterminated)
}

impl Op {
    pub fn run(self) -> ToolOutput {
        match self.read() {
            Ok(text) => ToolOutput::ok(text),
            Err(e) => ToolOutput::error(format!("{}: {e}", self.path.display())),
        }
    }

    fn read(&self) -> io::Result<String> {
        let mut reader = BufReader::new(fs::File::open(&self.path.abs)?);
        let mut out = String::new();
        let mut line = Vec::new();
        let mut number: u64 = 0;
        let last = self.offset.saturating_add(self.limit - 1);
        loop {
            line.clear();
            if reader.read_until(b'\n', &mut line)? == 0 {
                break;
            }
            number += 1;
            if number < self.offset || number > last {
                continue;
            }
            let text = String::from_utf8_lossy(&line);
            let text = text.trim_end_matches('\n').trim_end_matches('\r');
            out.push_str(&format!("{number:>6}→"));
            if text.chars().count() > MAX_LINE_CHARS {
                out.extend(text.chars().take(MAX_LINE_CHARS));
                out.push_str("… (line cut)");
            } else {
                out.push_str(text);
            }
            out.push('\n');
        }
        if number == 0 {
            return Ok("(empty file)".into());
        }
        if self.offset > number {
            return Ok(format!("(file has {} lines)", group(number as usize)));
        }
        if number > last {
            out.push_str(&format!(
                "… {} more lines; use offset {} to read on",
                group((number - last) as usize),
                last + 1
            ));
        } else if out.ends_with('\n') {
            out.pop();
        }
        Ok(out)
    }
}
