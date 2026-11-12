//! Language servers (RFC 0019): the client side of the Language Server
//! Protocol, used by the `lsp` tool.
//!
//! - [`transport`]: `Content-Length` framing and JSON-RPC over any stream.
//! - [`server`]: one server process: start, initialize, progress,
//!   diagnostics, server requests, stop.
//! - [`docs`]: open files and their sync, and positions in the server's
//!   encoding.
//! - [`config`]: the built-in servers, the servers in use, project roots.
//! - [`pool`]: the running servers, shared by sessions, stopped when idle.

pub mod config;
pub mod docs;
pub mod pool;
pub mod server;
pub mod transport;

#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
mod tests;

/// Errors of talking to a language server. They never end a session; the
/// tool turns them into an error result.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The connection is gone: the server exited, crashed or sent a
    /// malformed message. The text says why.
    #[error("{0}")]
    Closed(String),

    /// No answer within the timeout.
    #[error("no answer within {0} s")]
    Timeout(u64),

    /// No answer within the timeout while the server reports work, such
    /// as indexing.
    #[error("still indexing{}", percent.map(|p| format!(" ({p} %)")).unwrap_or_default())]
    Indexing { percent: Option<u32> },

    /// The server answered with an error.
    #[error("{message}")]
    Server { code: i64, message: String },

    /// The server's answer has an unexpected shape.
    #[error("unexpected answer: {0}")]
    Protocol(String),

    /// The server could not be started.
    #[error("{0}")]
    Start(String),

    /// A file could not be read.
    #[error("{0}")]
    Io(String),
}

/// `Result` of this module.
pub type Result<T> = std::result::Result<T, Error>;
