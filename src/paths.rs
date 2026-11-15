//! Every directory and file `codeagent` uses, from the [`Startup`] (RFC 0026;
//! see `spec/config.md`, "Files", and `spec/sessions.md`, "Server").

use std::path::{Path, PathBuf};

use crate::startup::Startup;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Paths {
    /// `--config FILE`, else `$XDG_CONFIG_HOME/codeagent/config.toml`, else
    /// `~/.config/codeagent/config.toml`; `None` without either variable.
    pub user_file: Option<PathBuf>,
    /// `$XDG_STATE_HOME/codeagent`, else `~/.local/state/codeagent`.
    pub state: Option<PathBuf>,
    /// The saved sessions, in the state directory.
    pub sessions: Option<PathBuf>,
    /// The log file, in the state directory.
    pub log: Option<PathBuf>,
    /// `$XDG_RUNTIME_DIR/codeagent`, else `/tmp/codeagent-$UID`.
    pub runtime: PathBuf,
    pub socket: PathBuf,
    pub lock: PathBuf,
}

impl Paths {
    pub fn new(startup: &Startup, config_file: Option<&Path>) -> Paths {
        let user_file = match config_file {
            Some(file) => Some(file.to_path_buf()),
            None => startup
                .xdg_config_home
                .clone()
                .or_else(|| startup.home.as_ref().map(|h| h.join(".config")))
                .map(|base| base.join("codeagent").join("config.toml")),
        };
        let state = startup
            .xdg_state_home
            .clone()
            .or_else(|| startup.home.as_ref().map(|h| h.join(".local/state")))
            .map(|base| base.join("codeagent"));
        let runtime = match &startup.xdg_runtime_dir {
            Some(dir) => dir.join("codeagent"),
            None => startup.temp_dir.join(format!("codeagent-{}", startup.uid)),
        };
        Paths {
            user_file,
            sessions: state.as_ref().map(|s| s.join("sessions")),
            log: state.as_ref().map(|s| s.join("codeagent.log")),
            state,
            socket: runtime.join("server.sock"),
            lock: runtime.join("server.lock"),
            runtime,
        }
    }

    /// The paths of the server's runtime directory `dir`, for a server or
    /// client told where it is (tests).
    pub fn in_runtime(dir: &Path) -> Paths {
        Paths {
            socket: dir.join("server.sock"),
            lock: dir.join("server.lock"),
            runtime: dir.to_path_buf(),
            ..Paths::default()
        }
    }
}

/// The name of the directories before 0.5.0, when `codeagent` was `clai`.
const OLD_NAME: &str = "clai";
const NAME: &str = "codeagent";

/// What [`move_old_dirs`] did with one directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Moved {
    /// Renamed from the first path to the second.
    Done(PathBuf, PathBuf),
    /// Left where it is, and why.
    Kept(PathBuf, String),
}

/// Moves the configuration and state directories of `clai` to their new
/// names, once: only when the old one is a directory and the new one does
/// not exist. The state stays while a server of `clai` still runs, as it
/// writes there. Never fails; problems are in the result.
pub fn move_old_dirs(startup: &Startup) -> Vec<Moved> {
    let config = startup
        .xdg_config_home
        .clone()
        .or_else(|| startup.home.as_ref().map(|h| h.join(".config")));
    let state = startup
        .xdg_state_home
        .clone()
        .or_else(|| startup.home.as_ref().map(|h| h.join(".local/state")));
    let old_runtime = match &startup.xdg_runtime_dir {
        Some(dir) => dir.join(OLD_NAME),
        None => startup.temp_dir.join(format!("{OLD_NAME}-{}", startup.uid)),
    };
    let mut moved = Vec::new();
    if let Some(base) = config {
        move_dir(&base.join(OLD_NAME), &base.join(NAME), None, &mut moved);
    }
    if let Some(base) = state {
        let running = old_server_runs(&old_runtime.join("server.sock"));
        move_dir(
            &base.join(OLD_NAME),
            &base.join(NAME),
            running.then_some("a clai server still runs; stop it with `clai server stop`"),
            &mut moved,
        );
    }
    moved
}

fn move_dir(old: &Path, new: &Path, keep: Option<&str>, moved: &mut Vec<Moved>) {
    let is_dir = std::fs::symlink_metadata(old).is_ok_and(|m| m.is_dir());
    if !is_dir || std::fs::symlink_metadata(new).is_ok() {
        return;
    }
    if let Some(why) = keep {
        moved.push(Moved::Kept(old.to_path_buf(), why.to_owned()));
        return;
    }
    match std::fs::rename(old, new) {
        Ok(()) => moved.push(Moved::Done(old.to_path_buf(), new.to_path_buf())),
        Err(e) => moved.push(Moved::Kept(old.to_path_buf(), e.to_string())),
    }
}

