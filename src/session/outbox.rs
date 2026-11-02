//! The queue of messages to one client connection.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::protocol::ServerMessage;

/// Live messages a client may have waiting before the server drops it
/// (see `spec/architecture.md`, "Concurrency").
pub const MAX_QUEUED: usize = 4096;

pub type ClientId = u64;

/// Where messages for one client go. Sending never blocks; a client that
/// falls too far behind is closed and reattaches.
#[derive(Debug, Clone)]
pub struct Outbox {
    pub id: ClientId,
    /// `tui pts/3`, `clai chat`.
    pub name: String,
    tx: mpsc::UnboundedSender<ServerMessage>,
    queued: Arc<AtomicUsize>,
    closed: CancellationToken,
    max: usize,
}

/// The connection's end of an [`Outbox`].
#[derive(Debug)]
pub struct OutboxReceiver {
    rx: mpsc::UnboundedReceiver<ServerMessage>,
    queued: Arc<AtomicUsize>,
}

impl Outbox {
    pub fn new(id: ClientId, name: String) -> (Outbox, OutboxReceiver) {
        Outbox::with_limit(id, name, MAX_QUEUED)
    }

    pub fn with_limit(id: ClientId, name: String, max: usize) -> (Outbox, OutboxReceiver) {
        let (tx, rx) = mpsc::unbounded_channel();
        let queued = Arc::new(AtomicUsize::new(0));
        (
            Outbox {
                id,
                name,
                tx,
                queued: queued.clone(),
                closed: CancellationToken::new(),
                max,
            },
            OutboxReceiver { rx, queued },
        )
    }

    /// A live event or notice. If the client is too far behind, the
    /// connection is closed instead.
    pub fn send(&self, message: ServerMessage) {
        if self.queued.load(Ordering::Relaxed) >= self.max {
            if !self.closed.is_cancelled() {
                tracing::warn!(client = %self.name, "client too slow; closing its connection");
                self.closed.cancel();
            }
            return;
        }
        self.send_always(message);
    }

    /// A response or the replayed log: never dropped for being behind.
    pub fn send_always(&self, message: ServerMessage) {
        if self.closed.is_cancelled() {
            return;
        }
        self.queued.fetch_add(1, Ordering::Relaxed);
        if self.tx.send(message).is_err() {
            self.queued.fetch_sub(1, Ordering::Relaxed);
        }
    }

    /// Cancelled when the connection must close.
    pub fn closed(&self) -> CancellationToken {
        self.closed.clone()
    }
}

impl OutboxReceiver {
    pub async fn recv(&mut self) -> Option<ServerMessage> {
        let message = self.rx.recv().await?;
        self.queued.fetch_sub(1, Ordering::Relaxed);
        Some(message)
    }
}
