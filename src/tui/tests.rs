//! State tests of `App::update` and frames with the test backend, without a
//! terminal or a server.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use ratatui::style::Color;
use serde_json::json;

use super::app::{
    App, Attachment, Dialog, Effect, Exit, Input, LOST, RECONNECT_FOR, Settings, clock,
};
use super::conversation::Kind;
use super::keys::Focus;
use super::prompt::Mode;
use super::style::Look;
use super::{check_terminal, exit_line, exit_result, only_session, parse_offset, view};
use crate::agent::{
    Answer, CallInfo, CallResult, CallState, Closed, EndReason, LIMIT_CHOICES, Limit,
    PERMISSION_CHOICES, Permission, Question, QuestionKind, SessionEvent, SessionRef, State,
    ToolCallEvent, TurnEnd, TurnInfo,
};
use crate::config::Tool;
use crate::error::Error;
use crate::permissions::Reason;
use crate::protocol::{Notice, Payload, Request, ServerMessage, SessionInfo};
use crate::tools::{Details, PathDetails, Text};

const DIFF: &str = "@@ -1,1 +1,1 @@\n-for i in 0..lines.len() - 1 {\n+for i in 0..lines.len() {\n";

fn settings(unicode: bool, colour: bool) -> Settings {
    Settings {
        look: Look::new(unicode, colour),
        client: "tui pts/3".to_owned(),
        tool_output_lines: 20,
        expand_tool_calls: false,
        scroll_lines: 3,
        timestamps: None,
        windows: HashMap::from([("anthropic/sonnet".to_owned(), 200_000)]),
    }
}

fn info() -> SessionInfo {
    SessionInfo {
        id: 1,
        name: "project".to_owned(),
        workspace: "~/ws/project".to_owned(),
        path: "/ws/project".into(),
        model: "anthropic/sonnet".to_owned(),
        state: State::Idle,
        created: 0,
        queued: 0,
        question: None,
    }
}

fn replayed(event: SessionEvent) -> ServerMessage {
    ServerMessage::Event {
        session: 1,
        replay: true,
        event,
    }
}

fn attachment(events: Vec<SessionEvent>) -> Attachment {
    Attachment {
        session: info(),
        sessions: vec![info()],
        backlog: events.into_iter().map(replayed).collect(),
    }
}

fn app_with(events: Vec<SessionEvent>) -> App {
    App::new(settings(true, true), attachment(events), Instant::now())
}

fn app() -> App {
    app_with(Vec::new())
}

fn live(event: SessionEvent) -> Input {
    Input::Server(ServerMessage::Event {
        session: 1,
        replay: false,
        event,
    })
}

fn press(code: KeyCode, modifiers: KeyModifiers) -> Input {
    Input::Key(KeyEvent::new(code, modifiers))
}

fn key(code: KeyCode) -> Input {
    press(code, KeyModifiers::NONE)
}

fn ch(c: char) -> Input {
    key(KeyCode::Char(c))
}

fn ctrl(c: char) -> Input {
    press(KeyCode::Char(c), KeyModifiers::CONTROL)
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        app.update(ch(c));
    }
}

fn requests(effects: &[Effect]) -> Vec<Request> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Request(r) => Some(r.clone()),
            _ => None,
        })
        .collect()
}

fn exit(effects: &[Effect]) -> Option<Exit> {
    effects.iter().find_map(|e| match e {
        Effect::Exit(exit) => Some(exit.clone()),
        _ => None,
    })
}

fn detached(name: &str) -> Option<Exit> {
    Some(Exit::Detached {
        name: name.to_owned(),
    })
}

fn kinds(app: &App) -> Vec<Kind> {
    app.conversation
        .entries
        .iter()
        .map(|e| e.kind.clone())
        .collect()
}

fn notices(app: &App) -> Vec<String> {
    kinds(app)
        .into_iter()
        .filter_map(|k| match k {
            Kind::Notice(text) => Some(text),
            _ => None,
        })
        .collect()
}

fn call(id: &str, name: &str, state: CallState) -> ToolCallEvent {
    ToolCallEvent {
        id: id.to_owned(),
        name: name.to_owned(),
        state,
        summary: Some("src/parse.rs".to_owned()),
        arguments: json!({"path": "src/parse.rs"}),
        output: None,
        result: None,
    }
}

fn done(mut event: ToolCallEvent, text: &str) -> ToolCallEvent {
    event.state = CallState::Done;
    event.result = Some(CallResult {
        text: text.to_owned(),
        is_error: false,
    });
    event
}

fn permission() -> Question {
    let path = PathDetails {
        written: "src/parse.rs".to_owned(),
        resolved: "src/parse.rs".to_owned(),
        area: "workspace".to_owned(),
    };
    Question {
        id: "q1".to_owned(),
        session: SessionRef::default(),
        opened: 0,
        turn: TurnInfo {
            number: 1,
            requests: 1,
            tool_calls: 1,
        },
        summary: "Allow edit of src/parse.rs?".to_owned(),
        choices: PERMISSION_CHOICES.to_vec(),
        kind: QuestionKind::Permission(Box::new(Permission {
            call: CallInfo {
                tool: Tool::EditFile,
                id: "c1".to_owned(),
                arguments: json!({"path": "src/parse.rs"}),
            },
            details: Details::EditFile {
                path,
                diff: Text {
                    text: DIFF.to_owned(),
                    cut: false,
                },
                first_line: 1,
                last_line: 1,
                occurrences: 1,
                size: 10,
                modified: None,
            },
            reason: Reason::Default {
                tool: Tool::EditFile,
            },
            reason_text: "default for edit_file".to_owned(),
            proposals: vec![
                vec!["edit_file(src/parse.rs)".to_owned()],
                vec!["edit_file(src/**)".to_owned()],
            ],
            text: None,
        })),
    }
}

fn limit() -> Question {
    Question {
        id: "q2".to_owned(),
        session: SessionRef::default(),
        opened: 0,
        turn: TurnInfo {
            number: 1,
            requests: 50,
            tool_calls: 120,
        },
        summary: "limit".to_owned(),
        choices: LIMIT_CHOICES.to_vec(),
        kind: QuestionKind::Limit(Limit {
            max_requests: 50,
            max_tool_calls: 120,
            requests: 50,
            tool_calls: 120,
        }),
    }
}

/// An app with the edit of `c1` waiting for an answer.
fn asked() -> App {
    let mut app = app();
    app.update(live(SessionEvent::State {
        state: State::Waiting,
    }));
    app.update(live(SessionEvent::ToolCall(call(
        "c1",
        "edit_file",
        CallState::Waiting,
    ))));
    app.update(live(SessionEvent::Question(Box::new(permission()))));
    app
}

fn answer_of(effects: &[Effect]) -> Answer {
    match requests(effects).as_slice() {
        [Request::Answer { answer, .. }] => answer.clone(),
        other => panic!("{other:?}"),
    }
}

