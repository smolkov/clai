//! One language server: its process, the `initialize` handshake, its
//! progress and diagnostics, the requests it sends, and its stop.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use indexmap::IndexMap;
use lsp_types::{
    Diagnostic, PublishDiagnosticsParams, ServerCapabilities, TextDocumentSyncCapability,
    TextDocumentSyncSaveOptions,
};
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Notify, mpsc};
use tokio::task::AbortHandle;

use super::docs::{Docs, Encoding, Stamp, path_to_uri, uri_to_path};
use super::transport::{Connection, Incoming, METHOD_NOT_FOUND};
use super::{Error, Result};
use crate::sync::lock;

/// Lines of standard error kept for `/lsp` and error results.
pub const STDERR_LINES: usize = 50;

/// Longest standard error line kept; longer ones are cut.
const STDERR_LINE_MAX: usize = 1000;

/// How long `shutdown` and the exit of the process are waited for.
pub const STOP_WAIT: Duration = Duration::from_secs(2);

/// What is needed to start one server for one root.
#[derive(Debug, Clone)]
pub struct Setup {
    /// The configured name, such as `rust-analyzer`.
    pub name: String,
    /// Program and arguments.
    pub command: Vec<String>,
    /// Extra environment variables.
    pub env: Vec<(String, String)>,
    /// The project root; the process's working directory.
    pub root: PathBuf,
    /// Sent as `initializationOptions` and for `workspace/configuration`;
    /// `Null` for none.
    pub settings: Value,
    /// Waiting for the `initialize` answer.
    pub start_timeout: Duration,
    /// Waiting for the answer of each request.
    pub request_timeout: Duration,
}

/// What a server is doing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Started, `initialize` not answered yet.
    Starting,
    /// Reports work in progress, such as indexing, with the last
    /// percentage it gave.
    Indexing {
        percent: Option<u32>,
    },
    Ready,
    /// The connection is gone; why.
    Failed(String),
}

/// The diagnostics a server published for one file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FileDiagnostics {
    /// The document version they are for, when the server says.
    pub version: Option<i32>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Shared between the server and its tasks.
#[derive(Default)]
struct Status {
    initialized: bool,
    /// Work-done progress in flight, by token: its last percentage.
    progress: IndexMap<String, Option<u32>>,
    last_percent: Option<u32>,
    diagnostics: HashMap<PathBuf, FileDiagnostics>,
    stderr: VecDeque<String>,
}

/// The process of a server, killed with its process group when dropped.
struct Process {
    child: Child,
    group: Option<Pid>,
}

impl Process {
    fn kill(&mut self) {
        if let Some(pid) = self.group.take() {
            // ESRCH: the group is already gone.
            let _ = killpg(pid, Signal::SIGKILL);
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        self.kill();
    }
}

/// A running language server.
pub struct Server {
    name: String,
    root: PathBuf,
    conn: Connection,
    status: Arc<Mutex<Status>>,
    diagnostics_changed: Arc<Notify>,
    capabilities: ServerCapabilities,
    encoding: Encoding,
    docs: tokio::sync::Mutex<Docs>,
    request_timeout: Duration,
    process: tokio::sync::Mutex<Option<Process>>,
    tasks: Vec<AbortHandle>,
}

impl Drop for Server {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

impl Server {
    /// Starts the process of `setup` and initializes it. The process gets
    /// its own process group and the environment of `clai` plus
    /// `setup.env`.
    pub async fn start(setup: &Setup) -> Result<Server> {
        let Some((program, args)) = setup.command.split_first() else {
            return Err(Error::Start("the command is empty".into()));
        };
        let mut command = Command::new(program);
        command
            .args(args)
            .envs(setup.env.iter().map(|(k, v)| (k, v)))
            .current_dir(&setup.root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .kill_on_drop(true);
        let mut child = command
            .spawn()
            .map_err(|e| Error::Start(format!("{program}: {e}")))?;
        let group = child
            .id()
            .and_then(|id| i32::try_from(id).ok())
            .map(Pid::from_raw);
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            return Err(Error::Start(format!("{program}: no standard streams")));
        };
        let process = Process { child, group };
        Server::connect(setup, stdout, stdin, Some(stderr), Some(process)).await
    }

    /// Initializes a server that talks over the given streams, without a
    /// process: a server in the same process, for tests.
    #[cfg(test)]
    pub(crate) async fn over<R, W>(setup: &Setup, reader: R, writer: W) -> Result<Server>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        Server::connect(setup, reader, writer, None::<tokio::io::Empty>, None).await
    }

