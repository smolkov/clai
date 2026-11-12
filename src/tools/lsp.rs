//! `lsp`: questions to a language server (RFC 0019; see `spec/tools.md`,
//! "lsp").
//!
//! [`prepare`] checks the arguments and the path, finds the server and the
//! project root, and the symbol on its line. Starting the server is a
//! question of its own, asked by the agent ([`super::PreparedCall::lsp_start`]).
//! [`Op::run`] gets the server from the pool, asks it and turns the answer
//! into text, leaving out what the `read_file` rules keep from the model.

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::config::{Action, Lsp, Tool};
use crate::lsp::config::{ServerDef, find_root, for_file, servers};
use crate::lsp::docs::{
    Encoding, find_symbol, from_lsp_character, lines, path_to_uri, to_lsp_character, uri_to_path,
};
use crate::lsp::pool::{Key, Pool};
use crate::lsp::server::{FileDiagnostics, Server};
use crate::lsp::{self, Error};
use crate::permissions::{Area, Resolved, path_proposals};
use crate::sync::lock;

use super::output::ToolOutput;
use super::question::{Details, PathDetails};
use super::{Call, Env, Failure, Op as AnyOp, cancelled};

/// Locations and symbols listed at most.
const MAX_LOCATIONS: usize = 100;
/// Diagnostics listed at most.
const MAX_DIAGNOSTICS: usize = 50;
/// Characters of a location's line, and of hover text.
const MAX_LINE: usize = 200;
const MAX_HOVER: usize = 4000;
/// Entries of an outline.
const MAX_OUTLINE: usize = 500;
/// How long `diagnostics` waits for a server to publish them.
pub const DIAGNOSTICS_WAIT: Duration = Duration::from_secs(3);

/// The language servers of a session: the pool they run in, the servers
/// in use, and which ones the user let start.
pub struct LspEnv {
    pub pool: Pool,
    pub config: Lsp,
    pub servers: Vec<ServerDef>,
    /// Who uses the servers, for `/lsp`.
    pub user: String,
    /// Starting needs no question: `start = "allow"`, or `--yes`.
    pub start_allowed: bool,
    /// Someone can answer the start question.
    pub can_ask: bool,
    /// Servers the user let start in this session.
    started: Mutex<HashSet<Key>>,
    /// Servers the user did not let start in this turn.
    denied: Mutex<HashSet<Key>>,
}

impl fmt::Debug for LspEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LspEnv")
            .field("servers", &self.servers)
            .field("user", &self.user)
            .finish_non_exhaustive()
    }
}

impl LspEnv {
    /// The servers of `config` with the programs on `path`, for `user` (the
    /// session's id). `start_allowed` also when the run allows all
    /// questions (`--yes`).
    pub fn new(
        pool: Pool,
        config: &Lsp,
        path: Option<&OsStr>,
        user: String,
        can_ask: bool,
        start_allowed: bool,
    ) -> LspEnv {
        LspEnv {
            pool,
            servers: servers(config, path),
            config: config.clone(),
            user,
            start_allowed: start_allowed || config.start == Action::Allow,
            can_ask,
            started: Mutex::new(HashSet::new()),
            denied: Mutex::new(HashSet::new()),
        }
    }

    /// Whether the model gets the tool: on, with a server, and a server
    /// may start (allowed, or someone can be asked).
    pub fn offered(&self) -> bool {
        self.config.enabled
            && !self.servers.is_empty()
            && self.config.start != Action::Deny
            && (self.start_allowed || self.can_ask)
    }

    /// `.rs and .go`, for the system prompt.
    pub fn languages(&self) -> String {
        let exts: Vec<String> = self
            .servers
            .iter()
            .flat_map(|s| s.extensions.iter().map(|e| format!(".{e}")))
            .collect();
        match exts.as_slice() {
            [] => String::new(),
            [one] => one.clone(),
            [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
        }
    }

    /// A new turn: servers denied in the last one may be asked about again.
    pub fn new_turn(&self) {
        lock(&self.denied).clear();
    }

    pub fn allow_start(&self, key: Key) {
        lock(&self.denied).remove(&key);
        lock(&self.started).insert(key);
    }

    pub fn deny_start(&self, key: Key) {
        lock(&self.denied).insert(key);
    }

    fn is_denied(&self, key: &Key) -> bool {
        lock(&self.denied).contains(key)
    }

    fn is_started(&self, key: &Key) -> bool {
        lock(&self.started).contains(key)
    }
}

/// What the agent does before an `lsp` call runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    /// The server may be used.
    Ready,
    /// Ask first, with these details.
    Ask(Box<Details>),
    /// Denied in this turn already.
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Operation {
    Definition,
    TypeDefinition,
    Implementation,
    References,
    Hover,
    IncomingCalls,
    OutgoingCalls,
    DocumentSymbols,
    WorkspaceSymbols,
    Diagnostics,
}

impl Operation {
    pub const ALL: [Operation; 10] = [
        Operation::Definition,
        Operation::TypeDefinition,
        Operation::Implementation,
        Operation::References,
        Operation::Hover,
        Operation::IncomingCalls,
        Operation::OutgoingCalls,
        Operation::DocumentSymbols,
        Operation::WorkspaceSymbols,
        Operation::Diagnostics,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Operation::Definition => "definition",
            Operation::TypeDefinition => "type_definition",
            Operation::Implementation => "implementation",
            Operation::References => "references",
            Operation::Hover => "hover",
            Operation::IncomingCalls => "incoming_calls",
            Operation::OutgoingCalls => "outgoing_calls",
            Operation::DocumentSymbols => "document_symbols",
            Operation::WorkspaceSymbols => "workspace_symbols",
            Operation::Diagnostics => "diagnostics",
        }
    }

