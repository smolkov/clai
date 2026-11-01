//! `clai run` as a process, against a local server that speaks the
//! `openai-compatible` streaming format.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// What the server sends for one request.
#[derive(Clone)]
enum Reply {
    /// A streamed answer.
    Sse(String),
    /// An error status with a JSON body.
    Status(u16, String),
    /// The headers, then nothing until the client goes away.
    Stall,
}

/// A text answer.
fn say(text: &str) -> Reply {
    let chunk = json!({"choices": [{"index": 0, "delta": {"content": text}}]});
    let end = json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]});
    Reply::Sse(format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n"))
}

/// An answer with one tool call.
fn call(id: &str, name: &str, arguments: Value) -> Reply {
    let chunk = json!({"choices": [{"index": 0, "delta": {"tool_calls": [{
        "index": 0, "id": id, "type": "function",
        "function": {"name": name, "arguments": arguments.to_string()}
    }]}}]});
    let end = json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]});
    Reply::Sse(format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n"))
}

struct Server {
    port: u16,
    /// The request bodies received.
    seen: Arc<Mutex<Vec<Value>>>,
}

impl Server {
    fn start(replies: Vec<Reply>) -> Server {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = seen.clone();
        thread::spawn(move || {
            let mut replies = replies.into_iter();
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let reply = replies.next().unwrap_or_else(|| {
                    Reply::Status(500, "{\"error\": \"no more replies\"}".into())
                });
                let record = record.clone();
                thread::spawn(move || answer(stream, &reply, &record));
            }
        });
        Server { port, seen }
    }

    fn requests(&self) -> Vec<Value> {
        self.seen.lock().unwrap().clone()
    }
}

fn answer(mut stream: TcpStream, reply: &Reply, record: &Mutex<Vec<Value>>) {
    let mut data = Vec::new();
    let mut buf = [0u8; 8192];
    let head_end = loop {
        let n = stream.read(&mut buf).unwrap_or(0);
        if n == 0 {
            return;
        }
        data.extend_from_slice(&buf[..n]);
        if let Some(i) = data.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&data[..head_end]).to_ascii_lowercase();
    let length: usize = head
        .lines()
        .find_map(|l| l.strip_prefix("content-length:"))
        .map_or(0, |v| v.trim().parse().unwrap());
    while data.len() < head_end + length {
        let n = stream.read(&mut buf).unwrap_or(0);
        if n == 0 {
            break;
        }
        data.extend_from_slice(&buf[..n]);
    }
    if let Ok(body) = serde_json::from_slice(&data[head_end..]) {
        record.lock().unwrap().push(body);
    }
    let (status, kind, body) = match reply {
        Reply::Sse(body) => (200, "text/event-stream", body.clone()),
        Reply::Status(status, body) => (*status, "application/json", body.clone()),
        Reply::Stall => {
            let _ = stream.write_all(
                b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n",
            );
            thread::sleep(Duration::from_secs(30));
            return;
        }
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status} X\r\ncontent-type: {kind}\r\nconnection: close\r\n\r\n{body}"
    );
}

/// A home with a config for the server, and a workspace.
struct Env {
    root: PathBuf,
}

impl Env {
    fn new(name: &str, server: &Server, extra: &str) -> Env {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("run")
            .join(name);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("config/clai")).unwrap();
        fs::create_dir_all(root.join("work")).unwrap();
        let config = format!(
            r#"
default_model = "test/m"

[providers.test]
api = "openai-compatible"
base_url = "http://127.0.0.1:{}/v1"
max_retries = 0

[providers.test.models.m]
id = "model-1"
context_window = 100000

{extra}
"#,
            server.port
        );
        fs::write(root.join("config/clai/config.toml"), config).unwrap();
        Env { root }
    }

    fn work(&self) -> PathBuf {
        self.root.join("work")
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_clai"));
        cmd.args(args)
            .current_dir(self.work())
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("LANG", "C.UTF-8")
            .env_remove("LC_ALL")
            .env_remove("LC_CTYPE")
            .env_remove("CLAI_MODEL")
            .env_remove("CLAI_THEME");
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    fn run_with_input(&self, args: &[&str], input: &str) -> Output {
        let mut child = self
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

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
