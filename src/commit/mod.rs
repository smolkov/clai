//! Commit messages: the staged (or all) changes of a repository, the project's commit
//! rules and one model request (see `spec/commit.md`). Used by `clai
//! commit`; it commits nothing.

pub mod changes;
pub mod git;
pub mod prompt;

use futures_util::StreamExt;
use tokio_util::sync::CancellationToken;

use crate::config::{Action, Model, Tool};
use crate::error::{Error, Result};
use crate::permissions::Checker;
use crate::provider::{Event, Message, Provider, ProviderError, Request};

pub use git::{Repo, Scope};

/// Output tokens for one message.
const MAX_OUTPUT_TOKENS: u64 = 1_024;

/// What `describe` sends, collected from the repository.
#[derive(Debug, Clone)]
pub struct Prepared {
    /// Changed files.
    pub files: usize,
    /// The user message.
    pub message: String,
}

/// Collects what the model gets: the changes of `scope`, the agent file
/// and the recent subjects. `checker` decides which files are protected.
/// No changes is an error.
pub fn prepare(
    repo: &Repo,
    scope: Scope,
    checker: &Checker<'_>,
    hint: Option<&str>,
) -> Result<Prepared> {
    let changed = repo.changes(scope)?;
    if changed.is_empty() {
        return Err(Error::Failed(match scope {
            Scope::Staged => "nothing staged; stage changes with git add first, or use -a".into(),
            Scope::All => "nothing to commit".into(),
        }));
    }
    let protected = |path: &str| is_protected(checker, path);
    let changes = changes::select(&changed, protected, changes::BUDGET, changes::PER_FILE);
    let agent_file = prompt::agent_file(repo.root(), protected);
    let subjects = repo.recent_subjects()?;
    let message = prompt::user_message(
        &prompt::Parts {
            agent_file: agent_file.as_ref(),
            subjects: &subjects,
            hint,
        },
        &changes,
    );
    Ok(Prepared {
        files: changed.len(),
        message,
    })
}

/// A file `read_file` may not read: its content is not sent.
fn is_protected(checker: &Checker<'_>, path: &str) -> bool {
    let resolved = checker.workspace.resolve(path);
    checker.check_path(Tool::ReadFile, &resolved).action == Action::Deny
}

/// The request for `prepared`, without tools.
pub fn request(prepared: &Prepared, model: &Model) -> Request {
    Request {
        model: model.id.clone(),
        system: prompt::SYSTEM.to_owned(),
        messages: vec![Message::user(prepared.message.clone())],
        tools: Vec::new(),
        max_output_tokens: Some(
            model
                .max_output_tokens
                .map_or(MAX_OUTPUT_TOKENS, |m| m.min(MAX_OUTPUT_TOKENS)),
        ),
        temperature: model.temperature,
    }
}

/// Sends the request and returns the answer's text.
pub async fn ask(
    provider: &dyn Provider,
    request: Request,
    cancel: CancellationToken,
) -> std::result::Result<String, ProviderError> {
    let mut stream = provider.stream(request, cancel);
    let mut text = String::new();
    while let Some(event) = stream.next().await {
        match event? {
            Event::TextDelta { text: delta } => text.push_str(&delta),
            Event::Retry {
                attempt,
                max,
                reason,
            } => tracing::info!(attempt, max, %reason, "commit: retry"),
            Event::Done { .. } => break,
            _ => {}
        }
    }
    Ok(text)
}

/// The answer as printed: without a fence around it, blank lines at its ends
/// or trailing spaces, with `trailer` after it unless it has that line.
/// `None` if nothing is left.
pub fn clean(answer: &str, trailer: Option<&str>) -> Option<String> {
    let lines: Vec<&str> = answer.lines().map(str::trim_end).collect();
    let mut lines = unfence(&lines);
    while lines.first().is_some_and(|l| l.is_empty()) {
        lines.remove(0);
    }
    while lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    if lines.is_empty() {
        return None;
    }
    let mut message = lines.join("\n");
    if let Some(trailer) = trailer.map(str::trim).filter(|t| !t.is_empty())
        && !lines.iter().any(|l| l.trim() == trailer)
    {
        message.push_str("\n\n");
        message.push_str(trailer);
    }
    message.push('\n');
    Some(message)
}

/// The lines inside a code fence that wraps the whole answer.
fn unfence<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    let content: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty())
        .map(|(i, _)| i)
        .collect();
    if let (Some(&first), Some(&last)) = (content.first(), content.last())
        && first < last
        && lines
            .get(first)
            .is_some_and(|l| l.trim_start().starts_with("```"))
        && lines.get(last).is_some_and(|l| l.trim() == "```")
    {
        return lines
            .iter()
            .skip(first + 1)
            .take(last - first - 1)
            .copied()
            .collect();
    }
    lines.to_vec()
}

/// The message for `prepared`, cleaned. Errors are for the user.
pub async fn describe(
    provider: &dyn Provider,
    model: &Model,
    prepared: &Prepared,
    trailer: Option<&str>,
    cancel: CancellationToken,
) -> Result<String> {
    let answer = ask(provider, request(prepared, model), cancel)
        .await
        .map_err(|e| Error::Failed(format!("model request failed: {e}")))?;
    clean(&answer, trailer).ok_or_else(|| Error::Failed("the model wrote no message".into()))
}

#[cfg(test)]
mod tests;
