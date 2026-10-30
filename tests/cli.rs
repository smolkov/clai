//! Exit codes and output of the `clai` binary.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::process::{Command, Output};

/// Runs `clai` with a private state directory, so tests never write to the
/// user's home.
fn clai(name: &str, args: &[&str]) -> (Output, PathBuf) {
    let state = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&state);
    let output = Command::new(env!("CARGO_BIN_EXE_clai"))
        .args(args)
        .env("XDG_STATE_HOME", &state)
        .env_remove("CLAI_LOG")
        .output()
        .unwrap();
    (output, state)
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn version_exits_0() {
    let (out, _) = clai("version", &["--version"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("clai "));
}

#[test]
fn help_exits_0() {
    let (out, _) = clai("help", &["--help"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("Usage: clai"));
}

#[test]
fn unknown_option_exits_2() {
    let (out, _) = clai("unknown", &["--nope"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
}

#[test]
fn unbuilt_command_exits_1() {
    let (out, state) = clai("ls", &["ls"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert_eq!(stderr(&out), "clai: 'ls' is not implemented yet\n");
    let log = std::fs::read_to_string(state.join("clai/clai.log")).unwrap();
    assert!(log.contains("not implemented"));
}

#[test]
fn short_form_is_run() {
    let (out, _) = clai("short", &["fix", "the", "test"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out), "clai: 'run' is not implemented yet\n");
}

#[test]
fn unwritable_log_only_warns() {
    let file = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("state-is-a-file");
    std::fs::write(&file, "").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_clai"))
        .arg("ls")
        .env("XDG_STATE_HOME", &file)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(
        err.starts_with("clai: warning: cannot open log file"),
        "{err}"
    );
    assert!(
        err.ends_with("clai: 'ls' is not implemented yet\n"),
        "{err}"
    );
}

#[test]
fn config_before_command() {
    let (out, _) = clai("config-before", &["--config", "c.toml", "ls"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out), "clai: 'ls' is not implemented yet\n");
}

#[test]
fn top_level_option_with_command_exits_2() {
    let (out, _) = clai("top-with-command", &["--model", "m", "ls"]);
    assert_eq!(out.status.code(), Some(2));
}
