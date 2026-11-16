//! The scratch directory of a session (RFC 0029; see `spec/sessions.md`,
//! "Scratch directory"): `<runtime dir>/sessions/<key>/tmp`, private to the
//! user, made at the start of each turn when it is missing and removed with
//! the session.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// The directory below the runtime directory that holds the sessions'.
const SESSIONS: &str = "sessions";
const TMP: &str = "tmp";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scratch {
    /// The runtime directory of the server, or of `codeagent run`.
    runtime: PathBuf,
    /// The session's key: the name of its saved directory, or a name of its
    /// own for a session that is not saved.
    key: String,
}

impl Scratch {
    pub fn new(runtime: &Path, key: &str) -> Scratch {
        Scratch {
            runtime: runtime.to_path_buf(),
            key: key.to_owned(),
        }
    }

    /// The key of a session saved in the directory `dir`; `fallback` when
    /// it has no name.
    pub fn key_of(dir: &Path, fallback: &str) -> String {
        dir.file_name()
            .map_or_else(|| fallback.to_owned(), |n| n.to_string_lossy().into_owned())
    }

    /// The scratch directory itself.
    pub fn path(&self) -> PathBuf {
        self.session_dir().join(TMP)
    }

    /// `<runtime>/sessions/<key>`, removed as a whole.
    fn session_dir(&self) -> PathBuf {
        self.runtime.join(SESSIONS).join(&self.key)
    }

    /// Makes every directory on the way that is missing, with mode `0700`,
    /// and checks that each belongs to the user and is not a link.
    pub fn prepare(&self) -> Result<(), String> {
        let sessions = self.runtime.join(SESSIONS);
        let session = self.session_dir();
        for dir in [&self.runtime, &sessions, &session, &self.path()] {
            crate::server::prepare(dir).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    pub fn exists(&self) -> bool {
        fs::symlink_metadata(self.path()).is_ok_and(|m| m.is_dir())
    }

    /// Removes the directory and everything in it; links are removed, not
    /// followed. Missing is fine.
    pub fn remove(&self) -> Result<(), String> {
        remove(&self.session_dir())
    }

    /// The size of the files in it, in bytes; links are not followed.
    pub fn size(&self) -> u64 {
        self.files().iter().map(|(_, size)| size).sum()
    }

    /// The files in it, relative to it and sorted, with their sizes; links
    /// are listed, not followed.
    pub fn files(&self) -> Vec<(String, u64)> {
        let root = self.path();
        let mut out = Vec::new();
        let mut dirs = vec![root.clone()];
        while let Some(dir) = dirs.pop() {
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(meta) = fs::symlink_metadata(&path) else {
                    continue;
                };
                if meta.is_dir() {
                    dirs.push(path);
                } else if let Ok(rel) = path.strip_prefix(&root) {
                    out.push((rel.to_string_lossy().into_owned(), meta.len()));
                }
            }
        }
        out.sort();
        out
    }
}

/// Removes the scratch directories below `runtime` for which `keep` says
/// no: those of sessions that no longer exist. Returns the keys removed;
/// failures go to the log.
pub fn sweep(runtime: &Path, keep: impl Fn(&str) -> bool) -> Vec<String> {
    let Ok(entries) = fs::read_dir(runtime.join(SESSIONS)) else {
        return Vec::new();
    };
    let mut removed = Vec::new();
    for entry in entries.flatten() {
        let key = entry.file_name().to_string_lossy().into_owned();
        if keep(&key) {
            continue;
        }
        match remove(&entry.path()) {
            Ok(()) => removed.push(key),
            Err(e) => tracing::warn!("removing a stale scratch directory: {e}"),
        }
    }
    removed.sort();
    removed
}

/// The key of a `codeagent run`: `run-<pid>`.
pub fn run_key(pid: u32) -> String {
    format!("{RUN}{pid}")
}

/// Whether `key` is that of a `codeagent run` whose process still runs.
pub fn is_running_run(key: &str) -> bool {
    let Some(pid) = key.strip_prefix(RUN).and_then(|p| p.parse::<i32>().ok()) else {
        return false;
    };
    pid > 0 && nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None).is_ok()
}

