//! Running `git` for a commit message: the repository, its state, the
//! changed files with their diffs, and recent subjects (see
//! `spec/commit.md`).

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::error::{Error, Result};

/// Options for every `git` call: no program from the repository's config
/// runs (`spec/commit.md`, "Git").
const SAFE: [&str; 6] = [
    "--no-pager",
    "-c",
    "core.fsmonitor=false",
    "-c",
    "core.quotepath=false",
    "--literal-pathspecs",
];

/// Options for every diff.
const DIFF: [&str; 5] = [
    "--no-color",
    "--no-ext-diff",
    "--no-textconv",
    "--find-renames",
    "--irreversible-delete",
];

/// Files whose presence means git writes the commit message itself.
const IN_PROGRESS: [(&str, &str); 5] = [
    ("MERGE_HEAD", "a merge"),
    ("rebase-merge", "a rebase"),
    ("rebase-apply", "a rebase"),
    ("CHERRY_PICK_HEAD", "a cherry-pick"),
    ("REVERT_HEAD", "a revert"),
];

/// How many recent subjects are sent.
const SUBJECTS: usize = 10;

/// Which changes are described.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scope {
    /// What `git commit` commits: the index against `HEAD`.
    #[default]
    Staged,
    /// What `git commit -a` commits: the work tree of tracked files (and
    /// files already staged) against `HEAD`. Untracked files are not in it.
    All,
}

/// One changed file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changed {
    /// `A`, `M`, `D`, `R`, `C`, `T`, …: the first letter of git's status.
    pub status: char,
    /// Relative to the root; the new path of a rename or copy.
    pub path: String,
    /// The old path of a rename or copy.
    pub from: Option<String>,
    /// Lines added and deleted; `None` for a binary file.
    pub lines: Option<(u64, u64)>,
    /// The file's part of the diff, from its `diff --git` line.
    pub diff: String,
}

/// A repository found from a directory in it.
#[derive(Debug, Clone)]
pub struct Repo {
    root: PathBuf,
}

impl Repo {
    /// The repository that holds `dir`, which must not be in the middle of a
    /// merge, rebase, cherry-pick or revert.
    pub fn open(dir: &Path) -> Result<Repo> {
        let out = run(dir, &["rev-parse", "--show-toplevel"])?;
        if !out.ok {
            return Err(Error::Failed(format!(
                "not a git repository: {}",
                dir.display()
            )));
        }
        let root = PathBuf::from(out.text.trim_end_matches(['\n', '\r']));
        if root.as_os_str().is_empty() {
            return Err(Error::Failed(format!(
                "not a git repository (no work tree): {}",
                dir.display()
            )));
        }
        let repo = Repo { root };
        repo.check_state()?;
        Ok(repo)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn git(&self, args: &[&str]) -> Result<String> {
        let out = run(&self.root, args)?;
        if out.ok {
            Ok(out.text)
        } else {
            Err(Error::Failed(format!("git: {}", out.error.trim())))
        }
    }

    fn check_state(&self) -> Result<()> {
        let mut args = vec!["rev-parse"];
        for (name, _) in IN_PROGRESS {
            args.extend(["--git-path", name]);
        }
        let text = self.git(&args)?;
        for (line, (_, what)) in text.lines().zip(IN_PROGRESS) {
            let path = self.root.join(line);
            if path.exists() {
                return Err(Error::Failed(format!(
                    "{what} is in progress; git writes its commit message"
                )));
            }
        }
        Ok(())
    }

    /// The changed files of `scope` with their diffs, in git's order.
    pub fn changes(&self, scope: Scope) -> Result<Vec<Changed>> {
        let base = match scope {
            Scope::Staged => "--cached".to_owned(),
            Scope::All => self.base()?,
        };
        let args = |extra: &[&'static str]| {
            let mut args = vec!["diff", base.as_str()];
            args.extend(DIFF);
            args.extend(extra);
            args
        };
        let names = self.git(&args(&["--name-status", "-z"]))?;
        let counts = self.git(&args(&["--numstat", "-z"]))?;
        let diff = self.git(&args(&["--unified=3"]))?;
        let mut files = parse_names(&names);
        for (file, lines) in files.iter_mut().zip(parse_numstat(&counts)) {
            file.lines = lines;
        }
        let pieces = split_diff(&diff);
        if pieces.len() == files.len() {
            for (file, piece) in files.iter_mut().zip(pieces) {
                file.diff = piece;
            }
        } else {
            tracing::warn!(
                files = files.len(),
                diffs = pieces.len(),
                "commit: diff pieces do not match the changed files"
            );
        }
        Ok(files)
    }

    /// What the work tree is compared with for [`Scope::All`]: `HEAD`, or
    /// the empty tree before the first commit.
    fn base(&self) -> Result<String> {
        if self.has_commits()? {
            return Ok("HEAD".to_owned());
        }
        // The empty tree's id depends on the repository's hash.
        let id = self.git(&["hash-object", "-t", "tree", "--stdin"])?;
        Ok(id.trim().to_owned())
    }

    fn has_commits(&self) -> Result<bool> {
        Ok(run(&self.root, &["rev-parse", "--verify", "--quiet", "HEAD"])?.ok)
    }

    /// The subjects of the last commits, newest first; none before the
    /// first commit.
    pub fn recent_subjects(&self) -> Result<Vec<String>> {
        if !self.has_commits()? {
            return Ok(Vec::new());
        }
        let count = format!("-n{SUBJECTS}");
        let text = self.git(&["log", &count, "--format=%s", "--no-show-signature"])?;
        Ok(text
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_owned)
            .collect())
    }
}

