//! What several commands share, and the TUI with them: the request for a new
//! session and its creation, the workspace check, and text given as words or
//! `-` for standard input.

use std::io::Read;
use std::path::{Path, PathBuf};

use crate::client::{Client, unexpected};
use crate::config::MODEL_ENV;
use crate::error::{Error, Result};
use crate::output::note;
use crate::protocol::{Payload, Request, SessionInfo};

/// The `new` request: `--model` or else this client's `CLAI_MODEL`, which
/// the server checks; `DIR` or the current directory.
pub fn request(name: Option<String>, model: Option<String>, dir: Option<&Path>) -> Result<Request> {
    let model = model.or_else(|| {
        std::env::var(MODEL_ENV)
            .ok()
            .filter(|v| !v.trim().is_empty())
    });
    Ok(Request::New {
        workspace: workspace(dir)?,
        name,
        model,
    })
}

/// Creates a session; a note on standard error names other sessions in the
/// same workspace.
pub async fn create(client: &mut Client, request: Request) -> Result<Box<SessionInfo>> {
    let session = match client.request(request).await? {
        Payload::Session { session } => session,
        other => return Err(unexpected(&other)),
    };
    let others: Vec<String> = client
        .list()
        .await?
        .into_iter()
        .filter(|s| s.path == session.path && s.id != session.id)
        .map(|s| s.name)
        .collect();
    if !others.is_empty() {
        note(&format!(
            "note: {} also {} in {}; sessions may change the same files",
            others.join(", "),
            if others.len() == 1 { "works" } else { "work" },
            session.workspace
        ));
    }
    Ok(session)
}

/// `DIR` or the current directory, absolute; it must be a directory.
pub fn workspace(dir: Option<&Path>) -> Result<PathBuf> {
    let dir = match dir {
        Some(dir) => dir.to_path_buf(),
        None => std::env::current_dir().map_err(Error::io("current directory"))?,
    };
    if !dir.is_dir() {
        return Err(Error::Usage(format!("{}: not a directory", dir.display())));
    }
    std::path::absolute(&dir).map_err(Error::io(dir.display().to_string()))
}

/// Text from the command line: the words joined, or standard input for `-`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    /// Trimmed, never empty.
    pub text: String,
    /// It came from standard input, which then is not a terminal to ask on.
    pub from_stdin: bool,
}

/// `words` as one text, or standard input when they are `-`; an empty text
/// is a usage error: `the {what} is empty`.
pub fn input_text(words: &[String], what: &str) -> Result<Input> {
    input_from(words, what, || {
        let mut text = String::new();
        std::io::stdin()
            .read_to_string(&mut text)
            .map_err(Error::io("standard input"))?;
        Ok(text)
    })
}

fn input_from(
    words: &[String],
    what: &str,
    stdin: impl FnOnce() -> Result<String>,
) -> Result<Input> {
    let from_stdin = words == ["-"];
    let text = if from_stdin {
        stdin()?
    } else {
        words.join(" ")
    };
    let text = text.trim().to_owned();
    if text.is_empty() {
        return Err(Error::Usage(format!("the {what} is empty")));
    }
    Ok(Input { text, from_stdin })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words(list: &[&str]) -> Vec<String> {
        list.iter().map(|w| (*w).to_owned()).collect()
    }

    fn never() -> Result<String> {
        panic!("standard input read")
    }

    #[test]
    fn words_are_joined() {
        let input = input_from(&words(&["fix", " the", "build "]), "task", never).unwrap();
        assert_eq!(input.text, "fix  the build");
        assert!(!input.from_stdin);
    }

    #[test]
    fn a_dash_reads_standard_input() {
        let input = input_from(&words(&["-"]), "task", || Ok("a\nb\n".into())).unwrap();
        assert_eq!(input.text, "a\nb");
        assert!(input.from_stdin);
        // `-` among other words is a word.
        let input = input_from(&words(&["a", "-"]), "task", never).unwrap();
        assert_eq!(input.text, "a -");
    }

    #[test]
    fn empty_text_is_a_usage_error() {
        for (list, stdin) in [(vec![" "], ""), (vec!["-"], " \n")] {
            let stdin = stdin.to_owned();
            match input_from(&words(&list), "message", move || Ok(stdin)) {
                Err(Error::Usage(message)) => assert_eq!(message, "the message is empty"),
                other => panic!("{other:?}"),
            }
        }
    }

    #[test]
    fn the_workspace_must_be_a_directory() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(workspace(Some(tmp.path())).unwrap().is_absolute());
        let missing = tmp.path().join("missing");
        match workspace(Some(&missing)) {
            Err(Error::Usage(message)) => {
                assert_eq!(message, format!("{}: not a directory", missing.display()));
            }
            other => panic!("{other:?}"),
        }
    }
}