const RUN: &str = "run-";

fn remove(dir: &Path) -> Result<(), String> {
    // `remove_dir_all` removes links themselves, without following them.
    match fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("{}: {e}", dir.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn mode(path: &Path) -> u32 {
        fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn made_private_and_again_when_missing() {
        let tmp = tempfile::tempdir().unwrap();
        let runtime = tmp.path().join("codeagent");
        let scratch = Scratch::new(&runtime, "abc");
        assert_eq!(scratch.path(), runtime.join("sessions/abc/tmp"));
        assert!(!scratch.exists());
        scratch.prepare().unwrap();
        assert!(scratch.exists());
        for dir in ["", "sessions", "sessions/abc", "sessions/abc/tmp"] {
            assert_eq!(mode(&runtime.join(dir)), 0o700, "{dir}");
        }
        fs::write(scratch.path().join("a.txt"), "12345").unwrap();
        fs::create_dir(scratch.path().join("d")).unwrap();
        fs::write(scratch.path().join("d/b.txt"), "1").unwrap();
        assert_eq!(
            scratch.files(),
            [("a.txt".to_owned(), 5), ("d/b.txt".to_owned(), 1)]
        );
        assert_eq!(scratch.size(), 6);
        // Made again, the content left alone.
        scratch.prepare().unwrap();
        assert_eq!(scratch.size(), 6);
        scratch.remove().unwrap();
        assert!(!runtime.join("sessions/abc").exists());
        scratch.remove().unwrap();
        scratch.prepare().unwrap();
        assert!(scratch.exists());
        assert!(scratch.files().is_empty());
    }

    #[test]
    fn a_link_on_the_way_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let runtime = tmp.path().join("codeagent");
        let other = tmp.path().join("other");
        fs::create_dir_all(runtime.join("sessions")).unwrap();
        fs::create_dir(&other).unwrap();
        symlink(&other, runtime.join("sessions/abc")).unwrap();
        assert!(Scratch::new(&runtime, "abc").prepare().is_err());
    }

    #[test]
    fn removal_does_not_follow_links() {
        let tmp = tempfile::tempdir().unwrap();
        let outside = tmp.path().join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep.txt"), "x").unwrap();
        let scratch = Scratch::new(&tmp.path().join("rt"), "s");
        scratch.prepare().unwrap();
        symlink(&outside, scratch.path().join("link")).unwrap();
        assert_eq!(
            scratch.files(),
            [("link".to_owned(), scratch_link_len(&scratch))]
        );
        scratch.remove().unwrap();
        assert!(outside.join("keep.txt").is_file());
    }

    fn scratch_link_len(scratch: &Scratch) -> u64 {
        fs::symlink_metadata(scratch.path().join("link"))
            .unwrap()
            .len()
    }

    #[test]
    fn stale_directories_are_swept() {
        let tmp = tempfile::tempdir().unwrap();
        let runtime = tmp.path();
        for key in ["a", "b", "c"] {
            Scratch::new(runtime, key).prepare().unwrap();
        }
        let mine = run_key(std::process::id());
        let gone = run_key(i32::MAX as u32);
        for key in [&mine, &gone] {
            Scratch::new(runtime, key).prepare().unwrap();
        }
        let keep = |key: &str| key == "b" || is_running_run(key);
        let mut expected = vec!["a".to_owned(), "c".to_owned(), gone.clone()];
        expected.sort();
        assert_eq!(sweep(runtime, keep), expected);
        assert!(Scratch::new(runtime, "b").exists());
        assert!(Scratch::new(runtime, &mine).exists());
        assert!(!Scratch::new(runtime, "a").exists());
        assert!(sweep(&runtime.join("none"), keep).is_empty());
    }
}
