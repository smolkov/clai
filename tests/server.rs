//! The server and the session commands as processes: `clai new -d`, `ls`,
//! `send`, `chat`, `kill`, `server`, against a local server that speaks the
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
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// A text answer.
fn say(text: &str) -> String {
    let chunk = json!({"choices": [{"index": 0, "delta": {"content": text}}]});
    let end = json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]});
    format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n")
}

/// An answer that writes a file.
fn write_file(id: &str, path: &str) -> String {
    let arguments = json!({"path": path, "content": "hi\n"}).to_string();
    let chunk = json!({"choices": [{"index": 0, "delta": {"tool_calls": [{
        "index": 0, "id": id, "type": "function",
        "function": {"name": "write_file", "arguments": arguments}
    }]}}]});
    let end = json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]});
    format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n")
}

/// Answers each request with the next streamed reply.
fn model_server(replies: Vec<String>) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    thread::spawn(move || {
        let mut replies = replies.into_iter();
        for stream in listener.incoming() {
            let Ok(stream) = stream else { return };
            let reply = replies.next().unwrap_or_else(|| say("no more replies"));
            thread::spawn(move || answer(stream, &reply));
        }
    });
    port
}

fn answer(mut stream: TcpStream, body: &str) {
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
    let _ = write!(
        stream,
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{body}"
    );
}

/// A home with a config, a workspace and a private runtime directory.
/// Stops its server when dropped.
struct Env {
    root: PathBuf,
}

