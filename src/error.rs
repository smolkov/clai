//! Crate error type.

use std::io;

/// Errors of `clai`. Usage errors are reported by the argument parser and
/// never reach this type.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A command or feature that is not built yet.
    #[error("'{what}' is not implemented yet")]
    Unsupported { what: &'static str },

    /// A file system or I/O operation failed.
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: io::Error,
    },
}

impl Error {
    /// Process exit code for this error (see `spec/command.md`, "Exit codes").
    pub fn exit_code(&self) -> u8 {
        match self {
            Error::Unsupported { .. } | Error::Io { .. } => 1,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_message_and_exit_code() {
        let err = Error::Unsupported { what: "ls" };
        assert_eq!(err.to_string(), "'ls' is not implemented yet");
        assert_eq!(err.exit_code(), 1);
    }
}
