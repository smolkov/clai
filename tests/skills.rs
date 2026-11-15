//! `codeagent skills` and `codeagent run --skill`, run as a process (RFC 0023).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::fs;
use std::path::Path;

use serde_json::Value;

use support::*;

fn skill(dir: &Path, name: &str, description: &str, body: &str) {
    let dir = dir.join(name);
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {description}\n---\n\n{body}\n"),
    )
    .unwrap();
}

/// A home with a user skill `pdf`, a team directory with `release-check`
/// and a workspace skill `pdf` that wins over the user one.
fn env(name: &str, server: &Server) -> Env {
    let probe = Env::new(name, server, "");
    let team = probe.root.join("team");
    let env = Env::new(
        name,
        server,
        &format!("[skills]\ndirs = [{:?}]\n", team.display().to_string()),
    );
    skill(
        &team,
        "release-check",
        "Steps before a release.",
        "Run the tests.",
    );
    skill(
        &env.root.join(".claude/skills"),
        "pdf",
        "Read PDF files.",
        "Use pdftotext.",
    );
    skill(
        &env.work().join(".agents/skills"),
        "pdf",
        "Project PDF.",
        "Project rules.",
    );
    fs::create_dir_all(env.root.join(".agents/skills/old")).unwrap();
    fs::write(
        env.root.join(".agents/skills/old/SKILL.md"),
        "---\nname: old\n---\n",
    )
    .unwrap();
    env
}

#[test]
fn list_as_table() {
    let server = Server::start(vec![]);
    let env = env("table", &server);
    let out = env.run(&["skills"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "release-check  config   ~/team/release-check  Steps before a release.\n\
         pdf            project  .agents/skills/pdf    Project PDF.\n\
         pdf            user     ~/.claude/skills/pdf  shadowed by .agents/skills/pdf\n"
    );
    assert_eq!(
        stderr(&out),
        "codeagent: warning: ~/.agents/skills/old/SKILL.md: no description\n"
    );
}

#[test]
fn list_as_json() {
    let server = Server::start(vec![]);
    let env = env("json", &server);
    let out = env.run(&["skills", "--json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let value: Value = serde_json::from_str(&stdout(&out)).unwrap();
    let skills = value["skills"].as_array().unwrap();
    assert_eq!(skills.len(), 3);
    assert_eq!(skills[0]["name"], "release-check");
    assert_eq!(skills[0]["source"], "config");
    assert_eq!(skills[1]["shadowed_by"], Value::Null);
    assert_eq!(
        skills[2]["shadowed_by"].as_str().unwrap(),
        env.work().join(".agents/skills/pdf").to_str().unwrap()
    );
    assert_eq!(value["dirs"].as_array().unwrap().len(), 7);
    assert_eq!(value["warnings"].as_array().unwrap().len(), 1);
}

#[test]
fn nothing_found() {
    let server = Server::start(vec![]);
    let env = Env::new("nothing", &server, "");
    let out = env.run(&["skills"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(
        text.starts_with("no skills found; searched:\n  .opencode/skills\n"),
        "{text}"
    );
    assert!(text.contains("  ~/config/opencode/skills\n"), "{text}");
}

#[test]
fn run_with_a_skill_puts_its_text_into_the_prompt() {
    let server = Server::start(vec![say("Done.")]);
    let env = env("run", &server);
    let out = env.run(&[
        "run",
        "--skill",
        "release-check",
        "--skill",
        "pdf",
        "check it",
    ]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let requests = server.requests();
    let system = requests[0]["messages"][0]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    let release = system.find("<skill name=\"release-check\"").unwrap();
    let pdf = system.find("<skill name=\"pdf\"").unwrap();
    assert!(release < pdf, "{system}");
    assert!(system.contains("Run the tests.\n</skill>"), "{system}");
    // The workspace's pdf wins over the user one.
    assert!(system.contains("Project rules."), "{system}");
    assert!(!system.contains("Use pdftotext."), "{system}");
    assert!(system.contains("(read only)"), "{system}");
}

#[test]
fn run_with_an_unknown_skill_is_a_usage_error() {
    let server = Server::start(vec![]);
    let env = env("unknown", &server);
    let out = env.run(&["run", "--skill", "nope", "task"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        stderr(&out),
        "codeagent: unknown skill `nope`; found: release-check, pdf\n"
    );
    assert!(server.requests().is_empty());
}
