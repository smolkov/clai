//! `[lsp]`: language servers for the `lsp` tool (RFC 0019).
//!
//! This module checks what the files say. Which servers are in use, with the
//! built-in ones and the programs on `PATH`, is decided by
//! [`crate::lsp::config`].

use std::time::Duration;

use indexmap::IndexMap;
use serde_json::Value;
use toml::Spanned;

use super::errors::{ConfigError, FileCtx};
use super::file::{LspFile, LspServerFile};
use super::int_key;
use super::permissions::Action;
use crate::lsp::config::builtin;

/// `[lsp]` as loaded.
#[derive(Debug, Clone, PartialEq)]
pub struct Lsp {
    /// `false`: no `lsp` tool and no servers.
    pub enabled: bool,
    /// Use the built-in servers whose program is on `PATH`.
    pub builtin: bool,
    /// Starting a server: ask, allow or deny.
    pub start: Action,
    /// Add the errors of a running server to `edit_file` and `write_file`.
    pub diagnostics_after_edit: bool,
    pub start_timeout: Duration,
    pub request_timeout: Duration,
    /// `[lsp.servers.NAME]`, in file order.
    pub servers: IndexMap<String, LspServer>,
}

impl Default for Lsp {
    fn default() -> Lsp {
        Lsp {
            enabled: true,
            builtin: true,
            start: Action::Ask,
            diagnostics_after_edit: true,
            start_timeout: Duration::from_secs(60),
            request_timeout: Duration::from_secs(20),
            servers: IndexMap::new(),
        }
    }
}

/// One `[lsp.servers.NAME]` table. A key left out of a table named like a
/// built-in server is taken from that server.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LspServer {
    /// Program and arguments; `Some(vec![])` turns a built-in server off.
    pub command: Option<Vec<String>>,
    /// Without the dot.
    pub extensions: Option<Vec<String>>,
    pub roots: Option<Vec<String>>,
    /// Sent as `initializationOptions` and for `workspace/configuration`.
    pub settings: Option<Value>,
    pub env: Vec<(String, String)>,
}

impl LspServer {
    /// Whether the table turns its server off (`command = []`).
    pub fn is_off(&self) -> bool {
        self.command.as_ref().is_some_and(Vec::is_empty)
    }
}

