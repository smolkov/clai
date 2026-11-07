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
use std::process::{Child, ChildStdin, Command, Output, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// A text answer.
fn say(text: &str) -> String {
    let chunk = json!({"choices": [{"index": 0, "delta": {"content": text}}]});
    let end = json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]});
    format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n")
}

/// A text answer that reports its tokens.
fn say_counted(text: &str, input: u64, output: u64) -> String {
    let chunk = json!({"choices": [{"index": 0, "delta": {"content": text}}]});
    let end = json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]});
    let usage = json!({"choices": [],
        "usage": {"prompt_tokens": input, "completion_tokens": output}});
    format!("data: {chunk}\n\ndata: {end}\n\ndata: {usage}\n\ndata: [DONE]\n\n")
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

    // A question, answered in `clai chat --questions`.
    // An empty message is a usage error.
    assert_eq!(code(&env.run(&["send", "s", "-"])), Some(2));
    let out = env.run_with_input(&["send", "s", "-"], "write b.txt\n");
    assert_eq!(code(&out), Some(0), "{}", text(&out.stderr));
    env.wait_for("a question", |s| !s["question"].is_null());
    let out = env.run_with_input(&["chat", "--questions", "s"], "x\ny\n");
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

/// A running `clai chat`: lines go to its standard input; standard output
/// and error are collected together.
struct Live {
    child: Child,
    stdin: Option<ChildStdin>,
    seen: Arc<Mutex<String>>,
}

