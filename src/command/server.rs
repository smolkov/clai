//! `clai server`, `clai server stop`, `clai server status` (see `spec/command.md`).

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::{Args, Subcommand};
use tokio::signal::unix::{SignalKind, signal};
use tokio_util::sync::CancellationToken;

use crate::client::{Client, Connect, runtime, unexpected};
use crate::config::{Environment, LoadOptions};
use crate::error::{Error, Result};
use crate::output::write_stdout;
use crate::protocol::{Payload, Request};
use crate::server::{self, Options, runtime_dir};

/// How long `stop` waits for the server to end its sessions.
const STOP_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Debug, Args)]
pub struct Server {
    #[command(subcommand)]
    pub action: Option<ServerAction>,
    /// Started by a client: leave the terminal's session, ignore `SIGHUP`.
    #[arg(long, hide = true)]
    pub detached: bool,
}

#[derive(Debug, Subcommand)]
pub enum ServerAction {
    /// Stop the running server and end all its sessions.
    Stop {
        /// Do not ask for confirmation.
        #[arg(long)]
        force: bool,
    },
    /// Print whether a server runs, its PID and session count.
    Status,
}

impl Server {
    pub fn name(&self) -> &'static str {
        match self.action {
            None => "server",
            Some(ServerAction::Stop { .. }) => "server stop",
            Some(ServerAction::Status) => "server status",
        }
    }

    pub fn run(&self, config: Option<&Path>) -> Result<()> {
        match self.action {
            None => foreground(config, self.detached),
            Some(ServerAction::Stop { force }) => stop(config, force),
            Some(ServerAction::Status) => status(config),
        }
    }
}

/// Runs the server until it is stopped, idle, or gets `SIGINT`/`SIGTERM`.
fn foreground(config: Option<&Path>, detached: bool) -> Result<()> {
    if detached {
        // Fails only if already a group leader; then it stays where it is.
        let _ = nix::unistd::setsid();
    }
    // `CLAI_MODEL` and `CLAI_THEME` come from each client, not from the
    // environment the server happened to start in.
    let env = Environment {
        model: None,
        theme: None,
        ..Environment::from_process()
    };
    let options = Options::new(
        runtime_dir(),
        LoadOptions {
            config_file: config.map(Path::to_path_buf),
            workspace: PathBuf::new(),
            env,
        },
    );
    let stop = options.stop.clone();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|source| Error::Io {
            context: "async runtime".to_owned(),
            source,
        })?;
    let result = runtime.block_on(async move {
        watch_signals(stop, detached)?;
        server::serve(options).await
    });
    runtime.shutdown_timeout(Duration::from_secs(1));
    result
}

fn watch_signals(stop: CancellationToken, detached: bool) -> Result<()> {
    let listen = |kind: SignalKind| {
        signal(kind).map_err(|source| Error::Io {
            context: "signal handler".to_owned(),
            source,
        })
    };
    let mut int = listen(SignalKind::interrupt())?;
    let mut term = listen(SignalKind::terminate())?;
    let mut hup = listen(SignalKind::hangup())?;
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = int.recv() => {}
                _ = term.recv() => {}
                _ = hup.recv() => {
                    if detached {
                        continue;
                    }
                }
            }
            tracing::info!("signal; stopping the server");
            stop.cancel();
        }
    });
    Ok(())
}

fn status(config: Option<&Path>) -> Result<()> {
    let result = runtime()?.block_on(async {
        let mut client = Client::connect(&Connect::cli(config, false)).await?;
        match client.request(Request::Status).await? {
            Payload::Status(status) => Ok(status),
            other => Err(unexpected(&other)),
        }
    });
    match result {
        Ok(status) => write_stdout(&format!(
            "running, pid {}, {} {}, version {}\n",
            status.pid,
            status.sessions,
            if status.sessions == 1 {
                "session"
            } else {
                "sessions"
            },
            status.version
        )),
        Err(Error::NoServer) => {
            write_stdout("not running\n")?;
            Err(Error::Exit(1))
        }
        Err(e) => Err(e),
    }
}

fn stop(config: Option<&Path>, force: bool) -> Result<()> {
    runtime()?.block_on(async {
        let mut client = Client::connect(&Connect::cli(config, false)).await?;
        let sessions = match client.request(Request::Status).await? {
            Payload::Status(status) => status.sessions,
            other => return Err(unexpected(&other)),
        };
        if sessions > 0 && !force && io::stdin().is_terminal() && !confirm(sessions)? {
            return Err(Error::Failed("not stopped".to_owned()));
        }
        client.request(Request::Stop).await?;
        // The server closes the connection when it has ended its sessions.
        let closed = tokio::time::timeout(STOP_TIMEOUT, async {
            while let Ok(Some(_)) = client.next().await {}
        })
        .await;
        match closed {
            Ok(()) => Ok(()),
            Err(_) => Err(Error::Failed("the server did not stop in time".to_owned())),
        }
    })
}

fn confirm(sessions: usize) -> Result<bool> {
    let mut stderr = io::stderr().lock();
    let plural = if sessions == 1 { "" } else { "s" };
    let _ = write!(
        stderr,
        "stop the server and end {sessions} session{plural}? [y/N] "
    );
    let _ = stderr.flush();
    let mut line = String::new();
    io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(|source| Error::Io {
            context: "standard input".to_owned(),
            source,
        })?;
    Ok(matches!(line.trim(), "y" | "Y" | "yes"))
}
