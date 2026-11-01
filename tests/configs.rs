//! The sample configurations in `tests/configs/`: each valid file loads,
//! each invalid file fails with exactly the errors in its `.err` file.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn configs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/configs")
}

/// The `.toml` files of a directory, sorted.
fn files(dir: &str) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(configs().join(dir))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "toml"))
        .collect();
    files.sort();
    files
}

/// `clai --config FILE models` in an empty workspace, with a private home.
fn models(name: &str, config: &Path, project: Option<&Path>) -> (Output, PathBuf) {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join("configs")
        .join(name);
    let _ = fs::remove_dir_all(&root);
    let work = root.join("work");
    fs::create_dir_all(&work).unwrap();
    if let Some(project) = project {
        fs::create_dir_all(work.join(".clai")).unwrap();
        fs::copy(project, work.join(".clai/config.toml")).unwrap();
    }
    let out = Command::new(env!("CARGO_BIN_EXE_clai"))
        .arg("--config")
        .arg(config)
        .arg("models")
        .current_dir(&work)
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_STATE_HOME", root.join("state"))
        .env_remove("CLAI_MODEL")
        .env_remove("CLAI_THEME")
        .output()
        .unwrap();
    (out, work)
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn valid_files_load() {
    let files = files("valid");
    assert!(files.len() >= 5);
    for file in files {
        let name = file.file_stem().unwrap().to_string_lossy().into_owned();
        let (out, _) = models(&format!("valid-{name}"), &file, None);
        assert_eq!(out.status.code(), Some(0), "{name}: {}", stderr(&out));
        assert_eq!(stderr(&out), "", "{name}");
    }
}

#[test]
fn invalid_files_fail_with_their_errors() {
    let files = files("invalid");
    assert!(files.len() >= 15);
    for file in files {
        let name = file.file_stem().unwrap().to_string_lossy().into_owned();
        let expected = fs::read_to_string(file.with_extension("err"))
            .unwrap_or_else(|_| panic!("{name}.toml has no {name}.err"))
            .replace("CONFIG", &file.display().to_string());
        let (out, _) = models(&format!("invalid-{name}"), &file, None);
        assert_eq!(out.status.code(), Some(2), "{name}");
        assert_eq!(stderr(&out), expected, "{name}");
        assert!(out.stdout.is_empty(), "{name}");
    }
    // No error file without its configuration.
    for entry in fs::read_dir(configs().join("invalid")).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "err") {
            assert!(path.with_extension("toml").exists(), "{}", path.display());
        }
    }
}

#[test]
fn project_file_with_every_user_file() {
    let project = configs().join("project/config.toml");
    for file in files("valid") {
        let name = file.file_stem().unwrap().to_string_lossy().into_owned();
        let (out, _) = models(&format!("project-{name}"), &file, Some(&project));
        assert_eq!(out.status.code(), Some(0), "{name}: {}", stderr(&out));
    }
}

#[test]
fn project_file_may_not_loosen() {
    let user = configs().join("valid/minimal.toml");
    let project = configs().join("project/loosens.toml");
    let (out, work) = models("project-loosens", &user, Some(&project));
    let expected = fs::read_to_string(configs().join("project/loosens.err"))
        .unwrap()
        .replace(
            "PROJECT",
            &work.join(".clai/config.toml").display().to_string(),
        )
        .replace("USER", &user.display().to_string());
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stderr(&out), expected);
}