/// Builds `[lsp]` of the user file.
pub(super) fn build(
    ctx: FileCtx<'_>,
    file: Option<Spanned<LspFile>>,
    errors: &mut Vec<ConfigError>,
) -> Lsp {
    let mut lsp = Lsp::default();
    let Some(file) = file else {
        return lsp;
    };
    let file = file.into_inner();
    lsp.enabled = file.enabled.unwrap_or(lsp.enabled);
    lsp.builtin = file.builtin.unwrap_or(lsp.builtin);
    lsp.start = file.start.map_or(lsp.start, Spanned::into_inner);
    lsp.diagnostics_after_edit = file
        .diagnostics_after_edit
        .unwrap_or(lsp.diagnostics_after_edit);
    let start = int_key(
        ctx,
        &file.start_timeout_s,
        "lsp.start_timeout_s".into(),
        1..=3600,
        60,
        errors,
    );
    lsp.start_timeout = Duration::from_secs(u64::try_from(start).unwrap_or(60));
    let request = int_key(
        ctx,
        &file.request_timeout_s,
        "lsp.request_timeout_s".into(),
        1..=600,
        20,
        errors,
    );
    lsp.request_timeout = Duration::from_secs(u64::try_from(request).unwrap_or(20));

    // Extension → the server that claimed it first.
    let mut claimed: IndexMap<String, String> = IndexMap::new();
    for (name, spanned) in file.servers {
        let span = spanned.span();
        let key = format!("lsp.servers.{name}");
        let before = errors.len();
        let server = build_server(ctx, &name, &key, spanned.into_inner(), errors);
        if !valid_name(&name) {
            errors.push(ctx.error(
                Some(span.clone()),
                Some(key.clone()),
                format!("name `{name}` may only have letters, digits, `.`, `_` and `-`"),
            ));
        }
        if errors.len() != before {
            continue;
        }
        if !server.is_off() {
            let extensions = server
                .extensions
                .clone()
                .or_else(|| {
                    builtin(&name).map(|b| b.extensions.iter().map(|e| (*e).to_owned()).collect())
                })
                .unwrap_or_default();
            for ext in extensions {
                if let Some(other) = claimed.get(&ext) {
                    errors.push(ctx.error(
                        Some(span.clone()),
                        Some(format!("{key}.extensions")),
                        format!("`{ext}` is already served by {other}; one server per extension"),
                    ));
                } else {
                    claimed.insert(ext, name.clone());
                }
            }
        }
        lsp.servers.insert(name, server);
    }
    lsp
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn build_server(
    ctx: FileCtx<'_>,
    name: &str,
    key: &str,
    file: LspServerFile,
    errors: &mut Vec<ConfigError>,
) -> LspServer {
    let is_builtin = builtin(name).is_some();
    let mut server = LspServer::default();

    if let Some(command) = file.command {
        let span = command.span();
        let words: Vec<String> = command
            .into_inner()
            .into_iter()
            .map(Spanned::into_inner)
            .collect();
        if words.is_empty() && !is_builtin {
            errors.push(ctx.error(
                Some(span),
                Some(format!("{key}.command")),
                "must not be empty; `command = []` only turns off a built-in server",
            ));
        } else if words.first().is_some_and(String::is_empty) {
            errors.push(ctx.error(
                Some(span),
                Some(format!("{key}.command")),
                "the program must not be empty",
            ));
        }
        server.command = Some(words);
    } else if !is_builtin {
        errors.push(ctx.error(None, Some(key.to_owned()), "missing command"));
    }

    if let Some(extensions) = file.extensions {
        let span = extensions.span();
        let list = strings(extensions);
        if list.is_empty() {
            errors.push(ctx.error(
                Some(span.clone()),
                Some(format!("{key}.extensions")),
                "must not be empty",
            ));
        }
        for ext in &list {
            if ext.is_empty() || ext.starts_with('.') || ext.contains('/') {
                errors.push(ctx.error(
                    Some(span.clone()),
                    Some(format!("{key}.extensions")),
                    format!("`{ext}`: write the extension without the dot, e.g. \"rs\""),
                ));
            }
        }
        server.extensions = Some(list);
    } else if !is_builtin && !server.is_off() {
        errors.push(ctx.error(None, Some(key.to_owned()), "missing extensions"));
    }

    if let Some(roots) = file.roots {
        let span = roots.span();
        let list = strings(roots);
        for root in &list {
            let bad = root.is_empty()
                || root.starts_with('/')
                || root.starts_with('~')
                || root.split('/').any(|part| part == "..");
            if bad {
                errors.push(ctx.error(
                    Some(span.clone()),
                    Some(format!("{key}.roots")),
                    format!("`{root}`: a file or directory name, relative to the project root"),
                ));
            }
        }
        server.roots = Some(list);
    }

    if let Some(settings) = file.settings {
        let span = settings.span();
        match serde_json::to_value(settings.into_inner()) {
            Ok(value) => server.settings = Some(value),
            Err(e) => {
                errors.push(ctx.error(Some(span), Some(format!("{key}.settings")), e.to_string()))
            }
        }
    }

    if let Some(env) = file.env {
        let span = env.span();
        for (name, value) in env.into_inner() {
            if name.is_empty() || name.contains('=') {
                errors.push(ctx.error(
                    Some(span.clone()),
                    Some(format!("{key}.env")),
                    format!("`{name}` is not a variable name"),
                ));
            }
            server.env.push((name, value));
        }
    }
    server
}

fn strings(list: Spanned<Vec<Spanned<String>>>) -> Vec<String> {
    list.into_inner()
        .into_iter()
        .map(Spanned::into_inner)
        .collect()
}

/// Applies `[lsp]` of a project file: only `enabled = false` and
/// `start = "deny"`, since a server is a command that runs.
pub(super) fn apply_project(
    ctx: FileCtx<'_>,
    value: &Spanned<toml::Value>,
    lsp: &mut Lsp,
    errors: &mut Vec<ConfigError>,
) {
    let only = "a project file may only set enabled = false and start = \"deny\"";
    let Some(table) = value.get_ref().as_table() else {
        errors.push(ctx.error(Some(value.span()), Some("lsp".into()), "must be a table"));
        return;
    };
    for (key, item) in table {
        let ok = match key.as_str() {
            "enabled" => item.as_bool() == Some(false),
            "start" => item.as_str() == Some("deny"),
            _ => false,
        };
        if ok {
            match key.as_str() {
                "enabled" => lsp.enabled = false,
                _ => lsp.start = Action::Deny,
            }
        } else {
            errors.push(ctx.error(
                Some(value.span()),
                Some(format!("lsp.{key}")),
                format!("{only} (the rest only in the user file)"),
            ));
        }
    }
}
