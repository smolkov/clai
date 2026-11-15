//! Logging to a file, never to the terminal (see `spec/config.md`, "Logging").

use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::MakeWriter;

use crate::error::{Error, Result};
use crate::paths::Paths;
use crate::startup::{LOG_ENV, Startup};
use crate::sync::lock;

const DEFAULT_FILTER: &str = "info";

/// Which process writes a log line: all of them write one file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A command that talks to the server, or none.
    Client,
    Server,
    /// `clai run`: a turn in its own process.
    Run,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Client => "client",
            Role::Server => "server",
            Role::Run => "run",
        }
    }

    /// Whether a log that cannot be opened is worth a warning: the server
    /// and `clai run` do work to look back on; a client does not.
    pub fn warns(self) -> bool {
        self != Role::Client
    }
}

/// Starts logging to the log file of `paths`, each line after the role and
/// the process id (`server[4242] …`). Returns the path of the file, or an
/// error if it cannot be opened; the caller then goes on without a log.
pub fn init(paths: &Paths, startup: &Startup, role: Role) -> Result<PathBuf> {
    let path = paths.log.clone().ok_or_else(|| Error::Io {
        context: "log file".to_owned(),
        source: io::Error::new(
            io::ErrorKind::NotFound,
            "neither XDG_STATE_HOME nor HOME is set",
        ),
    })?;
    let file = open(&path)?;
    let filter = startup
        .var(LOG_ENV)
        .and_then(|f| EnvFilter::try_new(f).ok())
        .unwrap_or_else(|| EnvFilter::new(DEFAULT_FILTER));
    let writer = Prefixed {
        file: Mutex::new(file),
        prefix: format!("{}[{}] ", role.name(), std::process::id()),
    };
    // Fails only if a subscriber is already set; logging then goes there.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(writer)
        .try_init();
    Ok(path)
}

/// The log file, with a prefix before each line. The formatter writes one
/// event with one writer, so the prefix goes first.
struct Prefixed<W> {
    file: Mutex<W>,
    prefix: String,
}

struct Line<'a, W> {
    file: MutexGuard<'a, W>,
    prefix: &'a str,
    started: bool,
}

impl<'a, W: Write + 'a> MakeWriter<'a> for Prefixed<W> {
    type Writer = Line<'a, W>;

    fn make_writer(&'a self) -> Self::Writer {
        Line {
            file: lock(&self.file),
            prefix: &self.prefix,
            started: false,
        }
    }
}

impl<W: Write> Write for Line<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if !self.started {
            self.file.write_all(self.prefix.as_bytes())?;
            self.started = true;
        }
        self.file.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
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
    fn lines_say_which_process_wrote_them() {
        let writer = Prefixed {
            file: Mutex::new(Vec::new()),
            prefix: "server[42] ".to_owned(),
        };
        writer.make_writer().write_all(b"one\n").unwrap();
        {
            let mut line = writer.make_writer();
            line.write_all(b"two ").unwrap();
            line.write_all(b"parts\n").unwrap();
        }
        let text = String::from_utf8(lock(&writer.file).clone()).unwrap();
        assert_eq!(text, "server[42] one\nserver[42] two parts\n");
        assert!(Role::Server.warns() && Role::Run.warns() && !Role::Client.warns());
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
