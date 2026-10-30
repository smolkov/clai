//! `clai models` and configuration loading, run as a process.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CONFIG: &str = r#"
default_model = "anthropic/sonnet"

[providers.anthropic]
api = "anthropic"
api_key_env = "CLAI_TEST_NO_SUCH_KEY"

[providers.anthropic.models.sonnet]
id = "claude-sonnet-5-5"
context_window = 200000
max_output_tokens = 64000

[providers.local]
api = "openai-compatible"
base_url = "http://localhost:11434/v1"

[providers.local.models.qwen]
id = "qwen3-coder:30b"
context_window = 32768
"#;

/// A temp home with config dir, state dir and an empty workspace.
struct Env {
    root: PathBuf,
}

impl Env {
    fn new(name: &str, config: Option<&str>) -> Env {
        let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
            .join("models")
            .join(name);
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("config/clai")).unwrap();
        fs::create_dir_all(root.join("work")).unwrap();
        if let Some(config) = config {
            fs::write(root.join("config/clai/config.toml"), config).unwrap();
        }
        Env { root }
    }

    fn workspace(&self) -> PathBuf {
        self.root.join("work")
    }

    fn run(&self, args: &[&str], vars: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_clai"));
        cmd.args(args)
            .current_dir(self.workspace())
            .env("HOME", &self.root)
            .env("XDG_CONFIG_HOME", self.root.join("config"))
            .env("XDG_STATE_HOME", self.root.join("state"))
            .env_remove("CLAI_MODEL")
            .env_remove("CLAI_THEME")
            .env_remove("CLAI_TEST_NO_SUCH_KEY");
        for (k, v) in vars {
            cmd.env(k, v);
        }
        cmd.output().unwrap()
    }
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn table() {
    let env = Env::new("table", Some(CONFIG));
    let out = env.run(&["models"], &[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "anthropic/sonnet  claude-sonnet-5-5  200k  default, no key\n\
         local/qwen        qwen3-coder:30b     32k\n"
    );
}

#[test]
fn key_present() {
    let env = Env::new("key", Some(CONFIG));
    let out = env.run(&["models"], &[("CLAI_TEST_NO_SUCH_KEY", "sk-test")]);
    let text = stdout(&out);
    assert!(!text.contains("no key"), "{text}");
    assert!(!text.contains("sk-test"), "{text}");
}

#[test]
fn json() {
    let env = Env::new("json", Some(CONFIG));
    let out = env.run(&["models", "--json"], &[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        value,
        serde_json::json!([
            {
                "model": "anthropic/sonnet", "provider": "anthropic", "name": "sonnet",
                "id": "claude-sonnet-5-5", "context_window": 200000,
                "max_output_tokens": 64000, "default": true, "key": "missing"
            },
            {
                "model": "local/qwen", "provider": "local", "name": "qwen",
                "id": "qwen3-coder:30b", "context_window": 32768,
                "max_output_tokens": null, "default": false, "key": "none"
            }
        ])
    );
}

#[test]
fn env_model_overrides_default() {
    let env = Env::new("env-model", Some(CONFIG));
    let out = env.run(&["models"], &[("CLAI_MODEL", "qwen")]);
    assert!(stdout(&out).contains("32k  default"), "{}", stdout(&out));
}

#[test]
fn project_file_is_merged() {
    let env = Env::new("project", Some(CONFIG));
    fs::create_dir_all(env.workspace().join(".clai")).unwrap();
    fs::write(
        env.workspace().join(".clai/config.toml"),
        "default_model = \"local/qwen\"\n",
    )
    .unwrap();
    let out = env.run(&["models"], &[]);
    assert!(stdout(&out).contains("32k  default"), "{}", stdout(&out));
}

#[test]
fn no_user_file_prints_example() {
    let env = Env::new("none", None);
    let out = env.run(&["models"], &[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    let err = stderr(&out);
    let path = env.root.join("config/clai/config.toml");
    assert!(
        err.starts_with(&format!(
            "clai: no configuration file at {}",
            path.display()
        )),
        "{err}"
    );
    assert!(err.contains("[providers.anthropic]"), "{err}");
}

#[test]
fn invalid_file_reports_line() {
    let env = Env::new(
        "invalid",
        Some(&CONFIG.replace("context_window = 32768", "")),
    );
    let out = env.run(&["models"], &[]);
    assert_eq!(out.status.code(), Some(2));
    let path = env.root.join("config/clai/config.toml");
    assert_eq!(
        stderr(&out),
        format!(
            "clai: {}:17: providers.local.models.qwen: missing context_window\n",
            path.display()
        )
    );
}

#[test]
fn config_option() {
    let env = Env::new("option", None);
    let file = env.root.join("other.toml");
    fs::write(&file, CONFIG).unwrap();
    let out = env.run(&["models", "--config", path(&file)], &[]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let missing = env.root.join("missing.toml");
    let out = env.run(&["--config", path(&missing), "models"], &[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).contains("missing.toml: cannot read"),
        "{}",
        stderr(&out)
    );
}

fn path(p: &Path) -> &str {
    p.to_str().unwrap()
}
