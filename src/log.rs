//! Logging to a file, never to the terminal (see `spec/config.md`, "Logging").

use std::env;
use std::fs::{DirBuilder, File, OpenOptions};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tracing_subscriber::EnvFilter;

use crate::error::{Error, Result};

/// Environment variable with the log filter.
const LOG_ENV: &str = "CLAI_LOG";
const DEFAULT_FILTER: &str = "info";

/// `$XDG_STATE_HOME/clai`, falling back to `~/.local/state/clai`: the log
/// and the saved sessions.
pub fn state_dir() -> Option<PathBuf> {
    let state = env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(state.join("clai"))
}

/// Path of the log file: `$XDG_STATE_HOME/clai/clai.log`, falling back to
/// `~/.local/state/clai/clai.log`.
pub fn log_path() -> Option<PathBuf> {
    Some(state_dir()?.join("clai.log"))
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

/// Opens the log for appending: the file `0600` and its directory (the
/// state directory) `0700`, narrowed if they existed wider.
fn open(path: &Path) -> Result<File> {
    let io_err = Error::io(format!("cannot open log file {}", path.display()));
    if let Some(dir) = path.parent() {
        DirBuilder::new()
            .mode(0o700)
            .recursive(true)
            .create(dir)
            .map_err(&io_err)?;
        crate::files::set_mode(dir, 0o700).map_err(&io_err)?;
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
        .map_err(&io_err)?;
    crate::files::set_mode(path, 0o600).map_err(&io_err)?;
    Ok(file)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn private_modes() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("state/clai");
        let path = dir.join("clai.log");
        open(&path).unwrap();
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&path), 0o600);
    }

    #[test]
    fn wider_modes_are_narrowed() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("clai");
        let path = dir.join("clai.log");
        fs::create_dir(&dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(&path, "old line\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        open(&path).unwrap();
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&path), 0o600);
        assert_eq!(fs::read_to_string(&path).unwrap(), "old line\n");
    }
}
