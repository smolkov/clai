//! `[permissions]`: tool defaults, rules and extra directories.
//!
//! Only loading and merging live here; matching rules against tool calls
//! belongs to the tools milestone.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use toml::Spanned;

use super::errors::{ConfigError, FileCtx};
use super::file::PermissionsFile;

/// What happens with a tool call. Ordered from loose to strict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Allow,
    Ask,
    Deny,
}

impl Action {
    pub fn name(self) -> &'static str {
        match self {
            Action::Allow => "allow",
            Action::Ask => "ask",
            Action::Deny => "deny",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tool {
    ReadFile,
    ListFiles,
    SearchFiles,
    EditFile,
    WriteFile,
    RunCommand,
}

impl Tool {
    pub const ALL: [Tool; 6] = [
        Tool::ReadFile,
        Tool::ListFiles,
        Tool::SearchFiles,
        Tool::EditFile,
        Tool::WriteFile,
        Tool::RunCommand,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Tool::ReadFile => "read_file",
            Tool::ListFiles => "list_files",
            Tool::SearchFiles => "search_files",
            Tool::EditFile => "edit_file",
            Tool::WriteFile => "write_file",
            Tool::RunCommand => "run_command",
        }
    }

    pub fn from_name(name: &str) -> Option<Tool> {
        Tool::ALL.into_iter().find(|t| t.name() == name)
    }

    /// Action when neither the configuration nor a rule decides.
    pub fn default_action(self) -> Action {
        match self {
            Tool::ReadFile | Tool::ListFiles | Tool::SearchFiles => Action::Allow,
            Tool::EditFile | Tool::WriteFile | Tool::RunCommand => Action::Ask,
        }
    }
}

/// `TOOL` or `TOOL(PATTERN)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub tool: Tool,
    pub pattern: Option<String>,
}

impl Rule {
    pub fn parse(text: &str) -> Result<Rule, String> {
        let (name, pattern) = match text.split_once('(') {
            None => (text, None),
            Some((name, rest)) => {
                let Some(pattern) = rest.strip_suffix(')') else {
                    return Err(format!("rule `{text}`: missing `)` at the end"));
                };
                if pattern.is_empty() {
                    return Err(format!("rule `{text}`: empty pattern"));
                }
                (name, Some(pattern.to_owned()))
            }
        };
        let tool = Tool::from_name(name).ok_or_else(|| {
            format!(
                "rule `{text}`: unknown tool `{name}`; tools are {}",
                tool_list()
            )
        })?;
        Ok(Rule { tool, pattern })
    }
}

impl fmt::Display for Rule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.pattern {
            Some(pattern) => write!(f, "{}({pattern})", self.tool.name()),
            None => f.write_str(self.tool.name()),
        }
    }
}

fn tool_list() -> String {
    Tool::ALL.map(Tool::name).join(", ")
}

/// Permissions in effect after merging user and project files.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Permissions {
    /// Tool keys set in a file; other tools use their default action.
    tools: Vec<(Tool, Action)>,
    pub allow: Vec<Rule>,
    pub ask: Vec<Rule>,
    pub deny: Vec<Rule>,
    pub extra_dirs: Vec<PathBuf>,
}

impl Permissions {
    /// Action for a tool when no rule matches.
    pub fn action(&self, tool: Tool) -> Action {
        self.tools
            .iter()
            .find(|(t, _)| *t == tool)
            .map_or(tool.default_action(), |(_, a)| *a)
    }

    fn set(&mut self, tool: Tool, action: Action) {
        match self.tools.iter_mut().find(|(t, _)| *t == tool) {
            Some(entry) => entry.1 = action,
            None => self.tools.push((tool, action)),
        }
    }

    /// From the user file.
    pub(crate) fn from_user(
        ctx: FileCtx<'_>,
        file: Option<PermissionsFile>,
        home: Option<&Path>,
        errors: &mut Vec<ConfigError>,
    ) -> Permissions {
        let mut perms = Permissions::default();
        let Some(file) = file else {
            return perms;
        };
        for (tool, value) in tool_keys(&file) {
            if let Some(value) = value {
                perms.set(tool, *value.get_ref());
            }
        }
        perms.allow = parse_rules(ctx, &file.allow, "permissions.allow", errors);
        perms.ask = parse_rules(ctx, &file.ask, "permissions.ask", errors);
        perms.deny = parse_rules(ctx, &file.deny, "permissions.deny", errors);
        if let Some(dirs) = &file.extra_dirs {
            for dir in dirs.get_ref() {
                match expand_dir(dir.get_ref(), home) {
                    Ok(path) => perms.extra_dirs.push(path),
                    Err(message) => errors.push(ctx.error(
                        Some(dir.span()),
                        Some("permissions.extra_dirs".into()),
                        message,
                    )),
                }
            }
        }
        perms
    }

