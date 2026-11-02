//! What a tool shows in a permission question (see `spec/tools.md`,
//! "Questions"). The same data goes to every client.

use serde::{Deserialize, Serialize};

use crate::permissions::{Area, PartDecision, RedirectDecision, Resolved};

use super::output::{FULL_BYTES, cap_bytes};

/// A path as the model wrote it and as it was resolved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathDetails {
    pub written: String,
    /// Relative to the workspace, or absolute.
    pub resolved: String,
    /// `workspace`, `extra_dir` or `outside`.
    pub area: String,
}

impl PathDetails {
    pub fn new(written: &str, resolved: &Resolved) -> PathDetails {
        PathDetails {
            written: written.to_owned(),
            resolved: resolved.display(),
            area: match resolved.area {
                Area::Workspace(_) => "workspace",
                Area::Extra { .. } => "extra_dir",
                Area::Outside => "outside",
            }
            .to_owned(),
        }
    }
}

/// Large texts in a question, cut at 1 MiB.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Text {
    pub text: String,
    pub cut: bool,
}

impl Text {
    pub fn new(text: String) -> Text {
        let cut = text.len() > FULL_BYTES;
        Text {
            text: cap_bytes(text, FULL_BYTES),
            cut,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum Details {
    ReadFile {
        path: PathDetails,
        first_line: u64,
        last_line: u64,
        size: u64,
        /// Counted for files up to 10 MiB.
        lines: Option<usize>,
    },
    ListFiles {
        path: PathDetails,
        pattern: String,
    },
    SearchFiles {
        path: PathDetails,
        pattern: String,
        ignore_case: bool,
        glob: Option<String>,
    },
    EditFile {
        path: PathDetails,
        diff: Text,
        first_line: usize,
        last_line: usize,
        occurrences: usize,
        size: u64,
        /// Seconds since the Unix epoch.
        modified: Option<u64>,
    },
    WriteFile {
        path: PathDetails,
        exists: bool,
        /// For a new file.
        content: Option<Text>,
        /// For `overwrite`: against the old content.
        diff: Option<Text>,
        lines: usize,
        created_dirs: Vec<String>,
    },
    RunCommand {
        command: String,
        cwd: String,
        timeout_s: u64,
        env: Vec<(String, String)>,
        parts: Vec<PartDecision>,
        redirects: Vec<RedirectDecision>,
    },
}
