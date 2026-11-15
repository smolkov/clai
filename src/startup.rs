//! What `clai` reads from its process, read once at startup (RFC 0026).
//!
//! [`Startup::from_process`] is the only code that reads the environment
//! variables, the current directory and the temporary directory; everything
//! else gets a [`Startup`] and asks it. Tests build one with
//! [`Startup::from_vars`].

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

/// Overrides `default_model`.
pub const MODEL_ENV: &str = "CLAI_MODEL";
/// Overrides `tui.theme`.
pub const THEME_ENV: &str = "CLAI_THEME";
/// The log filter.
pub const LOG_ENV: &str = "CLAI_LOG";

/// The environment of the process, as `clai` uses it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Startup {
    /// Every variable, as it was at startup.
    pub(crate) vars: BTreeMap<OsString, OsString>,
    pub home: Option<PathBuf>,
    /// Absolute, or `None`: a relative one is ignored, as XDG says.
    pub xdg_config_home: Option<PathBuf>,
    pub xdg_state_home: Option<PathBuf>,
    pub xdg_runtime_dir: Option<PathBuf>,
    /// `CLAI_MODEL`, not empty.
    pub model: Option<String>,
    /// `CLAI_THEME`, not empty.
    pub theme: Option<String>,
    pub current_dir: Option<PathBuf>,
    /// `TMPDIR`, else `/tmp`.
    pub temp_dir: PathBuf,
    pub uid: u32,
    pub stdin_terminal: bool,
    pub stdout_terminal: bool,
}

impl Default for Startup {
    /// No variables: as [`Startup::from_vars`] with none.
    fn default() -> Startup {
        Startup::from_vars(std::iter::empty::<(OsString, OsString)>())
    }
}

impl Startup {
    /// Reads the process: its variables, directories, user and terminals.
    pub fn from_process() -> Startup {
        let mut startup = Startup::from_vars(std::env::vars_os());
        startup.current_dir = std::env::current_dir().ok();
        startup.temp_dir = std::env::temp_dir();
        startup.uid = nix::unistd::getuid().as_raw();
        startup.stdin_terminal = std::io::stdin().is_terminal();
        startup.stdout_terminal = std::io::stdout().is_terminal();
        startup
    }

    /// A startup with these variables, for tests and for the parts of
    /// [`Startup::from_process`]; no current directory, `/tmp`, user 0, no
    /// terminals.
    pub fn from_vars<K, V>(vars: impl IntoIterator<Item = (K, V)>) -> Startup
    where
        K: Into<OsString>,
        V: Into<OsString>,
    {
        let vars: BTreeMap<OsString, OsString> = vars
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect();
        let path = |name: &str| {
            vars.get(OsStr::new(name))
                .filter(|v| !v.is_empty())
                .map(PathBuf::from)
        };
        let absolute = |name: &str| path(name).filter(|p| p.is_absolute());
        let text = |name: &str| {
            vars.get(OsStr::new(name))
                .and_then(|v| v.to_str())
                .filter(|v| !v.trim().is_empty())
                .map(str::to_owned)
        };
        Startup {
            home: path("HOME"),
            xdg_config_home: absolute("XDG_CONFIG_HOME"),
            xdg_state_home: absolute("XDG_STATE_HOME"),
            xdg_runtime_dir: absolute("XDG_RUNTIME_DIR"),
            model: text(MODEL_ENV),
            theme: text(THEME_ENV),
            current_dir: None,
            temp_dir: PathBuf::from("/tmp"),
            uid: 0,
            stdin_terminal: false,
            stdout_terminal: false,
            vars,
        }
    }

    /// A variable as text; `None` if it is not set or not UTF-8.
    pub fn var(&self, name: &str) -> Option<String> {
        self.vars
            .get(OsStr::new(name))
            .and_then(|v| v.to_str())
            .map(str::to_owned)
    }

    /// A variable as it is.
    pub fn var_os(&self, name: &str) -> Option<&OsStr> {
        self.vars.get(OsStr::new(name)).map(OsString::as_os_str)
    }

    /// [`Startup::var`] as a function, for the code that takes a getter.
    pub fn getenv(&self) -> impl Fn(&str) -> Option<String> + '_ {
        |name| self.var(name)
    }

    /// The variables, owned, for a getter that must outlive `self`.
    pub fn getenv_owned(&self) -> impl Fn(&str) -> Option<String> + Send + Sync + 'static {
        let copy = self.clone();
        move |name| copy.var(name)
    }

    /// Without `CLAI_MODEL` and `CLAI_THEME`: the server's, which takes them
    /// from each client instead.
    pub fn for_server(&self) -> Startup {
        Startup {
            model: None,
            theme: None,
            ..self.clone()
        }
    }

    /// `~/x` for a path in the home directory, for messages.
    pub fn tilde(&self, path: &Path) -> String {
        match self.home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
            Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
            Some(rest) => format!("~/{}", rest.display()),
            None => path.display().to_string(),
        }
    }
}

/// The path of the running `clai`, to start a server with.
pub fn current_exe() -> std::io::Result<PathBuf> {
    std::env::current_exe()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variables_and_their_rules() {
        let s = Startup::from_vars([
            ("HOME", "/home/u"),
            ("XDG_CONFIG_HOME", "relative/config"),
            ("XDG_STATE_HOME", "/state"),
            ("XDG_RUNTIME_DIR", ""),
            ("CLAI_MODEL", "  "),
            ("CLAI_THEME", "nord"),
            ("LANG", "C.UTF-8"),
        ]);
        assert_eq!(s.home.as_deref(), Some(Path::new("/home/u")));
        assert_eq!(s.xdg_config_home, None, "relative: ignored");
        assert_eq!(s.xdg_state_home.as_deref(), Some(Path::new("/state")));
        assert_eq!(s.xdg_runtime_dir, None, "empty: not set");
        assert_eq!(s.model, None, "blank: not set");
        assert_eq!(s.theme.as_deref(), Some("nord"));
        assert_eq!(s.var("LANG").as_deref(), Some("C.UTF-8"));
        assert_eq!(s.var("NOPE"), None);
        assert_eq!((s.getenv())("LANG").as_deref(), Some("C.UTF-8"));
        let server = s.for_server();
        assert_eq!(server.theme, None);
        assert_eq!(server.var("CLAI_THEME").as_deref(), Some("nord"));
    }

    #[test]
    fn tilde() {
        let s = Startup::from_vars([("HOME", "/home/u")]);
        assert_eq!(s.tilde(Path::new("/home/u")), "~");
        assert_eq!(s.tilde(Path::new("/home/u/ws/x")), "~/ws/x");
        assert_eq!(s.tilde(Path::new("/opt/x")), "/opt/x");
        assert_eq!(Startup::default().tilde(Path::new("/home/u")), "/home/u");
    }
}
