//! Configuration: files, precedence, validation (see `spec/config.md`).
//!
//! Precedence, later wins: user file, project file, environment, command
//! line. Command-line options are applied by each command, using
//! [`Config::resolve_model`].

pub mod edit;
mod errors;
mod file;
pub mod permissions;
mod secret;
pub mod themes;
pub mod tui;

use std::env;
use std::fmt;
use std::fs;
use std::io;
use std::ops::{Range, RangeInclusive};
use std::path::{Path, PathBuf};
use std::time::Duration;

use indexmap::IndexMap;
use toml::Spanned;

use crate::error::Error;
use errors::FileCtx;
use file::{LimitsFile, ModelFile, ProjectFile, ProviderFile, UserFile};

pub use errors::{ConfigError, ConfigErrors, Origin};
pub use permissions::{Access, Action, ExtraDir, Pattern, Permissions, Rule, Source, Tool};
pub use secret::{ApiKey, KEY_CMD_TIMEOUT, KeySource, KeyStatus};
pub use themes::ThemeText;
pub use tui::{BuiltinTheme, Colour, Symbols, Theme, ThemeSource, Tui};

/// Example user config, printed when there is none.
pub const EXAMPLE: &str = include_str!("example.toml");

/// Overrides `default_model`.
pub const MODEL_ENV: &str = "CLAI_MODEL";
/// Overrides `tui.theme`.
pub const THEME_ENV: &str = "CLAI_THEME";

/// API type of a provider: decides which adapter is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Api {
    Anthropic,
    OpenAiCompatible,
}

impl Api {
    pub fn from_name(name: &str) -> Option<Api> {
        match name {
            "anthropic" => Some(Api::Anthropic),
            "openai-compatible" => Some(Api::OpenAiCompatible),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Api::Anthropic => "anthropic",
            Api::OpenAiCompatible => "openai-compatible",
        }
    }

    /// Official address, if the API type has one.
    pub fn default_base_url(self) -> Option<&'static str> {
        match self {
            Api::Anthropic => Some("https://api.anthropic.com"),
            Api::OpenAiCompatible => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Model {
    /// Name of the config table.
    pub name: String,
    /// Model id sent to the API.
    pub id: String,
    pub display_name: String,
    pub context_window: u64,
    pub max_output_tokens: Option<u64>,
    pub temperature: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Provider {
    pub name: String,
    pub api: Api,
    pub base_url: String,
    pub key: KeySource,
    pub timeout: Duration,
    pub max_retries: u32,
    /// In config order.
    pub models: IndexMap<String, Model>,
}

/// `<provider>/<model>`, using config table names.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ModelRef {
    pub provider: String,
    pub model: String,
}

impl fmt::Display for ModelRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.provider, self.model)
    }
}

/// Budget of a turn (see `spec/agent-loop.md`, "Limits").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limits {
    pub max_requests: u32,
    pub max_tool_calls: u32,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_requests: 50,
            max_tool_calls: 120,
        }
    }
}

impl Limits {
    /// Reads `[limits]` of one file; with `lower_only`, a value only
    /// replaces the current one when it is lower.
    fn apply(
        &mut self,
        ctx: FileCtx<'_>,
        file: &LimitsFile,
        lower_only: bool,
        errors: &mut Vec<ConfigError>,
    ) {
        for (key, value, slot) in [
            (
                "limits.max_requests",
                &file.max_requests,
                &mut self.max_requests,
            ),
            (
                "limits.max_tool_calls",
                &file.max_tool_calls,
                &mut self.max_tool_calls,
            ),
        ] {
            let n = int_key(
                ctx,
                value,
                key.to_owned(),
                1..=10_000,
                i64::from(*slot),
                errors,
            );
            if let Ok(n) = u32::try_from(n)
                && (1..=10_000).contains(&n)
                && (!lower_only || n < *slot)
            {
                *slot = n;
            }
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Commit {
    pub model: Option<ModelRef>,
    pub trailer: Option<String>,
}

/// The loaded and validated configuration.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub user_file: PathBuf,
    /// The project file, if the workspace has one.
    pub project_file: Option<PathBuf>,
    pub default_model: ModelRef,
    /// In config order.
    pub providers: IndexMap<String, Provider>,
    pub limits: Limits,
    pub permissions: Permissions,
    pub commit: Commit,
    pub tui: Tui,
    /// Bundled themes, theme files and `[themes.NAME]` tables, in that order.
    pub themes: IndexMap<String, Theme>,
    /// Theme files that are broken but not in use: left out, and shown by the
    /// TUI.
    pub theme_warnings: Vec<ThemeWarning>,
}

/// A theme file left out, and why: `themes/x.toml:3: base: …`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeWarning {
    pub name: String,
    pub message: String,
}

impl Config {
    /// Resolves `provider/model`, or `model` when only one provider has it.
    pub fn resolve_model(&self, text: &str) -> Result<ModelRef, String> {
        let declared: Vec<(&str, &str)> = self
            .providers
            .iter()
            .flat_map(|(p, provider)| {
                provider
                    .models
                    .keys()
                    .map(move |m| (p.as_str(), m.as_str()))
            })
            .collect();
        resolve(&declared, text)
    }

