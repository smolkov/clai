//! The full-screen interface, a client of the server (see `spec/tui.md`,
//! `rfcs/0001-tui-core.md`): this file is the thin shell around [`App`], which
//! holds all view state. It connects, attaches, sets up the terminal, feeds
//! inputs to `App::update`, carries out its effects and draws.

mod app;
mod ask;
mod commands;
mod conversation;
mod editor;
mod keys;
mod markdown;
mod picker;
mod prompt;
mod sessions;
mod settings;
mod status;
mod style;
pub mod terminal;
mod themes;
mod view;
mod wrap;

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crossterm::event::{Event, EventStream};
use futures_util::StreamExt;
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::mpsc;
use tokio::time::{Instant, MissedTickBehavior, interval_at, sleep, sleep_until};

use crate::client::{Client, Connect, Connection, unexpected};
use crate::command::Context;
use crate::command::common::create;
use crate::config::{self, Config, LoadOptions, Symbols};
use crate::error::{Error, Result};
use crate::protocol::{Payload, Request, ServerMessage, SessionInfo};
use crate::runtime::current_thread;
use crate::server::runtime_dir;

pub use app::Exit;
use app::{App, Attachment, Effect, FileTui, Input, LOST, Reloaded, Settings, Theming};

/// How long to wait between two tries to connect again.
const RECONNECT_EVERY: Duration = Duration::from_millis(500);

/// Which session to show.
#[derive(Debug, Clone, PartialEq)]
pub enum Target {
    /// A `new` request (see `command::common::request`).
    New(Request),
    /// A session by name; without one, the only session there is.
    Attach(Option<String>),
}

/// Runs the TUI until the user leaves. Every failure before the terminal
/// changes is an ordinary error (see `spec/command.md`, "clai").
pub fn run(ctx: &Context, target: Target) -> Result<()> {
    let config = ctx.config();
    let workspace = match &target {
        Target::New(Request::New { workspace, .. }) => workspace.clone(),
        // No project file applies; the session has its own workspace.
        _ => runtime_dir(),
    };
    let options = ctx.load_options(workspace);
    let loaded = config::load(&options)?;
    check_terminal(
        std::io::stdin().is_terminal(),
        std::io::stdout().is_terminal(),
    )?;
    let connect = Connect::tui(config, client_name());
    let exit = current_thread()?.block_on(async {
        let mut client = Client::connect(&connect).await?;
        let (name, choose) = session_name(&mut client, target).await?;
        let attached = attach(client, &name).await?;
        terminal::install_panic_hook();
        let guard = terminal::Guard::new(loaded.tui.mouse)?;
        let exit = event_loop(
            settings(&loaded, &connect, &options),
            options,
            connect.clone(),
            attached,
            choose,
        )
        .await;
        // Restored before anything is printed, also after an error.
        drop(guard);
        exit
    })?;
    finish(&exit)
}

fn check_terminal(stdin: bool, stdout: bool) -> Result<()> {
    if stdin && stdout {
        Ok(())
    } else {
        Err(Error::Usage(
            "the TUI needs a terminal; use \"clai run TASK\" or \"clai chat\"".to_owned(),
        ))
    }
}

/// The line after leaving, and the exit code.
fn finish(exit: &Exit) -> Result<()> {
    if let Some(line) = exit_line(exit) {
        let _ = writeln!(std::io::stderr(), "{line}");
    }
    exit_result(exit)
}

/// What is printed on standard error after the terminal is restored.
fn exit_line(exit: &Exit) -> Option<String> {
    match exit {
        Exit::Detached { name } => Some(format!(
            "clai: detached from {name}; \"clai attach {name}\" to return"
        )),
        Exit::Ended { name, reason } => Some(format!("clai: session {name} ended: {reason}")),
        Exit::Lost | Exit::Signal => None,
    }
}

fn exit_result(exit: &Exit) -> Result<()> {
    match exit {
        Exit::Detached { .. } | Exit::Ended { .. } => Ok(()),
        Exit::Lost => Err(Error::Failed("server disconnected".to_owned())),
        Exit::Signal => Err(Error::Exit(1)),
    }
}

