//! `write_file` (see `spec/tools.md`, "write_file").

use std::fs;
use std::path::PathBuf;

use serde::Deserialize;
use similar::TextDiff;

use crate::config::{Action, Tool};
use crate::permissions::{Resolved, path_proposals};

use super::output::{ToolOutput, plural};
use super::question::{Details, PathDetails, Text};
use super::text::{is_binary, write_atomic};
use super::{Call, Env, Failure, Op as AnyOp};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Args {
    path: String,
    content: String,
    overwrite: Option<bool>,
}

#[derive(Debug)]
pub struct Op {
    pub path: Resolved,
    content: String,
    /// The old content of a replaced file, to see whether it changed.
    original: Option<Vec<u8>>,
    old_lines: usize,
    dirs: Vec<PathBuf>,
}

pub(super) fn prepare(env: &Env, args: Result<Args, Failure>) -> Result<Call, Failure> {
    let args = args?;
    let path = env.workspace.resolve(&args.path);
    let decision = env.checker().check_path(Tool::WriteFile, &path);
    if decision.action == Action::Deny {
        return Err(Failure::Denied(decision.denial()));
    }
    let shown = path.display();
    let overwrite = args.overwrite.unwrap_or(false);
    let original = match fs::metadata(&path.abs) {
        Ok(meta) if meta.is_dir() => return Err(format!("{shown} is a directory").into()),
        Ok(_) if !overwrite => {
            return Err(
                format!("{shown} exists; use edit_file to change it, or set overwrite").into(),
            );
        }
        Ok(_) => Some(fs::read(&path.abs).map_err(|e| Failure::Error(format!("{shown}: {e}")))?),
        Err(_) => None,
    };
    // Directories to create, and a parent that is a file.
    let mut dirs = Vec::new();
    let mut parent = path.abs.parent();
    while let Some(dir) = parent {
        match fs::metadata(dir) {
            Ok(meta) if meta.is_dir() => break,
            Ok(_) => {
                let shown_dir = env.workspace.locate(dir).display();
                return Err(format!("{shown_dir} is a file, not a directory").into());
            }
            Err(_) => dirs.push(dir.to_path_buf()),
        }
        parent = dir.parent();
    }
    dirs.reverse();
    let lines = args.content.lines().count();
    let (content, diff, old_lines) = match &original {
        None => (Some(Text::new(args.content.clone())), None, 0),
        Some(old) => {
            let old_lines = String::from_utf8_lossy(old).lines().count();
            let diff = if is_binary(old) {
                format!("{shown} is a binary file; its content is replaced\n")
            } else {
                let old_text = String::from_utf8_lossy(old);
                TextDiff::from_lines(old_text.as_ref(), args.content.as_str())
                    .unified_diff()
                    .context_radius(3)
                    .header(&shown, &shown)
                    .to_string()
            };
            (None, Some(Text::new(diff)), old_lines)
        }
    };
    Ok(Call {
        summary: format!("{shown} ({})", plural(lines, "line", "lines")),
        details: Details::WriteFile {
            path: PathDetails::new(&args.path, &path),
            exists: original.is_some(),
            content,
            diff,
            lines,
            created_dirs: dirs
                .iter()
                .map(|d| env.workspace.locate(d).display())
                .collect(),
        },
        proposals: path_proposals(Tool::WriteFile, &path),
        decision,
        denial: None,
        op: AnyOp::Write(Op {
            path,
            content: args.content,
            original,
            old_lines,
            dirs,
        }),
    })
}

impl Op {
    pub fn run(self) -> ToolOutput {
        let shown = self.path.display();
        let now = fs::read(&self.path.abs).ok();
        if now != self.original {
            return ToolOutput::error("file changed while waiting for permission");
        }
        for dir in &self.dirs {
            if let Err(e) = fs::create_dir_all(dir) {
                return ToolOutput::error(format!("{}: {e}", dir.display()));
            }
        }
        let keep = self
            .original
            .as_ref()
            .and_then(|_| fs::metadata(&self.path.abs).ok())
            .map(|m| m.permissions());
        if let Err(e) = write_atomic(&self.path.abs, self.content.as_bytes(), keep) {
            return ToolOutput::error(format!("{shown}: {e}"));
        }
        let lines = plural(self.content.lines().count(), "line", "lines");
        match self.original {
            None => ToolOutput::ok(format!("created {shown} ({lines})")),
            Some(_) => ToolOutput::ok(format!(
                "replaced {shown} ({lines}, was {})",
                self.old_lines
            )),
        }
    }
}