    /// Initializes a server on the given streams. `start` uses it with the
    /// process's pipes, tests with an in-process server.
    async fn connect<R, W, E>(
        setup: &Setup,
        reader: R,
        writer: W,
        stderr: Option<E>,
        process: Option<Process>,
    ) -> Result<Server>
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
        E: AsyncRead + Unpin + Send + 'static,
    {
        let (conn, incoming) = Connection::new(reader, writer);
        let status = Arc::new(Mutex::new(Status::default()));
        let diagnostics_changed = Arc::new(Notify::new());
        let mut tasks = vec![
            tokio::spawn(handle_incoming(
                incoming,
                conn.clone(),
                Arc::clone(&status),
                Arc::clone(&diagnostics_changed),
                setup.settings.clone(),
            ))
            .abort_handle(),
        ];
        let mut stderr_task = stderr.map(|stderr| {
            tokio::spawn(read_stderr(stderr, setup.name.clone(), Arc::clone(&status)))
        });
        if let Some(task) = &stderr_task {
            tasks.push(task.abort_handle());
        }
        let mut server = Server {
            name: setup.name.clone(),
            root: setup.root.clone(),
            conn,
            status,
            diagnostics_changed,
            capabilities: ServerCapabilities::default(),
            encoding: Encoding::default(),
            docs: tokio::sync::Mutex::new(Docs::default()),
            request_timeout: setup.request_timeout,
            process: tokio::sync::Mutex::new(process),
            tasks,
        };
        let answer = server
            .conn
            .request("initialize", initialize_params(setup), setup.start_timeout)
            .await;
        let answer = match answer {
            Ok(answer) => answer,
            Err(err) => {
                server.kill().await;
                // The last stderr lines say why; they may still be in the pipe.
                if let Some(task) = stderr_task.as_mut() {
                    let _ = tokio::time::timeout(STOP_WAIT, task).await;
                }
                return Err(server.failure_of(err));
            }
        };
        let capabilities = answer
            .get("capabilities")
            .cloned()
            .ok_or_else(|| Error::Protocol("initialize: no capabilities".into()))
            .and_then(|c| {
                serde_json::from_value::<ServerCapabilities>(c)
                    .map_err(|e| Error::Protocol(format!("initialize: {e}")))
            });
        let capabilities = match capabilities {
            Ok(capabilities) => capabilities,
            Err(err) => {
                server.kill().await;
                return Err(err);
            }
        };
        server.encoding = Encoding::from_kind(
            capabilities
                .position_encoding
                .as_ref()
                .map(|kind| kind.as_str()),
        );
        server.docs = tokio::sync::Mutex::new(Docs::new(wants_save(&capabilities)));
        server.capabilities = capabilities;
        server.conn.notify("initialized", json!({}))?;
        lock(&server.status).initialized = true;
        Ok(server)
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn capabilities(&self) -> &ServerCapabilities {
        &self.capabilities
    }

    /// How the server counts characters in a line.
    pub fn encoding(&self) -> Encoding {
        self.encoding
    }

    pub fn state(&self) -> State {
        if let Some(reason) = self.conn.closed() {
            return State::Failed(self.with_stderr(reason));
        }
        let status = lock(&self.status);
        if !status.initialized {
            State::Starting
        } else if status.progress.is_empty() {
            State::Ready
        } else {
            State::Indexing {
                percent: status.last_percent,
            }
        }
    }

    /// The last lines of the server's standard error, oldest first.
    pub fn stderr(&self) -> Vec<String> {
        lock(&self.status).stderr.iter().cloned().collect()
    }

    /// The diagnostics the server last published for a file.
    pub fn diagnostics(&self, path: &Path) -> Option<FileDiagnostics> {
        lock(&self.status).diagnostics.get(path).cloned()
    }

    /// Woken each time diagnostics arrive.
    pub fn diagnostics_changed(&self) -> Arc<Notify> {
        Arc::clone(&self.diagnostics_changed)
    }

    /// Sends a request about the code, after syncing the open files. A
    /// timeout while the server reports progress is [`Error::Indexing`].
    pub async fn request(&self, method: &str, params: Value) -> Result<Value> {
        self.sync().await?;
        match self
            .conn
            .request(method, params, self.request_timeout)
            .await
        {
            Err(Error::Timeout(s)) => {
                let status = lock(&self.status);
                if status.progress.is_empty() {
                    Err(Error::Timeout(s))
                } else {
                    Err(Error::Indexing {
                        percent: status.last_percent,
                    })
                }
            }
            Err(err) => Err(self.failure_of(err)),
            Ok(answer) => Ok(answer),
        }
    }

    /// Opens a file to ask about, with its text from disk, or sends its
    /// changes when it is open and changed.
    pub async fn open(&self, path: &Path) -> Result<()> {
        let mut docs = self.docs.lock().await;
        let stamp = stamp(path).await?;
        if docs.stamp(path) == Some(stamp) {
            // Only made the most recently used.
            let notes = docs.use_file(path, stamp, "");
            return self.send_all(notes);
        }
        let text = read_text(path).await?;
        let notes = docs.use_file(path, stamp, &text);
        self.send_all(notes)
    }

    /// Checks the open files against the disk: a changed one gets
    /// `didChange`, a removed one `didClose`.
    pub async fn sync(&self) -> Result<()> {
        let mut docs = self.docs.lock().await;
        for path in docs.paths() {
            let notes = match stamp(&path).await {
                Err(_) => docs.removed(&path),
                Ok(stamp) if docs.stamp(&path) == Some(stamp) => Vec::new(),
                Ok(stamp) => match read_text(&path).await {
                    Ok(text) => docs.changed(&path, stamp, &text),
                    Err(_) => docs.removed(&path),
                },
            };
            self.send_all(notes)?;
        }
        Ok(())
    }

    /// `edit_file` or `write_file` wrote `path`: an open file gets
    /// `didChange` and `didSave`. Returns the version sent, or `None` when
    /// the file is not open.
    pub async fn saved(&self, path: &Path) -> Result<Option<i32>> {
        let mut docs = self.docs.lock().await;
        if !docs.is_open(path) {
            return Ok(None);
        }
        let stamp = stamp(path).await?;
        let text = read_text(path).await?;
        let notes = docs.saved(path, stamp, &text);
        self.send_all(notes)?;
        Ok(docs.version(path))
    }

    /// Whether a file is open, and its version.
    pub async fn version(&self, path: &Path) -> Option<i32> {
        self.docs.lock().await.version(path)
    }

    /// Stops the server: `shutdown`, `exit`, and the process group is
    /// killed when it has not exited after [`STOP_WAIT`].
    pub async fn stop(&self) {
        if self.conn.closed().is_none() {
            let _ = self.conn.request("shutdown", Value::Null, STOP_WAIT).await;
            let _ = self.conn.notify("exit", Value::Null);
        }
        let mut process = self.process.lock().await;
        if let Some(process) = process.as_mut() {
            let _ = tokio::time::timeout(STOP_WAIT, process.child.wait()).await;
            // Also what the server left running in its group.
            process.kill();
            let _ = process.child.wait().await;
        }
    }

    async fn kill(&self) {
        if let Some(process) = self.process.lock().await.as_mut() {
            process.kill();
            let _ = process.child.wait().await;
        }
    }

    fn send_all(&self, notes: Vec<super::docs::Notification>) -> Result<()> {
        for (method, params) in notes {
            self.conn.notify(method, params)?;
        }
        Ok(())
    }

    /// A closed connection says the last line of standard error, which is
    /// usually why the server stopped.
    fn failure_of(&self, err: Error) -> Error {
        match err {
            Error::Closed(reason) => Error::Closed(self.with_stderr(reason)),
            other => other,
        }
    }

    fn with_stderr(&self, reason: String) -> String {
        match lock(&self.status).stderr.back() {
            Some(line) => line.clone(),
            None => reason,
        }
    }
}

/// Whether the server wants `didSave`.
fn wants_save(capabilities: &ServerCapabilities) -> bool {
    match &capabilities.text_document_sync {
        Some(TextDocumentSyncCapability::Options(options)) => match &options.save {
            Some(TextDocumentSyncSaveOptions::Supported(yes)) => *yes,
            Some(TextDocumentSyncSaveOptions::SaveOptions(_)) => true,
            None => false,
        },
        _ => false,
    }
}

fn initialize_params(setup: &Setup) -> Value {
    let uri = path_to_uri(&setup.root);
    let name = setup
        .root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "root".into());
    let mut params = json!({
        "processId": std::process::id(),
        "clientInfo": {"name": "clai", "version": env!("CARGO_PKG_VERSION")},
        "rootUri": uri,
        "workspaceFolders": [{"uri": uri, "name": name}],
        "capabilities": client_capabilities(),
        "trace": "off",
    });
    if !setup.settings.is_null()
        && let Some(map) = params.as_object_mut()
    {
        map.insert("initializationOptions".into(), setup.settings.clone());
    }
    params
}

/// What `clai` handles of the protocol.
fn client_capabilities() -> Value {
    json!({
        "general": {"positionEncodings": ["utf-8", "utf-16"]},
        "window": {"workDoneProgress": true},
        "workspace": {"configuration": true, "symbol": {}},
        "textDocument": {
            "synchronization": {"didSave": true},
            "definition": {"linkSupport": true},
            "typeDefinition": {"linkSupport": true},
            "implementation": {"linkSupport": true},
            "references": {},
            "hover": {"contentFormat": ["markdown", "plaintext"]},
            "documentSymbol": {"hierarchicalDocumentSymbolSupport": true},
            "callHierarchy": {},
            "publishDiagnostics": {"versionSupport": true},
            "diagnostic": {},
        },
    })
}

/// Handles notifications and requests of the server until the connection
/// ends.
async fn handle_incoming(
    mut incoming: mpsc::UnboundedReceiver<Incoming>,
    conn: Connection,
    status: Arc<Mutex<Status>>,
    diagnostics_changed: Arc<Notify>,
    settings: Value,
) {
    while let Some(message) = incoming.recv().await {
        match message {
            Incoming::Notification { method, params } => {
                notification(&method, params, &status, &diagnostics_changed);
            }
            Incoming::Request { id, method, params } => {
                let answer = server_request(&method, &params, &settings);
                let _ = conn.respond(id, answer);
            }
        }
    }
}

fn notification(method: &str, params: Value, status: &Mutex<Status>, changed: &Notify) {
    match method {
        "$/progress" => progress(&params, status),
        "textDocument/publishDiagnostics" => {
            let Ok(params) = serde_json::from_value::<PublishDiagnosticsParams>(params) else {
                tracing::debug!("language server: malformed publishDiagnostics");
                return;
            };
            let Some(path) = uri_to_path(params.uri.as_str()) else {
                return;
            };
            lock(status).diagnostics.insert(
                path,
                FileDiagnostics {
                    version: params.version,
                    diagnostics: params.diagnostics,
                },
            );
            changed.notify_waiters();
        }
        "window/logMessage" | "window/showMessage" => {
            let text = params.get("message").and_then(Value::as_str).unwrap_or("");
            tracing::debug!(%text, "language server message");
        }
        _ => {}
    }
}

/// `$/progress` with work-done progress: `begin`, `report`, `end`.
fn progress(params: &Value, status: &Mutex<Status>) {
    let token = match params.get("token") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => return,
    };
    let Some(value) = params.get("value") else {
        return;
    };
    let percent = value
        .get("percentage")
        .and_then(Value::as_u64)
        .and_then(|p| u32::try_from(p.min(100)).ok());
    let mut status = lock(status);
    match value.get("kind").and_then(Value::as_str) {
        Some("begin") => {
            status.progress.insert(token, percent);
            status.last_percent = percent;
        }
        Some("report") => {
            if let (Some(entry), Some(_)) = (status.progress.get_mut(&token), percent) {
                *entry = percent;
                status.last_percent = percent;
            }
        }
        Some("end") => {
            status.progress.shift_remove(&token);
            if status.progress.is_empty() {
                status.last_percent = None;
            }
        }
        _ => {}
    }
}