fn screen_of(app: &mut App, width: u16, height: u16) -> Terminal<TestBackend> {
    app.layout(Rect::new(0, 0, width, height));
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| view::draw(app, frame)).unwrap();
    terminal
}

/// The screen as text, one line per row.
fn screen(app: &mut App, width: u16, height: u16) -> String {
    let terminal = screen_of(app, width, height);
    let buffer = terminal.backend().buffer();
    let area = *buffer.area();
    let mut out = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            out.push_str(buffer[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}

// Replay and events.

#[test]
fn replay_builds_entries() {
    let app = app_with(vec![
        SessionEvent::UserMessage { text: "hi".into() },
        SessionEvent::TextDelta { text: "he".into() },
        SessionEvent::TextDone {
            text: "hello".into(),
        },
        SessionEvent::ToolCall(call("c1", "read_file", CallState::Running)),
        SessionEvent::ToolCall(done(call("c1", "read_file", CallState::Running), "a\nb")),
        SessionEvent::Notice {
            text: "turn cancelled".into(),
        },
        SessionEvent::Error {
            message: "boom".into(),
        },
    ]);
    let kinds = kinds(&app);
    assert_eq!(kinds.len(), 5, "{kinds:?}");
    assert_eq!(kinds[0], Kind::User("hi".into()));
    assert_eq!(kinds[1], Kind::Text("hello".into()));
    assert!(matches!(&kinds[2], Kind::Call(c) if c.state == CallState::Done));
    assert_eq!(kinds[3], Kind::Notice("turn cancelled".into()));
    assert_eq!(kinds[4], Kind::Error("boom".into()));
}

#[test]
fn delta_then_done() {
    let mut app = app();
    app.update(live(SessionEvent::TextDelta { text: "a".into() }));
    app.update(live(SessionEvent::TextDelta { text: "b".into() }));
    assert_eq!(kinds(&app), [Kind::Text("ab".into())]);
    app.update(live(SessionEvent::TextDone {
        text: "whole".into(),
    }));
    assert_eq!(kinds(&app), [Kind::Text("whole".into())]);
    // The next block is a new entry.
    app.update(live(SessionEvent::TextDelta { text: "c".into() }));
    assert_eq!(kinds(&app).len(), 2);
}

#[test]
fn a_done_without_deltas_is_an_entry() {
    let mut app = app();
    app.update(live(SessionEvent::TextDone { text: "x".into() }));
    app.update(live(SessionEvent::TextDone {
        text: String::new(),
    }));
    assert_eq!(kinds(&app), [Kind::Text("x".into())]);
}

#[test]
fn tool_call_states_and_live_output() {
    let mut app = app();
    let mut event = call("c1", "run_command", CallState::Pending);
    event.summary = None;
    event.arguments = serde_json::Value::Null;
    app.update(live(SessionEvent::ToolCall(event)));
    let mut running = call("c1", "run_command", CallState::Running);
    running.output = Some("one\n".into());
    app.update(live(SessionEvent::ToolCall(running.clone())));
    running.output = Some("two\n".into());
    running.summary = None;
    running.arguments = serde_json::Value::Null;
    app.update(live(SessionEvent::ToolCall(running)));
    let kinds = kinds(&app);
    let [Kind::Call(c)] = kinds.as_slice() else {
        panic!("not one call: {kinds:?}");
    };
    assert_eq!(c.output, "one\ntwo\n");
    assert_eq!(c.summary.as_deref(), Some("src/parse.rs"));
    assert_eq!(c.arguments["path"], "src/parse.rs");
    assert!(app.conversation.running_tool());
    app.update(live(SessionEvent::ToolCall(done(
        call("c1", "run_command", CallState::Running),
        "exit 0",
    ))));
    assert!(!app.conversation.running_tool());
    assert_eq!(app.tool_calls, 1);
}

#[test]
fn an_id_used_again_is_a_new_call() {
    let mut app = app();
    let first = call("call_1", "read_file", CallState::Running);
    app.update(live(SessionEvent::ToolCall(first.clone())));
    app.update(live(SessionEvent::ToolCall(done(first.clone(), "old"))));
    app.update(live(SessionEvent::ToolCall(first.clone())));
    app.update(live(SessionEvent::ToolCall(done(first, "new"))));
    let kinds = kinds(&app);
    let [Kind::Call(a), Kind::Call(b)] = kinds.as_slice() else {
        panic!("not two calls: {kinds:?}");
    };
    assert_eq!(a.result.as_ref().unwrap().text, "old");
    assert_eq!(b.result.as_ref().unwrap().text, "new");
}

#[test]
fn model_rename_and_usage_update_the_session() {
    let mut app = app();
    app.update(live(SessionEvent::ModelChanged {
        model: "local/qwen".into(),
    }));
    app.update(live(SessionEvent::Renamed { name: "new".into() }));
    app.update(live(SessionEvent::Usage {
        input_tokens: 1,
        output_tokens: 2,
        context_tokens: 3,
    }));
    assert_eq!(app.session.model, "local/qwen");
    assert_eq!(app.session.name, "new");
    assert_eq!(app.context_tokens, 3);
    assert_eq!(app.requests, 1);
    assert_eq!(
        notices(&app),
        [
            "model: anthropic/sonnet → local/qwen",
            "session renamed to new"
        ]
    );
}

#[test]
fn events_of_other_sessions_are_ignored() {
    let mut app = app();
    let effects = app.update(Input::Server(ServerMessage::Event {
        session: 2,
        replay: false,
        event: SessionEvent::Notice { text: "x".into() },
    }));
    assert!(effects.is_empty());
    assert!(kinds(&app).is_empty());
}

#[test]
fn server_notices_are_shown() {
    let mut app = app();
    app.update(Input::Server(ServerMessage::Notice {
        notice: Notice::ConfigReloaded,
    }));
    app.update(Input::Server(ServerMessage::Notice {
        notice: Notice::ConfigRejected {
            errors: vec!["bad".into()],
        },
    }));
    assert_eq!(
        kinds(&app),
        [
            Kind::Notice("configuration reloaded".into()),
            Kind::Error("configuration rejected: bad".into())
        ]
    );
}

#[test]
fn streaming_text_only_asks_for_a_redraw() {
    let mut app = app();
    let effects = app.update(live(SessionEvent::TextDelta { text: "a".into() }));
    assert_eq!(effects, [Effect::Redraw]);
}

#[test]
fn an_idle_tick_draws_nothing() {
    let mut app = app();
    assert!(app.update(Input::Tick).is_empty());
    // Every second tick asks for the session list, for the session row.
    assert_eq!(requests(&app.update(Input::Tick)), [Request::List]);
    app.update(live(SessionEvent::ToolCall(call(
        "c1",
        "read_file",
        CallState::Running,
    ))));
    assert!(app.update(Input::Tick).contains(&Effect::Redraw));
}

#[test]
fn the_session_row_counts_sessions_and_waiting_ones() {
    let mut app = app();
    assert_eq!(app.place.unwrap().total, 1);
    let mut waiting = info();
    waiting.id = 2;
    waiting.question = Some(limit());
    let result = Ok(Payload::Sessions {
        sessions: vec![waiting, info()],
    });
    app.update(Input::Response {
        request: Request::List,
        result,
    });
    let place = app.place.unwrap();
    assert_eq!((place.position, place.total, place.waiting), (2, 2, 1));
    assert!(screen(&mut app, 80, 24).contains("2/2 ●1"));
}

// The input.

#[test]
fn enter_sends_and_shows_it_as_queued() {
    let mut app = app();
    typed(&mut app, "hello");
    let effects = app.update(key(KeyCode::Enter));
    assert_eq!(
        requests(&effects),
        [Request::Send {
            session: "project".into(),
            text: "hello".into()
        }]
    );
    assert!(app.editor.is_empty());
    assert_eq!(app.conversation.queued, ["hello"]);
    assert!(screen(&mut app, 80, 24).contains("> hello (queued)"));
    app.update(live(SessionEvent::UserMessage {
        text: "hello".into(),
    }));
    assert!(app.conversation.queued.is_empty());
    assert_eq!(kinds(&app), [Kind::User("hello".into())]);
    assert!(!screen(&mut app, 80, 24).contains("queued"));
}

#[test]
fn messages_can_be_sent_while_a_turn_runs() {
    let mut app = app();
    app.update(live(SessionEvent::State {
        state: State::Working,
    }));
    typed(&mut app, "next");
    assert_eq!(requests(&app.update(key(KeyCode::Enter))).len(), 1);
    assert_eq!(app.conversation.queued, ["next"]);
}

#[test]
fn a_failed_send_comes_back_to_the_input() {
    let mut app = app();
    typed(&mut app, "hello");
    let effects = app.update(key(KeyCode::Enter));
    let request = requests(&effects).remove(0);
    app.update(Input::Response {
        request,
        result: Err("no such session".into()),
    });
    assert_eq!(app.editor.text(), "hello");
    assert!(app.conversation.queued.is_empty());
    assert_eq!(kinds(&app), [Kind::Error("no such session".into())]);
}

#[test]
fn alt_and_shift_enter_insert_a_newline() {
    let mut app = app();
    typed(&mut app, "a");
    app.update(press(KeyCode::Enter, KeyModifiers::ALT));
    typed(&mut app, "b");
    app.update(press(KeyCode::Enter, KeyModifiers::SHIFT));
    typed(&mut app, "c");
    assert_eq!(app.editor.text(), "a\nb\nc");
}

#[test]
fn slash_slash_sends_a_message() {
    let mut app = app();
    typed(&mut app, "//etc/passwd");
    assert_eq!(
        requests(&app.update(key(KeyCode::Enter))),
        [Request::Send {
            session: "project".into(),
            text: "/etc/passwd".into()
        }]
    );
}

#[test]
fn empty_input_sends_nothing() {
    let mut app = app();
    typed(&mut app, "  ");
    assert!(requests(&app.update(key(KeyCode::Enter))).is_empty());
}

#[test]
fn message_history_comes_from_the_conversation() {
    let mut app = app_with(vec![
        SessionEvent::UserMessage { text: "one".into() },
        SessionEvent::UserMessage { text: "two".into() },
    ]);
    typed(&mut app, "draft");
    app.update(key(KeyCode::Up));
    assert_eq!(app.editor.text(), "two");
    app.update(key(KeyCode::Up));
    assert_eq!(app.editor.text(), "one");
    app.update(key(KeyCode::Down));
    app.update(key(KeyCode::Down));
    assert_eq!(app.editor.text(), "draft");
}

#[test]
fn paste_goes_to_the_input_and_is_not_sent() {
    let mut app = app();
    let effects = app.update(Input::Paste("a\nb".into()));
    assert!(requests(&effects).is_empty());
    assert_eq!(app.editor.text(), "a\nb");
}

#[test]
fn ctrl_z_does_nothing() {
    let mut app = app();
    typed(&mut app, "a");
    let effects = app.update(ctrl('z'));
    assert!(requests(&effects).is_empty() && exit(&effects).is_none());
    assert_eq!(app.editor.text(), "a");
}

#[test]
fn key_releases_are_ignored() {
    let mut app = app();
    let mut release = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE);
    release.kind = KeyEventKind::Release;
    assert!(app.update(Input::Key(release)).is_empty());
    assert!(app.editor.is_empty());
}

// Scrolling.

fn notices_in(app: &mut App, count: usize) {
    for i in 0..count {
        app.update(live(SessionEvent::Notice {
            text: format!("line {i}"),
        }));
    }
}

#[test]
fn the_view_follows_the_end() {
    let mut app = app();
    notices_in(&mut app, 100);
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("line 99"), "{text}");
    assert!(app.follow && !app.new_output);
}

