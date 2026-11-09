//! The server in-process, with scripted providers, a temporary runtime
//! directory and real clients.

use std::collections::VecDeque;
use std::fs;
use std::sync::Mutex;

use futures_util::stream::{self, StreamExt};
use serde_json::json;
use tokio::io::AsyncWriteExt;
use tokio::task::JoinHandle;

use super::*;
use crate::agent::{Answer, CallState, Closed, EndReason, SessionEvent, State};
use crate::client::{Client, Connect};
use crate::config::Environment;
use crate::protocol::{ClientKind, ModelInfo, Payload, Request, SessionInfo, TokenTotals};
use crate::provider::{Event, EventStream, ProviderError, Request as ProviderRequest, StopReason};
use crate::session::Store;

mod connection;

const CONFIG: &str = r#"
default_model = "fake/one"

[providers.fake]
api      = "openai-compatible"
base_url = "http://127.0.0.1:9/v1"
api_key  = "k"

[providers.fake.models.one]
id             = "model-one"
context_window = 100000

[providers.fake.models.two]
id             = "model-two"
context_window = 100000
"#;

const WAIT: Duration = Duration::from_secs(10);

enum Step {
    Events(Vec<std::result::Result<Event, ProviderError>>),
    /// Nothing until the request is cancelled.
    Hang,
    /// A `usage` event and a text, then nothing until the request is cancelled.
    UsageThenHang(u64, u64),
    Panic,
}

#[derive(Default)]
struct Script {
    steps: Mutex<VecDeque<Step>>,
    models: Mutex<Vec<String>>,
    requests: Mutex<Vec<ProviderRequest>>,
}

impl Script {
    fn new(steps: Vec<Step>) -> Arc<Script> {
        Arc::new(Script {
            steps: Mutex::new(steps.into()),
            ..Script::default()
        })
    }
}

impl Provider for Script {
    fn stream(&self, request: ProviderRequest, cancel: CancellationToken) -> EventStream {
        self.models.lock().unwrap().push(request.model.clone());
        self.requests.lock().unwrap().push(request.clone());
        match self.steps.lock().unwrap().pop_front() {
            Some(Step::Events(events)) => stream::iter(events).boxed(),
            Some(Step::Hang) => stream::once(async move {
                cancel.cancelled().await;
                Err(ProviderError::cancelled())
            })
            .boxed(),
            Some(Step::UsageThenHang(input_tokens, output_tokens)) => stream::iter([
                Ok(Event::Usage {
                    input_tokens,
                    output_tokens,
                }),
                Ok(Event::TextDelta { text: "x".into() }),
            ])
            .chain(stream::once(async move {
                cancel.cancelled().await;
                Err(ProviderError::cancelled())
            }))
            .boxed(),
            Some(Step::Panic) => panic!("scripted panic"),
            None => stream::iter([Err(ProviderError::protocol("script: no more answers"))]).boxed(),
        }
    }
}

fn say(text: &str) -> Step {
    Step::Events(vec![
        Ok(Event::TextDelta { text: text.into() }),
        Ok(Event::Done {
            stop_reason: StopReason::EndTurn,
        }),
    ])
}

fn write_call(id: &str, path: &str) -> Step {
    let arguments = json!({"path": path, "content": "hello\n"});
    Step::Events(vec![
        Ok(Event::ToolCallStart {
            id: id.into(),
            name: "write_file".into(),
        }),
        Ok(Event::ToolCallEnd {
            id: id.into(),
            arguments,
        }),
        Ok(Event::Done {
            stop_reason: StopReason::ToolUse,
        }),
    ])
}

struct Fixture {
    tmp: tempfile::TempDir,
    dir: PathBuf,
    config: PathBuf,
    ws: PathBuf,
    stop: CancellationToken,
    server: JoinHandle<Result<()>>,
    scripts: Arc<Mutex<VecDeque<Arc<Script>>>>,
}

impl Fixture {
    async fn start(scripts: Vec<Arc<Script>>) -> Fixture {
        Fixture::start_with(scripts, Duration::from_secs(60)).await
    }

