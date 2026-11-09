use serde_json::json;
use tokio::io::BufReader;

use super::*;
use crate::agent::{
    ASK_USER_CHOICES, AskCall, AskUser, CallInfo, CallResult, CallState, Closed, EndReason,
    LIMIT_CHOICES, Limit, PERMISSION_CHOICES, Permission, QuestionKind, SessionRef, ToolCallEvent,
    TurnEnd, TurnInfo,
};
use crate::config::{Action, Tool};
use crate::permissions::{Decision, PartDecision, Reason};
use crate::tools::{Details, PathDetails, Text};

fn round_trip<T>(value: &T) -> T
where
    T: Serialize + DeserializeOwned,
{
    let text = serde_json::to_string(value).unwrap();
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{e}: {text}"))
}

fn permission(details: Details) -> Question {
    Question {
        id: "q7".into(),
        session: SessionRef {
            name: "clai".into(),
            workspace: "~/ws/clai".into(),
        },
        opened: 1_793_491_200,
        turn: TurnInfo {
            number: 2,
            requests: 3,
            tool_calls: 4,
        },
        summary: "edit_file: src/a.rs (line 3)".into(),
        choices: PERMISSION_CHOICES.to_vec(),
        kind: QuestionKind::Permission(Box::new(Permission {
            call: CallInfo {
                tool: Tool::EditFile,
                id: "call_1".into(),
                arguments: json!({"path": "src/a.rs", "old": "a", "new": "b"}),
            },
            details,
            reason: Reason::Rule {
                rule: "ask edit(src/**)".into(),
                source: "config.toml:3".into(),
                message: Some("careful".into()),
            },
            reason_text: "rule ask edit(src/**) (config.toml:3)".into(),
            proposals: vec![vec!["edit_file(src/a.rs)".into()]],
            text: Some("Fixing it.".into()),
        })),
    }
}

#[test]
fn questions_round_trip() {
    let edit = permission(Details::EditFile {
        path: PathDetails {
            written: "src/a.rs".into(),
            resolved: "src/a.rs".into(),
            area: "workspace".into(),
        },
        diff: Text::new("@@ -1 +1 @@\n-a\n+b\n".into()),
        first_line: 3,
        last_line: 3,
        occurrences: 1,
        size: 10,
        modified: Some(5),
    });
    assert_eq!(round_trip(&edit), edit);

    let command = permission(Details::RunCommand {
        command: "cargo test > out.txt".into(),
        cwd: "~/ws/clai".into(),
        timeout_s: 120,
        env: vec![("A".into(), "1".into())],
        parts: vec![PartDecision {
            text: "cargo test".into(),
            words: vec![],
            assignments: 0,
            decision: Decision {
                action: Action::Ask,
                reason: Reason::Default {
                    tool: Tool::RunCommand,
                },
            },
        }],
        redirects: vec![],
    });
    assert_eq!(round_trip(&command), command);

    let limit = Question {
        choices: LIMIT_CHOICES.to_vec(),
        kind: QuestionKind::Limit(Limit {
            max_requests: 50,
            max_tool_calls: 120,
            requests: 50,
            tool_calls: 7,
        }),
        ..edit
    };
    assert_eq!(round_trip(&limit), limit);

    let arguments = json!({"questions": [{"question": "Which?", "header": "Pick",
        "options": [{"label": "A", "description": "first"}, {"label": "B"}], "multiple": true}]});
    let asked = crate::tools::ask_user::check(&arguments).unwrap();
    let ask = Question {
        summary: "ask_user: Pick".into(),
        choices: ASK_USER_CHOICES.to_vec(),
        kind: QuestionKind::AskUser(Box::new(AskUser {
            call: AskCall {
                tool: "ask_user".into(),
                id: "call_2".into(),
                arguments,
            },
            questions: asked.questions,
            context: Some("Why.".into()),
            text: None,
        })),
        ..limit
    };
    assert_eq!(round_trip(&ask), ask);
    let value = serde_json::to_value(&ask).unwrap();
    assert_eq!(value["kind"], "ask_user");
    assert_eq!(value["choices"], json!(["reply", "decline", "cancel"]));
    assert_eq!(
        value["questions"],
        json!([{"question": "Which?", "header": "Pick", "multiple": true,
                "options": [{"label": "A", "description": "first"}, {"label": "B"}]}])
    );
    assert!(value.get("text").is_none());

    let reply = Answer::Reply {
        answers: vec![crate::tools::UserAnswer {
            selected: vec![0, 1],
            text: Some("both".into()),
        }],
    };
    assert_eq!(
        serde_json::to_value(&reply).unwrap(),
        json!({"choice": "reply", "answers": [{"selected": [0, 1], "text": "both"}]})
    );
    assert_eq!(round_trip(&reply), reply);
    assert_eq!(
        serde_json::from_value::<Answer>(json!({"choice": "reply", "answers": [{"text": "x"}]}))
            .unwrap(),
        Answer::Reply {
            answers: vec![crate::tools::UserAnswer {
                selected: vec![],
                text: Some("x".into()),
            }]
        }
    );
    assert_eq!(
        serde_json::to_value(Answer::Decline).unwrap(),
        json!({"choice": "decline"})
    );
}

