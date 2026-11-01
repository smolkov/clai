//! The TOML files as written, before validation.
//!
//! Required keys are `Option` here so that validation can report them with
//! the key path; unknown keys are rejected by serde.

use indexmap::IndexMap;
use serde::Deserialize;
use toml::Spanned;

use super::permissions::Action;
use super::tui::Symbols;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserFile {
    pub default_model: Option<Spanned<String>>,
    #[serde(default)]
    pub providers: IndexMap<String, Spanned<ProviderFile>>,
    pub limits: Option<LimitsFile>,
    pub permissions: Option<PermissionsFile>,
    pub commit: Option<CommitFile>,
    pub tui: Option<TuiFile>,
    #[serde(default)]
    pub themes: IndexMap<String, Spanned<ThemeFile>>,
}

/// A project file. Tables only allowed in the user file are accepted here
/// as any value, so they can be reported with a clear message.
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectFile {
    pub default_model: Option<Spanned<String>>,
    pub limits: Option<LimitsFile>,
    pub permissions: Option<PermissionsFile>,
    pub commit: Option<CommitFile>,
    pub providers: Option<Spanned<toml::Value>>,
    pub tui: Option<Spanned<toml::Value>>,
    pub themes: Option<Spanned<toml::Value>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderFile {
    pub api: Option<Spanned<String>>,
    pub base_url: Option<Spanned<String>>,
    pub api_key_env: Option<Spanned<String>>,
    pub api_key_cmd: Option<Spanned<String>>,
    pub timeout_s: Option<Spanned<i64>>,
    pub max_retries: Option<Spanned<i64>>,
    #[serde(default)]
    pub models: IndexMap<String, Spanned<ModelFile>>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFile {
    pub id: Option<Spanned<String>>,
    pub display_name: Option<String>,
    pub context_window: Option<Spanned<i64>>,
    pub max_output_tokens: Option<Spanned<i64>>,
    pub temperature: Option<Spanned<f64>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionsFile {
    pub read_file: Option<Spanned<Action>>,
    pub list_files: Option<Spanned<Action>>,
    pub search_files: Option<Spanned<Action>>,
    pub edit_file: Option<Spanned<Action>>,
    pub write_file: Option<Spanned<Action>>,
    pub run_command: Option<Spanned<Action>>,
    pub allow: Option<Spanned<RuleTable>>,
    pub ask: Option<Spanned<RuleTable>>,
    pub deny: Option<Spanned<RuleTable>>,
    #[serde(default)]
    pub rules: Vec<Spanned<DetailedRuleFile>>,
    /// Strings or tables; checked by hand for clear messages.
    pub extra_dirs: Option<Spanned<Vec<Spanned<toml::Value>>>>,
}

/// `[permissions.allow]` and the like: tool or group → patterns.
pub type RuleTable = IndexMap<String, Spanned<Vec<Spanned<String>>>>;

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DetailedRuleFile {
    pub action: Option<Spanned<Action>>,
    pub tools: Option<Spanned<Vec<Spanned<String>>>>,
    pub paths: Option<Spanned<Vec<Spanned<String>>>>,
    pub commands: Option<Spanned<Vec<Spanned<String>>>>,
    #[serde(default)]
    pub except: Vec<Spanned<String>>,
    pub message: Option<Spanned<String>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LimitsFile {
    pub max_requests: Option<Spanned<i64>>,
    pub max_tool_calls: Option<Spanned<i64>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommitFile {
    pub model: Option<Spanned<String>>,
    pub trailer: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TuiFile {
    pub theme: Option<Spanned<String>>,
    pub symbols: Option<Symbols>,
    pub mouse: Option<bool>,
    pub tool_output_lines: Option<Spanned<i64>>,
    pub expand_tool_calls: Option<bool>,
    pub syntax_highlight: Option<bool>,
    pub timestamps: Option<bool>,
    pub scroll_lines: Option<Spanned<i64>>,
    pub max_fps: Option<Spanned<i64>>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeFile {
    pub base: Option<Spanned<String>>,
    pub text: Option<Spanned<toml::Value>>,
    pub dim: Option<Spanned<toml::Value>>,
    pub user: Option<Spanned<toml::Value>>,
    pub accent: Option<Spanned<toml::Value>>,
    pub border: Option<Spanned<toml::Value>>,
    pub selection: Option<Spanned<toml::Value>>,
    pub success: Option<Spanned<toml::Value>>,
    pub error: Option<Spanned<toml::Value>>,
    pub warning: Option<Spanned<toml::Value>>,
    pub diff_add: Option<Spanned<toml::Value>>,
    pub diff_remove: Option<Spanned<toml::Value>>,
    pub code: Option<String>,
}
