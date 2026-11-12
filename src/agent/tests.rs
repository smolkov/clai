//! The agent loop against a scripted provider, with tools in a temporary
//! workspace.

use std::collections::VecDeque;
use std::fs;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use futures_util::future::{self, BoxFuture};
use futures_util::stream::{self, StreamExt};
use serde_json::{Value, json};

use crate::config::permissions::parse_for_test;
use crate::permissions::{SessionRules, Workspace};
use crate::provider::{Event, EventStream, Message};

use super::*;

const CONFIG: &str = r#"
[permissions]
run_command = "ask"

[permissions.allow]
run_command = ["echo", "sleep"]

[permissions.deny]
run_command = ["rm"]
"#;

/// One answer of the script.
enum Step {
    Events(Vec<Result<Event, ProviderError>>),
    /// These events, then nothing until the request is cancelled.
    Hang(Vec<Result<Event, ProviderError>>),
}

#[derive(Default)]
struct Script {
    steps: Mutex<VecDeque<Step>>,
    requests: Mutex<Vec<Request>>,
}

impl Provider for Script {
    fn stream(&self, request: Request, cancel: CancellationToken) -> EventStream {
        self.requests.lock().unwrap().push(request);
        match self.steps.lock().unwrap().pop_front() {
            Some(Step::Events(events)) => stream::iter(events).boxed(),
            Some(Step::Hang(events)) => stream::iter(events)
                .chain(stream::once(async move {
                    cancel.cancelled().await;
                    Err(ProviderError::cancelled())
                }))
                .boxed(),
            None => stream::iter([Err(ProviderError::protocol("script: no more answers"))]).boxed(),
        }
    }
}

/// What the test asker does when asked.
enum Act {
    Answer(Answer),
    NoOne,
    /// Cancels the turn and never answers.
    CancelTurn(CancellationToken),
}

#[derive(Default)]
struct TestAsker {
    acts: Mutex<VecDeque<Act>>,
    asked: Mutex<Vec<Question>>,
    rejected: Mutex<Vec<String>>,
}

impl TestAsker {
    fn new(acts: Vec<Act>) -> TestAsker {
        TestAsker {
            acts: Mutex::new(acts.into()),
            ..TestAsker::default()
        }
    }

    fn answers(answers: Vec<Answer>) -> TestAsker {
        TestAsker::new(answers.into_iter().map(Act::Answer).collect())
    }

    fn asked(&self) -> Vec<Question> {
        self.asked.lock().unwrap().clone()
    }

    fn rejected(&self) -> Vec<String> {
        self.rejected.lock().unwrap().clone()
    }
}

impl Asker for TestAsker {
    fn ask<'a>(&'a self, question: &'a Question) -> BoxFuture<'a, Reply> {
        self.asked.lock().unwrap().push(question.clone());
        let act = self.acts.lock().unwrap().pop_front();
        match act {
            Some(Act::Answer(answer)) => Box::pin(future::ready(Reply::Answer {
                answer,
                by: "test".into(),
            })),
            Some(Act::NoOne) => Box::pin(future::ready(Reply::NoOne)),
            Some(Act::CancelTurn(token)) => {
                token.cancel();
                Box::pin(future::pending())
            }
            None => panic!("unexpected question: {}", question.summary),
        }
    }

    fn reject(&self, _question: &Question, by: &str, message: &str) {
        assert_eq!(by, "test");
        self.rejected.lock().unwrap().push(message.to_owned());
    }
}

struct Fixture {
    _tmp: tempfile::TempDir,
    root: std::path::PathBuf,
    script: Arc<Script>,
    agent: Agent,
}

impl Fixture {
    fn new(steps: Vec<Step>) -> Fixture {
        Fixture::with_limits(steps, Limits::default())
    }

    fn with_limits(steps: Vec<Step>, limits: Limits) -> Fixture {
        Fixture::build(steps, limits, None)
    }

    /// With language servers (RFC 0019).
    fn with_lsp(steps: Vec<Step>, lsp: crate::tools::lsp::LspEnv) -> Fixture {
        Fixture::build(steps, Limits::default(), Some(Arc::new(lsp)))
    }

    fn build(
        steps: Vec<Step>,
        limits: Limits,
        lsp: Option<Arc<crate::tools::lsp::LspEnv>>,
    ) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("ws");
        fs::create_dir_all(&root).unwrap();
        let workspace = Workspace::new(&root, &[], None).unwrap();
        let root = workspace.root().to_path_buf();
        let script = Arc::new(Script {
            steps: Mutex::new(steps.into()),
            ..Script::default()
        });
        let system = system_prompt(&workspace, "2026-11-01");
        let agent = Agent::new(Setup {
            provider: script.clone(),
            model: Model {
                name: "m".into(),
                id: "model-1".into(),
                display_name: "Model One".into(),
                context_window: 200_000,
                max_output_tokens: Some(1000),
                temperature: None,
            },
            model_name: "test/m".into(),
            limits,
            env: Env {
                permissions: Arc::new(parse_for_test(CONFIG, None)),
                session: Arc::new(SessionRules::default()),
                workspace: Arc::new(workspace),
                lsp,
            },
            system,
            can_ask: true,
        });
        Fixture {
            _tmp: tmp,
            root,
            script,
            agent,
        }
    }

    fn write(&self, rel: &str, content: &str) {
        fs::write(self.root.join(rel), content).unwrap();
    }

    async fn turn(&mut self, message: &str, asker: &dyn Asker) -> (TurnEnd, Vec<SessionEvent>) {
        self.turn_with(message, asker, CancellationToken::new())
            .await
    }

    async fn turn_with(
        &mut self,
        message: &str,
        asker: &dyn Asker,
        cancel: CancellationToken,
    ) -> (TurnEnd, Vec<SessionEvent>) {
        let (sink, mut rx) = EventSink::channel();
        let end = self
            .agent
            .run_turn(message.into(), &sink, asker, cancel)
            .await;
        let mut events = Vec::new();
        while let Ok(event) = rx.try_recv() {
            events.push(event);
        }
        check_history(self.agent.history().messages());
        for request in self.script.requests.lock().unwrap().iter() {
            check_request(&request.messages);
        }
        (end, events)
    }

    fn messages(&self) -> Vec<Message> {
        self.agent.history().messages().to_vec()
    }

    fn requests(&self) -> Vec<Request> {
        self.script.requests.lock().unwrap().clone()
    }
}

