//! A language server in the same process, for tests: it answers with a
//! handler and logs every message it gets.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use tokio::io::{DuplexStream, ReadHalf, WriteHalf, duplex, split};

use super::pool::Starter;
use super::server::{Server, Setup};
use super::transport::{Connection, Incoming};

pub type Answer = Option<Result<Value, (i64, String)>>;
pub type Handler = Arc<dyn Fn(&str, &Value) -> Answer + Send + Sync>;

/// A language server in the same process. It answers with `handler`
/// (`None`: no answer) and logs every message it gets.
pub struct Fake {
    pub conn: Connection,
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
    pub fn start(handler: Handler) -> (Fake, ReadHalf<DuplexStream>, WriteHalf<DuplexStream>) {
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

    pub fn methods(&self) -> Vec<String> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .map(|(m, _)| m.clone())
            .collect()
    }

    /// The params of the last message with `method`, waiting for it.
    pub async fn wait_for(&self, method: &str) -> Value {
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

pub fn capabilities() -> Value {
    json!({
        "positionEncoding": "utf-8",
        "textDocumentSync": {"openClose": true, "change": 1, "save": {"includeText": false}},
        "definitionProvider": true,
        "referencesProvider": true,
    })
}

/// Answers `initialize`, `shutdown`, and `test/echo` with its params;
/// anything else gets no answer.
pub fn standard() -> Handler {
    Arc::new(|method, params| match method {
        "initialize" => Some(Ok(json!({"capabilities": capabilities()}))),
        "shutdown" => Some(Ok(Value::Null)),
        "test/echo" => Some(Ok(params.clone())),
        _ => None,
    })
}

/// Starts a fake with `handler` for each start, and keeps them.
#[derive(Clone)]
pub struct Fakes {
    pub fakes: Arc<Mutex<Vec<Option<Fake>>>>,
    handler: Arc<Mutex<Handler>>,
}

impl Fakes {
    pub fn new(handler: Handler) -> Fakes {
        Fakes {
            fakes: Arc::new(Mutex::new(Vec::new())),
            handler: Arc::new(Mutex::new(handler)),
        }
    }

    /// Later starts answer with this handler.
    pub fn set_handler(&self, handler: Handler) {
        *self.handler.lock().unwrap() = handler;
    }

    pub fn starter(&self) -> Starter {
        let fakes = self.clone();
        Arc::new(move |setup: Setup| {
            let fakes = fakes.clone();
            Box::pin(async move {
                let handler = fakes.handler.lock().unwrap().clone();
                let (fake, reader, writer) = Fake::start(handler);
                fakes.fakes.lock().unwrap().push(Some(fake));
                Server::over(&setup, reader, writer).await
            })
        })
    }

    pub fn starts(&self) -> usize {
        self.fakes.lock().unwrap().len()
    }

    /// Drops the last fake: its server sees the connection close.
    pub fn crash_last(&self) {
        if let Some(slot) = self.fakes.lock().unwrap().last_mut() {
            slot.take();
        }
    }

    /// The methods the last fake got.
    pub fn last_methods(&self) -> Vec<String> {
        self.fakes
            .lock()
            .unwrap()
            .last()
            .and_then(|f| f.as_ref().map(Fake::methods))
            .unwrap_or_default()
    }
}
