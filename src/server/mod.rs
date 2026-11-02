//! The background server (see `spec/sessions.md`, "Server"): a Unix
//! socket, the session registry, idle exit and configuration reload.

mod paths;

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant, SystemTime};

use nix::fcntl::{Flock, FlockArg};
use serde_json::Value;
use tokio::io::BufReader;
use tokio::net::{UnixListener, UnixStream};
use tokio_util::sync::CancellationToken;

use crate::agent::{Agent, Setup, system_prompt, today};
use crate::config::{self, Config, LoadOptions};
use crate::error::{Error, Result};
use crate::permissions::{SessionRules, Workspace};
use crate::protocol::{
    self, ClientHello, ClientMessage, ErrorReply, Notice, Outcome, Payload, Request, ServerHello,
    ServerMessage, SessionInfo, Status, VERSION,
};
use crate::provider::{self, Provider};
use crate::session::{self, ClientId, Outbox, SessionHandle, Spec};
use crate::tools::Env;

pub use paths::{lock_path, prepare, runtime_dir, socket_path};

/// Makes the provider for a configured provider; tests give a fake one.
pub type ProviderFactory =
    Arc<dyn Fn(&config::Provider) -> std::result::Result<Arc<dyn Provider>, String> + Send + Sync>;

/// The real providers.
pub fn real_providers() -> ProviderFactory {
    Arc::new(|p: &config::Provider| provider::from_config(p).map_err(|e| e.to_string()))
}

/// How long a client may take to say hello.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

pub struct Options {
    /// The runtime directory with the socket.
    pub dir: PathBuf,
    /// Exit after this long without sessions and clients.
    pub idle: Duration,
    /// How often the user config file is checked for changes.
    pub reload_every: Duration,
    /// `--config` and the environment; the workspace is set per session.
    pub load: LoadOptions,
    pub providers: ProviderFactory,
    /// Cancel to stop the server.
    pub stop: CancellationToken,
}

impl Options {
    pub fn new(dir: PathBuf, load: LoadOptions) -> Options {
        Options {
            dir,
            idle: Duration::from_secs(60),
            reload_every: Duration::from_secs(2),
            load,
            providers: real_providers(),
            stop: CancellationToken::new(),
        }
    }
}

/// The text of the user file the server uses: the last valid one.
struct UserConfig {
    path: PathBuf,
    text: String,
    /// Modification time and size when last read.
    stamp: Option<(SystemTime, u64)>,
}

struct Entry {
    name: String,
    handle: SessionHandle,
}

struct Shared {
    load: LoadOptions,
    providers: ProviderFactory,
    config: Mutex<UserConfig>,
    sessions: Mutex<BTreeMap<u64, Entry>>,
    clients: Mutex<HashMap<ClientId, Outbox>>,
    next_session: AtomicU64,
    next_client: AtomicU64,
    questions: Arc<AtomicU64>,
    stop: CancellationToken,
}

/// A poisoned lock means a bug in another task; the data is still usable.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn stamp(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// The configuration errors as lines.
fn error_lines(e: &Error) -> Vec<String> {
    match e {
        Error::Config(errors) => errors.0.iter().map(ToString::to_string).collect(),
        other => vec![other.to_string()],
    }
}

