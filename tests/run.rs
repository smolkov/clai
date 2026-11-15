//! `codeagent run` as a process, against a local server that speaks the
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
    let log =
        fs::read_to_string(env.root.join("state/codeagent/codeagent.log")).unwrap_or_default();
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
    assert_eq!(stderr(&out), "codeagent: the task is empty\n");
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
        stderr(&out).starts_with("codeagent: error: "),
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
        stderr(&out).ends_with("codeagent: limit reached\n"),
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
    assert_eq!(stderr(&out), "codeagent: turn cancelled\n");
}

// Reports (RFC 0027).

fn report_call(id: &str) -> Reply {
    call(
        id,
        "report",
        json!({
            "status": "partly_solved",
            "summary": "Found one problem.",
            "findings": [{"severity": "high", "title": "Secret in code",
                          "file": "a.txt", "line": 1, "detail": "Move it out."}],
            "follow_ups": ["rotate the key"]
        }),
    )
}

fn json_of(out: &std::process::Output) -> serde_json::Value {
    serde_json::from_str(&stdout(out)).unwrap_or_else(|e| panic!("{e}: {}", stdout(out)))
}

fn tool_names(request: &serde_json::Value) -> Vec<String> {
    request["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn report_as_json() {
    let server = Server::start(vec![
        call("c1", "search_files", json!({"pattern": "key"})),
        call("c2", "read_file", json!({"path": "a.txt"})),
        report_call("c3"),
        say("Done: one problem."),
    ]);
    let env = Env::new("report-json", &server, "");
    fs::write(env.work().join("a.txt"), "key=1\n").unwrap();
    let out = env.run(&["run", "--report=json", "find", "secrets"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let r = json_of(&out);
    assert_eq!(r["version"], 1);
    assert_eq!(r["outcome"], "done");
    assert_eq!(r["status"], "partly_solved");
    assert_eq!(r["summary"], "Found one problem.");
    assert_eq!(r["findings"][0]["severity"], "high");
    assert_eq!(r["findings"][0]["file"], "a.txt");
    assert_eq!(r["follow_ups"][0], "rotate the key");
    assert_eq!(r["model"], "test/m");
    assert_eq!(r["files_read"], json!(["a.txt"]));
    let calls = r["tool_calls"].as_array().unwrap();
    assert_eq!(calls.len(), 2, "report itself is left out: {calls:?}");
    assert_eq!(calls[0]["tool"], "search_files");
    assert_eq!(calls[1]["summary"], "a.txt (lines 1–1)");
    assert_eq!(calls[1]["result"], "1 line");
    assert_eq!(r["counts"]["done"], 2);
    assert_eq!(r["usage"]["requests"], 4);
    assert!(
        stderr(&out).contains("✓ report partly solved — 1 finding\n"),
        "{}",
        stderr(&out)
    );
    // The task in its frame, and the tool in every request.
    let requests = server.requests();
    let first = requests[0]["messages"][1]["content"].as_str().unwrap();
    assert!(first.contains("<task>\nfind secrets\n</task>"), "{first}");
    assert!(tool_names(&requests[0]).contains(&"report".to_owned()));
    assert!(requests[3]["messages"].as_array().unwrap().iter().any(|m| {
        m["role"] == "tool"
            && m["content"]
                .as_str()
                .unwrap()
                .starts_with("saved: 1 finding")
    }));
}

#[test]
fn report_as_text_from_a_pipe() {
    let server = Server::start(vec![report_call("c1"), say("Done.")]);
    let env = Env::new("report-text", &server, "");
    let out = env.run_with_input(&["run", "--report"], "find secrets\n");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.starts_with("# Partly solved\n\n"), "{text}");
    assert!(text.contains("\nFound one problem.\n"), "{text}");
    assert!(
        text.contains("- **high** Secret in code — `a.txt:1`\n  Move it out.\n"),
        "{text}"
    );
    assert!(
        text.contains("## Follow-ups\n\n- rotate the key\n"),
        "{text}"
    );
    let first = server.requests()[0]["messages"][1]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(first.contains("<task>\nfind secrets\n</task>"), "{first}");
}

#[test]
fn report_lists_denied_and_allowed_changes() {
    let script = || {
        vec![
            write_call(),
            call("c2", "run_command", json!({"command": "true"})),
            report_call("c3"),
            say("Done."),
        ]
    };
    // Without a terminal and without --yes: denied, no changes.
    let server = Server::start(script());
    let env = Env::new("report-denied", &server, "");
    let out = env.run(&["run", "--report=json", "fix", "it"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let r = json_of(&out);
    assert_eq!(r["tool_calls"][0]["state"], "denied");
    assert_eq!(r["changes"], json!([]));
    assert_eq!(r["commands"], json!([]));
    assert_eq!(r["counts"]["denied"], 2);
    assert!(!env.work().join("new.txt").exists());

    // With --yes: done.
    let server = Server::start(script());
    let env = Env::new("report-yes", &server, "");
    let out = env.run(&["run", "--yes", "--report=json", "fix", "it"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let r = json_of(&out);
    assert_eq!(
        r["changes"],
        json!([{"path": "new.txt", "kind": "written"}])
    );
    assert_eq!(r["commands"], json!([{"command": "true", "exit_code": 0}]));
}

#[test]
fn report_is_asked_for_once_more() {
    // No report in the work turn; one in the next.
    let server = Server::start(vec![say("All fine."), report_call("c1"), say("Done.")]);
    let env = Env::new("report-again", &server, "");
    let out = env.run(&["run", "--report=json", "check"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(json_of(&out)["status"], "partly_solved");
    let requests = server.requests();
    assert_eq!(requests.len(), 3);
    let messages = requests[1]["messages"].as_array().unwrap();
    assert!(
        messages.last().unwrap()["content"]
            .as_str()
            .unwrap()
            .starts_with("Call the report tool now")
    );

    // No report at all: status unknown, the answer as summary, a note.
    let server = Server::start(vec![say("All fine."), say("Still fine.")]);
    let env = Env::new("report-none", &server, "");
    let out = env.run(&["run", "--report=json", "check"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let r = json_of(&out);
    assert_eq!(r["status"], "unknown");
    assert_eq!(r["summary"], "All fine.");
    assert!(
        stderr(&out).contains("codeagent: the model gave no report; its status is unknown\n"),
        "{}",
        stderr(&out)
    );
}

#[test]
fn a_wrong_report_call_is_answered_with_the_problem() {
    let server = Server::start(vec![
        call("c1", "report", json!({"status": "great", "summary": "x"})),
        report_call("c2"),
        say("Done."),
    ]);
    let env = Env::new("report-wrong", &server, "");
    let out = env.run(&["run", "--report=json", "check"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(json_of(&out)["status"], "partly_solved");
    let tool = server.requests()[1]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["role"] == "tool")
        .unwrap()
        .clone();
    assert!(
        tool["content"]
            .as_str()
            .unwrap()
            .contains("status `great` is not one of"),
        "{tool}"
    );
}

#[test]
fn report_after_a_limit_or_an_error_exits_1() {
    let server = Server::start(vec![call("c1", "list_files", json!({})), say("never")]);
    let env = Env::new("report-limit", &server, "[limits]\nmax_requests = 1\n");
    let out = env.run(&["run", "--report=json", "list"]);
    assert_eq!(out.status.code(), Some(1));
    let r = json_of(&out);
    assert_eq!(r["outcome"], "limit");
    assert_eq!(r["status"], "unknown");
    assert_eq!(r["tool_calls"][0]["tool"], "list_files");

    let server = Server::start(vec![Reply::Status(
        401,
        r#"{"error": {"message": "invalid key", "code": "invalid_api_key"}}"#.into(),
    )]);
    let env = Env::new("report-failed", &server, "");
    let out = env.run(&["run", "--report=json", "hi"]);
    assert_eq!(out.status.code(), Some(1));
    let r = json_of(&out);
    assert_eq!(r["outcome"], "failed");
    assert_eq!(r["status"], "unknown");
    assert!(
        r["summary"].as_str().unwrap().contains("invalid key"),
        "{r}"
    );
}

#[test]
fn without_report_nothing_changes() {
    let server = Server::start(vec![say("Hi.")]);
    let env = Env::new("report-off", &server, "");
    // `json` after `--report` without `=` is a word of the task.
    let out = env.run(&["run", "--report", "json"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).starts_with("# Unknown"), "{}", stdout(&out));
    let first = server.requests()[0]["messages"][1]["content"]
        .as_str()
        .unwrap()
        .to_owned();
    assert!(first.contains("<task>\njson\n</task>"), "{first}");

    let server = Server::start(vec![say("Hi.")]);
    let env = Env::new("report-plain", &server, "");
    let out = env.run(&["run", "hi"]);
    assert_eq!(stdout(&out), "Hi.\n");
    assert!(!tool_names(&server.requests()[0]).contains(&"report".to_owned()));
}

#[test]
fn task_from_a_pipe_without_a_dash() {
    let server = Server::start(vec![say("ok")]);
    let env = Env::new("pipe", &server, "");
    let out = env.run_with_input(&["run"], "fix the build\n");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        server.requests()[0]["messages"][1]["content"],
        "fix the build"
    );
    // Standard input closed and empty: an empty task.
    let out = env.run(&["run"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stderr(&out), "codeagent: the task is empty\n");
    let out = env.run(&["run", "--report=xml", "x"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn old_directories_move_on_first_start() {
    // `codeagent` was `clai`: its config moves once, with a note.
    let server = Server::start(vec![say("Hi."), say("Again.")]);
    let env = Env::new("old-dirs", &server, "");
    fs::rename(
        env.root.join("config/codeagent"),
        env.root.join("config/clai"),
    )
    .unwrap();
    let out = env.run(&["run", "hi"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stderr(&out),
        "codeagent: moved ~/config/clai to ~/config/codeagent\n"
    );
    assert!(env.root.join("config/codeagent/config.toml").is_file());
    let out = env.run(&["run", "hi"]);
    assert_eq!(stderr(&out), "", "only once");
}