#[test]
fn scrolled_up_it_stays_and_says_new_output() {
    let mut app = app();
    notices_in(&mut app, 100);
    screen(&mut app, 80, 24);
    app.update(key(KeyCode::PageUp));
    let text = screen(&mut app, 80, 24);
    let top = app.top;
    assert!(!app.follow);
    assert!(!text.contains("line 99"));
    notices_in(&mut app, 5);
    let text = screen(&mut app, 80, 24);
    assert_eq!(app.top, top);
    assert!(app.new_output);
    assert!(text.contains("↓ new output"), "{text}");
    app.update(press(KeyCode::End, KeyModifiers::CONTROL));
    let text = screen(&mut app, 80, 24);
    assert!(app.follow && !app.new_output);
    assert!(
        text.contains("line 99") && !text.contains("new output"),
        "{text}"
    );
}

#[test]
fn scrolling_down_to_the_end_resumes_following() {
    let mut app = app();
    notices_in(&mut app, 100);
    screen(&mut app, 80, 24);
    app.update(key(KeyCode::PageUp));
    screen(&mut app, 80, 24);
    app.update(key(KeyCode::PageDown));
    screen(&mut app, 80, 24);
    assert!(app.follow);
}

#[test]
fn ctrl_home_goes_to_the_start() {
    let mut app = app();
    notices_in(&mut app, 100);
    screen(&mut app, 80, 24);
    app.update(press(KeyCode::Home, KeyModifiers::CONTROL));
    let text = screen(&mut app, 80, 24);
    assert_eq!(app.top, 0);
    assert!(text.contains("line 0"));
}

#[test]
fn the_mouse_wheel_scrolls_by_scroll_lines() {
    let mut app = app();
    notices_in(&mut app, 100);
    screen(&mut app, 80, 24);
    let top = app.top;
    let wheel = |kind| {
        Input::Mouse(MouseEvent {
            kind,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        })
    };
    app.update(wheel(MouseEventKind::ScrollUp));
    screen(&mut app, 80, 24);
    assert_eq!(app.top, top - 3);
    app.update(wheel(MouseEventKind::ScrollDown));
    screen(&mut app, 80, 24);
    assert!(app.follow);
}