/// Runs the server until it is stopped or idle. Fails if another server
/// runs or the configuration is invalid.
pub async fn serve(options: Options) -> Result<()> {
    prepare(&options.dir)?;
    let lock_file = File::create(lock_path(&options.dir)).map_err(|source| Error::Io {
        context: lock_path(&options.dir).display().to_string(),
        source,
    })?;
    let _lock = match Flock::lock(lock_file, FlockArg::LockExclusiveNonblock) {
        Ok(lock) => lock,
        Err((_, nix::errno::Errno::EWOULDBLOCK)) => {
            return Err(Error::Failed("a server is already running".to_owned()));
        }
        Err((_, errno)) => {
            return Err(Error::Io {
                context: lock_path(&options.dir).display().to_string(),
                source: std::io::Error::from(errno),
            });
        }
    };

    // The configuration must be valid to start.
    let (path, text) = config::read_user_file(&options.load)?;
    let base = LoadOptions {
        workspace: options.dir.clone(),
        ..options.load.clone()
    };
    config::load_with_user_text(&path, &text, &base)?;
    let user = UserConfig {
        stamp: stamp(&path),
        path,
        text,
    };

    let socket = socket_path(&options.dir);
    match std::fs::remove_file(&socket) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => {
            return Err(Error::Io {
                context: socket.display().to_string(),
                source,
            });
        }
    }
    let listener = UnixListener::bind(&socket).map_err(|source| Error::Io {
        context: socket.display().to_string(),
        source,
    })?;
    tracing::info!(socket = %socket.display(), pid = std::process::id(), "server started");

    let shared = Arc::new(Shared {
        load: options.load.clone(),
        providers: options.providers.clone(),
        config: Mutex::new(user),
        sessions: Mutex::new(BTreeMap::new()),
        clients: Mutex::new(HashMap::new()),
        next_session: AtomicU64::new(0),
        next_client: AtomicU64::new(0),
        questions: Arc::new(AtomicU64::new(0)),
        stop: options.stop.clone(),
    });
    tokio::spawn(idle_watch(shared.clone(), options.idle));
    tokio::spawn(reload_watch(shared.clone(), options.reload_every));

    loop {
        tokio::select! {
            () = shared.stop.cancelled() => break,
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    tokio::spawn(connection(shared.clone(), stream));
                }
                Err(e) => {
                    tracing::warn!("accept failed: {e}");
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            },
        }
    }

    tracing::info!("server stopping");
    broadcast(&shared, Notice::Stopping);
    let handles: Vec<SessionHandle> = lock(&shared.sessions)
        .values()
        .map(|e| e.handle.clone())
        .collect();
    let kills = handles
        .iter()
        .map(|handle| async move { handle.kill("server stopped").await });
    futures_util::future::join_all(kills).await;
    // Give the connections a moment to send what is queued.
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(listener);
    let _ = std::fs::remove_file(&socket);
    tracing::info!("server stopped");
    Ok(())
}

fn broadcast(shared: &Shared, notice: Notice) {
    for outbox in lock(&shared.clients).values() {
        outbox.send(ServerMessage::Notice {
            notice: notice.clone(),
        });
    }
}

/// Stops the server once it has had no sessions and no clients for `idle`.
async fn idle_watch(shared: Arc<Shared>, idle: Duration) {
    let tick = (idle / 4).min(Duration::from_secs(1));
    let mut since: Option<Instant> = None;
    loop {
        tokio::select! {
            () = shared.stop.cancelled() => return,
            () = tokio::time::sleep(tick) => {}
        }
        let empty = lock(&shared.sessions).is_empty() && lock(&shared.clients).is_empty();
        if !empty {
            since = None;
            continue;
        }
        let start = *since.get_or_insert_with(Instant::now);
        if start.elapsed() >= idle {
            tracing::info!("idle; stopping");
            shared.stop.cancel();
            return;
        }
    }
}

/// Reloads the user file when it changes; an invalid one is rejected and
/// the previous text stays.
async fn reload_watch(shared: Arc<Shared>, every: Duration) {
    loop {
        tokio::select! {
            () = shared.stop.cancelled() => return,
            () = tokio::time::sleep(every) => {}
        }
        let (path, old) = {
            let user = lock(&shared.config);
            (user.path.clone(), user.stamp)
        };
        let now = stamp(&path);
        if now == old {
            continue;
        }
        let checked = match std::fs::read_to_string(&path) {
            Ok(text) => {
                let options = LoadOptions {
                    workspace: path.parent().map(Path::to_path_buf).unwrap_or_default(),
                    ..shared.load.clone()
                };
                match config::load_with_user_text(&path, &text, &options) {
                    Ok(_) => Ok(text),
                    Err(e) => Err(error_lines(&e)),
                }
            }
            Err(e) => Err(vec![format!("{}: cannot read: {e}", path.display())]),
        };
        let notice = {
            let mut user = lock(&shared.config);
            user.stamp = now;
            match checked {
                Ok(text) => {
                    user.text = text;
                    tracing::info!("configuration reloaded");
                    Notice::ConfigReloaded
                }
                Err(errors) => {
                    tracing::warn!(?errors, "configuration rejected");
                    Notice::ConfigRejected { errors }
                }
            }
        };
        broadcast(&shared, notice);
    }
}

