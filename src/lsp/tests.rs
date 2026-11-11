//! Transport, server life, sync and positions, with a language server in
//! the same process over a `tokio` duplex stream.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use serde_json::{Value, json};
use tokio::io::{AsyncWriteExt, BufReader, DuplexStream, ReadHalf, WriteHalf, duplex, split};

use super::Error;
use super::docs::*;
use super::server::{Server, Setup, State};
use super::transport::*;

// ---------------------------------------------------------------- framing

fn frame(body: &str) -> String {
    format!("Content-Length: {}\r\n\r\n{body}", body.len())
}

async fn read_all(bytes: &[u8]) -> Vec<Result<Option<Value>, Error>> {
    let mut reader = BufReader::new(bytes);
    let mut out = Vec::new();
    loop {
        let message = read_message(&mut reader).await;
        let end = !matches!(message, Ok(Some(_)));
        out.push(message);
        if end {
            return out;
        }
    }
}

#[tokio::test]
async fn joined_messages_are_read_one_by_one() {
    let bytes = format!("{}{}", frame(r#"{"a":1}"#), frame(r#"{"b":2}"#));
    let read = read_all(bytes.as_bytes()).await;
    assert_eq!(
        read,
        vec![
            Ok(Some(json!({"a": 1}))),
            Ok(Some(json!({"b": 2}))),
            Ok(None)
        ]
    );
}

#[tokio::test]
async fn headers_are_case_insensitive_and_others_ignored() {
    let bytes =
        "content-length: 2\r\nContent-Type: application/vscode-jsonrpc; charset=utf-8\r\n\r\n{}";
    let read = read_all(bytes.as_bytes()).await;
    assert_eq!(read, vec![Ok(Some(json!({}))), Ok(None)]);
}

#[tokio::test]
async fn a_message_split_in_pieces_is_read_whole() {
    let (mut a, b) = duplex(64);
    let bytes = frame(r#"{"text":"hello, world"}"#);
    let writer = tokio::spawn(async move {
        for chunk in bytes.as_bytes().chunks(5) {
            a.write_all(chunk).await.unwrap();
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    });
    let mut reader = BufReader::new(b);
    let message = read_message(&mut reader).await;
    assert_eq!(message, Ok(Some(json!({"text": "hello, world"}))));
    writer.await.unwrap();
}

fn malformed(bytes: &str) -> String {
    let rt = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let read = rt.block_on(read_all(bytes.as_bytes()));
    match read.last() {
        Some(Err(Error::Closed(reason))) => reason.clone(),
        other => panic!("{bytes:?}: {other:?}"),
    }
}

#[test]
fn bad_headers_and_bodies_are_malformed() {
    assert!(malformed("Bogus\r\n\r\n").contains("header \"Bogus\""));
    assert!(malformed("Content-Type: x\r\n\r\n{}").contains("no Content-Length"));
    assert!(malformed("Content-Length: ten\r\n\r\n").contains("Content-Length \"ten\""));
    let long = format!("X-Long: {}\r\n\r\n", "x".repeat(MAX_HEADER_LINE));
    assert!(malformed(&long).contains("header line over"));
    let big = format!("Content-Length: {}\r\n\r\n", MAX_BODY + 1);
    assert!(malformed(&big).contains("over"));
    assert!(malformed("Content-Length: 5\r\n\r\n{}").contains("ended in a message body"));
    assert!(malformed("Content-Length: 3\r\n\r\nabc").contains("malformed message"));
    assert!(malformed("Content-Length: 2\r\n").contains("ended in a header"));
}

// ------------------------------------------------------------- connection

/// Both ends of a connection over a duplex stream.
fn connected() -> (
    (Connection, tokio::sync::mpsc::UnboundedReceiver<Incoming>),
    (Connection, tokio::sync::mpsc::UnboundedReceiver<Incoming>),
) {
    let (a, b) = duplex(1 << 20);
    let (ar, aw) = split(a);
    let (br, bw) = split(b);
    (Connection::new(ar, aw), Connection::new(br, bw))
}

async fn next_request(
    rx: &mut tokio::sync::mpsc::UnboundedReceiver<Incoming>,
) -> (Value, String, Value) {
    match rx.recv().await {
        Some(Incoming::Request { id, method, params }) => (id, method, params),
        other => panic!("expected a request: {other:?}"),
    }
}

const SECOND: Duration = Duration::from_secs(1);

#[tokio::test]
async fn responses_find_their_requests_by_id() {
    let ((client, _), (server, mut requests)) = connected();
    let first = client.request("first", json!(1), SECOND);
    let second = client.request("second", json!(2), SECOND);
    let answer = async {
        let (id1, m1, _) = next_request(&mut requests).await;
        let (id2, m2, _) = next_request(&mut requests).await;
        assert_eq!((m1.as_str(), m2.as_str()), ("first", "second"));
        assert_ne!(id1, id2);
        // Answered in the other order.
        server.respond(id2, Ok(json!("two"))).unwrap();
        server.respond(id1, Err((-32000, "no".into()))).unwrap();
    };
    let (first, second, ()) = tokio::join!(first, second, answer);
    assert_eq!(second, Ok(json!("two")));
    assert_eq!(
        first,
        Err(Error::Server {
            code: -32000,
            message: "no".into()
        })
    );
}

#[tokio::test]
async fn notifications_and_server_requests_reach_the_receiver() {
    let ((client, mut incoming), (server, _)) = connected();
    server.notify("note", json!({"x": 1})).unwrap();
    let reply = tokio::spawn({
        let server = server.clone();
        async move { server.request("ask", json!([]), SECOND).await }
    });
    assert_eq!(
        incoming.recv().await,
        Some(Incoming::Notification {
            method: "note".into(),
            params: json!({"x": 1})
        })
    );
    let (id, method, _) = next_request(&mut incoming).await;
    assert_eq!(method, "ask");
    client.respond(id, Ok(Value::Null)).unwrap();
    assert_eq!(reply.await.unwrap(), Ok(Value::Null));
}

#[tokio::test]
async fn a_timeout_cancels_the_request() {
    let ((client, _), (_server, mut incoming)) = connected();
    let answer = client
        .request("slow", Value::Null, Duration::from_millis(50))
        .await;
    assert_eq!(answer, Err(Error::Timeout(1)));
    let (id, _, _) = next_request(&mut incoming).await;
    assert_eq!(
        incoming.recv().await,
        Some(Incoming::Notification {
            method: "$/cancelRequest".into(),
            params: json!({"id": id})
        })
    );
}

#[tokio::test]
async fn a_dropped_request_is_cancelled() {
    let ((client, _), (_server, mut incoming)) = connected();
    let request = tokio::spawn({
        let client = client.clone();
        async move { client.request("slow", Value::Null, SECOND).await }
    });
    let (id, _, _) = next_request(&mut incoming).await;
    request.abort();
    let _ = request.await;
    match incoming.recv().await {
        Some(Incoming::Notification { method, params }) => {
            assert_eq!(method, "$/cancelRequest");
            assert_eq!(params, json!({"id": id}));
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_closed_stream_fails_waiting_and_later_requests() {
    let ((client, mut incoming), (server, mut requests)) = connected();
    let waiting = tokio::spawn({
        let client = client.clone();
        async move { client.request("x", Value::Null, SECOND).await }
    });
    next_request(&mut requests).await;
    drop(server);
    drop(requests);
    let closed: Result<Value, Error> =
        Err(Error::Closed("the server closed the connection".into()));
    assert_eq!(waiting.await.unwrap(), closed);
    assert_eq!(client.request("y", Value::Null, SECOND).await, closed);
    assert_eq!(client.notify("z", Value::Null), closed.map(|_: Value| ()));
    assert_eq!(incoming.recv().await, None);
}

#[tokio::test]
async fn a_malformed_message_closes_the_connection() {
    let (a, mut b) = duplex(1 << 16);
    let (ar, aw) = split(a);
    let (client, _incoming) = Connection::new(ar, aw);
    let waiting = tokio::spawn({
        let client = client.clone();
        async move { client.request("x", Value::Null, SECOND).await }
    });
    b.write_all(frame(r#"{"jsonrpc":"2.0"}"#).as_bytes())
        .await
        .unwrap();
    match waiting.await.unwrap() {
        Err(Error::Closed(reason)) => assert!(reason.contains("neither request"), "{reason}"),
        other => panic!("{other:?}"),
    }
    assert!(client.closed().is_some());
}

// ---------------------------------------------------------------- positions

#[test]
fn symbols_are_found_as_whole_words() {
    let line = "let parsed = parse(parse_all(x)); // parse";
    assert_eq!(find_symbol(line, "parse", 1), Some(13));
    assert_eq!(find_symbol(line, "parse", 2), Some(37));
    assert_eq!(find_symbol(line, "parse", 3), None);
    assert_eq!(find_symbol(line, "parse_all", 1), Some(19));
    assert_eq!(find_symbol(line, "pars", 1), None);
    assert_eq!(find_symbol(line, "x", 1), Some(29));
    assert_eq!(find_symbol(line, "parse", 0), None);
    assert_eq!(find_symbol(line, "", 1), None);
    // Operators match without word boundaries.
    assert_eq!(find_symbol("a+=b", "+=", 1), Some(1));
}

#[test]
fn symbols_after_unicode_count_characters() {
    let line = "let größe = \"😀\"; größe";
    assert_eq!(find_symbol(line, "größe", 1), Some(4));
    assert_eq!(find_symbol(line, "größe", 2), Some(17));
    // An umlaut is a word character: no match inside a word.
    assert_eq!(find_symbol("äb b", "b", 1), Some(3));
}

#[test]
fn columns_convert_to_and_from_the_server_encoding() {
    let line = "a😀ä b";
    // Columns: a=0, 😀=1, ä=2, ' '=3, b=4.
    assert_eq!(to_lsp_character(line, 4, Encoding::Utf16), 5);
    assert_eq!(to_lsp_character(line, 4, Encoding::Utf8), 8);
    assert_eq!(to_lsp_character(line, 4, Encoding::Utf32), 4);
    assert_eq!(to_lsp_character(line, 99, Encoding::Utf16), 6);
    assert_eq!(from_lsp_character(line, 5, Encoding::Utf16), 4);
    assert_eq!(from_lsp_character(line, 8, Encoding::Utf8), 4);
    assert_eq!(from_lsp_character(line, 4, Encoding::Utf32), 4);
    // Inside the emoji: the emoji.
    assert_eq!(from_lsp_character(line, 2, Encoding::Utf16), 1);
    assert_eq!(from_lsp_character(line, 99, Encoding::Utf8), 5);
    for column in 0..=5 {
        for encoding in [Encoding::Utf8, Encoding::Utf16, Encoding::Utf32] {
            let character = to_lsp_character(line, column, encoding);
            assert_eq!(from_lsp_character(line, character, encoding), column);
        }
    }
}

#[test]
fn lines_split_at_every_line_end() {
    assert_eq!(lines("a\nb\r\nc\rd"), vec!["a", "b", "c", "d"]);
    assert_eq!(lines("a\n"), vec!["a", ""]);
    assert_eq!(lines(""), vec![""]);
}

#[test]
fn paths_and_uris_round_trip() {
    let path = Path::new("/home/me/my project/größe#1.rs");
    let uri = path_to_uri(path);
    assert_eq!(uri, "file:///home/me/my%20project/gr%C3%B6%C3%9Fe%231.rs");
    assert_eq!(uri_to_path(&uri).as_deref(), Some(path));
    assert_eq!(
        uri_to_path("file://localhost/a/b").as_deref(),
        Some(Path::new("/a/b"))
    );
    assert_eq!(uri_to_path("https://x/a"), None);
    assert_eq!(uri_to_path("file:///a%2"), None);
    assert_eq!(uri_to_path("file://host/a"), None);
}

// ------------------------------------------------------------------- docs

fn stamp(size: u64) -> Stamp {
    Stamp {
        size,
        modified: Some(SystemTime::UNIX_EPOCH),
    }
}

fn methods(notes: &[Notification]) -> Vec<&str> {
    notes.iter().map(|(m, _)| *m).collect()
}

#[test]
fn the_51st_file_closes_the_least_recently_used() {
    let mut docs = Docs::new(false);
    for i in 0..MAX_OPEN {
        let notes = docs.use_file(&PathBuf::from(format!("/w/{i}.rs")), stamp(1), "x");
        assert_eq!(methods(&notes), ["textDocument/didOpen"]);
    }
    // Using file 0 again makes file 1 the oldest.
    assert!(
        docs.use_file(Path::new("/w/0.rs"), stamp(1), "x")
            .is_empty()
    );
    let notes = docs.use_file(Path::new("/w/new.rs"), stamp(1), "x");
    assert_eq!(
        methods(&notes),
        ["textDocument/didClose", "textDocument/didOpen"]
    );
    assert_eq!(
        notes[0].1,
        json!({"textDocument": {"uri": "file:///w/1.rs"}})
    );
    assert_eq!(docs.paths().len(), MAX_OPEN);
    assert!(docs.is_open(Path::new("/w/0.rs")));
    assert!(!docs.is_open(Path::new("/w/1.rs")));
}

#[test]
fn changes_and_saves_raise_the_version() {
    let path = Path::new("/w/a.rs");
    let mut docs = Docs::new(true);
    let open = docs.use_file(path, stamp(1), "a");
    assert_eq!(open[0].1["textDocument"]["languageId"], "rust");
    assert!(docs.changed(path, stamp(1), "a").is_empty());
    let change = docs.changed(path, stamp(2), "ab");
    assert_eq!(
        change[0].1,
        json!({
            "textDocument": {"uri": "file:///w/a.rs", "version": 2},
            "contentChanges": [{"text": "ab"}],
        })
    );
    let saved = docs.saved(path, stamp(2), "ab");
    assert_eq!(
        methods(&saved),
        ["textDocument/didChange", "textDocument/didSave"]
    );
    assert_eq!(docs.version(path), Some(3));
    assert!(
        docs.saved(Path::new("/w/other.rs"), stamp(1), "")
            .is_empty()
    );
    assert_eq!(methods(&docs.removed(path)), ["textDocument/didClose"]);
    assert!(docs.removed(path).is_empty());
}

// -------------------------------------------------------------- the server

type Answer = Option<Result<Value, (i64, String)>>;
type Handler = Arc<dyn Fn(&str, &Value) -> Answer + Send + Sync>;

/// A language server in the same process. It answers with `handler`
/// (`None`: no answer) and logs every message it gets.
struct Fake {
    conn: Connection,
    log: Arc<Mutex<Vec<(String, Value)>>>,
    task: tokio::task::AbortHandle,
}

/// Dropping the fake closes its end of the stream.
impl Drop for Fake {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Fake {
    fn start(handler: Handler) -> (Fake, ReadHalf<DuplexStream>, WriteHalf<DuplexStream>) {
        let (client, server) = duplex(1 << 20);
        let (cr, cw) = split(client);
        let (sr, sw) = split(server);
        let (conn, mut incoming) = Connection::new(sr, sw);
        let log = Arc::new(Mutex::new(Vec::new()));
        let task = tokio::spawn({
            let conn = conn.clone();
            let log = Arc::clone(&log);
            async move {
                while let Some(message) = incoming.recv().await {
                    match message {
                        Incoming::Notification { method, params } => {
                            log.lock().unwrap().push((method, params));
                        }
                        Incoming::Request { id, method, params } => {
                            log.lock().unwrap().push((method.clone(), params.clone()));
                            if let Some(answer) = handler(&method, &params) {
                                let _ = conn.respond(id, answer);
                            }
                        }
                    }
                }
            }
        })
        .abort_handle();
        (Fake { conn, log, task }, cr, cw)
    }

    fn methods(&self) -> Vec<String> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .map(|(m, _)| m.clone())
            .collect()
    }

    /// The params of the last message with `method`, waiting for it.
    async fn wait_for(&self, method: &str) -> Value {
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            let found = self
                .log
                .lock()
                .unwrap()
                .iter()
                .rev()
                .find(|(m, _)| m == method)
                .map(|(_, p)| p.clone());
            if let Some(params) = found {
                return params;
            }
            assert!(Instant::now() < until, "no {method}: {:?}", self.methods());
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }
}

fn capabilities() -> Value {
    json!({
        "positionEncoding": "utf-8",
        "textDocumentSync": {"openClose": true, "change": 1, "save": {"includeText": false}},
        "definitionProvider": true,
        "referencesProvider": true,
    })
}

/// Answers `initialize`, `shutdown`, and `test/echo` with its params;
/// anything else gets no answer.
fn standard() -> Handler {
    Arc::new(|method, params| match method {
        "initialize" => Some(Ok(json!({"capabilities": capabilities()}))),
        "shutdown" => Some(Ok(Value::Null)),
        "test/echo" => Some(Ok(params.clone())),
        _ => None,
    })
}

fn setup(root: &Path) -> Setup {
    Setup {
        name: "fake".into(),
        command: Vec::new(),
        env: Vec::new(),
        root: root.to_path_buf(),
        settings: json!({"checkOnSave": false, "fake": {"inner": 1}}),
        start_timeout: Duration::from_secs(2),
        request_timeout: Duration::from_millis(200),
    }
}

async fn started(handler: Handler) -> (Server, Fake, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let (fake, reader, writer) = Fake::start(handler);
    let server = Server::over(&setup(tmp.path()), reader, writer)
        .await
        .unwrap();
    (server, fake, tmp)
}

#[tokio::test]
async fn initialize_sends_root_capabilities_and_settings() {
    let (server, fake, tmp) = started(standard()).await;
    let params = fake.wait_for("initialize").await;
    let uri = path_to_uri(tmp.path());
    assert_eq!(params["rootUri"], json!(uri));
    assert_eq!(params["workspaceFolders"][0]["uri"], json!(uri));
    assert_eq!(params["initializationOptions"]["checkOnSave"], false);
    assert_eq!(
        params["capabilities"]["general"]["positionEncodings"],
        json!(["utf-8", "utf-16"])
    );
    assert_eq!(params["capabilities"]["window"]["workDoneProgress"], true);
    fake.wait_for("initialized").await;
    assert_eq!(fake.methods(), ["initialize", "initialized"]);
    assert_eq!(server.encoding(), Encoding::Utf8);
    assert!(server.capabilities().definition_provider.is_some());
    assert!(server.capabilities().hover_provider.is_none());
    assert_eq!(server.state(), State::Ready);
    assert_eq!(server.name(), "fake");
    assert_eq!(server.root(), tmp.path());
}

#[tokio::test]
async fn utf16_is_the_default_encoding() {
    let handler: Handler =
        Arc::new(|method, _| (method == "initialize").then(|| Ok(json!({"capabilities": {}}))));
    let (server, _fake, _tmp) = started(handler).await;
    assert_eq!(server.encoding(), Encoding::Utf16);
}

#[tokio::test]
async fn an_error_on_initialize_fails_the_start() {
    let tmp = tempfile::tempdir().unwrap();
    let handler: Handler = Arc::new(|_, _| Some(Err((-32603, "cannot index".into()))));
    let (_fake, reader, writer) = Fake::start(handler);
    let result = Server::over(&setup(tmp.path()), reader, writer).await;
    assert_eq!(
        result.err(),
        Some(Error::Server {
            code: -32603,
            message: "cannot index".into()
        })
    );
}

#[tokio::test]
async fn an_answer_without_capabilities_fails_the_start() {
    let tmp = tempfile::tempdir().unwrap();
    let handler: Handler = Arc::new(|_, _| Some(Ok(json!({}))));
    let (_fake, reader, writer) = Fake::start(handler);
    let result = Server::over(&setup(tmp.path()), reader, writer).await;
    assert!(
        matches!(result, Err(Error::Protocol(_))),
        "{:?}",
        result.err()
    );
}

async fn eventually(server: &Server, state: State) {
    let until = Instant::now() + Duration::from_secs(2);
    while server.state() != state {
        assert!(Instant::now() < until, "{:?} != {state:?}", server.state());
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

#[tokio::test]
async fn progress_is_indexing_until_it_ends() {
    let (server, fake, _tmp) = started(standard()).await;
    let progress = |kind: &str, percent: Option<u32>| {
        let mut value = json!({"kind": kind, "title": "Indexing"});
        if let Some(p) = percent {
            value["percentage"] = json!(p);
        }
        fake.conn
            .notify(
                "$/progress",
                json!({"token": "rustAnalyzer/Indexing", "value": value}),
            )
            .unwrap();
    };
    progress("begin", Some(0));
    eventually(&server, State::Indexing { percent: Some(0) }).await;
    progress("report", Some(42));
    eventually(&server, State::Indexing { percent: Some(42) }).await;
    // A request without an answer while indexing.
    let answer = server.request("test/slow", Value::Null).await;
    assert_eq!(answer, Err(Error::Indexing { percent: Some(42) }));
    progress("report", None);
    progress("end", None);
    eventually(&server, State::Ready).await;
    let answer = server.request("test/slow", Value::Null).await;
    assert_eq!(answer, Err(Error::Timeout(1)));
    assert_eq!(
        server.request("test/echo", json!([1])).await,
        Ok(json!([1]))
    );
}

#[tokio::test]
async fn server_requests_are_answered() {
    let (_server, fake, _tmp) = started(standard()).await;
    let ask = |method: &'static str, params: Value| {
        let conn = fake.conn.clone();
        async move { conn.request(method, params, SECOND).await }
    };
    let items = json!({"items": [{"section": "fake"}, {"section": "other"}, {}]});
    assert_eq!(
        ask("workspace/configuration", items).await,
        Ok(json!([
            {"inner": 1},
            {"checkOnSave": false, "fake": {"inner": 1}},
            {"checkOnSave": false, "fake": {"inner": 1}},
        ]))
    );
    assert_eq!(
        ask("window/workDoneProgress/create", json!({"token": 1})).await,
        Ok(Value::Null)
    );
    assert_eq!(
        ask("client/registerCapability", json!({"registrations": []})).await,
        Ok(Value::Null)
    );
    assert_eq!(
        ask(
            "window/showMessageRequest",
            json!({"type": 1, "message": "?"})
        )
        .await,
        Ok(Value::Null)
    );
    let applied = ask("workspace/applyEdit", json!({"edit": {}}))
        .await
        .unwrap();
    assert_eq!(applied["applied"], false);
    match ask("workspace/somethingElse", Value::Null).await {
        Err(Error::Server { code, .. }) => assert_eq!(code, METHOD_NOT_FOUND),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn published_diagnostics_are_kept_by_path() {
    let (server, fake, tmp) = started(standard()).await;
    let path = tmp.path().join("src/a b.rs");
    let changed = server.diagnostics_changed();
    let notified = changed.notified();
    fake.conn
        .notify(
            "textDocument/publishDiagnostics",
            json!({
                "uri": path_to_uri(&path),
                "version": 3,
                "diagnostics": [{
                    "range": {"start": {"line": 1, "character": 4}, "end": {"line": 1, "character": 8}},
                    "severity": 1,
                    "code": "E0425",
                    "message": "cannot find value `lnes` in this scope",
                }],
            }),
        )
        .unwrap();
    tokio::time::timeout(SECOND, notified).await.unwrap();
    let diagnostics = server.diagnostics(&path).unwrap();
    assert_eq!(diagnostics.version, Some(3));
    assert_eq!(diagnostics.diagnostics.len(), 1);
    assert_eq!(
        diagnostics.diagnostics[0].message,
        "cannot find value `lnes` in this scope"
    );
    assert_eq!(server.diagnostics(&tmp.path().join("other.rs")), None);
}

#[tokio::test]
async fn files_are_opened_and_synced_before_requests() {
    let (server, fake, tmp) = started(standard()).await;
    let path = tmp.path().join("a.rs");
    fs::write(&path, "fn a() {}\n").unwrap();
    server.open(&path).await.unwrap();
    let open = fake.wait_for("textDocument/didOpen").await;
    assert_eq!(open["textDocument"]["text"], "fn a() {}\n");
    assert_eq!(open["textDocument"]["version"], 1);
    // Opening it again without a change sends nothing.
    server.open(&path).await.unwrap();

    // Changed on disk (another size): didChange before the next request.
    fs::write(&path, "fn a() { b() }\n").unwrap();
    server.request("test/echo", json!("x")).await.unwrap();
    let methods = fake.methods();
    assert_eq!(
        methods[2..],
        [
            "textDocument/didOpen",
            "textDocument/didChange",
            "test/echo"
        ]
    );
    let change = fake.wait_for("textDocument/didChange").await;
    assert_eq!(change["textDocument"]["version"], 2);
    assert_eq!(change["contentChanges"][0]["text"], "fn a() { b() }\n");

    // An edit of clai: didChange and didSave at once.
    fs::write(&path, "fn a() { c() }\n").unwrap();
    assert_eq!(server.saved(&path).await, Ok(Some(3)));
    fake.wait_for("textDocument/didSave").await;
    assert_eq!(server.saved(&tmp.path().join("closed.rs")).await, Ok(None));

    // Removed from disk: closed before the next request.
    fs::remove_file(&path).unwrap();
    server.request("test/echo", json!("y")).await.unwrap();
    fake.wait_for("textDocument/didClose").await;
    assert_eq!(server.version(&path).await, None);
}

#[tokio::test]
async fn opening_a_missing_file_is_an_error() {
    let (server, _fake, tmp) = started(standard()).await;
    let err = server.open(&tmp.path().join("nope.rs")).await;
    assert!(matches!(err, Err(Error::Io(_))), "{err:?}");
}

#[tokio::test]
async fn stop_sends_shutdown_then_exit() {
    let (server, fake, _tmp) = started(standard()).await;
    server.stop().await;
    fake.wait_for("exit").await;
    assert_eq!(
        fake.methods(),
        ["initialize", "initialized", "shutdown", "exit"]
    );
}

#[tokio::test]
async fn a_server_that_goes_away_has_failed() {
    let (server, fake, _tmp) = started(standard()).await;
    drop(fake);
    let until = Instant::now() + Duration::from_secs(2);
    loop {
        if let State::Failed(reason) = server.state() {
            assert_eq!(reason, "the server closed the connection");
            break;
        }
        assert!(Instant::now() < until, "{:?}", server.state());
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let answer = server.request("test/echo", Value::Null).await;
    assert!(matches!(answer, Err(Error::Closed(_))), "{answer:?}");
}

// ------------------------------------------------------------ processes

fn process_setup(root: &Path, command: &[&str]) -> Setup {
    Setup {
        command: command.iter().map(|s| s.to_string()).collect(),
        ..setup(root)
    }
}

#[tokio::test]
async fn a_missing_program_fails_to_start() {
    let tmp = tempfile::tempdir().unwrap();
    let setup = process_setup(tmp.path(), &["clai-no-such-language-server"]);
    match Server::start(&setup).await {
        Err(Error::Start(reason)) => {
            assert!(
                reason.starts_with("clai-no-such-language-server: "),
                "{reason}"
            )
        }
        other => panic!("{:?}", other.err()),
    }
    let empty = process_setup(tmp.path(), &[]);
    assert!(matches!(Server::start(&empty).await, Err(Error::Start(_))));
}

#[tokio::test]
async fn a_server_that_exits_fails_with_its_last_stderr_line() {
    let tmp = tempfile::tempdir().unwrap();
    let setup = process_setup(
        tmp.path(),
        &[
            "sh",
            "-c",
            "echo starting >&2; echo 'error: no Cargo.toml' >&2; exit 1",
        ],
    );
    match Server::start(&setup).await {
        Err(Error::Closed(reason)) => assert_eq!(reason, "error: no Cargo.toml"),
        other => panic!("{:?}", other.err()),
    }
}