    fn has_position(self) -> bool {
        !matches!(
            self,
            Operation::DocumentSymbols | Operation::WorkspaceSymbols | Operation::Diagnostics
        )
    }

    /// The capability that offers it; `None`: always there.
    fn capability(self) -> Option<&'static str> {
        match self {
            Operation::Definition => Some("definitionProvider"),
            Operation::TypeDefinition => Some("typeDefinitionProvider"),
            Operation::Implementation => Some("implementationProvider"),
            Operation::References => Some("referencesProvider"),
            Operation::Hover => Some("hoverProvider"),
            Operation::IncomingCalls | Operation::OutgoingCalls => Some("callHierarchyProvider"),
            Operation::DocumentSymbols => Some("documentSymbolProvider"),
            Operation::WorkspaceSymbols => Some("workspaceSymbolProvider"),
            Operation::Diagnostics => None,
        }
    }

    /// The method of a location operation.
    fn location_method(self) -> Option<&'static str> {
        match self {
            Operation::Definition => Some("textDocument/definition"),
            Operation::TypeDefinition => Some("textDocument/typeDefinition"),
            Operation::Implementation => Some("textDocument/implementation"),
            Operation::References => Some("textDocument/references"),
            _ => None,
        }
    }

    /// `3 references to parse`, `1 definition of parse`.
    fn heading(self, count: usize, symbol: &str) -> String {
        let (one, many, word) = match self {
            Operation::Definition => ("definition", "definitions", "of"),
            Operation::TypeDefinition => ("type definition", "type definitions", "of"),
            Operation::Implementation => ("implementation", "implementations", "of"),
            _ => ("reference", "references", "to"),
        };
        let noun = if count == 1 { one } else { many };
        format!("{count} {noun} {word} {symbol}")
    }
}

