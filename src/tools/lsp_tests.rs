//! The `lsp` tool with a language server in the same process.

use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::config::permissions::parse_for_test;
use crate::config::{Action, Lsp};
use crate::lsp::config::ServerDef;
use crate::lsp::docs::path_to_uri;
use crate::lsp::fake::{Fakes, Handler};
use crate::lsp::pool::{IDLE, Pool};
use crate::permissions::{SessionRules, Workspace};

use super::lsp::{LspEnv, Start, started};
use super::*;

const CONFIG: &str = r#"
[permissions.deny]
read = ["secret.rs"]
"#;

fn def(name: &str, ext: &str) -> ServerDef {
    ServerDef {
        name: name.into(),
        command: vec![name.into()],
        extensions: vec![ext.into()],
        roots: vec!["Cargo.toml".into()],
        settings: Value::Null,
        env: Vec::new(),
        builtin: false,
    }
}

/// What the fake server answers: by method, from a table the test fills.
type Answers = Arc<Mutex<Vec<(String, Value)>>>;

struct Fixture {
    _tmp: tempfile::TempDir,
    outside: std::path::PathBuf,
    env: Env,
    fakes: Fakes,
    answers: Answers,
    /// The params of each request, by method.
    asked: Arc<Mutex<Vec<(String, Value)>>>,
}

fn capabilities() -> Value {
    json!({
        "positionEncoding": "utf-16",
        "textDocumentSync": {"openClose": true, "change": 1},
        "definitionProvider": true,
        "referencesProvider": true,
        "hoverProvider": true,
        "documentSymbolProvider": true,
        "workspaceSymbolProvider": true,
        "callHierarchyProvider": true,
        "diagnosticProvider": {"interFileDependencies": false, "workspaceDiagnostics": false},
    })
}

impl Fixture {
    fn new(start: Action, can_ask: bool) -> Fixture {
        Fixture::with_caps(start, can_ask, capabilities())
    }

    fn with_caps(start: Action, can_ask: bool, caps: Value) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("ws");
        fs::create_dir_all(root.join("src")).unwrap();
        let outside = tmp.path().join("registry");
        fs::create_dir_all(&outside).unwrap();
        let workspace = Workspace::new(&root, &[], None).unwrap();
        let answers: Answers = Arc::new(Mutex::new(Vec::new()));
        let asked = Arc::new(Mutex::new(Vec::new()));
        let handler: Handler = {
            let answers = Arc::clone(&answers);
            let asked = Arc::clone(&asked);
            Arc::new(move |method: &str, params: &Value| {
                asked
                    .lock()
                    .unwrap()
                    .push((method.to_owned(), params.clone()));
                match method {
                    "initialize" => Some(Ok(json!({"capabilities": caps.clone()}))),
                    "shutdown" => Some(Ok(Value::Null)),
                    _ => answers
                        .lock()
                        .unwrap()
                        .iter()
                        .find(|(m, _)| m == method)
                        .map(|(_, v)| Ok(v.clone()))
                        .or(Some(Err((-32601_i64, format!("no {method}"))))),
                }
            })
        };
        let fakes = Fakes::new(handler);
        let pool = Pool::with_starter(fakes.starter(), IDLE);
        let config = Lsp {
            start,
            ..Lsp::default()
        };
        let mut lsp = LspEnv::new(pool, &config, None, "1".into(), can_ask, false);
        lsp.servers = vec![def("fake-rs", "rs"), def("fake-go", "go")];
        let env = Env {
            permissions: Arc::new(parse_for_test(CONFIG, None)),
            session: Arc::new(SessionRules::default()),
            workspace: Arc::new(workspace),
            lsp: Some(Arc::new(lsp)),
        };
        Fixture {
            _tmp: tmp,
            outside,
            env,
            fakes,
            answers,
            asked,
        }
    }

    fn root(&self) -> &Path {
        self.env.workspace.root()
    }

    fn write(&self, rel: &str, content: &str) {
        let path = self.root().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }

    fn answer(&self, method: &str, value: Value) {
        self.answers
            .lock()
            .unwrap()
            .push((method.to_owned(), value));
    }

    fn uri(&self, rel: &str) -> String {
        path_to_uri(&self.root().join(rel))
    }

    fn asked(&self, method: &str) -> Value {
        self.asked
            .lock()
            .unwrap()
            .iter()
            .rev()
            .find(|(m, _)| m == method)
            .map(|(_, p)| p.clone())
            .unwrap_or_else(|| panic!("no {method}"))
    }

    fn prepare(&self, args: Value) -> Prepared {
        prepare(&self.env, "lsp", &args)
    }

    fn ready(&self, args: Value) -> PreparedCall {
        match self.prepare(args.clone()) {
            Prepared::Ready(call) => *call,
            other => panic!("{args}: {other:?}"),
        }
    }

    fn failed(&self, args: Value) -> String {
        match self.prepare(args.clone()) {
            Prepared::Failed(out) => out.text,
            other => panic!("{args}: {other:?}"),
        }
    }

    async fn run(&self, args: Value) -> ToolOutput {
        self.ready(args)
            .run(&self.env, CancellationToken::new())
            .await
    }

    async fn ok(&self, args: Value) -> String {
        let out = self.run(args.clone()).await;
        assert!(!out.is_error, "{args}: {}", out.text);
        out.text
    }
}

