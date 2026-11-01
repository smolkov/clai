//! Allow rules added in a session, and the rules proposed for "always"
//! (see `spec/tools.md`, "Permissions").

use std::path::Path;

use crate::config::{Action, Rule, Tool};

use super::check::{CommandDecision, Reason};
use super::path_pattern;
use super::shell;
use super::workspace::{Area, Resolved};

/// Allow rules the user added in this session with "always".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionRules {
    rules: Vec<Rule>,
}

impl SessionRules {
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    /// Adds allow rules; other actions are ignored.
    pub fn add(&mut self, rules: impl IntoIterator<Item = Rule>) {
        self.rules
            .extend(rules.into_iter().filter(|r| r.action == Action::Allow));
    }
}

/// One choice for "always": the rules added together, in notation.
pub type Proposal = Vec<String>;

/// Proposals for a file tool, narrowest first: the file itself, then the
/// directories it lies in, up to the workspace or extra directory.
pub fn path_proposals(tool: Tool, path: &Resolved) -> Vec<Proposal> {
    let (start, relative, stop): (&Path, bool, &Path) = match &path.area {
        Area::Workspace(rel) => (rel, true, Path::new("")),
        Area::Extra { dir, .. } => (&path.abs, false, dir),
        Area::Outside => return Vec::new(),
    };
    let rule = |tool: &str, pattern: String| vec![format!("{tool}({pattern})")];
    let mut out = Vec::new();
    let is_dir = path.abs.is_dir();
    let mut dir = if is_dir {
        Some(start)
    } else {
        out.push(rule(tool.name(), path_pattern::literal(start, relative)));
        start.parent()
    };
    let mut first = true;
    while let Some(d) = dir {
        out.push(rule(tool.name(), path_pattern::below(d, relative)));
        if first && tool.writes() {
            out.push(rule("edit", path_pattern::below(d, relative)));
        }
        first = false;
        if d == stop || d.as_os_str().is_empty() {
            break;
        }
        dir = d.parent();
    }
    out
}

/// Proposals for `run_command`: every part that asks, with all its words,
/// then with fewer words, down to the command name. Redirections that ask
/// get a `write_file` or `read_file` rule in every proposal. Empty when a
/// rule cannot make the call allowed (a part that cannot be checked).
pub fn command_proposals(decision: &CommandDecision) -> Vec<Proposal> {
    let mut asking = Vec::new();
    for part in &decision.parts {
        if part.decision.action != Action::Ask {
            continue;
        }
        if matches!(part.decision.reason, Reason::Unsafe { .. }) {
            return Vec::new();
        }
        // A part that an `ask` rule matches cannot be allowed by a rule.
        if matches!(part.decision.reason, Reason::Rule { .. }) {
            return Vec::new();
        }
        let words: Vec<String> = part.words.iter().map(|w| command_word(w)).collect();
        asking.push((words, part.assignments + 1));
    }
    let mut files = Vec::new();
    for redirect in &decision.redirects {
        if redirect.decision.action != Action::Ask {
            continue;
        }
        let Some(resolved) = &redirect.resolved else {
            return Vec::new();
        };
        if matches!(redirect.decision.reason, Reason::Rule { .. }) {
            return Vec::new();
        }
        let tool = if redirect.write {
            "write_file"
        } else {
            "read_file"
        };
        let relative = matches!(resolved.area, Area::Workspace(_));
        let start = match &resolved.area {
            Area::Workspace(rel) => rel.as_path(),
            _ => resolved.abs.as_path(),
        };
        files.push(format!(
            "{tool}({})",
            path_pattern::literal(start, relative)
        ));
    }
    if asking.is_empty() && files.is_empty() {
        return Vec::new();
    }
    let steps = asking
        .iter()
        .map(|(words, min)| words.len().saturating_sub(*min) + 1)
        .max()
        .unwrap_or(1);
    let mut out: Vec<Proposal> = Vec::new();
    for step in 0..steps {
        let mut proposal: Proposal = asking
            .iter()
            .map(|(words, min)| {
                let n = words.len().saturating_sub(step).max(*min).min(words.len());
                let kept = words.get(..n).unwrap_or(words);
                format!("run_command({})", kept.join(" "))
            })
            .collect();
        proposal.dedup();
        proposal.extend(files.iter().cloned());
        if out.last() != Some(&proposal) {
            out.push(proposal);
        }
    }
    out
}

/// A command word as a pattern word that matches only itself.
fn command_word(word: &str) -> String {
    let mut glob = String::new();
    for c in word.chars() {
        match c {
            '*' | '?' | '[' | ']' | '{' | '}' => {
                glob.push('[');
                glob.push(c);
                glob.push(']');
            }
            '\\' => glob.push_str("\\\\"),
            _ => glob.push(c),
        }
    }
    shell::quote(&glob)
}
