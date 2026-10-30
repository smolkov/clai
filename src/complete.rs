//! Values for shell completion (see `clai completion`).
//!
//! Completers run on every Tab, so they must be quiet: any problem gives no
//! candidates, never an error or output on standard error.

use std::ffi::OsStr;

use clap::builder::StyledStr;
use clap_complete::CompletionCandidate;

use crate::config::{self, Config, Environment, LoadOptions};

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
}
