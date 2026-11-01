//! `list_files` (see `spec/tools.md`, "list_files").

use std::fs;

use globset::{GlobBuilder, GlobMatcher};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::config::{Action, Tool};
use crate::permissions::{Resolved, path_proposals};

use super::output::{ToolOutput, group};
use super::question::{Details, PathDetails};
use super::walk::walk;
use super::{Call, Env, Failure, Op as AnyOp, cancelled};

const MAX_ENTRIES: usize = 1000;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Args {
    path: Option<String>,
    pattern: Option<String>,
}

#[derive(Debug)]
pub struct Op {
    pub path: Resolved,
    pattern: String,
    glob: GlobMatcher,
}

pub(super) fn prepare(env: &Env, args: Result<Args, Failure>) -> Result<Call, Failure> {
    let args = args?;
    let written = args.path.unwrap_or_default();
    let path = env.workspace.resolve(&written);
    let decision = env.checker().check_path(Tool::ListFiles, &path);
    if decision.action == Action::Deny {
        return Err(Failure::Denied(decision.denial()));
    }
    let pattern = args.pattern.unwrap_or_else(|| "*".into());
    let glob = path_glob(&pattern)?;
    directory(&path)?;
    Ok(Call {
        summary: format!("{} {pattern}", path.display()),
        details: Details::ListFiles {
            path: PathDetails::new(&written, &path),
            pattern: pattern.clone(),
        },
        proposals: path_proposals(Tool::ListFiles, &path),
        decision,
        denial: None,
        op: AnyOp::List(Op {
            path,
            pattern,
            glob,
        }),
    })
}

/// A glob over paths: `*` stays within one directory.
pub(super) fn path_glob(pattern: &str) -> Result<GlobMatcher, Failure> {
    GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()
        .map(|g| g.compile_matcher())
        .map_err(|e| Failure::Error(format!("invalid glob `{pattern}`: {}", e.kind())))
}

/// The path must be an existing directory.
fn directory(path: &Resolved) -> Result<(), Failure> {
    match fs::metadata(&path.abs) {
        Ok(meta) if meta.is_dir() => Ok(()),
        Ok(_) => Err(format!("{} is not a directory", path.display()).into()),
        Err(_) => Err(format!("directory not found: {}", path.display()).into()),
    }
}

impl Op {
    pub fn run(self, env: &Env, cancel: &CancellationToken) -> ToolOutput {
        let depth = if self.pattern.contains("**") {
            None
        } else {
            Some(self.pattern.matches('/').count() + 1)
        };
        let mut shown: Vec<String> = Vec::new();
        let mut more = 0usize;
        for entry in walk(&self.path.abs, depth, env, Tool::ListFiles, true) {
            if cancel.is_cancelled() {
                return cancelled(&shown.join("\n"));
            }
            let Ok(entry) = entry else { continue };
            if entry.depth() == 0 {
                continue;
            }
            let Ok(below) = entry.path().strip_prefix(&self.path.abs) else {
                continue;
            };
            if !self.glob.is_match(below) {
                continue;
            }
            if shown.len() >= MAX_ENTRIES {
                more += 1;
                continue;
            }
            let mut text = env.workspace.locate(entry.path()).display();
            if entry.file_type().is_some_and(|t| t.is_dir()) {
                text.push('/');
            }
            shown.push(text);
        }
        if shown.is_empty() {
            return ToolOutput::ok("(no files match)");
        }
        let mut out = shown.join("\n");
        if more > 0 {
            out.push_str(&format!(
                "\n… {} more entries; use a narrower path or pattern",
                group(more)
            ));
        }
        ToolOutput::ok(out)
    }
}