impl Live {
    fn start(env: &Env, args: &[&str]) -> Live {
        let mut child = env
            .command(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let seen = Arc::new(Mutex::new(String::new()));
        let mut pipes: Vec<Box<dyn Read + Send>> = vec![
            Box::new(child.stdout.take().unwrap()),
            Box::new(child.stderr.take().unwrap()),
        ];
        for mut pipe in pipes.drain(..) {
            let seen = seen.clone();
            thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = pipe.read(&mut buf) {
                    if n == 0 {
                        return;
                    }
                    seen.lock()
                        .unwrap()
                        .push_str(&String::from_utf8_lossy(&buf[..n]));
                }
            });
        }
        let stdin = child.stdin.take();
        Live { child, stdin, seen }
    }

    fn send(&mut self, line: &str) {
        let stdin = self.stdin.as_mut().unwrap();
        stdin.write_all(format!("{line}\n").as_bytes()).unwrap();
        stdin.flush().unwrap();
    }

    /// Waits until the output contains `text`.
    fn expect(&self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while !self.seen.lock().unwrap().contains(text) {
            assert!(
                Instant::now() < deadline,
                "waiting for {text:?} in:\n{}",
                self.seen.lock().unwrap()
            );
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// Closes standard input (Ctrl-D) and waits for the exit.
    fn finish(mut self) -> (Option<i32>, String) {
        drop(self.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(10);
        let status = loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() > deadline {
                let _ = self.child.kill();
                panic!("clai chat did not exit:\n{}", self.seen.lock().unwrap());
            }
            thread::sleep(Duration::from_millis(10));
        };
        thread::sleep(Duration::from_millis(50));
        let seen = self.seen.lock().unwrap().clone();
        (status.code(), seen)
    }
}

#[test]
fn chat_with_a_session() {
    let port = model_server(vec![
        write_file("c1", "c.txt"),
        say("I wrote c.txt."),
        say("first"),
        say("second"),
    ]);
    let env = Env::new("chat", port);

    // `clai chat` starts the server and a new session in this directory.
    let mut chat = Live::start(&env, &["chat"]);
    chat.expect("clai: attached to ws (");
    chat.send("write c.txt please");
    chat.expect("[y] allow  [a] always");
    chat.expect("? ");
    chat.send("z");
    chat.expect("answer y, a, n, e or c");
    chat.send("y");
    chat.expect("I wrote c.txt.");
    chat.expect("✓ write_file c.txt");
    chat.send("/model");
    chat.expect("clai: model: test/m");
    chat.send("/nope");
    chat.expect("unknown command /nope");
    let (status, seen) = chat.finish();
    assert_eq!(status, Some(0), "{seen}");
    assert!(
        seen.contains("clai: detached; `clai chat ws` returns to ws"),
        "{seen}"
    );
    assert_eq!(fs::read_to_string(env.ws().join("c.txt")).unwrap(), "hi\n");
    // The user's own message is not echoed back.
    assert!(!seen.contains("> write c.txt please"), "{seen}");

    // The session keeps running; attaching again shows its history.
    assert_eq!(env.sessions().len(), 1);
    let mut chat = Live::start(&env, &["chat", "ws"]);
    chat.expect("> write c.txt please\n");
    chat.expect("✓ write_file c.txt");
    chat.expect("I wrote c.txt.\n> ");
    // Two messages: the second is queued until the first turn ends.
    chat.send("one");
    chat.send("two");
    chat.expect("first");
    chat.expect("second");
    chat.send("/kill");
    chat.expect("session ws ended");
    let (status, seen) = chat.finish();
    assert_eq!(status, Some(0), "{seen}");
    assert!(env.sessions().is_empty());

    let out = env.run(&["chat", "nope"]);
    assert_eq!(code(&out), Some(1));
    assert_eq!(text(&out.stderr), "clai: no session named nope\n");
    let out = env.run(&["chat", "a", "b"]);
    assert_eq!(code(&out), Some(2));
}

#[test]
fn chat_switches_sessions() {
    let port = model_server(vec![say("in other")]);
    let env = Env::new("chat-switch", port);
    let mut chat = Live::start(&env, &["chat"]);
    chat.expect("clai: attached to ws (");
    chat.send("/models");
    chat.expect("test/m");
    chat.send("/new other test/m");
    chat.expect("clai: attached to other (");
    chat.send("hello other");
    chat.expect("in other");
    chat.send("/sessions");
    chat.expect("* other");
    chat.send("/attach nope");
    chat.expect("no session named nope");
    chat.send("/attach ws");
    chat.expect("clai: attached to ws (");
    chat.send("/attach other");
    // The history of the session it comes back to.
    chat.expect("> hello other");
    chat.expect("in other");
    chat.send("/new a b");
    chat.expect("usage: /new [NAME] [PROVIDER/MODEL]");
    let (status, seen) = chat.finish();
    assert_eq!(status, Some(0), "{seen}");
    assert!(
        seen.contains("`clai chat other` returns to other"),
        "{seen}"
    );
    let mut names: Vec<String> = env
        .sessions()
        .iter()
        .map(|s| s["name"].as_str().unwrap().to_owned())
        .collect();
    names.sort();
    assert_eq!(names, ["other", "ws"]);
}

#[test]
fn sessions_are_saved_and_resumed() {
    let port = model_server(vec![say("noted: 42"), say("it was 42")]);
    let env = Env::new("saved", port);

    let mut chat = Live::start(&env, &["chat", "--new", "--name", "s"]);
    chat.expect("clai: attached to s (");
    chat.send("remember 42");
    chat.expect("noted: 42");
    let (status, seen) = chat.finish();
    assert_eq!(status, Some(0), "{seen}");

    let out = env.run(&["server", "stop"]);
    assert_eq!(code(&out), Some(0), "{}", text(&out.stderr));
    let files = fs::read_dir(env.root.join("state/clai/sessions"))
        .unwrap()
        .count();
    assert_eq!(files, 1);

    // A new server lists it as saved; chat resumes it with its history.
    let out = env.run(&["ls"]);
    assert!(
        text(&out.stdout).contains("test/m  saved"),
        "{}",
        text(&out.stdout)
    );
    let mut chat = Live::start(&env, &["chat", "s"]);
    chat.expect("> remember 42\n");
    chat.expect("noted: 42\n> ");
    chat.send("what was it?");
    chat.expect("it was 42");
    let (status, seen) = chat.finish();
    assert_eq!(status, Some(0), "{seen}");

    // Export and import.
    let file = env.root.join("s.json");
    let out = env.run(&["export", "s", file.to_str().unwrap()]);
    assert_eq!(code(&out), Some(0), "{}", text(&out.stderr));
    let out = env.run(&["import", file.to_str().unwrap()]);
    assert_eq!(code(&out), Some(0), "{}", text(&out.stderr));
    assert_eq!(text(&out.stdout), "s-2\n");
    let out = env.run(&["chat", "--list"]);
    assert_eq!(code(&out), Some(0));
    let sessions = env.sessions();
    assert_eq!(sessions.len(), 2);
    assert_eq!(sessions[1]["state"], "saved");

    // Kill deletes the files.
    assert_eq!(code(&env.run(&["kill", "--all"])), Some(0));
    let files = fs::read_dir(env.root.join("state/clai/sessions"))
        .unwrap()
        .count();
    assert_eq!(files, 0);
}

#[test]
fn token_totals_in_ls_and_chat() {
    let port = model_server(vec![
        say_counted("first", 45_000, 3_000),
        say_counted("second", 100, 200),
    ]);
    let env = Env::new("totals", port);

    let mut chat = Live::start(&env, &["chat", "--new", "--name", "s"]);
    chat.expect("clai: attached to s (");
    chat.send("/usage");
    chat.expect("clai: tokens: in 0, out 0; context 0");
    chat.send("one");
    chat.expect("first");
    chat.send("two");
    chat.expect("second");
    let s = env.wait_for("the turn to end", |s| s["state"] == "idle");
    assert_eq!(s["input_tokens"], 45_100);
    assert_eq!(s["output_tokens"], 3_200);
    chat.send("/usage");
    chat.expect("clai: tokens: in 45.1k, out 3.2k; context 300");
    let (status, seen) = chat.finish();
    assert_eq!(status, Some(0), "{seen}");

    // After attaching again, the replayed log gives the same count.
    let mut chat = Live::start(&env, &["chat", "s"]);
    chat.expect("second");
    chat.send("/usage");
    chat.expect("clai: tokens: in 45.1k, out 3.2k; context 300");
    let (status, seen) = chat.finish();
    assert_eq!(status, Some(0), "{seen}");

    // The table stays as it is.
    let out = env.run(&["ls"]);
    let table = text(&out.stdout);
    assert!(
        !table.contains("45.1k") && !table.contains("45100"),
        "{table}"
    );
    let out = env.run(&["kill", "s"]);
    assert_eq!(code(&out), Some(0), "{}", text(&out.stderr));
}
