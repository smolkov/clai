//! The part all HTTP adapters share: key, retries, streaming, cancellation.
//!
//! An adapter only provides a [`Codec`]: how to build the request and how to
//! decode the answer.

use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::{self, StreamExt};
use tokio::sync::{OnceCell, mpsc};
use tokio_util::sync::CancellationToken;

use super::error::{ErrorKind, ProviderError};
use super::http::{Http, PostError, StatusError};
use super::sse::{SseEvent, SseParser};
use super::types::{Event, Request};
use super::{EventStream, Provider};
use crate::config::{self, ApiKey, KEY_CMD_TIMEOUT, KeySource};

/// Request format and answer decoding of one API type.
pub(crate) trait Codec: Send + Sync + 'static {
    fn url(&self, base_url: &str) -> String;
    fn headers(&self, key: Option<&ApiKey>) -> Vec<(&'static str, String)>;
    fn encode(&self, request: &Request) -> serde_json::Value;
    fn status_error(&self, error: StatusError) -> ProviderError;
    fn decoder(&self) -> Box<dyn Decode>;
}

/// Decodes the events of one answer.
pub(crate) trait Decode: Send {
    /// Events for one SSE event.
    fn event(&mut self, event: SseEvent) -> Result<Vec<Event>, ProviderError>;
    /// The body ended; events still to send, or an error if the answer was
    /// not complete.
    fn end(&mut self) -> Result<Vec<Event>, ProviderError>;
    /// Whether `Done` was produced.
    fn done(&self) -> bool;
}

/// When and how long to wait before a retry.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RetryPolicy {
    pub max_retries: u32,
    /// First wait; doubled for each retry.
    pub base: Duration,
}

/// Longest wait between retries, also for `retry-after`.
const MAX_BACKOFF: Duration = Duration::from_secs(30);
const MAX_RETRY_AFTER: Duration = Duration::from_secs(60);

impl RetryPolicy {
    /// Wait before retry number `attempt` (0-based).
    pub fn delay(&self, attempt: u32, retry_after: Option<Duration>) -> Duration {
        if let Some(wait) = retry_after {
            return wait.min(MAX_RETRY_AFTER);
        }
        let factor = 2u32.checked_pow(attempt).unwrap_or(u32::MAX);
        self.base.saturating_mul(factor).min(MAX_BACKOFF)
    }
}

struct Inner<C> {
    name: String,
    codec: C,
    http: Http,
    base_url: String,
    key_source: KeySource,
    key: OnceCell<Option<ApiKey>>,
    retry: RetryPolicy,
}

/// A provider reached over HTTP.
pub(crate) struct Remote<C> {
    inner: Arc<Inner<C>>,
}

impl<C: Codec> Remote<C> {
    pub fn new(
        codec: C,
        provider: &config::Provider,
        retry_base: Duration,
    ) -> Result<Self, ProviderError> {
        Ok(Remote {
            inner: Arc::new(Inner {
                name: provider.name.clone(),
                codec,
                http: Http::new(provider.timeout)?,
                base_url: provider.base_url.trim_end_matches('/').to_owned(),
                key_source: provider.key.clone(),
                key: OnceCell::new(),
                retry: RetryPolicy {
                    max_retries: provider.max_retries,
                    base: retry_base,
                },
            }),
        })
    }
}

impl<C: Codec> Provider for Remote<C> {
    fn stream(&self, request: Request, cancel: CancellationToken) -> EventStream {
        let inner = Arc::clone(&self.inner);
        spawn(cancel, move |mut sink| async move {
            inner.run(&request, &mut sink).await
        })
    }
}

/// Passes events to the consumer and remembers whether any was passed.
pub(crate) struct Sink {
    tx: mpsc::Sender<Result<Event, ProviderError>>,
    sent: bool,
}

impl Sink {
    /// Says that the request is tried again; a retry is still possible
    /// after it, as nothing of the answer was sent.
    pub async fn retry(&mut self, attempt: u32, max: u32, reason: String) {
        let _ = self
            .tx
            .send(Ok(Event::Retry {
                attempt,
                max,
                reason,
            }))
            .await;
    }

    pub async fn send(&mut self, event: Event) -> Result<(), ProviderError> {
        self.sent = true;
        self.tx
            .send(Ok(event))
            .await
            .map_err(|_| ProviderError::cancelled())
    }
}