#[test]
fn resizing_rewraps_and_keeps_the_entry_at_the_top() {
    let mut app = app();
    let long = "word ".repeat(40);
    for _ in 0..20 {
        app.update(live(SessionEvent::Notice { text: long.clone() }));
    }
    screen(&mut app, 100, 24);
    app.update(press(KeyCode::Home, KeyModifiers::CONTROL));
    app.update(key(KeyCode::PageDown));
    screen(&mut app, 100, 24);
    let entry = app.conversation.entry_at(app.top).unwrap();
    screen(&mut app, 50, 24);
    assert_eq!(app.conversation.entry_at(app.top), Some(entry));
    assert_eq!(app.top, app.conversation.span_of(entry).unwrap().0);
}

// Tool details.

/// Puts the focus on the last entry of the conversation.
fn select_last(app: &mut App) {
    app.update(key(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Conversation);
}

fn long_command(lines: usize) -> App {
    let output: Vec<String> = (1..=lines).map(|i| format!("output {i}")).collect();
    let mut event = call("c1", "run_command", CallState::Running);
    event.summary = Some("cargo test".to_owned());
    event.arguments = json!({"command": "cargo test"});
    app_with(vec![SessionEvent::ToolCall(done(
        event,
        &output.join("\n"),
    ))])
}

#[test]
fn tool_calls_are_collapsed_until_selected_and_toggled() {
    let mut app = long_command(3);
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("▸ run_command cargo test"), "{text}");
    assert!(!text.contains("output 1"));
    select_last(&mut app);
    app.update(key(KeyCode::Enter));
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("▾ run_command"), "{text}");
    assert!(text.contains("$ cargo test") && text.contains("output 3"));
    app.update(key(KeyCode::Enter));
    assert!(!screen(&mut app, 80, 24).contains("output 1"));
}

#[test]
fn long_output_is_cut_and_enter_shows_the_whole() {
    let mut app = long_command(50);
    select_last(&mut app);
    app.update(key(KeyCode::Enter));
    let text = screen(&mut app, 80, 40);
    // The command line and 19 lines of output: 20 together.
    assert!(
        text.contains("output 19") && !text.contains("output 20"),
        "{text}"
    );
    assert!(text.contains("… 31 more lines"), "{text}");
    app.update(key(KeyCode::Enter));
    let text = screen(&mut app, 80, 80);
    assert!(
        text.contains("output 50") && !text.contains("more lines"),
        "{text}"
    );
    app.update(key(KeyCode::Enter));
    assert!(!screen(&mut app, 80, 40).contains("$ cargo test"));
}

