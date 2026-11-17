use super::*;

fn sandbox(network: bool, hide: Vec<PathBuf>) -> Sandbox {
    Sandbox { network, hide }
}

#[test]
fn bwrap_arguments_bind_the_writable_dirs_after_the_hidden_ones() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("secrets");
    std::fs::create_dir(&dir).unwrap();
    let file = tmp.path().join(".netrc");
    std::fs::write(&file, "machine x").unwrap();
    let gone = tmp.path().join("missing");
    let ws = tmp.path().join("ws");
    let writable = Writable {
        dirs: vec![ws.clone()],
    };
    let args = bwrap_args(
        &sandbox(false, vec![dir.clone(), file.clone(), gone]),
        &ws,
        &writable,
    );
    let s = |p: &Path| p.to_string_lossy().into_owned();
    let mut want: Vec<String> = [
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
        "--unshare-net",
        "--tmpfs",
    ]
    .iter()
    .map(|a| (*a).to_owned())
    .collect();
    want.extend([s(&dir), "--ro-bind".into(), "/dev/null".into(), s(&file)]);
    want.extend(["--bind".into(), s(&ws), s(&ws), "--chdir".into(), s(&ws)]);
    assert_eq!(args, want);
    // With the network, no `--unshare-net`.
    let args = bwrap_args(&sandbox(true, vec![]), &ws, &writable);
    assert!(!args.contains(&"--unshare-net".to_owned()));
}

#[test]
fn removed_variables_come_from_the_providers_and_the_known_list() {
    let config = crate::config::load_for_test(
        r#"
default_model = "p/m"

[providers.p]
api = "anthropic"
api_key_env = "MY_ANTHROPIC_KEY"

[providers.p.models.m]
id = "m"
context_window = 1000

[run_command]
keep_env = ["OPENAI_API_KEY"]
"#,
    );
    let names = removed_env(&config);
    assert!(names.contains(&"ANTHROPIC_API_KEY".to_owned()));
    assert!(names.contains(&"GEMINI_API_KEY".to_owned()));
    assert!(names.contains(&"MY_ANTHROPIC_KEY".to_owned()));
    assert!(!names.contains(&"OPENAI_API_KEY".to_owned()), "kept");
    assert!(!names.contains(&"GITHUB_TOKEN".to_owned()), "tasks need it");
}

#[test]
fn the_shell_command_removes_the_variables() {
    let children = Children {
        remove_env: vec!["ANTHROPIC_API_KEY".into()],
        sandbox: None,
    };
    let command = shell(&children, "true", Path::new("/"), &Writable::default());
    let removed: Vec<_> = command
        .as_std()
        .get_envs()
        .filter(|(_, v)| v.is_none())
        .map(|(k, _)| k.to_string_lossy().into_owned())
        .collect();
    assert_eq!(removed, ["ANTHROPIC_API_KEY"]);
    assert_eq!(command.as_std().get_program(), "sh");
}

#[test]
fn a_missing_bwrap_is_said_and_nothing_runs_without_the_sandbox() {
    let children = Children {
        remove_env: vec![],
        sandbox: Some(sandbox(true, vec![])),
    };
    let mut command = shell(&children, "true", Path::new("/"), &Writable::default());
    assert_eq!(command.as_std().get_program(), "bwrap");
    command.env("PATH", "/nonexistent");
    let e = command.spawn().unwrap_err();
    assert_eq!(
        start_error(&children, &e).unwrap(),
        "sandbox: bwrap not found; install bubblewrap or set sandbox.enabled = false"
    );
    assert_eq!(start_error(&Children::default(), &e), None);
}
