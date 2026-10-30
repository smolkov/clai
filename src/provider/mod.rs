//! Providers: one interface with shared types; each API type has an adapter
//! that encodes requests and decodes answers (see `spec/providers.md`).

mod anthropic;
mod error;
pub mod fake;
mod http;
mod openai;
mod remote;
mod sse;
mod types;

use std::sync::Arc;
use std::time::Duration;

use futures_util::stream::BoxStream;
use tokio_util::sync::CancellationToken;

use crate::config::{self, Api};

pub use error::{ErrorKind, ProviderError};
pub use types::{Block, Event, Message, Request, Role, StopReason, ToolSpec, parse_arguments};

/// The answer to a request: events, ending with `Done` or an error.
pub type EventStream = BoxStream<'static, Result<Event, ProviderError>>;

pub trait Provider: Send + Sync {
    /// Sends the request and returns its events. Cancelling `cancel` closes
    /// the connection and ends the stream with `Cancelled`.
    fn stream(&self, request: Request, cancel: CancellationToken) -> EventStream;
}

/// First wait before a retry; doubled each time.
const RETRY_BASE: Duration = Duration::from_secs(1);

/// The provider for a configured provider, by its `api` type.
pub fn from_config(provider: &config::Provider) -> Result<Arc<dyn Provider>, ProviderError> {
    with_retry_base(provider, RETRY_BASE)
}

fn with_retry_base(
    provider: &config::Provider,
    base: Duration,
) -> Result<Arc<dyn Provider>, ProviderError> {
    Ok(match provider.api {
        Api::Anthropic => Arc::new(remote::Remote::new(anthropic::Anthropic, provider, base)?),
        Api::OpenAiCompatible => Arc::new(remote::Remote::new(openai::OpenAi, provider, base)?),
    })
}

#[cfg(test)]
mod tests_http;