/// Whether a server answers on the socket.
fn old_server_runs(socket: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home(root: &Path) -> Startup {
        let mut s = Startup::from_vars([("HOME", root.as_os_str())]);
        s.temp_dir = root.join("tmp");
        s
    }

    #[test]
    fn old_dirs_move_once() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".config/clai")).unwrap();
        std::fs::write(root.join(".config/clai/config.toml"), "x").unwrap();
        std::fs::create_dir_all(root.join(".local/state/clai/sessions")).unwrap();
        let moved = move_old_dirs(&home(root));
        assert_eq!(
            moved,
            [
                Moved::Done(root.join(".config/clai"), root.join(".config/codeagent")),
                Moved::Done(
                    root.join(".local/state/clai"),
                    root.join(".local/state/codeagent")
                ),
            ]
        );
        assert!(root.join(".config/codeagent/config.toml").is_file());
        assert!(root.join(".local/state/codeagent/sessions").is_dir());
        // Once: nothing is left to move.
        assert!(move_old_dirs(&home(root)).is_empty());
        // A new directory is never replaced.
        std::fs::create_dir_all(root.join(".config/clai")).unwrap();
        assert!(move_old_dirs(&home(root)).is_empty());
        assert!(root.join(".config/clai").is_dir());
    }

    #[test]
    fn the_state_stays_while_an_old_server_runs() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join(".local/state/clai")).unwrap();
        let s = home(root);
        let runtime = root.join("tmp").join(format!("clai-{}", s.uid));
        std::fs::create_dir_all(&runtime).unwrap();
        let _listener =
            std::os::unix::net::UnixListener::bind(runtime.join("server.sock")).unwrap();
        let moved = move_old_dirs(&s);
        assert!(
            matches!(&moved[..], [Moved::Kept(p, why)] if p.ends_with(".local/state/clai") && why.contains("clai server stop")),
            "{moved:?}"
        );
        assert!(root.join(".local/state/clai").is_dir());
    }

    #[test]
    fn from_xdg_variables() {
        let mut s = Startup::from_vars([
            ("HOME", "/home/u"),
            ("XDG_CONFIG_HOME", "/cfg"),
            ("XDG_STATE_HOME", "/st"),
            ("XDG_RUNTIME_DIR", "/run/user/7"),
        ]);
        s.uid = 7;
        let p = Paths::new(&s, None);
        assert_eq!(
            p.user_file.as_deref(),
            Some(Path::new("/cfg/codeagent/config.toml"))
        );
        assert_eq!(p.state.as_deref(), Some(Path::new("/st/codeagent")));
        assert_eq!(
            p.sessions.as_deref(),
            Some(Path::new("/st/codeagent/sessions"))
        );
        assert_eq!(
            p.log.as_deref(),
            Some(Path::new("/st/codeagent/codeagent.log"))
        );
        assert_eq!(p.runtime, Path::new("/run/user/7/codeagent"));
        assert_eq!(p.socket, Path::new("/run/user/7/codeagent/server.sock"));
        assert_eq!(p.lock, Path::new("/run/user/7/codeagent/server.lock"));
    }

    #[test]
    fn from_home_and_temp() {
        let mut s = Startup::from_vars([("HOME", "/home/u"), ("XDG_STATE_HOME", "rel")]);
        s.uid = 1000;
        s.temp_dir = PathBuf::from("/var/tmp");
        let p = Paths::new(&s, None);
        assert_eq!(
            p.user_file.as_deref(),
            Some(Path::new("/home/u/.config/codeagent/config.toml"))
        );
        assert_eq!(
            p.state.as_deref(),
            Some(Path::new("/home/u/.local/state/codeagent"))
        );
        assert_eq!(p.runtime, Path::new("/var/tmp/codeagent-1000"));
        let p = Paths::new(&s, Some(Path::new("/x/c.toml")));
        assert_eq!(p.user_file.as_deref(), Some(Path::new("/x/c.toml")));
    }

    #[test]
    fn without_home() {
        let p = Paths::new(&Startup::default(), None);
        assert_eq!(p.user_file, None);
        assert_eq!(p.state, None);
        assert_eq!(p.log, None);
        assert_eq!(p.runtime, Path::new("/tmp/codeagent-0"));
    }
}
