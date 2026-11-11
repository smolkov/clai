//! The background server (see `spec/sessions.md`, "Server"): a Unix
//! socket, the session registry, idle exit and configuration reload.

mod paths;
mod registry;
mod requests;

use std::collections::{BTreeSet, HashMap};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use nix::fcntl::{Flock, FlockArg};
use serde_json::Value;
use tokio::io::BufReader;
use tokio::net::{UnixListener, UnixStream};
use tokio_util::sync::CancellationToken;

use crate::agent::State;
use crate::config::{self, LoadOptions};
use crate::error::{Error, Result};
use crate::protocol::{
    self, ClientHello, ClientMessage, ErrorReply, Notice, ServerHello, ServerMessage, VERSION,
};
use crate::provider::{self, Provider};
use crate::session::store;
use crate::session::{ClientId, Outbox};
use crate::sync::lock;

use registry::{Entry, Registry, Slot};

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
    /// Where sessions are saved: `$XDG_STATE_HOME/clai/sessions`.
    pub sessions_dir: PathBuf,
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
    pub fn new(dir: PathBuf, sessions_dir: PathBuf, load: LoadOptions) -> Options {
        Options {
            dir,
            sessions_dir,
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

struct Shared {
    sessions_dir: PathBuf,
    load: LoadOptions,
    providers: ProviderFactory,
    config: Mutex<UserConfig>,
    sessions: Mutex<Registry>,
    clients: Mutex<HashMap<ClientId, Outbox>>,
    next_session: AtomicU64,
    next_client: AtomicU64,
    questions: Arc<AtomicU64>,
    stop: CancellationToken,
    /// The summaries of the session logs for `global_stats`, by directory,
    /// with the size and time of the log they were counted from.
    summaries: Mutex<HashMap<PathBuf, Counted>>,
}

/// A log counted for `global_stats`.
struct Counted {
    mark: Option<(u64, SystemTime)>,
    summary: Arc<crate::session::global::Summary>,
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
    let lock_file = File::create(lock_path(&options.dir))
        .map_err(Error::io(lock_path(&options.dir).display().to_string()))?;
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
    let listener = UnixListener::bind(&socket).map_err(Error::io(socket.display().to_string()))?;
    tracing::info!(socket = %socket.display(), pid = std::process::id(), "server started");

    let shared = Arc::new(Shared {
        sessions_dir: options.sessions_dir.clone(),
        load: options.load.clone(),
        providers: options.providers.clone(),
        config: Mutex::new(user),
        sessions: Mutex::new(Registry::default()),
        clients: Mutex::new(HashMap::new()),
        next_session: AtomicU64::new(0),
        next_client: AtomicU64::new(0),
        questions: Arc::new(AtomicU64::new(0)),
        stop: options.stop.clone(),
        summaries: Mutex::new(HashMap::new()),
    });
    load_saved(&shared);
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
    // Running sessions stop; their files stay, so they can be resumed.
    let handles = lock(&shared.sessions).handles();
    let kills = handles
        .iter()
        .map(|handle| async move { handle.kill("server stopped", true).await });
    futures_util::future::join_all(kills).await;
    // Give the connections a moment to send what is queued.
    tokio::time::sleep(Duration::from_millis(100)).await;
    drop(listener);
    let _ = std::fs::remove_file(&socket);
    tracing::info!("server stopped");
    Ok(())
}

/// Lists the saved sessions; names taken twice (e.g. by an import) are
/// made unique.
fn load_saved(shared: &Shared) {
    let mut sessions = lock(&shared.sessions);
    for (store, mut meta) in store::list(&shared.sessions_dir) {
        let name = sessions.unique_name(&meta.name);
        if name != meta.name {
            meta.name.clone_from(&name);
            if let Err(e) = store.write_meta(&meta) {
                tracing::warn!("renaming saved session: {e}");
            }
        }
        let id = shared.next_session.fetch_add(1, Ordering::Relaxed) + 1;
        sessions.insert(
            id,
            Entry {
                name,
                store: Some(store),
                slot: Slot::Saved(Box::new(meta)),
            },
        );
    }
    tracing::info!(saved = sessions.len(), "saved sessions listed");
}

/// Whether a running session works or waits for an answer.
async fn busy(shared: &Shared) -> usize {
    let handles = lock(&shared.sessions).handles();
    let mut busy = 0;
    for handle in handles {
        if handle
            .info()
            .await
            .is_some_and(|info| matches!(info.state, State::Working | State::Waiting))
        {
            busy += 1;
        }
    }
    busy
}

fn broadcast(shared: &Shared, notice: Notice) {
    for outbox in lock(&shared.clients).values() {
        outbox.send(ServerMessage::Notice {
            notice: notice.clone(),
        });
    }
}

/// Stops the server once it has had no clients and no working or waiting
/// session for `idle`; idle sessions are saved and resume later.
async fn idle_watch(shared: Arc<Shared>, idle: Duration) {
    let tick = (idle / 4).min(Duration::from_secs(1));
    let mut since: Option<Instant> = None;
    loop {
        tokio::select! {
            () = shared.stop.cancelled() => return,
            () = tokio::time::sleep(tick) => {}
        }
        let empty = lock(&shared.clients).is_empty() && busy(&shared).await == 0;
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
        refresh(&shared);
    }
}

/// Reads the user file again if it changed since it was read, and tells the
/// clients. Called by the poll and before a request uses the configuration,
/// so a file saved just now counts at once (RFC 0021).
fn refresh(shared: &Shared) {
    let (path, old) = {
        let user = lock(&shared.config);
        (user.path.clone(), user.stamp)
    };
    let now = stamp(&path);
    if now == old {
        return;
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
        // Another caller read it meanwhile: its result stands.
        if user.stamp != old {
            return;
        }
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
    broadcast(shared, notice);
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
        if let Some(handle) = lock(&shared.sessions).get(*session).and_then(Entry::handle) {
            handle.detach(id);
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

#[cfg(test)]
mod tests;
