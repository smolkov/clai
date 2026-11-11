//! JSON-RPC over `Content-Length` framing (the base protocol of LSP).
//!
//! A [`Connection`] runs two tasks: the writer sends queued messages; the
//! reader routes responses to the waiting [`Connection::request`] and
//! hands notifications and server requests to the receiver returned by
//! [`Connection::new`]. A malformed message or the end of the stream
//! closes the connection: every waiting request fails with
//! [`Error::Closed`], and the receiver ends.

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::io::{
    AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader,
};
use tokio::sync::{mpsc, oneshot};
use tokio::task::AbortHandle;

use super::{Error, Result};
use crate::sync::lock;

/// Largest message body accepted.
pub const MAX_BODY: usize = 64 << 20;

/// Longest header line accepted, with its line end.
pub const MAX_HEADER_LINE: usize = 1024;

/// "Method not found", for server requests `clai` does not handle.
pub const METHOD_NOT_FOUND: i64 = -32601;

/// A message from the server that is not a response.
#[derive(Debug, Clone, PartialEq)]
pub enum Incoming {
    Notification {
        method: String,
        params: Value,
    },
    /// Must be answered with [`Connection::respond`].
    Request {
        id: Value,
        method: String,
        params: Value,
    },
}

/// Reads one framed message. `Ok(None)` at the end of the stream before a
/// message starts.
pub async fn read_message<R: AsyncBufRead + Unpin>(reader: &mut R) -> Result<Option<Value>> {
    let mut length: Option<usize> = None;
    let mut first = true;
    loop {
        let mut line = Vec::new();
        let n = (&mut *reader)
            .take(MAX_HEADER_LINE as u64)
            .read_until(b'\n', &mut line)
            .await
            .map_err(|e| Error::Closed(format!("read failed: {e}")))?;
        if n == 0 {
            if first {
                return Ok(None);
            }
            return Err(Error::Closed("stream ended in a header".into()));
        }
        first = false;
        if line.last() != Some(&b'\n') {
            return Err(Error::Closed(format!(
                "malformed message: header line over {MAX_HEADER_LINE} bytes"
            )));
        }
        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches(['\r', '\n']);
        if text.is_empty() {
            break;
        }
        let Some((name, value)) = text.split_once(':') else {
            return Err(Error::Closed(format!("malformed message: header {text:?}")));
        };
        if name.trim().eq_ignore_ascii_case("content-length") {
            let value = value.trim();
            let parsed = value.parse::<usize>().map_err(|_| {
                Error::Closed(format!("malformed message: Content-Length {value:?}"))
            })?;
            length = Some(parsed);
        }
    }
    let Some(length) = length else {
        return Err(Error::Closed("malformed message: no Content-Length".into()));
    };
    if length > MAX_BODY {
        return Err(Error::Closed(format!(
            "malformed message: body of {length} bytes is over {MAX_BODY}"
        )));
    }
    let mut body = vec![0u8; length];
    reader
        .read_exact(&mut body)
        .await
        .map_err(|e| Error::Closed(format!("stream ended in a message body: {e}")))?;
    let value = serde_json::from_slice(&body)
        .map_err(|e| Error::Closed(format!("malformed message: {e}")))?;
    Ok(Some(value))
}

/// Writes one framed message.
pub async fn write_message<W: AsyncWrite + Unpin>(writer: &mut W, message: &Value) -> Result<()> {
    let body = serde_json::to_vec(message).map_err(|e| Error::Protocol(e.to_string()))?;
    let header = format!("Content-Length: {}\r\n\r\n", body.len());
    let io = |e: std::io::Error| Error::Closed(format!("write failed: {e}"));
    writer.write_all(header.as_bytes()).await.map_err(io)?;
    writer.write_all(&body).await.map_err(io)?;
    writer.flush().await.map_err(io)
}

type Reply = oneshot::Sender<Result<Value>>;

#[derive(Default)]
struct Pending {
    waiting: HashMap<i64, Reply>,
    /// Set when the connection closed; the reason.
    closed: Option<String>,
}

impl Pending {
    fn close(&mut self, reason: &str) {
        if self.closed.is_none() {
            self.closed = Some(reason.to_string());
        }
        for (_, reply) in self.waiting.drain() {
            let _ = reply.send(Err(Error::Closed(reason.to_string())));
        }
    }
}

struct Shared {
    pending: Mutex<Pending>,
}

struct Inner {
    out: mpsc::UnboundedSender<Value>,
    shared: Arc<Shared>,
    next_id: AtomicI64,
    tasks: Vec<AbortHandle>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// A JSON-RPC connection. Cheap to clone; the tasks stop when the last
/// clone is dropped or the stream ends.
#[derive(Clone)]
pub struct Connection {
    inner: Arc<Inner>,
}

impl Connection {
    /// Starts the reader and writer tasks. Must be called inside a `tokio`
    /// runtime.
    pub fn new<R, W>(reader: R, writer: W) -> (Connection, mpsc::UnboundedReceiver<Incoming>)
    where
        R: AsyncRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (out_tx, out_rx) = mpsc::unbounded_channel::<Value>();
        let (in_tx, in_rx) = mpsc::unbounded_channel::<Incoming>();
        let shared = Arc::new(Shared {
            pending: Mutex::new(Pending::default()),
        });
        let writer_task = tokio::spawn(write_loop(writer, out_rx, Arc::clone(&shared)));
        let reader_task = tokio::spawn(read_loop(
            BufReader::new(reader),
            in_tx,
            Arc::clone(&shared),
        ));
        let inner = Inner {
            out: out_tx,
            shared,
            next_id: AtomicI64::new(1),
            tasks: vec![writer_task.abort_handle(), reader_task.abort_handle()],
        };
        (
            Connection {
                inner: Arc::new(inner),
            },
            in_rx,
        )
    }

