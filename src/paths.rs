//! Every directory and file `clai` uses, from the [`Startup`] (RFC 0026;
//! see `spec/config.md`, "Files", and `spec/sessions.md`, "Server").

use std::path::{Path, PathBuf};

use crate::startup::Startup;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Paths {
    /// `--config FILE`, else `$XDG_CONFIG_HOME/clai/config.toml`, else
    /// `~/.config/clai/config.toml`; `None` without either variable.
    pub user_file: Option<PathBuf>,
    /// `$XDG_STATE_HOME/clai`, else `~/.local/state/clai`.
    pub state: Option<PathBuf>,
    /// The saved sessions, in the state directory.
    pub sessions: Option<PathBuf>,
    /// The log file, in the state directory.
    pub log: Option<PathBuf>,
    /// `$XDG_RUNTIME_DIR/clai`, else `/tmp/clai-$UID`.
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
                .map(|base| base.join("clai").join("config.toml")),
        };
        let state = startup
            .xdg_state_home
            .clone()
            .or_else(|| startup.home.as_ref().map(|h| h.join(".local/state")))
            .map(|base| base.join("clai"));
        let runtime = match &startup.xdg_runtime_dir {
            Some(dir) => dir.join("clai"),
            None => startup.temp_dir.join(format!("clai-{}", startup.uid)),
        };
        Paths {
            user_file,
            sessions: state.as_ref().map(|s| s.join("sessions")),
            log: state.as_ref().map(|s| s.join("clai.log")),
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

#[cfg(test)]
mod tests {
    use super::*;

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
            Some(Path::new("/cfg/clai/config.toml"))
        );
        assert_eq!(p.state.as_deref(), Some(Path::new("/st/clai")));
        assert_eq!(p.sessions.as_deref(), Some(Path::new("/st/clai/sessions")));
        assert_eq!(p.log.as_deref(), Some(Path::new("/st/clai/clai.log")));
        assert_eq!(p.runtime, Path::new("/run/user/7/clai"));
        assert_eq!(p.socket, Path::new("/run/user/7/clai/server.sock"));
        assert_eq!(p.lock, Path::new("/run/user/7/clai/server.lock"));
    }

    #[test]
    fn from_home_and_temp() {
        let mut s = Startup::from_vars([("HOME", "/home/u"), ("XDG_STATE_HOME", "rel")]);
        s.uid = 1000;
        s.temp_dir = PathBuf::from("/var/tmp");
        let p = Paths::new(&s, None);
        assert_eq!(
            p.user_file.as_deref(),
            Some(Path::new("/home/u/.config/clai/config.toml"))
        );
        assert_eq!(
            p.state.as_deref(),
            Some(Path::new("/home/u/.local/state/clai"))
        );
        assert_eq!(p.runtime, Path::new("/var/tmp/clai-1000"));
        let p = Paths::new(&s, Some(Path::new("/x/c.toml")));
        assert_eq!(p.user_file.as_deref(), Some(Path::new("/x/c.toml")));
    }

    #[test]
    fn without_home() {
        let p = Paths::new(&Startup::default(), None);
        assert_eq!(p.user_file, None);
        assert_eq!(p.state, None);
        assert_eq!(p.log, None);
        assert_eq!(p.runtime, Path::new("/tmp/clai-0"));
    }
}
