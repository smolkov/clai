//! Commit messages against real repositories in temporary directories.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tokio_util::sync::CancellationToken;

use super::*;
use crate::config::permissions::parse_for_test;
use crate::permissions::{SessionRules, Workspace};
use crate::provider::StopReason;
use crate::provider::fake::Fake;

struct TestRepo {
    _tmp: tempfile::TempDir,
    root: PathBuf,
}

fn git(root: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

impl TestRepo {
    fn new() -> TestRepo {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap().join("repo");
        fs::create_dir(&root).unwrap();
        git(&root, &["init", "-q", "-b", "main"]);
        git(&root, &["config", "commit.gpgsign", "false"]);
        TestRepo { _tmp: tmp, root }
    }

    fn write(&self, path: &str, text: &str) {
        let path = self.root.join(path);
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).unwrap();
        }
        fs::write(path, text).unwrap();
    }

    fn git(&self, args: &[&str]) {
        git(&self.root, args);
    }

    fn commit(&self, subject: &str) {
        self.git(&["add", "-A"]);
        self.git(&["commit", "-q", "-m", subject]);
    }

    fn open(&self) -> Result<Repo> {
        Repo::open(&self.root)
    }
}

fn staged_names(repo: &Repo, scope: Scope) -> Vec<(char, String, Option<String>)> {
    repo.changes(scope)
        .unwrap()
        .into_iter()
        .map(|f| (f.status, f.path, f.from))
        .collect()
}

#[test]
fn not_a_repository() {
    let tmp = tempfile::tempdir().unwrap();
    let err = Repo::open(tmp.path()).unwrap_err();
    assert!(
        err.to_string().starts_with("not a git repository: "),
        "{err}"
    );
    assert_eq!(err.exit_code(), 1);
}

#[test]
fn a_subdirectory_opens_the_repository() {
    let r = TestRepo::new();
    r.write("src/a.rs", "a\n");
    let repo = Repo::open(&r.root.join("src")).unwrap();
    assert_eq!(repo.root(), r.root);
}

#[test]
fn only_staged_changes() {
    let r = TestRepo::new();
    r.write("kept.txt", "one\n");
    r.write("changed.txt", "one\n");
    r.commit("First");
    r.write("changed.txt", "two\n");
    r.write("kept.txt", "unstaged\n");
    r.write("new.txt", "untracked\n");
    r.git(&["add", "changed.txt"]);
    let repo = r.open().unwrap();
    let files = repo.changes(Scope::Staged).unwrap();
    assert_eq!(files.len(), 1);
    let file = files.first().unwrap();
    assert_eq!((file.status, file.path.as_str()), ('M', "changed.txt"));
    assert_eq!(file.lines, Some((1, 1)));
    assert!(
        file.diff
            .starts_with("diff --git a/changed.txt b/changed.txt\n")
    );
    assert!(file.diff.contains("-one\n+two\n"));
    assert!(!file.diff.contains("unstaged"));
}

#[test]
fn all_changes_of_tracked_files() {
    let r = TestRepo::new();
    r.write("staged.txt", "one\n");
    r.write("unstaged.txt", "one\n");
    r.write("removed.txt", "one\n");
    r.commit("First");
    r.write("staged.txt", "two\n");
    r.git(&["add", "staged.txt"]);
    r.write("unstaged.txt", "two\n");
    fs::remove_file(r.root.join("removed.txt")).unwrap();
    r.write("added.txt", "new\n");
    r.git(&["add", "added.txt"]);
    r.write("untracked.txt", "not in git\n");
    let repo = r.open().unwrap();
    let mut names = staged_names(&repo, Scope::All);
    names.sort();
    assert_eq!(
        names,
        [
            ('A', "added.txt".to_owned(), None),
            ('D', "removed.txt".to_owned(), None),
            ('M', "staged.txt".to_owned(), None),
            ('M', "unstaged.txt".to_owned(), None),
        ]
    );
    let files = repo.changes(Scope::All).unwrap();
    let unstaged = files.iter().find(|f| f.path == "unstaged.txt").unwrap();
    assert!(unstaged.diff.contains("-one\n+two\n"));
    // Staged only: the work tree's changes are not in it.
    let mut names = staged_names(&repo, Scope::Staged);
    names.sort();
    assert_eq!(
        names,
        [
            ('A', "added.txt".to_owned(), None),
            ('M', "staged.txt".to_owned(), None),
        ]
    );
}

#[test]
fn all_changes_before_the_first_commit() {
    let r = TestRepo::new();
    r.write("a.txt", "a\n");
    r.git(&["add", "a.txt"]);
    r.write("a.txt", "a\nb\n");
    r.write("untracked.txt", "x\n");
    let repo = r.open().unwrap();
    let files = repo.changes(Scope::All).unwrap();
    assert_eq!(files.len(), 1);
    let file = files.first().unwrap();
    assert_eq!((file.status, file.path.as_str()), ('A', "a.txt"));
    assert_eq!(file.lines, Some((2, 0)));
}

