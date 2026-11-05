//! Exit codes and output of the `clai` binary.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::process::{Command, Output};

/// Runs `clai` with a private home and state directory, so tests never
/// read the user's configuration or write to their home.
fn clai(name: &str, args: &[&str]) -> (Output, PathBuf) {
    let state = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&state);
    let output = Command::new(env!("CARGO_BIN_EXE_clai"))
        .args(args)
        .env("HOME", state.join("home"))
        .env("XDG_CONFIG_HOME", state.join("config"))
        .env("XDG_STATE_HOME", &state)
        .env("XDG_RUNTIME_DIR", state.join("run"))
        .env_remove("CLAI_LOG")
        .env_remove("CLAI_MODEL")
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
    let (out, state) = clai("commit", &["commit"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert_eq!(stderr(&out), "clai: 'commit' is not implemented yet\n");
    let log = std::fs::read_to_string(state.join("clai/clai.log")).unwrap();
    assert!(log.contains("not implemented"));
}

#[test]
fn short_form_is_run() {
    // `clai run` needs a configuration; the short form gets as far.
    let (out, _) = clai("short", &["fix", "the", "test"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).starts_with("clai: no configuration file at "),
        "{}",
        stderr(&out)
    );
}

#[test]
fn unwritable_log_only_warns() {
    let file = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("state-is-a-file");
    std::fs::write(&file, "").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_clai"))
        .arg("commit")
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
        err.ends_with("clai: 'commit' is not implemented yet\n"),
        "{err}"
    );
}

#[test]
fn config_before_command() {
    let (out, _) = clai("config-before", &["--config", "c.toml", "commit"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stderr(&out), "clai: 'commit' is not implemented yet\n");
}

#[test]
fn top_level_option_with_command_exits_2() {
    let (out, _) = clai("top-with-command", &["--model", "m", "ls"]);
    assert_eq!(out.status.code(), Some(2));
}

const CONFIG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/configs/valid/minimal.toml"
);

/// Standard input of these runs is a pipe: no terminal.
fn assert_needs_terminal(name: &str, args: &[&str]) {
    let mut all = vec!["--config", CONFIG];
    all.extend_from_slice(args);
    let (out, state) = clai(name, &all);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert_eq!(
        stderr(&out),
        "clai: the TUI needs a terminal; use \"clai run TASK\" or \"clai chat\"\n"
    );
    // Neither a server nor a session came of it.
    assert!(!state.join("run/clai/server.sock").exists());
    assert!(!state.join("run/clai/sessions").exists());
}

#[test]
fn tui_commands_need_a_terminal() {
    assert_needs_terminal("tui-clai", &[]);
    assert_needs_terminal("tui-new", &["new"]);
    assert_needs_terminal("tui-attach", &["attach"]);
    assert_needs_terminal("tui-attach-name", &["attach", "docs"]);
}

#[test]
fn clai_without_a_config_explains_it() {
    let (out, _) = clai("tui-no-config", &[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).starts_with("clai: no configuration file at "),
        "{}",
        stderr(&out)
    );
}

#[test]
fn options_of_one_shot_mode_need_a_task() {
    for (name, args) in [
        ("tui-yes", &["--yes"][..]),
        ("tui-no-ask", &["--no-ask"][..]),
        ("tui-dir", &["-C", "/tmp"][..]),
    ] {
        let (out, _) = clai(name, args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
}

#[test]
fn new_checks_the_directory_before_the_terminal() {
    let (out, _) = clai(
        "tui-no-dir",
        &["--config", CONFIG, "new", "/no/such/directory"],
    );
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stderr(&out), "clai: /no/such/directory: not a directory\n");
}
