//! Talking to the server: connect, start it when needed, requests and
//! responses, the event stream (see `spec/sessions.md`, "Server").

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use tokio::io::BufReader;
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};

use crate::config::{self, Environment, LoadOptions};
use crate::error::{Error, Result};
use crate::protocol::{
    self, ClientHello, ClientKind, ClientMessage, Outcome, Payload, Request, ServerHello,
    ServerMessage, SessionInfo, VERSION,
};
use crate::server::{runtime_dir, socket_path, version_mismatch};

/// How long a started server may take until it accepts connections.
const START_TIMEOUT: Duration = Duration::from_secs(5);

/// How to reach the server.
#[derive(Debug, Clone)]
pub struct Connect {
    /// The runtime directory; see [`runtime_dir`].
    pub dir: PathBuf,
    /// Start a server if none runs, with this `--config`.
    pub start: bool,
    pub config_file: Option<PathBuf>,
    pub kind: ClientKind,
    pub name: String,
    pub answers: bool,
}

impl Connect {
    /// `clai chat`.
    pub fn chat(config_file: Option<&Path>, answers: bool) -> Connect {
        Connect {
            kind: ClientKind::Chat,
            name: "clai chat".to_owned(),
            answers,
            ..Connect::cli(config_file, true)
        }
    }

    /// A terminal command such as `clai ls`.
    pub fn cli(config_file: Option<&Path>, start: bool) -> Connect {
        Connect {
            dir: runtime_dir(),
            start,
            config_file: config_file.map(Path::to_path_buf),
            kind: ClientKind::Cli,
            name: "clai".to_owned(),
            answers: false,
        }
    }
}

pub struct Client {
    reader: BufReader<OwnedReadHalf>,
    writer: OwnedWriteHalf,
    next_id: u64,
    /// Events and notices read while waiting for a response.
    backlog: VecDeque<ServerMessage>,
    pub server: ServerHello,
}

impl Client {
    /// Connects, starting a server first if `start` is set and none runs.
    /// Without `start`, no server is [`Error::NoServer`].
    pub async fn connect(options: &Connect) -> Result<Client> {
        let socket = socket_path(&options.dir);
        let stream = match UnixStream::connect(&socket).await {
            Ok(stream) => stream,
            Err(e) if no_server(&e) && options.start => start(options, &socket).await?,
            Err(e) if no_server(&e) => return Err(Error::NoServer),
            Err(source) => {
                return Err(Error::Io {
                    context: socket.display().to_string(),
                    source,
                });
            }
        };
        Client::handshake(stream, options).await
    }

    async fn handshake(stream: UnixStream, options: &Connect) -> Result<Client> {
        let (read, mut writer) = stream.into_split();
        let mut reader = BufReader::new(read);
        let hello = ClientMessage::Hello(ClientHello {
            version: VERSION,
            kind: options.kind,
            name: options.name.clone(),
            answers: options.answers,
        });
        protocol::write(&mut writer, &hello).await.map_err(lost)?;
        let server = match protocol::read::<serde_json::Value, _>(&mut reader).await {
            Ok(Some(value)) => value,
            Ok(None) => return Err(Error::Failed("the server closed the connection".into())),
            Err(e) => return Err(Error::Failed(format!("server: {e}"))),
        };
        let version = server.get("version").and_then(serde_json::Value::as_u64);
        match serde_json::from_value::<ServerMessage>(server.clone()) {
            Ok(ServerMessage::Hello(hello)) if hello.version == VERSION => Ok(Client {
                reader,
                writer,
                next_id: 0,
                backlog: VecDeque::new(),
                server: hello,
            }),
            Ok(ServerMessage::Error { message }) => Err(Error::Failed(message)),
            _ => Err(Error::Failed(version_mismatch(
                u32::try_from(version.unwrap_or(0)).unwrap_or(0),
                &VERSION.to_string(),
            ))),
        }
    }