#[test]
fn nothing_staged() {
    let r = TestRepo::new();
    r.write("a.txt", "a\n");
    let repo = r.open().unwrap();
    let perms = parse_for_test("", None);
    let session = SessionRules::default();
    let workspace = Workspace::new(&r.root, &[], None).unwrap();
    let checker = Checker {
        permissions: &perms,
        session: &session,
        workspace: &workspace,
    };
    let err = prepare(&repo, Scope::Staged, &checker, None).unwrap_err();
    assert_eq!(
        err.to_string(),
        "nothing staged; stage changes with git add first, or use -a"
    );
    let err = prepare(&repo, Scope::All, &checker, None).unwrap_err();
    assert_eq!(err.to_string(), "nothing to commit");
}

#[test]
fn renames_deletions_and_binary_files() {
    let r = TestRepo::new();
    let body: String = (0..20).map(|i| format!("line {i}\n")).collect();
    r.write("old.txt", &body);
    r.write("gone.txt", "secret old content\n");
    r.commit("First");
    r.git(&["mv", "old.txt", "new name.txt"]);
    r.git(&["rm", "-q", "gone.txt"]);
    fs::write(r.root.join("logo.png"), [0u8, 159, 146, 150, 0, 1]).unwrap();
    r.git(&["add", "logo.png"]);
    let repo = r.open().unwrap();
    let mut names = staged_names(&repo, Scope::Staged);
    names.sort();
    assert_eq!(
        names,
        [
            ('A', "logo.png".to_owned(), None),
            ('D', "gone.txt".to_owned(), None),
            ('R', "new name.txt".to_owned(), Some("old.txt".to_owned())),
        ]
    );
    let files = repo.changes(Scope::Staged).unwrap();
    let gone = files.iter().find(|f| f.path == "gone.txt").unwrap();
    assert!(!gone.diff.contains("secret old content"), "{}", gone.diff);
    let logo = files.iter().find(|f| f.path == "logo.png").unwrap();
    assert_eq!(logo.lines, None);
}

#[test]
fn an_external_diff_program_is_not_run() {
    let r = TestRepo::new();
    r.write("a.txt", "a\n");
    r.commit("First");
    let marker = r.root.join("ran");
    let script = r.root.join("ext.sh");
    fs::write(
        &script,
        format!("#!/bin/sh\ntouch '{}'\n", marker.display()),
    )
    .unwrap();
    fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let script = script.display().to_string();
    r.git(&["config", "diff.external", &script]);
    r.write(".gitattributes", "*.txt diff=conv\n");
    r.git(&["config", "diff.conv.textconv", &script]);
    r.git(&["config", "core.fsmonitor", &script]);
    r.write("a.txt", "b\n");
    r.git(&[
        "-c",
        "core.fsmonitor=false",
        "add",
        "a.txt",
        ".gitattributes",
    ]);
    let _ = fs::remove_file(&marker);
    let repo = r.open().unwrap();
    let files = repo.changes(Scope::Staged).unwrap();
    assert!(files.iter().any(|f| f.diff.contains("+b\n")));
    let _ = repo.recent_subjects().unwrap();
    assert!(
        !marker.exists(),
        "a program from the repository's config ran"
    );
}

#[test]
fn a_merge_in_progress() {
    let r = TestRepo::new();
    r.write("a.txt", "a\n");
    r.commit("First");
    fs::write(r.root.join(".git/MERGE_HEAD"), "0000\n").unwrap();
    let err = r.open().unwrap_err();
    assert_eq!(
        err.to_string(),
        "a merge is in progress; git writes its commit message"
    );
}

#[test]
fn recent_subjects_newest_first() {
    let r = TestRepo::new();
    assert_eq!(
        r.open().unwrap().recent_subjects().unwrap(),
        Vec::<String>::new()
    );
    for i in 0..12 {
        r.write("a.txt", &format!("{i}\n"));
        r.commit(&format!("Change {i}"));
    }
    let subjects = r.open().unwrap().recent_subjects().unwrap();
    assert_eq!(subjects.len(), 10);
    assert_eq!(subjects.first().map(String::as_str), Some("Change 11"));
}

/// The user message for the staged changes of `r`, with `config` as the
/// permissions.
fn message(r: &TestRepo, config: &str, hint: Option<&str>) -> Prepared {
    let perms = parse_for_test(config, None);
    let session = SessionRules::default();
    let workspace = Workspace::new(&r.root, &[], None).unwrap();
    let checker = Checker {
        permissions: &perms,
        session: &session,
        workspace: &workspace,
    };
    prepare(&r.open().unwrap(), Scope::Staged, &checker, hint).unwrap()
}

