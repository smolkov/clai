//! Where the server's socket lives (see `spec/sessions.md`, "Server").

use std::fs::{self, DirBuilder};
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// `$XDG_RUNTIME_DIR/clai`, or `/tmp/clai-$UID` without it.
pub fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        Some(dir) => dir.join("clai"),
        None => std::env::temp_dir().join(format!("clai-{}", nix::unistd::getuid())),
    }
}

pub fn socket_path(dir: &Path) -> PathBuf {
    dir.join("server.sock")
}

pub fn lock_path(dir: &Path) -> PathBuf {
    dir.join("server.lock")
}

/// Creates the directory with mode `0700`, or checks that an existing one
/// belongs to the user and is not a link, then sets its mode to `0700`.
pub fn prepare(dir: &Path) -> Result<()> {
    let io_err = Error::io(dir.display().to_string());
    match DirBuilder::new().mode(0o700).recursive(true).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(io_err(e)),
    }
    let meta = fs::symlink_metadata(dir).map_err(&io_err)?;
    if !meta.is_dir() {
        return Err(Error::Failed(format!("{}: not a directory", dir.display())));
    }
    if meta.uid() != nix::unistd::getuid().as_raw() {
        return Err(Error::Failed(format!(
            "{}: belongs to another user; remove it or set XDG_RUNTIME_DIR",
            dir.display()
        )));
    }
    if meta.mode() & 0o777 != 0o700 {
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700)).map_err(&io_err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("a/clai");
        prepare(&dir).unwrap();
        let mode = fs::metadata(&dir).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o700);
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).unwrap();
        prepare(&dir).unwrap();
        assert_eq!(fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);

        let file = tmp.path().join("file");
        fs::write(&file, "").unwrap();
        assert!(prepare(&file).is_err());
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert!(prepare(&link).is_err());
    }
}
