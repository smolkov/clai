//! `clai completion` (see `spec/command.md`).
//!
//! Prints the shell code that registers `clai` for Tab completion. The code
//! calls `COMPLETE=<shell> clai -- <words>` on every Tab; `main` answers
//! that through `clap_complete::CompleteEnv` before anything else runs.

use std::path::{Path, PathBuf};

use clap::Args;
use clap_complete::env::Shells;

use crate::error::{Error, Result};
use crate::output::write_stdout;

/// Environment variable that switches `clai` into completion mode.
pub const COMPLETE_ENV: &str = "COMPLETE";

#[derive(Debug, Args)]
pub struct Completion {
    /// Shell. Default: the shell named by $SHELL.
    #[arg(value_parser = ["bash", "zsh", "fish", "elvish", "powershell"])]
    pub shell: Option<String>,
}

impl Completion {
    pub fn run(&self, _config: Option<&Path>) -> Result<()> {
        let shell = match &self.shell {
            Some(shell) => shell.clone(),
            None => shell_from_env(std::env::var_os("SHELL").as_deref())?,
        };
        write_stdout(&registration(&shell, &completer())?)
    }
}

/// Registration script for `shell`; `completer` is the command the script
/// runs to get candidates.
pub fn registration(shell: &str, completer: &str) -> Result<String> {
    let shells = Shells::builtins();
    let Some(env_completer) = shells.completer(shell) else {
        return Err(Error::Usage(format!(
            "unknown shell `{shell}`; expected one of {}",
            shells.names().collect::<Vec<_>>().join(", ")
        )));
    };
    let mut buf = Vec::new();
    env_completer
        .write_registration(COMPLETE_ENV, "clai", "clai", completer, &mut buf)
        .map_err(Error::io("completion script"))?;
    Ok(String::from_utf8_lossy(&buf).into_owned())
}

/// The shell named by `$SHELL`, e.g. `/usr/bin/zsh` → `zsh`.
fn shell_from_env(shell: Option<&std::ffi::OsStr>) -> Result<String> {
    let name = shell
        .map(Path::new)
        .and_then(Path::file_name)
        .and_then(|n| n.to_str())
        .filter(|n| Shells::builtins().completer(n).is_some());
    name.map(str::to_owned).ok_or_else(|| {
        Error::Usage(
            "cannot tell the shell from $SHELL; name it, e.g. `clai completion bash`".to_owned(),
        )
    })
}

/// How the script calls `clai`: as started, found through `PATH`, or as an
/// absolute path when it was started by a path (as `CompleteEnv` does).
fn completer() -> String {
    let Some(argv0) = std::env::args_os().next() else {
        return "clai".to_owned();
    };
    let mut path = PathBuf::from(argv0);
    if path.components().count() > 1
        && let Ok(cwd) = std::env::current_dir()
    {
        path = cwd.join(path);
    }
    path.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn scripts_call_clai_with_complete() {
        for shell in ["bash", "zsh", "fish", "elvish", "powershell"] {
            let script = registration(shell, "/opt/bin/clai").unwrap();
            assert!(script.contains("COMPLETE"), "{shell}: {script}");
            assert!(script.contains("/opt/bin/clai"), "{shell}: {script}");
        }
    }

    #[test]
    fn unknown_shell() {
        let err = registration("tcsh", "clai").unwrap_err();
        assert_eq!(err.exit_code(), 2);
        assert!(err.to_string().contains("bash"), "{err}");
    }

    #[test]
    fn shell_from_env_var() {
        assert_eq!(
            shell_from_env(Some(OsStr::new("/usr/bin/zsh"))).unwrap(),
            "zsh"
        );
        assert_eq!(shell_from_env(Some(OsStr::new("bash"))).unwrap(), "bash");
        assert_eq!(
            shell_from_env(Some(OsStr::new("/bin/tcsh")))
                .unwrap_err()
                .exit_code(),
            2
        );
        assert_eq!(shell_from_env(None).unwrap_err().exit_code(), 2);
    }
}
