//! Shared by the process tests: a local server that speaks the
//! `openai-compatible` streaming format, and a home with a config for it.

#![allow(
    dead_code,
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
use std::time::Duration;

use serde_json::{Value, json};

/// What the server sends for one request.
#[derive(Clone)]
pub enum Reply {
    /// A streamed answer.
    Sse(String),
    /// An error status with a JSON body.
    Status(u16, String),
    /// The headers, then nothing until the client goes away.
    Stall,
}

/// A text answer.
pub fn say(text: &str) -> Reply {
    let chunk = json!({"choices": [{"index": 0, "delta": {"content": text}}]});
    let end = json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "stop"}]});
    Reply::Sse(format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n"))
}

/// An answer with one tool call.
pub fn call(id: &str, name: &str, arguments: Value) -> Reply {
    let chunk = json!({"choices": [{"index": 0, "delta": {"tool_calls": [{
        "index": 0, "id": id, "type": "function",
        "function": {"name": name, "arguments": arguments.to_string()}
    }]}}]});
    let end = json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]});
    Reply::Sse(format!("data: {chunk}\n\ndata: {end}\n\ndata: [DONE]\n\n"))
}

pub struct Server {
    pub port: u16,
    /// The request bodies received.
    seen: Arc<Mutex<Vec<Value>>>,
}

impl Server {
    pub fn start(replies: Vec<Reply>) -> Server {
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

    pub fn requests(&self) -> Vec<Value> {
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
pub struct Env {
    pub root: PathBuf,
}

impl Env {
    pub fn new(name: &str, server: &Server, extra: &str) -> Env {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join(env!("CARGO_CRATE_NAME"))
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

    pub fn work(&self) -> PathBuf {
        self.root.join("work")
    }

    pub fn command(&self, args: &[&str]) -> Command {
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

    pub fn run(&self, args: &[&str]) -> Output {
        self.command(args).output().unwrap()
    }

    pub fn run_with_input(&self, args: &[&str], input: &str) -> Output {
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

pub fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

pub fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}
