//! Crate error type.

use std::io;
use std::path::PathBuf;

use crate::config::{ConfigErrors, EXAMPLE};

/// Errors of `clai`. Errors in the command-line syntax are reported by the
/// argument parser and never reach this type.
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

    /// The configuration is invalid; one entry per problem.
    #[error("{0}")]
    Config(ConfigErrors),

    /// There is no user config file.
    #[error(
        "no configuration file at {}\n\nCreate it with at least one provider and model, for example:\n\n{}",
        path.display(),
        EXAMPLE.trim_end()
    )]
    NoUserConfig { path: PathBuf },

    /// An argument value is invalid, e.g. an unknown model in `--model`.
    #[error("{0}")]
    Usage(String),

    /// A task or request failed; the message says why.
    #[error("{0}")]
    Failed(String),

    /// The command failed and has already said why on standard error; only
    /// the exit code is left.
    #[error("exit code {0}")]
    Exit(u8),

    /// The API key of a provider could not be read.
    #[error("provider {provider}: {message}")]
    ApiKey { provider: String, message: String },
}

impl Error {
    /// Process exit code for this error (see `spec/command.md`, "Exit codes").
    pub fn exit_code(&self) -> u8 {
        match self {
            Error::Unsupported { .. }
            | Error::Io { .. }
            | Error::ApiKey { .. }
            | Error::Failed(_) => 1,
            Error::Exit(code) => *code,
            Error::Config(_) | Error::NoUserConfig { .. } | Error::Usage(_) => 2,
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

    #[test]
    fn config_errors_exit_2() {
        assert_eq!(Error::Config(ConfigErrors(Vec::new())).exit_code(), 2);
        assert_eq!(
            Error::NoUserConfig {
                path: PathBuf::from("x")
            }
            .exit_code(),
            2
        );
        assert_eq!(Error::Usage(String::new()).exit_code(), 2);
    }
}
