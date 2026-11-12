//! The running language servers of a clai server (or of `clai run`), keyed
//! by server name and project root, shared by the sessions that use them.
//!
//! - A server starts on the first [`Pool::get`] for its key; callers that
//!   come while it starts wait for the same start.
//! - A server that crashed is started again once on the next `get`; after
//!   a second crash within [`CRASH_WINDOW`] it stays failed until
//!   [`Pool::restart`] or a changed `[lsp]` ([`Pool::apply_config`]). A
//!   start that fails counts as a crash.
//! - [`Pool::reap`] stops servers without requests for the idle time.

use std::collections::{BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use tokio::task::AbortHandle;

use super::server::{Server, Setup, State};
use super::{Error, Result};
use crate::config::Lsp;
use crate::sync::lock;

/// A server without requests for this long is stopped.
pub const IDLE: Duration = Duration::from_secs(10 * 60);

/// Two crashes within this time leave a server failed.
pub const CRASH_WINDOW: Duration = Duration::from_secs(5 * 60);

/// Starts a server; [`Server::start`] outside the tests.
pub type Starter = Arc<dyn Fn(Setup) -> BoxFuture<'static, Result<Server>> + Send + Sync>;

/// Server name and project root.
pub type Key = (String, PathBuf);

/// What `/lsp` shows of one server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerStatus {
    pub name: String,
    pub root: PathBuf,
    pub state: PoolState,
    /// Sessions that used it, by id, sorted.
    pub sessions: Vec<String>,
    /// The last lines of its standard error.
    pub stderr: Vec<String>,
}

/// A server's state as the pool sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PoolState {
    /// Being started; also while its slot is busy.
    Starting,
    /// Started; what it does.
    Running(State),
    /// Stopped when idle, or never started again after a crash.
    Stopped,
    /// Crashed twice within [`CRASH_WINDOW`], or failed to start twice:
    /// not started again until a restart. Why.
    Failed(String),
}

#[derive(Default)]
struct Slot {
    server: Option<Arc<Server>>,
    /// Crashes and failed starts within the window.
    crashes: VecDeque<Instant>,
    /// Set when it stays failed.
    failed: Option<String>,
    last_used: Option<Instant>,
    sessions: BTreeSet<String>,
    /// The last stderr lines of a server that is gone.
    stderr: Vec<String>,
}

struct Entry {
    setup: Setup,
    slot: tokio::sync::Mutex<Slot>,
}

struct Inner {
    entries: Mutex<IndexMap<Key, Arc<Entry>>>,
    config: Mutex<Option<Lsp>>,
    starter: Starter,
    idle: Duration,
}

/// The pool; clones share it.
#[derive(Clone)]
pub struct Pool {
    inner: Arc<Inner>,
}

impl Default for Pool {
    fn default() -> Pool {
        Pool::new()
    }
}

impl Pool {
    /// A pool that starts server processes.
    pub fn new() -> Pool {
        let starter: Starter = Arc::new(|setup: Setup| {
            Box::pin(async move { Server::start(&setup).await })
                as BoxFuture<'static, Result<Server>>
        });
        Pool::with_starter(starter, IDLE)
    }

    /// A pool with its own way to start servers and idle time; for tests.
    pub fn with_starter(starter: Starter, idle: Duration) -> Pool {
        Pool {
            inner: Arc::new(Inner {
                entries: Mutex::new(IndexMap::new()),
                config: Mutex::new(None),
                starter,
                idle,
            }),
        }
    }

    /// The server for `setup`'s name and root, started when it does not
    /// run, for `session`. The permission to start it is the caller's
    /// business.
    pub async fn get(&self, setup: &Setup, session: &str) -> Result<Arc<Server>> {
        let entry = self.entry(setup);
        let mut slot = entry.slot.lock().await;
        slot.sessions.insert(session.to_owned());
        slot.last_used = Some(Instant::now());
        if let Some(reason) = &slot.failed {
            return Err(Error::Closed(failed_text(reason)));
        }
        if let Some(server) = &slot.server {
            match server.state() {
                State::Failed(reason) => {
                    slot.stderr = server.stderr();
                    slot.server = None;
                    if crashed(&mut slot, Instant::now()) {
                        slot.failed = Some(reason.clone());
                        return Err(Error::Closed(failed_text(&reason)));
                    }
                }
                _ => return Ok(Arc::clone(server)),
            }
        }
        match (self.inner.starter)(entry.setup.clone()).await {
            Ok(server) => {
                let server = Arc::new(server);
                slot.server = Some(Arc::clone(&server));
                slot.last_used = Some(Instant::now());
                Ok(server)
            }
            Err(err) => {
                if crashed(&mut slot, Instant::now()) {
                    slot.failed = Some(err.to_string());
                }
                Err(err)
            }
        }
    }

    /// The server for a name and root if it runs and has not failed;
    /// never starts one. For diagnostics after an edit.
    pub fn running(&self, name: &str, root: &Path) -> Option<Arc<Server>> {
        let entry = lock(&self.inner.entries)
            .get(&(name.to_owned(), root.to_owned()))
            .cloned()?;
        let slot = entry.slot.try_lock().ok()?;
        let server = slot.server.as_ref()?;
        match server.state() {
            State::Failed(_) => None,
            _ => Some(Arc::clone(server)),
        }
    }