#[test]
fn events_round_trip() {
    let events = vec![
        SessionEvent::State {
            state: State::Waiting,
        },
        SessionEvent::ToolCall(ToolCallEvent {
            id: "c1".into(),
            name: "read_file".into(),
            state: CallState::Done,
            summary: Some("a.rs".into()),
            arguments: json!({"path": "a.rs"}),
            output: None,
            result: Some(CallResult {
                text: "x".into(),
                is_error: false,
            }),
        }),
        SessionEvent::QuestionClosed {
            id: "q1".into(),
            closed: Closed::Answered {
                answer: Answer::AllowAlways {
                    rules: vec!["run_command(ls)".into()],
                },
                by: "clai chat".into(),
            },
        },
        SessionEvent::QuestionClosed {
            id: "q2".into(),
            closed: Closed::Cancelled,
        },
        SessionEvent::TurnEnd(TurnEnd {
            reason: EndReason::Done,
            requests: 1,
            tool_calls: 0,
            input_tokens: 10,
            output_tokens: 2,
        }),
        SessionEvent::ModelChanged {
            from: "anthropic/sonnet".into(),
            model: "local/qwen".into(),
        },
        SessionEvent::Ended {
            reason: "killed".into(),
        },
    ];
    for event in events {
        let message = ServerMessage::Event {
            session: 3,
            replay: false,
            event,
        };
        assert_eq!(round_trip(&message), message);
    }
}

#[test]
fn wire_format() {
    let request = ClientMessage::Request {
        id: 4,
        request: Request::Answer {
            session: "clai".into(),
            question: "q3".into(),
            answer: Answer::DenyMessage { text: "no".into() },
        },
    };
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({"type": "request", "id": 4, "request": {
            "command": "answer", "session": "clai", "question": "q3",
            "answer": {"choice": "deny_message", "text": "no"}}})
    );
    let response = ServerMessage::Response {
        id: 4,
        result: Outcome::Error(ErrorReply::failed("already answered by clai chat")),
    };
    assert_eq!(
        serde_json::to_value(&response).unwrap(),
        json!({"type": "response", "id": 4,
               "result": {"error": {"message": "already answered by clai chat"}}})
    );
    assert_eq!(round_trip(&response), response);
    let hello = ClientMessage::Hello(ClientHello {
        version: VERSION,
        kind: ClientKind::Chat,
        name: "clai chat".into(),
        answers: true,
    });
    assert_eq!(round_trip(&hello), hello);
}