/// `tui pts/3`, from the terminal of standard input; `tui` if it has none.
fn client_name() -> String {
    let tty = nix::unistd::ttyname(std::io::stdin()).ok();
    let tty = tty.as_deref().map(|path| {
        let path = path.strip_prefix("/dev").unwrap_or(path);
        path.strip_prefix("/").unwrap_or(path).display().to_string()
    });
    tty.map_or_else(|| "tui".to_owned(), |tty| format!("tui {tty}"))
}

/// The name of the session to attach to, and whether the user still has to
/// choose one in the session list; creates it for [`Target::New`].
async fn session_name(client: &mut Client, target: Target) -> Result<(String, bool)> {
    match target {
        Target::New(request) => Ok((create(client, request).await?.name, false)),
        Target::Attach(Some(name)) => Ok((name, false)),
        Target::Attach(None) => first_session(&client.list().await?),
    }
}

/// The one session; with several, the first, under the session list.
fn first_session(sessions: &[SessionInfo]) -> Result<(String, bool)> {
    match sessions {
        [] => Err(Error::Failed("no sessions".to_owned())),
        [session] => Ok((session.name.clone(), false)),
        [first, ..] => Ok((first.name.clone(), true)),
    }
}

/// A client attached to a session, with the replayed log.
struct Attached {
    client: Client,
    attachment: Attachment,
}

/// Attaches; the `list` after it is answered after the whole log, so the
/// backlog then holds all of it and the session's current question.
async fn attach(mut client: Client, name: &str) -> Result<Attached> {
    let session = match client
        .request(Request::Attach {
            session: name.to_owned(),
        })
        .await?
    {
        Payload::Session { session } => *session,
        other => return Err(unexpected(&other)),
    };
    let sessions = client.list().await?;
    let backlog: Vec<ServerMessage> = client.take_backlog();
    Ok(Attached {
        client,
        attachment: Attachment {
            session,
            sessions,
            backlog,
        },
    })
}

fn settings(config: &Config, connect: &Connect, options: &LoadOptions) -> Settings {
    let tui = &config.tui;
    let terminal = crate::text::Terminal::detect(tui.symbols);
    let theming = Theming {
        themes: config.themes.clone(),
        warnings: config.theme_warnings.clone(),
        file: file_tui(config, options),
        colouring: colouring(),
        unicode_locale: crate::text::unicode(Symbols::Auto, |name| std::env::var(name).ok()),
        theme_env: options.env.theme.clone(),
        offset: local_offset(),
        user_file: config.user_file.clone(),
    };
    let mut windows = HashMap::new();
    for (provider, p) in &config.providers {
        for (model, m) in &p.models {
            windows.insert(format!("{provider}/{model}"), m.context_window);
        }
    }
    Settings {
        look: style::Look::themed(
            terminal.unicode,
            style::Styles::themed(&tui.theme, &config.themes, colouring()),
        ),
        client: connect.name.clone(),
        tool_output_lines: usize::try_from(tui.tool_output_lines).unwrap_or(usize::MAX),
        expand_tool_calls: tui.expand_tool_calls,
        scroll_lines: usize::from(tui.scroll_lines),
        timestamps: tui.timestamps.then_some(theming.offset),
        windows,
        tui: tui.clone(),
        theming,
    }
}

/// `[tui]` and the themes from the user file again, after `config_reloaded`.
fn reload(options: &LoadOptions) -> std::result::Result<Reloaded, String> {
    let config = config::load(options).map_err(|e| e.to_string())?;
    let file = file_tui(&config, options);
    Ok(Reloaded {
        tui: config.tui,
        themes: config.themes,
        warnings: config.theme_warnings,
        file,
    })
}

/// What the user file itself says about `[tui]`: without `CLAI_THEME`, which
/// loading applied, with the keys it sets and its stamp.
fn file_tui(config: &Config, options: &LoadOptions) -> FileTui {
    let tui = if options.env.theme.is_some() {
        let mut without = options.clone();
        without.env.theme = None;
        config::load(&without).map_or_else(|_| config.tui.clone(), |c| c.tui)
    } else {
        config.tui.clone()
    };
    let text = std::fs::read_to_string(&config.user_file).unwrap_or_default();
    FileTui {
        tui,
        keys: config::edit::tui_keys(&text),
        stamp: config::edit::stamp(&config.user_file),
    }
}

