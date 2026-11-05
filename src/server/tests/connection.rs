//! `client::Connection` against the in-process server.

use tokio::sync::oneshot;

use super::*;
use crate::client::Connection;
use crate::error::Result as ClaiResult;

/// A client connected and attached, its socket moved into a connection.
async fn connected(fx: &Fixture, name: &str, session: &str) -> Connection {
    let mut client = fx.client(name, true).await;
    attach(&mut client, session).await;
    Connection::spawn(client)
}

async fn response(rx: oneshot::Receiver<ClaiResult<Payload>>) -> ClaiResult<Payload> {
    tokio::time::timeout(WAIT, rx)
        .await
        .expect("no response in time")
        .expect("connection lost")
}

/// Reads live events until `f` picks one.
async fn until_event<T>(
    connection: &mut Connection,
    mut f: impl FnMut(&SessionEvent) -> Option<T>,
) -> T {
    tokio::time::timeout(WAIT, async {
        loop {
            match connection.next().await {
                Some(ServerMessage::Event {
                    event,
                    replay: false,
                    ..
                }) => {
                    if let Some(found) = f(&event) {
                        return found;
                    }
                }
                Some(_) => {}
                None => panic!("connection lost"),
            }
        }
    })
    .await
    .expect("no such event in time")
}

#[tokio::test]
async fn requests_are_answered_while_events_stream() {
    let fx = Fixture::start(vec![Script::new(vec![Step::Hang])]).await;
    let mut c = fx.client("cli", false).await;
    new_session(&mut c, &fx.ws, Some("a")).await;
    let mut connection = connected(&fx, "tui pts/1", "a").await;
    let sent = connection.request(Request::Send {
        session: "a".into(),
        text: "hi".into(),
    });
    assert_eq!(response(sent).await.unwrap(), Payload::Done);
    until_event(&mut connection, |e| match e {
        SessionEvent::UserMessage { text } => Some(text.clone()),
        _ => None,
    })
    .await;
    // The model hangs: a request still gets its answer.
    let listed = connection.request(Request::List);
    match response(listed).await.unwrap() {
        Payload::Sessions { sessions } => assert_eq!(sessions[0].state, State::Working),
        other => panic!("{other:?}"),
    }
    let cancelled = connection.request(Request::Cancel {
        session: "a".into(),
    });
    response(cancelled).await.unwrap();
    let end = until_event(&mut connection, |e| match e {
        SessionEvent::TurnEnd(end) => Some(end.reason),
        _ => None,
    })
    .await;
    assert_eq!(end, EndReason::Cancelled);
    fx.stop().await;
}

#[tokio::test]
async fn responses_come_back_by_id() {
    let fx = Fixture::start(vec![]).await;
    let mut c = fx.client("cli", false).await;
    new_session(&mut c, &fx.ws, Some("a")).await;
    let mut connection = connected(&fx, "tui pts/1", "a").await;
    let wrong = connection.request(Request::Kill {
        session: "nope".into(),
    });
    let right = connection.request(Request::Status);
    assert!(matches!(response(right).await, Ok(Payload::Status(_))));
    assert!(response(wrong).await.is_err());
    fx.stop().await;
}

