//! Each tool in a temporary workspace.

use std::fs;
use std::os::unix::fs::symlink;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::config::permissions::parse_for_test;
use crate::config::{Action, Tool};
use crate::permissions::{SessionRules, Workspace};

use super::*;

const CONFIG: &str = r#"
[permissions]
read_file = "allow"
run_command = "allow"

[permissions.ask]
read = ["notes/**"]

[permissions.deny]
read = [".env*"]
run_command = ["rm"]
"#;

struct Fixture {
    tmp: tempfile::TempDir,
    env: Env,
}

impl Fixture {
    fn new() -> Fixture {
        Fixture::with(CONFIG)
    }

    fn with(config: &str) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("ws");
        fs::create_dir_all(&root).unwrap();
        let env = Env {
            permissions: Arc::new(parse_for_test(config, None)),
            session: Arc::new(SessionRules::default()),
            workspace: Arc::new(Workspace::new(&root, &[], None).unwrap()),
            lsp: None,
        };
        Fixture { tmp, env }
    }

    fn root(&self) -> &Path {
        self.env.workspace.root()
    }

    fn write(&self, rel: &str, content: impl AsRef<[u8]>) {
        let path = self.root().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.root().join(rel)).unwrap()
    }

    fn prepare(&self, tool: &str, args: Value) -> Prepared {
        prepare(&self.env, tool, &args)
    }

    fn ready(&self, tool: &str, args: Value) -> PreparedCall {
        match self.prepare(tool, args.clone()) {
            Prepared::Ready(call) => *call,
            other => panic!("{tool} {args}: {other:?}"),
        }
    }

    fn failed(&self, tool: &str, args: Value) -> String {
        match self.prepare(tool, args.clone()) {
            Prepared::Failed(out) => {
                assert!(out.is_error);
                out.text
            }
            other => panic!("{tool} {args}: {other:?}"),
        }
    }

    fn denied(&self, tool: &str, args: Value) -> String {
        match self.prepare(tool, args.clone()) {
            Prepared::Denied(out) => out.text,
            other => panic!("{tool} {args}: {other:?}"),
        }
    }

    async fn run(&self, tool: &str, args: Value) -> ToolOutput {
        self.ready(tool, args)
            .run(&self.env, CancellationToken::new())
            .await
    }

    /// The text of a successful run.
    async fn ok(&self, tool: &str, args: Value) -> String {
        let out = self.run(tool, args.clone()).await;
        assert!(!out.is_error, "{tool} {args}: {}", out.text);
        out.text
    }
}

#[test]
fn specs_cover_all_tools() {
    // `lsp` is added by the agent when the session has a server.
    let mut names: Vec<String> = specs().into_iter().map(|s| s.name).collect();
    names.push(super::lsp::spec().name);
    assert_eq!(names, Tool::ALL.map(|t| t.name().to_owned()));
    for spec in specs().into_iter().chain([super::lsp::spec()]) {
        assert_eq!(spec.input_schema["type"], "object");
    }
}

#[test]
fn argument_errors() {
    let f = Fixture::new();
    let e = f.failed("read_file", json!({}));
    assert!(e.contains("missing field `path`"), "{e}");
    let e = f.failed("read_file", json!({"path": "a", "colour": 1}));
    assert!(e.contains("unknown field `colour`"), "{e}");
    let e = f.failed("read_file", json!({"path": 1}));
    assert!(e.contains("invalid type"), "{e}");
    let e = f.failed("read_file", Value::String("{\"path\": ".into()));
    assert!(e.contains("not valid JSON"), "{e}");
    let e = f.failed("read_file", json!({"path": "a", "limit": 5000}));
    assert_eq!(e, "limit must be 1–2000, not 5000");
    let e = f.failed("delete_file", json!({}));
    assert!(
        e.starts_with("unknown tool `delete_file`; tools are read_file"),
        "{e}"
    );
    let e = f.failed("run_command", json!({"command": "ls", "timeout_s": 0}));
    assert_eq!(e, "timeout_s must be 1–600, not 0");
}

