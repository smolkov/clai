//! Tab completion through `COMPLETE=fish clai -- <words>`, and `clai completion`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Output};

const CONFIG: &str = r#"
default_model = "anthropic/sonnet"

[providers.anthropic]
api = "anthropic"

[providers.anthropic.models.sonnet]
id = "claude-sonnet-5-5"
context_window = 200000

[providers.local]
api = "openai-compatible"
base_url = "http://localhost:11434/v1"

[providers.local.models.qwen]
id = "qwen3-coder:30b"
context_window = 32768
"#;

fn home(name: &str, config: Option<&str>) -> PathBuf {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("completion")
        .join(name);
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("config/clai")).unwrap();
    if let Some(config) = config {
        fs::write(root.join("config/clai/config.toml"), config).unwrap();
    }
    root
}

fn clai(root: &PathBuf, complete: Option<&str>, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_clai"));
    cmd.args(args)
        .current_dir(root)
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env_remove("CLAI_MODEL")
        .env_remove("COMPLETE");
    if let Some(shell) = complete {
        cmd.env("COMPLETE", shell);
    }
    cmd.output().unwrap()
}

/// Candidate values for the last word, as the fish protocol prints them.
fn complete(root: &PathBuf, words: &[&str]) -> Vec<String> {
    let mut args = vec!["--", "clai"];
    args.extend_from_slice(words);
    let out = clai(root, Some("fish"), &args);
    assert_eq!(
        out.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        out.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| l.split('\t').next().unwrap_or("").to_owned())
        .collect()
}

#[test]
fn completes_command_names() {
    let root = home("names", None);
    assert_eq!(complete(&root, &["mo"]), ["models"]);
    let all = complete(&root, &["co"]);
    assert!(
        all.contains(&"commit".to_owned()) && all.contains(&"completion".to_owned()),
        "{all:?}"
    );
}

#[test]
fn completes_models_from_config() {
    let root = home("models", Some(CONFIG));
    assert_eq!(
        complete(&root, &["run", "--model", ""]),
        ["anthropic/sonnet", "local/qwen"]
    );
    assert_eq!(complete(&root, &["--model", "lo"]), ["local/qwen"]);
}

#[test]
fn no_config_gives_no_models_and_no_error() {
    let root = home("no-config", None);
    assert!(complete(&root, &["run", "--model", ""]).is_empty());
}

#[test]
fn completes_shell_names() {
    let root = home("shells", None);
    assert_eq!(complete(&root, &["completion", "z"]), ["zsh"]);
}

#[test]
fn completion_command_prints_script() {
    let root = home("script", None);
    let out = clai(&root, None, &["completion", "bash"]);
    assert_eq!(out.status.code(), Some(0));
    let script = String::from_utf8_lossy(&out.stdout);
    assert!(script.contains("COMPLETE=\"bash\""), "{script}");
    assert!(script.contains("complete"), "{script}");

    let out = clai(&root, None, &["completion", "tcsh"]);
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn completion_shell_from_env() {
    let root = home("env-shell", None);
    let out = Command::new(env!("CARGO_BIN_EXE_clai"))
        .args(["completion"])
        .env("SHELL", "/usr/bin/zsh")
        .env("XDG_STATE_HOME", root.join("state"))
        .env_remove("COMPLETE")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("compdef"));

    let out = Command::new(env!("CARGO_BIN_EXE_clai"))
        .args(["completion"])
        .env("SHELL", "/bin/tcsh")
        .env("XDG_STATE_HOME", root.join("state"))
        .env_remove("COMPLETE")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
}