    /// Why the connection closed, once it has.
    pub fn closed(&self) -> Option<String> {
        lock(&self.inner.shared.pending).closed.clone()
    }

    /// Sends a request and waits for its response, at most `timeout`. On
    /// a timeout, or when the future is dropped, the server gets
    /// `$/cancelRequest`.
    pub async fn request(&self, method: &str, params: Value, timeout: Duration) -> Result<Value> {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply_tx, reply_rx) = oneshot::channel();
        {
            let mut pending = lock(&self.inner.shared.pending);
            if let Some(reason) = &pending.closed {
                return Err(Error::Closed(reason.clone()));
            }
            pending.waiting.insert(id, reply_tx);
        }
        let mut guard = CancelOnDrop {
            connection: self,
            id,
            armed: true,
        };
        self.send(json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        let answer = tokio::time::timeout(timeout, reply_rx).await;
        match answer {
            Ok(Ok(result)) => {
                guard.armed = false;
                result
            }
            // The reader dropped the reply: the connection closed.
            Ok(Err(_)) => {
                guard.armed = false;
                Err(Error::Closed(
                    self.closed()
                        .unwrap_or_else(|| "the connection closed".into()),
                ))
            }
            Err(_) => Err(Error::Timeout(timeout_secs(timeout))),
        }
    }

    /// Sends a notification.
    pub fn notify(&self, method: &str, params: Value) -> Result<()> {
        self.send(json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    /// Answers a server request: a result, or an error with code and
    /// message.
    pub fn respond(
        &self,
        id: Value,
        answer: std::result::Result<Value, (i64, String)>,
    ) -> Result<()> {
        let message = match answer {
            Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Err((code, message)) => {
                json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
            }
        };
        self.send(message)
    }

    fn send(&self, message: Value) -> Result<()> {
        if let Some(reason) = self.closed() {
            return Err(Error::Closed(reason));
        }
        self.inner
            .out
            .send(message)
            .map_err(|_| Error::Closed("the connection closed".into()))
    }
}

/// Whole seconds, rounded up, for messages.
fn timeout_secs(timeout: Duration) -> u64 {
    u64::try_from(timeout.as_millis().div_ceil(1000)).unwrap_or(u64::MAX)
}

/// Withdraws a request whose answer is no longer awaited.
struct CancelOnDrop<'a> {
    connection: &'a Connection,
    id: i64,
    armed: bool,
}

impl Drop for CancelOnDrop<'_> {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let was_waiting = lock(&self.connection.inner.shared.pending)
            .waiting
            .remove(&self.id)
            .is_some();
        if was_waiting {
            let _ = self
                .connection
                .notify("$/cancelRequest", json!({"id": self.id}));
        }
    }
}

async fn write_loop<W: AsyncWrite + Unpin>(
    mut writer: W,
    mut queue: mpsc::UnboundedReceiver<Value>,
    shared: Arc<Shared>,
) {
    while let Some(message) = queue.recv().await {
        if let Err(err) = write_message(&mut writer, &message).await {
            lock(&shared.pending).close(&err.to_string());
            return;
        }
    }
}

async fn read_loop<R: AsyncBufRead + Unpin>(
    mut reader: R,
    incoming: mpsc::UnboundedSender<Incoming>,
    shared: Arc<Shared>,
) {
    let reason = loop {
        let message = match read_message(&mut reader).await {
            Ok(Some(message)) => message,
            Ok(None) => break "the server closed the connection".to_string(),
            Err(err) => break err.to_string(),
        };
        match route(message) {
            Ok(Routed::Response(id, result)) => {
                let reply = lock(&shared.pending).waiting.remove(&id);
                if let Some(reply) = reply {
                    let _ = reply.send(result);
                }
            }
            Ok(Routed::Incoming(message)) => {
                let _ = incoming.send(message);
            }
            Err(err) => break err.to_string(),
        }
    };
    tracing::debug!(%reason, "language server connection closed");
    lock(&shared.pending).close(&reason);
}

enum Routed {
    Response(i64, Result<Value>),
    Incoming(Incoming),
}

fn route(message: Value) -> Result<Routed> {
    let Value::Object(mut map) = message else {
        return Err(Error::Closed("malformed message: not an object".into()));
    };
    let method = map.remove("method");
    let id = map.remove("id");
    let params = map.remove("params").unwrap_or(Value::Null);
    match (method, id) {
        (Some(Value::String(method)), Some(id)) => {
            Ok(Routed::Incoming(Incoming::Request { id, method, params }))
        }
        (Some(Value::String(method)), None) => {
            Ok(Routed::Incoming(Incoming::Notification { method, params }))
        }
        (None, Some(id)) => {
            // Ids are ours, so integers; a response to another id is
            // dropped by the caller.
            let id = id.as_i64().unwrap_or(-1);
            if let Some(error) = map.remove("error") {
                let code = error.get("code").and_then(Value::as_i64).unwrap_or(0);
                let message = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("request failed")
                    .to_string();
                Ok(Routed::Response(id, Err(Error::Server { code, message })))
            } else {
                Ok(Routed::Response(
                    id,
                    Ok(map.remove("result").unwrap_or(Value::Null)),
                ))
            }
        }
        _ => Err(Error::Closed(
            "malformed message: neither request, notification nor response".into(),
        )),
    }
}