#[test]
fn expand_tool_calls_setting_and_zero_means_no_limit() {
    let mut settings = settings(true, true);
    settings.expand_tool_calls = true;
    settings.tool_output_lines = 0;
    let output = (1..=30)
        .map(|i| format!("o{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let event = done(call("c1", "read_file", CallState::Running), &output);
    let mut app = App::new(
        settings,
        attachment(vec![SessionEvent::ToolCall(event)]),
        Instant::now(),
    );
    let text = screen(&mut app, 80, 50);
    assert!(
        text.contains("o30") && !text.contains("more lines"),
        "{text}"
    );
}

#[test]
fn up_and_down_select_entries_in_the_conversation() {
    let mut app = app_with(vec![
        SessionEvent::UserMessage { text: "a".into() },
        SessionEvent::UserMessage { text: "b".into() },
    ]);
    select_last(&mut app);
    assert_eq!(app.selected, Some(1));
    app.update(key(KeyCode::Up));
    assert_eq!(app.selected, Some(0));
    app.update(key(KeyCode::Up));
    assert_eq!(app.selected, Some(0));
    app.update(key(KeyCode::Down));
    app.update(key(KeyCode::Down));
    assert_eq!(app.selected, Some(1));
    // Keys do not type into the input from here.
    app.update(ch('x'));
    assert!(app.editor.is_empty());
    app.update(key(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Input);
}

#[test]
fn question_mark_opens_help_in_the_conversation_only() {
    let mut app = app();
    app.update(ch('?'));
    assert_eq!(app.dialog, None);
    assert_eq!(app.editor.text(), "?");
    app.update(key(KeyCode::Tab));
    app.update(ch('?'));
    assert_eq!(app.dialog, Some(Dialog::Help));
    let text = screen(&mut app, 100, 30);
    assert!(
        text.contains("Help") && text.contains("Alt-Enter"),
        "{text}"
    );
    app.update(ch('x'));
    assert_eq!(app.dialog, None);
}

#[test]
fn colours_of_a_diff() {
    let mut app = asked();
    let terminal = screen_of(&mut app, 80, 24);
    let buffer = terminal.backend().buffer();
    let colour_of = |wanted: char| {
        for y in 0..24 {
            let row: String = (0..80).map(|x| buffer[(x, y)].symbol()).collect();
            let text = row.trim_start();
            if text.starts_with(wanted) {
                let x = row.chars().position(|c| c == wanted).unwrap();
                return buffer[(u16::try_from(x).unwrap(), y)].fg;
            }
        }
        panic!("no line starts with {wanted}");
    };
    assert_eq!(colour_of('+'), Color::Green);
    assert_eq!(colour_of('-'), Color::Red);
}

#[test]
fn no_color_uses_no_colours() {
    let mut app = App::new(
        settings(true, false),
        attachment(Vec::new()),
        Instant::now(),
    );
    app.update(live(SessionEvent::ToolCall(call(
        "c1",
        "edit_file",
        CallState::Waiting,
    ))));
    app.update(live(SessionEvent::Question(Box::new(permission()))));
    app.update(live(SessionEvent::Error {
        message: "boom".into(),
    }));
    let terminal = screen_of(&mut app, 80, 24);
    let buffer = terminal.backend().buffer();
    assert!(
        buffer
            .content()
            .iter()
            .all(|c| c.fg == Color::Reset && c.bg == Color::Reset)
    );
}

// Prompts.

#[test]
fn a_permission_question_opens_the_prompt_and_takes_the_keys() {
    let mut app = asked();
    assert!(app.prompt.is_some());
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("▾ edit_file"), "{text}");
    assert!(text.contains("+for i in 0..lines.len() {"), "{text}");
    assert!(text.contains("asks: default for edit_file"), "{text}");
    // A key that answers nothing goes nowhere.
    app.update(ch('x'));
    assert!(app.editor.is_empty());
    let effects = app.update(ch('y'));
    assert_eq!(answer_of(&effects), Answer::Allow);
    // No second answer while the first is on its way.
    assert!(requests(&app.update(ch('n'))).is_empty());
}

#[test]
fn the_choices_answer() {
    for (input, expected) in [
        (ch('y'), Answer::Allow),
        (ch('n'), Answer::Deny),
        (key(KeyCode::Esc), Answer::Cancel),
    ] {
        let mut app = asked();
        assert_eq!(answer_of(&app.update(input)), expected);
    }
}

#[test]
fn a_and_tab_send_the_widened_proposal() {
    let mut app = asked();
    assert!(requests(&app.update(ch('a'))).is_empty());
    assert!(screen(&mut app, 80, 24).contains("edit_file(src/parse.rs)"));
    app.update(key(KeyCode::Tab));
    assert!(screen(&mut app, 80, 24).contains("edit_file(src/**)"));
    let effects = app.update(key(KeyCode::Enter));
    assert_eq!(
        answer_of(&effects),
        Answer::AllowAlways {
            rules: vec!["edit_file(src/**)".into()]
        }
    );
}

#[test]
fn esc_goes_back_from_always() {
    let mut app = asked();
    app.update(ch('a'));
    assert!(requests(&app.update(key(KeyCode::Esc))).is_empty());
    assert_eq!(app.prompt.as_ref().unwrap().mode, Mode::Choices);
}

#[test]
fn e_sends_a_message_to_the_model() {
    let mut app = asked();
    app.update(ch('e'));
    assert!(requests(&app.update(key(KeyCode::Enter))).is_empty());
    typed(&mut app, "use b.rs");
    app.update(Input::Paste("!\nnow".into()));
    assert!(screen(&mut app, 80, 24).contains("Message to the model: use b.rs! now"));
    assert_eq!(
        answer_of(&app.update(key(KeyCode::Enter))),
        Answer::DenyMessage {
            text: "use b.rs! now".into()
        }
    );
}

#[test]
fn esc_goes_back_from_the_message() {
    let mut app = asked();
    app.update(ch('e'));
    typed(&mut app, "x");
    assert!(requests(&app.update(key(KeyCode::Esc))).is_empty());
    assert_eq!(app.prompt.as_ref().unwrap().mode, Mode::Choices);
}

#[test]
fn a_rejected_answer_keeps_the_prompt() {
    let mut app = asked();
    let effects = app.update(ch('y'));
    let request = requests(&effects).remove(0);
    app.update(Input::Response {
        request,
        result: Err("not allowed".into()),
    });
    assert!(screen(&mut app, 80, 24).contains("not accepted: not allowed"));
    assert_eq!(answer_of(&app.update(ch('n'))), Answer::Deny);
}

#[test]
fn answered_elsewhere_closes_the_prompt_and_says_who() {
    let mut app = asked();
    app.update(live(SessionEvent::QuestionClosed {
        id: "q1".into(),
        closed: Closed::Answered {
            answer: Answer::Allow,
            by: "tui pts/4".into(),
        },
    }));
    assert!(app.prompt.is_none());
    assert_eq!(notices(&app), ["answered by tui pts/4: allow"]);
}

#[test]
fn own_answers_add_no_notice() {
    let mut app = asked();
    app.update(ch('y'));
    app.update(live(SessionEvent::QuestionClosed {
        id: "q1".into(),
        closed: Closed::Answered {
            answer: Answer::Allow,
            by: "tui pts/3".into(),
        },
    }));
    assert!(app.prompt.is_none());
    assert!(notices(&app).is_empty());
}

#[test]
fn another_question_closing_changes_nothing() {
    let mut app = asked();
    app.update(live(SessionEvent::QuestionClosed {
        id: "q9".into(),
        closed: Closed::Cancelled,
    }));
    assert!(app.prompt.is_some());
}

#[test]
fn the_call_collapses_after_the_question() {
    let mut app = asked();
    app.update(live(SessionEvent::QuestionClosed {
        id: "q1".into(),
        closed: Closed::Cancelled,
    }));
    app.update(live(SessionEvent::ToolCall(done(
        call("c1", "edit_file", CallState::Running),
        "edited",
    ))));
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("▸ edit_file"), "{text}");
    assert!(!text.contains("+for i"), "{text}");
}

#[test]
fn a_long_diff_is_shown_in_full_from_its_start_and_can_be_scrolled() {
    let mut question = permission();
    let QuestionKind::Permission(p) = &mut question.kind else {
        panic!("not a permission");
    };
    let diff: String = (0..60).map(|i| format!("+line {i}\n")).collect();
    let Details::EditFile { diff: text, .. } = &mut p.details else {
        panic!("not an edit");
    };
    text.text = diff;
    let mut app = app();
    notices_in(&mut app, 5);
    app.update(live(SessionEvent::ToolCall(call(
        "c1",
        "edit_file",
        CallState::Waiting,
    ))));
    app.update(live(SessionEvent::Question(Box::new(question))));
    let text = screen(&mut app, 80, 24);
    // The summary line is in view, and no "more lines": nothing is cut.
    assert!(text.contains("▾ edit_file"), "{text}");
    assert!(!text.contains("more lines"), "{text}");
    assert!(!app.follow);
    // Scrolling works while the prompt is open, and is no answer.
    let effects = app.update(key(KeyCode::PageDown));
    assert!(requests(&effects).is_empty());
    assert!(app.prompt.is_some());
    app.update(press(KeyCode::End, KeyModifiers::CONTROL));
    assert!(screen(&mut app, 80, 24).contains("+line 59"));
}

#[test]
fn following_resumes_when_a_long_question_closes() {
    let mut question = permission();
    if let QuestionKind::Permission(p) = &mut question.kind
        && let Details::EditFile { diff, .. } = &mut p.details
    {
        diff.text = (0..60).map(|i| format!("+line {i}\n")).collect();
    }
    let mut app = app();
    app.update(live(SessionEvent::ToolCall(call(
        "c1",
        "edit_file",
        CallState::Waiting,
    ))));
    app.update(live(SessionEvent::Question(Box::new(question))));
    screen(&mut app, 80, 24);
    assert!(!app.follow);
    app.update(live(SessionEvent::QuestionClosed {
        id: "q1".into(),
        closed: Closed::Cancelled,
    }));
    assert!(app.follow);
}

#[test]
fn ctrl_d_leaves_a_question_for_another_client() {
    let mut app = asked();
    let effects = app.update(ctrl('d'));
    assert_eq!(exit(&effects), detached("project"));
    assert!(requests(&effects).is_empty());
}

#[test]
fn the_limit_prompt() {
    for (input, expected) in [
        (ch('y'), Answer::Continue),
        (ch('n'), Answer::Stop),
        (key(KeyCode::Esc), Answer::Stop),
    ] {
        let mut app = app();
        app.update(live(SessionEvent::Question(Box::new(limit()))));
        let text = screen(&mut app, 100, 24);
        assert!(
            text.contains("Turn limit reached (50 requests, 120 tool calls)"),
            "{text}"
        );
        assert_eq!(answer_of(&app.update(input)), expected);
    }
}

#[test]
fn an_open_question_opens_the_prompt_on_attach() {
    let mut session = info();
    session.state = State::Waiting;
    session.question = Some(permission());
    let events = vec![
        SessionEvent::ToolCall(call("c1", "edit_file", CallState::Waiting)),
        SessionEvent::Question(Box::new(permission())),
    ];
    let attachment = Attachment {
        session: session.clone(),
        sessions: vec![session],
        backlog: events.into_iter().map(replayed).collect(),
    };
    let mut app = App::new(settings(true, true), attachment, Instant::now());
    assert_eq!(app.prompt.as_ref().unwrap().id(), "q1");
    assert!(screen(&mut app, 80, 24).contains("+for i in"));
}

#[test]
fn a_question_in_the_log_that_is_no_longer_open_opens_nothing() {
    let mut app = app_with(vec![
        SessionEvent::ToolCall(call("c1", "edit_file", CallState::Waiting)),
        SessionEvent::Question(Box::new(permission())),
    ]);
    assert!(app.prompt.is_none());
    // The replayed question still gives the call its diff.
    select_last(&mut app);
    assert!(screen(&mut app, 80, 24).contains("+for i in"));
}

// Keys and exit.

#[test]
fn esc_and_ctrl_c_cancel_a_running_turn() {
    for input in [key(KeyCode::Esc), ctrl('c')] {
        let mut app = app();
        app.update(live(SessionEvent::State {
            state: State::Working,
        }));
        assert_eq!(
            requests(&app.update(input)),
            [Request::Cancel {
                session: "project".into()
            }]
        );
    }
}

#[test]
fn without_a_turn_ctrl_c_clears_the_input_and_esc_does_nothing() {
    let mut app = app();
    typed(&mut app, "abc");
    assert!(requests(&app.update(key(KeyCode::Esc))).is_empty());
    assert_eq!(app.editor.text(), "abc");
    assert!(requests(&app.update(ctrl('c'))).is_empty());
    assert!(app.editor.is_empty());
}

#[test]
fn ctrl_c_twice_detaches_only_within_a_second() {
    let mut app = app();
    assert_eq!(exit(&app.update(ctrl('c'))), None);
    app.now += Duration::from_millis(1500);
    assert_eq!(exit(&app.update(ctrl('c'))), None);
    app.now += Duration::from_millis(500);
    assert_eq!(exit(&app.update(ctrl('c'))), detached("project"));
}

#[test]
fn ctrl_d_detaches_with_empty_input_only() {
    let mut app = app();
    typed(&mut app, "a");
    assert_eq!(exit(&app.update(ctrl('d'))), None);
    app.update(ctrl('c'));
    assert_eq!(exit(&app.update(ctrl('d'))), detached("project"));
}

#[test]
fn a_detach_uses_the_current_name() {
    let mut app = app();
    app.update(live(SessionEvent::Renamed { name: "new".into() }));
    assert_eq!(exit(&app.update(ctrl('d'))), detached("new"));
}

#[test]
fn session_ended_disables_the_input_and_any_key_exits() {
    let mut app = app();
    app.update(live(SessionEvent::Ended {
        reason: "killed".into(),
    }));
    assert_eq!(notices(&app), ["session ended: killed"]);
    assert!(screen(&mut app, 80, 24).contains("any key exits"));
    let paste = app.update(Input::Paste("x".into()));
    assert!(exit(&paste).is_none());
    assert!(app.editor.is_empty());
    assert_eq!(
        exit(&app.update(ch('x'))),
        Some(Exit::Ended {
            name: "project".into(),
            reason: "killed".into()
        })
    );
}

#[test]
fn an_ended_session_does_not_reconnect() {
    let mut app = app();
    app.update(live(SessionEvent::Ended {
        reason: "server stopped".into(),
    }));
    assert!(!app.update(Input::Disconnected).contains(&Effect::Reconnect));
    assert!(app.lost_since.is_none());
    // No more requests for the session row either.
    app.update(Input::Tick);
    assert!(requests(&app.update(Input::Tick)).is_empty());
}

// The connection.

fn lost() -> App {
    let mut app = app_with(vec![
        SessionEvent::UserMessage { text: "hi".into() },
        SessionEvent::ToolCall(done(call("c1", "read_file", CallState::Running), "a\nb")),
    ]);
    screen(&mut app, 80, 24);
    assert!(app.update(Input::Disconnected).contains(&Effect::Reconnect));
    app
}

#[test]
fn a_lost_connection_is_retried_for_ten_seconds() {
    let mut app = lost();
    assert!(screen(&mut app, 80, 24).contains("server disconnected"));
    app.now += RECONNECT_FOR - Duration::from_millis(1);
    assert_eq!(app.update(Input::ReconnectFailed), [Effect::Reconnect]);
    app.now += Duration::from_millis(1);
    assert_eq!(
        app.update(Input::ReconnectFailed),
        [Effect::Exit(Exit::Lost)]
    );
}

#[test]
fn the_input_stays_while_disconnected() {
    let mut app = lost();
    typed(&mut app, "keep");
    assert!(requests(&app.update(key(KeyCode::Enter))).is_empty());
    assert_eq!(app.editor.text(), "keep");
    assert!(screen(&mut app, 100, 24).contains("the input is kept"));
}

#[test]
fn reconnecting_rebuilds_the_conversation_and_keeps_the_view() {
    let mut app = app_with(vec![
        SessionEvent::UserMessage { text: "hi".into() },
        SessionEvent::ToolCall(done(call("c1", "read_file", CallState::Running), "a\nb")),
    ]);
    notices_in(&mut app, 100);
    select_last(&mut app);
    app.selected = Some(1);
    app.update(key(KeyCode::Enter));
    app.update(key(KeyCode::Tab));
    typed(&mut app, "unsent");
    screen(&mut app, 80, 24);
    app.update(key(KeyCode::PageUp));
    screen(&mut app, 80, 24);
    let top = app.top;
    app.update(Input::Disconnected);
    let log = vec![
        SessionEvent::UserMessage { text: "hi".into() },
        SessionEvent::ToolCall(done(call("c1", "read_file", CallState::Running), "a\nb")),
    ];
    let mut events = log;
    for i in 0..100 {
        events.push(SessionEvent::Notice {
            text: format!("line {i}"),
        });
    }
    app.update(Input::Attached(Box::new(attachment(events))));
    assert!(app.lost_since.is_none());
    assert_eq!(app.conversation.entries.len(), 102);
    assert_eq!(app.editor.text(), "unsent");
    screen(&mut app, 80, 24);
    assert_eq!(app.top, top);
    assert!(!app.follow && !app.new_output);
    // The call stays expanded.
    app.update(press(KeyCode::Home, KeyModifiers::CONTROL));
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("▾ read_file"), "{text}");
}