/// Writes `[tui]` changes into the user file, checked as loading would; the
/// new stamp on success.
fn save(
    options: &LoadOptions,
    path: &Path,
    stamp: Option<config::edit::Stamp>,
    changes: &[(&str, Option<config::edit::Setting>)],
) -> std::result::Result<Option<config::edit::Stamp>, String> {
    config::edit::save(path, stamp, changes, |text| {
        config::load_with_user_text(path, text, options)
            .map(drop)
            .map_err(|e| e.to_string())
    })?;
    Ok(config::edit::stamp(path))
}

/// The colours of this terminal, from `NO_COLOR`, standard output and
/// `COLORTERM`.
fn colouring() -> style::Colouring {
    let colour = std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none();
    style::Colouring::detect(colour, std::env::var("COLORTERM").ok().as_deref())
}

/// The offset of local time from UTC in seconds, from `date`: the standard
/// library has no local time, and it is not worth a dependency.
pub(crate) fn local_offset() -> i64 {
    Command::new("date")
        .arg("+%z")
        .output()
        .ok()
        .and_then(|out| parse_offset(String::from_utf8_lossy(&out.stdout).trim()))
        .unwrap_or(0)
}

/// `+0530` in seconds.
fn parse_offset(text: &str) -> Option<i64> {
    let digits = text.strip_prefix(['+', '-'])?;
    let hours: i64 = digits.get(..2)?.parse().ok()?;
    let minutes: i64 = digits.get(2..4)?.parse().ok()?;
    let seconds = hours * 3600 + minutes * 60;
    Some(if text.starts_with('-') {
        -seconds
    } else {
        seconds
    })
}

/// What the helper tasks report to the loop.
enum Report {
    Input(Input),
    Attached(Box<Attached>),
}

/// Attaches to another session on a new connection.
async fn switch(connect: Connect, name: String) -> Result<Attached> {
    let client = Client::connect(&connect).await?;
    attach(client, &name).await
}

/// One try to get back to the session: starts the server if needed.
async fn reconnect(connect: Connect, id: u64, name: String) -> Result<Attached> {
    let mut client = Client::connect(&connect).await?;
    let sessions = client.list().await?;
    // By id, in case it was renamed meanwhile.
    let current = sessions
        .iter()
        .find(|s| s.id == id)
        .or_else(|| sessions.iter().find(|s| s.name == name))
        .ok_or_else(|| Error::Failed(format!("no session named {name}")))?
        .name
        .clone();
    attach(client, &current).await
}

async fn next_message(connection: &mut Option<Connection>) -> Option<ServerMessage> {
    match connection {
        Some(connection) => connection.next().await,
        None => std::future::pending().await,
    }
}