#[tokio::test]
async fn answering_a_question_through_the_connection() {
    let fx = Fixture::start(vec![Script::new(vec![
        write_call("c1", "a.txt"),
        say("done"),
    ])])
    .await;
    let mut c = fx.client("cli", false).await;
    new_session(&mut c, &fx.ws, Some("a")).await;
    let mut connection = connected(&fx, "tui pts/1", "a").await;
    let sent = connection.request(Request::Send {
        session: "a".into(),
        text: "write".into(),
    });
    response(sent).await.unwrap();
    let id = until_event(&mut connection, |e| match e {
        SessionEvent::Question(q) => Some(q.id.clone()),
        _ => None,
    })
    .await;
    let answered = connection.request(Request::Answer {
        session: "a".into(),
        question: id.clone(),
        answer: Answer::Allow,
    });
    response(answered).await.unwrap();
    let by = until_event(&mut connection, |e| match e {
        SessionEvent::QuestionClosed {
            closed: Closed::Answered { by, .. },
            ..
        } => Some(by.clone()),
        _ => None,
    })
    .await;
    assert_eq!(by, "tui pts/1");
    until_event(&mut connection, |e| match e {
        SessionEvent::TurnEnd(_) => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(fs::read_to_string(fx.ws.join("a.txt")).unwrap(), "hello\n");
    fx.stop().await;
}

#[tokio::test]
async fn a_second_client_answers_first() {
    let fx = Fixture::start(vec![Script::new(vec![
        write_call("c1", "a.txt"),
        say("done"),
    ])])
    .await;
    let mut c = fx.client("cli", false).await;
    new_session(&mut c, &fx.ws, Some("a")).await;
    let mut slow = connected(&fx, "tui pts/1", "a").await;
    let mut fast = connected(&fx, "tui pts/2", "a").await;
    let sent = slow.request(Request::Send {
        session: "a".into(),
        text: "write".into(),
    });
    response(sent).await.unwrap();
    let question = |e: &SessionEvent| match e {
        SessionEvent::Question(q) => Some(q.id.clone()),
        _ => None,
    };
    let id = until_event(&mut slow, question).await;
    until_event(&mut fast, question).await;
    let first = fast.request(Request::Answer {
        session: "a".into(),
        question: id.clone(),
        answer: Answer::Allow,
    });
    response(first).await.unwrap();
    let by = until_event(&mut slow, |e| match e {
        SessionEvent::QuestionClosed {
            closed: Closed::Answered { by, .. },
            ..
        } => Some(by.clone()),
        _ => None,
    })
    .await;
    assert_eq!(by, "tui pts/2");
    // Too late: the error comes back as a response, the question is gone.
    let late = slow.request(Request::Answer {
        session: "a".into(),
        question: id,
        answer: Answer::Deny,
    });
    assert!(response(late).await.is_err());
    fx.stop().await;
}

#[tokio::test]
async fn a_kill_reaches_the_attached_client() {
    let fx = Fixture::start(vec![]).await;
    let mut c = fx.client("cli", false).await;
    new_session(&mut c, &fx.ws, Some("a")).await;
    let mut connection = connected(&fx, "tui pts/1", "a").await;
    let killed = connection.request(Request::Kill {
        session: "a".into(),
    });
    let reason = until_event(&mut connection, |e| match e {
        SessionEvent::Ended { reason } => Some(reason.clone()),
        _ => None,
    })
    .await;
    assert_eq!(reason, "killed");
    response(killed).await.unwrap();
    fx.stop().await;
}

/// That the socket then closes needs a real process: tasks of the in-process
/// server live as long as the test's runtime. See the next test for that end.
#[tokio::test]
async fn a_server_stop_says_so_and_ends_the_session() {
    let fx = Fixture::start(vec![]).await;
    let mut c = fx.client("cli", false).await;
    new_session(&mut c, &fx.ws, Some("a")).await;
    let mut connection = connected(&fx, "tui pts/1", "a").await;
    let stopper = tokio::spawn(fx.stop());
    let reason = until_event(&mut connection, |e| match e {
        SessionEvent::Ended { reason } => Some(reason.clone()),
        _ => None,
    })
    .await;
    assert_eq!(reason, "server stopped");
    stopper.await.unwrap();
}

#[tokio::test]
async fn pending_requests_fail_when_the_connection_ends() {
    let tmp = tempfile::tempdir().unwrap();
    let socket = tmp.path().join("fake.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    // A server that says hello, takes one request, and goes away silently.
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        let mut reader = BufReader::new(read);
        let _: Option<ClientMessage> = protocol::read(&mut reader).await.unwrap();
        let hello = ServerMessage::Hello(crate::protocol::ServerHello {
            version: protocol::VERSION,
            pid: 1,
        });
        protocol::write(&mut write, &hello).await.unwrap();
        let _: Option<ClientMessage> = protocol::read(&mut reader).await.unwrap();
    });
    let dir = tmp.path().to_path_buf();
    fs::rename(&socket, socket_path(&dir)).unwrap();
    let client = Client::connect(&Connect {
        dir,
        start: false,
        config_file: None,
        kind: ClientKind::Tui,
        name: "tui".into(),
        answers: true,
    })
    .await
    .unwrap();
    let mut connection = Connection::spawn(client);
    let pending = connection.request(Request::List);
    let result = tokio::time::timeout(WAIT, pending)
        .await
        .expect("must not hang");
    assert!(result.is_err());
    assert!(connection.next().await.is_none());
    server.await.unwrap();
}
