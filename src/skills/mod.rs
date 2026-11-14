//! Skills: directories with a `SKILL.md` the user turns on for a session
//! (see `spec/skills.md`, RFC 0023).
//!
//! [`find`] searches the configured directories and the default ones of
//! other agents, in order of priority, and marks a skill whose name a
//! directory before it already has as shadowed.

pub mod parse;

use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{Config, Environment, Skills};

/// Where a skills directory comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    /// `skills.dirs` of the user file.
    Config,
    /// A default directory in the workspace.
    Project,
    /// A default directory in the home directory.
    User,
}

impl Source {
    pub fn name(self) -> &'static str {
        match self {
            Source::Config => "config",
            Source::Project => "project",
            Source::User => "user",
        }
    }
}

/// A directory searched for skills.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchDir {
    pub path: PathBuf,
    pub source: Source,
}

/// A skill found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skill {
    pub name: String,
    pub description: String,
    /// The skill's directory, as found (links not resolved).
    pub dir: PathBuf,
    pub source: Source,
    /// The directory of the skill of the same name that wins, if any.
    pub shadowed_by: Option<PathBuf>,
    /// The instructions of `SKILL.md`.
    pub body: String,
}

/// What [`find`] found.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Found {
    /// The directories searched, in order of priority.
    pub dirs: Vec<SearchDir>,
    /// In order of priority, and by directory name within a directory.
    pub skills: Vec<Skill>,
    /// Skipped files and directories that cannot be read.
    pub warnings: Vec<String>,
}

impl Found {
    /// The skill of that name that can be turned on: not shadowed.
    pub fn get(&self, name: &str) -> Option<&Skill> {
        self.skills
            .iter()
            .find(|s| s.name == name && s.shadowed_by.is_none())
    }

    /// The names that can be turned on, in order.
    pub fn names(&self) -> Vec<&str> {
        self.skills
            .iter()
            .filter(|s| s.shadowed_by.is_none())
            .map(|s| s.name.as_str())
            .collect()
    }

    /// The error for a name that is not found: it lists the found names.
    pub fn unknown(&self, name: &str) -> String {
        unknown(name, &self.names())
    }

    /// The skill `name` as a session keeps it once turned on.
    pub fn activate(&self, name: &str) -> Option<Active> {
        self.get(name).map(|s| Active {
            name: s.name.clone(),
            dir: s.dir.clone(),
            text: s.body.clone(),
        })
    }

    /// Whether the active skill is no longer the one found under its name:
    /// gone, shadowed now, or in another directory.
    pub fn missing(&self, active: &Active) -> bool {
        self.get(&active.name).is_none_or(|s| s.dir != active.dir)
    }

    /// Whether the active skill's `SKILL.md` has other instructions now.
    pub fn changed_on_disk(&self, active: &Active) -> bool {
        self.get(&active.name)
            .is_some_and(|s| s.dir == active.dir && s.body != active.text)
    }
}

/// The error for a skill name that is not found, with the names that are.
pub fn unknown(name: &str, found: &[&str]) -> String {
    if found.is_empty() {
        format!("unknown skill `{name}`; no skills are found")
    } else {
        format!("unknown skill `{name}`; found: {}", found.join(", "))
    }
}

/// A skill a session has turned on: its instructions as read then.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Active {
    pub name: String,
    pub dir: PathBuf,
    pub text: String,
}

impl Active {
    /// A rough count of the tokens its text adds to the prompt: four bytes
    /// a token.
    pub fn tokens(&self) -> u64 {
        u64::try_from(self.text.len().div_ceil(4)).unwrap_or(u64::MAX)
    }
}

/// What [`apply`] makes of a change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Applied {
    /// The active skills after it.
    pub active: Vec<Active>,
    pub added: Vec<Active>,
    pub removed: Vec<String>,
}

