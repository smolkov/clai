//! The request for a commit message: the system prompt, the agent file and
//! the user message (see `spec/commit.md`, "Agent file" and "Prompt").

use std::fs;
use std::path::Path;

use super::changes::Changes;

/// The fixed system prompt.
pub const SYSTEM: &str = include_str!("prompt.md");

/// Names of the agent file at the repository root, the first found counts.
pub const AGENT_FILES: [&str; 3] = ["AGENTS.md", "AGENT.md", "agent.md"];
/// Characters of the agent file sent; a longer file is cut.
pub const AGENT_FILE_MAX: usize = 32_000;

/// The agent file as sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentFile {
    pub name: String,
    pub text: String,
    pub cut: bool,
}

/// The agent file at `root`: a regular file, or a link to one inside
/// `root`. `protected` says whether a name must not be sent.
pub fn agent_file(root: &Path, protected: impl Fn(&str) -> bool) -> Option<AgentFile> {
    let canonical_root = fs::canonicalize(root).ok()?;
    AGENT_FILES.iter().find_map(|name| {
        let target = fs::canonicalize(root.join(name)).ok()?;
        if !target.starts_with(&canonical_root) || !target.is_file() || protected(name) {
            return None;
        }
        let bytes = fs::read(&target).ok()?;
        let text = String::from_utf8_lossy(&bytes);
        let (text, cut) = cut(&text, AGENT_FILE_MAX);
        Some(AgentFile {
            name: (*name).to_owned(),
            text,
            cut,
        })
    })
}

/// `text` up to `max` characters, and whether it was cut.
fn cut(text: &str, max: usize) -> (String, bool) {
    match text.char_indices().nth(max) {
        Some((at, _)) => (text.get(..at).unwrap_or(text).to_owned(), true),
        None => (text.to_owned(), false),
    }
}

/// What the user message is made from.
#[derive(Debug, Clone, Default)]
pub struct Parts<'a> {
    pub agent_file: Option<&'a AgentFile>,
    pub subjects: &'a [String],
    pub hint: Option<&'a str>,
}

/// The user message: the parts in tagged blocks, the changes last.
pub fn user_message(parts: &Parts<'_>, changes: &Changes) -> String {
    let mut out = String::new();
    if let Some(file) = parts.agent_file {
        out.push_str(&format!("<agent_file name=\"{}\">\n", file.name));
        out.push_str(file.text.trim_end());
        if file.cut {
            out.push_str("\n(The file is longer; the rest is left out.)");
        }
        out.push_str("\n</agent_file>\n\n");
    }
    if !parts.subjects.is_empty() {
        out.push_str("<recent_subjects>\n");
        for subject in parts.subjects {
            out.push_str(&format!("- {subject}\n"));
        }
        out.push_str("</recent_subjects>\n\n");
    }
    if let Some(hint) = parts.hint.map(str::trim).filter(|h| !h.is_empty()) {
        out.push_str(&format!("<hint>\n{hint}\n</hint>\n\n"));
    }
    out.push_str("<files>\n");
    out.push_str(&changes.files);
    out.push_str("</files>\n\n");
    if changes.left_out > 0 {
        out.push_str(&format!(
            "Diffs of {} {} were left out for their size; describe {} from the file list only.\n\n",
            changes.left_out,
            if changes.left_out == 1 {
                "file"
            } else {
                "files"
            },
            if changes.left_out == 1 { "it" } else { "them" },
        ));
    }
    out.push_str("<diffs>\n");
    out.push_str(&changes.diffs);
    out.push_str("</diffs>\n\nWrite the commit message.");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn changes(left_out: usize) -> Changes {
        Changes {
            files: "M a (+1 -0)\n".into(),
            diffs: "diff --git a/a b/a\n+x\n".into(),
            left_out,
        }
    }

    #[test]
    fn agent_file_by_each_name_at_the_root_only() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        assert_eq!(agent_file(root, |_| false), None);
        fs::create_dir(root.join("sub")).unwrap();
        fs::write(root.join("sub/AGENTS.md"), "below").unwrap();
        assert_eq!(agent_file(root, |_| false), None);
        for name in AGENT_FILES.iter().rev() {
            fs::write(root.join(name), format!("rules of {name}")).unwrap();
            let found = agent_file(root, |_| false).unwrap();
            // Case-insensitive file systems see one file under all names.
            assert!(found.text.starts_with("rules of "), "{found:?}");
            assert!(!found.cut);
        }
        let found = agent_file(root, |_| false).unwrap();
        assert_eq!(found.name, "AGENTS.md");
        assert_eq!(agent_file(root, |_| true), None);
    }

    #[test]
    fn a_link_out_of_the_root_is_not_read() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("repo");
        fs::create_dir(&root).unwrap();
        fs::write(tmp.path().join("outside.md"), "secret").unwrap();
        std::os::unix::fs::symlink(tmp.path().join("outside.md"), root.join("AGENTS.md")).unwrap();
        assert_eq!(agent_file(&root, |_| false), None);
        fs::write(root.join("rules.md"), "inside").unwrap();
        fs::remove_file(root.join("AGENTS.md")).unwrap();
        std::os::unix::fs::symlink(root.join("rules.md"), root.join("AGENTS.md")).unwrap();
        assert_eq!(agent_file(&root, |_| false).unwrap().text, "inside");
    }

    #[test]
    fn a_long_agent_file_is_cut() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("AGENTS.md"), "é".repeat(AGENT_FILE_MAX + 5)).unwrap();
        let found = agent_file(tmp.path(), |_| false).unwrap();
        assert!(found.cut);
        assert_eq!(found.text.chars().count(), AGENT_FILE_MAX);
        let message = user_message(
            &Parts {
                agent_file: Some(&found),
                ..Parts::default()
            },
            &changes(0),
        );
        assert!(message.contains("(The file is longer; the rest is left out.)"));
    }

    #[test]
    fn message_blocks_in_order() {
        let file = AgentFile {
            name: "AGENTS.md".into(),
            text: "Subjects in English.\n".into(),
            cut: false,
        };
        let subjects = ["Fix a".to_owned(), "Add b".to_owned()];
        let message = user_message(
            &Parts {
                agent_file: Some(&file),
                subjects: &subjects,
                hint: Some(" fixes #12 "),
            },
            &changes(2),
        );
        assert_eq!(
            message,
            "<agent_file name=\"AGENTS.md\">\nSubjects in English.\n</agent_file>\n\n\
             <recent_subjects>\n- Fix a\n- Add b\n</recent_subjects>\n\n\
             <hint>\nfixes #12\n</hint>\n\n\
             <files>\nM a (+1 -0)\n</files>\n\n\
             Diffs of 2 files were left out for their size; describe them from the file list only.\n\n\
             <diffs>\ndiff --git a/a b/a\n+x\n</diffs>\n\nWrite the commit message."
        );
    }

    #[test]
    fn message_without_optional_parts() {
        let message = user_message(&Parts::default(), &changes(0));
        assert!(message.starts_with("<files>\n"));
        assert!(!message.contains("<hint>"));
        assert!(!message.contains("left out"));
    }
}