#[tokio::test]
async fn read_file_lines() {
    let f = Fixture::new();
    f.write("a.txt", "one\ntwo\r\nthree\n");
    assert_eq!(
        f.ok("read_file", json!({"path": "a.txt"})).await,
        "     1→one\n     2→two\n     3→three"
    );
    assert_eq!(
        f.ok(
            "read_file",
            json!({"path": "a.txt", "offset": 2, "limit": 1})
        )
        .await,
        "     2→two\n… 1 more lines; use offset 3 to read on"
    );
    assert_eq!(
        f.ok("read_file", json!({"path": "a.txt", "offset": 9}))
            .await,
        "(file has 3 lines)"
    );
    f.write("empty", "");
    assert_eq!(
        f.ok("read_file", json!({"path": "empty"})).await,
        "(empty file)"
    );
    f.write("long", "x".repeat(2500));
    let text = f.ok("read_file", json!({"path": "long"})).await;
    assert!(text.ends_with("… (line cut)"));
    let call = f.ready("read_file", json!({"path": "a.txt"}));
    assert_eq!(call.summary, "a.txt (lines 1–3)");
    assert_eq!(call.action(), Action::Allow);
}

#[test]
fn read_file_errors() {
    let f = Fixture::new();
    f.write("src/parse.rs", "");
    let e = f.failed("read_file", json!({"path": "src/pars.rs"}));
    assert_eq!(e, "file not found: src/pars.rs; similar names: parse.rs");
    let e = f.failed("read_file", json!({"path": "src"}));
    assert_eq!(e, "src is a directory; use list_files");
    f.write("bin", b"\x7fELF\0\0");
    assert_eq!(
        f.failed("read_file", json!({"path": "bin"})),
        "bin is a binary file"
    );
    let e = f.denied("read_file", json!({"path": "/etc/passwd"}));
    assert_eq!(e, "denied: /etc/passwd is outside the workspace");
    f.write(".env", "KEY=1");
    let e = f.denied("read_file", json!({"path": ".env"}));
    assert_eq!(e, "denied by permission rule: deny read(.env*)");
    f.write("notes/a.md", "x");
    let call = f.ready("read_file", json!({"path": "notes/a.md"}));
    assert_eq!(call.action(), Action::Ask);
    assert_eq!(call.proposals[0], ["read_file(notes/a.md)"]);
}

#[tokio::test]
async fn list_files_entries() {
    let f = Fixture::new();
    f.write("b.rs", "");
    f.write("a/x.rs", "");
    f.write("a/deep/y.rs", "");
    f.write(".hidden", "");
    f.write(".env", "");
    f.write("target/out", "");
    f.write(".gitignore", "target/\n");
    fs::create_dir_all(f.root().join(".git")).unwrap();
    assert_eq!(
        f.ok("list_files", json!({})).await,
        ".gitignore\n.hidden\na/\nb.rs"
    );
    assert_eq!(
        f.ok("list_files", json!({"pattern": "**/*.rs"})).await,
        "a/deep/y.rs\na/x.rs\nb.rs"
    );
    assert_eq!(
        f.ok("list_files", json!({"path": "a"})).await,
        "a/deep/\na/x.rs"
    );
    assert_eq!(
        f.ok("list_files", json!({"pattern": "*.md"})).await,
        "(no files match)"
    );
    assert_eq!(
        f.failed("list_files", json!({"path": "b.rs"})),
        "b.rs is not a directory"
    );
    assert_eq!(
        f.failed("list_files", json!({"path": "nope"})),
        "directory not found: nope"
    );
    assert!(
        f.failed("list_files", json!({"pattern": "a["}))
            .starts_with("invalid glob")
    );
}