/// The active skills after a change: those kept in their order, then the
/// new ones in the order named. `names` is the whole new list; a name not
/// active yet must be in `found`.
pub fn apply(
    active: &[Active],
    names: &[String],
    found: &[Active],
    found_names: &[&str],
) -> Result<Applied, String> {
    let mut next: Vec<Active> = active
        .iter()
        .filter(|a| names.contains(&a.name))
        .cloned()
        .collect();
    let mut added = Vec::new();
    for name in names {
        if next.iter().any(|a| &a.name == name) {
            continue;
        }
        let skill = found
            .iter()
            .find(|a| &a.name == name)
            .ok_or_else(|| unknown(name, found_names))?;
        next.push(skill.clone());
        added.push(skill.clone());
    }
    let removed = active
        .iter()
        .filter(|a| !names.contains(&a.name))
        .map(|a| a.name.clone())
        .collect();
    Ok(Applied {
        active: next,
        added,
        removed,
    })
}

/// The active skills a session log leaves: its `skills_changed` events
/// replayed.
pub fn replay<'a>(changes: impl IntoIterator<Item = (&'a [Active], &'a [String])>) -> Vec<Active> {
    let mut active: Vec<Active> = Vec::new();
    for (added, removed) in changes {
        active.retain(|a| !removed.contains(&a.name));
        for skill in added {
            active.retain(|a| a.name != skill.name);
            active.push(skill.clone());
        }
    }
    active
}

/// The notice of a change: `skills: + release-check, − pdf`.
pub fn change_text(added: &[Active], removed: &[String]) -> String {
    let parts: Vec<String> = added
        .iter()
        .map(|a| format!("+ {}", a.name))
        .chain(removed.iter().map(|n| format!("\u{2212} {n}")))
        .collect();
    format!("skills: {}", parts.join(", "))
}

/// The directories to search, highest priority first.
pub fn search_dirs(
    config: &Skills,
    workspace: &Path,
    home: Option<&Path>,
    xdg_config_home: Option<&Path>,
) -> Vec<SearchDir> {
    let mut dirs: Vec<SearchDir> = config
        .dirs
        .iter()
        .map(|path| SearchDir {
            path: path.clone(),
            source: Source::Config,
        })
        .collect();
    if !config.default_dirs {
        return dirs;
    }
    for sub in [".opencode", ".claude", ".agents"] {
        dirs.push(SearchDir {
            path: workspace.join(sub).join("skills"),
            source: Source::Project,
        });
    }
    let config_home = xdg_config_home
        .filter(|p| p.is_absolute())
        .map(Path::to_path_buf)
        .or_else(|| home.map(|h| h.join(".config")));
    if let Some(base) = config_home {
        dirs.push(SearchDir {
            path: base.join("opencode").join("skills"),
            source: Source::User,
        });
    }
    if let Some(home) = home {
        for sub in [".claude", ".agents"] {
            dirs.push(SearchDir {
                path: home.join(sub).join("skills"),
                source: Source::User,
            });
        }
    }
    dirs
}

/// Finds the skills for `workspace` with the configuration and environment
/// `clai` loaded.
pub fn find_for(config: &Config, workspace: &Path, env: &Environment) -> Found {
    find(
        &config.skills,
        workspace,
        env.home.as_deref(),
        env.xdg_config_home.as_deref(),
    )
}

