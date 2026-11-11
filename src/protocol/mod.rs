//! Messages between clients and the server (see `spec/architecture.md`,
//! "Protocol"): one JSON object per line over the Unix socket. The first
//! message in each direction is `hello` with the protocol version.

mod stats;
#[cfg(test)]
mod tests;

pub use stats::{
    AnswerCounts, ClientInfo, DayTokens, DirInfo, GlobalStats, GlobalTotals, LimitsInfo,
    ModelStatus, ModelTotals, Range, RunningTurn, SessionCounts, SessionStatus, Stats, ToolCounts,
    TurnCounts,
};

use std::path::PathBuf;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::agent::{Answer, Question, SessionEvent, State};

/// The protocol version; a new message or event kind needs a new one.
pub const VERSION: u32 = 7;

/// The longest message accepted, in bytes.
pub const MAX_LINE: usize = 16 << 20;

/// What kind of program a client is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClientKind {
    Tui,
    Chat,
    Cli,
    Bridge,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientHello {
    pub version: u32,
    pub kind: ClientKind,
    /// Shown as who answered a question: `tui pts/3`, `clai chat`.
    pub name: String,
    /// Whether it answers questions; clients that only watch get no
    /// choices.
    pub answers: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerHello {
    pub version: u32,
    pub pid: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello(ClientHello),
    /// Gets exactly one `response` with the same id.
    Request {
        id: u64,
        request: Request,
    },
}

/// Session commands. Sessions are named by their current name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum Request {
    New {
        /// Absolute path of the workspace.
        workspace: PathBuf,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// `provider/model` or the short form; `None` for `default_model`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
    },
    List,
    Kill {
        session: String,
    },
    Rename {
        session: String,
        name: String,
    },
    /// Answered with the session's info, then its event log and live
    /// events.
    Attach {
        session: String,
    },
    Detach {
        session: String,
    },
    Send {
        session: String,
        text: String,
    },
    SetModel {
        session: String,
        model: String,
    },
    Cancel {
        session: String,
    },
    /// Answers the open question; the response says whether it counted.
    Answer {
        session: String,
        question: String,
        answer: Answer,
    },
    /// Writes a session, running or saved, to one file.
    Export {
        session: String,
        /// Absolute path of the file.
        path: PathBuf,
    },
    /// Adds a saved session from a file written by `export`.
    Import {
        /// Absolute path of the file.
        path: PathBuf,
        /// Default: the name in the file, made unique.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Status,
    /// The full status of a session, running or saved, with its statistics;
    /// a saved session is not resumed.
    SessionStatus {
        session: String,
    },
    /// Statistics over the logs of all saved sessions.
    GlobalStats {
        range: Range,
        /// The client's time zone, in seconds east of UTC: it sets the days.
        #[serde(default)]
        offset: i64,
    },
    /// The models of the server's configuration. With `workspace`, the
    /// default is the one new sessions there get: its project file counts.
    Models {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        workspace: Option<PathBuf>,
    },
    /// Stops the server; running sessions stop and stay saved.
    Stop,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Hello(ServerHello),
    Response {
        id: u64,
        result: Outcome,
    },
    /// An event of an attached session.
    Event {
        /// The session's id (names can change).
        session: u64,
        /// Part of the log sent on attach, not a live event.
        #[serde(default, skip_serializing_if = "is_false")]
        replay: bool,
        event: SessionEvent,
    },
    Notice {
        notice: Notice,
    },
    /// The connection cannot go on, e.g. a version mismatch.
    Error {
        message: String,
    },
}

fn is_false(b: &bool) -> bool {
    !b
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Ok(Payload),
    Error(ErrorReply),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorReply {
    pub message: String,
    /// The request itself was wrong (unknown model, bad name): exit `2`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub usage: bool,
}

impl ErrorReply {
    pub fn failed(message: impl Into<String>) -> ErrorReply {
        ErrorReply {
            message: message.into(),
            usage: false,
        }
    }

    pub fn usage(message: impl Into<String>) -> ErrorReply {
        ErrorReply {
            message: message.into(),
            usage: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Payload {
    Done,
    Session {
        session: Box<SessionInfo>,
    },
    Sessions {
        sessions: Vec<SessionInfo>,
    },
    Status(Status),
    SessionStatus {
        status: Box<SessionStatus>,
    },
    GlobalStats {
        stats: Box<GlobalStats>,
    },
    Models {
        models: Vec<ModelInfo>,
        /// The user config file the server reads.
        config: PathBuf,
        /// What sets the default: the user file, the project file of the
        /// workspace asked about, or `CLAI_MODEL`.
        #[serde(default)]
        default_from: DefaultFrom,
    },
}

/// Where the default model comes from; see `config::DefaultFrom`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DefaultFrom {
    #[default]
    User,
    Project,
    Env,
}

impl From<crate::config::DefaultFrom> for DefaultFrom {
    fn from(from: crate::config::DefaultFrom) -> DefaultFrom {
        match from {
            crate::config::DefaultFrom::User => DefaultFrom::User,
            crate::config::DefaultFrom::Project => DefaultFrom::Project,
            crate::config::DefaultFrom::Env => DefaultFrom::Env,
        }
    }
}

/// A configured model, in config order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelInfo {
    /// `provider/model`.
    pub model: String,
    pub provider: String,
    pub name: String,
    pub id: String,
    pub context_window: u64,
    pub default: bool,
    /// The key's status as in `clai models --json`; `api_key_cmd` is not run.
    pub key: String,
}

/// Tokens a session has used, summed over its requests from `usage` events
/// (see `spec/sessions.md`, "Persistence"). Input counts the history again with
/// each request, so this is neither the context size nor a bill.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenTotals {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

impl TokenTotals {
    /// Adds a `usage` event; other events change nothing.
    pub fn count(&mut self, event: &SessionEvent) {
        if let SessionEvent::Usage {
            input_tokens,
            output_tokens,
            ..
        } = event
        {
            self.input_tokens = self.input_tokens.saturating_add(*input_tokens);
            self.output_tokens = self.output_tokens.saturating_add(*output_tokens);
        }
    }

    /// The sum over a session's log.
    pub fn of_log(log: &[SessionEvent]) -> TokenTotals {
        let mut totals = TokenTotals::default();
        for event in log {
            totals.count(event);
        }
        totals
    }
}

/// A session as `clai ls` and the session list show it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: u64,
    pub name: String,
    /// As shown: `~/ws/rust/clai`.
    pub workspace: String,
    pub path: PathBuf,
    /// `provider/model`.
    pub model: String,
    pub state: State,
    /// Seconds since 1970.
    pub created: u64,
    /// Seconds since 1970 at the last change; `0` from an older server.
    #[serde(default)]
    pub updated: u64,
    /// Turns started so far.
    #[serde(default)]
    pub turns: u64,
    /// Messages waiting for the running turn to end.
    pub queued: usize,
    /// The open question, if the session waits.
    pub question: Option<Question>,
    pub usage: TokenTotals,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    pub pid: u32,
    /// Running and saved.
    pub sessions: usize,
    /// Loaded in the server.
    pub running: usize,
    /// Working or waiting for an answer.
    pub busy: usize,
    pub clients: usize,
    /// Version of the server binary.
    pub version: String,
}

/// Server-wide news for every client.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Notice {
    /// The user config file changed and was loaded.
    ConfigReloaded,
    /// The user config file changed but is invalid; the previous one stays.
    ConfigRejected {
        errors: Vec<String>,
    },
    SessionCreated {
        session: Box<SessionInfo>,
    },
    Stopping,
}

/// Why reading a message failed.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("message longer than {} MiB", MAX_LINE >> 20)]
    TooLong,
    #[error("invalid message: {0}")]
    Json(#[from] serde_json::Error),
}

/// Reads the next message; `None` at the end of the stream. Empty lines are
/// skipped.
pub async fn read<T, R>(reader: &mut R) -> Result<Option<T>, ReadError>
where
    T: DeserializeOwned,
    R: AsyncBufRead + Unpin,
{
    let mut line = Vec::new();
    loop {
        line.clear();
        let limit = u64::try_from(MAX_LINE).unwrap_or(u64::MAX) + 1;
        let n = (&mut *reader)
            .take(limit)
            .read_until(b'\n', &mut line)
            .await?;
        if n == 0 {
            return Ok(None);
        }
        if line.last() != Some(&b'\n') && line.len() > MAX_LINE {
            return Err(ReadError::TooLong);
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        return Ok(Some(serde_json::from_slice(&line)?));
    }
}

/// Writes one message and its newline.
pub async fn write<T, W>(writer: &mut W, message: &T) -> std::io::Result<()>
where
    T: Serialize,
    W: AsyncWrite + Unpin,
{
    let mut line = serde_json::to_vec(message).map_err(std::io::Error::other)?;
    line.push(b'\n');
    writer.write_all(&line).await?;
    writer.flush().await
}