/// One client connection: handshake, then requests until it closes.
async fn connection(shared: Arc<Shared>, stream: UnixStream) {
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let hello = match tokio::time::timeout(HELLO_TIMEOUT, hello(&mut reader)).await {
        Ok(Ok(hello)) => hello,
        Ok(Err(message)) => {
            let _ = protocol::write(&mut write, &ServerMessage::Error { message }).await;
            return;
        }
        Err(_) => return,
    };
    let server_hello = ServerMessage::Hello(ServerHello {
        version: VERSION,
        pid: std::process::id(),
    });
    if protocol::write(&mut write, &server_hello).await.is_err() {
        return;
    }

    let id = shared.next_client.fetch_add(1, Ordering::Relaxed) + 1;
    let (outbox, mut out_rx) = Outbox::new(id, hello.name.clone());
    let closed = outbox.closed();
    lock(&shared.clients).insert(id, outbox.clone());
    tracing::debug!(client = %hello.name, "client connected");

    let writer_closed = closed.clone();
    let writer = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            if protocol::write(&mut write, &message).await.is_err() {
                break;
            }
        }
        writer_closed.cancel();
    });

    let mut conn = Conn {
        shared: shared.clone(),
        outbox,
        answers: hello.answers,
        attached: BTreeSet::new(),
    };
    loop {
        let message = tokio::select! {
            () = closed.cancelled() => break,
            message = protocol::read::<ClientMessage, _>(&mut reader) => message,
        };
        match message {
            Ok(Some(ClientMessage::Request { id, request })) => {
                if let Some(result) = conn.request(id, request).await {
                    conn.outbox
                        .send_always(ServerMessage::Response { id, result });
                }
            }
            Ok(Some(ClientMessage::Hello(_))) => {
                conn.outbox.send_always(ServerMessage::Error {
                    message: "hello sent twice".to_owned(),
                });
            }
            Ok(None) => break,
            Err(e) => {
                tracing::debug!(client = %hello.name, "bad message: {e}");
                conn.outbox.send_always(ServerMessage::Error {
                    message: e.to_string(),
                });
                break;
            }
        }
    }

    for session in &conn.attached {
        if let Some(entry) = lock(&shared.sessions).get(session) {
            entry.handle.detach(id);
        }
    }
    lock(&shared.clients).remove(&id);
    // Let the writer send what is queued (an error, the last response).
    drop(conn);
    let _ = tokio::time::timeout(Duration::from_millis(200), writer).await;
    tracing::debug!(client = %hello.name, "client disconnected");
}

/// Reads the client's hello and checks its version.
async fn hello(
    reader: &mut BufReader<tokio::net::unix::OwnedReadHalf>,
) -> std::result::Result<ClientHello, String> {
    let value: Value = match protocol::read(reader).await {
        Ok(Some(value)) => value,
        Ok(None) => return Err("no hello".to_owned()),
        Err(e) => return Err(e.to_string()),
    };
    let version = value.get("version").and_then(Value::as_u64);
    if version != Some(u64::from(VERSION)) {
        let theirs = version.map_or_else(|| "none".to_owned(), |v| v.to_string());
        return Err(version_mismatch(VERSION, &theirs));
    }
    match serde_json::from_value(value) {
        Ok(ClientMessage::Hello(hello)) => Ok(hello),
        Ok(_) => Err("the first message must be hello".to_owned()),
        Err(e) => Err(format!("invalid hello: {e}")),
    }
}

/// The error for clients and servers of different versions.
pub fn version_mismatch(server: u32, client: &str) -> String {
    format!(
        "the server speaks protocol version {server} and this client version {client}; \
         restart the server with `clai server stop`"
    )
}

/// The state of one connection.
struct Conn {
    shared: Arc<Shared>,
    outbox: Outbox,
    answers: bool,
    attached: BTreeSet<u64>,
}

type Reply = std::result::Result<Payload, ErrorReply>;

impl Conn {
    fn find(&self, name: &str) -> std::result::Result<SessionHandle, ErrorReply> {
        lock(&self.shared.sessions)
            .values()
            .find(|e| e.name == name)
            .map(|e| e.handle.clone())
            .ok_or_else(|| ErrorReply::failed(format!("no session named {name}")))
    }

