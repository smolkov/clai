//! `[permissions]`: tool defaults, rules and extra directories (see
//! `spec/config.md`, "Permissions").
//!
//! Loading and merging live here; matching a call against the rules is in
//! [`crate::permissions`].

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use toml::Spanned;

use super::errors::{ConfigError, FileCtx};
use super::file::{DetailedRuleFile, PermissionsFile, RuleTable};
use crate::permissions::{CommandPattern, PathPattern};

/// What happens with a tool call. Ordered from loose to strict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Action {
    Allow,
    Ask,
    Deny,
}

impl Action {
    pub const ALL: [Action; 3] = [Action::Allow, Action::Ask, Action::Deny];

    pub fn name(self) -> &'static str {
        match self {
            Action::Allow => "allow",
            Action::Ask => "ask",
            Action::Deny => "deny",
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
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

    /// Tools whose patterns are paths.
    pub fn is_file_tool(self) -> bool {
        self != Tool::RunCommand
    }

    /// Tools that change files.
    pub fn writes(self) -> bool {
        matches!(self, Tool::EditFile | Tool::WriteFile)
    }
}

impl Serialize for Tool {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.name())
    }
}

impl<'de> Deserialize<'de> for Tool {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Tool, D::Error> {
        let name = String::deserialize(d)?;
        Tool::from_name(&name)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown tool `{name}`")))
    }
}

impl fmt::Display for Tool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Tool groups usable in rules.
pub const GROUPS: [(&str, &[Tool]); 2] = [
    (
        "read",
        &[Tool::ReadFile, Tool::ListFiles, Tool::SearchFiles],
    ),
    ("edit", &[Tool::EditFile, Tool::WriteFile]),
];

/// The tools a rule names: a tool or a group.
pub fn tools_named(name: &str) -> Option<Vec<Tool>> {
    if let Some(tool) = Tool::from_name(name) {
        return Some(vec![tool]);
    }
    GROUPS
        .iter()
        .find(|(group, _)| *group == name)
        .map(|(_, tools)| tools.to_vec())
}

fn unknown_tool(name: &str) -> String {
    format!(
        "unknown tool or group `{name}`; tools are {}; groups are read, edit",
        Tool::ALL.map(Tool::name).join(", ")
    )
}

/// Where a rule comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    File {
        path: PathBuf,
        line: Option<usize>,
    },
    Session,
    /// The rules for secret files that `clai` ships (see [`builtin_rules`]).
    BuiltIn,
}

impl fmt::Display for Source {
    /// `config.toml:31`, `.clai/config.toml:3`, `session`, `built-in`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Source::Session => f.write_str("session"),
            Source::BuiltIn => f.write_str("built-in"),
            Source::File { path, line } => {
                let name = path.file_name().map(|n| n.to_string_lossy());
                let in_clai = path
                    .parent()
                    .and_then(Path::file_name)
                    .is_some_and(|d| d == ".clai");
                match (name, in_clai) {
                    (Some(name), true) => write!(f, ".clai/{name}")?,
                    (Some(name), false) => f.write_str(&name)?,
                    (None, _) => write!(f, "{}", path.display())?,
                }
                match line {
                    Some(line) => write!(f, ":{line}"),
                    None => Ok(()),
                }
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Path(PathPattern),
    Command(CommandPattern),
}

impl Pattern {
    pub fn text(&self) -> &str {
        match self {
            Pattern::Path(p) => p.text(),
            Pattern::Command(p) => p.text(),
        }
    }

    fn parse(text: &str, commands: bool, home: Option<&Path>) -> Result<Pattern, String> {
        if commands {
            CommandPattern::parse(text).map(Pattern::Command)
        } else {
            PathPattern::parse(text, home).map(Pattern::Path)
        }
    }
}

impl PartialEq for Pattern {
    fn eq(&self, other: &Pattern) -> bool {
        matches!(
            (self, other),
            (Pattern::Path(_), Pattern::Path(_)) | (Pattern::Command(_), Pattern::Command(_))
        ) && self.text() == other.text()
    }
}

/// One rule, from an action table, a `[[permissions.rules]]` table or the
/// session.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub action: Action,
    pub tools: Vec<Tool>,
    /// The tools as written: `read`, `run_command`, `read_file, edit`.
    pub label: String,
    /// Empty: every call of the tools.
    pub patterns: Vec<Pattern>,
    pub except: Vec<Pattern>,
    pub message: Option<String>,
    pub source: Source,
}