/// Every tool call has exactly one result, in the next message, in order.
fn check_history(messages: &[Message]) {
    for (i, message) in messages.iter().enumerate() {
        let calls: Vec<&str> = message
            .content
            .iter()
            .filter_map(|b| match b {
                Block::ToolCall { id, .. } => Some(id.as_str()),
                _ => None,
            })
            .collect();
        if calls.is_empty() {
            continue;
        }
        assert_eq!(message.role, Role::Assistant);
        let next = messages
            .get(i + 1)
            .unwrap_or_else(|| panic!("calls {calls:?} have no results"));
        assert_eq!(next.role, Role::User);
        let results: Vec<&str> = next
            .content
            .iter()
            .filter_map(|b| match b {
                Block::ToolResult { call_id, .. } => Some(call_id.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(results, calls, "results of message {i}");
    }
}

/// A request never ends with calls that have no results.
fn check_request(messages: &[Message]) {
    check_history(messages);
    if let Some(last) = messages.last() {
        assert_eq!(last.role, Role::User, "a request ends with a user message");
    }
}

// Script helpers.

fn text(t: &str) -> Result<Event, ProviderError> {
    Ok(Event::TextDelta { text: t.into() })
}

fn call(id: &str, name: &str, arguments: Value) -> Vec<Result<Event, ProviderError>> {
    vec![
        Ok(Event::ToolCallStart {
            id: id.into(),
            name: name.into(),
        }),
        Ok(Event::ToolCallArgsDelta {
            id: id.into(),
            json: arguments.to_string(),
        }),
        Ok(Event::ToolCallEnd {
            id: id.into(),
            arguments,
        }),
    ]
}

fn done(stop_reason: StopReason) -> Vec<Result<Event, ProviderError>> {
    vec![
        Ok(Event::Usage {
            input_tokens: 100,
            output_tokens: 10,
        }),
        Ok(Event::Done { stop_reason }),
    ]
}

/// An answer with only text.
fn say(t: &str) -> Step {
    let mut events = vec![text(t)];
    events.extend(done(StopReason::EndTurn));
    Step::Events(events)
}

/// An answer with tool calls: `(id, tool, arguments)`.
fn calls(list: &[(&str, &str, Value)]) -> Step {
    let mut events = Vec::new();
    for (id, name, arguments) in list {
        events.extend(call(id, name, arguments.clone()));
    }
    events.extend(done(StopReason::ToolUse));
    Step::Events(events)
}

fn results(message: &Message) -> Vec<(String, bool)> {
    message
        .content
        .iter()
        .filter_map(|b| match b {
            Block::ToolResult {
                content, is_error, ..
            } => Some((content.clone(), *is_error)),
            _ => None,
        })
        .collect()
}

fn last_results(f: &Fixture) -> Vec<(String, bool)> {
    let messages = f.messages();
    let last = messages
        .iter()
        .rev()
        .find(|m| m.role == Role::User && !results(m).is_empty())
        .unwrap();
    results(last)
}

/// The final state of each call, by id, in order.
fn final_states(events: &[SessionEvent]) -> Vec<(String, CallState)> {
    events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::ToolCall(c) if c.state.is_final() => Some((c.id.clone(), c.state)),
            _ => None,
        })
        .collect()
}

fn notices(events: &[SessionEvent]) -> Vec<String> {
    events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::Notice { text } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn no_questions() -> TestAsker {
    TestAsker::default()
}

#[tokio::test]
async fn plain_answer() {
    let mut f = Fixture::new(vec![say("Hello.")]);
    let (end, events) = f.turn("hi", &no_questions()).await;
    assert_eq!(end.reason, EndReason::Done);
    assert_eq!((end.requests, end.tool_calls), (1, 0));
    assert_eq!((end.input_tokens, end.output_tokens), (100, 10));
    assert_eq!(
        events,
        [
            SessionEvent::State {
                state: State::Working
            },
            SessionEvent::UserMessage { text: "hi".into() },
            SessionEvent::TextDelta {
                text: "Hello.".into()
            },
            SessionEvent::TextDone {
                text: "Hello.".into()
            },
            SessionEvent::Usage {
                input_tokens: 100,
                output_tokens: 10,
                context_tokens: 110,
                model: "test/m".into(),
            },
            SessionEvent::TurnEnd(end),
            SessionEvent::State { state: State::Idle },
        ]
    );
    assert_eq!(
        f.messages(),
        [Message::user("hi"), Message::assistant("Hello.")]
    );
    let request = &f.requests()[0];
    assert_eq!(request.model, "model-1");
    assert_eq!(request.max_output_tokens, Some(1000));
    assert_eq!(request.tools.len(), 7);
    assert_eq!(request.tools[6].name, "ask_user");
    assert!(request.system.contains("Today: 2026-11-01"));
}

#[tokio::test]
async fn tool_round_trip_in_order() {
    let mut first = vec![text("Reading the file.")];
    first.extend(call("c1", "read_file", json!({"path": "a.txt"})));
    first.extend(call("c2", "list_files", json!({})));
    first.extend(done(StopReason::ToolUse));
    let mut f = Fixture::new(vec![Step::Events(first), say("It says hello.")]);
    f.write("a.txt", "hello\n");
    let (end, events) = f.turn("what is in a.txt?", &no_questions()).await;
    assert_eq!(end.reason, EndReason::Done);
    assert_eq!((end.requests, end.tool_calls), (2, 2));

    let messages = f.messages();
    assert_eq!(messages.len(), 4);
    let r = results(&messages[2]);
    assert!(
        r[0].0.contains("1\thello") || r[0].0.contains("hello"),
        "{r:?}"
    );
    assert!(r[1].0.contains("a.txt"), "{r:?}");
    assert!(!r[0].1 && !r[1].1);

    // The second request carries both results.
    let second = &f.requests()[1];
    assert_eq!(second.messages.len(), 3);

    // Each call: pending (start), pending (arguments), running, done.
    let c1: Vec<CallState> = events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::ToolCall(c) if c.id == "c1" => Some(c.state),
            _ => None,
        })
        .collect();
    assert_eq!(
        c1,
        [
            CallState::Pending,
            CallState::Pending,
            CallState::Running,
            CallState::Done
        ]
    );
    assert_eq!(
        final_states(&events),
        [
            ("c1".into(), CallState::Done),
            ("c2".into(), CallState::Done)
        ]
    );
    // A call allowed without a question sends its details with `running`,
    // once; a question sends them with `waiting`.
    let details: Vec<(CallState, bool)> = events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::ToolCall(c) if c.id == "c1" => Some((c.state, c.details.is_some())),
            _ => None,
        })
        .collect();
    assert_eq!(
        details,
        [
            (CallState::Pending, false),
            (CallState::Pending, false),
            (CallState::Running, true),
            (CallState::Done, false)
        ]
    );
    assert!(matches!(
        events.iter().find_map(|e| match e {
            SessionEvent::ToolCall(c) => c.details.as_deref(),
            _ => None,
        }),
        Some(crate::tools::Details::ReadFile { .. })
    ));
}

