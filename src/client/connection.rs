//! A connection that never blocks its user, for the TUI: a reader and a
//! writer task own the socket; responses come back by id, events and notices
//! through [`Connection::next`].

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::error::Result;
use crate::protocol::{self, ClientMessage, Payload, Request, ServerMessage};
use crate::sync::lock;

use super::{Client, outcome};

/// Waiting requests by id; `None` once the socket is gone.
type Pending = Arc<Mutex<Option<HashMap<u64, oneshot::Sender<Result<Payload>>>>>>;

pub struct Connection {
    outgoing: mpsc::UnboundedSender<ClientMessage>,
    pending: Pending,
    next_id: u64,
    messages: mpsc::UnboundedReceiver<ServerMessage>,
    /// Ends both tasks when the connection is dropped or a write fails.
    stop: CancellationToken,
}

impl Connection {
    /// Moves the socket of `client` into a reader and a writer task. Events
    /// the client read while waiting for responses come first.
    pub fn spawn(client: Client) -> Connection {
        let Client {
            reader,
            writer,
            next_id,
            backlog,
            ..
        } = client;
        let (outgoing, to_write) = mpsc::unbounded_channel();
        let (tx, messages) = mpsc::unbounded_channel();
        for message in backlog {
            let _ = tx.send(message);
        }
        let pending: Pending = Arc::new(Mutex::new(Some(HashMap::new())));
        let stop = CancellationToken::new();
        tokio::spawn(write_task(writer, to_write, stop.clone()));
        tokio::spawn(read_task(reader, tx, pending.clone(), stop.clone()));
        Connection {
            outgoing,
            pending,
            next_id,
            messages,
            stop,
        }
    }

    /// Sends a request without waiting. The receiver gets the response; it
    /// fails if the connection is lost first.
    pub fn request(&mut self, request: Request) -> oneshot::Receiver<Result<Payload>> {
        self.next_id += 1;
        let id = self.next_id;
        let (reply, response) = oneshot::channel();
        // With the socket gone, `reply` is dropped and `response` fails.
        if let Some(pending) = lock(&self.pending).as_mut() {
            pending.insert(id, reply);
            let _ = self.outgoing.send(ClientMessage::Request { id, request });
        }
        response
    }

    /// The next event or notice; `None` when the connection is lost.
    pub async fn next(&mut self) -> Option<ServerMessage> {
        self.messages.recv().await
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}

async fn write_task(
    mut writer: OwnedWriteHalf,
    mut messages: mpsc::UnboundedReceiver<ClientMessage>,
    stop: CancellationToken,
) {
    loop {
        let message = tokio::select! {
            () = stop.cancelled() => return,
            message = messages.recv() => message,
        };
        let Some(message) = message else { return };
        if let Err(e) = protocol::write(&mut writer, &message).await {
            tracing::info!("writing to the server: {e}");
            stop.cancel();
            return;
        }
    }
}

async fn read_task(
    mut reader: tokio::io::BufReader<OwnedReadHalf>,
    messages: mpsc::UnboundedSender<ServerMessage>,
    pending: Pending,
    stop: CancellationToken,
) {
    loop {
        let read = tokio::select! {
            () = stop.cancelled() => break,
            read = protocol::read::<ServerMessage, _>(&mut reader) => read,
        };
        match read {
            Ok(Some(ServerMessage::Response { id, result })) => {
                let waiting = lock(&pending).as_mut().and_then(|p| p.remove(&id));
                if let Some(reply) = waiting {
                    let _ = reply.send(outcome(result));
                }
            }
            Ok(Some(message)) => {
                if messages.send(message).is_err() {
                    break;
                }
            }
            Ok(None) => break,
            Err(e) => {
                tracing::info!("reading from the server: {e}");
                break;
            }
        }
    }
    // Fails all waiting requests; dropping `messages` ends `next`.
    lock(&pending).take();
    stop.cancel();
}