/// Whether the server offers an operation.
fn offers(capabilities: &Value, operation: Operation) -> bool {
    match operation.capability() {
        None => true,
        Some(key) => !matches!(
            capabilities.get(key),
            None | Some(Value::Null) | Some(Value::Bool(false))
        ),
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Args {
    operation: Operation,
    path: Option<String>,
    line: Option<u64>,
    symbol: Option<String>,
    occurrence: Option<u64>,
    column: Option<u64>,
    query: Option<String>,
    include_declaration: Option<bool>,
}

#[derive(Debug)]
pub struct Op {
    operation: Operation,
    pub path: Option<Resolved>,
    server: ServerDef,
    root: PathBuf,
    /// 0-based line and character column, for position operations.
    position: Option<(u32, usize)>,
    /// The symbol asked about, as shown in the result.
    symbol: String,
    query: String,
    include_declaration: bool,
}

pub(super) fn prepare(env: &Env, args: Result<Args, Failure>) -> Result<Call, Failure> {
    let args = args?;
    let Some(lsp) = env.lsp.as_deref().filter(|l| l.offered()) else {
        return Err(Failure::Error(
            "language servers are off in this session".into(),
        ));
    };
    let operation = args.operation;
    let path = match (&args.path, operation) {
        (Some(path), _) => Some(env.workspace.resolve(path)),
        (None, Operation::WorkspaceSymbols) => None,
        (None, _) => return Err(format!("{} needs a path", operation.name()).into()),
    };
    let decision = match &path {
        Some(path) => env.checker().check_path(Tool::Lsp, path),
        None => env
            .checker()
            .check_path(Tool::Lsp, &env.workspace.resolve("")),
    };
    if decision.action == Action::Deny {
        return Err(Failure::Denied(decision.denial()));
    }

    // The server and the directory its root is looked for in.
    let (server, boundary) = match &path {
        Some(path) => {
            let shown = path.display();
            if path.abs.is_dir() {
                return Err(format!("{shown} is a directory; give a file").into());
            }
            if !path.abs.exists() {
                return Err(format!("file not found: {shown}").into());
            }
            let Some(server) = for_file(&lsp.servers, &path.abs) else {
                return Err(no_server(lsp, &path.abs).into());
            };
            let boundary = match &path.area {
                Area::Extra { dir, .. } => dir.clone(),
                _ => env.workspace.root().to_path_buf(),
            };
            (server.clone(), boundary)
        }
        None => match lsp.servers.as_slice() {
            [one] => (one.clone(), env.workspace.root().to_path_buf()),
            _ => {
                return Err(
                    "workspace_symbols needs a path: a file of the language to search"
                        .to_string()
                        .into(),
                );
            }
        },
    };
    let root = match &path {
        Some(path) => find_root(&server, &path.abs, &boundary),
        None => boundary,
    };

    let query = args.query.clone().unwrap_or_default();
    if operation == Operation::WorkspaceSymbols && query.trim().is_empty() {
        return Err("workspace_symbols needs a query".to_string().into());
    }
    let (position, symbol) = match (&path, operation.has_position()) {
        (Some(path), true) => position(path, &args)?,
        _ => (None, String::new()),
    };

    let shown = path.as_ref().map(Resolved::display).unwrap_or_default();
    let summary = match (operation, position) {
        (Operation::WorkspaceSymbols, _) => format!("workspace_symbols {query}"),
        (_, Some((line, _))) if !symbol.is_empty() => {
            format!("{} {symbol} {shown}:{}", operation.name(), line + 1)
        }
        (_, Some((line, column))) => {
            format!("{} {shown}:{}:{}", operation.name(), line + 1, column + 1)
        }
        _ => format!("{} {shown}", operation.name()),
    };
    let details = Details::Lsp {
        path: match (&path, &args.path) {
            (Some(resolved), Some(written)) => Some(PathDetails::new(written, resolved)),
            _ => None,
        },
        operation: operation.name().to_owned(),
        server: server.name.clone(),
        root: env.workspace.tilde(&root),
    };
    let proposals = path
        .as_ref()
        .map(|p| path_proposals(Tool::Lsp, p))
        .unwrap_or_default();
    Ok(Call {
        summary,
        decision,
        denial: None,
        details,
        proposals,
        op: AnyOp::Lsp(Op {
            operation,
            path,
            server,
            root,
            position,
            symbol,
            query,
            include_declaration: args.include_declaration.unwrap_or(false),
        }),
    })
}

/// The line and column of a position operation, and the symbol's name.
fn position(path: &Resolved, args: &Args) -> Result<(Option<(u32, usize)>, String), Failure> {
    let shown = path.display();
    let Some(line) = args.line.filter(|l| *l >= 1) else {
        return Err("line is required, counted from 1".to_string().into());
    };
    let text =
        fs::read_to_string(&path.abs).map_err(|e| Failure::Error(format!("{shown}: {e}")))?;
    let all = lines(&text);
    let index = usize::try_from(line - 1).unwrap_or(usize::MAX);
    let Some(row) = all.get(index) else {
        return Err(format!("{shown} has {} lines, not {line}", all.len()).into());
    };
    let line0 = u32::try_from(index).unwrap_or(u32::MAX);
    match (&args.symbol, args.column) {
        (_, Some(column)) if column >= 1 => {
            let column = usize::try_from(column - 1).unwrap_or(usize::MAX);
            let symbol = args.symbol.clone().unwrap_or_default();
            Ok((Some((line0, column)), symbol))
        }
        (Some(symbol), _) => {
            let occurrence = args.occurrence.unwrap_or(1);
            let occurrence = usize::try_from(occurrence).unwrap_or(0);
            match find_symbol(row, symbol, occurrence) {
                Some(column) => Ok((Some((line0, column)), symbol.clone())),
                None => Err(format!(
                    "\"{symbol}\" is not on line {line} of {shown}: {}",
                    cut(row.trim(), MAX_LINE)
                )
                .into()),
            }
        }
        _ => Err(
            "give the symbol on the line (or its column, counted from 1)"
                .to_string()
                .into(),
        ),
    }
}

fn no_server(lsp: &LspEnv, file: &Path) -> String {
    let kind = match file.extension().and_then(OsStr::to_str) {
        Some(ext) => format!(".{ext} files"),
        None => format!(
            "{}",
            file.file_name()
                .map(|n| n.to_string_lossy())
                .unwrap_or_default()
        ),
    };
    let list: Vec<String> = lsp.servers.iter().map(ServerDef::label).collect();
    format!(
        "no language server for {kind}; servers: {}",
        list.join(", ")
    )
}

impl Op {
    pub fn key(&self) -> Key {
        (self.server.name.clone(), self.root.clone())
    }

    /// Whether the call may use its server, and what to ask if not.
    pub fn start(&self, env: &Env) -> Start {
        let Some(lsp) = env.lsp.as_deref() else {
            return Start::Ready;
        };
        let key = self.key();
        if lsp.start_allowed || lsp.is_started(&key) {
            Start::Ready
        } else if lsp.is_denied(&key) {
            Start::Denied
        } else {
            Start::Ask(Box::new(Details::LspStart {
                server: self.server.name.clone(),
                command: self.server.command.clone(),
                root: env.workspace.tilde(&self.root),
                env: self.server.env.clone(),
            }))
        }
    }

    /// `Start language server rust-analyzer in ~/ws/clai`.
    pub fn start_summary(&self, env: &Env) -> String {
        format!(
            "start language server {} in {}",
            self.server.name,
            env.workspace.tilde(&self.root)
        )
    }

    pub async fn run(self, env: &Env, cancel: CancellationToken) -> ToolOutput {
        tokio::select! {
            output = self.ask(env) => output,
            () = cancel.cancelled() => cancelled(""),
        }
    }

    async fn ask(self, env: &Env) -> ToolOutput {
        let Some(lsp) = env.lsp.as_deref() else {
            return ToolOutput::error("language servers are off in this session");
        };
        let name = self.server.name.clone();
        let setup = self.server.setup(&self.root, &lsp.config);
        let server = match lsp.pool.get(&setup, &lsp.user).await {
            Ok(server) => server,
            Err(err) => return ToolOutput::error(failure(&name, &err)),
        };
        lsp.allow_start(self.key());
        let capabilities = serde_json::to_value(server.capabilities()).unwrap_or(Value::Null);
        if !offers(&capabilities, self.operation) {
            let has: Vec<&str> = Operation::ALL
                .iter()
                .filter(|op| offers(&capabilities, **op))
                .map(|op| op.name())
                .collect();
            return ToolOutput::error(format!(
                "{name} does not offer {}; it offers {}",
                self.operation.name(),
                has.join(", ")
            ));
        }
        if let Some(path) = &self.path
            && let Err(err) = server.open(&path.abs).await
        {
            return ToolOutput::error(failure(&name, &err));
        }
        match self.answer(env, &server).await {
            Ok(text) => ToolOutput::ok(text),
            Err(err) => ToolOutput::error(failure(&name, &err)),
        }
    }

    fn document(&self) -> Value {
        let uri = self
            .path
            .as_ref()
            .map(|p| path_to_uri(&p.abs))
            .unwrap_or_default();
        json!({ "uri": uri })
    }

    /// `textDocument` and `position` in the server's encoding.
    fn position_params(&self, encoding: Encoding) -> Value {
        let (line, column) = self.position.unwrap_or((0, 0));
        let character = self
            .path
            .as_ref()
            .and_then(|p| fs::read_to_string(&p.abs).ok())
            .map(|text| {
                let all = lines(&text);
                let row = usize::try_from(line)
                    .ok()
                    .and_then(|i| all.get(i).copied())
                    .unwrap_or("");
                to_lsp_character(row, column, encoding)
            })
            .unwrap_or(0);
        json!({
            "textDocument": self.document(),
            "position": {"line": line, "character": character},
        })
    }

    async fn answer(&self, env: &Env, server: &Server) -> lsp::Result<String> {
        let encoding = server.encoding();
        let mut files = Files::new(env, encoding);
        match self.operation {
            op @ (Operation::Definition
            | Operation::TypeDefinition
            | Operation::Implementation
            | Operation::References) => {
                let mut params = self.position_params(encoding);
                if op == Operation::References
                    && let Some(map) = params.as_object_mut()
                {
                    map.insert(
                        "context".into(),
                        json!({"includeDeclaration": self.include_declaration}),
                    );
                }
                let method = op.location_method().unwrap_or("textDocument/definition");
                let answer = server.request(method, params).await?;
                let found = locations(&answer);
                Ok(files.locations(op, &self.symbol, &found))
            }
            Operation::Hover => {
                let answer = server
                    .request("textDocument/hover", self.position_params(encoding))
                    .await?;
                Ok(hover_text(&answer))
            }
            Operation::IncomingCalls | Operation::OutgoingCalls => {
                let items = server
                    .request(
                        "textDocument/prepareCallHierarchy",
                        self.position_params(encoding),
                    )
                    .await?;
                let Some(item) = items.as_array().and_then(|a| a.first()).cloned() else {
                    return Ok(format!("no function at {} to ask about", self.symbol));
                };
                let incoming = self.operation == Operation::IncomingCalls;
                let method = if incoming {
                    "callHierarchy/incomingCalls"
                } else {
                    "callHierarchy/outgoingCalls"
                };
                let answer = server.request(method, json!({"item": item})).await?;
                Ok(files.calls(&answer, incoming, &self.symbol))
            }
            Operation::DocumentSymbols => {
                let answer = server
                    .request(
                        "textDocument/documentSymbol",
                        json!({"textDocument": self.document()}),
                    )
                    .await?;
                Ok(outline(&answer))
            }
            Operation::WorkspaceSymbols => {
                let answer = server
                    .request("workspace/symbol", json!({"query": self.query}))
                    .await?;
                Ok(files.symbols(&answer))
            }
            Operation::Diagnostics => {
                let Some(path) = &self.path else {
                    return Ok("(no errors or warnings)".into());
                };
                let pull = server.capabilities().diagnostic_provider.is_some();
                let diagnostics = if pull {
                    let answer = server
                        .request(
                            "textDocument/diagnostic",
                            json!({"textDocument": self.document()}),
                        )
                        .await?;
                    pulled(&answer)
                } else {
                    let version = server.version(&path.abs).await;
                    wait_for_diagnostics(server, &path.abs, version, 0, DIAGNOSTICS_WAIT)
                        .await
                        .map(|d| diagnostics_json(&d))
                        .unwrap_or_default()
                };
                Ok(files.diagnostics(&path.abs, &diagnostics, MAX_DIAGNOSTICS, true))
            }
        }
    }
}

/// Waits up to `wait` for diagnostics published after the `after`-th
/// publication for the file (see [`FileDiagnostics::count`]), and of
/// `version` or later when both sides have one; `None` when none came.
pub async fn wait_for_diagnostics(
    server: &Server,
    path: &Path,
    version: Option<i32>,
    after: u64,
    wait: Duration,
) -> Option<FileDiagnostics> {
    let changed = server.diagnostics_changed();
    let fits = |d: &FileDiagnostics| {
        d.count > after
            && match (d.version, version) {
                (Some(have), Some(want)) => have >= want,
                _ => true,
            }
    };
    let until = tokio::time::Instant::now() + wait;
    loop {
        let notified = changed.notified();
        if let Some(found) = server.diagnostics(path).filter(fits) {
            return Some(found);
        }
        if tokio::time::timeout_at(until, notified).await.is_err() {
            return server.diagnostics(path).filter(fits);
        }
    }
}

/// Errors listed after an edit, at most.
const MAX_EDIT_ERRORS: usize = 10;

/// After `edit_file` or `write_file` wrote `path`: the errors that a
/// server this session already uses reports for it, as text to add to
/// the result. Never starts a server or asks; `None` when there is no
/// such server, no answer within [`DIAGNOSTICS_WAIT`], or no error.
pub async fn after_edit(env: &Env, path: &Resolved) -> Option<String> {
    after_edit_within(env, path, DIAGNOSTICS_WAIT).await
}

async fn after_edit_within(env: &Env, path: &Resolved, wait: Duration) -> Option<String> {
    let lsp = env.lsp.as_deref()?;
    if !lsp.config.enabled || !lsp.config.diagnostics_after_edit {
        return None;
    }
    let def = for_file(&lsp.servers, &path.abs)?;
    let boundary = match &path.area {
        Area::Extra { dir, .. } => dir.clone(),
        Area::Workspace(_) => env.workspace.root().to_path_buf(),
        Area::Outside => return None,
    };
    let root = find_root(def, &path.abs, &boundary);
    let key = (def.name.clone(), root);
    if !lsp.is_started(&key) {
        return None;
    }
    let server = lsp.pool.running(&key.0, &key.1)?;
    let before = server.diagnostics(&path.abs).map_or(0, |d| d.count);
    let version = match server.saved(&path.abs).await.ok()? {
        Some(version) => Some(version),
        None => {
            server.open(&path.abs).await.ok()?;
            server.version(&path.abs).await
        }
    };
    let diagnostics = if server.capabilities().diagnostic_provider.is_some() {
        let document = json!({"textDocument": {"uri": path_to_uri(&path.abs)}});
        let answer =
            tokio::time::timeout(wait, server.request("textDocument/diagnostic", document))
                .await
                .ok()?
                .ok()?;
        pulled(&answer)
    } else {
        let found = wait_for_diagnostics(&server, &path.abs, version, before, wait).await?;
        diagnostics_json(&found)
    };
    let errors = diagnostics.iter().filter(|d| severity(d) == 1).count();
    if errors == 0 {
        return None;
    }
    let s = if errors == 1 { "" } else { "s" };
    Some(format!(
        "{} reports {errors} error{s} in {}:\n{}",
        def.name,
        path.display(),
        render_diagnostics(env, &path.abs, &diagnostics, MAX_EDIT_ERRORS, false)
    ))
}

#[cfg(test)]
pub async fn after_edit_for_test(env: &Env, path: &Resolved, wait: Duration) -> Option<String> {
    after_edit_within(env, path, wait).await
}

/// The error text for the model.
fn failure(name: &str, err: &Error) -> String {
    match err {
        Error::Start(reason) | Error::Closed(reason) => {
            format!("{name} failed: {reason}; use search_files instead")
        }
        Error::Indexing { percent } => format!(
            "{name} is still indexing{}; try again later, or use search_files",
            percent.map(|p| format!(" ({p} %)")).unwrap_or_default()
        ),
        Error::Timeout(s) => {
            format!("{name}: no answer within {s} s; try again later, or use search_files")
        }
        Error::Server { message, .. } => format!("{name}: {message}"),
        Error::Protocol(text) => format!("{name}: unexpected answer: {text}"),
        Error::Io(text) => text.clone(),
    }
}

/// A place in a file, 0-based, `character` in the server's encoding.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Place {
    path: PathBuf,
    line: u32,
    character: u32,
}

fn place(uri: &Value, range: &Value) -> Option<Place> {
    let path = uri_to_path(uri.as_str()?)?;
    let start = range.get("start")?;
    Some(Place {
        path,
        line: u32::try_from(start.get("line")?.as_u64()?).ok()?,
        character: u32::try_from(start.get("character")?.as_u64()?).unwrap_or(0),
    })
}

/// `Location`, `Location[]` or `LocationLink[]`; `null` is none.
fn locations(answer: &Value) -> Vec<Place> {
    let one = |v: &Value| {
        if let Some(uri) = v.get("targetUri") {
            let range = v
                .get("targetSelectionRange")
                .or_else(|| v.get("targetRange"))?;
            place(uri, range)
        } else {
            place(v.get("uri")?, v.get("range")?)
        }
    };
    let mut found: Vec<Place> = match answer {
        Value::Array(items) => items.iter().filter_map(one).collect(),
        Value::Null => Vec::new(),
        other => one(other).into_iter().collect(),
    };
    found.sort();
    found.dedup();
    found
}

/// What a place may show: its line, only its path, or nothing.
enum Shown {
    Text(String, String),
    /// Outside the workspace and the extra directories.
    PathOnly(String),
    /// Kept from the model by the `read_file` rules.
    Skipped,
}

/// Reads the lines of the files results point to, once each, and decides
/// what the model may see of them.
struct Files<'a> {
    env: &'a Env,
    encoding: Encoding,
    texts: HashMap<PathBuf, Option<String>>,
}

