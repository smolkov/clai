//! `search_files` (see `spec/tools.md`, "search_files").

use std::fs;
use std::path::Path;

use globset::GlobMatcher;
use regex::{Regex, RegexBuilder};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

use crate::config::{Action, Tool};
use crate::permissions::{Resolved, path_proposals};

use super::list_files::path_glob;
use super::output::{ToolOutput, plural};
use super::question::{Details, PathDetails};
use super::text::is_binary;
use super::walk::walk;
use super::{Call, Env, Failure, Op as AnyOp, cancelled};

const MAX_MATCHES: usize = 200;
const LINE_CHARS: usize = 300;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Args {
    pattern: String,
    path: Option<String>,
    glob: Option<String>,
    ignore_case: Option<bool>,
}

#[derive(Debug)]
pub struct Op {
    pub path: Resolved,
    regex: Regex,
    glob: Option<(GlobMatcher, GlobMatcher)>,
}

pub(super) fn prepare(env: &Env, args: Result<Args, Failure>) -> Result<Call, Failure> {
    let args = args?;
    let ignore_case = args.ignore_case.unwrap_or(false);
    let regex = RegexBuilder::new(&args.pattern)
        .case_insensitive(ignore_case)
        .build()
        .map_err(|e| Failure::Error(format!("invalid regular expression: {e}")))?;
    let written = args.path.unwrap_or_default();
    let path = env.workspace.resolve(&written);
    let decision = env.checker().check_path(Tool::SearchFiles, &path);
    if decision.action == Action::Deny {
        return Err(Failure::Denied(decision.denial()));
    }
    let glob = match &args.glob {
        Some(g) => {
            // By name (any `*`), or by path below `path` (`*` within a directory).
            let name = globset::GlobBuilder::new(g)
                .build()
                .map(|g| g.compile_matcher())
                .map_err(|e| Failure::Error(format!("invalid glob `{g}`: {}", e.kind())))?;
            Some((name, path_glob(g)?))
        }
        None => None,
    };
    if fs::metadata(&path.abs).is_err() {
        return Err(format!("path not found: {}", path.display()).into());
    }
    Ok(Call {
        summary: format!("{} in {}", args.pattern, path.display()),
        details: Details::SearchFiles {
            path: PathDetails::new(&written, &path),
            pattern: args.pattern,
            ignore_case,
            glob: args.glob,
        },
        proposals: path_proposals(Tool::SearchFiles, &path),
        decision,
        denial: None,
        op: AnyOp::Search(Op { path, regex, glob }),
    })
}

impl Op {
    pub fn run(self, env: &Env, cancel: &CancellationToken) -> ToolOutput {
        let mut lines: Vec<String> = Vec::new();
        let mut skipped = 0usize;
        let mut more_files = 0usize;
        let base = &self.path.abs;
        let single = base.is_file();
        for entry in walk(base, None, env, Tool::ReadFile, false) {
            if cancel.is_cancelled() {
                return cancelled(&lines.join("\n"));
            }
            let Ok(entry) = entry else { continue };
            let Some(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() || (entry.depth() == 0 && !single) {
                continue;
            }
            if !self.wanted(entry.path(), single) {
                continue;
            }
            let located = env.workspace.locate(entry.path());
            let decision = if kind.is_symlink() {
                // Reading follows the link: check where it points.
                let target = env.workspace.resolve(&entry.path().to_string_lossy());
                env.checker().check_path(Tool::ReadFile, &target)
            } else {
                env.checker().rules_for_path(Tool::ReadFile, &located)
            };
            if decision.action != Action::Allow {
                skipped += 1;
                continue;
            }
            let Ok(bytes) = fs::read(entry.path()) else {
                continue;
            };
            if is_binary(&bytes) {
                continue;
            }
            let text = String::from_utf8_lossy(&bytes);
            let shown = located.display();
            let mut file_has_more = false;
            for (i, line) in text.lines().enumerate() {
                let Some(found) = self.regex.find(line) else {
                    continue;
                };
                if lines.len() >= MAX_MATCHES {
                    file_has_more = true;
                    break;
                }
                lines.push(format!(
                    "{shown}:{}: {}",
                    i + 1,
                    excerpt(line, found.start())
                ));
            }
            more_files += usize::from(file_has_more);
        }
        let mut out = if lines.is_empty() {
            "(no matches)".to_owned()
        } else {
            lines.join("\n")
        };
        if more_files > 0 {
            out.push_str(&format!(
                "\n… more matches in {}; use a narrower pattern, path or glob",
                plural(more_files, "file", "files")
            ));
        }
        if skipped > 0 {
            out.push_str(&format!(
                "\n({} skipped by permission rules; read them with read_file to be asked)",
                plural(skipped, "file", "files")
            ));
        }
        ToolOutput::ok(out)
    }

    fn wanted(&self, path: &Path, single: bool) -> bool {
        let Some((by_name, by_path)) = &self.glob else {
            return true;
        };
        if single {
            return true;
        }
        let name_hit = path.file_name().is_some_and(|n| by_name.is_match(n));
        let path_hit = path
            .strip_prefix(&self.path.abs)
            .is_ok_and(|below| by_path.is_match(below));
        name_hit || path_hit
    }
}

/// A matching line, cut to 300 characters around the match.
fn excerpt(line: &str, match_start: usize) -> String {
    let total = line.chars().count();
    if total <= LINE_CHARS {
        return line.to_owned();
    }
    let at = line.get(..match_start).map_or(0, |b| b.chars().count());
    let start = at.saturating_sub(LINE_CHARS / 3).min(total - LINE_CHARS);
    let mut out: String = line.chars().skip(start).take(LINE_CHARS).collect();
    if start > 0 {
        out.insert(0, '…');
    }
    if start + LINE_CHARS < total {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpts() {
        assert_eq!(excerpt("short line", 0), "short line");
        let line = format!("{}MATCH{}", "a".repeat(500), "b".repeat(500));
        let cut = excerpt(&line, 500);
        assert!(cut.starts_with('…') && cut.ends_with('…'));
        assert!(cut.contains("MATCH"));
        assert_eq!(cut.chars().count(), LINE_CHARS + 2);
        let start = format!("MATCH{}", "x".repeat(400));
        assert!(excerpt(&start, 0).starts_with("MATCH"));
    }
}