#[tokio::test]
async fn end_turn_with_calls_runs_them() {
    let mut first = call("c1", "list_files", json!({}));
    first.extend(done(StopReason::EndTurn));
    let mut f = Fixture::new(vec![Step::Events(first), say("ok")]);
    let (end, _) = f.turn("list", &no_questions()).await;
    assert_eq!((end.reason, end.requests), (EndReason::Done, 2));
}

#[tokio::test]
async fn denied_and_wrong_calls() {
    let mut f = Fixture::new(vec![
        calls(&[
            ("c1", "run_command", json!({"command": "rm -rf target"})),
            ("c2", "read_file", json!({})),
            ("c3", "read_file", Value::String("{\"path\": ".into())),
            ("c4", "fly", json!({})),
        ]),
        say("I could not."),
    ]);
    let (end, events) = f.turn("go", &no_questions()).await;
    assert_eq!(end.reason, EndReason::Done);
    assert_eq!(end.tool_calls, 4);
    let r = last_results(&f);
    assert!(r.iter().all(|(_, is_error)| *is_error));
    assert!(
        r[0].0
            .starts_with("denied by permission rule: deny run_command(rm)"),
        "{r:?}"
    );
    assert!(r[1].0.contains("invalid arguments for read_file"), "{r:?}");
    assert!(r[2].0.contains("not valid JSON"), "{r:?}");
    assert!(r[3].0.starts_with("unknown tool `fly`"), "{r:?}");
    let states: Vec<CallState> = final_states(&events).into_iter().map(|s| s.1).collect();
    assert_eq!(
        states,
        [
            CallState::Denied,
            CallState::Error,
            CallState::Error,
            CallState::Error
        ]
    );
}

#[tokio::test]
async fn permission_answers() {
    let mut f = Fixture::new(vec![
        calls(&[
            (
                "c1",
                "write_file",
                json!({"path": "a.txt", "content": "a\n"}),
            ),
            (
                "c2",
                "write_file",
                json!({"path": "b.txt", "content": "b\n"}),
            ),
            (
                "c3",
                "write_file",
                json!({"path": "c.txt", "content": "c\n"}),
            ),
        ]),
        say("done"),
    ]);
    let asker = TestAsker::answers(vec![
        Answer::Allow,
        Answer::Deny,
        Answer::DenyMessage {
            text: " write d.txt instead ".into(),
        },
    ]);
    let (end, events) = f.turn("write", &asker).await;
    assert_eq!(end.reason, EndReason::Done);
    let waiting: Vec<bool> = events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::ToolCall(c) if c.id == "c1" && c.state != CallState::Pending => {
                Some(c.details.is_some())
            }
            _ => None,
        })
        .collect();
    // `waiting` carries them; `running` after the answer does not again.
    assert_eq!(waiting, [true, false, false]);
    assert!(f.root.join("a.txt").exists());
    assert!(!f.root.join("b.txt").exists());
    assert!(!f.root.join("c.txt").exists());
    let r = last_results(&f);
    assert_eq!(r[0], ("created a.txt (1 line)".into(), false));
    assert_eq!(r[1], ("denied by the user".into(), true));
    assert_eq!(
        r[2],
        ("denied by the user: write d.txt instead".into(), true)
    );

    let asked = asker.asked();
    assert_eq!(
        asked.iter().map(|q| q.id.as_str()).collect::<Vec<_>>(),
        ["q1", "q2", "q3"]
    );
    assert_eq!(asked[0].summary, "write_file: a.txt (1 line)");
    assert_eq!(asked[0].choices, PERMISSION_CHOICES);
    let QuestionKind::Permission(p) = &asked[0].kind else {
        panic!("not a permission question");
    };
    assert_eq!(p.reason_text, "default for write_file");
    assert_eq!(p.call.id, "c1");
    assert!(!p.proposals.is_empty());

    // Waiting while a question is open; closed with the answer.
    let waiting = events.iter().position(|e| {
        *e == SessionEvent::State {
            state: State::Waiting,
        }
    });
    let question = events
        .iter()
        .position(|e| matches!(e, SessionEvent::Question(_)));
    assert!(waiting < question);
    assert!(events.contains(&SessionEvent::QuestionClosed {
        id: "q2".into(),
        closed: Closed::Answered {
            answer: Answer::Deny,
            by: "test".into()
        }
    }));
}

