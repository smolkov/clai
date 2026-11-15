//! What several commands share, and the TUI with them: the request for a new
//! session and its creation, the workspace check, and text given as words or
//! `-` for standard input.

use std::io::Read;
use std::path::Path;

use crate::client::{Client, unexpected};
use crate::error::{Error, Result};
use crate::output::note;
use crate::protocol::{Payload, Request, SessionInfo};
use crate::startup::Startup;

use super::Context;

/// The `new` request: `--model` or else this client's `CLAI_MODEL`, which
/// the server checks; `DIR` or the current directory.
pub fn request(
    ctx: &Context,
    name: Option<String>,
    model: Option<String>,
    dir: Option<&Path>,
    skills: Vec<String>,
) -> Result<Request> {
    let model = model.or_else(|| ctx.startup.model.clone());
    Ok(Request::New {
        workspace: ctx.workspace(dir)?,
        name,
        model,
        skills,
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

/// The session of `workspace` that changed last (RFC 0025): its `path` is
/// the workspace, resolved; the latest `updated` wins, and on a tie the
/// higher id, the one created later. Running and saved sessions count
/// alike.
pub fn last_session<'a>(sessions: &'a [SessionInfo], workspace: &Path) -> Option<&'a SessionInfo> {
    // `Context::workspace` resolves links as the server does, so one form
    // is enough (RFC 0026).
    sessions
        .iter()
        .filter(|s| s.path == workspace)
        .max_by_key(|s| (s.updated, s.id))
}

/// The session `--continue` goes to: the last one of the workspace of the
/// `new` request, with its model changed to `model` if given; without one,
/// a new session from `request`, with a note.
pub async fn continue_session(
    client: &mut Client,
    startup: &Startup,
    request: Request,
    model: Option<String>,
) -> Result<String> {
    let Request::New { workspace, .. } = &request else {
        return Err(Error::Failed(
            "--continue needs a new-session request".to_owned(),
        ));
    };
    let sessions = client.list().await?;
    if let Some(session) = last_session(&sessions, workspace) {
        let name = session.name.clone();
        if let Some(model) = model {
            client
                .request(Request::SetModel {
                    session: name.clone(),
                    model,
                })
                .await?;
        }
        return Ok(name);
    }
    note(&format!(
        "no session in {} yet; starting one",
        startup.tilde(workspace)
    ));
    Ok(create(client, request).await?.name)
}

/// The session `clai send --continue` goes to; none is an error (exit `1`).
pub fn last_session_name(
    sessions: &[SessionInfo],
    workspace: &Path,
    startup: &Startup,
) -> Result<String> {
    last_session(sessions, workspace)
        .map(|s| s.name.clone())
        .ok_or_else(|| Error::Failed(format!("no session in {}", startup.tilde(workspace))))
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
        let ctx = Context::default();
        assert!(ctx.workspace(Some(tmp.path())).unwrap().is_absolute());
        // Links are resolved, as the server keeps a session's workspace.
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(tmp.path(), &link).unwrap();
        assert_eq!(
            ctx.workspace(Some(&link)).unwrap(),
            tmp.path().canonicalize().unwrap()
        );
        // A relative one is taken from the current directory of the startup.
        let ctx = Context {
            startup: Startup {
                current_dir: Some(tmp.path().to_path_buf()),
                ..Startup::default()
            },
            ..Context::default()
        };
        assert_eq!(
            ctx.workspace(Some(Path::new("link"))).unwrap(),
            tmp.path().canonicalize().unwrap()
        );
        assert_eq!(
            ctx.workspace(None).unwrap(),
            tmp.path().canonicalize().unwrap()
        );
        let missing = tmp.path().join("missing");
        match ctx.workspace(Some(&missing)) {
            Err(Error::Usage(message)) => {
                assert_eq!(message, format!("{}: not a directory", missing.display()));
            }
            other => panic!("{other:?}"),
        }
    }
}

#[cfg(test)]
mod last_session_tests {
    use super::*;
    use crate::agent::State;

    fn session(id: u64, path: &str, updated: u64) -> SessionInfo {
        SessionInfo {
            id,
            name: format!("s{id}"),
            workspace: path.to_owned(),
            path: path.into(),
            model: "test/m".into(),
            state: State::Idle,
            created: 0,
            updated,
            turns: 0,
            queued: 0,
            question: None,
            usage: Default::default(),
        }
    }

    #[test]
    fn the_last_session_of_the_workspace() {
        let ws = Path::new("/nowhere/ws");
        let mut working = session(4, "/nowhere/ws", 150);
        working.state = State::Working;
        let mut saved = session(5, "/nowhere/ws", 90);
        saved.state = State::Saved;
        let sessions = [
            session(1, "/nowhere/ws", 100),
            working,
            saved,
            // Other workspaces, a parent and a subdirectory, do not count.
            session(2, "/nowhere/other", 500),
            session(3, "/nowhere", 500),
            session(6, "/nowhere/ws/src", 500),
        ];
        assert_eq!(last_session(&sessions, ws).unwrap().id, 4);
        // A saved session counts as well.
        assert_eq!(last_session(&sessions[2..], ws).unwrap().id, 5);
        assert!(last_session(&sessions[3..], ws).is_none());
        assert_eq!(
            last_session_name(&sessions[3..], ws, &Startup::default())
                .unwrap_err()
                .to_string(),
            "no session in /nowhere/ws"
        );
    }

    #[test]
    fn a_tie_goes_to_the_newer_session() {
        let ws = Path::new("/nowhere/ws");
        let sessions = [session(7, "/nowhere/ws", 0), session(3, "/nowhere/ws", 0)];
        assert_eq!(last_session(&sessions, ws).unwrap().id, 7);
    }
}