    pub fn model(&self, model: &ModelRef) -> Option<(&Provider, &Model)> {
        let provider = self.providers.get(&model.provider)?;
        Some((provider, provider.models.get(&model.model)?))
    }
}

fn resolve(declared: &[(&str, &str)], text: &str) -> Result<ModelRef, String> {
    let unknown = || {
        let list: Vec<String> = declared.iter().map(|(p, m)| format!("{p}/{m}")).collect();
        if list.is_empty() {
            format!("unknown model `{text}`; no models are configured")
        } else {
            format!("unknown model `{text}`; configured: {}", list.join(", "))
        }
    };
    let found = |p: &str, m: &str| ModelRef {
        provider: p.to_owned(),
        model: m.to_owned(),
    };
    if let Some((provider, model)) = text.split_once('/') {
        return if declared.contains(&(provider, model)) {
            Ok(found(provider, model))
        } else {
            Err(unknown())
        };
    }
    let matches: Vec<&(&str, &str)> = declared.iter().filter(|(_, m)| *m == text).collect();
    match matches.as_slice() {
        [(p, m)] => Ok(found(p, m)),
        [] => Err(unknown()),
        several => {
            let list: Vec<String> = several.iter().map(|(p, m)| format!("{p}/{m}")).collect();
            Err(format!(
                "`{text}` is defined by several providers: {}; write it as provider/model",
                list.join(", ")
            ))
        }
    }
}

/// The environment `clai` reads its configuration from.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Environment {
    pub xdg_config_home: Option<PathBuf>,
    pub home: Option<PathBuf>,
    /// `CLAI_MODEL`
    pub model: Option<String>,
    /// `CLAI_THEME`
    pub theme: Option<String>,
}

