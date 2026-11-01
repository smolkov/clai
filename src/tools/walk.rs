//! Walking directories for `list_files` and `search_files`: `.gitignore`
//! respected, `.git` skipped, links not followed, denied entries left out.

use std::path::Path;

use ignore::{Walk, WalkBuilder};

use crate::config::{Action, Tool};

use super::Env;

/// Walks `base` in sorted order. Directories that a `deny` rule of `tool`
/// matches are not entered; denied files are left out too if `files` is
/// set (otherwise the caller checks them, e.g. to count them).
pub fn walk(base: &Path, max_depth: Option<usize>, env: &Env, tool: Tool, files: bool) -> Walk {
    let env = env.clone();
    WalkBuilder::new(base)
        .hidden(false)
        .ignore(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .parents(true)
        .require_git(false)
        .follow_links(false)
        .max_depth(max_depth)
        .sort_by_file_path(|a, b| a.cmp(b))
        .filter_entry(move |entry| {
            if entry.depth() == 0 {
                return true;
            }
            if entry.file_name() == ".git" {
                return false;
            }
            if !files && !entry.file_type().is_some_and(|t| t.is_dir()) {
                return true;
            }
            let path = env.workspace.locate(entry.path());
            env.checker().rules_for_path(tool, &path).action != Action::Deny
        })
        .build()
}
