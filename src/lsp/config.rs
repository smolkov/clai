//! Which language servers are in use: the built-in ones, merged with
//! `[lsp.servers]`, and the project root of a file.

use std::ffi::OsStr;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::server::Setup;
use crate::config::{Lsp, LspServer};

/// A server `clai` knows without configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Builtin {
    pub name: &'static str,
    pub command: &'static [&'static str],
    pub extensions: &'static [&'static str],
    pub roots: &'static [&'static str],
}

/// The built-in servers (`spec/config.md`, "LSP"), used when their program
/// is on `PATH` and `[lsp] builtin` is on.
pub const BUILTINS: [Builtin; 5] = [
    Builtin {
        name: "rust-analyzer",
        command: &["rust-analyzer"],
        extensions: &["rs"],
        roots: &["Cargo.toml"],
    },
    Builtin {
        name: "gopls",
        command: &["gopls"],
        extensions: &["go"],
        roots: &["go.mod"],
    },
    Builtin {
        name: "clangd",
        command: &["clangd"],
        extensions: &["c", "h", "cc", "cpp", "hpp"],
        roots: &["compile_commands.json", ".clangd"],
    },
    Builtin {
        name: "pyright",
        command: &["pyright-langserver", "--stdio"],
        extensions: &["py", "pyi"],
        roots: &["pyproject.toml", "setup.py"],
    },
    Builtin {
        name: "typescript-language-server",
        command: &["typescript-language-server", "--stdio"],
        extensions: &["ts", "tsx", "js", "jsx"],
        roots: &["package.json", "tsconfig.json"],
    },
];

/// The built-in server with this name.
pub fn builtin(name: &str) -> Option<&'static Builtin> {
    BUILTINS.iter().find(|b| b.name == name)
}

/// A server in use.
#[derive(Debug, Clone, PartialEq)]
pub struct ServerDef {
    pub name: String,
    pub command: Vec<String>,
    /// Without the dot.
    pub extensions: Vec<String>,
    /// Names that mark a project root; empty: the boundary is the root.
    pub roots: Vec<String>,
    /// `Null` for none.
    pub settings: Value,
    pub env: Vec<(String, String)>,
    /// A built-in server, maybe with keys of the user's table.
    pub builtin: bool,
}

impl ServerDef {
    /// What starts this server for `root`.
    pub fn setup(&self, root: &Path, lsp: &Lsp) -> Setup {
        Setup {
            name: self.name.clone(),
            command: self.command.clone(),
            env: self.env.clone(),
            root: root.to_owned(),
            settings: self.settings.clone(),
            start_timeout: lsp.start_timeout,
            request_timeout: lsp.request_timeout,
        }
    }

    /// `rust-analyzer (.rs)`, for messages that list the servers.
    pub fn label(&self) -> String {
        let exts: Vec<String> = self.extensions.iter().map(|e| format!(".{e}")).collect();
        format!("{} ({})", self.name, exts.join(", "))
    }
}

/// The servers in use, given `PATH` (`None`: no built-in server is
/// found). The user's servers come first, in file order; then the built-in
/// ones not replaced by name, whose program is on `PATH`, for the
/// extensions no user server takes.
pub fn servers(lsp: &Lsp, path: Option<&OsStr>) -> Vec<ServerDef> {
    if !lsp.enabled {
        return Vec::new();
    }
    let mut defs: Vec<ServerDef> = Vec::new();
    for (name, server) in &lsp.servers {
        if server.is_off() {
            continue;
        }
        defs.push(merged(name, server, builtin(name)));
    }
    if !lsp.builtin {
        return defs;
    }
    for b in &BUILTINS {
        if lsp.servers.contains_key(b.name) {
            continue;
        }
        let Some(program) = b.command.first() else {
            continue;
        };
        if !on_path(program, path) {
            continue;
        }
        let extensions: Vec<String> = b
            .extensions
            .iter()
            .filter(|ext| !defs.iter().any(|d| d.extensions.iter().any(|e| e == *ext)))
            .map(|e| (*e).to_owned())
            .collect();
        if extensions.is_empty() {
            continue;
        }
        let mut def = merged(b.name, &LspServer::default(), Some(b));
        def.extensions = extensions;
        defs.push(def);
    }
    defs
}

fn merged(name: &str, server: &LspServer, base: Option<&Builtin>) -> ServerDef {
    let owned = |list: &[&str]| list.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    ServerDef {
        name: name.to_owned(),
        command: server
            .command
            .clone()
            .or_else(|| base.map(|b| owned(b.command)))
            .unwrap_or_default(),
        extensions: server
            .extensions
            .clone()
            .or_else(|| base.map(|b| owned(b.extensions)))
            .unwrap_or_default(),
        roots: server
            .roots
            .clone()
            .or_else(|| base.map(|b| owned(b.roots)))
            .unwrap_or_default(),
        settings: server.settings.clone().unwrap_or(Value::Null),
        env: server.env.clone(),
        builtin: base.is_some(),
    }
}

/// The server for a file, by its extension.
pub fn for_file<'a>(defs: &'a [ServerDef], file: &Path) -> Option<&'a ServerDef> {
    let ext = file.extension()?.to_str()?;
    defs.iter().find(|d| d.extensions.iter().any(|e| e == ext))
}

/// The project root of `file` for `def`: the nearest directory, from the
/// file's own upwards and not above `boundary` (the workspace, or the extra
/// directory the file is in), that has one of the root names. Without
/// one, `boundary`.
pub fn find_root(def: &ServerDef, file: &Path, boundary: &Path) -> PathBuf {
    let mut dir = file.parent();
    while let Some(current) = dir {
        if !current.starts_with(boundary) {
            break;
        }
        if def.roots.iter().any(|name| current.join(name).exists()) {
            return current.to_owned();
        }
        if current == boundary {
            break;
        }
        dir = current.parent();
    }
    boundary.to_owned()
}

/// Whether `program` can be run: an executable file at that path when it
/// has a `/`, else in one of the directories of `path`.
pub fn on_path(program: &str, path: Option<&OsStr>) -> bool {
    let executable = |file: &Path| {
        file.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if program.contains('/') {
        return executable(Path::new(program));
    }
    let Some(path) = path else {
        return false;
    };
    std::env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .any(|dir| executable(&dir.join(program)))
}