impl Environment {
    pub fn from_process() -> Environment {
        let text = |name| env::var(name).ok().filter(|v: &String| !v.is_empty());
        Environment {
            xdg_config_home: env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
            home: env::var_os("HOME").map(PathBuf::from),
            model: text(MODEL_ENV),
            theme: text(THEME_ENV),
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadOptions {
    /// `--config FILE`: used instead of the user file.
    pub config_file: Option<PathBuf>,
    /// Directory whose `.clai/config.toml` is the project file.
    pub workspace: PathBuf,
    pub env: Environment,
}

/// `$XDG_CONFIG_HOME/clai/config.toml`, else `~/.config/clai/config.toml`.
pub fn user_config_path(env: &Environment) -> Option<PathBuf> {
    let base = env
        .xdg_config_home
        .clone()
        .filter(|p| p.is_absolute())
        .or_else(|| env.home.as_ref().map(|h| h.join(".config")))?;
    Some(base.join("clai").join("config.toml"))
}

/// Loads and validates the configuration.
pub fn load(options: &LoadOptions) -> Result<Config, Error> {
    let (user_path, user_text) = read_user_file(options)?;
    load_with_user_text(&user_path, &user_text, options)
}

/// The path and text of the user file.
pub fn read_user_file(options: &LoadOptions) -> Result<(PathBuf, String), Error> {
    let user_path = match &options.config_file {
        Some(path) => path.clone(),
        None => user_config_path(&options.env).ok_or_else(|| {
            Error::Usage(
                "cannot find the user config file: neither XDG_CONFIG_HOME nor HOME is set"
                    .to_owned(),
            )
        })?,
    };
    let user_text = match fs::read_to_string(&user_path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound && options.config_file.is_none() => {
            return Err(Error::NoUserConfig { path: user_path });
        }
        Err(e) => return Err(read_error(&user_path, &e)),
    };
    Ok((user_path, user_text))
}

/// Loads the configuration with the user file's text given, e.g. the last
/// valid one the server kept (see `spec/config.md`, "Reloading").
pub fn load_with_user_text(
    user_path: &Path,
    user_text: &str,
    options: &LoadOptions,
) -> Result<Config, Error> {
    let project_path = options.workspace.join(".clai").join("config.toml");
    let project_text = match fs::read_to_string(&project_path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == io::ErrorKind::NotFound => None,
        Err(e) => return Err(read_error(&project_path, &e)),
    };

    let user = FileCtx {
        path: user_path,
        text: user_text,
    };
    let project = project_text.as_deref().map(|text| FileCtx {
        path: &project_path,
        text,
    });
    let theme_files = themes::read_directory(&themes::directory(user_path));
    build(user, project, &options.env, &theme_files).map_err(Error::Config)
}

fn read_error(path: &Path, e: &io::Error) -> Error {
    Error::Config(ConfigErrors(vec![ConfigError {
        origin: Origin::File {
            path: path.to_owned(),
            line: None,
        },
        key: None,
        message: format!("cannot read: {e}"),
    }]))
}

fn env_error(name: &'static str, message: String) -> ConfigError {
    ConfigError {
        origin: Origin::Env(name),
        key: None,
        message,
    }
}

/// Builds the configuration from file texts and the environment.
fn build(
    user: FileCtx<'_>,
    project: Option<FileCtx<'_>>,
    env: &Environment,
    theme_files: &[ThemeText],
) -> Result<Config, ConfigErrors> {
    let user_file: UserFile = user.parse().map_err(|e| ConfigErrors(vec![e]))?;
    let project_file: Option<ProjectFile> = match project {
        Some(ctx) => Some(ctx.parse().map_err(|e| ConfigErrors(vec![e]))?),
        None => None,
    };
    let mut errors = Vec::new();

    // Every model named in the user file, valid or not, so that one bad
    // model does not also make every reference to it an error.
    let declared_owned: Vec<(String, String)> = user_file
        .providers
        .iter()
        .flat_map(|(p, provider)| {
            provider
                .get_ref()
                .models
                .keys()
                .map(move |m| (p.clone(), m.clone()))
        })
        .collect();
    let declared: Vec<(&str, &str)> = declared_owned
        .iter()
        .map(|(p, m)| (p.as_str(), m.as_str()))
        .collect();
    if user_file.providers.is_empty() {
        errors.push(user.error(None, None, "no provider is defined"));
    } else if declared.is_empty() {
        errors.push(user.error(None, None, "no model is defined"));
    }

    let resolve_at =
        |ctx: FileCtx<'_>, value: &Spanned<String>, key: &str, errors: &mut Vec<ConfigError>| {
            resolve(&declared, value.get_ref())
                .map_err(|message| {
                    errors.push(ctx.error(Some(value.span()), Some(key.into()), message))
                })
                .ok()
        };

    let providers = build_providers(user, user_file.providers, &mut errors);
    // Themes: bundled, then files, then tables. A broken file counts only if
    // its theme is the one in use.
    let (mut themes, broken) = themes::file_themes(theme_files);
    let tables = tui::build_themes(user, user_file.themes, &mut errors);
    for (name, theme) in tables {
        if let Some(Theme {
            source: ThemeSource::File(path),
            ..
        }) = themes.get(&name)
        {
            errors.push(user.error(
                None,
                Some(format!("themes.{name}")),
                format!("also defined in {}; remove one", path.display()),
            ));
        }
        themes.insert(name, theme);
    }
    let wanted: Vec<&str> = user_file
        .tui
        .as_ref()
        .and_then(|t| t.theme.as_ref())
        .map(|t| t.get_ref().as_str())
        .into_iter()
        .chain(env.theme.as_deref())
        .collect();
    let mut theme_warnings = Vec::new();
    let mut in_use_broken = Vec::new();
    for b in broken {
        if wanted.contains(&b.name.as_str()) {
            in_use_broken.push(b.name.clone());
            errors.extend(b.errors);
        } else {
            let messages: Vec<String> = b.errors.iter().map(ToString::to_string).collect();
            theme_warnings.push(ThemeWarning {
                name: b.name,
                message: messages.join("; "),
            });
        }
    }
    let mut tui = tui::build_tui(user, user_file.tui, &themes, &in_use_broken, &mut errors);
    let mut permissions = Permissions::from_user(
        user,
        user_file.permissions,
        env.home.as_deref(),
        &mut errors,
    );

    let mut limits = Limits::default();
    if let Some(file) = &user_file.limits {
        limits.apply(user, file, false, &mut errors);
    }

    let mut default_given = false;
    let mut default_model = None;
    if let Some(value) = &user_file.default_model {
        default_given = true;
        default_model = resolve_at(user, value, "default_model", &mut errors);
    }
    let mut commit = Commit::default();
    if let Some(file) = user_file.commit {
        if let Some(value) = &file.model {
            commit.model = resolve_at(user, value, "commit.model", &mut errors);
        }
        commit.trailer = file.trailer;
    }

    if let (Some(ctx), Some(file)) = (project, project_file) {
        for (key, value) in [
            ("providers", &file.providers),
            ("tui", &file.tui),
            ("themes", &file.themes),
        ] {
            if let Some(value) = value {
                errors.push(ctx.error(
                    Some(value.span()),
                    Some(key.into()),
                    format!("only allowed in the user file ({})", user.path.display()),
                ));
            }
        }
        if let Some(value) = &file.default_model {
            default_given = true;
            if let Some(model) = resolve_at(ctx, value, "default_model", &mut errors) {
                default_model = Some(model);
            }
        }
        if let Some(limits_file) = &file.limits {
            limits.apply(ctx, limits_file, true, &mut errors);
        }
        if let Some(perms) = file.permissions {
            permissions.apply_project(ctx, perms, env.home.as_deref(), &mut errors);
        }
        if let Some(file) = file.commit {
            if let Some(value) = &file.model
                && let Some(model) = resolve_at(ctx, value, "commit.model", &mut errors)
            {
                commit.model = Some(model);
            }
            if file.trailer.is_some() {
                commit.trailer = file.trailer;
            }
        }
    }

    if let Some(text) = &env.model {
        default_given = true;
        match resolve(&declared, text) {
            Ok(model) => default_model = Some(model),
            Err(message) => errors.push(env_error(MODEL_ENV, message)),
        }
    }
    if let Some(theme) = &env.theme {
        if tui::theme_exists(theme, &themes) || in_use_broken.contains(theme) {
            tui.theme = theme.clone();
        } else {
            errors.push(env_error(THEME_ENV, format!("unknown theme `{theme}`")));
        }
    }

    if !default_given {
        errors.push(user.error(None, None, "missing default_model"));
    }
    match default_model {
        Some(default_model) if errors.is_empty() => Ok(Config {
            user_file: user.path.to_owned(),
            project_file: project.map(|p| p.path.to_owned()),
            default_model,
            providers,
            limits,
            permissions,
            commit,
            tui,
            themes,
            theme_warnings,
        }),
        _ => Err(ConfigErrors(errors)),
    }
}

/// Checks an integer key and returns it, or `default` when it is not set.
fn int_key(
    ctx: FileCtx<'_>,
    value: &Option<Spanned<i64>>,
    key: String,
    range: RangeInclusive<i64>,
    default: i64,
    errors: &mut Vec<ConfigError>,
) -> i64 {
    let Some(value) = value else {
        return default;
    };
    let n = *value.get_ref();
    if !range.contains(&n) {
        errors.push(ctx.error(
            Some(value.span()),
            Some(key),
            format!("{n} is not in {}–{}", range.start(), range.end()),
        ));
    }
    n
}

fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.contains('/') {
        Err(format!("name `{name}` must not be empty or contain `/`"))
    } else {
        Ok(())
    }
}

fn build_providers(
    ctx: FileCtx<'_>,
    providers: IndexMap<String, Spanned<ProviderFile>>,
    errors: &mut Vec<ConfigError>,
) -> IndexMap<String, Provider> {
    let mut built = IndexMap::new();
    for (name, spanned) in providers {
        let before = errors.len();
        let span = spanned.span();
        let file = spanned.into_inner();
        let key = format!("providers.{name}");
        let err = |span: Range<usize>, key: String, message: String| {
            ctx.error(Some(span), Some(key), message)
        };

        if let Err(message) = check_name(&name) {
            errors.push(err(span.clone(), key.clone(), message));
        }
        let api = match &file.api {
            None => {
                errors.push(err(span.clone(), key.clone(), "missing api".into()));
                None
            }
            Some(api) => {
                let parsed = Api::from_name(api.get_ref());
                if parsed.is_none() {
                    errors.push(err(
                        api.span(),
                        format!("{key}.api"),
                        format!(
                            "unknown api `{}`; expected anthropic or openai-compatible",
                            api.get_ref()
                        ),
                    ));
                }
                parsed
            }
        };
        let sources = [&file.api_key, &file.api_key_env, &file.api_key_cmd]
            .iter()
            .filter(|s| s.is_some())
            .count();
        let key_source = if sources > 1 {
            errors.push(err(
                span.clone(),
                key.clone(),
                "set only one of api_key, api_key_env and api_key_cmd".into(),
            ));
            KeySource::None
        } else if let Some(value) = &file.api_key {
            if value.get_ref().is_empty() {
                errors.push(err(
                    value.span(),
                    format!("{key}.api_key"),
                    "must not be empty".into(),
                ));
            }
            KeySource::Value(ApiKey::new(value.get_ref().clone()))
        } else if let Some(var) = &file.api_key_env {
            KeySource::Env(var.get_ref().clone())
        } else if let Some(cmd) = &file.api_key_cmd {
            KeySource::Cmd(cmd.get_ref().clone())
        } else {
            KeySource::None
        };
        let base_url = match (&file.base_url, api) {
            (Some(url), _) => {
                let text = url.get_ref();
                if !(text.starts_with("http://") || text.starts_with("https://")) {
                    errors.push(err(
                        url.span(),
                        format!("{key}.base_url"),
                        format!("`{text}` must start with http:// or https://"),
                    ));
                }
                text.clone()
            }
            (None, Some(api)) => match api.default_base_url() {
                Some(url) => url.to_owned(),
                None => {
                    errors.push(err(
                        span.clone(),
                        key.clone(),
                        format!("missing base_url (required for {})", api.name()),
                    ));
                    String::new()
                }
            },
            (None, None) => String::new(),
        };
        let timeout = int_key(
            ctx,
            &file.timeout_s,
            format!("{key}.timeout_s"),
            1..=86_400,
            300,
            errors,
        );
        let max_retries = int_key(
            ctx,
            &file.max_retries,
            format!("{key}.max_retries"),
            0..=100,
            3,
            errors,
        );

        let mut models = IndexMap::new();
        for (model_name, model) in file.models {
            if let Some(model) = build_model(ctx, &key, model_name.clone(), model, errors) {
                models.insert(model_name, model);
            }
        }

        if let (Some(api), true) = (api, errors.len() == before) {
            built.insert(
                name.clone(),
                Provider {
                    name,
                    api,
                    base_url,
                    key: key_source,
                    timeout: Duration::from_secs(u64::try_from(timeout).unwrap_or(300)),
                    max_retries: u32::try_from(max_retries).unwrap_or(3),
                    models,
                },
            );
        }
    }
    built
}

fn build_model(
    ctx: FileCtx<'_>,
    provider_key: &str,
    name: String,
    spanned: Spanned<ModelFile>,
    errors: &mut Vec<ConfigError>,
) -> Option<Model> {
    let before = errors.len();
    let span = spanned.span();
    let file = spanned.into_inner();
    let key = format!("{provider_key}.models.{name}");
    if let Err(message) = check_name(&name) {
        errors.push(ctx.error(Some(span.clone()), Some(key.clone()), message));
    }
    let id = match file.id {
        Some(id) => id.into_inner(),
        None => {
            errors.push(ctx.error(Some(span.clone()), Some(key.clone()), "missing id"));
            String::new()
        }
    };
    let context_window = match &file.context_window {
        Some(_) => int_key(
            ctx,
            &file.context_window,
            format!("{key}.context_window"),
            1..=i64::MAX,
            0,
            errors,
        ),
        None => {
            errors.push(ctx.error(Some(span), Some(key.clone()), "missing context_window"));
            0
        }
    };
    let max_output_tokens = file.max_output_tokens.as_ref().map(|_| {
        int_key(
            ctx,
            &file.max_output_tokens,
            format!("{key}.max_output_tokens"),
            1..=i64::MAX,
            0,
            errors,
        )
    });
    if let Some(t) = &file.temperature
        && !(0.0..=2.0).contains(t.get_ref())
    {
        errors.push(ctx.error(
            Some(t.span()),
            Some(format!("{key}.temperature")),
            format!("{} is not in 0–2", t.get_ref()),
        ));
    }
    if errors.len() != before {
        return None;
    }
    Some(Model {
        display_name: file.display_name.unwrap_or_else(|| name.clone()),
        name,
        id,
        context_window: u64::try_from(context_window).unwrap_or(0),
        max_output_tokens: max_output_tokens.and_then(|n| u64::try_from(n).ok()),
        temperature: file.temperature.map(Spanned::into_inner),
    })
}

#[cfg(test)]
mod tests;
