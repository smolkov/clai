//! Exit codes and output of the `codeagent` binary.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::process::{Command, Output};

/// Runs `codeagent` with a private home and state directory, so tests never
/// read the user's configuration or write to their home.
fn codeagent(name: &str, args: &[&str]) -> (Output, PathBuf) {
    let state = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&state);
    let output = Command::new(env!("CARGO_BIN_EXE_codeagent"))
        .args(args)
        .env("HOME", state.join("home"))
        .env("XDG_CONFIG_HOME", state.join("config"))
        .env("XDG_STATE_HOME", &state)
        .env("XDG_RUNTIME_DIR", state.join("run"))
        .env("GIT_CEILING_DIRECTORIES", env!("CARGO_TARGET_TMPDIR"))
        .env_remove("CODEAGENT_LOG")
        .env_remove("CODEAGENT_MODEL")
        .output()
        .unwrap();
    (output, state)
}

/// A directory outside any git repository, for a command that fails:
/// `codeagent commit` in it.
fn no_repository() -> String {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("no-repository");
    std::fs::create_dir_all(&dir).unwrap();
    dir.display().to_string()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn version_exits_0() {
    let (out, _) = codeagent("version", &["--version"]);
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let version = concat!("codeagent ", env!("CARGO_PKG_VERSION"));
    assert!(stdout.starts_with(version), "{stdout}");
}

#[test]
fn help_exits_0() {
    let (out, _) = codeagent("help", &["--help"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(String::from_utf8_lossy(&out.stdout).contains("Usage: codeagent"));
}

#[test]
fn unknown_option_exits_2() {
    let (out, _) = codeagent("unknown", &["--nope"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
}

#[test]
fn failed_command_exits_1() {
    let dir = no_repository();
    let (out, state) = codeagent("commit", &["commit", &dir]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert_eq!(
        stderr(&out),
        format!("codeagent: not a git repository: {dir}\n")
    );
    let log = std::fs::read_to_string(state.join("codeagent/codeagent.log")).unwrap();
    assert!(log.contains("not a git repository"));
}

#[test]
fn short_form_is_run() {
    // `codeagent run` needs a configuration; the short form gets as far.
    let (out, _) = codeagent("short", &["fix", "the", "test"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).starts_with("codeagent: no configuration file at "),
        "{}",
        stderr(&out)
    );
}

#[test]
fn unwritable_log_warns_only_where_work_is_logged() {
    let file = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("state-is-a-file");
    std::fs::write(&file, "").unwrap();
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_codeagent"))
            .args(args)
            .env("GIT_CEILING_DIRECTORIES", env!("CARGO_TARGET_TMPDIR"))
            .env("XDG_STATE_HOME", &file)
            .env("XDG_CONFIG_HOME", env!("CARGO_TARGET_TMPDIR"))
            .output()
            .unwrap()
    };
    // A client goes on without a log, quietly (RFC 0026).
    let out = run(&["commit", &no_repository()]);
    assert_eq!(out.status.code(), Some(1));
    let err = stderr(&out);
    assert!(!err.contains("cannot open log file"), "{err}");
    assert!(err.contains("codeagent: not a git repository: "), "{err}");
    // `codeagent run` does work to look back on: it warns, and goes on.
    let out = run(&["run", "a task"]);
    let err = stderr(&out);
    assert!(
        err.starts_with("codeagent: warning: cannot open log file"),
        "{err}"
    );
}

#[test]
fn config_before_command() {
    let dir = no_repository();
    let (out, _) = codeagent("config-before", &["--config", "c.toml", "commit", &dir]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(
        stderr(&out),
        format!("codeagent: not a git repository: {dir}\n")
    );
}

#[test]
fn top_level_option_with_command_exits_2() {
    let (out, _) = codeagent("top-with-command", &["--model", "m", "ls"]);
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
    let (out, state) = codeagent(name, &all);
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    assert_eq!(
        stderr(&out),
        "codeagent: the TUI needs a terminal; use \"codeagent run TASK\" or \"codeagent chat\"\n"
    );
    // Neither a server nor a session came of it.
    assert!(!state.join("run/codeagent/server.sock").exists());
    assert!(!state.join("run/codeagent/sessions").exists());
}

#[test]
fn tui_commands_need_a_terminal() {
    assert_needs_terminal("tui-codeagent", &[]);
    assert_needs_terminal("tui-new", &["new"]);
    assert_needs_terminal("tui-attach", &["attach"]);
    assert_needs_terminal("tui-attach-name", &["attach", "docs"]);
}

#[test]
fn codeagent_without_a_config_explains_it() {
    let (out, _) = codeagent("tui-no-config", &[]);
    assert_eq!(out.status.code(), Some(2));
    assert!(
        stderr(&out).starts_with("codeagent: no configuration file at "),
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
        let (out, _) = codeagent(name, args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
    }
}

#[test]
fn new_checks_the_directory_before_the_terminal() {
    let (out, _) = codeagent(
        "tui-no-dir",
        &["--config", CONFIG, "new", "/no/such/directory"],
    );
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(
        stderr(&out),
        "codeagent: /no/such/directory: not a directory\n"
    );
}