    /// Stops a server and forgets its crashes, then starts it again.
    pub async fn restart(&self, name: &str, root: &Path) -> Result<Arc<Server>> {
        let key = (name.to_owned(), root.to_owned());
        let Some(entry) = lock(&self.inner.entries).get(&key).cloned() else {
            return Err(Error::Start(format!(
                "{name} has not run for {}",
                root.display()
            )));
        };
        let mut slot = entry.slot.lock().await;
        if let Some(server) = slot.server.take() {
            server.stop().await;
        }
        slot.crashes.clear();
        slot.failed = None;
        slot.last_used = Some(Instant::now());
        match (self.inner.starter)(entry.setup.clone()).await {
            Ok(server) => {
                let server = Arc::new(server);
                slot.server = Some(Arc::clone(&server));
                Ok(server)
            }
            Err(err) => {
                crashed(&mut slot, Instant::now());
                Err(err)
            }
        }
    }

    /// Stops one server. It starts again on the next `get`.
    pub async fn stop(&self, name: &str, root: &Path) {
        let key = (name.to_owned(), root.to_owned());
        let entry = lock(&self.inner.entries).get(&key).cloned();
        if let Some(entry) = entry {
            stop_slot(&mut *entry.slot.lock().await).await;
        }
    }

    /// Stops all servers and forgets them: when the clai server or
    /// `clai run` exits, and when `[lsp]` changes.
    pub async fn stop_all(&self) {
        let entries: Vec<Arc<Entry>> = lock(&self.inner.entries)
            .drain(..)
            .map(|(_, e)| e)
            .collect();
        for entry in entries {
            stop_slot(&mut *entry.slot.lock().await).await;
        }
    }

    /// Takes the `[lsp]` of a (re)loaded configuration. When it differs
    /// from the one before, all servers stop; returns whether they did.
    pub async fn apply_config(&self, lsp: &Lsp) -> bool {
        let changed = {
            let mut config = lock(&self.inner.config);
            let changed = config.as_ref().is_some_and(|old| old != lsp);
            *config = Some(lsp.clone());
            changed
        };
        if changed {
            self.stop_all().await;
        }
        changed
    }

    /// Stops the servers without requests for the idle time. Returns
    /// their keys.
    pub async fn reap(&self) -> Vec<Key> {
        self.reap_at(Instant::now()).await
    }

    async fn reap_at(&self, now: Instant) -> Vec<Key> {
        let entries: Vec<(Key, Arc<Entry>)> = lock(&self.inner.entries)
            .iter()
            .map(|(k, e)| (k.clone(), Arc::clone(e)))
            .collect();
        let mut stopped = Vec::new();
        for (key, entry) in entries {
            // A busy slot is in use.
            let Ok(mut slot) = entry.slot.try_lock() else {
                continue;
            };
            let idle = slot
                .last_used
                .is_none_or(|t| now.saturating_duration_since(t) >= self.inner.idle);
            if slot.server.is_some() && idle {
                stop_slot(&mut slot).await;
                stopped.push(key);
            }
        }
        stopped
    }

    /// Runs [`Pool::reap`] every minute (or every tenth of the idle time,
    /// when shorter) until the handle is aborted.
    pub fn spawn_reaper(&self) -> AbortHandle {
        let pool = self.clone();
        let every =
            (self.inner.idle / 10).clamp(Duration::from_millis(10), Duration::from_secs(60));
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(every);
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tick.tick().await;
                pool.reap().await;
            }
        })
        .abort_handle()
    }

    /// A session ended: it no longer counts as a user of any server.
    pub fn forget_session(&self, session: &str) {
        for entry in lock(&self.inner.entries).values() {
            if let Ok(mut slot) = entry.slot.try_lock() {
                slot.sessions.remove(session);
            }
        }
    }

    /// The servers the pool knows, in the order they were first asked for.
    pub fn status(&self) -> Vec<ServerStatus> {
        lock(&self.inner.entries)
            .iter()
            .map(|((name, root), entry)| {
                let (state, sessions, stderr) = match entry.slot.try_lock() {
                    Err(_) => (PoolState::Starting, Vec::new(), Vec::new()),
                    Ok(slot) => {
                        let sessions = slot.sessions.iter().cloned().collect();
                        match (&slot.failed, &slot.server) {
                            (Some(reason), _) => (
                                PoolState::Failed(reason.clone()),
                                sessions,
                                slot.stderr.clone(),
                            ),
                            (None, Some(server)) => (
                                PoolState::Running(server.state()),
                                sessions,
                                server.stderr(),
                            ),
                            (None, None) => (PoolState::Stopped, sessions, slot.stderr.clone()),
                        }
                    }
                };
                ServerStatus {
                    name: name.clone(),
                    root: root.clone(),
                    state,
                    sessions,
                    stderr,
                }
            })
            .collect()
    }

    fn entry(&self, setup: &Setup) -> Arc<Entry> {
        let key = (setup.name.clone(), setup.root.clone());
        let mut entries = lock(&self.inner.entries);
        Arc::clone(entries.entry(key).or_insert_with(|| {
            Arc::new(Entry {
                setup: setup.clone(),
                slot: tokio::sync::Mutex::new(Slot::default()),
            })
        }))
    }
}

/// Counts a crash at `now`; whether it is the second within the window.
fn crashed(slot: &mut Slot, now: Instant) -> bool {
    while slot
        .crashes
        .front()
        .is_some_and(|t| now.saturating_duration_since(*t) > CRASH_WINDOW)
    {
        slot.crashes.pop_front();
    }
    slot.crashes.push_back(now);
    slot.crashes.len() >= 2
}

async fn stop_slot(slot: &mut Slot) {
    if let Some(server) = slot.server.take() {
        slot.stderr = server.stderr();
        server.stop().await;
    }
}

fn failed_text(reason: &str) -> String {
    format!("{reason}; it failed twice and stays stopped until /lsp restart")
}
