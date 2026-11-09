//! Values for shell completion (see `clai completion`).
//!
//! Completers run on every Tab, so they must be quiet: any problem gives no
//! candidates, never an error or output on standard error.

use std::ffi::OsStr;

use clap::builder::StyledStr;
use clap_complete::CompletionCandidate;

use std::time::Duration;

use crate::client::{Client, Connect};
use crate::config::{self, Config, Environment, LoadOptions};
use crate::protocol::SessionInfo;

/// `--model`: every configured `provider/model`, described by its model id.
///
/// A `--config` option earlier on the command line is not visible here, so
/// the default user file is used.
pub fn models(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };
    let Ok(workspace) = std::env::current_dir() else {
        return Vec::new();
    };
    let options = LoadOptions {
        config_file: None,
        workspace,
        env: Environment::from_process(),
    };
    match config::load(&options) {
        Ok(config) => model_candidates(&config, current),
        Err(_) => Vec::new(),
    }
}

/// Session names of a running server; never starts one, and gives up
/// after a short time.
pub fn sessions(current: &OsStr) -> Vec<CompletionCandidate> {
    let Some(current) = current.to_str() else {
        return Vec::new();
    };
    let Ok(runtime) = crate::runtime::current_thread() else {
        return Vec::new();
    };
    let options = Connect::cli(None, false);
    let listed = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(1), async {
            Client::connect(&options).await?.list().await
        })
        .await
    });
    match listed {
        Ok(Ok(sessions)) => session_candidates(&sessions, current),
        _ => Vec::new(),
    }
}

/// Sessions whose name starts with `prefix`, described by their workspace.
pub fn session_candidates(sessions: &[SessionInfo], prefix: &str) -> Vec<CompletionCandidate> {
    sessions
        .iter()
        .filter(|s| s.name.starts_with(prefix))
        .map(|s| {
            CompletionCandidate::new(s.name.clone())
                .help(Some(StyledStr::from(s.workspace.clone())))
        })
        .collect()
}

/// Models whose reference starts with `prefix`, in config order.
pub fn model_candidates(config: &Config, prefix: &str) -> Vec<CompletionCandidate> {
    config
        .providers
        .values()
        .flat_map(|provider| {
            provider.models.values().map(move |model| {
                (
                    format!("{}/{}", provider.name, model.name),
                    model.id.clone(),
                )
            })
        })
        .filter(|(reference, _)| reference.starts_with(prefix))
        .map(|(reference, id)| CompletionCandidate::new(reference).help(Some(StyledStr::from(id))))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn candidates_filter_by_prefix() {
        let dir = std::env::temp_dir().join(format!("clai-complete-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config.toml");
        std::fs::write(&file, config::EXAMPLE).unwrap();
        let config = config::load(&LoadOptions {
            config_file: Some(file),
            workspace: PathBuf::from("/nonexistent"),
            env: Environment::default(),
        })
        .unwrap();
        let values = |prefix| {
            model_candidates(&config, prefix)
                .iter()
                .map(|c| {
                    let help = c.get_help().map(ToString::to_string).unwrap_or_default();
                    format!("{}={help}", c.get_value().to_string_lossy())
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            values(""),
            [
                "anthropic/sonnet=claude-sonnet-5-5",
                "local/qwen=qwen3-coder:30b"
            ]
        );
        assert_eq!(values("lo"), ["local/qwen=qwen3-coder:30b"]);
        assert!(values("x").is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_names_by_prefix() {
        let info = |name: &str| SessionInfo {
            id: 1,
            name: name.into(),
            workspace: "~/ws".into(),
            path: PathBuf::from("/ws"),
            model: "a/b".into(),
            state: crate::agent::State::Idle,
            created: 0,
            updated: 0,
            turns: 0,
            queued: 0,
            question: None,
            usage: Default::default(),
        };
        let sessions = [info("clai"), info("clai-2"), info("docs")];
        let names: Vec<String> = session_candidates(&sessions, "cl")
            .iter()
            .map(|c| c.get_value().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["clai", "clai-2"]);
        assert!(session_candidates(&sessions, "x").is_empty());
    }
}
