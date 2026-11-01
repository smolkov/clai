//! Permission checks for tool calls (see `spec/config.md`, "Permissions"):
//! the workspace boundary, rules, command splitting and session rules.

mod check;
mod command_pattern;
mod path_pattern;
mod session;
pub mod shell;
mod workspace;

#[cfg(test)]
mod tests;

pub use check::{Checker, CommandDecision, Decision, PartDecision, Reason, RedirectDecision};
pub use command_pattern::CommandPattern;
pub use path_pattern::{PathInfo, PathPattern};
pub use session::{Proposal, SessionRules, command_proposals, path_proposals};
pub use workspace::{Area, Resolved, Workspace};