#[tokio::test]
async fn allow_always_applies_to_later_calls() {
    let mut f = Fixture::new(vec![
        calls(&[
            ("c1", "run_command", json!({"command": "true"})),
            ("c2", "run_command", json!({"command": "true"})),
        ]),
        say("done"),
    ]);
    let asker = TestAsker::answers(vec![
        Answer::Continue,
        Answer::AllowAlways {
            rules: vec!["run_command(false)".into()],
        },
        Answer::DenyMessage { text: "  ".into() },
        Answer::AllowAlways {
            rules: vec!["run_command(true)".into()],
        },
    ]);
    let (end, events) = f.turn("run", &asker).await;
    assert_eq!(end.reason, EndReason::Done);
    // One question, asked four times: three answers were rejected.
    let asked = asker.asked();
    assert_eq!(asked.len(), 4);
    assert!(asked.iter().all(|q| q.id == "q1"));
    assert_eq!(
        asker.rejected(),
        [
            "`continue` is not an answer to this question",
            "the rules do not allow this call",
            "the message is empty",
        ]
    );
    let r = last_results(&f);
    assert_eq!(r[0], ("exit code 0".into(), false));
    assert_eq!(r[1], ("exit code 0".into(), false));
    assert_eq!(f.agent.env().session.rules().len(), 1);
    assert!(notices(&events).contains(&"rules added: allow run_command(true)".to_owned()));
}

#[tokio::test]
async fn no_one_can_answer() {
    let step = || calls(&[("c1", "write_file", json!({"path": "a.txt", "content": "a"}))]);
    let mut f = Fixture::new(vec![step(), say("no")]);
    let (_, events) = f.turn("write", &PolicyAsker::Deny).await;
    assert_eq!(
        last_results(&f)[0],
        (
            "denied: needs permission, and no one can answer".into(),
            true
        )
    );
    assert!(events.iter().any(|e| matches!(
        e,
        SessionEvent::QuestionClosed {
            closed: Closed::NoOne,
            ..
        }
    )));

    let mut f = Fixture::new(vec![step(), say("yes")]);
    f.turn("write", &PolicyAsker::Allow).await;
    assert!(f.root.join("a.txt").exists());
}

#[tokio::test]
async fn cancel_answer_cancels_the_turn() {
    let mut f = Fixture::new(vec![calls(&[
        ("c1", "write_file", json!({"path": "a.txt", "content": "a"})),
        ("c2", "list_files", json!({})),
    ])]);
    let asker = TestAsker::answers(vec![Answer::Cancel]);
    let (end, events) = f.turn("write", &asker).await;
    assert_eq!(end.reason, EndReason::Cancelled);
    assert_eq!(end.requests, 1);
    let r = last_results(&f);
    assert_eq!(r[0], ("cancelled by the user".into(), true));
    assert_eq!(r[1], ("cancelled by the user".into(), true));
    assert_eq!(notices(&events), ["turn cancelled"]);
    assert_eq!(
        final_states(&events),
        [
            ("c1".into(), CallState::Cancelled),
            ("c2".into(), CallState::Cancelled)
        ]
    );
}

#[tokio::test]
async fn cancel_while_streaming() {
    let mut events = vec![text("Let me look")];
    events.extend(call("c1", "list_files", json!({})));
    events.push(Ok(Event::ToolCallStart {
        id: "c2".into(),
        name: "read_file".into(),
    }));
    let mut f = Fixture::new(vec![Step::Hang(events), say("next")]);
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        token.cancel();
    });
    let (end, events) = f.turn_with("look", &no_questions(), cancel).await;
    assert_eq!(end.reason, EndReason::Cancelled);
    // The text is kept, the calls are left out.
    assert_eq!(
        f.messages(),
        [Message::user("look"), Message::assistant("Let me look")]
    );
    assert_eq!(
        final_states(&events),
        [
            ("c2".into(), CallState::Cancelled),
            ("c1".into(), CallState::Cancelled)
        ]
    );

    // The next turn goes on from there.
    let (end, _) = f.turn("again", &no_questions()).await;
    assert_eq!(end.reason, EndReason::Done);
}

#[tokio::test]
async fn cancel_while_waiting() {
    let mut f = Fixture::new(vec![calls(&[(
        "c1",
        "write_file",
        json!({"path": "a.txt", "content": "a"}),
    )])]);
    let cancel = CancellationToken::new();
    let asker = TestAsker::new(vec![Act::CancelTurn(cancel.clone())]);
    let (end, events) = f.turn_with("write", &asker, cancel).await;
    assert_eq!(end.reason, EndReason::Cancelled);
    assert_eq!(last_results(&f)[0], ("cancelled by the user".into(), true));
    assert!(events.iter().any(|e| matches!(
        e,
        SessionEvent::QuestionClosed {
            closed: Closed::Cancelled,
            ..
        }
    )));
}

#[tokio::test]
async fn cancel_while_a_command_runs() {
    let mut f = Fixture::new(vec![calls(&[
        (
            "c1",
            "run_command",
            json!({"command": "echo start; sleep 5"}),
        ),
        ("c2", "list_files", json!({})),
    ])]);
    let cancel = CancellationToken::new();
    let token = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        token.cancel();
    });
    let started = Instant::now();
    let (end, events) = f.turn_with("run", &no_questions(), cancel).await;
    assert!(started.elapsed() < Duration::from_secs(4));
    assert_eq!(end.reason, EndReason::Cancelled);
    let r = last_results(&f);
    assert_eq!(r[0], ("start\ncancelled by the user".into(), true));
    assert_eq!(r[1], ("cancelled by the user".into(), true));
    assert_eq!(
        final_states(&events),
        [
            ("c1".into(), CallState::Cancelled),
            ("c2".into(), CallState::Cancelled)
        ]
    );
}