impl<'a> Files<'a> {
    fn new(env: &'a Env, encoding: Encoding) -> Files<'a> {
        Files {
            env,
            encoding,
            texts: HashMap::new(),
        }
    }

    /// `src/main.rs:17:5` and the line's text, if the model may read it.
    fn show(&mut self, place: &Place) -> Shown {
        let resolved = self.env.workspace.resolve(&place.path.to_string_lossy());
        if resolved.is_outside() {
            return Shown::PathOnly(format!(
                "{}:{} (outside the workspace)",
                self.env.workspace.tilde(&place.path),
                place.line + 1
            ));
        }
        let decision = self.env.checker().check_path(Tool::ReadFile, &resolved);
        if decision.action != Action::Allow {
            return Shown::Skipped;
        }
        let text = self
            .texts
            .entry(resolved.abs.clone())
            .or_insert_with(|| fs::read_to_string(&resolved.abs).ok());
        let row = text
            .as_deref()
            .and_then(|t| {
                lines(t)
                    .get(usize::try_from(place.line).ok()?)
                    .map(|s| (*s).to_owned())
            })
            .unwrap_or_default();
        let column = from_lsp_character(&row, place.character, self.encoding) + 1;
        Shown::Text(
            format!("{}:{}:{column}", resolved.display(), place.line + 1),
            cut(row.trim(), MAX_LINE),
        )
    }

    fn locations(&mut self, op: Operation, symbol: &str, found: &[Place]) -> String {
        let mut rows = Vec::new();
        let mut skipped = 0;
        let mut more = 0;
        let mut more_files = HashSet::new();
        for place in found {
            let row = match self.show(place) {
                Shown::Skipped => {
                    skipped += 1;
                    continue;
                }
                Shown::Text(at, text) => format!("{at}: {text}"),
                Shown::PathOnly(at) => at,
            };
            if rows.len() < MAX_LOCATIONS {
                rows.push(row);
            } else {
                more += 1;
                more_files.insert(place.path.clone());
            }
        }
        let mut out = if rows.is_empty() {
            vec!["(none found)".to_owned()]
        } else {
            let mut out = vec![format!("{}:", op.heading(rows.len() + more, symbol))];
            out.append(&mut rows);
            out
        };
        if more > 0 {
            out.push(format!(
                "… {more} more in {} files; ask about a narrower symbol",
                more_files.len()
            ));
        }
        if skipped > 0 {
            out.push(skipped_text(skipped, "location"));
        }
        out.join("\n")
    }

    fn calls(&mut self, answer: &Value, incoming: bool, symbol: &str) -> String {
        let items = answer.as_array().cloned().unwrap_or_default();
        let mut rows = Vec::new();
        let mut skipped = 0;
        for call in &items {
            let item = call.get(if incoming { "from" } else { "to" });
            let Some(item) = item else { continue };
            let name = item.get("name").and_then(Value::as_str).unwrap_or("?");
            let kind = kind_name(item.get("kind").and_then(Value::as_u64).unwrap_or(0));
            let ranges = call
                .get("fromRanges")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            if incoming {
                // The calls are in the caller's file.
                let uri = item.get("uri").cloned().unwrap_or(Value::Null);
                for range in &ranges {
                    let Some(p) = place(&uri, range) else {
                        continue;
                    };
                    match self.show(&p) {
                        Shown::Skipped => skipped += 1,
                        Shown::Text(at, text) => rows.push(format!("{at}: {kind} {name}: {text}")),
                        Shown::PathOnly(at) => rows.push(format!("{at}: {kind} {name}")),
                    }
                }
            } else {
                let at = item
                    .get("uri")
                    .zip(item.get("selectionRange").or_else(|| item.get("range")))
                    .and_then(|(uri, range)| place(uri, range));
                let Some(p) = at else { continue };
                let lines: Vec<String> = ranges
                    .iter()
                    .filter_map(|r| r.get("start")?.get("line")?.as_u64())
                    .map(|l| (l + 1).to_string())
                    .collect();
                let called = if lines.is_empty() {
                    String::new()
                } else {
                    format!("; called on line {}", lines.join(", "))
                };
                match self.show(&p) {
                    Shown::Skipped => skipped += 1,
                    Shown::Text(at, _) | Shown::PathOnly(at) => {
                        rows.push(format!("{kind} {name}  {at}{called}"));
                    }
                }
            }
            if rows.len() >= MAX_LOCATIONS {
                break;
            }
        }
        let mut out = if rows.is_empty() {
            vec!["(none found)".to_owned()]
        } else {
            let what = if incoming { "callers of" } else { "calls from" };
            let mut out = vec![format!("{} {what} {symbol}:", rows.len())];
            out.extend(rows);
            out
        };
        if skipped > 0 {
            out.push(skipped_text(skipped, "call"));
        }
        out.join("\n")
    }

    fn symbols(&mut self, answer: &Value) -> String {
        let items = answer.as_array().cloned().unwrap_or_default();
        let mut rows = Vec::new();
        let mut skipped = 0;
        for item in &items {
            let name = item.get("name").and_then(Value::as_str).unwrap_or("?");
            let kind = kind_name(item.get("kind").and_then(Value::as_u64).unwrap_or(0));
            let Some(location) = item.get("location") else {
                continue;
            };
            let Some(uri) = location.get("uri") else {
                continue;
            };
            let p = match location.get("range") {
                Some(range) => place(uri, range),
                None => uri.as_str().and_then(uri_to_path).map(|path| Place {
                    path,
                    line: 0,
                    character: 0,
                }),
            };
            let Some(p) = p else { continue };
            match self.show(&p) {
                Shown::Skipped => skipped += 1,
                Shown::Text(at, _) | Shown::PathOnly(at) => {
                    if rows.len() < MAX_LOCATIONS {
                        rows.push(format!("{kind} {name}  {at}"));
                    }
                }
            }
        }
        let mut out = if rows.is_empty() {
            vec!["(none found)".to_owned()]
        } else {
            rows
        };
        if items.len() > MAX_LOCATIONS {
            out.push(format!(
                "… {} more; use a longer query",
                items.len() - MAX_LOCATIONS
            ));
        }
        if skipped > 0 {
            out.push(skipped_text(skipped, "symbol"));
        }
        out.join("\n")
    }

    /// One line per diagnostic, errors first; `with_warnings: false` lists
    /// only errors and counts the rest.
    fn diagnostics(
        &mut self,
        path: &Path,
        diagnostics: &[Value],
        max: usize,
        with_warnings: bool,
    ) -> String {
        render_diagnostics(self.env, path, diagnostics, max, with_warnings)
    }
}

fn skipped_text(n: usize, what: &str) -> String {
    let s = if n == 1 { "" } else { "s" };
    format!("({n} {what}{s} skipped by permission rules)")
}

/// Diagnostics as text, errors first. With `with_warnings`, all up to
/// `max`; without, only the errors and a count of the rest.
pub fn render_diagnostics(
    env: &Env,
    path: &Path,
    diagnostics: &[Value],
    max: usize,
    with_warnings: bool,
) -> String {
    let shown = env.workspace.resolve(&path.to_string_lossy()).display();
    let mut sorted: Vec<&Value> = diagnostics.iter().collect();
    sorted.sort_by_key(|d| {
        let line = d
            .get("range")
            .and_then(|r| r.get("start")?.get("line")?.as_u64())
            .unwrap_or(0);
        (severity(d), line)
    });
    let errors = sorted.iter().filter(|d| severity(d) == 1).count();
    let rest = sorted.len() - errors;
    let listed: Vec<String> = sorted
        .iter()
        .filter(|d| with_warnings || severity(d) == 1)
        .take(max)
        .map(|d| diagnostic_line(&shown, d))
        .collect();
    if sorted.is_empty() {
        return "(no errors or warnings)".into();
    }
    let mut out = listed;
    let total = if with_warnings { sorted.len() } else { errors };
    if total > max {
        out.push(format!("… {} more", total - max));
    }
    if !with_warnings && rest > 0 {
        let s = if rest == 1 { "" } else { "s" };
        out.push(format!(
            "(and {rest} warning{s} or note{s}; lsp diagnostics lists them)"
        ));
    }
    out.join("\n")
}

/// 1 error … 4 hint; a missing severity counts as an error.
fn severity(d: &Value) -> u64 {
    d.get("severity").and_then(Value::as_u64).unwrap_or(1)
}

fn diagnostic_line(shown: &str, d: &Value) -> String {
    let start = d.get("range").and_then(|r| r.get("start"));
    let line = start.and_then(|s| s.get("line")?.as_u64()).unwrap_or(0) + 1;
    let column = start
        .and_then(|s| s.get("character")?.as_u64())
        .unwrap_or(0)
        + 1;
    let level = match severity(d) {
        1 => "error",
        2 => "warning",
        3 => "info",
        _ => "hint",
    };
    let code = match d.get("code") {
        Some(Value::String(code)) => format!("[{code}]"),
        Some(Value::Number(code)) => format!("[{code}]"),
        _ => String::new(),
    };
    let message = d
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("")
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("; ");
    format!("{shown}:{line}:{column}: {level}{code}: {message}")
}

/// The items of a `textDocument/diagnostic` answer.
fn pulled(answer: &Value) -> Vec<Value> {
    answer
        .get("items")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// The diagnostics a server published, as JSON, for rendering.
pub fn diagnostics_json(found: &FileDiagnostics) -> Vec<Value> {
    found
        .diagnostics
        .iter()
        .filter_map(|d| serde_json::to_value(d).ok())
        .collect()
}

fn hover_text(answer: &Value) -> String {
    fn part(v: &Value) -> String {
        match v {
            Value::String(s) => s.clone(),
            Value::Object(o) => {
                let value = o.get("value").and_then(Value::as_str).unwrap_or("");
                match o.get("language").and_then(Value::as_str) {
                    Some(lang) => format!("```{lang}\n{value}\n```"),
                    None => value.to_owned(),
                }
            }
            Value::Array(items) => items.iter().map(part).collect::<Vec<_>>().join("\n\n"),
            _ => String::new(),
        }
    }
    let text = answer.get("contents").map(part).unwrap_or_default();
    let text = text.trim();
    if text.is_empty() {
        "(no information)".into()
    } else {
        cut(text, MAX_HOVER)
    }
}

/// An indented outline: `struct Parser            12-40`.
fn outline(answer: &Value) -> String {
    let mut rows: Vec<(String, String)> = Vec::new();
    fn walk(items: &[Value], depth: usize, rows: &mut Vec<(String, String)>) {
        for item in items {
            let name = item.get("name").and_then(Value::as_str).unwrap_or("?");
            let kind = kind_name(item.get("kind").and_then(Value::as_u64).unwrap_or(0));
            let range = item
                .get("range")
                .or_else(|| item.get("location").and_then(|l| l.get("range")));
            let line = |end: &str| {
                range
                    .and_then(|r| r.get(end)?.get("line")?.as_u64())
                    .map_or(0, |l| l + 1)
            };
            let (start, end) = (line("start"), line("end"));
            let lines = if start == end {
                start.to_string()
            } else {
                format!("{start}-{end}")
            };
            rows.push((format!("{}{kind} {name}", "  ".repeat(depth)), lines));
            if let Some(children) = item.get("children").and_then(Value::as_array) {
                walk(children, depth + 1, rows);
            }
        }
    }
    let items = answer.as_array().cloned().unwrap_or_default();
    walk(&items, 0, &mut rows);
    if rows.is_empty() {
        return "(no symbols)".into();
    }
    let width = rows
        .iter()
        .take(MAX_OUTLINE)
        .map(|(l, _)| l.chars().count())
        .max()
        .unwrap_or(0)
        .min(60);
    let mut out: Vec<String> = rows
        .iter()
        .take(MAX_OUTLINE)
        .map(|(left, lines)| format!("{left:<width$}  {lines}"))
        .collect();
    if rows.len() > MAX_OUTLINE {
        out.push(format!("… {} more", rows.len() - MAX_OUTLINE));
    }
    out.join("\n")
}

/// The LSP `SymbolKind` in words.
fn kind_name(kind: u64) -> &'static str {
    match kind {
        1 => "file",
        2 => "module",
        3 => "namespace",
        4 => "package",
        5 => "class",
        6 => "method",
        7 => "property",
        8 => "field",
        9 => "constructor",
        10 => "enum",
        11 => "interface",
        12 => "function",
        13 => "variable",
        14 => "constant",
        15 => "string",
        16 => "number",
        17 => "boolean",
        18 => "array",
        19 => "object",
        20 => "key",
        21 => "null",
        22 => "enum member",
        23 => "struct",
        24 => "event",
        25 => "operator",
        26 => "type parameter",
        _ => "symbol",
    }
}

/// At most `max` characters, with `…` when cut.
fn cut(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// The tool's spec.
pub fn spec() -> crate::provider::ToolSpec {
    crate::provider::ToolSpec {
        name: "lsp".into(),
        description: "Ask the language server about the code: definition, type_definition, \
             implementation, references, hover (type and docs), incoming_calls, \
             outgoing_calls of the symbol on a line; document_symbols (outline of a \
             file); workspace_symbols (find by name); diagnostics (errors of a file)."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "operation": {"type": "string", "enum": Operation::ALL.map(Operation::name)},
                "path": {"type": "string", "description": "The file. Required, except for workspace_symbols."},
                "line": {"type": "integer", "minimum": 1, "description": "Line of the symbol, as read_file shows it."},
                "symbol": {"type": "string", "description": "The name on that line to ask about."},
                "occurrence": {"type": "integer", "minimum": 1, "description": "Which occurrence of symbol on the line. Default 1."},
                "column": {"type": "integer", "minimum": 1, "description": "Column, in characters, instead of symbol."},
                "query": {"type": "string", "description": "workspace_symbols: name or part of it."},
                "include_declaration": {"type": "boolean", "description": "references: also list the definition. Default false."}
            },
            "required": ["operation"],
            "additionalProperties": false
        }),
    }
}

/// Shares the started-servers list in tests.
#[cfg(test)]
pub fn started(lsp: &LspEnv) -> Vec<Key> {
    let mut keys: Vec<Key> = lock(&lsp.started).iter().cloned().collect();
    keys.sort();
    keys
}