impl Rule {
    /// Parses `TOOL` or `TOOL(PATTERN)` as an allow rule of the session.
    pub fn parse_session(text: &str, home: Option<&Path>) -> Result<Rule, String> {
        let text = text.trim();
        let text = text.strip_prefix("allow ").unwrap_or(text).trim();
        let (name, pattern) = match text.split_once('(') {
            None => (text, None),
            Some((name, rest)) => {
                let Some(pattern) = rest.strip_suffix(')') else {
                    return Err(format!("rule `{text}`: missing `)` at the end"));
                };
                (name.trim(), Some(pattern))
            }
        };
        let tools =
            tools_named(name).ok_or_else(|| format!("rule `{text}`: {}", unknown_tool(name)))?;
        let commands = tools == [Tool::RunCommand];
        let patterns = match pattern {
            Some(p) => {
                let pattern =
                    Pattern::parse(p, commands, home).map_err(|e| format!("rule `{text}`: {e}"))?;
                if let Pattern::Command(c) = &pattern
                    && c.is_pipeline()
                {
                    return Err(format!(
                        "rule `{text}`: `|` is only allowed in deny and ask rules"
                    ));
                }
                vec![pattern]
            }
            None => Vec::new(),
        };
        Ok(Rule {
            action: Action::Allow,
            tools,
            label: name.to_owned(),
            patterns,
            except: Vec::new(),
            message: None,
            source: Source::Session,
        })
    }

    /// The rule without its action: `run_command(cargo test)`.
    pub fn notation(&self) -> String {
        if self.patterns.is_empty() {
            return self.label.clone();
        }
        let patterns: Vec<&str> = self.patterns.iter().map(Pattern::text).collect();
        format!("{}({})", self.label, patterns.join(", "))
    }
}

impl fmt::Display for Rule {
    /// `deny run_command(rm)`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.action, self.notation())
    }
}

/// Access to an extra directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtraDir {
    /// Absolute, with `~` expanded; not resolved yet.
    pub path: PathBuf,
    pub access: Access,
}

/// Permissions in effect after merging user and project files.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Permissions {
    /// Tool keys set in a file; other tools use their default action.
    tools: Vec<(Tool, Action)>,
    /// The rules of the user and project files, in order.
    pub rules: Vec<Rule>,
    /// The built-in rules, after `builtin_except`; checked with the others.
    pub builtin: Vec<Rule>,
    pub extra_dirs: Vec<ExtraDir>,
}

impl Permissions {
    /// Action for a tool when no rule matches.
    pub fn action(&self, tool: Tool) -> Action {
        self.tools
            .iter()
            .find(|(t, _)| *t == tool)
            .map_or(tool.default_action(), |(_, a)| *a)
    }