#[tokio::test]
async fn command_output_arrives_while_it_runs() {
    let mut f = Fixture::new(vec![
        calls(&[(
            "c1",
            "run_command",
            json!({"command": "echo one; sleep 0.3; echo two"}),
        )]),
        say("ok"),
    ]);
    let (_, events) = f.turn("run", &no_questions()).await;
    let output: Vec<String> = events
        .iter()
        .filter_map(|e| match e {
            SessionEvent::ToolCall(c) => c.output.clone(),
            _ => None,
        })
        .collect();
    assert_eq!(output.concat(), "one\ntwo\n");
    assert!(output.len() >= 2, "{output:?}");
}

fn read_call(id: &str) -> Step {
    calls(&[(id, "list_files", json!({}))])
}

#[tokio::test]
async fn request_limit_continue() {
    let limits = Limits {
        max_requests: 2,
        max_tool_calls: 100,
    };
    let steps = vec![
        read_call("c1"),
        read_call("c2"),
        read_call("c3"),
        say("done"),
    ];
    let mut f = Fixture::with_limits(steps, limits);
    let asker = TestAsker::answers(vec![Answer::Continue]);
    let (end, _) = f.turn("go", &asker).await;
    assert_eq!(end.reason, EndReason::Done);
    assert_eq!((end.requests, end.tool_calls), (4, 3));
    let asked = asker.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(
        asked[0].summary,
        "turn limit reached (2 requests, 2 tool calls)"
    );
    assert_eq!(asked[0].choices, LIMIT_CHOICES);
    assert_eq!(
        asked[0].kind,
        QuestionKind::Limit(Limit {
            max_requests: 2,
            max_tool_calls: 100,
            requests: 2,
            tool_calls: 2
        })
    );
}

#[tokio::test]
async fn request_limit_stop() {
    let limits = Limits {
        max_requests: 2,
        max_tool_calls: 100,
    };
    for act in [Act::Answer(Answer::Stop), Act::NoOne] {
        let steps = vec![read_call("c1"), read_call("c2"), say("never")];
        let mut f = Fixture::with_limits(steps, limits);
        let (end, events) = f.turn("go", &TestAsker::new(vec![act])).await;
        assert_eq!(end.reason, EndReason::Limit);
        assert_eq!(end.requests, 2);
        assert_eq!(notices(&events), ["limit reached"]);
        assert_eq!(f.messages().len(), 5);
    }
}

#[tokio::test]
async fn tool_call_limit() {
    let limits = Limits {
        max_requests: 100,
        max_tool_calls: 2,
    };
    let three = || {
        calls(&[
            ("c1", "list_files", json!({})),
            ("c2", "list_files", json!({})),
            ("c3", "list_files", json!({})),
        ])
    };

    let mut f = Fixture::with_limits(vec![three(), say("never")], limits);
    let asker = TestAsker::answers(vec![Answer::Stop]);
    let (end, events) = f.turn("go", &asker).await;
    assert_eq!(end.reason, EndReason::Limit);
    assert_eq!((end.requests, end.tool_calls), (1, 2));
    let r = last_results(&f);
    assert!(!r[0].1 && !r[1].1);
    assert_eq!(
        r[2],
        ("not run: the turn was stopped at its limit".into(), true)
    );
    assert_eq!(final_states(&events)[2].1, CallState::Cancelled);

    let mut f = Fixture::with_limits(vec![three(), say("done")], limits);
    let asker = TestAsker::answers(vec![Answer::Continue]);
    let (end, _) = f.turn("go", &asker).await;
    assert_eq!(end.reason, EndReason::Done);
    assert_eq!(end.tool_calls, 3);
    assert!(!last_results(&f)[2].1);
}

#[tokio::test]
async fn a_new_turn_has_a_full_budget() {
    let limits = Limits {
        max_requests: 1,
        max_tool_calls: 100,
    };
    let mut f = Fixture::with_limits(vec![say("one"), say("two")], limits);
    let (end, _) = f.turn("1", &no_questions()).await;
    assert_eq!(end.reason, EndReason::Done);
    let (end, _) = f.turn("2", &no_questions()).await;
    assert_eq!(end.reason, EndReason::Done);
}

#[tokio::test]
async fn provider_error_before_the_stream() {
    let auth = ProviderError::new(ErrorKind::Auth, "anthropic: invalid API key");
    let mut f = Fixture::new(vec![Step::Events(vec![Err(auth)]), say("hello")]);
    let (end, events) = f.turn("hi", &no_questions()).await;
    assert_eq!(end.reason, EndReason::Failed);
    assert!(events.contains(&SessionEvent::Error {
        message: "anthropic: invalid API key".into()
    }));
    assert_eq!(f.messages(), [Message::user("hi")]);

    // The session goes on with the next message.
    let (end, _) = f.turn("hi again", &no_questions()).await;
    assert_eq!(end.reason, EndReason::Done);
    assert_eq!(f.messages().len(), 3);
}