impl Env {
    fn new(name: &str, port: u16) -> Env {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("srv")
            .join(name);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("config/clai")).unwrap();
        fs::create_dir_all(root.join("ws")).unwrap();
        let config = format!(
            r#"
default_model = "test/m"

[providers.test]
api = "openai-compatible"
base_url = "http://127.0.0.1:{port}/v1"
max_retries = 0

[providers.test.models.m]
id = "model-1"
context_window = 100000
"#
        );
        fs::write(root.join("config/clai/config.toml"), config).unwrap();
        Env { root }
    }

    fn ws(&self) -> PathBuf {
        self.root.join("ws")
    }

    fn command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_clai"));
        cmd.args(args)
            .current_dir(self.ws())
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env("XDG_RUNTIME_DIR", self.root.join("run"))
            .env("LANG", "C.UTF-8")
            .env_remove("LC_ALL")
            .env_remove("LC_CTYPE")
            .env_remove("CLAI_MODEL")
            .env_remove("CLAI_THEME");
        cmd
    }

    fn run(&self, args: &[&str]) -> Output {
        self.command(args).stdin(Stdio::null()).output().unwrap()
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

    /// `clai ls --json`.
    fn sessions(&self) -> Vec<Value> {
        let out = self.run(&["ls", "--json"]);
        assert_eq!(out.status.code(), Some(0), "{}", text(&out.stderr));
        serde_json::from_slice::<Vec<Value>>(&out.stdout).unwrap()
    }

    /// Waits until session 0 satisfies `f`.
    fn wait_for(&self, what: &str, f: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let sessions = self.sessions();
            if let Some(s) = sessions.first()
                && f(s)
            {
                return s.clone();
            }
            assert!(
                Instant::now() < deadline,
                "waiting for {what}: {sessions:?}"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = self.run(&["server", "stop", "--force"]);
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn code(out: &Output) -> Option<i32> {
    out.status.code()
}

#[test]
fn sessions_through_the_server() {
    let port = model_server(vec![
        write_file("c1", "a.txt"),
        say("wrote a.txt"),
        write_file("c2", "b.txt"),
        say("wrote b.txt"),
    ]);
    let env = Env::new("sessions", port);

    let out = env.run(&["server", "status"]);
    assert_eq!(code(&out), Some(1));
    assert_eq!(text(&out.stdout), "not running\n");
    assert_eq!(code(&env.run(&["server", "stop"])), Some(1));

    // The first session command starts the server.
    let out = env.run(&["new", "-d", "--name", "s"]);
    assert_eq!(code(&out), Some(0), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "s\n");
    let out = env.run(&["new", "-d"]);
    assert_eq!(text(&out.stdout), "ws\n");
    assert!(
        text(&out.stderr).contains("s also works in"),
        "{}",
        text(&out.stderr)
    );
    let out = env.run(&["new", "-d", "--name", "s"]);
    assert_eq!(code(&out), Some(1));
    assert_eq!(text(&out.stderr), "clai: the name s is taken\n");
    let out = env.run(&["new", "-d", "--model", "nope"]);
    assert_eq!(code(&out), Some(2));

    let out = env.run(&["ls"]);
    let ls = text(&out.stdout);
    assert!(ls.starts_with("s   "), "{ls}");
    assert!(ls.contains("test/m  idle"), "{ls}");

    let out = env.run(&["server"]);
    assert_eq!(code(&out), Some(1));
    assert_eq!(text(&out.stderr), "clai: a server is already running\n");
    let out = env.run(&["server", "status"]);
    assert_eq!(code(&out), Some(0));
    assert!(
        text(&out.stdout).starts_with("running, pid "),
        "{}",
        text(&out.stdout)
    );

    // A question, answered by a script.
    assert_eq!(code(&env.run(&["send", "s", "write", "a.txt"])), Some(0));
    let s = env.wait_for("a question", |s| !s["question"].is_null());
    assert_eq!(s["state"], "waiting");
    let id = s["question"]["id"].as_str().unwrap().to_owned();
    let out = env.run(&["chat", "--list"]);
    let list = text(&out.stdout);
    assert!(list.starts_with(&format!("s  {id}  ")), "{list}");
    assert!(list.contains("write_file: a.txt"), "{list}");
    let out = env.run(&["chat", "--list", "--json"]);
    let questions: Vec<Value> = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(questions[0]["details"]["tool"], "write_file");

    let out = env.run(&["chat", "s", "allow", "--id", "q999"]);
    assert_eq!(code(&out), Some(1));
    assert_eq!(
        text(&out.stderr),
        format!("clai: the open question of s is {id}, not q999\n")
    );
    let out = env.run(&["chat", "s", "continue"]);
    assert_eq!(code(&out), Some(1));
    assert!(
        text(&out.stderr).contains("not an answer"),
        "{}",
        text(&out.stderr)
    );
    let out = env.run(&["chat", "s", "allow", "--id", &id]);
    assert_eq!(code(&out), Some(0), "{}", text(&out.stderr));
    env.wait_for("the turn to end", |s| s["state"] == "idle");
    assert_eq!(fs::read_to_string(env.ws().join("a.txt")).unwrap(), "hi\n");
    let out = env.run(&["chat", "s", "allow"]);
    assert_eq!(code(&out), Some(1));
    assert_eq!(text(&out.stderr), "clai: s has no open question\n");

    // A question, answered in interactive `clai chat`.
    // An empty message is a usage error.
    assert_eq!(code(&env.run(&["send", "s", "-"])), Some(2));
    let out = env.run_with_input(&["send", "s", "-"], "write b.txt\n");
    assert_eq!(code(&out), Some(0), "{}", text(&out.stderr));
    env.wait_for("a question", |s| !s["question"].is_null());
    let out = env.run_with_input(&["chat", "s"], "x\ny\n");
    assert_eq!(code(&out), Some(0), "{}", text(&out.stderr));
    let shown = text(&out.stdout);
    assert!(shown.starts_with("── s · "), "{shown}");
    assert!(shown.contains("new file b.txt (1 lines)"), "{shown}");
    assert!(shown.contains("answer y, a, n, e or c"), "{shown}");
    env.wait_for("the turn to end", |s| s["state"] == "idle");
    assert!(env.ws().join("b.txt").exists());

    // Kill: unknown names fail, the known ones still end.
    let out = env.run(&["kill", "ws", "nope"]);
    assert_eq!(code(&out), Some(1));
    assert_eq!(text(&out.stderr), "clai: no session named nope\n");
    assert_eq!(env.sessions().len(), 1);
    assert_eq!(code(&env.run(&["kill", "--all"])), Some(0));
    assert!(env.sessions().is_empty());

    let out = env.run(&["server", "stop"]);
    assert_eq!(code(&out), Some(0), "{}", text(&out.stderr));
    assert_eq!(code(&env.run(&["server", "status"])), Some(1));
}

#[test]
fn invalid_config_does_not_start_a_server() {
    let env = Env::new("invalid", 9);
    fs::write(
        env.root.join("config/clai/config.toml"),
        "default_model = 3\n",
    )
    .unwrap();
    let out = env.run(&["ls"]);
    assert_eq!(code(&out), Some(2), "{}", text(&out.stderr));
    assert_eq!(code(&env.run(&["server", "status"])), Some(1));
}