/// Finds the skills for `workspace`.
pub fn find(
    config: &Skills,
    workspace: &Path,
    home: Option<&Path>,
    xdg_config_home: Option<&Path>,
) -> Found {
    let dirs = search_dirs(config, workspace, home, xdg_config_home);
    let show = |path: &Path| display(path, workspace, home);
    let mut skills: Vec<Skill> = Vec::new();
    let mut warnings = Vec::new();
    for search in &dirs {
        match fs::read_dir(&search.path) {
            Ok(_) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                if search.source == Source::Config {
                    warnings.push(format!("{}: not found", show(&search.path)));
                }
                continue;
            }
            Err(e) => {
                warnings.push(format!("{}: cannot read: {e}", show(&search.path)));
                continue;
            }
        }
        let mut subdirs = Vec::new();
        let mut visited = HashSet::new();
        walk(
            &search.path,
            0,
            &mut visited,
            &mut subdirs,
            &mut warnings,
            &show,
        );
        for (dir_name, dir) in subdirs {
            let file = dir.join("SKILL.md");
            let bytes = match fs::read(&file) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => {
                    warnings.push(format!("{}: cannot read: {e}", show(&file)));
                    continue;
                }
            };
            let parsed = match parse::parse(&bytes) {
                Ok(parsed) => parsed,
                Err(problem) => {
                    let at = problem.line.map(|l| format!(":{l}")).unwrap_or_default();
                    warnings.push(format!("{}{at}: {}", show(&file), problem.message));
                    continue;
                }
            };
            let name = parsed.name.unwrap_or_else(|| dir_name.clone());
            if !parse::valid_name(&name) {
                warnings.push(format!(
                    "{}: name `{name}`: use 1 to 64 lowercase letters, digits and -, not at the start or end",
                    show(&file)
                ));
                continue;
            }
            if name != dir_name {
                warnings.push(format!(
                    "{}: name `{name}` differs from the directory name; `{name}` is used",
                    show(&file)
                ));
            }
            let shadowed_by = skills
                .iter()
                .find(|s| s.name == name && s.shadowed_by.is_none())
                .map(|s| s.dir.clone());
            skills.push(Skill {
                name,
                description: parsed.description,
                dir,
                source: search.source,
                shadowed_by,
                body: parsed.body,
            });
        }
    }
    Found {
        dirs,
        skills,
        warnings,
    }
}

/// How deep below a skills directory skills are looked for:
/// `synced/<id>/<skill>/SKILL.md` is 3.
pub const MAX_DEPTH: usize = 6;

/// Adds the skill directories below `dir` to `out`, depth first, by name: a
/// directory with a `SKILL.md` is a skill and is not searched further.
/// Hidden directories are passed over, links are followed, and each
/// directory is visited once, so a link loop ends.
fn walk(
    dir: &Path,
    depth: usize,
    visited: &mut HashSet<PathBuf>,
    out: &mut Vec<(String, PathBuf)>,
    warnings: &mut Vec<String>,
    show: &dyn Fn(&Path) -> String,
) {
    if depth >= MAX_DEPTH {
        return;
    }
    if let Ok(real) = dir.canonicalize()
        && !visited.insert(real)
    {
        return;
    }
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => {
            // The skills directory itself is reported by the caller.
            if depth > 0 {
                warnings.push(format!("{}: cannot read: {e}", show(dir)));
            }
            return;
        }
    };
    let mut subdirs: Vec<(String, PathBuf)> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().to_str()?.to_owned();
            let path = entry.path();
            // `metadata` follows links: a linked skill counts.
            (!name.starts_with('.') && fs::metadata(&path).is_ok_and(|m| m.is_dir()))
                .then_some((name, path))
        })
        .collect();
    subdirs.sort();
    for (name, path) in subdirs {
        if fs::symlink_metadata(path.join("SKILL.md")).is_ok() {
            out.push((name, path));
        } else {
            walk(&path, depth + 1, visited, out, warnings, show);
        }
    }
}

/// A path as the lists show it: relative to the workspace inside it,
/// `~/…` in the home directory, else absolute.
pub fn display(path: &Path, workspace: &Path, home: Option<&Path>) -> String {
    if let Ok(rel) = path.strip_prefix(workspace)
        && !rel.as_os_str().is_empty()
    {
        return rel.display().to_string();
    }
    if let Some(home) = home
        && let Ok(rel) = path.strip_prefix(home)
    {
        return if rel.as_os_str().is_empty() {
            "~".to_owned()
        } else {
            format!("~/{}", rel.display())
        };
    }
    path.display().to_string()
}

#[cfg(test)]
mod tests;