#[tokio::test]
async fn builtin_rules_hide_secret_files() {
    let f = Fixture::with("[permissions]\n");
    f.write(".env", "KEY=parse");
    f.write("certs/tls.key", "parse");
    f.write(".env.example", "KEY=parse");
    f.write("src/main.rs", "parse");
    assert_eq!(
        f.ok("list_files", json!({})).await,
        ".env.example\ncerts/\nsrc/"
    );
    assert_eq!(
        f.ok("search_files", json!({"pattern": "parse"})).await,
        ".env.example:1: KEY=parse\nsrc/main.rs:1: parse\n(2 files skipped by permission rules; read them with read_file to be asked)"
    );
    let e = f.denied("read_file", json!({"path": ".env"}));
    assert!(
        e.starts_with(
            "denied by permission rule: deny read(.env, .env.*): this file usually holds secrets"
        ),
        "{e}"
    );
}

#[tokio::test]
async fn list_files_limit() {
    let f = Fixture::new();
    for i in 0..1005 {
        f.write(&format!("d/f{i:04}"), "");
    }
    let text = f.ok("list_files", json!({"path": "d"})).await;
    assert!(
        text.ends_with("… 5 more entries; use a narrower path or pattern"),
        "{}",
        &text[text.len() - 80..]
    );
    assert_eq!(text.lines().count(), 1001);
}

#[tokio::test]
async fn search_files_matches() {
    let f = Fixture::new();
    f.write("src/parse.rs", "use std::fs;\nfn parse(input: &str) {}\n");
    f.write(
        "src/main.rs",
        "fn main() {\n    let lines = parse(&text);\n}\n",
    );
    f.write("README.md", "parse things\n");
    f.write(".env", "parse=secret\n");
    f.write("notes/todo.md", "parse later\n");
    f.write("bin.dat", b"parse\0\0");
    assert_eq!(
        f.ok(
            "search_files",
            json!({"pattern": "parse\\(", "path": "src"})
        )
        .await,
        "src/main.rs:2:     let lines = parse(&text);\nsrc/parse.rs:2: fn parse(input: &str) {}"
    );
    let text = f
        .ok(
            "search_files",
            json!({"pattern": "PARSE", "ignore_case": true}),
        )
        .await;
    assert_eq!(
        text,
        "README.md:1: parse things\nsrc/main.rs:2:     let lines = parse(&text);\nsrc/parse.rs:2: fn parse(input: &str) {}\n(2 files skipped by permission rules; read them with read_file to be asked)"
    );
    assert_eq!(
        f.ok("search_files", json!({"pattern": "parse", "glob": "*.md"}))
            .await,
        "README.md:1: parse things\n(1 file skipped by permission rules; read them with read_file to be asked)"
    );
    assert_eq!(
        f.ok(
            "search_files",
            json!({"pattern": "fs;", "path": "src/parse.rs"})
        )
        .await,
        "src/parse.rs:1: use std::fs;"
    );
    assert_eq!(
        f.ok("search_files", json!({"pattern": "zzz"})).await,
        "(no matches)\n(2 files skipped by permission rules; read them with read_file to be asked)"
    );
    let e = f.failed("search_files", json!({"pattern": "("}));
    assert!(e.starts_with("invalid regular expression"), "{e}");
}

#[tokio::test]
async fn search_files_limits_and_links() {
    let f = Fixture::new();
    f.write("a.txt", "hit\n".repeat(250));
    f.write("b.txt", "hit\n");
    let text = f.ok("search_files", json!({"pattern": "hit"})).await;
    assert_eq!(text.lines().count(), 201);
    assert!(
        text.ends_with("… more matches in 2 files; use a narrower pattern, path or glob"),
        "{text}"
    );
    // A link to a file outside the workspace is not read.
    let outside = f.tmp.path().join("secret.txt");
    fs::write(&outside, "hit\n").unwrap();
    let g = Fixture::new();
    symlink(&outside, g.root().join("link.txt")).unwrap();
    let text = g.ok("search_files", json!({"pattern": "hit"})).await;
    assert_eq!(
        text,
        "(no matches)\n(1 file skipped by permission rules; read them with read_file to be asked)"
    );
}

