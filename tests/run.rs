//! `clai run` as a process, against a local server that speaks the
//! `openai-compatible` streaming format (`support`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::fs;
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;

use support::*;

#[test]
fn answer_on_stdout() {
    let server = Server::start(vec![say("Hello there.")]);
    let env = Env::new("answer", &server, "");
    let out = env.run(&["say", "hello"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), "Hello there.\n");
    assert_eq!(stderr(&out), "");
    let requests = server.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0]["model"], "model-1");
    let messages = requests[0]["messages"].as_array().unwrap();
    assert_eq!(messages[0]["role"], "system");
    assert_eq!(messages[1]["content"], "say hello");
    assert_eq!(requests[0]["tools"].as_array().unwrap().len(), 6);
}

#[test]
fn tool_round_trip() {
    let server = Server::start(vec![
        call("c1", "read_file", json!({"path": "a.txt"})),
        say("It says hi."),
    ]);
    let env = Env::new("tools", &server, "");
    fs::write(env.work().join("a.txt"), "hi\n").unwrap();
    let out = env.run(&["run", "what", "is", "in", "a.txt?"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), "It says hi.\n");
    assert_eq!(stderr(&out), "✓ read_file a.txt (lines 1–1) — 1 line\n");
    let second = &server.requests()[1];
    let tool = second["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .unwrap();
    assert_eq!(tool["tool_call_id"], "c1");
    assert!(tool["content"].as_str().unwrap().contains("hi"));
}

fn write_call() -> Reply {
    call(
        "c1",
        "write_file",
        json!({"path": "new.txt", "content": "new\n"}),
    )
}

#[test]
fn asking_without_a_terminal_denies() {
    for args in [
        &["write", "it"][..],
        &["run", "--no-ask", "write", "it"][..],
    ] {
        let server = Server::start(vec![write_call(), say("I could not.")]);
        let env = Env::new("no-terminal", &server, "");
        let out = env.run(args);
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        assert!(!env.work().join("new.txt").exists());
        assert_eq!(
            stderr(&out),
            "⊘ write_file new.txt (1 line) — denied: needs permission, and no one can answer\n"
        );
        assert_eq!(stdout(&out), "I could not.\n");
    }
}

#[test]
fn ask_user_is_not_offered_without_a_terminal() {
    let ask =
        json!({"questions": [{"question": "Which?", "options": [{"label": "A"}, {"label": "B"}]}]});
    let server = Server::start(vec![call("c1", "ask_user", ask), say("I chose A.")]);
    let env = Env::new("no-ask-user", &server, "");
    let out = env.run(&["choose", "one"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out), "I chose A.\n");
    let requests = server.requests();
    let tools = requests[0]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 6);
    assert!(tools.iter().all(|t| t["function"]["name"] != "ask_user"));
    let tool = requests[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .unwrap();
    assert!(
        tool["content"]
            .as_str()
            .unwrap()
            .contains("ask_user is not available: no one can answer"),
        "{tool}"
    );
    assert!(stderr(&out).contains("ask_user"), "{}", stderr(&out));
}

#[test]
fn yes_allows() {
    let server = Server::start(vec![write_call(), say("Written.")]);
    let env = Env::new("yes", &server, "");
    let out = env.run(&["--yes", "write", "it"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        fs::read_to_string(env.work().join("new.txt")).unwrap(),
        "new\n"
    );
    assert_eq!(
        stderr(&out),
        "✓ write_file new.txt (1 line) — created new.txt (1 line)\n"
    );
}

#[test]
fn deny_rules_apply_with_yes() {
    let server = Server::start(vec![
        call("c1", "run_command", json!({"command": "rm -rf target"})),
        say("Denied."),
    ]);
    let env = Env::new(
        "deny-yes",
        &server,
        "[permissions.deny]\nrun_command = [\"rm\"]\n",
    );
    let out = env.run(&["--yes", "clean"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stderr(&out).contains("⊘ run_command rm -rf target — denied by permission rule"));
}

#[test]
fn secret_files_are_denied_with_yes() {
    let server = Server::start(vec![
        call("c1", "read_file", json!({"path": ".env"})),
        say("I cannot read it."),
    ]);
    let env = Env::new("secret-files", &server, "");
    fs::write(env.work().join(".env"), "PASSWORD=hunter2\n").unwrap();
    let out = env.run(&["--yes", "show", ".env"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stderr(&out)
            .contains("⊘ read_file .env — denied by permission rule: deny read(.env, .env.*)"),
        "{}",
        stderr(&out)
    );
    let second = &server.requests()[1];
    let tool = second["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .unwrap();
    let content = tool["content"].as_str().unwrap();
    assert!(
        content.contains("this file usually holds secrets"),
        "{content}"
    );
    assert!(!content.contains("hunter2"));
    // Nothing of the file reached the log either.
    let log = fs::read_to_string(env.root.join("state/clai/clai.log")).unwrap_or_default();
    assert!(!log.contains("hunter2"));
}

#[test]
fn task_from_stdin() {
    let server = Server::start(vec![say("ok")]);
    let env = Env::new("stdin", &server, "");
    let out = env.run_with_input(&["-"], "  fix the\nbuild\n");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let messages = server.requests()[0]["messages"].clone();
    assert_eq!(messages[1]["content"], "fix the\nbuild");

    let out = env.run_with_input(&["run", "-"], " \n");
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stderr(&out), "clai: the task is empty\n");
}

#[test]
fn usage_errors_exit_2() {
    let server = Server::start(vec![]);
    let env = Env::new("usage", &server, "");
    let out = env.run(&["--yes", "--no-ask", "x"]);
    assert_eq!(out.status.code(), Some(2));
    let out = env.run(&["run", "--model", "nope", "x"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("unknown model `nope`"),
        "{}",
        stderr(&out)
    );
    let out = env.run(&["run", "-C", "missing", "x"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("missing: not a directory"),
        "{}",
        stderr(&out)
    );
    assert!(server.requests().is_empty());
}

#[test]
fn provider_error_exits_1() {
    let server = Server::start(vec![Reply::Status(
        401,
        r#"{"error": {"message": "invalid key", "code": "invalid_api_key"}}"#.into(),
    )]);
    let env = Env::new("auth", &server, "");
    let out = env.run(&["hi"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).starts_with("clai: error: "),
        "{}",
        stderr(&out)
    );
    assert!(stderr(&out).contains("invalid key"), "{}", stderr(&out));
    assert_eq!(stdout(&out), "");
}

#[test]
fn limit_reached_exits_1() {
    let server = Server::start(vec![call("c1", "list_files", json!({})), say("never")]);
    let env = Env::new("limit", &server, "[limits]\nmax_requests = 1\n");
    let out = env.run(&["list"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stderr(&out).ends_with("clai: limit reached\n"),
        "{}",
        stderr(&out)
    );
    assert_eq!(server.requests().len(), 1);
}

#[test]
fn interrupt_exits_130() {
    let server = Server::start(vec![Reply::Stall]);
    let env = Env::new("interrupt", &server, "");
    let child = env
        .command(&["wait"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Wait until the request arrived, then press Ctrl-C.
    let started = Instant::now();
    while server.requests().is_empty() && started.elapsed() < Duration::from_secs(5) {
        thread::sleep(Duration::from_millis(20));
    }
    let pid = nix::unistd::Pid::from_raw(i32::try_from(child.id()).unwrap());
    nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGINT).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(out.status.code(), Some(130), "{}", stderr(&out));
    assert_eq!(stderr(&out), "clai: turn cancelled\n");
}
