//! `ProviderError`: how a request or stream fails.

use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Key missing, wrong or not allowed.
    Auth,
    /// Too many requests.
    RateLimit,
    /// The provider is busy or failing.
    Overloaded,
    /// The request is larger than the model's context window.
    ContextTooLong,
    /// The API rejected the request.
    InvalidRequest,
    /// Connection failed or broke.
    Network,
    /// No answer within `timeout_s`.
    Timeout,
    /// The answer could not be decoded.
    Protocol,
    /// The API type or model cannot do what the request needs.
    Unsupported,
    /// The agent loop cancelled the request.
    Cancelled,
}

impl ErrorKind {
    /// Errors the adapter retries before the first event.
    pub fn retryable(self) -> bool {
        matches!(
            self,
            ErrorKind::RateLimit | ErrorKind::Overloaded | ErrorKind::Network | ErrorKind::Timeout
        )
    }
}

/// A failed request. `message` is for the user and never contains a key.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ProviderError {
    pub kind: ErrorKind,
    pub message: String,
    /// Wait time the API asked for (`retry-after`).
    pub retry_after: Option<Duration>,
}

impl ProviderError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> ProviderError {
        ProviderError {
            kind,
            message: message.into(),
            retry_after: None,
        }
    }

    pub fn protocol(message: impl Into<String>) -> ProviderError {
        ProviderError::new(ErrorKind::Protocol, message)
    }

    pub fn cancelled() -> ProviderError {
        ProviderError::new(ErrorKind::Cancelled, "request cancelled")
    }
}