    /// Handles one request. `None` when the session sends the response
    /// itself (`attach`).
    async fn request(&mut self, id: u64, request: Request) -> Option<Outcome> {
        let reply: Reply = match request {
            Request::Attach { session } => match self.find(&session) {
                Ok(handle) => match handle.attach(self.outbox.clone(), self.answers, id) {
                    Ok(()) => {
                        self.attached.insert(handle.id);
                        return None;
                    }
                    Err(e) => Err(ErrorReply::failed(e)),
                },
                Err(e) => Err(e),
            },
            Request::New {
                workspace,
                name,
                model,
            } => self.new_session(workspace, name, model).await,
            Request::List => Ok(Payload::Sessions {
                sessions: self.list().await,
            }),
            Request::Kill { session } => match self.find(&session) {
                Ok(handle) => {
                    handle.kill("killed").await;
                    lock(&self.shared.sessions).remove(&handle.id);
                    Ok(Payload::Done)
                }
                Err(e) => Err(e),
            },
            Request::Rename { session, name } => self.rename(&session, name),
            Request::Detach { session } => self.find(&session).map(|handle| {
                handle.detach(self.outbox.id);
                self.attached.remove(&handle.id);
                Payload::Done
            }),
            Request::Send { session, text } => {
                if text.trim().is_empty() {
                    Err(ErrorReply::usage("the message is empty"))
                } else {
                    self.find(&session).and_then(|handle| {
                        handle
                            .message(text)
                            .map(|()| Payload::Done)
                            .map_err(ErrorReply::failed)
                    })
                }
            }
            Request::SetModel { session, model } => self.set_model(&session, &model),
            Request::Cancel { session } => self.find(&session).and_then(|handle| {
                handle
                    .cancel()
                    .map(|()| Payload::Done)
                    .map_err(ErrorReply::failed)
            }),
            Request::Answer {
                session,
                question,
                answer,
            } => match self.find(&session) {
                Ok(handle) => handle
                    .answer(question, answer, self.outbox.name.clone())
                    .await
                    .map(|()| Payload::Done)
                    .map_err(ErrorReply::failed),
                Err(e) => Err(e),
            },
            Request::Status => Ok(Payload::Status(Status {
                pid: std::process::id(),
                sessions: lock(&self.shared.sessions).len(),
                clients: lock(&self.shared.clients).len(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
            })),
            Request::Stop => {
                tracing::info!(client = %self.outbox.name, "stop requested");
                self.shared.stop.cancel();
                Ok(Payload::Done)
            }
        };
        Some(match reply {
            Ok(payload) => Outcome::Ok(payload),
            Err(e) => Outcome::Error(e),
        })
    }

    async fn list(&self) -> Vec<SessionInfo> {
        let handles: Vec<SessionHandle> = lock(&self.shared.sessions)
            .values()
            .map(|e| e.handle.clone())
            .collect();
        let mut sessions = Vec::new();
        for handle in handles {
            if let Some(info) = handle.info().await {
                sessions.push(info);
            }
        }
        sessions
    }

    /// The configuration for a workspace: the kept user file and the
    /// workspace's project file.
    fn config(&self, workspace: &Path) -> std::result::Result<Config, ErrorReply> {
        let (path, text) = {
            let user = lock(&self.shared.config);
            (user.path.clone(), user.text.clone())
        };
        let options = LoadOptions {
            workspace: workspace.to_path_buf(),
            ..self.shared.load.clone()
        };
        config::load_with_user_text(&path, &text, &options)
            .map_err(|e| ErrorReply::usage(error_lines(&e).join("\n")))
    }

    async fn new_session(
        &mut self,
        workspace: PathBuf,
        name: Option<String>,
        model: Option<String>,
    ) -> Reply {
        if !workspace.is_absolute() || !workspace.is_dir() {
            return Err(ErrorReply::usage(format!(
                "{}: not a directory",
                workspace.display()
            )));
        }
        let config = self.config(&workspace)?;
        let model_ref = match &model {
            Some(text) => config.resolve_model(text).map_err(ErrorReply::usage)?,
            None => config.default_model.clone(),
        };
        let Some((provider_config, model_config)) = config.model(&model_ref) else {
            return Err(ErrorReply::usage(format!("unknown model `{model_ref}`")));
        };
        let provider = (self.shared.providers)(provider_config)
            .map_err(|e| ErrorReply::failed(format!("provider {}: {e}", provider_config.name)))?;
        let ws = Workspace::new(
            &workspace,
            &config.permissions.extra_dirs,
            self.shared.load.env.home.clone(),
        )
        .map_err(|e| ErrorReply::failed(format!("{}: {e}", workspace.display())))?;
        let place = ws.tilde(ws.root());
        let path = ws.root().to_path_buf();
        let mut agent = Agent::new(Setup {
            provider,
            model: model_config.clone(),
            limits: config.limits,
            system: system_prompt(&ws, &today()),
            env: Env {
                permissions: Arc::new(config.permissions.clone()),
                session: Arc::new(SessionRules::default()),
                workspace: Arc::new(ws),
            },
        });
        agent.share_question_ids(self.shared.questions.clone());

        let id = self.shared.next_session.fetch_add(1, Ordering::Relaxed) + 1;
        let handle = {
            let mut sessions = lock(&self.shared.sessions);
            let name = match name {
                Some(name) => {
                    check_name(&name)?;
                    if sessions.values().any(|e| e.name == name) {
                        return Err(ErrorReply::failed(format!("the name {name} is taken")));
                    }
                    name
                }
                None => unique_name(&default_name(&path), &sessions),
            };
            let weak = Arc::downgrade(&self.shared);
            let handle = session::spawn(
                Spec {
                    id,
                    name: name.clone(),
                    place,
                    path,
                    model: model_ref.to_string(),
                    agent,
                    created: crate::agent::now_secs(),
                },
                move |id| {
                    if let Some(shared) = weak.upgrade() {
                        lock(&shared.sessions).remove(&id);
                    }
                },
            );
            sessions.insert(
                id,
                Entry {
                    name,
                    handle: handle.clone(),
                },
            );
            handle
        };
        let info = handle.info().await.ok_or_else(session::gone)?;
        tracing::info!(session = %info.name, model = %info.model, "session created");
        broadcast(
            &self.shared,
            Notice::SessionCreated {
                session: Box::new(info.clone()),
            },
        );
        Ok(Payload::Session {
            session: Box::new(info),
        })
    }

    fn rename(&self, session: &str, name: String) -> Reply {
        check_name(&name)?;
        let mut sessions = lock(&self.shared.sessions);
        if sessions.values().any(|e| e.name == name) {
            return Err(ErrorReply::failed(format!("the name {name} is taken")));
        }
        let entry = sessions
            .values_mut()
            .find(|e| e.name == session)
            .ok_or_else(|| ErrorReply::failed(format!("no session named {session}")))?;
        entry
            .handle
            .rename(name.clone())
            .map_err(ErrorReply::failed)?;
        entry.name = name;
        Ok(Payload::Done)
    }

    fn set_model(&self, session: &str, model: &str) -> Reply {
        let handle = self.find(session)?;
        // Models come only from the user file; any workspace will do.
        let config = self.config(&std::env::temp_dir())?;
        let model_ref = config.resolve_model(model).map_err(ErrorReply::usage)?;
        let Some((provider_config, model_config)) = config.model(&model_ref) else {
            return Err(ErrorReply::usage(format!("unknown model `{model_ref}`")));
        };
        let provider = (self.shared.providers)(provider_config)
            .map_err(|e| ErrorReply::failed(format!("provider {}: {e}", provider_config.name)))?;
        handle
            .set_model(model_ref.to_string(), provider, model_config.clone())
            .map_err(ErrorReply::failed)?;
        Ok(Payload::Done)
    }
}

/// Names are one word: no spaces or control characters, at most 64
/// characters, not starting with `-`.
fn check_name(name: &str) -> std::result::Result<(), ErrorReply> {
    let ok = !name.is_empty()
        && name.chars().count() <= 64
        && !name.starts_with('-')
        && !name.chars().any(|c| c.is_whitespace() || c.is_control());
    if ok {
        Ok(())
    } else {
        Err(ErrorReply::usage(format!(
            "invalid session name `{name}`: one word of at most 64 characters, not starting with -"
        )))
    }
}

/// The workspace directory's name as a session name.
fn default_name(workspace: &Path) -> String {
    let name: String = workspace
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_whitespace() || c.is_control() {
                '-'
            } else {
                c
            }
        })
        .take(60)
        .collect();
    let name = name.trim_start_matches('-');
    if name.is_empty() {
        "session".to_owned()
    } else {
        name.to_owned()
    }
}

/// `name`, else `name-2`, `name-3`, …
fn unique_name(name: &str, sessions: &BTreeMap<u64, Entry>) -> String {
    let taken = |n: &str| sessions.values().any(|e| e.name == n);
    if !taken(name) {
        return name.to_owned();
    }
    (2u64..)
        .map(|i| format!("{name}-{i}"))
        .find(|n| !taken(n))
        .unwrap_or_else(|| name.to_owned())
}

#[cfg(test)]
mod tests;