async fn event_loop(
    settings: Settings,
    options: LoadOptions,
    connect: Connect,
    attached: Attached,
    choose: bool,
) -> Result<Exit> {
    let drawing = Error::io("drawing the screen");
    let backend = CrosstermBackend::new(std::io::stdout());
    let mut screen = Terminal::new(backend).map_err(&drawing)?;
    screen.clear().map_err(&drawing)?;
    let Attached { client, attachment } = attached;
    let mut connection = Some(Connection::spawn(client));
    let mut app = App::new(settings, attachment, std::time::Instant::now());
    if choose {
        app.choose_session();
    }
    let (reports, mut incoming) = mpsc::unbounded_channel();

    let mut terminal_events = EventStream::new();
    let mut terminate = signal(SignalKind::terminate()).map_err(Error::io("signal handler"))?;
    let mut hangup = signal(SignalKind::hangup()).map_err(Error::io("signal handler"))?;
    let second = Duration::from_secs(1);
    let mut ticks = interval_at(Instant::now() + second, second);
    ticks.set_missed_tick_behavior(MissedTickBehavior::Delay);

    // A whole replay is applied before this first frame.
    draw(&mut screen, &mut app)?;
    let mut dirty = false;
    let mut last_frame = Instant::now();
    loop {
        let input = tokio::select! {
            event = terminal_events.next() => match event {
                Some(Ok(event)) => terminal_input(event),
                Some(Err(source)) => return Err(Error::Io {
                    context: "reading the terminal".to_owned(),
                    source,
                }),
                // No terminal any more: as after `SIGHUP`.
                None => return Ok(Exit::Signal),
            },
            message = next_message(&mut connection) => match message {
                Some(message) => Some(Input::Server(message)),
                None => {
                    connection = None;
                    Some(Input::Disconnected)
                }
            },
            Some(report) = incoming.recv() => match report {
                Report::Input(input) => Some(input),
                Report::Attached(attached) => {
                    let Attached { client, attachment } = *attached;
                    connection = Some(Connection::spawn(client));
                    Some(Input::Attached(Box::new(attachment)))
                }
            },
            () = sleep_until(last_frame + app.frame_time()), if dirty => {
                draw(&mut screen, &mut app)?;
                dirty = false;
                last_frame = Instant::now();
                None
            }
            _ = ticks.tick() => Some(Input::Tick),
            _ = terminate.recv() => return Ok(Exit::Signal),
            _ = hangup.recv() => return Ok(Exit::Signal),
        };
        let Some(input) = input else { continue };
        app.now = std::time::Instant::now();
        for effect in app.update(input) {
            match effect {
                Effect::Redraw => dirty = true,
                Effect::Exit(exit) => return Ok(exit),
                Effect::Request(request) => send(&mut connection, request, &reports),
                Effect::Mouse(on) => {
                    if let Err(e) = terminal::set_mouse(on) {
                        tracing::info!("mouse capture: {e}");
                    }
                }
                Effect::Save {
                    path,
                    stamp,
                    changes,
                } => {
                    let result = save(&options, &path, stamp, &changes);
                    let _ = reports.send(Report::Input(Input::Saved(result)));
                }
                Effect::ReloadConfig => {
                    let reloaded = reload(&options);
                    let _ = reports.send(Report::Input(Input::Reloaded(Box::new(reloaded))));
                }
                Effect::Switch(name) => {
                    let (connect, reports) = (connect.clone(), reports.clone());
                    tokio::spawn(async move {
                        let report = match switch(connect, name).await {
                            Ok(attached) => Report::Attached(Box::new(attached)),
                            Err(e) => Report::Input(Input::SwitchFailed(e.to_string())),
                        };
                        let _ = reports.send(report);
                    });
                }
                Effect::Reconnect => {
                    let (connect, reports) = (connect.clone(), reports.clone());
                    let (id, name) = (app.session.id, app.session.name.clone());
                    tokio::spawn(async move {
                        sleep(RECONNECT_EVERY).await;
                        let report = match reconnect(connect, id, name).await {
                            Ok(attached) => Report::Attached(Box::new(attached)),
                            Err(e) => {
                                tracing::info!("connecting again: {e}");
                                Report::Input(Input::ReconnectFailed)
                            }
                        };
                        let _ = reports.send(report);
                    });
                }
            }
        }
    }
}

/// Sends a request; its response comes back as an input, in its own time.
fn send(
    connection: &mut Option<Connection>,
    request: Request,
    reports: &mpsc::UnboundedSender<Report>,
) {
    let response = connection.as_mut().map(|c| c.request(request.clone()));
    let reports = reports.clone();
    tokio::spawn(async move {
        let result = match response {
            Some(response) => match response.await {
                Ok(result) => result.map_err(|e| e.to_string()),
                Err(_) => Err(LOST.to_owned()),
            },
            None => Err(LOST.to_owned()),
        };
        let _ = reports.send(Report::Input(Input::Response { request, result }));
    });
}

fn terminal_input(event: Event) -> Option<Input> {
    match event {
        Event::Key(key) => Some(Input::Key(key)),
        Event::Paste(text) => Some(Input::Paste(text)),
        Event::Mouse(mouse) => Some(Input::Mouse(mouse)),
        Event::Resize(..) => Some(Input::Resize),
        Event::FocusGained | Event::FocusLost => None,
    }
}

fn draw(screen: &mut Terminal<CrosstermBackend<std::io::Stdout>>, app: &mut App) -> Result<()> {
    let drawing = Error::io("drawing the screen");
    let size = screen.size().map_err(&drawing)?;
    app.layout(Rect::new(0, 0, size.width, size.height));
    screen
        .draw(|frame| view::draw(app, frame))
        .map_err(&drawing)?;
    Ok(())
}