#[tokio::test]
async fn framing() {
    let mut out = Vec::new();
    write(
        &mut out,
        &ClientMessage::Request {
            id: 1,
            request: Request::List,
        },
    )
    .await
    .unwrap();
    out.extend_from_slice(b"\n");
    write(
        &mut out,
        &ClientMessage::Request {
            id: 2,
            request: Request::Status,
        },
    )
    .await
    .unwrap();
    out.extend_from_slice(b"{broken\n");
    let mut reader = BufReader::new(out.as_slice());
    let first: Option<ClientMessage> = read(&mut reader).await.unwrap();
    assert!(matches!(first, Some(ClientMessage::Request { id: 1, .. })));
    let second: Option<ClientMessage> = read(&mut reader).await.unwrap();
    assert!(matches!(second, Some(ClientMessage::Request { id: 2, .. })));
    assert!(matches!(
        read::<ClientMessage, _>(&mut reader).await,
        Err(ReadError::Json(_))
    ));
    assert!(
        read::<ClientMessage, _>(&mut reader)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn too_long() {
    let long = vec![b'x'; MAX_LINE + 10];
    let mut reader = BufReader::new(long.as_slice());
    assert!(matches!(
        read::<ClientMessage, _>(&mut reader).await,
        Err(ReadError::TooLong)
    ));
}

#[test]
fn models_round_trip() {
    let request = ClientMessage::Request {
        id: 1,
        request: Request::Models,
    };
    assert_eq!(
        serde_json::to_value(&request).unwrap(),
        json!({"type": "request", "id": 1, "request": {"command": "models"}})
    );
    assert_eq!(round_trip(&request), request);
    let response = ServerMessage::Response {
        id: 1,
        result: Outcome::Ok(Payload::Models {
            models: vec![ModelInfo {
                model: "local/qwen".into(),
                provider: "local".into(),
                name: "qwen".into(),
                id: "qwen3-coder:30b".into(),
                context_window: 32768,
                default: true,
                key: "missing".into(),
            }],
            config: PathBuf::from("/home/u/.config/clai/config.toml"),
        }),
    };
    assert_eq!(round_trip(&response), response);
}

#[test]
fn model_changed_without_from() {
    // A log of an older `clai` has no `from`, and none is written when empty.
    let old: SessionEvent =
        serde_json::from_value(json!({"event": "model_changed", "model": "a/b"})).unwrap();
    assert_eq!(
        old,
        SessionEvent::ModelChanged {
            from: String::new(),
            model: "a/b".into()
        }
    );
    assert_eq!(
        serde_json::to_value(&old).unwrap(),
        json!({"event": "model_changed", "model": "a/b"})
    );
}

#[test]
fn totals_sum_only_usage_events() {
    let usage = |input_tokens, output_tokens| SessionEvent::Usage {
        input_tokens,
        output_tokens,
        context_tokens: 999,
    };
    let log = [
        usage(10, 1),
        SessionEvent::Notice { text: "x".into() },
        usage(20, 2),
    ];
    assert_eq!(
        TokenTotals::of_log(&log),
        TokenTotals {
            input_tokens: 30,
            output_tokens: 3
        }
    );
    // A provider's nonsense does not overflow.
    let huge = [usage(u64::MAX, u64::MAX), usage(5, 5)];
    assert_eq!(TokenTotals::of_log(&huge).input_tokens, u64::MAX);
}

#[test]
fn session_info_of_an_older_server() {
    let old = json!({
        "id": 1, "name": "s", "workspace": "~/ws", "path": "/home/u/ws",
        "model": "a/b", "state": "saved", "created": 100, "queued": 0,
        "question": null,
        "usage": {"input_tokens": 0, "output_tokens": 0}
    });
    let info: SessionInfo = serde_json::from_value(old).unwrap();
    assert_eq!((info.updated, info.turns), (0, 0));
    let info = SessionInfo {
        updated: 200,
        turns: 3,
        ..info
    };
    let back = round_trip(&info);
    assert_eq!((back.updated, back.turns), (200, 3));
}
