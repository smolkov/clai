//! The checking order of `spec/config.md`: boundary, deny, ask, allow,
//! default; commands part by part.

use std::fmt;
use std::path::Path;

use serde::Serialize;

use crate::config::{Action, Pattern, Permissions, Rule, Tool};

use super::command_pattern::{CommandPattern, file_name};
use super::session::SessionRules;
use super::shell::{self, Part, Script, Word};
use super::workspace::{Resolved, Workspace};

/// Why a call is allowed, asked about or denied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Reason {
    Rule {
        /// `deny run_command(rm)`.
        rule: String,
        /// `config.toml:31`, `session`.
        source: String,
        message: Option<String>,
    },
    Default {
        tool: Tool,
    },
    Outside {
        path: String,
    },
    ReadOnly {
        dir: String,
    },
    /// The command could not be split safely.
    Unsafe {
        why: String,
    },
    /// The part runs nothing (only redirections), or only its inner
    /// commands count (`sh -c`).
    Nothing,
    Inner,
}

impl fmt::Display for Reason {
    /// For questions: `asks: <reason>`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Reason::Rule { rule, source, .. } => write!(f, "rule {rule} ({source})"),
            Reason::Default { tool } => write!(f, "default for {tool}"),
            Reason::Outside { path } => write!(f, "{path} is outside the workspace"),
            Reason::ReadOnly { dir } => write!(f, "{dir} is read-only"),
            Reason::Unsafe { why } => write!(f, "the command cannot be checked: {why}"),
            Reason::Nothing => f.write_str("nothing to run"),
            Reason::Inner => f.write_str("the commands inside are checked"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Decision {
    pub action: Action,
    pub reason: Reason,
}

impl Decision {
    fn new(action: Action, reason: Reason) -> Decision {
        Decision { action, reason }
    }

    fn from_rule(rule: &Rule) -> Decision {
        Decision::new(
            rule.action,
            Reason::Rule {
                rule: rule.to_string(),
                source: rule.source.to_string(),
                message: rule.message.clone(),
            },
        )
    }

    /// The rule's message, if a rule decided and has one.
    pub fn message(&self) -> Option<&str> {
        match &self.reason {
            Reason::Rule { message, .. } => message.as_deref(),
            _ => None,
        }
    }

    /// The error result for the model when the call is denied (see
    /// `spec/tools.md`, "Denied and cancelled calls").
    pub fn denial(&self) -> String {
        match &self.reason {
            Reason::Rule { rule, message, .. } => match message {
                Some(m) => format!("denied by permission rule: {rule}: {m}"),
                None => format!("denied by permission rule: {rule}"),
            },
            Reason::Default { tool } => format!("denied: {tool} is not allowed"),
            other => format!("denied: {other}"),
        }
    }
}

/// The result for one part of a command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PartDecision {
    pub text: String,
    #[serde(skip)]
    pub words: Vec<String>,
    #[serde(skip)]
    pub assignments: usize,
    pub decision: Decision,
}

/// A file a command reads or writes through a redirection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RedirectDecision {
    /// As written.
    pub target: String,
    /// Resolved, for display; empty if only known when the command runs.
    pub path: String,
    pub write: bool,
    pub decision: Decision,
    #[serde(skip)]
    pub resolved: Option<Resolved>,
}

/// The result for a whole command line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CommandDecision {
    /// The strictest result of the parts and redirections.
    pub decision: Decision,
    pub parts: Vec<PartDecision>,
    pub redirects: Vec<RedirectDecision>,
}

impl CommandDecision {
    /// The error result for the model, naming the denied part.
    pub fn denial(&self) -> String {
        let text = self.decision.denial();
        let part = self
            .parts
            .iter()
            .find(|p| p.decision == self.decision && self.parts.len() > 1);
        match part {
            Some(part) => format!("{text}, for the part \"{}\"", part.text),
            None => text,
        }
    }
}

/// Checks tool calls against the permissions of a session.
#[derive(Clone, Copy)]
pub struct Checker<'a> {
    pub permissions: &'a Permissions,
    pub session: &'a SessionRules,
    pub workspace: &'a Workspace,
}

/// Files a command may always redirect to or from.
const DEVICES: [&str; 5] = [
    "/dev/null",
    "/dev/stdout",
    "/dev/stderr",
    "/dev/stdin",
    "/dev/tty",
];
/// How deep `sh -c "sh -c …"` is followed.
const MAX_SHELLS: usize = 5;