#[test]
fn the_message_holds_rules_subjects_and_changes() {
    let r = TestRepo::new();
    r.write(
        "AGENTS.md",
        "## Commits\n\n- End with a Signed-off-by line.\n",
    );
    r.commit("Add agent file");
    r.write("src/lib.rs", "pub fn x() {}\n");
    r.git(&["add", "src/lib.rs"]);
    let prepared = message(&r, "", Some("fixes #12"));
    assert_eq!(prepared.files, 1);
    let m = &prepared.message;
    assert!(
        m.contains("<agent_file name=\"AGENTS.md\">\n## Commits"),
        "{m}"
    );
    assert!(m.contains("- End with a Signed-off-by line."));
    assert!(m.contains("<recent_subjects>\n- Add agent file\n</recent_subjects>"));
    assert!(m.contains("<hint>\nfixes #12\n</hint>"));
    assert!(m.contains("<files>\nA src/lib.rs (+1 -0)\n</files>"));
    assert!(m.contains("+pub fn x() {}"));
}

#[test]
fn protected_files_are_named_only() {
    let r = TestRepo::new();
    r.write(".env", "API_KEY=abc123\n");
    r.write("keys/deploy.txt", "hunter2\n");
    r.write("ok.txt", "fine\n");
    r.git(&["add", "-A"]);
    let config = "[permissions.deny]\nread = [\"keys\"]\n";
    let m = message(&r, config, None).message;
    assert!(m.contains(".env: diff not sent (protected file)"), "{m}");
    assert!(m.contains("keys/deploy.txt: diff not sent (protected file)"));
    assert!(!m.contains("abc123"));
    assert!(!m.contains("hunter2"));
    assert!(m.contains("+fine"));
}

fn model() -> Model {
    Model {
        name: "m".into(),
        id: "model-1".into(),
        display_name: "M".into(),
        context_window: 100_000,
        max_output_tokens: None,
        temperature: None,
    }
}

fn prepared() -> Prepared {
    Prepared {
        files: 1,
        message: "<files>\nM a (+1 -0)\n</files>".into(),
    }
}

fn says(parts: &[&str]) -> crate::provider::fake::Answer {
    let mut answer: crate::provider::fake::Answer = parts
        .iter()
        .map(|t| Ok(Event::TextDelta { text: (*t).into() }))
        .collect();
    answer.push(Ok(Event::Done {
        stop_reason: StopReason::EndTurn,
    }));
    answer
}

#[tokio::test]
async fn describe_sends_one_request_without_tools() {
    let fake = Fake::new(vec![says(&["```\nAdd a", "\n\nBecause.  \n```\n"])]);
    let text = describe(
        &fake,
        &model(),
        &prepared(),
        Some("Co-Authored-By: clai <clai@localhost>"),
        CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(
        text,
        "Add a\n\nBecause.\n\nCo-Authored-By: clai <clai@localhost>\n"
    );
    let requests = fake.requests();
    assert_eq!(requests.len(), 1);
    let request = requests.first().unwrap();
    assert!(request.tools.is_empty());
    assert_eq!(request.model, "model-1");
    assert_eq!(request.system, prompt::SYSTEM);
    assert_eq!(request.max_output_tokens, Some(1_024));
}

#[tokio::test]
async fn describe_errors() {
    let fake = Fake::new(vec![says(&["  \n\n"])]);
    let err = describe(&fake, &model(), &prepared(), None, CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "the model wrote no message");
    let fake = Fake::new(vec![vec![Err(ProviderError::new(
        crate::provider::ErrorKind::Auth,
        "bad key",
    ))]]);
    let err = describe(&fake, &model(), &prepared(), None, CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(err.to_string(), "model request failed: bad key");
    assert_eq!(err.exit_code(), 1);
}

#[test]
fn cleaning() {
    let trailer = Some("Signed-off-by: me");
    assert_eq!(clean("Fix it", None).as_deref(), Some("Fix it\n"));
    assert_eq!(
        clean("\n\nFix it  \n\n- one \n\n", None).as_deref(),
        Some("Fix it\n\n- one\n")
    );
    assert_eq!(
        clean("```text\nFix it\n```", None).as_deref(),
        Some("Fix it\n")
    );
    // A fence inside the body stays.
    assert_eq!(
        clean("Fix it\n\n```\ncode\n```", None).as_deref(),
        Some("Fix it\n\n```\ncode\n```\n")
    );
    assert_eq!(
        clean("Fix it", trailer).as_deref(),
        Some("Fix it\n\nSigned-off-by: me\n")
    );
    assert_eq!(
        clean("Fix it\n\nSigned-off-by: me", trailer).as_deref(),
        Some("Fix it\n\nSigned-off-by: me\n")
    );
    assert_eq!(clean("", trailer), None);
    assert_eq!(clean("```\n\n```", None), None);
}