#[tokio::test]
async fn provider_error_during_the_stream() {
    let mut events = vec![text("Half an")];
    events.extend(call("c1", "list_files", json!({})));
    events.push(Err(ProviderError::new(
        ErrorKind::Network,
        "connection reset",
    )));
    let mut f = Fixture::new(vec![Step::Events(events)]);
    let (end, events) = f.turn("hi", &no_questions()).await;
    assert_eq!(end.reason, EndReason::Failed);
    assert_eq!(
        f.messages(),
        [Message::user("hi"), Message::assistant("Half an")]
    );
    let call = events
        .iter()
        .rev()
        .find_map(|e| match e {
            SessionEvent::ToolCall(c) if c.state.is_final() => Some(c.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(call.state, CallState::Error);
    assert_eq!(
        call.result.unwrap().text,
        "not run: the answer broke off: connection reset"
    );
}

#[tokio::test]
async fn stream_without_done() {
    let mut f = Fixture::new(vec![Step::Events(vec![text("hm")])]);
    let (end, events) = f.turn("hi", &no_questions()).await;
    assert_eq!(end.reason, EndReason::Failed);
    assert!(events.contains(&SessionEvent::Error {
        message: "the answer ended without Done".into()
    }));
}

#[tokio::test]
async fn context_too_long() {
    let error = ProviderError::new(ErrorKind::ContextTooLong, "prompt is too long");
    let mut f = Fixture::new(vec![Step::Events(vec![Err(error)])]);
    let (end, events) = f.turn("hi", &no_questions()).await;
    assert_eq!(end.reason, EndReason::Failed);
    assert!(
        events.contains(&SessionEvent::Error {
            message: "the conversation is too long for Model One (200 000 tokens); \
                  start a new session"
                .into()
        })
    );
}

#[tokio::test]
async fn max_tokens_cuts_a_call() {
    let mut events = call("c1", "list_files", json!({}));
    events.push(Ok(Event::ToolCallStart {
        id: "c2".into(),
        name: "write_file".into(),
    }));
    events.push(Ok(Event::ToolCallArgsDelta {
        id: "c2".into(),
        json: "{\"path\": \"a".into(),
    }));
    events.extend(done(StopReason::MaxTokens));
    let mut f = Fixture::new(vec![Step::Events(events), say("ok")]);
    let (end, events) = f.turn("go", &no_questions()).await;
    assert_eq!(end.reason, EndReason::Done);
    assert_eq!(end.tool_calls, 1);
    assert_eq!(results(&f.messages()[2]).len(), 1);
    assert!(
        notices(&events)
            .contains(&"answer cut at max_output_tokens; the last tool call is left out".into())
    );
    assert!(final_states(&events).contains(&("c2".into(), CallState::Error)));
}

#[tokio::test]
async fn max_tokens_other_and_empty_answers() {
    let mut cut = vec![text("A long")];
    cut.extend(done(StopReason::MaxTokens));
    let mut other = vec![text("Stopped")];
    other.extend(done(StopReason::Other));
    let mut f = Fixture::new(vec![
        Step::Events(cut),
        Step::Events(other),
        Step::Events(done(StopReason::EndTurn)),
    ]);
    let (end, events) = f.turn("1", &no_questions()).await;
    assert_eq!(end.reason, EndReason::Done);
    assert_eq!(notices(&events), ["answer cut at max_output_tokens"]);
    let (_, events) = f.turn("2", &no_questions()).await;
    assert_eq!(notices(&events), ["the model stopped: other reason"]);
    let (end, events) = f.turn("3", &no_questions()).await;
    assert_eq!(end.reason, EndReason::Done);
    assert_eq!(notices(&events), ["the model gave an empty answer"]);
    // The empty answer is not in the history.
    assert_eq!(f.messages().len(), 5);
}

#[tokio::test]
async fn question_text_and_events_as_json() {
    let mut first = vec![text("I will write it.")];
    first.extend(call(
        "c1",
        "write_file",
        json!({"path": "a.txt", "content": "a"}),
    ));
    first.extend(done(StopReason::ToolUse));
    let mut f = Fixture::new(vec![Step::Events(first), say("ok")]);
    let asker = TestAsker::answers(vec![Answer::Allow]);
    let (_, events) = f.turn("write", &asker).await;
    let QuestionKind::Permission(p) = &asker.asked()[0].kind else {
        panic!("not a permission question");
    };
    assert_eq!(p.text.as_deref(), Some("I will write it."));

    let question = events
        .iter()
        .find(|e| matches!(e, SessionEvent::Question(_)))
        .unwrap();
    let value = serde_json::to_value(question).unwrap();
    assert_eq!(value["event"], "question");
    assert_eq!(value["kind"], "permission");
    assert_eq!(value["id"], "q1");
    assert_eq!(value["call"]["tool"], "write_file");
    assert_eq!(value["choices"][1], "allow_always");
    let closed = events
        .iter()
        .find(|e| matches!(e, SessionEvent::QuestionClosed { .. }))
        .unwrap();
    assert_eq!(
        serde_json::to_value(closed).unwrap(),
        json!({"event": "question_closed", "id": "q1", "closed": "answered",
               "answer": {"choice": "allow"}, "by": "test"})
    );
    let answer: Answer =
        serde_json::from_value(json!({"choice": "allow_always", "rules": ["edit(src/**)"]}))
            .unwrap();
    assert_eq!(
        answer,
        Answer::AllowAlways {
            rules: vec!["edit(src/**)".into()]
        }
    );
}

#[tokio::test]
async fn full_output_is_kept() {
    let mut f = Fixture::new(vec![
        calls(&[(
            "c1",
            "run_command",
            json!({"command": "echo start; sleep 0; yes x | head -c 40000; echo; echo end"}),
        )]),
        say("ok"),
    ]);
    // `yes` and `head` are not allowed by the config; allow them.
    let asker = TestAsker::answers(vec![Answer::Allow]);
    f.turn("run", &asker).await;
    let r = last_results(&f);
    assert!(r[0].0.contains("characters cut"), "{}", r[0].0.len());
    let full = f.agent.history().full_output("c1").unwrap();
    assert!(full.len() > 40_000 && full.contains("end\nexit code 0"));
}

fn ask_args() -> Value {
    json!({
        "questions": [
            {"question": "Which database?", "header": "Database",
             "options": [{"label": "SQLite"}, {"label": "Postgres"}]},
            {"question": "Which features?", "header": "Features",
             "options": [{"label": "Search"}, {"label": "Export"}], "multiple": true}
        ],
        "context": "Both work."
    })
}

fn reply(answers: &[(&[usize], Option<&str>)]) -> Answer {
    Answer::Reply {
        answers: answers
            .iter()
            .map(|(selected, text)| crate::tools::UserAnswer {
                selected: selected.to_vec(),
                text: text.map(str::to_owned),
            })
            .collect(),
    }
}

fn ask_step() -> Step {
    let mut events = vec![text("I need two decisions.")];
    events.extend(call("c1", "ask_user", ask_args()));
    events.extend(done(StopReason::ToolUse));
    Step::Events(events)
}

#[tokio::test]
async fn ask_user_opens_a_question_and_gives_the_answer() {
    let mut f = Fixture::new(vec![ask_step(), say("SQLite it is.")]);
    let asker = TestAsker::answers(vec![reply(&[(&[0], None), (&[0, 1], Some("and fast"))])]);
    let (end, events) = f.turn("build the cache", &asker).await;
    assert_eq!(end.reason, EndReason::Done);
    assert_eq!(end.tool_calls, 1);
    let asked = asker.asked();
    assert_eq!(asked.len(), 1);
    let q = &asked[0];
    assert_eq!(q.summary, "ask_user: Database, Features");
    assert_eq!(q.choices, ASK_USER_CHOICES);
    let QuestionKind::AskUser(a) = &q.kind else {
        panic!("not an ask_user question: {q:?}")
    };
    assert_eq!(a.call.tool, "ask_user");
    assert_eq!(a.call.id, "c1");
    assert_eq!(a.call.arguments, ask_args());
    assert_eq!(a.questions.len(), 2);
    assert_eq!(a.context.as_deref(), Some("Both work."));
    assert_eq!(a.text.as_deref(), Some("I need two decisions."));
    assert_eq!(
        last_results(&f),
        [(
            "The user answered:\n\n1. Which database?\n   SQLite\n\n2. Which features?\n   \
             Search, Export\n   The user added: and fast"
                .into(),
            false
        )]
    );
    assert_eq!(final_states(&events), [("c1".into(), CallState::Done)]);
    // The call waits with the headers as its summary.
    assert!(events.iter().any(|e| matches!(
        e,
        SessionEvent::ToolCall(c) if c.state == CallState::Waiting
            && c.summary.as_deref() == Some("Database, Features")
    )));
    assert!(events.iter().any(|e| matches!(
        e,
        SessionEvent::State {
            state: State::Waiting
        }
    )));
}

#[tokio::test]
async fn ask_user_rejects_wrong_replies() {
    let mut f = Fixture::new(vec![ask_step(), say("ok")]);
    let asker = TestAsker::answers(vec![
        reply(&[(&[0], None)]),
        reply(&[(&[2], None), (&[0], None)]),
        reply(&[(&[0, 1], None), (&[0], None)]),
        reply(&[(&[0], None), (&[1, 1], None)]),
        reply(&[(&[], Some(" ")), (&[0], None)]),
        Answer::Allow,
        reply(&[(&[1], None), (&[], Some("none"))]),
    ]);
    f.turn("go", &asker).await;
    assert_eq!(
        asker.rejected(),
        [
            "1 answers for 2 questions",
            "answer 1: option 2 does not exist; there are 2",
            "answer 1: only one option may be chosen",
            "answer 2: option 1 is chosen twice",
            "answer 1: neither an option nor a text",
            "`allow` is not an answer to this question",
        ]
    );
    assert_eq!(asker.asked().len(), 7);
    assert_eq!(
        last_results(&f)[0].0,
        "The user answered:\n\n1. Which database?\n   Postgres\n\n2. Which features?\n   \
         The user wrote: none"
    );
}

#[tokio::test]
async fn ask_user_decline_cancel_and_no_one() {
    let mut f = Fixture::new(vec![ask_step(), say("I will decide.")]);
    let (end, _) = f
        .turn("go", &TestAsker::answers(vec![Answer::Decline]))
        .await;
    assert_eq!(end.reason, EndReason::Done);
    assert_eq!(
        last_results(&f),
        [(crate::tools::ask_user::DECLINED.into(), false)]
    );

    let mut f = Fixture::new(vec![ask_step()]);
    let (end, events) = f
        .turn("go", &TestAsker::answers(vec![Answer::Cancel]))
        .await;
    assert_eq!(end.reason, EndReason::Cancelled);
    assert_eq!(last_results(&f), [("cancelled by the user".into(), true)]);
    assert_eq!(final_states(&events), [("c1".into(), CallState::Cancelled)]);

    let mut f = Fixture::new(vec![ask_step(), say("ok")]);
    f.turn("go", &PolicyAsker::Deny).await;
    assert_eq!(
        last_results(&f),
        [(crate::tools::ask_user::NOT_AVAILABLE.into(), true)]
    );
}

#[tokio::test]
async fn ask_user_wrong_arguments() {
    let step = calls(&[("c1", "ask_user", json!({"questions": []}))]);
    let mut f = Fixture::new(vec![step, say("ok")]);
    f.turn("go", &no_questions()).await;
    assert_eq!(
        last_results(&f),
        [("questions must have 1–4 entries, not 0".into(), true)]
    );
}

#[tokio::test]
async fn ask_user_is_left_out_when_no_one_can_answer() {
    let mut f = Fixture::new(vec![ask_step(), say("ok")]);
    f.agent.can_ask = false;
    let (_, events) = f.turn("go", &no_questions()).await;
    assert_eq!(f.requests()[0].tools.len(), 6);
    assert!(f.requests()[0].tools.iter().all(|t| t.name != "ask_user"));
    // Called anyway, e.g. from an earlier turn's history.
    assert_eq!(
        last_results(&f),
        [(crate::tools::ask_user::NOT_AVAILABLE.into(), true)]
    );
    assert_eq!(final_states(&events), [("c1".into(), CallState::Denied)]);

    let mut f = Fixture::new(vec![say("ok")]);
    f.agent.env.permissions =
        Arc::new(parse_for_test("[permissions]\nask_user = \"deny\"\n", None));
    f.turn("go", &no_questions()).await;
    assert_eq!(f.requests()[0].tools.len(), 6);
}

// ---------------------------------------------------------- lsp (RFC 0019)

mod lsp {
    use super::*;
    use crate::config::Lsp;
    use crate::lsp::config::ServerDef;
    use crate::lsp::fake::{Fakes, Handler};
    use crate::lsp::pool::{IDLE, Pool};
    use crate::tools::Details;
    use crate::tools::lsp::LspEnv;

    fn handler() -> Handler {
        Arc::new(|method: &str, _: &Value| match method {
            "initialize" => Some(Ok(json!({"capabilities": {
                "textDocumentSync": 1,
                "diagnosticProvider": {"interFileDependencies": false, "workspaceDiagnostics": false}
            }}))),
            "shutdown" => Some(Ok(Value::Null)),
            "textDocument/diagnostic" => Some(Ok(json!({"kind": "full", "items": []}))),
            _ => None,
        })
    }

    fn lsp_env(fakes: &Fakes, start: Action, can_ask: bool) -> LspEnv {
        let config = Lsp {
            start,
            ..Lsp::default()
        };
        let mut env = LspEnv::new(
            Pool::with_starter(fakes.starter(), IDLE),
            &config,
            None,
            "1".into(),
            can_ask,
            false,
        );
        env.servers = vec![ServerDef {
            name: "fake-rs".into(),
            command: vec!["fake-rs".into(), "--stdio".into()],
            extensions: vec!["rs".into()],
            roots: Vec::new(),
            settings: Value::Null,
            env: Vec::new(),
            builtin: false,
        }];
        env
    }

    fn diagnostics(id: &str) -> Step {
        calls(&[(
            id,
            "lsp",
            json!({"operation": "diagnostics", "path": "a.rs"}),
        )])
    }

    #[tokio::test]
    async fn the_tool_is_offered_and_the_start_asked_once() {
        let fakes = Fakes::new(handler());
        let mut f = Fixture::with_lsp(
            vec![diagnostics("c1"), say("ok"), diagnostics("c2"), say("ok")],
            lsp_env(&fakes, Action::Ask, true),
        );
        f.write("a.rs", "fn a() {}\n");
        let asker = TestAsker::answers(vec![Answer::Allow]);
        let (end, _) = f.turn("check", &asker).await;
        assert_eq!(end.reason, EndReason::Done);
        let asked = asker.asked();
        assert_eq!(asked.len(), 1);
        let question = &asked[0];
        assert_eq!(
            question.summary,
            format!("start language server fake-rs in {}", f.root.display())
        );
        assert_eq!(question.choices, LSP_START_CHOICES);
        match &question.kind {
            QuestionKind::Permission(p) => {
                assert_eq!(p.call.tool, crate::config::Tool::Lsp);
                assert!(p.proposals.is_empty());
                match &p.details {
                    Details::LspStart {
                        server, command, ..
                    } => {
                        assert_eq!(server, "fake-rs");
                        assert_eq!(command, &["fake-rs", "--stdio"]);
                    }
                    other => panic!("{other:?}"),
                }
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            last_results(&f),
            [("(no errors or warnings)".to_owned(), false)]
        );
        let request = &f.requests()[0];
        assert!(request.tools.iter().any(|t| t.name == "lsp"));
        assert!(
            request
                .system
                .contains("symbol in .rs files, use lsp before search_files")
        );

        // The next turn uses the server without asking.
        let (end, _) = f.turn("again", &no_questions()).await;
        assert_eq!(end.reason, EndReason::Done);
        assert_eq!(fakes.starts(), 1);
    }

    #[tokio::test]
    async fn no_means_not_again_in_this_turn() {
        let fakes = Fakes::new(handler());
        let mut f = Fixture::with_lsp(
            vec![
                diagnostics("c1"),
                diagnostics("c2"),
                say("ok"),
                diagnostics("c3"),
                say("ok"),
            ],
            lsp_env(&fakes, Action::Ask, true),
        );
        f.write("a.rs", "fn a() {}\n");
        let asker = TestAsker::answers(vec![Answer::Deny]);
        let (_, events) = f.turn("check", &asker).await;
        assert_eq!(asker.asked().len(), 1);
        let denied = "language server not started: denied by the user".to_owned();
        let messages = f.messages();
        let firsts: Vec<_> = messages
            .iter()
            .filter(|m| m.role == Role::User)
            .flat_map(results)
            .collect();
        assert_eq!(firsts, [(denied.clone(), true), (denied.clone(), true)]);
        assert!(
            final_states(&events)
                .iter()
                .all(|(_, state)| *state == CallState::Denied)
        );
        assert_eq!(fakes.starts(), 0);

        // A new turn asks again.
        let asker = TestAsker::answers(vec![Answer::DenyMessage {
            text: "use grep".into(),
        }]);
        f.turn("again", &asker).await;
        assert_eq!(asker.asked().len(), 1);
        assert_eq!(last_results(&f), [(format!("{denied}: use grep"), true)]);
    }

    #[tokio::test]
    async fn without_anyone_to_ask_the_tool_is_left_out() {
        let fakes = Fakes::new(handler());
        let mut f = Fixture::with_lsp(vec![say("ok")], lsp_env(&fakes, Action::Ask, false));
        f.turn("hi", &no_questions()).await;
        let request = &f.requests()[0];
        assert!(!request.tools.iter().any(|t| t.name == "lsp"));
        assert!(!request.system.contains("lsp"));

        // `start = "allow"`: offered, and started without a question.
        let mut f = Fixture::with_lsp(
            vec![diagnostics("c1"), say("ok")],
            lsp_env(&fakes, Action::Allow, false),
        );
        f.write("a.rs", "fn a() {}\n");
        f.turn("check", &no_questions()).await;
        assert!(f.requests()[0].tools.iter().any(|t| t.name == "lsp"));
        assert_eq!(
            last_results(&f),
            [("(no errors or warnings)".to_owned(), false)]
        );
    }
}