#[test]
fn a_question_still_open_after_reconnect_keeps_its_prompt() {
    let mut app = asked();
    app.update(ch('e'));
    typed(&mut app, "half a mess");
    app.update(Input::Disconnected);
    let mut session = info();
    session.state = State::Waiting;
    session.question = Some(permission());
    app.update(Input::Attached(Box::new(Attachment {
        session: session.clone(),
        sessions: vec![session],
        backlog: Vec::new(),
    })));
    let prompt = app.prompt.as_ref().unwrap();
    assert!(matches!(&prompt.mode, Mode::Explain(e) if e.text() == "half a mess"));
}

#[test]
fn a_question_answered_while_away_closes_on_reconnect() {
    let mut app = asked();
    app.update(Input::Disconnected);
    app.update(Input::Attached(Box::new(attachment(Vec::new()))));
    assert!(app.prompt.is_none());
}

#[test]
fn an_answer_lost_with_the_connection_can_be_given_again() {
    let mut app = asked();
    let effects = app.update(ch('y'));
    let request = requests(&effects).remove(0);
    app.update(Input::Response {
        request,
        result: Err(LOST.into()),
    });
    let prompt = app.prompt.as_ref().unwrap();
    assert!(prompt.error.is_none() && !prompt.sending);
    assert_eq!(answer_of(&app.update(ch('y'))), Answer::Allow);
}