#[tokio::test]
async fn edit_file_replaces() {
    let f = Fixture::new();
    f.write("src/parse.rs", "a\nfor i in 0..lines.len() - 1 {\nb\n");
    let call = f.ready(
        "edit_file",
        json!({"path": "src/parse.rs", "old_text": "for i in 0..lines.len() - 1 {", "new_text": "for i in 0..lines.len() {"}),
    );
    assert_eq!(call.action(), Action::Ask);
    assert_eq!(call.summary, "src/parse.rs (line 2)");
    let Details::EditFile {
        diff, occurrences, ..
    } = &call.details
    else {
        panic!("{:?}", call.details)
    };
    assert_eq!(*occurrences, 1);
    assert!(
        diff.text
            .contains("-for i in 0..lines.len() - 1 {\n+for i in 0..lines.len() {"),
        "{}",
        diff.text
    );
    let out = call.run(&f.env, CancellationToken::new()).await;
    assert_eq!(
        out.text,
        "edited src/parse.rs: replaced 1 occurrence (line 2)"
    );
    assert_eq!(f.read("src/parse.rs"), "a\nfor i in 0..lines.len() {\nb\n");
}

#[tokio::test]
async fn edit_file_occurrences() {
    let f = Fixture::new();
    f.write("a.rs", "x\nfoo\nfoo\ny\nfoo");
    let e = f.failed(
        "edit_file",
        json!({"path": "a.rs", "old_text": "foo", "new_text": "bar"}),
    );
    assert_eq!(
        e,
        "old_text occurs 3 times (lines 2, 3, 5); add surrounding text to make it unique, or set replace_all"
    );
    let text = f
        .ok(
            "edit_file",
            json!({"path": "a.rs", "old_text": "foo", "new_text": "bar", "replace_all": true}),
        )
        .await;
    assert_eq!(text, "edited a.rs: replaced 3 occurrences (lines 2, 3, 5)");
    assert_eq!(f.read("a.rs"), "x\nbar\nbar\ny\nbar");
}

#[tokio::test]
async fn edit_file_keeps_crlf_and_final_newline() {
    let f = Fixture::new();
    f.write("w.txt", "one\r\ntwo\r\nthree");
    f.ok(
        "edit_file",
        json!({"path": "w.txt", "old_text": "one\ntwo", "new_text": "1\n2"}),
    )
    .await;
    assert_eq!(f.read("w.txt"), "1\r\n2\r\nthree");
}

#[tokio::test]
async fn edit_file_errors() {
    let f = Fixture::new();
    f.write("a.rs", "    let x = 1;\n");
    let e = f.failed(
        "edit_file",
        json!({"path": "a.rs", "old_text": "let  x = 1;", "new_text": "y"}),
    );
    assert!(
        e.starts_with("old_text not found in a.rs; this text at line 1 differs only in whitespace"),
        "{e}"
    );
    let e = f.failed(
        "edit_file",
        json!({"path": "a.rs", "old_text": "", "new_text": "y"}),
    );
    assert_eq!(e, "old_text is empty; to create a file, use write_file");
    let e = f.failed(
        "edit_file",
        json!({"path": "a.rs", "old_text": "x", "new_text": "x"}),
    );
    assert!(e.contains("same as old_text"), "{e}");
    f.write("latin1", b"caf\xe9\n");
    let e = f.failed(
        "edit_file",
        json!({"path": "latin1", "old_text": "caf", "new_text": "x"}),
    );
    assert!(e.contains("not valid UTF-8"), "{e}");
    let e = f.failed(
        "edit_file",
        json!({"path": "missing.rs", "old_text": "a", "new_text": "b"}),
    );
    assert!(e.starts_with("file not found: missing.rs"), "{e}");
    // The file changes between the question and the answer.
    let call = f.ready(
        "edit_file",
        json!({"path": "a.rs", "old_text": "x", "new_text": "y"}),
    );
    f.write("a.rs", "    let x = 2;\n");
    let out = call.run(&f.env, CancellationToken::new()).await;
    assert!(out.is_error);
    assert_eq!(out.text, "file changed while waiting for permission");
    assert_eq!(f.read("a.rs"), "    let x = 2;\n");
}

