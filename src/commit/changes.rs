//! Which changes are sent to the model: the file list in full, then
//! the diffs within a size budget, never those of protected files (see
//! `spec/commit.md`, "Size budget" and "Protected files").

use super::git::Changed;

/// Characters of changes sent in all, file list included.
pub const BUDGET: usize = 48_000;
/// Characters of one file's diff; a longer one is left out.
pub const PER_FILE: usize = 8_000;

/// The changes as sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Changes {
    /// One line per changed file.
    pub files: String,
    /// The diffs, and a line in place of each one left out.
    pub diffs: String,
    /// Diffs left out for their size.
    pub left_out: usize,
}

/// A line of the file list: `M src/a.rs (+3 -1)`, `R old -> new (+0 -0)`.
fn file_line(file: &Changed) -> String {
    let name = match &file.from {
        Some(from) => format!("{from} -> {}", file.path),
        None => file.path.clone(),
    };
    let lines = match file.lines {
        Some((added, deleted)) => format!("+{added} -{deleted}"),
        None => "binary".to_owned(),
    };
    format!("{} {name} ({lines})\n", file.status)
}

fn changed(file: &Changed) -> String {
    match file.lines {
        Some((added, deleted)) => format!("{} lines changed", added + deleted),
        None => "binary".to_owned(),
    }
}

/// The changes to send. `protected` says whether a path's content must not
/// reach the model.
pub fn select(
    files: &[Changed],
    protected: impl Fn(&str) -> bool,
    budget: usize,
    per_file: usize,
) -> Changes {
    let list: String = files.iter().map(file_line).collect();
    let mut left = budget.saturating_sub(list.chars().count());
    let mut diffs = String::new();
    let mut left_out = 0;
    for file in files {
        let is_protected = protected(&file.path) || file.from.as_deref().is_some_and(&protected);
        if is_protected {
            diffs.push_str(&format!("{}: diff not sent (protected file)\n", file.path));
            continue;
        }
        let size = file.diff.chars().count();
        if size > per_file || size > left {
            left_out += 1;
            diffs.push_str(&format!(
                "{}: diff left out ({})\n",
                file.path,
                changed(file)
            ));
            continue;
        }
        left -= size;
        diffs.push_str(&file.diff);
        if !file.diff.ends_with('\n') {
            diffs.push('\n');
        }
    }
    Changes {
        files: list,
        diffs,
        left_out,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, diff: &str) -> Changed {
        Changed {
            status: 'M',
            path: path.into(),
            from: None,
            lines: Some((2, 1)),
            diff: diff.into(),
        }
    }

    #[test]
    fn small_diffs_are_sent_whole() {
        let files = [file("a", "diff a\n"), file("b", "diff b")];
        let changes = select(&files, |_| false, BUDGET, PER_FILE);
        assert_eq!(changes.files, "M a (+2 -1)\nM b (+2 -1)\n");
        assert_eq!(changes.diffs, "diff a\ndiff b\n");
        assert_eq!(changes.left_out, 0);
    }

    #[test]
    fn long_diffs_are_left_out_and_the_list_stays_whole() {
        let long = "x".repeat(30);
        let files = [
            file("a", &long),
            file("b", "short\n"),
            file("c", "c is long\n"),
        ];
        // The list takes 36 of 50; `a` is too long for one file, `b` fits,
        // `c` no longer does.
        let changes = select(&files, |_| false, 50, 20);
        assert_eq!(changes.files.lines().count(), 3);
        assert_eq!(
            changes.diffs,
            "a: diff left out (3 lines changed)\nshort\nc: diff left out (3 lines changed)\n"
        );
        assert_eq!(changes.left_out, 2);
    }

    #[test]
    fn protected_files_show_no_content() {
        let mut renamed = file("public.txt", "diff --git secret\n+KEY=1\n");
        renamed.status = 'R';
        renamed.from = Some(".env".into());
        let files = [file(".env", "+KEY=1\n"), renamed];
        let changes = select(&files, |p| p == ".env", BUDGET, PER_FILE);
        assert_eq!(
            changes.diffs,
            ".env: diff not sent (protected file)\npublic.txt: diff not sent (protected file)\n"
        );
        assert!(!changes.diffs.contains("KEY"));
        assert_eq!(changes.left_out, 0);
        assert!(changes.files.contains("R .env -> public.txt (+2 -1)"));
    }

    #[test]
    fn binary_files() {
        let mut logo = file("logo.png", &"b".repeat(100));
        logo.lines = None;
        let changes = select(&[logo], |_| false, BUDGET, 10);
        assert_eq!(changes.files, "M logo.png (binary)\n");
        assert_eq!(changes.diffs, "logo.png: diff left out (binary)\n");
    }
}