    pub fn set(&mut self, tool: Tool, action: Action) {
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
            perms.builtin = builtin_rules(Vec::new());
            return perms;
        };
        for (tool, value) in tool_keys(&file) {
            if let Some(value) = value {
                perms.set(tool, *value.get_ref());
            }
        }
        let loader = Loader { ctx, home };
        for (action, table) in action_tables(&file) {
            perms.rules.extend(loader.table(action, table, errors));
        }
        for rule in &file.rules {
            perms.rules.extend(loader.detailed(rule, errors));
        }
        if let Some(dirs) = &file.extra_dirs {
            for dir in dirs.get_ref() {
                match extra_dir(dir.get_ref(), home) {
                    Ok(dir) => perms.extra_dirs.push(dir),
                    Err(message) => errors.push(ctx.error(
                        Some(dir.span()),
                        Some("permissions.extra_dirs".into()),
                        message,
                    )),
                }
            }
        }
        let mut except = Vec::new();
        for text in file.builtin_except.iter().flat_map(|l| l.get_ref()) {
            match PathPattern::parse(text.get_ref(), home) {
                Ok(p) => except.push(Pattern::Path(p)),
                Err(message) => errors.push(ctx.error(
                    Some(text.span()),
                    Some("permissions.builtin_except".into()),
                    message,
                )),
            }
        }
        perms.builtin = builtin_rules(except);
        perms
    }

    /// Adds a project file, which may only make permissions stricter.
    pub(crate) fn apply_project(
        &mut self,
        ctx: FileCtx<'_>,
        file: PermissionsFile,
        home: Option<&Path>,
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
        if let Some(allow) = &file.allow {
            errors.push(ctx.error(
                Some(allow.span()),
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
        if let Some(except) = &file.builtin_except {
            errors.push(ctx.error(
                Some(except.span()),
                Some("permissions.builtin_except".into()),
                "a project file may not set builtin_except",
            ));
        }
        let loader = Loader { ctx, home };
        for (action, table) in action_tables(&file) {
            if action != Action::Allow {
                self.rules.extend(loader.table(action, table, errors));
            }
        }
        for rule in &file.rules {
            if rule.get_ref().action.as_ref().map(|a| *a.get_ref()) == Some(Action::Allow) {
                errors.push(ctx.error(
                    Some(rule.span()),
                    Some("permissions.rules".into()),
                    "a project file may not add allow rules",
                ));
                continue;
            }
            self.rules.extend(loader.detailed(rule, errors));
        }
    }
}

/// The message of the built-in rules, for the model.
const BUILTIN_MESSAGE: &str = "this file usually holds secrets and is not shown to the model. \
     Ask the user for the value you need; the user can add the path to \
     permissions.builtin_except.";

/// The built-in `deny` rules for well-known secret files (see
/// `spec/config.md`, "Built-in rules"), with `extra` added to the
/// exceptions of each.
pub fn builtin_rules(extra: Vec<Pattern>) -> Vec<Rule> {
    const RULES: [(&[&str], &[&str]); 3] = [
        (
            &[".env", ".env.*"],
            &[".env.example", ".env.sample", ".env.template", ".env.dist"],
        ),
        (
            &[
                "*.pem",
                "*.key",
                "*.p12",
                "*.pfx",
                "id_rsa",
                "id_ecdsa",
                "id_ed25519",
            ],
            &[],
        ),
        (&[".netrc", ".pgpass"], &[]),
    ];
    let parse = |texts: &[&str]| -> Vec<Pattern> {
        texts
            .iter()
            .filter_map(|t| PathPattern::parse(t, None).ok().map(Pattern::Path))
            .collect()
    };
    RULES
        .iter()
        .map(|(patterns, except)| {
            let mut except = parse(except);
            except.extend(extra.iter().cloned());
            Rule {
                action: Action::Deny,
                tools: tools_named("read").unwrap_or_default(),
                label: "read".to_owned(),
                patterns: parse(patterns),
                except,
                message: Some(BUILTIN_MESSAGE.to_owned()),
                source: Source::BuiltIn,
            }
        })
        .collect()
}

/// `[permissions]` from TOML text, for tests of the checks.
#[cfg(test)]
pub(crate) fn parse_for_test(text: &str, home: Option<&Path>) -> Permissions {
    #[derive(Deserialize)]
    struct Wrapper {
        permissions: Option<PermissionsFile>,
    }
    let ctx = FileCtx {
        path: Path::new("config.toml"),
        text,
    };
    let wrapper: Wrapper = ctx.parse().unwrap_or_else(|e| panic!("{e}"));
    let mut errors = Vec::new();
    let perms = Permissions::from_user(ctx, wrapper.permissions, home, &mut errors);
    assert!(errors.is_empty(), "{errors:?}");
    perms
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

fn action_tables(file: &PermissionsFile) -> impl Iterator<Item = (Action, &Spanned<RuleTable>)> {
    [
        (Action::Allow, &file.allow),
        (Action::Ask, &file.ask),
        (Action::Deny, &file.deny),
    ]
    .into_iter()
    .filter_map(|(action, table)| table.as_ref().map(|t| (action, t)))
}

struct Loader<'a> {
    ctx: FileCtx<'a>,
    home: Option<&'a Path>,
}

impl Loader<'_> {
    fn source(&self, span: std::ops::Range<usize>) -> Source {
        Source::File {
            path: self.ctx.path.to_owned(),
            line: self.ctx.line_of(span.start),
        }
    }

    fn error(&self, span: std::ops::Range<usize>, key: String, message: String) -> ConfigError {
        self.ctx.error(Some(span), Some(key), message)
    }

    /// `[permissions.ACTION]`: one rule per pattern.
    fn table(
        &self,
        action: Action,
        table: &Spanned<RuleTable>,
        errors: &mut Vec<ConfigError>,
    ) -> Vec<Rule> {
        let mut rules = Vec::new();
        for (name, patterns) in table.get_ref() {
            let key = format!("permissions.{action}.{name}");
            let Some(tools) = tools_named(name) else {
                errors.push(self.error(patterns.span(), key, unknown_tool(name)));
                continue;
            };
            let commands = tools == [Tool::RunCommand];
            for text in patterns.get_ref() {
                match self.pattern(text.get_ref(), commands, action) {
                    Ok(pattern) => rules.push(Rule {
                        action,
                        tools: tools.clone(),
                        label: name.clone(),
                        patterns: vec![pattern],
                        except: Vec::new(),
                        message: None,
                        source: self.source(text.span()),
                    }),
                    Err(message) => errors.push(self.error(text.span(), key.clone(), message)),
                }
            }
        }
        rules
    }

    fn pattern(&self, text: &str, commands: bool, action: Action) -> Result<Pattern, String> {
        let pattern = Pattern::parse(text, commands, self.home)?;
        if action == Action::Allow
            && let Pattern::Command(c) = &pattern
            && c.is_pipeline()
        {
            return Err(format!(
                "pattern `{text}`: `|` is only allowed in deny and ask rules"
            ));
        }
        Ok(pattern)
    }

    /// One `[[permissions.rules]]` table.
    fn detailed(
        &self,
        spanned: &Spanned<DetailedRuleFile>,
        errors: &mut Vec<ConfigError>,
    ) -> Option<Rule> {
        let span = spanned.span();
        let rule = spanned.get_ref();
        let key = "permissions.rules".to_owned();
        let before = errors.len();
        let action = rule.action.as_ref().map(|a| *a.get_ref());
        if action.is_none() {
            errors.push(self.error(span.clone(), key.clone(), "missing action".into()));
        }
        let mut tools: Vec<Tool> = Vec::new();
        let mut names: Vec<&str> = Vec::new();
        match &rule.tools {
            Some(list) if !list.get_ref().is_empty() => {
                for name in list.get_ref() {
                    match tools_named(name.get_ref()) {
                        Some(named) => {
                            names.push(name.get_ref());
                            for tool in named {
                                if !tools.contains(&tool) {
                                    tools.push(tool);
                                }
                            }
                        }
                        None => errors.push(self.error(
                            name.span(),
                            key.clone(),
                            unknown_tool(name.get_ref()),
                        )),
                    }
                }
            }
            _ => errors.push(self.error(span.clone(), key.clone(), "missing tools".into())),
        }
        let commands = tools.contains(&Tool::RunCommand);
        if commands && tools.len() > 1 {
            errors.push(self.error(
                span.clone(),
                key.clone(),
                "tools mixes file tools and run_command; use one rule for each".into(),
            ));
        }
        if let Some(paths) = &rule.paths
            && commands
        {
            errors.push(self.error(
                paths.span(),
                key.clone(),
                "`paths` is for file tools; use `commands` for run_command".into(),
            ));
        }
        if let Some(list) = &rule.commands
            && !commands
            && !tools.is_empty()
        {
            errors.push(self.error(
                list.span(),
                key.clone(),
                "`commands` is for run_command; use `paths` for file tools".into(),
            ));
        }
        if let (Some(message), Some(Action::Allow)) = (&rule.message, action) {
            errors.push(self.error(
                message.span(),
                key.clone(),
                "`message` is not allowed with action = \"allow\"".into(),
            ));
        }
        let given = if commands {
            &rule.commands
        } else {
            &rule.paths
        };
        let mut patterns = Vec::new();
        let mut except = Vec::new();
        let lists = given
            .iter()
            .flat_map(|l| l.get_ref().iter().map(|p| (p, false)))
            .chain(rule.except.iter().map(|p| (p, true)));
        for (text, is_except) in lists {
            let checked_action = if is_except {
                Action::Deny
            } else {
                action.unwrap_or(Action::Deny)
            };
            match self.pattern(text.get_ref(), commands, checked_action) {
                Ok(p) if is_except => except.push(p),
                Ok(p) => patterns.push(p),
                Err(message) => errors.push(self.error(text.span(), key.clone(), message)),
            }
        }
        if errors.len() > before {
            return None;
        }
        Some(Rule {
            action: action?,
            tools,
            label: names.join(", "),
            patterns,
            except,
            message: rule.message.as_ref().map(|m| m.get_ref().clone()),
            source: self.source(span),
        })
    }
}

/// An `extra_dirs` entry: a path, or `{ path = "...", access = "read" }`.
fn extra_dir(value: &toml::Value, home: Option<&Path>) -> Result<ExtraDir, String> {
    match value {
        toml::Value::String(path) => Ok(ExtraDir {
            path: expand_dir(path, home)?,
            access: Access::Write,
        }),
        toml::Value::Table(table) => {
            let mut path = None;
            let mut access = Access::Write;
            for (key, value) in table {
                match (key.as_str(), value) {
                    ("path", toml::Value::String(p)) => path = Some(expand_dir(p, home)?),
                    ("access", toml::Value::String(a)) => {
                        access = match a.as_str() {
                            "read" => Access::Read,
                            "write" => Access::Write,
                            other => {
                                return Err(format!("unknown access `{other}`; use read or write"));
                            }
                        }
                    }
                    ("path" | "access", _) => return Err(format!("`{key}` must be a string")),
                    (other, _) => {
                        return Err(format!("unknown key `{other}`; keys are path, access"));
                    }
                }
            }
            let path = path.ok_or("missing path")?;
            Ok(ExtraDir { path, access })
        }
        _ => Err("an entry must be a path or a table with path and access".into()),
    }
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
    fn session_rule_syntax() {
        let rule = Rule::parse_session("run_command(cargo test)", None).unwrap();
        assert_eq!(rule.tools, [Tool::RunCommand]);
        assert_eq!(rule.to_string(), "allow run_command(cargo test)");
        assert_eq!(rule.source, Source::Session);
        let rule = Rule::parse_session("allow edit(src/**)", None).unwrap();
        assert_eq!(rule.tools, [Tool::EditFile, Tool::WriteFile]);
        assert_eq!(rule.notation(), "edit(src/**)");
        assert_eq!(Rule::parse_session("read_file", None).unwrap().patterns, []);
        let e = Rule::parse_session("run_command(x", None).unwrap_err();
        assert!(e.contains("missing `)`"), "{e}");
        let e = Rule::parse_session("rm(x)", None).unwrap_err();
        assert!(e.contains("unknown tool or group `rm`"), "{e}");
        let e = Rule::parse_session("run_command(curl ** | sh)", None).unwrap_err();
        assert!(e.contains("only allowed in deny and ask"), "{e}");
        assert!(Rule::parse_session("edit_file()", None).is_err());
    }

    #[test]
    fn source_display() {
        let user = Source::File {
            path: PathBuf::from("/home/u/.config/clai/config.toml"),
            line: Some(31),
        };
        assert_eq!(user.to_string(), "config.toml:31");
        let project = Source::File {
            path: PathBuf::from("/ws/.clai/config.toml"),
            line: Some(3),
        };
        assert_eq!(project.to_string(), ".clai/config.toml:3");
        assert_eq!(Source::Session.to_string(), "session");
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
    fn groups() {
        assert_eq!(
            tools_named("edit").unwrap(),
            [Tool::EditFile, Tool::WriteFile]
        );
        assert_eq!(tools_named("read").unwrap().len(), 3);
        assert_eq!(tools_named("run_command").unwrap(), [Tool::RunCommand]);
        assert!(tools_named("bash").is_none());
    }

    #[test]
    fn extra_dir_entries() {
        let home = Some(Path::new("/home/u"));
        let dir = extra_dir(&toml::Value::String("~/ws/lib".into()), home).unwrap();
        assert_eq!(
            dir,
            ExtraDir {
                path: PathBuf::from("/home/u/ws/lib"),
                access: Access::Write
            }
        );
        let table: toml::Value = toml::from_str("path = \"/opt\"\naccess = \"read\"").unwrap();
        assert_eq!(extra_dir(&table, home).unwrap().access, Access::Read);
        let bad: toml::Value = toml::from_str("path = \"/opt\"\naccess = \"all\"").unwrap();
        assert!(
            extra_dir(&bad, home)
                .unwrap_err()
                .contains("unknown access")
        );
        let bad: toml::Value = toml::from_str("access = \"read\"").unwrap();
        assert_eq!(extra_dir(&bad, home).unwrap_err(), "missing path");
        assert!(extra_dir(&toml::Value::Integer(1), home).is_err());
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
