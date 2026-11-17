//! How child processes start (RFC 0035): `run_command` and language
//! servers run the repository's code, so they get no provider keys, and
//! with `[sandbox] enabled` a command runs inside `bwrap`.

use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::config::Config;

/// Variables of provider keys, removed even when no provider names them.
pub const PROVIDER_KEYS: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "OPENAI_API_KEY",
    "AZURE_OPENAI_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "OPENROUTER_API_KEY",
    "MISTRAL_API_KEY",
    "GROQ_API_KEY",
    "DEEPSEEK_API_KEY",
    "XAI_API_KEY",
    "TOGETHER_API_KEY",
    "FIREWORKS_API_KEY",
    "COHERE_API_KEY",
    "CO_API_KEY",
];

/// The program that makes the sandbox.
const BWRAP: &str = "bwrap";

/// What child processes of a session get.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Children {
    /// Variables removed from the environment.
    pub remove_env: Vec<String>,
    /// Set when commands run in the sandbox.
    pub sandbox: Option<Sandbox>,
}

/// The sandbox for commands (`[sandbox]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sandbox {
    pub network: bool,
    /// Directories and files that are empty inside; absolute.
    pub hide: Vec<PathBuf>,
}

impl Children {
    /// From the configuration; `home` and `state` and `runtime` are the
    /// user's directories that the sandbox hides.
    pub fn from_config(
        config: &Config,
        home: Option<&Path>,
        state: Option<&Path>,
        runtime: &Path,
    ) -> Children {
        Children {
            remove_env: removed_env(config),
            sandbox: config.sandbox.enabled.then(|| Sandbox {
                network: config.sandbox.network,
                hide: hidden(home, state, runtime, &config.sandbox.hide),
            }),
        }
    }

    /// Removes the variables from `command`'s environment.
    pub fn apply_env(&self, command: &mut Command) {
        for name in &self.remove_env {
            command.env_remove(name);
        }
    }

    /// How a question names where the command runs: `sandbox`,
    /// `sandbox, no network`, or nothing.
    pub fn sandbox_text(&self) -> Option<String> {
        self.sandbox.as_ref().map(|s| {
            if s.network {
                "sandbox".to_owned()
            } else {
                "sandbox, no network".to_owned()
            }
        })
    }
}

/// The provider key variables of `config` and the known ones, less those in
/// `[run_command] keep_env`.
pub fn removed_env(config: &Config) -> Vec<String> {
    let mut names: Vec<String> = PROVIDER_KEYS.iter().map(|s| (*s).to_owned()).collect();
    for provider in config.providers.values() {
        if let crate::config::KeySource::Env(name) = &provider.key
            && !names.contains(name)
        {
            names.push(name.clone());
        }
    }
    names.retain(|n| !config.keep_env.contains(n));
    names
}

/// The built-in hidden places, then `extra`.
fn hidden(
    home: Option<&Path>,
    state: Option<&Path>,
    runtime: &Path,
    extra: &[PathBuf],
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(home) = home {
        for rel in [".config/codeagent", ".ssh", ".gnupg", ".aws", ".netrc"] {
            out.push(home.join(rel));
        }
    }
    if let Some(state) = state {
        out.push(state.to_path_buf());
    }
    out.push(runtime.to_path_buf());
    for path in extra {
        if !out.contains(path) {
            out.push(path.clone());
        }
    }
    out
}

/// Where a sandboxed command may write: the workspace, the scratch
/// directory and the extra directories with write access.
#[derive(Debug, Clone, Default)]
pub struct Writable {
    pub dirs: Vec<PathBuf>,
}

/// `sh -c COMMAND` in `cwd`, in the sandbox when `children` has one.
pub fn shell(children: &Children, command: &str, cwd: &Path, writable: &Writable) -> Command {
    let mut out = match &children.sandbox {
        None => {
            let mut c = Command::new("sh");
            c.arg("-c").arg(command);
            c
        }
        Some(sandbox) => {
            let mut c = Command::new(BWRAP);
            c.args(bwrap_args(sandbox, cwd, writable));
            c.arg("--").arg("sh").arg("-c").arg(command);
            c
        }
    };
    out.current_dir(cwd);
    children.apply_env(&mut out);
    out
}

/// Whether `error` from starting a sandboxed command means `bwrap` is
/// missing; then the message says what to do.
pub fn start_error(children: &Children, error: &std::io::Error) -> Option<String> {
    (children.sandbox.is_some() && error.kind() == std::io::ErrorKind::NotFound).then(|| {
        "sandbox: bwrap not found; install bubblewrap or set sandbox.enabled = false".to_owned()
    })
}

/// The arguments of `bwrap` before `--`: everything read-only, a private
/// `/tmp`, the hidden places empty, then the writable directories bound
/// again (also inside a hidden place, like the scratch directory in the
/// runtime directory).
pub fn bwrap_args(sandbox: &Sandbox, cwd: &Path, writable: &Writable) -> Vec<String> {
    let mut args: Vec<String> = [
        "--die-with-parent",
        "--new-session",
        "--unshare-pid",
        "--ro-bind",
        "/",
        "/",
        "--dev",
        "/dev",
        "--proc",
        "/proc",
        "--tmpfs",
        "/tmp",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    if !sandbox.network {
        args.push("--unshare-net".into());
    }
    for path in &sandbox.hide {
        let text = path.to_string_lossy().into_owned();
        match std::fs::metadata(path) {
            Ok(meta) if meta.is_dir() => args.extend(["--tmpfs".into(), text]),
            Ok(_) => args.extend(["--ro-bind".into(), "/dev/null".into(), text]),
            Err(_) => {}
        }
    }
    for dir in &writable.dirs {
        let text = dir.to_string_lossy().into_owned();
        args.extend(["--bind".into(), text.clone(), text]);
    }
    args.extend(["--chdir".into(), cwd.to_string_lossy().into_owned()]);
    args
}

#[cfg(test)]
mod tests;
