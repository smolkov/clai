//! `clai commit` as a process, in a git repository, against the local model
//! server of `support`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::fs;
use std::path::Path;
use std::process::Command;

use support::*;

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
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

/// Runs `clai` in `env`; git does not look above its home for a
/// repository (the test directories are inside this one).
fn clai(env: &Env, args: &[&str]) -> std::process::Output {
    env.command(args)
        .env("GIT_CEILING_DIRECTORIES", &env.root)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap()
}

/// The workspace of `env` as a repository with one commit.
fn repository(env: &Env) {
    let work = env.work();
    git(&work, &["init", "-q", "-b", "main"]);
    git(&work, &["config", "commit.gpgsign", "false"]);
    fs::write(
        work.join("AGENTS.md"),
        "# Rules\n\n## Commits\n\n- Subjects start with the area: `parser: …`.\n",
    )
    .unwrap();
    fs::write(work.join("readme.md"), "hello\n").unwrap();
    git(&work, &["add", "-A"]);
    git(&work, &["commit", "-q", "-m", "Start the project"]);
}

const TWO_MODELS: &str = r#"
[providers.test.models.n]
id = "model-2"
context_window = 100000
"#;

#[test]
fn message_on_stdout() {
    let server = Server::start(vec![say("parser: add the parser\n\nIt reads numbers.\n")]);
    let env = Env::new(
        "message",
        &server,
        "[commit]\ntrailer = \"Co-Authored-By: clai <clai@localhost>\"\n",
    );
    repository(&env);
    fs::write(env.work().join("parser.rs"), "fn parse() {}\n").unwrap();
    fs::write(env.work().join("notes.txt"), "not staged\n").unwrap();
    git(&env.work(), &["add", "parser.rs"]);

    let out = clai(&env, &["commit"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "parser: add the parser\n\nIt reads numbers.\n\nCo-Authored-By: clai <clai@localhost>\n"
    );
    // Standard error is not a terminal: no progress line.
    assert_eq!(stderr(&out), "");

    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request["model"], "model-1");
    assert!(
        request
            .get("tools")
            .is_none_or(|t| t.as_array().unwrap().is_empty())
    );
    let messages = request["messages"].as_array().unwrap();
    assert_eq!(messages[0]["role"], "system");
    assert!(
        messages[0]["content"]
            .as_str()
            .unwrap()
            .contains("You write git commit messages.")
    );
    let user = messages[1]["content"].as_str().unwrap();
    assert!(user.contains("Subjects start with the area"), "{user}");
    assert!(user.contains("<recent_subjects>\n- Start the project\n"));
    assert!(user.contains("A parser.rs (+1 -0)"));
    assert!(user.contains("+fn parse() {}"));
    assert!(!user.contains("notes.txt"));
}

#[test]
fn hint_and_subdirectory() {
    let server = Server::start(vec![say("Fix it")]);
    let env = Env::new("hint", &server, "");
    repository(&env);
    fs::create_dir(env.work().join("src")).unwrap();
    fs::write(env.work().join("src/a.rs"), "x\n").unwrap();
    git(&env.work(), &["add", "-A"]);
    let src = env.work().join("src");
    let out = clai(
        &env,
        &["commit", "--hint", "fixes #12", src.to_str().unwrap()],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), "Fix it\n");
    let user = server.requests()[0]["messages"][1]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(user.contains("<hint>\nfixes #12\n</hint>"), "{user}");
    // The agent file of the root is found from a subdirectory.
    assert!(user.contains("<agent_file name=\"AGENTS.md\">"));
}

#[test]
fn commit_model_then_option() {
    let server = Server::start(vec![say("One"), say("Two")]);
    let extra = format!("{TWO_MODELS}\n[commit]\nmodel = \"test/n\"\n");
    let env = Env::new("models", &server, &extra);
    repository(&env);
    fs::write(env.work().join("a.txt"), "a\n").unwrap();
    git(&env.work(), &["add", "-A"]);
    let out = clai(&env, &["commit"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let out = clai(&env, &["commit", "--model", "test/m"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let requests = server.requests();
    assert_eq!(requests[0]["model"], "model-2");
    assert_eq!(requests[1]["model"], "model-1");
}

#[test]
fn all_changes_like_commit_a() {
    let server = Server::start(vec![say("Update the readme")]);
    let env = Env::new("all", &server, "");
    repository(&env);
    fs::write(env.work().join("readme.md"), "hello again\n").unwrap();
    fs::write(env.work().join("untracked.txt"), "not in git\n").unwrap();
    let out = clai(&env, &["commit", "-a"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), "Update the readme\n");
    let user = server.requests()[0]["messages"][1]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(user.contains("M readme.md (+1 -1)"), "{user}");
    assert!(user.contains("+hello again"));
    assert!(!user.contains("untracked.txt"));
}

#[test]
fn nothing_staged() {
    let server = Server::start(Vec::new());
    let env = Env::new("nothing", &server, "");
    repository(&env);
    fs::write(env.work().join("readme.md"), "changed, not staged\n").unwrap();
    let out = clai(&env, &["commit"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stdout(&out), "");
    assert!(
        stderr(&out).contains("nothing staged; stage changes with git add first, or use -a"),
        "{}",
        stderr(&out)
    );
    assert!(server.requests().is_empty());
}

#[test]
fn not_a_repository() {
    let server = Server::start(Vec::new());
    let env = Env::new("norepo", &server, "");
    let out = clai(&env, &["commit"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).contains("not a git repository"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn unknown_model() {
    let server = Server::start(Vec::new());
    let env = Env::new("unknown", &server, "");
    repository(&env);
    let out = clai(&env, &["commit", "--model", "test/nope"]);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}

#[test]
fn model_error() {
    let server = Server::start(vec![Reply::Status(
        401,
        r#"{"error": {"message": "bad key"}}"#.into(),
    )]);
    let env = Env::new("error", &server, "");
    repository(&env);
    fs::write(env.work().join("a.txt"), "a\n").unwrap();
    git(&env.work(), &["add", "-A"]);
    let out = clai(&env, &["commit"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stdout(&out), "");
    assert!(
        stderr(&out).contains("model request failed"),
        "{}",
        stderr(&out)
    );
}
