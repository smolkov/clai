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
use crate::protocol::ClientKind;
use crate::provider::{Event, EventStream, ProviderError, Request as ProviderRequest, StopReason};

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
    Panic,
}

#[derive(Default)]
struct Script {
    steps: Mutex<VecDeque<Step>>,
    models: Mutex<Vec<String>>,
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
        match self.steps.lock().unwrap().pop_front() {
            Some(Step::Events(events)) => stream::iter(events).boxed(),
            Some(Step::Hang) => stream::once(async move {
                cancel.cancelled().await;
                Err(ProviderError::cancelled())
            })
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
        let options = options(&dir, &config, &scripts, idle);
        let stop = options.stop.clone();
        let server = tokio::spawn(serve(options));
        let socket = socket_path(&dir);
        let deadline = Instant::now() + WAIT;
        while UnixStream::connect(&socket).await.is_err() {
            assert!(!server.is_finished(), "server ended early");
            assert!(Instant::now() < deadline, "server did not start");
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
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

async fn turn_end(client: &mut Client) -> EndReason {
    until(client, |e, _| match e {
        SessionEvent::TurnEnd(end) => Some(end.reason),
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
    // Removed from the registry once it has ended.
    let deadline = Instant::now() + WAIT;
    while c.list().await.unwrap().len() != 1 {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    attach(&mut c, "good").await;
    send(&mut c, "good", "go").await;
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
        SessionEvent::ModelChanged { model } => Some(model.clone()),
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
    assert!(message.contains("version 1"), "{message}");
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