/// Runs `work` in a task and returns its events as a stream that always ends
/// with `Done` or an error, also if the task fails.
pub(crate) fn spawn<F, Fut>(cancel: CancellationToken, work: F) -> EventStream
where
    F: FnOnce(Sink) -> Fut + Send + 'static,
    Fut: Future<Output = Result<(), ProviderError>> + Send,
{
    let Ok(runtime) = tokio::runtime::Handle::try_current() else {
        return error_stream(ProviderError::new(
            ErrorKind::Unsupported,
            "provider used outside of an async runtime",
        ));
    };
    let (tx, rx) = mpsc::channel(64);
    runtime.spawn(async move {
        let sink = Sink {
            tx: tx.clone(),
            sent: false,
        };
        let result = tokio::select! {
            biased;
            () = cancel.cancelled() => Err(ProviderError::cancelled()),
            result = work(sink) => result,
        };
        if let Err(e) = result {
            let _ = tx.send(Err(e)).await;
        }
    });
    finish_stream(rx)
}

/// Ends the stream after `Done` or an error; a channel that closes before
/// either becomes a `Protocol` error.
fn finish_stream(rx: mpsc::Receiver<Result<Event, ProviderError>>) -> EventStream {
    stream::unfold(Some(rx), |rx| async move {
        let mut rx = rx?;
        match rx.recv().await {
            Some(Ok(event @ Event::Done { .. })) => Some((Ok(event), None)),
            Some(Ok(event)) => Some((Ok(event), Some(rx))),
            Some(Err(e)) => Some((Err(e), None)),
            None => Some((
                Err(ProviderError::protocol("answer ended without completing")),
                None,
            )),
        }
    })
    .boxed()
}

pub(crate) fn error_stream(error: ProviderError) -> EventStream {
    stream::iter([Err(error)]).boxed()
}

impl<C: Codec> Inner<C> {
    async fn run(&self, request: &Request, sink: &mut Sink) -> Result<(), ProviderError> {
        let key = self.key().await?;
        let body = serde_json::to_vec(&self.codec.encode(request))
            .map_err(|e| ProviderError::new(ErrorKind::InvalidRequest, e.to_string()))?;
        let mut attempt = 0;
        loop {
            match self.attempt(key.as_ref(), body.clone(), sink).await {
                Ok(()) => return Ok(()),
                Err(e) if e.kind.retryable() && !sink.sent && attempt < self.retry.max_retries => {
                    let wait = self.retry.delay(attempt, e.retry_after);
                    tracing::info!(
                        provider = %self.name,
                        error = %e,
                        wait_ms = wait.as_millis(),
                        attempt = attempt + 1,
                        "retrying"
                    );
                    sink.retry(attempt + 1, self.retry.max_retries, e.message.clone())
                        .await;
                    tokio::time::sleep(wait).await;
                    attempt += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }

    async fn attempt(
        &self,
        key: Option<&ApiKey>,
        body: Vec<u8>,
        sink: &mut Sink,
    ) -> Result<(), ProviderError> {
        let url = self.codec.url(&self.base_url);
        let headers = self.codec.headers(key);
        let mut response = match self.http.post(&url, &headers, body).await {
            Ok(response) => response,
            Err(PostError::Provider(e)) => return Err(e),
            Err(PostError::Status(status)) => return Err(self.codec.status_error(status)),
        };
        let mut parser = SseParser::new();
        let mut decoder = self.codec.decoder();
        while let Some(chunk) = self.http.chunk(&mut response).await? {
            for sse in parser.push(&chunk)? {
                for event in decoder.event(sse)? {
                    sink.send(event).await?;
                }
                if decoder.done() {
                    return Ok(());
                }
            }
        }
        for event in decoder.end()? {
            sink.send(event).await?;
        }
        if decoder.done() {
            Ok(())
        } else {
            Err(ProviderError::new(
                ErrorKind::Network,
                "connection closed before the answer was complete",
            ))
        }
    }

    /// The API key, read on first use; a failure is not kept, so a later
    /// request tries again.
    async fn key(&self) -> Result<Option<ApiKey>, ProviderError> {
        self.key
            .get_or_try_init(|| async {
                let source = self.key_source.clone();
                let resolved = tokio::task::spawn_blocking(move || {
                    source.resolve(|name| std::env::var(name).ok(), KEY_CMD_TIMEOUT)
                })
                .await
                .map_err(|e| format!("reading the API key failed: {e}"))
                .and_then(|r| r);
                resolved.map_err(|message| {
                    ProviderError::new(
                        ErrorKind::Auth,
                        format!("provider {}: {message}", self.name),
                    )
                })
            })
            .await
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff() {
        let policy = RetryPolicy {
            max_retries: 10,
            base: Duration::from_secs(1),
        };
        let secs = |attempt| policy.delay(attempt, None).as_secs();
        assert_eq!(
            [secs(0), secs(1), secs(2), secs(3), secs(4), secs(5)],
            [1, 2, 4, 8, 16, 30]
        );
        assert_eq!(secs(40), 30);
        assert_eq!(
            policy.delay(0, Some(Duration::from_secs(7))),
            Duration::from_secs(7)
        );
        assert_eq!(
            policy.delay(0, Some(Duration::from_secs(600))),
            Duration::from_secs(60)
        );
    }
}