impl Checker<'_> {
    fn rules(&self) -> impl Iterator<Item = &Rule> {
        self.permissions.rules.iter().chain(self.session.rules())
    }

    fn default(&self, tool: Tool) -> Decision {
        Decision::new(self.permissions.action(tool), Reason::Default { tool })
    }

    /// Checks a file tool on a resolved path.
    pub fn check_path(&self, tool: Tool, path: &Resolved) -> Decision {
        if path.is_outside() {
            return Decision::new(
                Action::Deny,
                Reason::Outside {
                    path: path.abs.display().to_string(),
                },
            );
        }
        if tool.writes()
            && let Some(dir) = path.read_only_dir()
        {
            return Decision::new(
                Action::Deny,
                Reason::ReadOnly {
                    dir: self.workspace.tilde(dir),
                },
            );
        }
        self.rules_for_path(tool, path)
    }

    /// The rules only, without the boundary: for entries found while
    /// listing or searching, which are inside it already.
    pub fn rules_for_path(&self, tool: Tool, path: &Resolved) -> Decision {
        let info = path.info();
        let matches = |p: &Pattern| matches!(p, Pattern::Path(p) if p.matches(info));
        let applies = |rule: &&Rule| {
            rule.tools.contains(&tool)
                && (rule.patterns.is_empty() || rule.patterns.iter().any(matches))
                && !rule.except.iter().any(matches)
        };
        self.first_by_action(applies)
            .unwrap_or_else(|| self.default(tool))
    }

    /// The first matching deny rule, else ask, else allow.
    fn first_by_action(&self, applies: impl Fn(&&Rule) -> bool) -> Option<Decision> {
        [Action::Deny, Action::Ask, Action::Allow]
            .into_iter()
            .find_map(|action| {
                self.rules()
                    .filter(|r| r.action == action)
                    .find(&applies)
                    .map(Decision::from_rule)
            })
    }

    /// Checks a command line of `run_command`.
    pub fn check_command(&self, command: &str) -> CommandDecision {
        let mut parts = Vec::new();
        let mut redirects = Vec::new();
        self.check_line(command, 0, &mut parts, &mut redirects);
        let decision = parts
            .iter()
            .map(|p: &PartDecision| &p.decision)
            .chain(redirects.iter().map(|r: &RedirectDecision| &r.decision))
            .fold(None::<&Decision>, |best, d| match best {
                Some(b) if b.action >= d.action => Some(b),
                _ => Some(d),
            })
            .cloned()
            .unwrap_or(Decision::new(Action::Allow, Reason::Nothing));
        CommandDecision {
            decision,
            parts,
            redirects,
        }
    }

    fn check_line(
        &self,
        line: &str,
        depth: usize,
        parts: &mut Vec<PartDecision>,
        redirects: &mut Vec<RedirectDecision>,
    ) {
        let script = match shell::parse(line) {
            Ok(script) => script,
            Err(why) => {
                parts.push(self.unparsed(line, why));
                return;
            }
        };
        // Index in `parts` of each part of the script.
        let mut indices = Vec::with_capacity(script.parts.len());
        for part in &script.parts {
            indices.push(parts.len());
            if let Some(inner) = shell_string(part) {
                let decision = self.strict_rules(part).unwrap_or_else(|| match inner {
                    Some(_) if depth >= MAX_SHELLS => unsafe_reason("shells nested too deeply"),
                    Some(_) => Decision::new(Action::Allow, Reason::Inner),
                    None => unsafe_reason("the command for -c is only known when it runs"),
                });
                parts.push(part_decision(part, decision));
                self.check_redirects(part, redirects);
                if let Some(inner) = inner
                    && depth < MAX_SHELLS
                {
                    self.check_line(&inner, depth + 1, parts, redirects);
                }
                continue;
            }
            parts.push(part_decision(part, self.check_part(part)));
            self.check_redirects(part, redirects);
        }
        self.check_pipelines(&script, &indices, parts);
    }

    /// A command line the parser cannot read: asks, unless a deny rule
    /// matches the start of its text.
    fn unparsed(&self, line: &str, why: String) -> PartDecision {
        let words: Vec<Word> = line
            .split_whitespace()
            .map(|w| Word {
                text: w.to_owned(),
                dynamic: false,
            })
            .collect();
        let part = Part {
            text: line.trim().to_owned(),
            words,
            assignments: 0,
            redirects: Vec::new(),
        };
        let deny = self
            .strict_rules(&part)
            .filter(|d| d.action == Action::Deny);
        part_decision(&part, deny.unwrap_or_else(|| unsafe_reason(&why)))
    }

    fn check_part(&self, part: &Part) -> Decision {
        let Some(first) = part.command().first() else {
            return Decision::new(Action::Allow, Reason::Nothing);
        };
        if let Some(decision) = self.strict_rules(part) {
            return decision;
        }
        if first.dynamic {
            return unsafe_reason(&format!(
                "the command name `{}` is only known when it runs",
                display_word(first)
            ));
        }
        let allowed = self
            .rules()
            .filter(|r| r.action == Action::Allow)
            .find(|rule| command_rule_applies(rule, |p| p.matches(part)));
        match allowed {
            Some(rule) => Decision::from_rule(rule),
            None => self.default(Tool::RunCommand),
        }
    }

    /// The first `deny`, else `ask` rule that matches the part.
    fn strict_rules(&self, part: &Part) -> Option<Decision> {
        [Action::Deny, Action::Ask].into_iter().find_map(|action| {
            self.rules()
                .filter(|r| r.action == action)
                .find(|rule| command_rule_applies(rule, |p| p.matches_strict(part)))
                .map(Decision::from_rule)
        })
    }

    /// Pipeline patterns (`curl ** | sh`) make the first part of the
    /// pipeline stricter.
    fn check_pipelines(&self, script: &Script, indices: &[usize], parts: &mut [PartDecision]) {
        for pipeline in &script.pipelines {
            let stages: Vec<&Part> = pipeline
                .iter()
                .filter_map(|&i| script.parts.get(i))
                .collect();
            let hit = [Action::Deny, Action::Ask].into_iter().find_map(|action| {
                self.rules()
                    .filter(|r| r.action == action && r.tools.contains(&Tool::RunCommand))
                    .find(|rule| {
                        rule.patterns.iter().any(|p| match p {
                            Pattern::Command(c) => c.matches_pipeline(&stages),
                            Pattern::Path(_) => false,
                        })
                    })
            });
            let Some(rule) = hit else { continue };
            let Some(&index) = pipeline.first().and_then(|&i| indices.get(i)) else {
                continue;
            };
            if let Some(part) = parts.get_mut(index)
                && part.decision.action < rule.action
            {
                part.decision = Decision::from_rule(rule);
            }
        }
    }

    fn check_redirects(&self, part: &Part, out: &mut Vec<RedirectDecision>) {
        for redirect in &part.redirects {
            let target = &redirect.target;
            if target.dynamic {
                out.push(RedirectDecision {
                    target: display_word(target),
                    path: String::new(),
                    write: redirect.write,
                    decision: unsafe_reason("the file of a redirection is only known when it runs"),
                    resolved: None,
                });
                continue;
            }
            if DEVICES.contains(&target.text.as_str()) {
                continue;
            }
            let resolved = self.resolve_shell_path(&target.text);
            let tool = if redirect.write {
                Tool::WriteFile
            } else {
                Tool::ReadFile
            };
            out.push(RedirectDecision {
                target: target.text.clone(),
                path: resolved.display(),
                write: redirect.write,
                decision: self.check_path(tool, &resolved),
                resolved: Some(resolved),
            });
        }
    }

    /// The shell expands `~/` in redirections.
    fn resolve_shell_path(&self, text: &str) -> Resolved {
        match (text.strip_prefix("~/"), self.workspace.home()) {
            (Some(rest), Some(home)) => self.workspace.resolve(&home.join(rest).to_string_lossy()),
            _ if text == "~" => self.workspace.resolve(
                &self
                    .workspace
                    .home()
                    .unwrap_or(Path::new("~"))
                    .to_string_lossy(),
            ),
            _ => self.workspace.resolve(text),
        }
    }
}