fn range(line: u32, character: u32) -> Value {
    json!({"start": {"line": line, "character": character}, "end": {"line": line, "character": character + 1}})
}

const MAIN: &str = "fn main() {\n    let lines = parse(&text);\n}\n";

#[test]
fn arguments_are_checked_before_a_server_is_asked() {
    let f = Fixture::new(Action::Allow, true);
    f.write("src/main.rs", MAIN);
    f.write("Cargo.toml", "");
    f.write("notes.toml", "");
    let e = f
        .failed(json!({"operation": "definition", "path": "notes.toml", "line": 1, "symbol": "x"}));
    assert_eq!(
        e,
        "no language server for .toml files; servers: fake-rs (.rs), fake-go (.go)"
    );
    let e = f.failed(
        json!({"operation": "references", "path": "src/main.rs", "line": 2, "symbol": "parser"}),
    );
    assert_eq!(
        e,
        "\"parser\" is not on line 2 of src/main.rs: let lines = parse(&text);"
    );
    let e =
        f.failed(json!({"operation": "hover", "path": "src/main.rs", "line": 9, "symbol": "x"}));
    assert_eq!(e, "src/main.rs has 4 lines, not 9");
    let e = f.failed(json!({"operation": "hover", "path": "src/main.rs"}));
    assert!(e.starts_with("line is required"), "{e}");
    let e = f.failed(json!({"operation": "hover", "path": "src", "line": 1, "symbol": "x"}));
    assert_eq!(e, "src is a directory; give a file");
    let e = f.failed(json!({"operation": "diagnostics", "path": "src/gone.rs"}));
    assert_eq!(e, "file not found: src/gone.rs");
    let e = f.failed(json!({"operation": "document_symbols"}));
    assert_eq!(e, "document_symbols needs a path");
    let e = f.failed(json!({"operation": "workspace_symbols", "path": "src/main.rs"}));
    assert_eq!(e, "workspace_symbols needs a query");
    let e = f.failed(json!({"operation": "workspace_symbols", "query": "Parser"}));
    assert!(e.starts_with("workspace_symbols needs a path"), "{e}");
    let e = f.failed(json!({"operation": "rename", "path": "src/main.rs"}));
    assert!(e.contains("unknown variant `rename`"), "{e}");

    let denied = match f.prepare(json!({"operation": "diagnostics", "path": "/etc/passwd"})) {
        Prepared::Denied(out) => out.text,
        other => panic!("{other:?}"),
    };
    assert!(denied.contains("outside"), "{denied}");

    let call = f.ready(
        json!({"operation": "references", "path": "src/main.rs", "line": 2, "symbol": "parse"}),
    );
    assert_eq!(call.tool, Tool::Lsp);
    assert_eq!(call.summary, "references parse src/main.rs:2");
    match &call.details {
        Details::Lsp {
            path,
            operation,
            server,
            root,
        } => {
            assert_eq!(path.as_ref().unwrap().resolved, "src/main.rs");
            assert_eq!(operation, "references");
            assert_eq!(server, "fake-rs");
            assert_eq!(root, &f.root().display().to_string());
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(call.proposals[0], ["lsp(src/main.rs)"]);
}

#[test]
fn the_tool_is_off_without_servers() {
    let mut f = Fixture::new(Action::Allow, true);
    f.write("src/main.rs", MAIN);
    f.env.lsp = None;
    let e = f.failed(json!({"operation": "diagnostics", "path": "src/main.rs"}));
    assert_eq!(e, "language servers are off in this session");

    // Starting denied, or no one to ask: not offered.
    for (start, can_ask, offered) in [
        (Action::Ask, true, true),
        (Action::Ask, false, false),
        (Action::Allow, false, true),
        (Action::Deny, true, false),
    ] {
        let f = Fixture::new(start, can_ask);
        assert_eq!(
            f.env.lsp.as_ref().unwrap().offered(),
            offered,
            "{start} {can_ask}"
        );
    }
}

#[tokio::test]
async fn references_show_lines_and_hide_what_the_model_may_not_read() {
    let f = Fixture::new(Action::Allow, true);
    f.write("src/main.rs", MAIN);
    f.write("src/lib.rs", "pub use parse::parse;\n");
    f.write("secret.rs", "parse(KEY)\n");
    let outside = f.outside.join("de.rs");
    fs::write(&outside, "fn parse() {}\n").unwrap();
    f.answer(
        "textDocument/references",
        json!([
            {"uri": f.uri("src/main.rs"), "range": range(1, 16)},
            {"uri": f.uri("src/lib.rs"), "range": range(0, 8)},
            {"uri": f.uri("secret.rs"), "range": range(0, 0)},
            {"uri": path_to_uri(&outside), "range": range(0, 3)},
            {"uri": f.uri("src/lib.rs"), "range": range(0, 8)},
        ]),
    );
    let text = f
        .ok(json!({"operation": "references", "path": "src/main.rs", "line": 2, "symbol": "parse"}))
        .await;
    assert_eq!(
        text,
        format!(
            "3 references to parse:\n\
             {}:1 (outside the workspace)\n\
             src/lib.rs:1:9: pub use parse::parse;\n\
             src/main.rs:2:17: let lines = parse(&text);\n\
             (1 location skipped by permission rules)",
            outside.display()
        )
    );
    let params = f.asked("textDocument/references");
    assert_eq!(params["position"], json!({"line": 1, "character": 16}));
    assert_eq!(params["context"]["includeDeclaration"], false);
    assert_eq!(params["textDocument"]["uri"], f.uri("src/main.rs"));
    // The file was opened before the request.
    assert!(
        f.fakes
            .last_methods()
            .contains(&"textDocument/didOpen".to_owned())
    );
    // Started once, and now counts as started for the session.
    let lsp = f.env.lsp.as_ref().unwrap();
    assert_eq!(started(lsp), [("fake-rs".to_owned(), f.root().to_owned())]);
}

#[tokio::test]
async fn positions_count_in_the_server_encoding() {
    let f = Fixture::new(Action::Allow, true);
    f.write("src/main.rs", "let ü = \"😀\"; parse(ü);\n");
    f.answer("textDocument/definition", Value::Null);
    let text = f
        .ok(json!({"operation": "definition", "path": "src/main.rs", "line": 1, "symbol": "parse"}))
        .await;
    assert_eq!(text, "(none found)");
    // 13 characters before `parse`; the emoji is two UTF-16 units.
    assert_eq!(
        f.asked("textDocument/definition")["position"],
        json!({"line": 0, "character": 14})
    );
    // A column instead of a symbol, and the second occurrence.
    f.ok(json!({"operation": "definition", "path": "src/main.rs", "line": 1, "column": 5}))
        .await;
    assert_eq!(
        f.asked("textDocument/definition")["position"]["character"],
        4
    );
    f.ok(json!({"operation": "definition", "path": "src/main.rs", "line": 1, "symbol": "ü", "occurrence": 2}))
        .await;
    assert_eq!(
        f.asked("textDocument/definition")["position"]["character"],
        20
    );
}

#[tokio::test]
async fn location_links_and_a_single_location() {
    let f = Fixture::new(Action::Allow, true);
    f.write("src/main.rs", MAIN);
    f.answer(
        "textDocument/definition",
        json!([{"targetUri": f.uri("src/main.rs"), "targetRange": range(0, 0), "targetSelectionRange": range(0, 3)}]),
    );
    let text = f
        .ok(json!({"operation": "definition", "path": "src/main.rs", "line": 2, "symbol": "parse"}))
        .await;
    assert_eq!(text, "1 definition of parse:\nsrc/main.rs:1:4: fn main() {");
}

#[tokio::test]
async fn hover_outline_symbols_calls_and_diagnostics() {
    let f = Fixture::new(Action::Allow, true);
    f.write("src/main.rs", MAIN);
    f.answer(
        "textDocument/hover",
        json!({"contents": {"kind": "markdown", "value": "```rust\nfn parse(&str) -> Vec<Line>\n```"}}),
    );
    let text = f
        .ok(json!({"operation": "hover", "path": "src/main.rs", "line": 2, "symbol": "parse"}))
        .await;
    assert_eq!(text, "```rust\nfn parse(&str) -> Vec<Line>\n```");

    f.answer(
        "textDocument/documentSymbol",
        json!([
            {"name": "Parser", "kind": 23, "range": {"start": {"line": 11, "character": 0}, "end": {"line": 39, "character": 1}},
             "selectionRange": range(11, 7),
             "children": [{"name": "new", "kind": 6, "range": {"start": {"line": 17, "character": 4}, "end": {"line": 21, "character": 5}}, "selectionRange": range(17, 7)}]},
            {"name": "parse", "kind": 12, "range": {"start": {"line": 42, "character": 0}, "end": {"line": 42, "character": 9}}, "selectionRange": range(42, 3)}
        ]),
    );
    let text = f
        .ok(json!({"operation": "document_symbols", "path": "src/main.rs"}))
        .await;
    assert_eq!(
        text,
        "struct Parser   12-40\n  method new    18-22\nfunction parse  43"
    );

    f.answer(
        "workspace/symbol",
        json!([{"name": "Parser", "kind": 23, "location": {"uri": f.uri("src/main.rs"), "range": range(0, 3)}}]),
    );
    let text = f
        .ok(json!({"operation": "workspace_symbols", "path": "src/main.rs", "query": "Pars"}))
        .await;
    assert_eq!(text, "struct Parser  src/main.rs:1:4");
    assert_eq!(f.asked("workspace/symbol"), json!({"query": "Pars"}));

    let item = json!({"name": "parse", "kind": 12, "uri": f.uri("src/main.rs"), "range": range(1, 16), "selectionRange": range(1, 16)});
    f.answer("textDocument/prepareCallHierarchy", json!([item]));
    f.answer(
        "callHierarchy/incomingCalls",
        json!([{"from": {"name": "main", "kind": 12, "uri": f.uri("src/main.rs"), "range": range(0, 3), "selectionRange": range(0, 3)},
                "fromRanges": [range(1, 16)]}]),
    );
    let text = f
        .ok(json!({"operation": "incoming_calls", "path": "src/main.rs", "line": 2, "symbol": "parse"}))
        .await;
    assert_eq!(
        text,
        "1 callers of parse:\nsrc/main.rs:2:17: function main: let lines = parse(&text);"
    );
    assert_eq!(f.asked("callHierarchy/incomingCalls")["item"], item);

    f.answer(
        "textDocument/diagnostic",
        json!({"kind": "full", "items": [
            {"range": range(1, 4), "severity": 2, "message": "unused variable: `lines`"},
            {"range": range(1, 16), "severity": 1, "code": "E0425", "message": "cannot find value `text`\nin this scope"},
        ]}),
    );
    let text = f
        .ok(json!({"operation": "diagnostics", "path": "src/main.rs"}))
        .await;
    assert_eq!(
        text,
        "src/main.rs:2:17: error[E0425]: cannot find value `text`; in this scope\n\
         src/main.rs:2:5: warning: unused variable: `lines`"
    );
}

#[tokio::test]
async fn errors_of_the_server_become_results() {
    let f = Fixture::with_caps(
        Action::Allow,
        true,
        json!({"textDocumentSync": 1, "referencesProvider": true}),
    );
    f.write("src/main.rs", MAIN);
    let out = f
        .run(json!({"operation": "hover", "path": "src/main.rs", "line": 2, "symbol": "parse"}))
        .await;
    assert!(out.is_error);
    assert_eq!(
        out.text,
        "fake-rs does not offer hover; it offers references, diagnostics"
    );
    let out = f
        .run(
            json!({"operation": "references", "path": "src/main.rs", "line": 2, "symbol": "parse"}),
        )
        .await;
    assert_eq!(out.text, "fake-rs: no textDocument/references");

    // A server that cannot start.
    f.fakes
        .set_handler(Arc::new(|method: &str, _: &Value| match method {
            "initialize" => Some(Err((-32603_i64, "no Cargo.toml".to_owned()))),
            _ => None,
        }));
    f.write("main.go", "package main\n");
    let out = f
        .run(json!({"operation": "diagnostics", "path": "main.go"}))
        .await;
    assert_eq!(out.text, "fake-go: no Cargo.toml");
}

#[test]
fn starting_asks_once_per_session_unless_allowed() {
    let f = Fixture::new(Action::Ask, true);
    f.write("src/main.rs", MAIN);
    f.write("Cargo.toml", "");
    let call = f.ready(json!({"operation": "diagnostics", "path": "src/main.rs"}));
    assert_eq!(call.action(), Action::Allow, "the path is allowed");
    match call.lsp_start(&f.env) {
        Start::Ask(details) => assert_eq!(
            *details,
            Details::LspStart {
                server: "fake-rs".into(),
                command: vec!["fake-rs".into()],
                root: f.root().display().to_string(),
                env: Vec::new(),
            }
        ),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        call.lsp_start_summary(&f.env),
        format!("start language server fake-rs in {}", f.root().display())
    );
    call.lsp_started(&f.env, false);
    assert_eq!(call.lsp_start(&f.env), Start::Denied);
    // A new turn may ask again.
    f.env.lsp.as_ref().unwrap().new_turn();
    assert!(matches!(call.lsp_start(&f.env), Start::Ask(_)));
    call.lsp_started(&f.env, true);
    assert_eq!(call.lsp_start(&f.env), Start::Ready);
    // Another server still asks.
    f.write("main.go", "package main\n");
    let go = f.ready(json!({"operation": "diagnostics", "path": "main.go"}));
    assert!(matches!(go.lsp_start(&f.env), Start::Ask(_)));

    let allowed = Fixture::new(Action::Allow, true);
    allowed.write("src/main.rs", MAIN);
    let call = allowed.ready(json!({"operation": "diagnostics", "path": "src/main.rs"}));
    assert_eq!(call.lsp_start(&allowed.env), Start::Ready);
}

#[test]
fn the_rules_of_lsp_and_read_apply() {
    let mut f = Fixture::new(Action::Allow, true);
    f.write("src/main.rs", MAIN);
    f.write("secret.rs", "x\n");
    f.env.permissions = Arc::new(parse_for_test(
        "[permissions.deny]\nlsp = [\"secret.rs\"]\n[permissions.ask]\nread = [\"src/**\"]\n",
        None,
    ));
    match f.prepare(json!({"operation": "diagnostics", "path": "secret.rs"})) {
        Prepared::Denied(out) => assert!(out.text.contains("deny lsp(secret.rs)"), "{}", out.text),
        other => panic!("{other:?}"),
    }
    let call = f.ready(json!({"operation": "diagnostics", "path": "src/main.rs"}));
    assert_eq!(call.action(), Action::Ask, "read includes lsp");
}

#[test]
fn details_on_the_wire() {
    let start = Details::LspStart {
        server: "rust-analyzer".into(),
        command: vec!["rust-analyzer".into()],
        root: "~/ws/clai".into(),
        env: vec![("RA_LOG".into(), "info".into())],
    };
    let wire = serde_json::to_value(&start).unwrap();
    assert_eq!(
        wire,
        json!({"tool": "lsp_start", "server": "rust-analyzer", "command": ["rust-analyzer"],
               "root": "~/ws/clai", "env": [["RA_LOG", "info"]]})
    );
    assert_eq!(serde_json::from_value::<Details>(wire).unwrap(), start);
    let ask = Details::Lsp {
        path: None,
        operation: "workspace_symbols".into(),
        server: "gopls".into(),
        root: "~/ws/go".into(),
    };
    let wire = serde_json::to_value(&ask).unwrap();
    assert_eq!(wire["tool"], "lsp");
    assert_eq!(serde_json::from_value::<Details>(wire).unwrap(), ask);
}

// ----------------------------------------------------- after an edit

impl Fixture {
    /// Starts `fake-rs` for the session with a `references` call.
    async fn start_rs(&self) {
        self.answer("textDocument/references", json!([]));
        self.ok(
            json!({"operation": "references", "path": "src/main.rs", "line": 2, "symbol": "parse"}),
        )
        .await;
    }

    async fn edit(&self, old: &str, new: &str) -> ToolOutput {
        let call = match prepare(
            &self.env,
            "edit_file",
            &json!({"path": "src/main.rs", "old_text": old, "new_text": new}),
        ) {
            Prepared::Ready(call) => *call,
            other => panic!("{other:?}"),
        };
        call.run(&self.env, CancellationToken::new()).await
    }

    /// The connection of the last fake, to publish from.
    fn fake_conn(&self) -> crate::lsp::transport::Connection {
        self.fakes
            .fakes
            .lock()
            .unwrap()
            .last()
            .and_then(|f| f.as_ref().map(|f| f.conn.clone()))
            .unwrap()
    }
}

fn error_at(line: u32, message: &str) -> Value {
    json!({"range": range(line, 4), "severity": 1, "code": "E0425", "message": message})
}

#[tokio::test]
async fn an_edit_shows_the_errors_the_server_publishes() {
    let f = Fixture::with_caps(
        Action::Allow,
        true,
        json!({"textDocumentSync": {"openClose": true, "change": 1}, "referencesProvider": true}),
    );
    f.write("src/main.rs", MAIN);
    f.start_rs().await;
    let conn = f.fake_conn();
    let uri = f.uri("src/main.rs");
    // Old diagnostics, published before the edit, do not count.
    conn.notify(
        "textDocument/publishDiagnostics",
        json!({"uri": uri, "version": 1, "diagnostics": [error_at(0, "old")]}),
    )
    .unwrap();
    let fakes = f.fakes.clone();
    let publisher = tokio::spawn(async move {
        let fake_methods = || fakes.last_methods();
        let until = std::time::Instant::now() + Duration::from_secs(2);
        while !fake_methods().contains(&"textDocument/didChange".to_owned()) {
            assert!(std::time::Instant::now() < until);
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        conn.notify(
            "textDocument/publishDiagnostics",
            json!({"uri": uri, "version": 2, "diagnostics": [
                error_at(1, "cannot find value `lnes` in this scope"),
                {"range": range(0, 3), "severity": 2, "message": "unused"},
            ]}),
        )
        .unwrap();
    });
    let out = f.edit("lines", "lnes").await;
    publisher.await.unwrap();
    assert!(!out.is_error, "{}", out.text);
    let (head, tail) = out.text.split_once("\n\n").unwrap();
    assert_eq!(head, "edited src/main.rs: replaced 1 occurrence (line 2)");
    assert_eq!(
        tail,
        "fake-rs reports 1 error in src/main.rs:\n\
         src/main.rs:2:5: error[E0425]: cannot find value `lnes` in this scope\n\
         (and 1 warning or note; lsp diagnostics lists them)"
    );
}

#[tokio::test]
async fn late_or_missing_diagnostics_add_nothing() {
    let f = Fixture::with_caps(
        Action::Allow,
        true,
        json!({"textDocumentSync": 1, "referencesProvider": true}),
    );
    f.write("src/main.rs", MAIN);
    // No server started in this session: no question, nothing added.
    let out = f.edit("lines", "rows").await;
    assert!(!out.text.contains("reports"), "{}", out.text);
    assert_eq!(f.fakes.starts(), 0);

    f.start_rs().await;
    let path = f.env.workspace.resolve("src/main.rs");
    let found = super::lsp::after_edit_for_test(&f.env, &path, Duration::from_millis(50)).await;
    assert_eq!(found, None);

    // Only warnings: nothing added.
    f.fake_conn()
        .notify(
            "textDocument/publishDiagnostics",
            json!({"uri": f.uri("src/main.rs"), "diagnostics": [{"range": range(0, 0), "severity": 2, "message": "w"}]}),
        )
        .unwrap();
    let found = super::lsp::after_edit_for_test(&f.env, &path, Duration::from_millis(200)).await;
    assert_eq!(found, None);

    // Turned off in the configuration.
    let mut off = Fixture::with_caps(
        Action::Allow,
        true,
        json!({"textDocumentSync": 1, "referencesProvider": true}),
    );
    let lsp = Arc::get_mut(off.env.lsp.as_mut().unwrap()).unwrap();
    lsp.config.diagnostics_after_edit = false;
    off.write("src/main.rs", MAIN);
    off.start_rs().await;
    let path = off.env.workspace.resolve("src/main.rs");
    assert_eq!(
        super::lsp::after_edit_for_test(&off.env, &path, Duration::from_millis(50)).await,
        None
    );
}

#[tokio::test]
async fn a_pull_server_is_asked_after_an_edit() {
    let f = Fixture::new(Action::Allow, true);
    f.write("src/main.rs", MAIN);
    f.start_rs().await;
    f.answer(
        "textDocument/diagnostic",
        json!({"kind": "full", "items": [error_at(1, "a"), error_at(2, "b")]}),
    );
    let out = f.edit("lines", "lnes").await;
    assert!(
        out.text.ends_with(
            "fake-rs reports 2 errors in src/main.rs:\n\
             src/main.rs:2:5: error[E0425]: a\n\
             src/main.rs:3:5: error[E0425]: b"
        ),
        "{}",
        out.text
    );
}