#[tokio::test]
async fn write_file_creates_and_replaces() {
    let f = Fixture::new();
    let call = f.ready(
        "write_file",
        json!({"path": "tests/new/a.rs", "content": "#[test]\nfn t() {}\n"}),
    );
    let Details::WriteFile {
        exists,
        created_dirs,
        content,
        ..
    } = &call.details
    else {
        panic!()
    };
    assert!(!exists);
    assert_eq!(created_dirs, &["tests", "tests/new"]);
    assert!(content.as_ref().unwrap().text.contains("fn t()"));
    assert_eq!(call.summary, "tests/new/a.rs (2 lines)");
    let out = call.run(&f.env, CancellationToken::new()).await;
    assert_eq!(out.text, "created tests/new/a.rs (2 lines)");
    let e = f.failed(
        "write_file",
        json!({"path": "tests/new/a.rs", "content": "x"}),
    );
    assert_eq!(
        e,
        "tests/new/a.rs exists; use edit_file to change it, or set overwrite"
    );
    let call = f.ready(
        "write_file",
        json!({"path": "tests/new/a.rs", "content": "one\n", "overwrite": true}),
    );
    let Details::WriteFile { diff, .. } = &call.details else {
        panic!()
    };
    assert!(diff.as_ref().unwrap().text.contains("+one"));
    let out = call.run(&f.env, CancellationToken::new()).await;
    assert_eq!(out.text, "replaced tests/new/a.rs (1 line, was 2)");
    assert_eq!(f.read("tests/new/a.rs"), "one\n");
}

#[tokio::test]
async fn write_file_errors() {
    let f = Fixture::new();
    f.write("file", "x");
    assert_eq!(
        f.failed("write_file", json!({"path": "file/a.rs", "content": ""})),
        "file is a file, not a directory"
    );
    fs::create_dir_all(f.root().join("dir")).unwrap();
    assert_eq!(
        f.failed(
            "write_file",
            json!({"path": "dir", "content": "", "overwrite": true})
        ),
        "dir is a directory"
    );
    let e = f.denied("write_file", json!({"path": "../x", "content": ""}));
    assert!(e.ends_with("is outside the workspace"), "{e}");
    // Created by someone else while the question was open.
    let call = f.ready("write_file", json!({"path": "late.txt", "content": "mine"}));
    f.write("late.txt", "theirs");
    let out = call.run(&f.env, CancellationToken::new()).await;
    assert_eq!(out.text, "file changed while waiting for permission");
    assert_eq!(f.read("late.txt"), "theirs");
}

#[tokio::test]
async fn run_command_output() {
    let f = Fixture::new();
    assert_eq!(
        f.ok("run_command", json!({"command": "echo hi"})).await,
        "hi\nexit code 0"
    );
    assert_eq!(
        f.ok("run_command", json!({"command": "true"})).await,
        "exit code 0"
    );
    assert_eq!(
        f.ok(
            "run_command",
            json!({"command": "echo out; echo err >&2; echo out2"})
        )
        .await,
        "out\nerr\nout2\nexit code 0"
    );
    let out = f
        .run("run_command", json!({"command": "echo no; exit 3"}))
        .await;
    assert!(out.is_error);
    assert_eq!(out.text, "no\nexit code 3");
    // Runs in the workspace, with empty input and the extra environment.
    let pwd = f.ok("run_command", json!({"command": "pwd"})).await;
    assert_eq!(pwd, format!("{}\nexit code 0", f.root().display()));
    assert_eq!(
        f.ok("run_command", json!({"command": "cat"})).await,
        "exit code 0"
    );
    assert_eq!(
        f.ok(
            "run_command",
            json!({"command": "echo $PAGER $GIT_TERMINAL_PROMPT"})
        )
        .await,
        "cat 0\nexit code 0"
    );
    let call = f.ready("run_command", json!({"command": "ls && cat x"}));
    let Details::RunCommand {
        parts,
        cwd,
        timeout_s,
        ..
    } = &call.details
    else {
        panic!()
    };
    assert_eq!(parts.len(), 2);
    assert_eq!(*timeout_s, 120);
    assert_eq!(cwd, &f.root().display().to_string());
}