struct Out {
    ok: bool,
    text: String,
    error: String,
}

/// Runs `git -C dir ARGS` with the safe options, without a shell.
fn run(dir: &Path, args: &[&str]) -> Result<Out> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(SAFE)
        .args(args)
        .env("GIT_PAGER", "cat")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => Error::Failed("git not found".into()),
            _ => Error::Failed(format!("git: {e}")),
        })?;
    Ok(Out {
        ok: output.status.success(),
        text: String::from_utf8_lossy(&output.stdout).into_owned(),
        error: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

/// `--name-status -z`: `M\0path\0`, or `R100\0old\0new\0` for renames and
/// copies.
fn parse_names(text: &str) -> Vec<Changed> {
    let mut fields = text.split('\0').filter(|f| !f.is_empty());
    let mut files = Vec::new();
    while let Some(code) = fields.next() {
        let status = code.chars().next().unwrap_or('M');
        let Some(first) = fields.next() else { break };
        let (from, path) = if matches!(status, 'R' | 'C') {
            let Some(second) = fields.next() else { break };
            (Some(first.to_owned()), second.to_owned())
        } else {
            (None, first.to_owned())
        };
        files.push(Changed {
            status,
            path,
            from,
            lines: None,
            diff: String::new(),
        });
    }
    files
}

/// `--numstat -z`: `added\tdeleted\tpath\0`, or `added\tdeleted\t\0old\0new\0`
/// for renames; `-` counts for binary files.
fn parse_numstat(text: &str) -> Vec<Option<(u64, u64)>> {
    let mut fields = text.split('\0');
    let mut counts = Vec::new();
    while let Some(field) = fields.next() {
        if field.is_empty() {
            continue;
        }
        let mut parts = field.splitn(3, '\t');
        let added = parts.next().and_then(|n| n.parse().ok());
        let deleted = parts.next().and_then(|n| n.parse().ok());
        // A rename: the two paths follow as fields of their own.
        if parts.next().is_some_and(str::is_empty) {
            fields.next();
            fields.next();
        }
        counts.push(added.zip(deleted));
    }
    counts
}

/// The diff, cut before each `diff --git` line.
fn split_diff(text: &str) -> Vec<String> {
    let mut pieces: Vec<String> = Vec::new();
    for line in text.split_inclusive('\n') {
        if line.starts_with("diff --git ") {
            pieces.push(String::new());
        }
        if let Some(piece) = pieces.last_mut() {
            piece.push_str(line);
        }
    }
    pieces
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_with_renames() {
        let files = parse_names("M\0src/a.rs\0R087\0old.rs\0new name.rs\0D\0gone\0");
        let short: Vec<_> = files
            .iter()
            .map(|f| (f.status, f.path.as_str(), f.from.as_deref()))
            .collect();
        assert_eq!(
            short,
            [
                ('M', "src/a.rs", None),
                ('R', "new name.rs", Some("old.rs")),
                ('D', "gone", None)
            ]
        );
    }

    #[test]
    fn numstat_with_renames_and_binary() {
        let counts = parse_numstat("3\t1\tsrc/a.rs\0");
        assert_eq!(counts, [Some((3, 1))]);
        let counts = parse_numstat("0\t0\t\0old.rs\0new.rs\0-\t-\tlogo.png\x002\t0\tb\0");
        assert_eq!(counts, [Some((0, 0)), None, Some((2, 0))]);
    }

    #[test]
    fn diff_is_split_per_file() {
        let pieces = split_diff("diff --git a/x b/x\n+1\ndiff --git a/y b/y\n-2\n");
        assert_eq!(
            pieces,
            ["diff --git a/x b/x\n+1\n", "diff --git a/y b/y\n-2\n"]
        );
    }
}
