//! `clai models`: list configured providers and models.

use std::path::Path;

use clap::Args;

use crate::config::{self, Config, Environment, KeyStatus, LoadOptions};
use crate::error::{Error, Result};
use crate::output::write_stdout;

#[derive(Debug, Args)]
pub struct Models {
    /// Print a JSON array instead of the table.
    #[arg(long)]
    pub json: bool,
}

impl Models {
    pub fn run(&self, config_file: Option<&Path>) -> Result<()> {
        let workspace = std::env::current_dir().map_err(Error::io("current directory"))?;
        let config = config::load(&LoadOptions {
            config_file: config_file.map(Path::to_path_buf),
            workspace,
            env: Environment::from_process(),
        })?;
        let getenv = |name: &str| std::env::var(name).ok();
        let text = if self.json {
            render_json(&config, getenv)
        } else {
            render_table(&config, getenv)
        };
        write_stdout(&text)
    }
}

struct Row {
    reference: String,
    id: String,
    context: String,
    marks: Vec<&'static str>,
}

/// One line per model: `PROVIDER/MODEL  ID  CONTEXT  MARKS`.
pub fn render_table(config: &Config, getenv: impl Fn(&str) -> Option<String>) -> String {
    let mut rows = Vec::new();
    for provider in config.providers.values() {
        let missing = provider.key.status(&getenv) == KeyStatus::Missing;
        for model in provider.models.values() {
            let mut marks = Vec::new();
            if config.default_model.provider == provider.name
                && config.default_model.model == model.name
            {
                marks.push("default");
            }
            if missing {
                marks.push("no key");
            }
            rows.push(Row {
                reference: format!("{}/{}", provider.name, model.name),
                id: model.id.clone(),
                context: short_tokens(model.context_window),
                marks,
            });
        }
    }
    let width = |f: fn(&Row) -> usize| rows.iter().map(f).max().unwrap_or(0);
    let w_ref = width(|r| r.reference.chars().count());
    let w_id = width(|r| r.id.chars().count());
    let w_ctx = width(|r| r.context.len());
    let mut out = String::new();
    for row in &rows {
        let line = format!(
            "{:<w_ref$}  {:<w_id$}  {:>w_ctx$}  {}",
            row.reference,
            row.id,
            row.context,
            row.marks.join(", ")
        );
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// JSON array with one object per model.
pub fn render_json(config: &Config, getenv: impl Fn(&str) -> Option<String>) -> String {
    let models: Vec<serde_json::Value> = config
        .providers
        .values()
        .flat_map(|provider| {
            let key = provider.key.status(&getenv).name();
            provider.models.values().map(move |model| {
                serde_json::json!({
                    "model": format!("{}/{}", provider.name, model.name),
                    "provider": provider.name,
                    "name": model.name,
                    "id": model.id,
                    "context_window": model.context_window,
                    "max_output_tokens": model.max_output_tokens,
                    "default": config.default_model.provider == provider.name
                        && config.default_model.model == model.name,
                    "key": key,
                })
            })
        })
        .collect();
    let mut text = serde_json::to_string_pretty(&models).unwrap_or_else(|_| "[]".to_owned());
    text.push('\n');
    text
}

/// `32768` → `32k`, `1000000` → `1M`; rounded down.
fn short_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{}M", n / 1_000_000)
    } else if n >= 1000 {
        format!("{}k", n / 1000)
    } else {
        n.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens() {
        assert_eq!(short_tokens(999), "999");
        assert_eq!(short_tokens(32768), "32k");
        assert_eq!(short_tokens(200_000), "200k");
        assert_eq!(short_tokens(1_000_000), "1M");
    }
}