    /// Adds a project file, which may only make permissions stricter.
    pub(crate) fn apply_project(
        &mut self,
        ctx: FileCtx<'_>,
        file: PermissionsFile,
        errors: &mut Vec<ConfigError>,
    ) {
        for (tool, value) in tool_keys(&file) {
            let Some(value) = value else { continue };
            let action = *value.get_ref();
            if action == Action::Allow {
                errors.push(ctx.error(
                    Some(value.span()),
                    Some(format!("permissions.{}", tool.name())),
                    "a project file may not set a tool to allow",
                ));
            } else {
                self.set(tool, self.action(tool).max(action));
            }
        }
        if let Some(first) = file.allow.first() {
            errors.push(ctx.error(
                Some(first.span()),
                Some("permissions.allow".into()),
                "a project file may not add allow rules",
            ));
        }
        if let Some(dirs) = &file.extra_dirs {
            errors.push(ctx.error(
                Some(dirs.span()),
                Some("permissions.extra_dirs".into()),
                "a project file may not set extra_dirs",
            ));
        }
        self.ask
            .extend(parse_rules(ctx, &file.ask, "permissions.ask", errors));
        self.deny
            .extend(parse_rules(ctx, &file.deny, "permissions.deny", errors));
    }
}

fn tool_keys(file: &PermissionsFile) -> [(Tool, &Option<Spanned<Action>>); 6] {
    [
        (Tool::ReadFile, &file.read_file),
        (Tool::ListFiles, &file.list_files),
        (Tool::SearchFiles, &file.search_files),
        (Tool::EditFile, &file.edit_file),
        (Tool::WriteFile, &file.write_file),
        (Tool::RunCommand, &file.run_command),
    ]
}

fn parse_rules(
    ctx: FileCtx<'_>,
    list: &[Spanned<String>],
    key: &str,
    errors: &mut Vec<ConfigError>,
) -> Vec<Rule> {
    let mut rules = Vec::new();
    for text in list {
        match Rule::parse(text.get_ref()) {
            Ok(rule) => rules.push(rule),
            Err(message) => errors.push(ctx.error(Some(text.span()), Some(key.into()), message)),
        }
    }
    rules
}

/// `~/x` becomes `$HOME/x`; other paths must be absolute.
fn expand_dir(dir: &str, home: Option<&Path>) -> Result<PathBuf, String> {
    let tilde = dir == "~" || dir.starts_with("~/");
    if tilde {
        let home = home.ok_or_else(|| format!("`{dir}`: HOME is not set"))?;
        let rest = dir.get(1..).unwrap_or("").trim_start_matches('/');
        return Ok(home.join(rest));
    }
    let path = PathBuf::from(dir);
    if path.is_absolute() {
        Ok(path)
    } else {
        Err(format!(
            "`{dir}` must be an absolute path or start with `~/`"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rule_syntax() {
        assert_eq!(
            Rule::parse("run_command(cargo test *)").unwrap(),
            Rule {
                tool: Tool::RunCommand,
                pattern: Some("cargo test *".into())
            }
        );
        assert_eq!(Rule::parse("read_file").unwrap().pattern, None);
        assert!(
            Rule::parse("run_command(x")
                .unwrap_err()
                .contains("missing `)`")
        );
        assert!(
            Rule::parse("edit_file()")
                .unwrap_err()
                .contains("empty pattern")
        );
        assert!(
            Rule::parse("rm(x)")
                .unwrap_err()
                .contains("unknown tool `rm`")
        );
    }

    #[test]
    fn rule_display_round_trip() {
        for text in ["edit_file(src/**)", "write_file"] {
            assert_eq!(Rule::parse(text).unwrap().to_string(), text);
        }
    }

    #[test]
    fn defaults() {
        let perms = Permissions::default();
        assert_eq!(perms.action(Tool::ReadFile), Action::Allow);
        assert_eq!(perms.action(Tool::RunCommand), Action::Ask);
    }

    #[test]
    fn stricter_wins() {
        assert!(Action::Deny > Action::Ask && Action::Ask > Action::Allow);
    }

    #[test]
    fn expand_dirs() {
        let home = Path::new("/home/u");
        assert_eq!(
            expand_dir("~/ws/lib", Some(home)).unwrap(),
            PathBuf::from("/home/u/ws/lib")
        );
        assert_eq!(
            expand_dir("~", Some(home)).unwrap(),
            PathBuf::from("/home/u")
        );
        assert_eq!(expand_dir("/opt", None).unwrap(), PathBuf::from("/opt"));
        assert!(expand_dir("rel", Some(home)).is_err());
        assert!(expand_dir("~/x", None).is_err());
    }
}