    /// Sends a request and waits for its response; events that come
    /// before it are kept for [`Client::next`].
    pub async fn request(&mut self, request: Request) -> Result<Payload> {
        self.next_id += 1;
        let id = self.next_id;
        protocol::write(&mut self.writer, &ClientMessage::Request { id, request })
            .await
            .map_err(lost)?;
        loop {
            let message = self.read().await?;
            match message {
                ServerMessage::Response { id: got, result } if got == id => {
                    return match result {
                        Outcome::Ok(payload) => Ok(payload),
                        Outcome::Error(e) if e.usage => Err(Error::Usage(e.message)),
                        Outcome::Error(e) => Err(Error::Failed(e.message)),
                    };
                }
                ServerMessage::Error { message } => return Err(Error::Failed(message)),
                other => self.backlog.push_back(other),
            }
        }
    }

    /// The next event or notice; `None` when the server closed the
    /// connection.
    pub async fn next(&mut self) -> Result<Option<ServerMessage>> {
        if let Some(message) = self.backlog.pop_front() {
            return Ok(Some(message));
        }
        protocol::read(&mut self.reader)
            .await
            .map_err(|e| Error::Failed(format!("server: {e}")))
    }

    async fn read(&mut self) -> Result<ServerMessage> {
        match protocol::read(&mut self.reader).await {
            Ok(Some(message)) => Ok(message),
            Ok(None) => Err(Error::Failed("the server closed the connection".into())),
            Err(e) => Err(Error::Failed(format!("server: {e}"))),
        }
    }

    pub async fn list(&mut self) -> Result<Vec<SessionInfo>> {
        match self.request(Request::List).await? {
            Payload::Sessions { sessions } => Ok(sessions),
            other => Err(unexpected(&other)),
        }
    }
}

pub fn unexpected(payload: &Payload) -> Error {
    Error::Failed(format!("unexpected answer from the server: {payload:?}"))
}

fn lost(source: std::io::Error) -> Error {
    Error::Io {
        context: "connection to the server".to_owned(),
        source,
    }
}

fn no_server(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
    )
}

/// Starts `clai server` in the background and waits until it accepts. It
/// inherits the environment, so it finds the same runtime directory.
async fn start(options: &Connect, socket: &Path) -> Result<UnixStream> {
    // A server cannot start with an invalid user file; say why here, with
    // the configuration's exit code.
    config::load(&LoadOptions {
        config_file: options.config_file.clone(),
        workspace: options.dir.clone(),
        env: Environment::from_process(),
    })?;
    let exe = std::env::current_exe().map_err(|source| Error::Io {
        context: "path of clai".to_owned(),
        source,
    })?;
    let mut command = Command::new(exe);
    if let Some(file) = &options.config_file {
        command.arg("--config").arg(file);
    }
    command
        .args(["server", "--detached"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = command.spawn().map_err(|source| Error::Io {
        context: "starting the server".to_owned(),
        source,
    })?;
    tracing::info!(pid = child.id(), "server started");
    let deadline = Instant::now() + START_TIMEOUT;
    loop {
        match UnixStream::connect(socket).await {
            Ok(stream) => {
                // Reap it when it exits; nothing waits for it otherwise.
                std::thread::spawn(move || child.wait());
                return Ok(stream);
            }
            Err(e) if no_server(&e) && Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(_) => {
                let status = child.try_wait().ok().flatten();
                let how = status.map_or_else(
                    || "did not start in time".to_owned(),
                    |s| format!("exited with {s}"),
                );
                return Err(Error::Failed(format!(
                    "the server {how}; run `clai server` to see why"
                )));
            }
        }
    }
}

/// A runtime for a client command.
pub fn runtime() -> Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|source| Error::Io {
            context: "async runtime".to_owned(),
            source,
        })
}

/// Runs `f` with a client connected as a terminal command.
pub fn with_client<T>(
    config_file: Option<&Path>,
    start: bool,
    f: impl AsyncFnOnce(&mut Client) -> Result<T>,
) -> Result<T> {
    let options = Connect::cli(config_file, start);
    runtime()?.block_on(async move {
        let mut client = Client::connect(&options).await?;
        f(&mut client).await
    })
}
