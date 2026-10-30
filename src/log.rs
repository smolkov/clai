//! Logging to a file, never to the terminal (see `spec/config.md`, "Logging").

use std::env;
use std::fs::{self, File, OpenOptions};
use std::path::PathBuf;
use std::sync::Mutex;

use tracing_subscriber::EnvFilter;

use crate::error::{Error, Result};

/// Environment variable with the log filter.
const LOG_ENV: &str = "CLAI_LOG";
const DEFAULT_FILTER: &str = "info";

/// Path of the log file: `$XDG_STATE_HOME/clai/clai.log`, falling back to
/// `~/.local/state/clai/clai.log`.
pub fn log_path() -> Option<PathBuf> {
    let state = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(state.join("clai").join("clai.log"))
}

/// Starts logging to the log file. Returns the path of the file, or an error
/// if it cannot be opened; the caller then continues without a log.
pub fn init() -> Result<PathBuf> {
    let path = log_path().ok_or_else(|| Error::Io {
        context: "log file".to_owned(),
        source: std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "neither XDG_STATE_HOME nor HOME is set",
        ),
    })?;
    let file = open(&path)?;
    let filter =
        EnvFilter::try_from_env(LOG_ENV).unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));
    // Fails only if a subscriber is already set; logging then goes there.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(Mutex::new(file))
        .try_init();
    Ok(path)
}

fn open(path: &PathBuf) -> Result<File> {
    let io_err = |source| Error::Io {
        context: format!("cannot open log file {}", path.display()),
        source,
    };
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io_err)?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(io_err)
}