/// Answers a request from the server.
fn server_request(
    method: &str,
    params: &Value,
    settings: &Value,
) -> std::result::Result<Value, (i64, String)> {
    match method {
        "workspace/configuration" => {
            let items = params
                .get("items")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let answers = items
                .iter()
                .map(|item| {
                    // A section the settings name gets that part, any other
                    // the settings as a whole.
                    item.get("section")
                        .and_then(Value::as_str)
                        .and_then(|section| settings.get(section))
                        .unwrap_or(settings)
                        .clone()
                })
                .collect();
            Ok(Value::Array(answers))
        }
        "window/workDoneProgress/create"
        | "client/registerCapability"
        | "client/unregisterCapability"
        | "window/showMessageRequest" => Ok(Value::Null),
        "workspace/applyEdit" => Ok(json!({
            "applied": false,
            "failureReason": "clai does not apply edits of a language server",
        })),
        _ => Err((METHOD_NOT_FOUND, format!("method not found: {method}"))),
    }
}

/// Logs standard error at `debug` and keeps its last lines.
async fn read_stderr<E: AsyncRead + Unpin>(stderr: E, name: String, status: Arc<Mutex<Status>>) {
    let mut reader = BufReader::new(stderr);
    loop {
        let mut line = Vec::new();
        let read = (&mut reader)
            .take(64 << 10)
            .read_until(b'\n', &mut line)
            .await;
        match read {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end();
        if text.is_empty() {
            continue;
        }
        tracing::debug!(server = %name, line = %text, "language server stderr");
        let kept: String = text.chars().take(STDERR_LINE_MAX).collect();
        let mut status = lock(&status);
        if status.stderr.len() >= STDERR_LINES {
            status.stderr.pop_front();
        }
        status.stderr.push_back(kept);
    }
}

async fn stamp(path: &Path) -> Result<Stamp> {
    tokio::fs::metadata(path)
        .await
        .map(|meta| Stamp::of(&meta))
        .map_err(|e| Error::Io(format!("{}: {e}", path.display())))
}

async fn read_text(path: &Path) -> Result<String> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| Error::Io(format!("{}: {e}", path.display())))?;
    Ok(String::from_utf8(bytes)
        .unwrap_or_else(|e| String::from_utf8_lossy(e.as_bytes()).into_owned()))
}