    async fn start_with(scripts: Vec<Arc<Script>>, idle: Duration) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("run/clai");
        let config = tmp.path().join("config.toml");
        fs::write(&config, CONFIG).unwrap();
        let ws = tmp.path().join("ws");
        fs::create_dir_all(&ws).unwrap();
        let scripts = Arc::new(Mutex::new(VecDeque::from(scripts)));
        let (stop, server) = launch(&dir, &config, &scripts, idle).await;
        Fixture {
            tmp,
            dir,
            config,
            ws,
            stop,
            server,
            scripts,
        }
    }

    /// Stops the server and starts a new one on the same directories.
    async fn restart(&mut self) {
        self.stop.cancel();
        tokio::time::timeout(WAIT, &mut self.server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let (stop, server) = launch(&self.dir, &self.config, &self.scripts, WAIT).await;
        self.stop = stop;
        self.server = server;
    }

    fn sessions_dir(&self) -> PathBuf {
        self.dir.join("sessions")
    }

    async fn client(&self, name: &str, answers: bool) -> Client {
        Client::connect(&Connect {
            dir: self.dir.clone(),
            start: false,
            config_file: None,
            kind: ClientKind::Chat,
            name: name.into(),
            answers,
        })
        .await
        .unwrap()
    }

    fn add_script(&self, script: Arc<Script>) {
        self.scripts.lock().unwrap().push_back(script);
    }

    async fn stop(self) {
        self.stop.cancel();
        tokio::time::timeout(WAIT, self.server)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}

/// Starts a server and waits until it accepts connections.
async fn launch(
    dir: &Path,
    config: &Path,
    scripts: &Arc<Mutex<VecDeque<Arc<Script>>>>,
    idle: Duration,
) -> (CancellationToken, JoinHandle<Result<()>>) {
    let options = options(dir, config, scripts, idle);
    let stop = options.stop.clone();
    let server = tokio::spawn(serve(options));
    let socket = socket_path(dir);
    let deadline = Instant::now() + WAIT;
    while UnixStream::connect(&socket).await.is_err() {
        assert!(!server.is_finished(), "server ended early");
        assert!(Instant::now() < deadline, "server did not start");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    (stop, server)
}

fn options(
    dir: &Path,
    config: &Path,
    scripts: &Arc<Mutex<VecDeque<Arc<Script>>>>,
    idle: Duration,
) -> Options {
    let scripts = scripts.clone();
    let providers: ProviderFactory = Arc::new(move |_| {
        let script = scripts.lock().unwrap().pop_front().unwrap_or_default();
        Ok(script as Arc<dyn Provider>)
    });
    Options {
        dir: dir.to_path_buf(),
        sessions_dir: dir.join("sessions"),
        idle,
        reload_every: Duration::from_millis(50),
        load: LoadOptions {
            config_file: Some(config.to_path_buf()),
            workspace: PathBuf::new(),
            env: Environment::default(),
        },
        providers,
        stop: CancellationToken::new(),
    }
}

async fn new_session(client: &mut Client, ws: &Path, name: Option<&str>) -> SessionInfo {
    match client
        .request(Request::New {
            workspace: ws.to_path_buf(),
            name: name.map(str::to_owned),
            model: None,
        })
        .await
        .unwrap()
    {
        Payload::Session { session } => *session,
        other => panic!("{other:?}"),
    }
}

async fn attach(client: &mut Client, name: &str) -> SessionInfo {
    match client
        .request(Request::Attach {
            session: name.into(),
        })
        .await
        .unwrap()
    {
        Payload::Session { session } => *session,
        other => panic!("{other:?}"),
    }
}

async fn send(client: &mut Client, name: &str, text: &str) {
    client
        .request(Request::Send {
            session: name.into(),
            text: text.into(),
        })
        .await
        .unwrap();
}

async fn answer(
    client: &mut Client,
    name: &str,
    question: &str,
    answer: Answer,
) -> std::result::Result<Payload, Error> {
    client
        .request(Request::Answer {
            session: name.into(),
            question: question.into(),
            answer,
        })
        .await
}

/// Reads events until `f` picks one; returns it and the events before it.
async fn until<T>(
    client: &mut Client,
    mut f: impl FnMut(&SessionEvent, bool) -> Option<T>,
) -> (T, Vec<SessionEvent>) {
    let mut seen = Vec::new();
    let result = tokio::time::timeout(WAIT, async {
        loop {
            match client.next().await.unwrap() {
                Some(ServerMessage::Event { event, replay, .. }) => {
                    if let Some(t) = f(&event, replay) {
                        return t;
                    }
                    seen.push(event);
                }
                Some(_) => {}
                None => panic!("connection closed; seen {seen:#?}"),
            }
        }
    })
    .await;
    match result {
        Ok(t) => (t, seen),
        Err(_) => panic!("timed out; seen {seen:#?}"),
    }
}

/// The end of the next live turn; replayed ones do not count.
async fn turn_end(client: &mut Client) -> EndReason {
    until(client, |e, replay| match e {
        SessionEvent::TurnEnd(end) if !replay => Some(end.reason),
        _ => None,
    })
    .await
    .0
}

async fn question(client: &mut Client) -> crate::agent::Question {
    until(client, |e, _| match e {
        SessionEvent::Question(q) => Some((**q).clone()),
        _ => None,
    })
    .await
    .0
}

/// The next notice about the configuration.
async fn notice(client: &mut Client) -> Notice {
    tokio::time::timeout(WAIT, async {
        loop {
            if let Some(ServerMessage::Notice { notice }) = client.next().await.unwrap()
                && !matches!(notice, Notice::SessionCreated { .. })
            {
                return notice;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn sessions_run_in_parallel() {
    let slow = Script::new(vec![Step::Hang, say("later")]);
    let fast = Script::new(vec![say("hi")]);
    let fx = Fixture::start(vec![slow, fast]).await;
    let mut c = fx.client("tui pts/1", true).await;
    let a = new_session(&mut c, &fx.ws, None).await;
    let b = new_session(&mut c, &fx.ws, None).await;
    assert_eq!((a.name.as_str(), b.name.as_str()), ("ws", "ws-2"));
    attach(&mut c, "ws").await;
    attach(&mut c, "ws-2").await;
    send(&mut c, "ws", "slow").await;
    send(&mut c, "ws-2", "fast").await;
    // ws-2 finishes while ws still waits for its model.
    assert_eq!(turn_end(&mut c).await, EndReason::Done);
    let sessions = c.list().await.unwrap();
    assert_eq!(sessions[0].state, State::Working);
    assert_eq!(sessions[1].state, State::Idle);
    c.request(Request::Cancel {
        session: "ws".into(),
    })
    .await
    .unwrap();
    assert_eq!(turn_end(&mut c).await, EndReason::Cancelled);
    fx.stop().await;
}

#[tokio::test]
async fn names() {
    let fx = Fixture::start(vec![]).await;
    let mut c = fx.client("cli", false).await;
    new_session(&mut c, &fx.ws, Some("one")).await;
    let taken = c
        .request(Request::New {
            workspace: fx.ws.clone(),
            name: Some("one".into()),
            model: None,
        })
        .await;
    assert!(matches!(taken, Err(Error::Failed(m)) if m == "the name one is taken"));
    let bad = c
        .request(Request::New {
            workspace: fx.ws.clone(),
            name: Some("a b".into()),
            model: None,
        })
        .await;
    assert!(matches!(bad, Err(Error::Usage(_))));
    let unknown = c
        .request(Request::New {
            workspace: fx.ws.clone(),
            name: None,
            model: Some("nope".into()),
        })
        .await;
    assert!(matches!(unknown, Err(Error::Usage(m)) if m.contains("unknown model `nope`")));
    let not_dir = c
        .request(Request::New {
            workspace: fx.ws.join("missing"),
            name: None,
            model: None,
        })
        .await;
    assert!(matches!(not_dir, Err(Error::Usage(_))));

    c.request(Request::Rename {
        session: "one".into(),
        name: "two".into(),
    })
    .await
    .unwrap();
    let sessions = c.list().await.unwrap();
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].name, "two");
    assert_eq!(sessions[0].model, "fake/one");
    let gone = c
        .request(Request::Kill {
            session: "one".into(),
        })
        .await;
    assert!(matches!(gone, Err(Error::Failed(m)) if m == "no session named one"));
    fx.stop().await;
}

#[tokio::test]
async fn first_answer_counts() {
    let script = Script::new(vec![write_call("c1", "a.txt"), say("written")]);
    let fx = Fixture::start(vec![script]).await;
    let mut tui = fx.client("tui pts/3", true).await;
    let mut chat = fx.client("clai chat", true).await;
    let mut watch = fx.client("watcher", false).await;
    new_session(&mut tui, &fx.ws, Some("s")).await;
    for c in [&mut tui, &mut chat, &mut watch] {
        attach(c, "s").await;
    }
    send(&mut tui, "s", "write a.txt").await;
    let q = question(&mut tui).await;
    assert_eq!(q.session.name, "s");
    assert!(q.summary.starts_with("write_file: a.txt"), "{}", q.summary);
    assert_eq!(question(&mut chat).await.id, q.id);
    // Clients that only watch get no choices.
    assert!(question(&mut watch).await.choices.is_empty());

    answer(&mut chat, "s", &q.id, Answer::Allow).await.unwrap();
    let late = answer(&mut tui, "s", &q.id, Answer::Deny).await;
    assert!(
        matches!(&late, Err(Error::Failed(m)) if m == "already answered by clai chat"),
        "{late:?}"
    );
    let (closed, _) = until(&mut tui, |e, _| match e {
        SessionEvent::QuestionClosed { closed, .. } => Some(closed.clone()),
        _ => None,
    })
    .await;
    assert_eq!(
        closed,
        Closed::Answered {
            answer: Answer::Allow,
            by: "clai chat".into()
        }
    );
    assert_eq!(turn_end(&mut tui).await, EndReason::Done);
    assert_eq!(fs::read_to_string(fx.ws.join("a.txt")).unwrap(), "hello\n");
    // A watcher cannot answer: it has no choices, and a stale id fails.
    let stale = answer(&mut watch, "s", &q.id, Answer::Allow).await;
    assert!(matches!(stale, Err(Error::Failed(_))));
    fx.stop().await;
}

#[tokio::test]
async fn wrong_answers_keep_the_question_open() {
    let script = Script::new(vec![write_call("c1", "a.txt"), say("ok")]);
    let fx = Fixture::start(vec![script]).await;
    let mut c = fx.client("clai chat", true).await;
    new_session(&mut c, &fx.ws, Some("s")).await;
    attach(&mut c, "s").await;
    send(&mut c, "s", "go").await;
    let q = question(&mut c).await;

    let rule = answer(
        &mut c,
        "s",
        &q.id,
        Answer::AllowAlways {
            rules: vec!["write_file(b.txt)".into()],
        },
    )
    .await;
    assert!(matches!(&rule, Err(Error::Failed(_))), "{rule:?}");
    let choice = answer(&mut c, "s", &q.id, Answer::Continue).await;
    assert!(
        matches!(&choice, Err(Error::Failed(m)) if m.contains("not an answer")),
        "{choice:?}"
    );
    let wrong_id = answer(&mut c, "s", "q999", Answer::Allow).await;
    assert!(matches!(&wrong_id, Err(Error::Failed(m)) if m.contains("not the open question")));
    let info = c.list().await.unwrap().remove(0);
    assert_eq!(info.state, State::Waiting);
    assert_eq!(info.question.map(|q| q.id), Some(q.id.clone()));

    answer(
        &mut c,
        "s",
        &q.id,
        Answer::AllowAlways {
            rules: vec!["write_file(a.txt)".into()],
        },
    )
    .await
    .unwrap();
    assert_eq!(turn_end(&mut c).await, EndReason::Done);
    fx.stop().await;
}

#[tokio::test]
async fn detach_and_replay() {
    let script = Script::new(vec![write_call("c1", "a.txt"), say("done")]);
    let fx = Fixture::start(vec![script]).await;
    let mut first = fx.client("tui pts/1", true).await;
    new_session(&mut first, &fx.ws, Some("s")).await;
    attach(&mut first, "s").await;
    send(&mut first, "s", "write it").await;
    let q = question(&mut first).await;
    drop(first);

    // The session waits without a client; a new one gets the whole log.
    let mut second = fx.client("tui pts/2", true).await;
    let info = attach(&mut second, "s").await;
    assert_eq!(info.state, State::Waiting);
    assert_eq!(info.question.as_ref().map(|q| &q.id), Some(&q.id));
    let (replayed, before) = until(&mut second, |e, replay| match e {
        SessionEvent::Question(q) => Some((q.id.clone(), replay)),
        _ => None,
    })
    .await;
    assert_eq!(replayed, (q.id.clone(), true));
    assert!(before.contains(&SessionEvent::UserMessage {
        text: "write it".into()
    }));
    assert!(
        before
            .iter()
            .any(|e| matches!(e, SessionEvent::ToolCall(c) if c.state == CallState::Waiting))
    );
    answer(&mut second, "s", &q.id, Answer::Deny).await.unwrap();
    assert_eq!(turn_end(&mut second).await, EndReason::Done);
    fx.stop().await;
}

#[tokio::test]
async fn messages_are_queued() {
    let script = Script::new(vec![Step::Hang, say("second")]);
    let fx = Fixture::start(vec![script]).await;
    let mut c = fx.client("tui", true).await;
    new_session(&mut c, &fx.ws, Some("s")).await;
    attach(&mut c, "s").await;
    send(&mut c, "s", "one").await;
    send(&mut c, "s", "two").await;
    assert_eq!(c.list().await.unwrap()[0].queued, 1);
    c.request(Request::Cancel {
        session: "s".into(),
    })
    .await
    .unwrap();
    assert_eq!(turn_end(&mut c).await, EndReason::Cancelled);
    let (_, events) = until(&mut c, |e, _| match e {
        SessionEvent::TurnEnd(end) => Some(end.reason),
        _ => None,
    })
    .await;
    assert!(events.contains(&SessionEvent::UserMessage { text: "two".into() }));
    assert!(events.contains(&SessionEvent::TextDone {
        text: "second".into()
    }));
    fx.stop().await;
}

#[tokio::test]
async fn kill_during_a_question() {
    let script = Script::new(vec![write_call("c1", "a.txt")]);
    let fx = Fixture::start(vec![script]).await;
    let mut c = fx.client("tui", true).await;
    new_session(&mut c, &fx.ws, Some("s")).await;
    attach(&mut c, "s").await;
    send(&mut c, "s", "go").await;
    let q = question(&mut c).await;
    let mut other = fx.client("cli", false).await;
    other
        .request(Request::Kill {
            session: "s".into(),
        })
        .await
        .unwrap();
    let (reason, events) = until(&mut c, |e, _| match e {
        SessionEvent::Ended { reason } => Some(reason.clone()),
        _ => None,
    })
    .await;
    assert_eq!(reason, "killed");
    assert!(events.iter().any(|e| matches!(
        e,
        SessionEvent::QuestionClosed {
            closed: Closed::Cancelled,
            ..
        }
    )));
    assert!(!fx.ws.join("a.txt").exists());
    assert!(other.list().await.unwrap().is_empty());
    let late = answer(&mut c, "s", &q.id, Answer::Allow).await;
    assert!(matches!(late, Err(Error::Failed(m)) if m == "no session named s"));
    fx.stop().await;
}

#[tokio::test]
async fn a_failing_session_does_not_affect_another() {
    let bad = Script::new(vec![Step::Panic]);
    let good = Script::new(vec![say("fine")]);
    let fx = Fixture::start(vec![bad, good]).await;
    let mut c = fx.client("tui", true).await;
    new_session(&mut c, &fx.ws, Some("bad")).await;
    new_session(&mut c, &fx.ws, Some("good")).await;
    attach(&mut c, "bad").await;
    send(&mut c, "bad", "go").await;
    let (reason, events) = until(&mut c, |e, _| match e {
        SessionEvent::Ended { reason } => Some(reason.clone()),
        _ => None,
    })
    .await;
    assert_eq!(reason, "failed");
    assert!(events.iter().any(
        |e| matches!(e, SessionEvent::Error { message } if message.starts_with("the session failed"))
    ));
    // Saved again once it has ended: its files are intact.
    let deadline = Instant::now() + WAIT;
    while c.list().await.unwrap()[0].state != State::Saved {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    attach(&mut c, "good").await;
    send(&mut c, "good", "go").await;
    assert_eq!(turn_end(&mut c).await, EndReason::Done);
    // The failed one resumes; its interrupted turn is closed.
    fx.add_script(Script::new(vec![say("better")]));
    let info = attach(&mut c, "bad").await;
    assert_eq!(info.state, State::Idle);
    send(&mut c, "bad", "again").await;
    assert_eq!(turn_end(&mut c).await, EndReason::Done);
    fx.stop().await;
}

#[tokio::test]
async fn model_change_applies_to_the_next_request() {
    let script = Script::new(vec![say("one"), say("two")]);
    let fx = Fixture::start(vec![script.clone()]).await;
    fx.add_script(script.clone());
    let mut c = fx.client("tui", true).await;
    new_session(&mut c, &fx.ws, Some("s")).await;
    attach(&mut c, "s").await;
    send(&mut c, "s", "a").await;
    turn_end(&mut c).await;
    c.request(Request::SetModel {
        session: "s".into(),
        model: "two".into(),
    })
    .await
    .unwrap();
    let (model, _) = until(&mut c, |e, _| match e {
        SessionEvent::ModelChanged { model, .. } => Some(model.clone()),
        _ => None,
    })
    .await;
    assert_eq!(model, "fake/two");
    send(&mut c, "s", "b").await;
    turn_end(&mut c).await;
    assert_eq!(*script.models.lock().unwrap(), ["model-one", "model-two"]);
    assert_eq!(c.list().await.unwrap()[0].model, "fake/two");
    let bad = c
        .request(Request::SetModel {
            session: "s".into(),
            model: "fake/zzz".into(),
        })
        .await;
    assert!(matches!(bad, Err(Error::Usage(_))));
    fx.stop().await;
}

#[tokio::test]
async fn version_mismatch() {
    let fx = Fixture::start(vec![]).await;
    let mut stream = UnixStream::connect(socket_path(&fx.dir)).await.unwrap();
    stream
        .write_all(b"{\"type\":\"hello\",\"version\":99,\"kind\":\"cli\",\"name\":\"x\",\"answers\":false}\n")
        .await
        .unwrap();
    let mut reader = BufReader::new(stream);
    let reply: ServerMessage = protocol::read(&mut reader).await.unwrap().unwrap();
    let ServerMessage::Error { message } = reply else {
        panic!("{reply:?}");
    };
    assert!(
        message.contains(&format!("version {}", protocol::VERSION)),
        "{message}"
    );
    assert!(message.contains("clai server stop"), "{message}");
    fx.stop().await;
}

#[tokio::test]
async fn only_one_server() {
    let fx = Fixture::start(vec![]).await;
    let scripts = Arc::new(Mutex::new(VecDeque::new()));
    let second = serve(options(&fx.dir, &fx.config, &scripts, WAIT)).await;
    assert!(matches!(second, Err(Error::Failed(m)) if m == "a server is already running"));
    let mut c = fx.client("cli", false).await;
    let Payload::Status(status) = c.request(Request::Status).await.unwrap() else {
        panic!("not status");
    };
    assert_eq!(status.pid, std::process::id());
    assert_eq!(status.clients, 1);
    fx.stop().await;
}

#[tokio::test]
async fn stop_request_ends_sessions() {
    let fx = Fixture::start(vec![Script::new(vec![Step::Hang])]).await;
    let mut c = fx.client("tui", true).await;
    new_session(&mut c, &fx.ws, Some("s")).await;
    attach(&mut c, "s").await;
    send(&mut c, "s", "go").await;
    let mut cli = fx.client("cli", false).await;
    cli.request(Request::Stop).await.unwrap();
    let (reason, _) = until(&mut c, |e, _| match e {
        SessionEvent::Ended { reason } => Some(reason.clone()),
        _ => None,
    })
    .await;
    assert_eq!(reason, "server stopped");
    tokio::time::timeout(WAIT, fx.server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(!socket_path(&fx.dir).exists());
}

#[tokio::test]
async fn idle_exit() {
    let fx = Fixture::start_with(vec![], Duration::from_millis(200)).await;
    let started = Instant::now();
    tokio::time::timeout(WAIT, fx.server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(started.elapsed() >= Duration::from_millis(200));
    assert!(!socket_path(&fx.dir).exists());
    drop(fx.tmp);
}

#[tokio::test]
async fn config_reload() {
    let fx = Fixture::start(vec![]).await;
    let mut c = fx.client("tui", true).await;
    fs::write(&fx.config, "default_model = 3\n").unwrap();
    let Notice::ConfigRejected { errors } = notice(&mut c).await else {
        panic!("not rejected");
    };
    assert!(!errors.is_empty());
    // The previous configuration stays.
    new_session(&mut c, &fx.ws, Some("a")).await;

    let three = CONFIG.replace(
        "[providers.fake.models.two]",
        "[providers.fake.models.three]",
    );
    fs::write(&fx.config, three).unwrap();
    assert_eq!(notice(&mut c).await, Notice::ConfigReloaded);
    let created = c
        .request(Request::New {
            workspace: fx.ws.clone(),
            name: Some("b".into()),
            model: Some("three".into()),
        })
        .await
        .unwrap();
    assert!(matches!(created, Payload::Session { session } if session.model == "fake/three"));
    fx.stop().await;
}

/// The texts of the user messages a request sent to the model.
fn user_texts(script: &Script) -> Vec<Vec<String>> {
    script
        .requests
        .lock()
        .unwrap()
        .iter()
        .map(|r| {
            r.messages
                .iter()
                .filter(|m| m.role == crate::provider::Role::User)
                .flat_map(|m| &m.content)
                .filter_map(|b| match b {
                    crate::provider::Block::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect()
        })
        .collect()
}

#[tokio::test]
async fn sessions_survive_a_restart() {
    let script = Script::new(vec![
        write_call("c1", "a.txt"),
        say("written"),
        say("again"),
    ]);
    let mut fx = Fixture::start(vec![script.clone()]).await;
    fx.add_script(script.clone());
    let mut c = fx.client("tui", true).await;
    new_session(&mut c, &fx.ws, Some("s")).await;
    attach(&mut c, "s").await;
    send(&mut c, "s", "write a.txt").await;
    let q = question(&mut c).await;
    answer(
        &mut c,
        "s",
        &q.id,
        Answer::AllowAlways {
            rules: vec!["write_file(a.txt)".into()],
        },
    )
    .await
    .unwrap();
    assert_eq!(turn_end(&mut c).await, EndReason::Done);
    drop(c);

    fx.restart().await;
    let mut c = fx.client("tui", true).await;
    let listed = c.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].state, State::Saved);
    assert_eq!(listed[0].name, "s");

    // Attach resumes it and replays the whole conversation.
    let info = attach(&mut c, "s").await;
    assert_eq!(info.state, State::Idle);
    let (_, replayed) = until(&mut c, |e, replay| {
        (replay && matches!(e, SessionEvent::TurnEnd(_))).then_some(())
    })
    .await;
    assert!(replayed.contains(&SessionEvent::UserMessage {
        text: "write a.txt".into()
    }));
    assert!(replayed.contains(&SessionEvent::TextDelta {
        text: "written".into()
    }));
    // The model gets the old history with the next message.
    send(&mut c, "s", "and now?").await;
    assert_eq!(turn_end(&mut c).await, EndReason::Done);
    let texts = user_texts(&script);
    assert_eq!(texts.last().unwrap(), &["write a.txt", "and now?"]);
    // The session rule came back: the notice is in the log once, from
    // the first run, and the rule is in the history file.
    let history = fs::read_to_string(
        store::list(&fx.sessions_dir())[0]
            .0
            .dir()
            .join("history.jsonl"),
    )
    .unwrap();
    assert!(history.contains("allow write_file(a.txt)"), "{history}");
    // No API key in any file.
    for (s, _) in store::list(&fx.sessions_dir()) {
        for file in ["session.json", "history.jsonl", "events.jsonl"] {
            let text = fs::read_to_string(s.dir().join(file)).unwrap();
            assert!(!text.contains("\"k\""), "{file}: {text}");
        }
    }
    fx.stop().await;
}

#[tokio::test]
async fn a_stop_interrupts_the_turn_and_its_question() {
    let script = Script::new(vec![write_call("c1", "a.txt"), say("after")]);
    let mut fx = Fixture::start(vec![script.clone()]).await;
    fx.add_script(script.clone());
    let mut c = fx.client("tui", true).await;
    new_session(&mut c, &fx.ws, Some("s")).await;
    attach(&mut c, "s").await;
    send(&mut c, "s", "write").await;
    question(&mut c).await;
    // Busy: the server waits for no one but reports it.
    let Payload::Status(status) = c.request(Request::Status).await.unwrap() else {
        panic!("not status");
    };
    assert_eq!((status.sessions, status.running, status.busy), (1, 1, 1));
    drop(c);
    fx.restart().await;

    let mut c = fx.client("tui", true).await;
    let info = attach(&mut c, "s").await;
    assert_eq!(info.state, State::Idle);
    assert!(info.question.is_none());
    send(&mut c, "s", "go on").await;
    assert_eq!(turn_end(&mut c).await, EndReason::Done);
    // The call that was waiting got a result before the next request.
    let requests = script.requests.lock().unwrap().clone();
    let last = requests.last().unwrap();
    let result = last
        .messages
        .iter()
        .flat_map(|m| &m.content)
        .find_map(|b| match b {
            crate::provider::Block::ToolResult {
                call_id, content, ..
            } if call_id == "c1" => Some(content.clone()),
            _ => None,
        });
    assert_eq!(result.as_deref(), Some("not run: the server stopped"));
    assert!(!fx.ws.join("a.txt").exists());
    fx.stop().await;
}

#[tokio::test]
async fn kill_deletes_and_saved_sessions_resume_on_request() {
    let mut fx = Fixture::start(vec![]).await;
    let mut c = fx.client("cli", false).await;
    new_session(&mut c, &fx.ws, Some("a")).await;
    new_session(&mut c, &fx.ws, Some("b")).await;
    assert_eq!(store::list(&fx.sessions_dir()).len(), 2);
    drop(c);
    fx.restart().await;
    let mut c = fx.client("cli", false).await;

    // A saved session can be renamed and killed without resuming it.
    c.request(Request::Rename {
        session: "a".into(),
        name: "c".into(),
    })
    .await
    .unwrap();
    c.request(Request::Kill {
        session: "b".into(),
    })
    .await
    .unwrap();
    let saved = store::list(&fx.sessions_dir());
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].1.name, "c");
    let answered = answer(&mut c, "c", "q1", Answer::Allow).await;
    assert!(matches!(answered, Err(Error::Failed(m)) if m == "c has no open question"));
    assert_eq!(c.list().await.unwrap()[0].state, State::Saved);

    // A model removed from the configuration: resume fails until another
    // is given.
    fs::write(
        &fx.config,
        CONFIG
            .replace("[providers.fake.models.one]", "[providers.fake.models.uno]")
            .replace(
                r#"default_model = "fake/one""#,
                r#"default_model = "fake/uno""#,
            ),
    )
    .unwrap();
    assert_eq!(notice(&mut c).await, Notice::ConfigReloaded);
    let failed = c
        .request(Request::Attach {
            session: "c".into(),
        })
        .await;
    assert!(
        matches!(&failed, Err(Error::Usage(m)) if m.contains("--model")),
        "{failed:?}"
    );
    c.request(Request::SetModel {
        session: "c".into(),
        model: "two".into(),
    })
    .await
    .unwrap();
    let info = c.list().await.unwrap().remove(0);
    assert_eq!((info.state, info.model.as_str()), (State::Idle, "fake/two"));
    c.request(Request::Kill {
        session: "c".into(),
    })
    .await
    .unwrap();
    assert!(store::list(&fx.sessions_dir()).is_empty());
    fx.stop().await;
}

#[tokio::test]
async fn a_missing_workspace_keeps_it_saved() {
    let mut fx = Fixture::start(vec![]).await;
    let other = fx.tmp.path().join("gone");
    fs::create_dir_all(&other).unwrap();
    let mut c = fx.client("cli", false).await;
    new_session(&mut c, &other, Some("g")).await;
    drop(c);
    fx.restart().await;
    fs::remove_dir_all(&other).unwrap();
    let mut c = fx.client("cli", false).await;
    let failed = c
        .request(Request::Send {
            session: "g".into(),
            text: "hi".into(),
        })
        .await;
    assert!(
        matches!(&failed, Err(Error::Failed(m)) if m.contains("not a directory") && m.contains("stays saved")),
        "{failed:?}"
    );
    assert_eq!(c.list().await.unwrap()[0].state, State::Saved);
    fx.stop().await;
}

#[tokio::test]
async fn export_and_import() {
    let fx = Fixture::start(vec![Script::new(vec![say("hello")])]).await;
    let mut c = fx.client("tui", true).await;
    new_session(&mut c, &fx.ws, Some("s")).await;
    attach(&mut c, "s").await;
    send(&mut c, "s", "hi").await;
    turn_end(&mut c).await;
    let file = fx.tmp.path().join("s.clai.json");
    c.request(Request::Export {
        session: "s".into(),
        path: file.clone(),
    })
    .await
    .unwrap();
    let relative = c
        .request(Request::Export {
            session: "s".into(),
            path: PathBuf::from("x.json"),
        })
        .await;
    assert!(matches!(relative, Err(Error::Usage(_))));

    let imported = c
        .request(Request::Import {
            path: file.clone(),
            name: None,
        })
        .await
        .unwrap();
    let Payload::Session { session } = imported else {
        panic!("not a session");
    };
    assert_eq!(
        (session.name.as_str(), session.state),
        ("s-2", State::Saved)
    );
    let taken = c
        .request(Request::Import {
            path: file,
            name: Some("s".into()),
        })
        .await;
    assert!(matches!(taken, Err(Error::Failed(m)) if m == "the name s is taken"));

    // The copy has the same conversation.
    attach(&mut c, "s-2").await;
    let (_, replayed) = until(&mut c, |e, replay| {
        (replay && matches!(e, SessionEvent::TurnEnd(_))).then_some(())
    })
    .await;
    assert!(replayed.contains(&SessionEvent::TextDelta {
        text: "hello".into()
    }));
    fx.stop().await;
}

#[tokio::test]
async fn idle_exit_with_saved_sessions() {
    let fx = Fixture::start_with(vec![], Duration::from_millis(200)).await;
    let mut c = fx.client("cli", false).await;
    new_session(&mut c, &fx.ws, Some("s")).await;
    drop(c);
    // An idle session does not keep the server running.
    tokio::time::timeout(WAIT, fx.server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(store::list(&fx.dir.join("sessions")).len(), 1);
}

/// An answer that reports its tokens.
fn say_counted(text: &str, input_tokens: u64, output_tokens: u64) -> Step {
    Step::Events(vec![
        Ok(Event::TextDelta { text: text.into() }),
        Ok(Event::Usage {
            input_tokens,
            output_tokens,
        }),
        Ok(Event::Done {
            stop_reason: StopReason::EndTurn,
        }),
    ])
}

fn totals(input_tokens: u64, output_tokens: u64) -> TokenTotals {
    TokenTotals {
        input_tokens,
        output_tokens,
    }
}

/// The directory of the only saved session.
fn saved_dir(fx: &Fixture) -> PathBuf {
    fs::read_dir(fx.sessions_dir())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
}

/// The saved log of a session.
fn saved_log(dir: &Path) -> Vec<SessionEvent> {
    fs::read_to_string(dir.join("events.jsonl"))
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[tokio::test]
async fn token_totals_are_counted_and_survive_a_restart() {
    let script = Script::new(vec![say_counted("a", 100, 10), say_counted("b", 200, 20)]);
    let mut fx = Fixture::start(vec![script]).await;
    let mut c = fx.client("tui", true).await;
    let created = new_session(&mut c, &fx.ws, Some("s")).await;
    assert_eq!(created.usage, totals(0, 0));
    attach(&mut c, "s").await;
    send(&mut c, "s", "one").await;
    turn_end(&mut c).await;
    send(&mut c, "s", "two").await;
    turn_end(&mut c).await;
    let listed = c.list().await.unwrap();
    assert_eq!(listed[0].usage, totals(300, 30));
    // The list carries the turns and the time of the last change.
    assert_eq!(created.turns, 0);
    assert_eq!(listed[0].turns, 2);
    assert!(listed[0].updated >= created.created && listed[0].updated > 0);
    drop(c);

    // Written as it goes, not only at the stop.
    let dir = saved_dir(&fx);
    let meta = Store::open(dir).read_meta().unwrap();
    assert_eq!(meta.usage, totals(300, 30));

    fx.restart().await;
    let mut c = fx.client("tui", true).await;
    // A saved session reports them without being loaded.
    let listed = c.list().await.unwrap();
    assert_eq!(listed[0].state, State::Saved);
    assert_eq!(listed[0].usage, totals(300, 30));
    assert_eq!(listed[0].turns, 2);
    // Resuming sums the log once, not on top of the saved totals.
    assert_eq!(attach(&mut c, "s").await.usage, totals(300, 30));
    fx.stop().await;
}

#[tokio::test]
async fn token_totals_are_summed_from_the_log_on_resume() {
    let script = Script::new(vec![say_counted("a", 100, 10)]);
    let mut fx = Fixture::start(vec![script]).await;
    let mut c = fx.client("tui", true).await;
    new_session(&mut c, &fx.ws, Some("s")).await;
    attach(&mut c, "s").await;
    send(&mut c, "s", "one").await;
    turn_end(&mut c).await;
    drop(c);
    fx.stop.cancel();
    tokio::time::timeout(WAIT, &mut fx.server)
        .await
        .unwrap()
        .unwrap()
        .unwrap();

    // As an older `clai` left it: no totals in the meta.
    let dir = saved_dir(&fx);
    let path = dir.join("session.json");
    let mut meta: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    meta.as_object_mut().unwrap().remove("usage");
    fs::write(&path, meta.to_string()).unwrap();

    let (stop, server) = launch(&fx.dir, &fx.config, &fx.scripts, WAIT).await;
    fx.stop = stop;
    fx.server = server;
    let mut c = fx.client("tui", true).await;
    assert_eq!(c.list().await.unwrap()[0].usage, totals(0, 0));
    assert_eq!(attach(&mut c, "s").await.usage, totals(100, 10));
    fx.stop().await;
}

#[tokio::test]
async fn a_cancelled_request_counts_what_was_reported() {
    let script = Script::new(vec![Step::UsageThenHang(70, 7), say("no usage")]);
    let fx = Fixture::start(vec![script]).await;
    let mut c = fx.client("tui", true).await;
    new_session(&mut c, &fx.ws, Some("s")).await;
    attach(&mut c, "s").await;
    send(&mut c, "s", "one").await;
    // The text comes after the usage, so the request has reported it by then.
    until(&mut c, |e, _| {
        matches!(e, SessionEvent::TextDelta { .. }).then_some(())
    })
    .await;
    c.request(Request::Cancel {
        session: "s".into(),
    })
    .await
    .unwrap();
    assert_eq!(turn_end(&mut c).await, EndReason::Cancelled);
    assert_eq!(c.list().await.unwrap()[0].usage, totals(70, 7));

    // A provider that reports nothing adds nothing.
    send(&mut c, "s", "two").await;
    turn_end(&mut c).await;
    assert_eq!(c.list().await.unwrap()[0].usage, totals(70, 7));
    fx.stop().await;
}

#[tokio::test]
async fn export_and_import_keep_the_totals() {
    let script = Script::new(vec![say_counted("a", 100, 10)]);
    let fx = Fixture::start(vec![script]).await;
    let mut c = fx.client("tui", true).await;
    new_session(&mut c, &fx.ws, Some("s")).await;
    attach(&mut c, "s").await;
    send(&mut c, "s", "one").await;
    turn_end(&mut c).await;
    let file = fx.tmp.path().join("s.json");
    c.request(Request::Export {
        session: "s".into(),
        path: file.clone(),
    })
    .await
    .unwrap();
    let imported = c
        .request(Request::Import {
            path: file,
            name: Some("copy".into()),
        })
        .await
        .unwrap();
    let Payload::Session { session } = imported else {
        panic!("not a session");
    };
    assert_eq!(session.usage, totals(100, 10));
    assert_eq!(attach(&mut c, "copy").await.usage, totals(100, 10));
    fx.stop().await;
}

#[tokio::test]
async fn a_model_change_says_where_it_came_from() {
    let fx = Fixture::start(vec![Script::new(vec![])]).await;
    fx.add_script(Script::new(vec![]));
    let mut c = fx.client("tui", true).await;
    new_session(&mut c, &fx.ws, Some("s")).await;
    attach(&mut c, "s").await;
    c.request(Request::SetModel {
        session: "s".into(),
        model: "two".into(),
    })
    .await
    .unwrap();
    let (change, _) = until(&mut c, |e, _| {
        matches!(e, SessionEvent::ModelChanged { .. }).then(|| e.clone())
    })
    .await;
    assert_eq!(
        change,
        SessionEvent::ModelChanged {
            from: "fake/one".into(),
            model: "fake/two".into(),
        }
    );
    fx.stop().await;
}

#[tokio::test]
async fn setting_the_same_model_publishes_nothing() {
    let script = Script::new(vec![say("a")]);
    let fx = Fixture::start(vec![script.clone()]).await;
    // The provider is made again: it may come from a reloaded configuration.
    let applied = Script::new(vec![say("b")]);
    fx.add_script(applied.clone());
    let mut c = fx.client("tui", true).await;
    new_session(&mut c, &fx.ws, Some("s")).await;
    attach(&mut c, "s").await;
    c.request(Request::SetModel {
        session: "s".into(),
        model: "fake/one".into(),
    })
    .await
    .unwrap();
    c.request(Request::Rename {
        session: "s".into(),
        name: "t".into(),
    })
    .await
    .unwrap();
    let (_, before) = until(&mut c, |e, _| {
        matches!(e, SessionEvent::Renamed { .. }).then_some(())
    })
    .await;
    assert!(
        !before
            .iter()
            .any(|e| matches!(e, SessionEvent::ModelChanged { .. })),
        "{before:?}"
    );
    send(&mut c, "t", "go").await;
    turn_end(&mut c).await;
    assert!(script.models.lock().unwrap().is_empty());
    assert_eq!(*applied.models.lock().unwrap(), ["model-one"]);
    fx.stop().await;
}

#[tokio::test]
async fn changing_the_model_of_a_saved_session_is_logged() {
    let mut fx = Fixture::start(vec![Script::new(vec![])]).await;
    let mut c = fx.client("tui", true).await;
    new_session(&mut c, &fx.ws, Some("s")).await;
    drop(c);
    fx.restart().await;
    let mut c = fx.client("tui", true).await;
    assert_eq!(c.list().await.unwrap()[0].state, State::Saved);
    c.request(Request::SetModel {
        session: "s".into(),
        model: "fake/two".into(),
    })
    .await
    .unwrap();
    let changed = SessionEvent::ModelChanged {
        from: "fake/one".into(),
        model: "fake/two".into(),
    };
    assert!(saved_log(&saved_dir(&fx)).contains(&changed));
    drop(c);

    // Also after another restart, and for a client that attaches later.
    fx.restart().await;
    let mut c = fx.client("tui", true).await;
    assert_eq!(c.list().await.unwrap()[0].model, "fake/two");
    attach(&mut c, "s").await;
    let (replayed, _) = until(&mut c, |e, replay| {
        (replay && matches!(e, SessionEvent::ModelChanged { .. })).then(|| e.clone())
    })
    .await;
    assert_eq!(replayed, changed);
    fx.stop().await;
}

#[tokio::test]
async fn resuming_on_the_same_model_logs_nothing() {
    let mut fx = Fixture::start(vec![Script::new(vec![])]).await;
    let mut c = fx.client("tui", true).await;
    new_session(&mut c, &fx.ws, Some("s")).await;
    drop(c);
    fx.restart().await;
    let mut c = fx.client("tui", true).await;
    c.request(Request::SetModel {
        session: "s".into(),
        model: "fake/one".into(),
    })
    .await
    .unwrap();
    let dir = saved_dir(&fx);
    let log = if dir.join("events.jsonl").exists() {
        saved_log(&dir)
    } else {
        Vec::new()
    };
    assert!(
        !log.iter()
            .any(|e| matches!(e, SessionEvent::ModelChanged { .. })),
        "{log:?}"
    );
    fx.stop().await;
}

async fn models(client: &mut Client) -> (Vec<ModelInfo>, PathBuf) {
    match client.request(Request::Models).await.unwrap() {
        Payload::Models { models, config } => (models, config),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn the_models_request_lists_the_servers_models() {
    let fx = Fixture::start(vec![]).await;
    let mut c = fx.client("tui", true).await;
    let (list, config) = models(&mut c).await;
    assert_eq!(config, fx.config);
    assert_eq!(
        list[0],
        ModelInfo {
            model: "fake/one".into(),
            provider: "fake".into(),
            name: "one".into(),
            id: "model-one".into(),
            context_window: 100_000,
            default: true,
            key: "config".into(),
        }
    );
    assert_eq!(list[1].model, "fake/two");
    assert!(!list[1].default);

    // After a reload it is the new configuration.
    let three = CONFIG.replace(
        "[providers.fake.models.two]",
        "[providers.fake.models.three]",
    );
    fs::write(&fx.config, three).unwrap();
    assert_eq!(notice(&mut c).await, Notice::ConfigReloaded);
    let (list, _) = models(&mut c).await;
    let names: Vec<&str> = list.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, ["one", "three"]);
    fx.stop().await;
}

#[tokio::test]
async fn an_older_client_gets_the_version_error() {
    let fx = Fixture::start(vec![]).await;
    let mut stream = UnixStream::connect(socket_path(&fx.dir)).await.unwrap();
    stream
        .write_all(b"{\"type\":\"hello\",\"version\":2,\"kind\":\"cli\",\"name\":\"x\",\"answers\":false}\n")
        .await
        .unwrap();
    let mut reader = BufReader::new(stream);
    let reply: ServerMessage = protocol::read(&mut reader).await.unwrap().unwrap();
    let ServerMessage::Error { message } = reply else {
        panic!("{reply:?}");
    };
    assert!(
        message.contains(&format!("version {}", protocol::VERSION)),
        "{message}"
    );
    assert!(message.contains("clai server stop"), "{message}");
    fx.stop().await;
}

/// `e` in the TUI: the call is denied with the message, the question closes
/// and the turn goes on to the model's next answer.
#[tokio::test]
async fn deny_with_a_message_closes_the_question_and_goes_on() {
    let script = Script::new(vec![write_call("c1", "a.txt"), say("ok, not writing")]);
    let fx = Fixture::start(vec![script]).await;
    let mut tui = fx.client("tui pts/3", true).await;
    new_session(&mut tui, &fx.ws, Some("s")).await;
    attach(&mut tui, "s").await;
    send(&mut tui, "s", "write a.txt").await;
    let q = question(&mut tui).await;
    let text = "use b.txt instead".to_owned();
    answer(
        &mut tui,
        "s",
        &q.id,
        Answer::DenyMessage { text: text.clone() },
    )
    .await
    .unwrap();
    let (closed, _) = until(&mut tui, |e, _| match e {
        SessionEvent::QuestionClosed { closed, .. } => Some(closed.clone()),
        _ => None,
    })
    .await;
    assert_eq!(
        closed,
        Closed::Answered {
            answer: Answer::DenyMessage { text },
            by: "tui pts/3".into()
        }
    );
    until(&mut tui, |e, _| match e {
        SessionEvent::TextDone { text } if text == "ok, not writing" => Some(()),
        _ => None,
    })
    .await;
    fx.stop().await;
}

async fn session_status(client: &mut Client, name: &str) -> crate::protocol::SessionStatus {
    match client
        .request(Request::SessionStatus {
            session: name.into(),
        })
        .await
        .unwrap()
    {
        Payload::SessionStatus { status } => *status,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn session_status_of_a_running_and_a_saved_session() {
    let script = Script::new(vec![
        write_call("c1", "a.txt"),
        say_counted("done", 100, 10),
    ]);
    let mut fx = Fixture::start(vec![script]).await;
    let mut c = fx.client("tui", true).await;
    new_session(&mut c, &fx.ws, Some("s")).await;
    attach(&mut c, "s").await;
    send(&mut c, "s", "write a.txt").await;
    let q = question(&mut c).await;

    // While it waits: the turn runs, the client is listed.
    let status = session_status(&mut c, "s").await;
    assert_eq!(status.session.state, State::Waiting);
    assert!(status.stats.turn.is_some());
    assert_eq!(
        status.clients,
        [crate::protocol::ClientInfo {
            name: "tui".into(),
            answers: true
        }]
    );
    let limits = status.limits.unwrap();
    assert_eq!((limits.max_requests, limits.max_tool_calls), (50, 120));
    let model = status.model.unwrap();
    assert_eq!(model.info.model, "fake/one");
    assert_eq!(model.api, "openai-compatible");
    assert_eq!(model.info.key, "config");
    assert!(status.dir.is_some());
    assert!(status.size.unwrap() > 0);
    assert_eq!(status.server.running, 1);

    answer(&mut c, "s", &q.id, Answer::Allow).await.unwrap();
    assert_eq!(turn_end(&mut c).await, EndReason::Done);
    let status = session_status(&mut c, "s").await;
    let stats = &status.stats;
    assert!(stats.turn.is_none());
    assert_eq!(stats.turns.done, 1);
    // The first request reported no usage; the end of the turn counts it.
    assert_eq!(stats.requests, 2);
    assert_eq!(stats.tokens, totals(100, 10));
    assert_eq!(stats.tools["write_file"].done, 1);
    assert_eq!(stats.answers.allow, 1);
    assert_eq!(stats.by["tui"], 1);
    assert!(stats.last_active.is_some());
    let (_, exact) = stats.working_time(status.now);
    assert!(exact);
    let running = status.stats.clone();
    drop(c);

    // Saved: counted from the log, with the same result, and not resumed.
    fx.restart().await;
    let mut c = fx.client("tui", true).await;
    let status = session_status(&mut c, "s").await;
    assert_eq!(status.session.state, State::Saved);
    assert!(status.clients.is_empty());
    assert_eq!(status.stats.tools, running.tools);
    assert_eq!(status.stats.answers, running.answers);
    assert_eq!(status.stats.turns, running.turns);
    assert_eq!(status.stats.tokens, running.tokens);
    assert_eq!(status.session.usage, totals(100, 10));
    assert_eq!(c.list().await.unwrap()[0].state, State::Saved);
    // Every saved event has its time.
    let text = fs::read_to_string(saved_dir(&fx).join("events.jsonl")).unwrap();
    assert!(text.lines().all(|line| line.contains("\"at\":")), "{text}");

    let error = c
        .request(Request::SessionStatus {
            session: "nope".into(),
        })
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("no session named nope"),
        "{error}"
    );
    fx.stop().await;
}

#[tokio::test]
async fn session_status_of_a_model_no_longer_configured() {
    let fx = Fixture::start(vec![Script::new(Vec::new())]).await;
    let mut c = fx.client("tui", true).await;
    c.request(Request::New {
        workspace: fx.ws.clone(),
        name: Some("s".into()),
        model: Some("fake/two".into()),
    })
    .await
    .unwrap();
    let without_two = CONFIG.split("[providers.fake.models.two]").next().unwrap();
    fs::write(&fx.config, without_two).unwrap();
    assert!(matches!(notice(&mut c).await, Notice::ConfigReloaded));
    let status = session_status(&mut c, "s").await;
    assert_eq!(status.session.model, "fake/two");
    assert!(status.model.is_none());
    assert!(status.limits.is_some());
    fx.stop().await;
}

#[tokio::test]
async fn a_question_from_the_model_waits_for_any_client() {
    let arguments = json!({"questions": [{"question": "Which database?", "header": "Database",
        "options": [{"label": "SQLite"}, {"label": "Postgres"}]}]});
    let ask = Step::Events(vec![
        Ok(Event::ToolCallStart {
            id: "c1".into(),
            name: "ask_user".into(),
        }),
        Ok(Event::ToolCallEnd {
            id: "c1".into(),
            arguments,
        }),
        Ok(Event::Done {
            stop_reason: StopReason::ToolUse,
        }),
    ]);
    let script = Script::new(vec![ask, say("SQLite then")]);
    let fx = Fixture::start(vec![script.clone()]).await;
    let mut tui = fx.client("tui pts/3", true).await;
    let mut chat = fx.client("clai chat", true).await;
    new_session(&mut tui, &fx.ws, Some("s")).await;
    attach(&mut tui, "s").await;
    attach(&mut chat, "s").await;
    send(&mut tui, "s", "build the cache").await;
    let q = question(&mut tui).await;
    assert_eq!(q.summary, "ask_user: Database");
    assert!(matches!(q.kind, crate::agent::QuestionKind::AskUser(_)));
    assert_eq!(question(&mut chat).await.id, q.id);
    let info = chat.list().await.unwrap().remove(0);
    assert_eq!(info.state, State::Waiting);

    // A wrong reply keeps it open; the first valid one counts.
    let reply = |selected: Vec<usize>| Answer::Reply {
        answers: vec![crate::tools::UserAnswer {
            selected,
            text: None,
        }],
    };
    let wrong = answer(&mut chat, "s", &q.id, reply(vec![5])).await;
    assert!(
        matches!(&wrong, Err(Error::Failed(m)) if m.contains("option 5 does not exist")),
        "{wrong:?}"
    );
    answer(&mut chat, "s", &q.id, reply(vec![0])).await.unwrap();
    let late = answer(&mut tui, "s", &q.id, Answer::Decline).await;
    assert!(matches!(&late, Err(Error::Failed(m)) if m.contains("already answered")));
    assert_eq!(turn_end(&mut tui).await, EndReason::Done);
    let requests = script.requests.lock().unwrap().clone();
    assert!(requests[0].tools.iter().any(|t| t.name == "ask_user"));
    let result = requests[1]
        .messages
        .iter()
        .flat_map(|m| &m.content)
        .find_map(|b| match b {
            crate::provider::Block::ToolResult { content, .. } => Some(content.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        result,
        "The user answered:\n\n1. Which database?\n   SQLite"
    );
    fx.stop().await;
}