#[tokio::test]
async fn run_command_denied() {
    let f = Fixture::new();
    let e = f.denied("run_command", json!({"command": "ls && rm -rf x"}));
    assert_eq!(
        e,
        "denied by permission rule: deny run_command(rm), for the part \"rm -rf x\""
    );
    assert_eq!(
        f.failed("run_command", json!({"command": "  "})),
        "command is empty"
    );
}

#[tokio::test]
async fn run_command_timeout_stops_the_group() {
    let f = Fixture::new();
    let marker = f.root().join("survived");
    let start = Instant::now();
    let out = f
        .run(
            "run_command",
            json!({
                "command": format!("echo started; (sleep 3; touch {}) & sleep 30", marker.display()),
                "timeout_s": 1
            }),
        )
        .await;
    assert!(out.is_error);
    assert_eq!(out.text, "started\nstopped after 1 s (timeout)");
    assert!(
        start.elapsed() < Duration::from_secs(5),
        "{:?}",
        start.elapsed()
    );
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(
        !marker.exists(),
        "a background process survived the timeout"
    );
}

#[tokio::test]
async fn run_command_cancel() {
    let f = Fixture::new();
    let call = f.ready("run_command", json!({"command": "echo before; sleep 30"}));
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        trigger.cancel();
    });
    let start = Instant::now();
    let out = call.run(&f.env, cancel).await;
    assert_eq!(out.text, "before\ncancelled by the user");
    assert!(out.is_error);
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn run_command_background_does_not_hang() {
    let f = Fixture::new();
    let start = Instant::now();
    let out = f
        .run("run_command", json!({"command": "sleep 30 & echo hi"}))
        .await;
    assert_eq!(out.text, "hi\nexit code 0");
    assert!(
        start.elapsed() < Duration::from_secs(3),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn accept_rules() {
    let f = Fixture::with("[permissions.allow]\nrun_command = [\"cargo build\"]\n");
    let call = f.ready(
        "run_command",
        json!({"command": "cargo build && ./deploy.sh staging"}),
    );
    assert_eq!(call.action(), Action::Ask);
    assert_eq!(call.proposals[0], ["run_command(./deploy.sh staging)"]);
    let rules = call
        .accept_rules(&f.env, &["run_command(./deploy.sh)".into()])
        .unwrap();
    assert_eq!(rules[0].to_string(), "allow run_command(./deploy.sh)");
    let e = call
        .accept_rules(&f.env, &["run_command(./other.sh)".into()])
        .unwrap_err();
    assert_eq!(e, "the rules do not allow this call");
    let e = call
        .accept_rules(&f.env, &["edit_file(x)".into()])
        .unwrap_err();
    assert_eq!(e, "rule `edit_file(x)` is not for run_command");
    assert!(call.accept_rules(&f.env, &[]).is_err());
    // File tools: every proposal is accepted and allows the call.
    f.write("src/a.rs", "x");
    let call = f.ready(
        "edit_file",
        json!({"path": "src/a.rs", "old_text": "x", "new_text": "y"}),
    );
    assert_eq!(call.action(), Action::Ask);
    for proposal in &call.proposals {
        assert!(call.accept_rules(&f.env, proposal).is_ok(), "{proposal:?}");
    }
}