fn command_rule_applies(rule: &Rule, matches: impl Fn(&CommandPattern) -> bool) -> bool {
    let single = |p: &Pattern| match p {
        Pattern::Command(c) if !c.is_pipeline() => matches(c),
        _ => false,
    };
    rule.tools.contains(&Tool::RunCommand)
        && (rule.patterns.is_empty() || rule.patterns.iter().any(single))
        && !rule.except.iter().any(single)
}

fn part_decision(part: &Part, decision: Decision) -> PartDecision {
    PartDecision {
        text: part.text.clone(),
        words: part.words.iter().map(|w| w.text.clone()).collect(),
        assignments: part.assignments,
        decision,
    }
}

fn unsafe_reason(why: &str) -> Decision {
    Decision::new(
        Action::Ask,
        Reason::Unsafe {
            why: why.to_owned(),
        },
    )
}

fn display_word(word: &Word) -> String {
    if word.dynamic {
        format!("{}…", word.text)
    } else {
        word.text.clone()
    }
}

/// For `sh -c STRING` (also `bash`, `zsh`, …, and flags like `-ec`):
/// `Some(Some(string))`, or `Some(None)` if the string is only known when
/// the command runs. `None` if the part is not such a shell call.
fn shell_string(part: &Part) -> Option<Option<String>> {
    const SHELLS: [&str; 5] = ["sh", "bash", "zsh", "dash", "ksh"];
    let command = part.command();
    let (first, args) = command.split_first()?;
    if first.dynamic || !SHELLS.contains(&file_name(&first.text)) {
        return None;
    }
    let mut args = args.iter();
    for arg in args.by_ref() {
        let text = arg.text.as_str();
        if text == "--" || !text.starts_with('-') {
            return None;
        }
        if text.len() > 1 && !text.starts_with("--") && text.contains('c') {
            break;
        }
    }
    let string = args.next()?;
    Some((!string.dynamic).then(|| string.text.clone()))
}