#[test]
fn a_lost_connection_is_no_error_entry() {
    let mut app = app();
    app.update(Input::Response {
        request: Request::Cancel {
            session: "project".into(),
        },
        result: Err(LOST.into()),
    });
    assert!(kinds(&app).is_empty());
}

// Commands.

fn command(app: &mut App, text: &str) -> Vec<Effect> {
    typed(app, text);
    app.update(key(KeyCode::Enter))
}

#[test]
fn help_detach_and_quit() {
    let mut app = app();
    command(&mut app, "/help");
    assert_eq!(app.dialog, Some(Dialog::Help));
    assert!(app.editor.is_empty());
    for name in ["/detach", "/quit"] {
        let mut app = self::app();
        assert_eq!(exit(&command(&mut app, name)), detached("project"));
    }
}

#[test]
fn model_with_an_argument() {
    let mut app = app();
    assert_eq!(
        requests(&command(&mut app, "/model local/qwen")),
        [Request::SetModel {
            session: "project".into(),
            model: "local/qwen".into()
        }]
    );
    assert!(app.editor.is_empty());
}

#[test]
fn a_rejected_model_is_shown() {
    let mut app = app();
    let request = requests(&command(&mut app, "/model nope")).remove(0);
    app.update(Input::Response {
        request,
        result: Err("unknown model nope".into()),
    });
    assert_eq!(kinds(&app), [Kind::Error("unknown model nope".into())]);
}

#[test]
fn rename_needs_a_name() {
    let mut app = app();
    assert!(requests(&command(&mut app, "/rename")).is_empty());
    assert_eq!(notices(&app), ["usage: /rename NAME"]);
    app.editor.clear();
    assert_eq!(
        requests(&command(&mut app, "/rename docs")),
        [Request::Rename {
            session: "project".into(),
            name: "docs".into()
        }]
    );
}

#[test]
fn kill_asks_first() {
    let mut app = app();
    assert!(requests(&command(&mut app, "/kill")).is_empty());
    assert_eq!(app.dialog, Some(Dialog::ConfirmKill));
    assert!(screen(&mut app, 80, 24).contains("End session project"));
    // Other keys do nothing, `n` closes.
    assert!(requests(&app.update(ch('x'))).is_empty());
    assert_eq!(app.dialog, Some(Dialog::ConfirmKill));
    app.update(ch('n'));
    assert_eq!(app.dialog, None);
    command(&mut app, "/kill");
    assert_eq!(
        requests(&app.update(ch('y'))),
        [Request::Kill {
            session: "project".into()
        }]
    );
}

#[test]
fn later_commands_say_so() {
    for name in [
        "clear",
        "permissions",
        "sessions",
        "new",
        "attach",
        "commit",
    ] {
        let mut app = app();
        assert!(requests(&command(&mut app, &format!("/{name}"))).is_empty());
        assert_eq!(
            notices(&app),
            [format!("`/{name}` comes with a later version")]
        );
        assert!(app.editor.is_empty());
    }
    let mut app = app();
    command(&mut app, "/model");
    assert!(notices(&app)[0].contains("comes with a later version"));
}

#[test]
fn unknown_commands_stay_in_the_input() {
    let mut app = app();
    assert!(requests(&command(&mut app, "/nope")).is_empty());
    assert_eq!(app.editor.text(), "/nope");
    assert!(notices(&app)[0].starts_with("unknown command /nope"));
}

// Helpers of the shell.

#[test]
fn exit_lines_and_codes() {
    let detached = Exit::Detached {
        name: "docs".into(),
    };
    assert_eq!(
        exit_line(&detached).unwrap(),
        "clai: detached from docs; \"clai attach docs\" to return"
    );
    assert!(exit_result(&detached).is_ok());
    let ended = Exit::Ended {
        name: "docs".into(),
        reason: "killed".into(),
    };
    assert_eq!(
        exit_line(&ended).unwrap(),
        "clai: session docs ended: killed"
    );
    assert!(exit_result(&ended).is_ok());
    assert_eq!(exit_line(&Exit::Lost), None);
    let lost = exit_result(&Exit::Lost).unwrap_err();
    assert_eq!(lost.to_string(), "server disconnected");
    assert_eq!(lost.exit_code(), 1);
    assert_eq!(exit_line(&Exit::Signal), None);
    assert_eq!(exit_result(&Exit::Signal).unwrap_err().exit_code(), 1);
}

#[test]
fn no_terminal_is_a_usage_error() {
    assert!(check_terminal(true, true).is_ok());
    for (stdin, stdout) in [(false, true), (true, false)] {
        let error = check_terminal(stdin, stdout).unwrap_err();
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().starts_with("the TUI needs a terminal"));
    }
}

#[test]
fn attach_without_a_name() {
    let named = |name: &str| SessionInfo {
        name: name.to_owned(),
        ..info()
    };
    assert_eq!(only_session(&[named("a")]).unwrap(), "a");
    let none = only_session(&[]).unwrap_err();
    assert!(matches!(&none, Error::Failed(m) if m == "no sessions"));
    let several = only_session(&[named("a"), named("b"), named("c")]).unwrap_err();
    assert!(matches!(&several, Error::Failed(m) if m == "several sessions: a, b, c; name one"));
    assert_eq!(several.exit_code(), 1);
}

#[test]
fn clock_and_offsets() {
    assert_eq!(clock(0, 0), "00:00");
    assert_eq!(clock(3_600 + 120, 0), "01:02");
    assert_eq!(clock(0, 5 * 3600 + 1800), "05:30");
    assert_eq!(clock(1800, -3600), "23:30");
    assert_eq!(parse_offset("+0530"), Some(19_800));
    assert_eq!(parse_offset("-0100"), Some(-3600));
    assert_eq!(parse_offset("UTC"), None);
}

