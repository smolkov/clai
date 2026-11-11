//! `clai models`: list configured providers and models.

use clap::Args;

use super::{Context, Execute};
use crate::config::{self, Config, DefaultFrom, KeyStatus};
use crate::error::{Error, Result};
use crate::output::{Align, columns, note, write_stdout};
use crate::text::short_tokens;

#[derive(Debug, Args)]
pub struct Models {
    /// Print a JSON array instead of the table.
    #[arg(long, conflicts_with = "set_default")]
    pub json: bool,

    /// Save MODEL as default_model in the user config file, the model of
    /// new sessions from now on.
    #[arg(long, value_name = "MODEL")]
    pub set_default: Option<String>,
}

impl Execute for Models {
    fn name(&self) -> &'static str {
        "models"
    }

    fn run(&self, ctx: &Context) -> Result<()> {
        let workspace = std::env::current_dir().map_err(Error::io("current directory"))?;
        let config = ctx.load(workspace.clone())?;
        if let Some(model) = &self.set_default {
            let options = ctx.load_options(workspace);
            let saved = config::save_default_model(&options, &config.user_file, model)
                .map_err(Error::Usage)?;
            let mut notes = saved.notes("warning:").into_iter();
            if let Some(first) = notes.next() {
                write_stdout(&format!("{first}\n"))?;
            }
            for line in notes {
                note(line.trim_start_matches("warning: "));
            }
            return Ok(());
        }
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

/// `default`, and where it comes from when it is not the user file.
fn default_mark(from: DefaultFrom) -> &'static str {
    match from {
        DefaultFrom::User => "default",
        DefaultFrom::Project => "default (project)",
        DefaultFrom::Env => "default (CLAI_MODEL)",
    }
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
                marks.push(default_mark(config.default_from));
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
    let cells: Vec<Vec<String>> = rows
        .into_iter()
        .map(|row| vec![row.reference, row.id, row.context, row.marks.join(", ")])
        .collect();
    let mut out = String::new();
    for line in columns(&cells, &[Align::Left, Align::Left, Align::Right]) {
        out.push_str(&line);
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
                let default = config.default_model.provider == provider.name
                    && config.default_model.model == model.name;
                serde_json::json!({
                    "model": format!("{}/{}", provider.name, model.name),
                    "provider": provider.name,
                    "name": model.name,
                    "id": model.id,
                    "context_window": model.context_window,
                    "max_output_tokens": model.max_output_tokens,
                    "default": default,
                    "default_from": default.then(|| config.default_from.name()),
                    "key": key,
                })
            })
        })
        .collect();
    let mut text = serde_json::to_string_pretty(&models).unwrap_or_else(|_| "[]".to_owned());
    text.push('\n');
    text
}
