//! API keys: where they come from, and a type that never shows them.

use std::fmt;
use std::io::Read;
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

/// An API key. `Debug` and `Display` print `***`; use [`ApiKey::expose`]
/// only to put it into a request.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(***)")
    }
}

impl fmt::Display for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

/// How a provider gets its key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySource {
    /// No key; requests are sent without one.
    None,
    /// Name of an environment variable.
    Env(String),
    /// Shell command whose output is the key.
    Cmd(String),
}

/// Key state as shown by `clai models`, without running any command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStatus {
    /// No key configured.
    None,
    /// The environment variable is set.
    Env,
    /// The environment variable is not set or empty.
    Missing,
    /// The key comes from a command, which was not run.
    Command,
}

impl KeyStatus {
    pub fn name(self) -> &'static str {
        match self {
            KeyStatus::None => "none",
            KeyStatus::Env => "env",
            KeyStatus::Missing => "missing",
            KeyStatus::Command => "command",
        }
    }
}

/// Time an `api_key_cmd` may take.
pub const KEY_CMD_TIMEOUT: Duration = Duration::from_secs(30);

impl KeySource {
    pub fn status(&self, getenv: impl Fn(&str) -> Option<String>) -> KeyStatus {
        match self {
            KeySource::None => KeyStatus::None,
            KeySource::Env(name) => match getenv(name) {
                Some(v) if !v.is_empty() => KeyStatus::Env,
                _ => KeyStatus::Missing,
            },
            KeySource::Cmd(_) => KeyStatus::Command,
        }
    }

    /// Reads the key. The error message never contains the key.
    pub fn resolve(
        &self,
        getenv: impl Fn(&str) -> Option<String>,
        timeout: Duration,
    ) -> Result<Option<ApiKey>, String> {
        match self {
            KeySource::None => Ok(None),
            KeySource::Env(name) => match getenv(name) {
                Some(v) if !v.is_empty() => Ok(Some(ApiKey(v))),
                _ => Err(format!("environment variable {name} is not set")),
            },
            KeySource::Cmd(cmd) => run_key_cmd(cmd, timeout).map(Some),
        }
    }
}

/// Runs `sh -c CMD` and returns its output without the trailing newline.
fn run_key_cmd(cmd: &str, timeout: Duration) -> Result<ApiKey, String> {
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(cmd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("api_key_cmd: cannot start sh: {e}"))?;
    // Read both pipes in threads, so a full pipe cannot block the command.
    let stdout = child.stdout.take().map(read_in_thread);
    let stderr = child.stderr.take().map(read_in_thread);

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "api_key_cmd did not finish within {} s",
                    timeout.as_secs_f32()
                ));
            }
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(e) => return Err(format!("api_key_cmd: {e}")),
        }
    };
    let join = |handle: Option<thread::JoinHandle<Vec<u8>>>| {
        handle.and_then(|h| h.join().ok()).unwrap_or_default()
    };
    let out = join(stdout);
    let err = join(stderr);

    if !status.success() {
        let err = String::from_utf8_lossy(&err);
        let detail = err.trim();
        return Err(if detail.is_empty() {
            format!("api_key_cmd failed ({status})")
        } else {
            format!("api_key_cmd failed ({status}): {detail}")
        });
    }
    let key = String::from_utf8(out).map_err(|_| "api_key_cmd output is not UTF-8".to_owned())?;
    let key = key.trim_end_matches(['\n', '\r']);
    if key.is_empty() {
        return Err("api_key_cmd printed nothing".to_owned());
    }
    Ok(ApiKey(key.to_owned()))
}

fn read_in_thread(mut pipe: impl Read + Send + 'static) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = pipe.read_to_end(&mut buf);
        buf
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_env(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn key_is_never_shown() {
        let key = ApiKey("sk-secret".into());
        assert_eq!(format!("{key:?}"), "ApiKey(***)");
        assert_eq!(key.to_string(), "***");
        assert_eq!(key.expose(), "sk-secret");
    }

    #[test]
    fn env_key() {
        let source = KeySource::Env("K".into());
        let env = |name: &str| (name == "K").then(|| "v".to_owned());
        assert_eq!(source.status(env), KeyStatus::Env);
        assert_eq!(
            source
                .resolve(env, KEY_CMD_TIMEOUT)
                .unwrap()
                .unwrap()
                .expose(),
            "v"
        );
        assert_eq!(source.status(no_env), KeyStatus::Missing);
        assert_eq!(source.status(|_| Some(String::new())), KeyStatus::Missing);
        assert!(source.resolve(no_env, KEY_CMD_TIMEOUT).is_err());
    }

    #[test]
    fn no_key() {
        assert_eq!(KeySource::None.status(no_env), KeyStatus::None);
        assert_eq!(KeySource::None.resolve(no_env, KEY_CMD_TIMEOUT), Ok(None));
    }

    #[test]
    fn cmd_key() {
        let source = KeySource::Cmd("printf 'sk-1\\n'".into());
        assert_eq!(source.status(no_env), KeyStatus::Command);
        let key = source.resolve(no_env, KEY_CMD_TIMEOUT).unwrap().unwrap();
        assert_eq!(key.expose(), "sk-1");
    }

    #[test]
    fn cmd_failure_shows_stderr() {
        let source = KeySource::Cmd("echo locked >&2; exit 3".into());
        let err = source.resolve(no_env, KEY_CMD_TIMEOUT).unwrap_err();
        assert!(err.contains("locked"), "{err}");
    }

    #[test]
    fn cmd_empty_output() {
        let source = KeySource::Cmd("true".into());
        let err = source.resolve(no_env, KEY_CMD_TIMEOUT).unwrap_err();
        assert!(err.contains("printed nothing"), "{err}");
    }

    #[test]
    fn cmd_timeout() {
        let source = KeySource::Cmd("sleep 5".into());
        let start = Instant::now();
        let err = source
            .resolve(no_env, Duration::from_millis(200))
            .unwrap_err();
        assert!(err.contains("did not finish"), "{err}");
        assert!(start.elapsed() < Duration::from_secs(4));
    }
}