#[test]
fn timestamps_prefix_live_entries_only() {
    let mut settings = settings(true, true);
    settings.timestamps = Some(0);
    let mut app = App::new(
        settings,
        attachment(vec![SessionEvent::UserMessage { text: "old".into() }]),
        Instant::now(),
    );
    app.update(live(SessionEvent::UserMessage { text: "new".into() }));
    let text = screen(&mut app, 80, 24);
    let line_of = |wanted: &str| {
        text.lines()
            .find(|l| l.contains(wanted))
            .unwrap_or_else(|| panic!("no {wanted} in {text}"))
            .to_owned()
    };
    assert!(line_of("> old").starts_with("> old"));
    let new = line_of("> new");
    assert_eq!(new.chars().nth(2), Some(':'), "{new}");
    assert!(new.get(5..).unwrap().starts_with(" > new"), "{new}");
}

// Frames.

fn idle_session() -> App {
    let mut app = app_with(vec![
        SessionEvent::UserMessage {
            text: "fix the failing test".into(),
        },
        SessionEvent::TextDone {
            text: "The test fails because `parse` drops the last line. Let me look.".into(),
        },
        SessionEvent::ToolCall(done(
            call("c1", "read_file", CallState::Running),
            "one\ntwo\nthree",
        )),
        SessionEvent::Usage {
            input_tokens: 12_000,
            output_tokens: 300,
            context_tokens: 12_300,
        },
        SessionEvent::TurnEnd(TurnEnd {
            reason: EndReason::Done,
            requests: 1,
            tool_calls: 1,
            input_tokens: 12_000,
            output_tokens: 300,
        }),
    ]);
    app.place = app.place.map(|mut place| {
        place.total = 3;
        place.waiting = 1;
        place
    });
    app
}

fn with_prompt(mut app: App) -> App {
    app.update(live(SessionEvent::ToolCall(call(
        "c2",
        "edit_file",
        CallState::Waiting,
    ))));
    let mut question = permission();
    if let QuestionKind::Permission(p) = &mut question.kind {
        p.call.id = "c2".into();
    }
    app.update(live(SessionEvent::Question(Box::new(question))));
    app
}

#[test]
fn frame_idle_80x24() {
    let mut app = idle_session();
    insta::assert_snapshot!(screen(&mut app, 80, 24));
}

#[test]
fn frame_idle_120x40() {
    let mut app = idle_session();
    insta::assert_snapshot!(screen(&mut app, 120, 40));
}

#[test]
fn frame_permission_prompt_with_a_diff() {
    let mut app = with_prompt(idle_session());
    insta::assert_snapshot!(screen(&mut app, 80, 24));
}

#[test]
fn frame_limit_prompt() {
    let mut app = idle_session();
    app.update(live(SessionEvent::Question(Box::new(limit()))));
    insta::assert_snapshot!(screen(&mut app, 80, 24));
}

#[test]
fn frame_command_with_cut_output() {
    let mut app = long_command(30);
    select_last(&mut app);
    app.update(key(KeyCode::Enter));
    insta::assert_snapshot!(screen(&mut app, 80, 30));
}

#[test]
fn frame_ascii_symbols() {
    let events = vec![
        SessionEvent::UserMessage {
            text: "fix the failing test".into(),
        },
        SessionEvent::ToolCall(done(
            call("c1", "read_file", CallState::Running),
            "one\ntwo",
        )),
    ];
    let app = App::new(settings(false, true), attachment(events), Instant::now());
    let mut app = with_prompt(app);
    let text = screen(&mut app, 80, 24);
    assert!(text.is_ascii(), "{text}");
    insta::assert_snapshot!(text);
}

#[test]
fn frame_terminal_too_small() {
    let mut app = idle_session();
    let small = screen(&mut app, 39, 10);
    assert!(small.contains("terminal too small"), "{small}");
    assert!(!small.contains("fix the failing test"));
    insta::assert_snapshot!(small);
    // At the smallest size all parts fit, also with a prompt.
    let mut app = with_prompt(idle_session());
    let least = screen(&mut app, 40, 10);
    assert!(!least.contains("terminal too small"), "{least}");
    assert!(
        least.contains("[y] yes") || least.contains("asks"),
        "{least}"
    );
    let mut app = idle_session();
    assert!(screen(&mut app, 40, 9).contains("terminal too small"));
}

#[test]
fn there_is_no_box_around_the_screen() {
    for (unicode, sides) in [(true, "│┌┐└┘├┤"), (false, "|+")] {
        let mut app = App::new(
            settings(unicode, true),
            attachment(Vec::new()),
            Instant::now(),
        );
        let text = screen(&mut app, 80, 24);
        assert!(!text.contains(|c| sides.contains(c)), "{text}");
        let lines: Vec<&str> = text.lines().collect();
        // Lines above and below the input, then the two footer rows.
        let rule = if unicode { "─" } else { "-" }.repeat(80);
        assert_eq!(lines[19], rule);
        assert!(lines[20].starts_with('>'), "{text}");
        assert_eq!(lines[21], rule);
        assert!(lines[22].starts_with("  idle"), "{text}");
        assert!(lines[23].starts_with("  project"), "{text}");
    }
}

#[test]
fn the_cursor_is_in_the_input() {
    let mut app = app();
    typed(&mut app, "abc");
    let mut terminal = screen_of(&mut app, 80, 24);
    let position = terminal.get_cursor_position().unwrap();
    assert_eq!((position.x, position.y), (5, 20));
}

#[test]
fn long_input_grows_and_stays_within_the_screen() {
    let mut app = app();
    for _ in 0..30 {
        app.update(press(KeyCode::Enter, KeyModifiers::ALT));
    }
    typed(&mut app, "last");
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("  last"), "{text}");
    let areas = view::areas(&app, Rect::new(0, 0, 80, 24)).unwrap();
    assert_eq!(areas.input.height, 6);
    assert!(areas.conversation.height > 0);
}

// Performance.

/// A frame must not reflow everything. The bound is generous, for a debug
/// build on a busy machine: full reflows of 10 000 lines per frame exceed it.
#[test]
fn ten_thousand_lines_scroll_quickly() {
    let mut app = app();
    let text: String = (0..1000)
        .map(|i| format!("Paragraph {i} with some `code` in it.\n\n"))
        .collect();
    for i in 0..10 {
        app.update(live(SessionEvent::UserMessage {
            text: format!("question {i}"),
        }));
        app.update(live(SessionEvent::TextDone { text: text.clone() }));
    }
    let size = Rect::new(0, 0, 120, 40);
    app.layout(size);
    assert!(app.conversation.total() >= 10_000);
    let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
    let start = Instant::now();
    for _ in 0..100 {
        app.update(key(KeyCode::PageUp));
        app.layout(size);
        terminal.draw(|frame| view::draw(&app, frame)).unwrap();
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(3),
        "100 frames took {elapsed:?}"
    );
    // A streaming delta only rebuilds its own entry.
    let start = Instant::now();
    for _ in 0..100 {
        app.update(live(SessionEvent::TextDelta { text: "x".into() }));
        app.layout(size);
        terminal.draw(|frame| view::draw(&app, frame)).unwrap();
    }
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(3),
        "100 frames took {elapsed:?}"
    );
}
