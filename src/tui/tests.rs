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
use super::commands::COMMANDS;
use super::conversation::Kind;
use super::keys::Focus;
use super::prompt::Mode;
use super::style::Look;
use super::{check_terminal, exit_line, exit_result, first_session, parse_offset, save, view};
use crate::agent::{
    Answer, CallInfo, CallResult, CallState, Closed, EndReason, LIMIT_CHOICES, Limit,
    PERMISSION_CHOICES, Permission, Question, QuestionKind, SessionEvent, SessionRef, State,
    ToolCallEvent, TurnEnd, TurnInfo,
};
use crate::config::Tool;
use crate::error::Error;
use crate::permissions::Reason;
use crate::protocol::{
    ModelInfo, Notice, Payload, Request, ServerMessage, SessionInfo, TokenTotals,
};
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
        tui: crate::config::Tui {
            symbols: if unicode {
                crate::config::Symbols::Unicode
            } else {
                crate::config::Symbols::Ascii
            },
            ..crate::config::Tui::default()
        },
        theming: super::app::Theming {
            themes: crate::config::themes::file_themes(&[]).0,
            warnings: Vec::new(),
            file: super::app::FileTui {
                tui: crate::config::Tui::default(),
                keys: Vec::new(),
                stamp: None,
            },
            colouring: if colour {
                super::style::Colouring::Ansi256
            } else {
                super::style::Colouring::None
            },
            unicode_locale: unicode,
            theme_env: None,
            offset: 3600,
            user_file: "/home/u/.config/clai/config.toml".into(),
        },
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
        updated: 0,
        turns: 0,
        queued: 0,
        question: None,
        usage: Default::default(),
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
        from: "anthropic/sonnet".into(),
        model: "local/qwen".into(),
    }));
    app.update(live(SessionEvent::Renamed { name: "new".into() }));
    app.update(live(SessionEvent::Usage {
        input_tokens: 1,
        output_tokens: 2,
        context_tokens: 3,
        model: String::new(),
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
    // In the list, `Enter` completes it, so that the name can be typed.
    assert!(requests(&command(&mut app, "/rename")).is_empty());
    assert_eq!(app.editor.text(), "/rename ");
    assert!(app.command_list.is_none());
    assert!(notices(&app).is_empty());
    // With the list closed, the text is a command as it stands.
    app.editor.clear();
    typed(&mut app, "/rename");
    app.update(key(KeyCode::Esc));
    assert!(requests(&app.update(key(KeyCode::Enter))).is_empty());
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
    assert!(screen(&mut app, 80, 24).contains("Delete project?"));
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
    for name in ["clear", "permissions", "commit"] {
        let mut app = app();
        assert!(requests(&command(&mut app, &format!("/{name}"))).is_empty());
        assert_eq!(
            notices(&app),
            [format!("`/{name}` comes with a later version")]
        );
        assert!(app.editor.is_empty());
    }
}

#[test]
fn unknown_commands_stay_in_the_input() {
    let mut app = app();
    // In the list, `Enter` does nothing: it already says `no command`.
    assert!(requests(&command(&mut app, "/nope")).is_empty());
    assert_eq!(app.editor.text(), "/nope");
    assert!(app.command_list.is_some());
    assert!(notices(&app).is_empty());
    // With the list closed, it gets a notice.
    app.update(key(KeyCode::Esc));
    assert!(requests(&app.update(key(KeyCode::Enter))).is_empty());
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
    assert_eq!(first_session(&[named("a")]).unwrap(), ("a".into(), false));
    let none = first_session(&[]).unwrap_err();
    assert!(matches!(&none, Error::Failed(m) if m == "no sessions"));
    assert_eq!(none.exit_code(), 1);
    // Several: the first, and the user chooses in the session list.
    let several = first_session(&[named("a"), named("b"), named("c")]).unwrap();
    assert_eq!(several, ("a".into(), true));
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
            model: String::new(),
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

// Token counters.

fn usage(input_tokens: u64, output_tokens: u64, context_tokens: u64) -> SessionEvent {
    SessionEvent::Usage {
        input_tokens,
        output_tokens,
        context_tokens,
        model: String::new(),
    }
}

fn totals(input_tokens: u64, output_tokens: u64) -> Option<TokenTotals> {
    Some(TokenTotals {
        input_tokens,
        output_tokens,
    })
}

/// The session row, at a width where nothing is cut.
fn session_row(app: &mut App) -> String {
    let text = screen(app, 200, 24);
    text.lines().last().unwrap().trim().to_owned()
}

#[test]
fn counters_are_hidden_before_the_first_usage() {
    let mut app = app();
    assert_eq!(app.usage, None);
    assert!(!session_row(&mut app).contains(" in "));
    // Zero tokens reported is still a report.
    app.update(live(usage(0, 0, 0)));
    assert!(session_row(&mut app).ends_with("· in 0 out 0"));
}

#[test]
fn counters_sum_live_and_replayed_events() {
    let mut app = app_with(vec![usage(40_000, 3_000, 12_000)]);
    assert_eq!(app.usage, totals(40_000, 3_000));
    app.update(live(usage(5_100, 200, 12_300)));
    assert_eq!(app.usage, totals(45_100, 3_200));
    assert_eq!(app.context_tokens, 12_300);
    assert_eq!(
        session_row(&mut app),
        "project · ~/ws/project · anthropic/sonnet · 1/1 · 12.3k/200k · in 45.1k out 3.2k"
    );
}

#[test]
fn a_reconnect_does_not_double_the_counters() {
    let log = vec![usage(100, 10, 110), usage(200, 20, 220)];
    let mut app = app_with(log.clone());
    assert_eq!(app.usage, totals(300, 30));
    app.update(Input::Disconnected);
    // The session info carries the totals too; only the log may count.
    let mut session = info();
    session.usage = totals(300, 30).unwrap();
    let again = Attachment {
        session: session.clone(),
        sessions: vec![session],
        backlog: log.into_iter().map(replayed).collect(),
    };
    app.update(Input::Attached(Box::new(again)));
    assert_eq!(app.usage, totals(300, 30));
    assert_eq!(app.context_tokens, 220);
}

#[test]
fn counters_are_cut_first_on_a_narrow_terminal() {
    let mut app = app_with(vec![usage(45_100, 3_200, 12_300)]);
    let text = screen(&mut app, 80, 24);
    let row = text.lines().last().unwrap();
    assert!(row.contains("12.3k/200k"), "{row}");
    assert!(!row.contains("out 3.2k"), "{row}");
}

#[test]
fn a_model_without_a_known_window_shows_the_size_alone() {
    let mut app = app();
    app.settings.windows.clear();
    // Nothing known yet: no size to show.
    assert_eq!(
        session_row(&mut app),
        "project · ~/ws/project · anthropic/sonnet · 1/1"
    );
    app.update(live(usage(12_000, 300, 12_300)));
    assert_eq!(
        session_row(&mut app),
        "project · ~/ws/project · anthropic/sonnet · 1/1 · 12.3k · in 12k out 300"
    );
}

// The model change on replay.

#[test]
fn replay_shows_the_old_model_from_the_event() {
    // `attached` sets the current model first; the notice must not use it.
    let mut session = info();
    session.model = "local/qwen".into();
    let attachment = Attachment {
        session: session.clone(),
        sessions: vec![session],
        backlog: vec![replayed(SessionEvent::ModelChanged {
            from: "anthropic/sonnet".into(),
            model: "local/qwen".into(),
        })],
    };
    let app = App::new(settings(true, true), attachment, Instant::now());
    assert_eq!(notices(&app), ["model: anthropic/sonnet → local/qwen"]);
    assert_eq!(app.session.model, "local/qwen");
}

#[test]
fn an_old_log_without_from_names_only_the_new_model() {
    let app = app_with(vec![SessionEvent::ModelChanged {
        from: String::new(),
        model: "local/qwen".into(),
    }]);
    assert_eq!(notices(&app), ["model: local/qwen"]);
}

// The `models` request and the picker.

fn model(provider: &str, name: &str, id: &str, window: u64) -> ModelInfo {
    ModelInfo {
        model: format!("{provider}/{name}"),
        provider: provider.to_owned(),
        name: name.to_owned(),
        id: id.to_owned(),
        context_window: window,
        default: false,
        key: "env".to_owned(),
    }
}

fn model_list() -> Vec<ModelInfo> {
    vec![
        ModelInfo {
            default: true,
            ..model("anthropic", "sonnet", "claude-sonnet-5-5", 200_000)
        },
        model("anthropic", "opus", "claude-opus-5-5", 200_000),
        ModelInfo {
            key: "missing".to_owned(),
            ..model("local", "qwen", "qwen3-coder:30b", 32_768)
        },
    ]
}

fn models_answer(models: Vec<ModelInfo>) -> Input {
    Input::Response {
        request: Request::Models,
        result: Ok(Payload::Models {
            models,
            config: "/home/u/.config/clai/config.toml".into(),
        }),
    }
}

/// An app with the picker open on the usual list.
fn picking() -> App {
    let mut app = app();
    assert_eq!(requests(&app.update(ctrl('o'))), [Request::Models]);
    assert!(app.picker.is_none(), "it opens with the answer");
    app.update(models_answer(model_list()));
    assert!(app.picker.is_some());
    app
}

fn chosen(app: &App) -> String {
    let picker = app.picker.as_ref().unwrap();
    picker.chosen().unwrap().model.clone()
}

#[test]
fn slash_model_opens_the_picker() {
    let mut app = app();
    let effects = command(&mut app, "/model");
    assert_eq!(requests(&effects), [Request::Models]);
    assert!(app.editor.is_empty());
    app.update(models_answer(model_list()));
    assert!(app.picker.is_some());
}

#[test]
fn ctrl_o_works_in_the_conversation_too() {
    let mut app = app();
    app.update(key(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Conversation);
    assert_eq!(requests(&app.update(ctrl('o'))), [Request::Models]);
}

#[test]
fn the_answer_replaces_the_windows() {
    let mut app = app();
    assert_eq!(app.context_window(), Some(200_000));
    app.update(models_answer(vec![model(
        "anthropic",
        "sonnet",
        "x",
        50_000,
    )]));
    assert_eq!(app.context_window(), Some(50_000));
    // Without having asked for the picker, it does not open.
    assert!(app.picker.is_none());
    app.update(models_answer(vec![model("other", "m", "x", 1)]));
    assert_eq!(app.context_window(), None);
}

#[test]
fn a_reload_asks_for_the_models() {
    let mut app = app();
    let effects = app.update(Input::Server(ServerMessage::Notice {
        notice: Notice::ConfigReloaded,
    }));
    assert_eq!(requests(&effects), [Request::Models]);
}

#[test]
fn a_reload_refreshes_an_open_picker() {
    let mut app = picking();
    app.update(key(KeyCode::Down));
    assert_eq!(chosen(&app), "anthropic/opus");
    let mut newer = model_list();
    newer.remove(0);
    app.update(models_answer(newer));
    assert_eq!(chosen(&app), "anthropic/opus");
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.visible().len(), 2);
}

#[test]
fn a_failed_models_request_goes_to_the_conversation() {
    let mut app = app();
    app.update(ctrl('o'));
    app.update(Input::Response {
        request: Request::Models,
        result: Err("bad config".to_owned()),
    });
    assert!(app.picker.is_none());
    assert_eq!(kinds(&app), [Kind::Error("bad config".into())]);
    // A lost connection is no news, and a refresh fails quietly.
    let mut app = self::app();
    app.update(ctrl('o'));
    app.update(Input::Response {
        request: Request::Models,
        result: Err(LOST.to_owned()),
    });
    assert!(kinds(&app).is_empty());
    let mut app = self::app();
    app.update(Input::Response {
        request: Request::Models,
        result: Err("bad config".to_owned()),
    });
    assert!(kinds(&app).is_empty());
}

#[test]
fn the_picker_starts_on_the_current_model() {
    let mut session_on_opus = app();
    session_on_opus.session.model = "anthropic/opus".into();
    session_on_opus.update(ctrl('o'));
    session_on_opus.update(models_answer(model_list()));
    assert_eq!(chosen(&session_on_opus), "anthropic/opus");
}

#[test]
fn up_and_down_select_and_stop_at_the_ends() {
    let mut app = picking();
    assert_eq!(chosen(&app), "anthropic/sonnet");
    app.update(key(KeyCode::Up));
    assert_eq!(chosen(&app), "anthropic/sonnet");
    app.update(key(KeyCode::Down));
    app.update(key(KeyCode::Down));
    assert_eq!(chosen(&app), "local/qwen");
    app.update(key(KeyCode::Down));
    assert_eq!(chosen(&app), "local/qwen");
}

#[test]
fn typing_filters_by_provider_name_and_id() {
    for (filter, expected) in [
        ("local", vec!["local/qwen"]),
        ("OPUS", vec!["anthropic/opus"]),
        ("coder:30", vec!["local/qwen"]),
        ("claude-", vec!["anthropic/sonnet", "anthropic/opus"]),
        ("anthropic/s", vec!["anthropic/sonnet"]),
    ] {
        let mut app = picking();
        typed(&mut app, filter);
        let picker = app.picker.as_ref().unwrap();
        let shown: Vec<&str> = picker.visible().iter().map(|m| m.model.as_str()).collect();
        assert_eq!(shown, expected, "{filter}");
    }
    let mut app = picking();
    typed(&mut app, "qwenx");
    app.update(key(KeyCode::Backspace));
    assert_eq!(chosen(&app), "local/qwen");
    // Typing goes to the filter, not to the input.
    assert!(app.editor.is_empty());
}

#[test]
fn paste_goes_to_the_filter() {
    let mut app = picking();
    app.update(Input::Paste("qwen\n".to_owned()));
    assert!(app.editor.is_empty());
    let picker = app.picker.as_ref().unwrap();
    assert_eq!(picker.filter.text(), "qwen ");
}

#[test]
fn enter_sets_the_selected_model_and_closes() {
    let mut app = picking();
    app.update(key(KeyCode::Down));
    let effects = app.update(key(KeyCode::Enter));
    assert_eq!(
        requests(&effects),
        [Request::SetModel {
            session: "project".into(),
            model: "anthropic/opus".into(),
        }]
    );
    assert!(app.picker.is_none());
}

#[test]
fn enter_on_the_current_model_only_closes() {
    let mut app = picking();
    let effects = app.update(key(KeyCode::Enter));
    assert!(requests(&effects).is_empty());
    assert!(app.picker.is_none());
}

#[test]
fn enter_with_no_match_does_nothing() {
    let mut app = picking();
    typed(&mut app, "zzz");
    let effects = app.update(key(KeyCode::Enter));
    assert!(requests(&effects).is_empty());
    assert!(app.picker.is_some());
}

#[test]
fn esc_closes_and_cancels_nothing() {
    let mut app = picking();
    app.session.state = State::Working;
    let effects = app.update(key(KeyCode::Esc));
    assert!(requests(&effects).is_empty());
    assert!(app.picker.is_none());
}

#[test]
fn a_model_can_be_chosen_during_a_turn() {
    let mut app = picking();
    app.session.state = State::Working;
    app.update(key(KeyCode::Down));
    assert_eq!(requests(&app.update(key(KeyCode::Enter))).len(), 1);
}

#[test]
fn the_picker_does_not_open_while_a_prompt_is_open() {
    let mut app = with_prompt(idle_session());
    let effects = app.update(ctrl('o'));
    assert!(requests(&effects).is_empty());
    assert!(app.picker.is_none());
    // An answer that was on its way does not open it either.
    app.update(models_answer(model_list()));
    assert!(app.picker.is_none());
}

#[test]
fn a_question_closes_the_picker_and_keeps_the_keys_from_answering_it() {
    let mut app = picking();
    app.update(live(SessionEvent::Question(Box::new(limit()))));
    assert!(app.prompt.is_some());
    assert!(app.picker.is_none());
    // The answer of a request made just before the question does not open it.
    let mut app = self::app();
    app.update(ctrl('o'));
    app.update(live(SessionEvent::Question(Box::new(limit()))));
    app.update(models_answer(model_list()));
    assert!(app.picker.is_none());
}

#[test]
fn the_end_of_the_session_closes_the_picker() {
    let mut app = picking();
    app.update(live(SessionEvent::Ended {
        reason: "killed".into(),
    }));
    assert!(app.picker.is_none());
}

#[test]
fn frame_picker_with_marks() {
    let mut app = app_with(vec![usage(40_000, 1_000, 41_000)]);
    app.update(ctrl('o'));
    app.update(models_answer(model_list()));
    insta::assert_snapshot!(screen(&mut app, 80, 24));
}

#[test]
fn frame_picker_ascii() {
    let mut app = App::new(
        settings(false, true),
        attachment(vec![usage(40_000, 1_000, 41_000)]),
        Instant::now(),
    );
    app.update(ctrl('o'));
    app.update(models_answer(model_list()));
    let text = screen(&mut app, 80, 24);
    assert!(text.is_ascii(), "{text}");
    insta::assert_snapshot!(text);
}

#[test]
fn frame_picker_with_no_match() {
    let mut app = picking();
    typed(&mut app, "zzz");
    insta::assert_snapshot!(screen(&mut app, 80, 24));
}

#[test]
fn frame_picker_with_no_models() {
    let mut app = app();
    app.update(ctrl('o'));
    app.update(models_answer(Vec::new()));
    insta::assert_snapshot!(screen(&mut app, 80, 24));
}

#[test]
fn frame_picker_keeps_the_selected_row_in_a_small_box() {
    let mut app = app();
    app.update(ctrl('o'));
    let many: Vec<ModelInfo> = (0..30)
        .map(|i| model("p", &format!("m{i}"), &format!("id-{i}"), 1000))
        .collect();
    app.update(models_answer(many));
    for _ in 0..29 {
        app.update(key(KeyCode::Down));
    }
    let text = screen(&mut app, 60, 12);
    assert!(text.contains("m29"), "{text}");
    assert!(text.contains("Esc cancel"), "{text}");
    assert!(text.contains("Filter:"), "{text}");
}

// The command list.

fn listed(app: &App) -> Vec<&'static str> {
    app.listed().iter().map(|c| c.name).collect()
}

fn selected_command(app: &App) -> Option<&'static str> {
    let row = app.command_list?;
    app.listed().get(row).map(|c| c.name)
}

fn working(app: &mut App) {
    app.update(live(SessionEvent::State {
        state: State::Working,
    }));
}

#[test]
fn command_list_opens_on_a_slash_in_the_empty_input() {
    let mut app = app();
    assert!(app.command_list.is_none());
    typed(&mut app, "/");
    assert_eq!(app.command_list, Some(0));
    assert_eq!(listed(&app).len(), COMMANDS.len());
}

#[test]
fn command_list_does_not_open_otherwise() {
    // After other text.
    let mut app = app();
    typed(&mut app, "a/");
    assert!(app.command_list.is_none());
    // From a paste.
    let mut app = self::app();
    app.update(Input::Paste("/model".into()));
    assert!(app.command_list.is_none());
    // From a message of the history.
    let mut app = app_with(vec![SessionEvent::UserMessage {
        text: "/etc/passwd".into(),
    }]);
    app.update(key(KeyCode::Up));
    assert_eq!(app.editor.text(), "/etc/passwd");
    assert!(app.command_list.is_none());
    // With focus on the conversation.
    let mut app = self::app();
    app.update(key(KeyCode::Tab));
    typed(&mut app, "/");
    assert!(app.command_list.is_none());
}

#[test]
fn command_list_closes_when_the_text_is_no_search() {
    for (keys, text) in [("//", "//"), ("/model ", "/model ")] {
        let mut app = app();
        typed(&mut app, keys);
        assert!(app.command_list.is_none(), "{keys}");
        assert_eq!(app.editor.text(), text);
    }
    // A new line.
    let mut app = app();
    typed(&mut app, "/m");
    app.update(press(KeyCode::Enter, KeyModifiers::ALT));
    assert!(app.command_list.is_none());
    // `Backspace` on the `/`.
    let mut app = self::app();
    typed(&mut app, "/");
    app.update(key(KeyCode::Backspace));
    assert!(app.command_list.is_none());
    assert!(app.editor.is_empty());
    // `Ctrl-C` clears the input.
    let mut app = self::app();
    typed(&mut app, "/m");
    app.update(ctrl('c'));
    assert!(app.command_list.is_none());
    assert!(app.editor.is_empty());
}

#[test]
fn a_message_written_with_two_slashes_is_still_sent() {
    let mut app = app();
    assert_eq!(
        requests(&command(&mut app, "//text")),
        [Request::Send {
            session: "project".into(),
            text: "/text".into()
        }]
    );
}

#[test]
fn esc_only_closes_the_command_list() {
    let mut app = app();
    working(&mut app);
    typed(&mut app, "/m");
    // The first closes the list; the text stays, the turn goes on.
    assert!(requests(&app.update(key(KeyCode::Esc))).is_empty());
    assert!(app.command_list.is_none());
    assert_eq!(app.editor.text(), "/m");
    // The second cancels the turn.
    assert_eq!(
        requests(&app.update(key(KeyCode::Esc))),
        [Request::Cancel {
            session: "project".into()
        }]
    );
}

#[test]
fn a_closed_command_list_opens_again_only_from_an_empty_input() {
    let mut app = app();
    typed(&mut app, "/m");
    app.update(key(KeyCode::Esc));
    typed(&mut app, "o");
    assert!(app.command_list.is_none());
    app.update(key(KeyCode::Backspace));
    assert!(app.command_list.is_none());
    app.update(key(KeyCode::Backspace));
    app.update(key(KeyCode::Backspace));
    assert!(app.editor.is_empty());
    typed(&mut app, "/");
    assert_eq!(app.command_list, Some(0));
}

#[test]
fn up_and_down_select_in_the_command_list() {
    let mut app = app_with(vec![SessionEvent::UserMessage {
        text: "earlier".into(),
    }]);
    typed(&mut app, "/");
    app.update(key(KeyCode::Up));
    assert_eq!(app.command_list, Some(0));
    assert_eq!(app.editor.text(), "/");
    app.update(key(KeyCode::Down));
    assert_eq!(selected_command(&app), Some("clear"));
    for _ in 0..COMMANDS.len() * 2 {
        app.update(key(KeyCode::Down));
    }
    assert_eq!(selected_command(&app), Some("detach"));
    assert_eq!(app.editor.text(), "/");
}

#[test]
fn a_new_search_selects_the_best_match() {
    let mut app = app();
    typed(&mut app, "/");
    app.update(key(KeyCode::Down));
    app.update(key(KeyCode::Down));
    typed(&mut app, "m");
    assert_eq!(selected_command(&app), Some("model"));
    assert_eq!(
        listed(&app),
        ["model", "permissions", "rename", "theme", "commit"]
    );
    // Moving the cursor is no new search.
    app.update(key(KeyCode::Down));
    app.update(key(KeyCode::Left));
    assert_eq!(selected_command(&app), Some("permissions"));
    // A paste is.
    app.update(key(KeyCode::Right));
    app.update(Input::Paste("o".into()));
    assert_eq!(selected_command(&app), Some("model"));
}

#[test]
fn tab_completes_the_selected_command() {
    let mut app = app();
    typed(&mut app, "/mo");
    app.update(key(KeyCode::Tab));
    assert_eq!(app.editor.text(), "/model ");
    assert!(app.command_list.is_none());
    assert_eq!(app.focus, Focus::Input);
    // Then it is typed as before.
    typed(&mut app, "local/qwen");
    assert_eq!(
        requests(&app.update(key(KeyCode::Enter))),
        [Request::SetModel {
            session: "project".into(),
            model: "local/qwen".into()
        }]
    );
    // No space after a command without arguments.
    let mut app = self::app();
    typed(&mut app, "/he");
    app.update(key(KeyCode::Tab));
    assert_eq!(app.editor.text(), "/help");
    // With no match, nothing happens, and focus stays.
    let mut app = self::app();
    typed(&mut app, "/zzz");
    app.update(key(KeyCode::Tab));
    assert_eq!(app.editor.text(), "/zzz");
    assert!(app.command_list.is_some());
    assert_eq!(app.focus, Focus::Input);
}

#[test]
fn enter_runs_the_selected_command() {
    let mut app = app();
    command(&mut app, "/he");
    assert_eq!(app.dialog, Some(Dialog::Help));
    assert!(app.editor.is_empty());
    assert!(app.command_list.is_none());
    // `q` finds `/detach` by its other name.
    let mut app = self::app();
    assert_eq!(exit(&command(&mut app, "/q")), detached("project"));
    // A selection counts, not the text.
    let mut app = self::app();
    typed(&mut app, "/m");
    app.update(key(KeyCode::Down));
    app.update(key(KeyCode::Down));
    assert_eq!(selected_command(&app), Some("rename"));
    app.update(key(KeyCode::Enter));
    assert_eq!(app.editor.text(), "/rename ");
}

#[test]
fn an_unbuilt_command_says_so_from_the_list() {
    let mut app = app();
    command(&mut app, "/comm");
    assert_eq!(notices(&app), ["`/commit` comes with a later version"]);
    assert!(app.editor.is_empty());
}

#[test]
fn every_name_of_the_table_is_known() {
    for command in COMMANDS {
        for name in std::iter::once(command.name).chain(command.alias) {
            let mut app = app();
            typed(&mut app, &format!("/{name}"));
            // Closed, so the text goes to the command as it stands.
            app.update(key(KeyCode::Esc));
            app.update(key(KeyCode::Enter));
            assert!(
                !notices(&app).iter().any(|n| n.starts_with("unknown")),
                "{name}"
            );
        }
    }
}

/// The command of each box. No `_` arm: a new dialog needs a command to compile.
fn command_of(dialog: Dialog) -> &'static str {
    match dialog {
        Dialog::Help => "/help",
        Dialog::ConfirmKill => "/kill",
    }
}

#[test]
fn every_box_opens_from_the_command_list() {
    for dialog in [Dialog::Help, Dialog::ConfirmKill] {
        let mut app = app();
        command(&mut app, command_of(dialog));
        assert_eq!(app.dialog, Some(dialog));
    }
    let mut app = app();
    assert_eq!(requests(&command(&mut app, "/model")), [Request::Models]);
    app.update(models_answer(model_list()));
    assert!(app.picker.is_some());
    assert!(app.editor.is_empty());
}

#[test]
fn ctrl_o_closes_the_command_list_and_keeps_the_text() {
    let mut app = app();
    typed(&mut app, "/m");
    assert_eq!(requests(&app.update(ctrl('o'))), [Request::Models]);
    assert!(app.command_list.is_none());
    assert_eq!(app.editor.text(), "/m");
}

#[test]
fn a_picker_that_opens_late_closes_the_command_list() {
    let mut app = app();
    app.update(ctrl('o'));
    typed(&mut app, "/m");
    assert!(app.command_list.is_some());
    app.update(models_answer(model_list()));
    assert!(app.picker.is_some());
    assert!(app.command_list.is_none());
    assert_eq!(app.editor.text(), "/m");
}

#[test]
fn a_question_closes_the_command_list_and_keeps_the_text() {
    let mut app = app();
    typed(&mut app, "/m");
    app.update(live(SessionEvent::Question(Box::new(permission()))));
    assert!(app.command_list.is_none());
    assert!(app.prompt.is_some());
    assert_eq!(app.editor.text(), "/m");
}

#[test]
fn the_end_of_the_session_closes_the_command_list() {
    let mut app = app();
    typed(&mut app, "/m");
    app.update(live(SessionEvent::Ended {
        reason: "killed".into(),
    }));
    assert!(app.command_list.is_none());
}

#[test]
fn scrolling_works_with_the_command_list_open() {
    let mut app = idle_session();
    screen(&mut app, 80, 24);
    typed(&mut app, "/");
    app.update(key(KeyCode::PageUp));
    assert!(!app.follow);
    assert_eq!(app.command_list, Some(0));
}

#[test]
fn frame_command_list() {
    let mut app = idle_session();
    typed(&mut app, "/");
    insta::assert_snapshot!(screen(&mut app, 80, 24));
}

#[test]
fn frame_command_list_with_a_search() {
    let mut app = idle_session();
    typed(&mut app, "/m");
    app.update(key(KeyCode::Down));
    insta::assert_snapshot!(screen(&mut app, 80, 24));
}

#[test]
fn frame_command_list_with_no_match() {
    let mut app = idle_session();
    typed(&mut app, "/zzz");
    insta::assert_snapshot!(screen(&mut app, 80, 24));
}

#[test]
fn frame_command_list_ascii() {
    let mut app = App::new(
        settings(false, true),
        attachment(Vec::new()),
        Instant::now(),
    );
    typed(&mut app, "/");
    let text = screen(&mut app, 80, 24);
    assert!(text.is_ascii(), "{text}");
    insta::assert_snapshot!(text);
}

#[test]
fn frame_command_list_keeps_the_selected_row_on_a_small_screen() {
    let mut app = app();
    typed(&mut app, "/");
    for _ in 0..COMMANDS.len() {
        app.update(key(KeyCode::Down));
    }
    let text = screen(&mut app, 40, 10);
    assert!(text.contains("/detach, /quit"), "{text}");
    assert!(!text.contains("Show keys"), "{text}");
    insta::assert_snapshot!(text);
}

// Sessions: switching, the session list, new sessions (RFC 0005).

fn session(id: u64, name: &str) -> SessionInfo {
    SessionInfo {
        id,
        name: name.to_owned(),
        ..info()
    }
}

fn three() -> Vec<SessionInfo> {
    vec![session(1, "project"), session(2, "docs"), session(3, "web")]
}

fn sessions_answer(sessions: Vec<SessionInfo>) -> Input {
    Input::Response {
        request: Request::List,
        result: Ok(Payload::Sessions { sessions }),
    }
}

/// An app that knows three sessions, attached to the first.
fn among_three() -> App {
    let mut app = app();
    app.update(sessions_answer(three()));
    app
}

fn switches(effects: &[Effect]) -> Vec<String> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Switch(name) => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// The attachment a switch to session `id` brings.
fn attached_to(id: u64, name: &str, events: Vec<SessionEvent>) -> Input {
    Input::Attached(Box::new(Attachment {
        session: session(id, name),
        sessions: three(),
        backlog: events
            .into_iter()
            .map(|event| ServerMessage::Event {
                session: id,
                replay: true,
                event,
            })
            .collect(),
    }))
}

#[test]
fn ctrl_n_and_ctrl_p_go_round_the_sessions() {
    let mut app = among_three();
    assert_eq!(switches(&app.update(ctrl('n'))), ["docs"]);
    assert_eq!(app.switching.as_deref(), Some("docs"));
    assert!(screen(&mut app, 80, 24).contains("attaching docs"));
    // One switch at a time.
    assert!(switches(&app.update(ctrl('n'))).is_empty());
    app.update(Input::SwitchFailed("no session named docs".into()));
    assert!(app.switching.is_none());
    assert!(kinds(&app).contains(&Kind::Error("no session named docs".into())));
    // Backwards from the first is the last.
    assert_eq!(switches(&app.update(ctrl('p'))), ["web"]);
}

#[test]
fn with_one_session_ctrl_n_does_nothing() {
    let mut app = app();
    app.update(sessions_answer(vec![info()]));
    assert!(switches(&app.update(ctrl('n'))).is_empty());
    assert!(switches(&app.update(ctrl('p'))).is_empty());
}

#[test]
fn a_switch_keeps_the_view_of_each_session() {
    let mut app = among_three();
    typed(&mut app, "half a thought");
    app.update(ctrl('n'));
    app.update(attached_to(
        2,
        "docs",
        vec![SessionEvent::UserMessage {
            text: "in docs".into(),
        }],
    ));
    assert_eq!(app.session.name, "docs");
    assert!(app.switching.is_none());
    assert!(app.editor.is_empty());
    assert_eq!(kinds(&app), [Kind::User("in docs".into())]);
    // Events of the session left are not shown.
    app.update(live(SessionEvent::Notice {
        text: "from project".into(),
    }));
    assert_eq!(kinds(&app).len(), 1);
    typed(&mut app, "x");
    app.update(attached_to(1, "project", Vec::new()));
    assert_eq!(app.editor.text(), "half a thought");
    app.update(attached_to(2, "docs", Vec::new()));
    assert_eq!(app.editor.text(), "x");
}

#[test]
fn attach_by_name() {
    let mut app = among_three();
    // Not in the last list: a notice, and the input stays.
    let effects = command(&mut app, "/attach nope");
    assert!(switches(&effects).is_empty());
    assert_eq!(
        notices(&app),
        ["no session named nope; /sessions lists them"]
    );
    assert_eq!(app.editor.text(), "/attach nope");
    app.editor.clear();
    assert_eq!(switches(&command(&mut app, "/attach web")), ["web"]);
    assert!(app.editor.is_empty());
    // The attached one: nothing.
    let mut app = among_three();
    assert!(switches(&command(&mut app, "/attach project")).is_empty());
}

#[test]
fn the_session_list_opens_with_the_answer() {
    for open in ["ctrl-s", "/sessions", "/attach"] {
        let mut app = among_three();
        let effects = if open == "ctrl-s" {
            app.update(ctrl('s'))
        } else {
            command(&mut app, open)
        };
        assert_eq!(requests(&effects), [Request::List], "{open}");
        assert!(app.session_list.is_none());
        app.update(sessions_answer(three()));
        let list = app.session_list.as_ref().expect(open);
        assert_eq!(list.chosen().unwrap().name, "project");
    }
}

/// The session list open on three sessions.
fn listing() -> App {
    let mut app = among_three();
    app.update(ctrl('s'));
    app.update(sessions_answer(three()));
    app
}

#[test]
fn enter_in_the_list_switches() {
    let mut app = listing();
    app.update(key(KeyCode::Down));
    assert_eq!(switches(&app.update(key(KeyCode::Enter))), ["docs"]);
    app.update(attached_to(2, "docs", Vec::new()));
    assert!(app.session_list.is_none());
    // Enter on the attached session only closes.
    let mut app = listing();
    assert!(switches(&app.update(key(KeyCode::Enter))).is_empty());
    assert!(app.session_list.is_none());
    // Esc closes and leaves nothing.
    let mut app = listing();
    assert!(exit(&app.update(key(KeyCode::Esc))).is_none());
    assert!(app.session_list.is_none());
}

#[test]
fn rename_and_kill_from_the_list() {
    let mut app = listing();
    app.update(key(KeyCode::Down));
    app.update(ch('r'));
    typed(&mut app, "2");
    let effects = app.update(key(KeyCode::Enter));
    assert_eq!(
        requests(&effects),
        [Request::Rename {
            session: "docs".into(),
            name: "docs2".into()
        }]
    );
    // A refused name is shown in the box.
    app.update(Input::Response {
        request: requests(&effects).remove(0),
        result: Err("the name docs2 is taken".into()),
    });
    assert_eq!(
        app.session_list.as_ref().unwrap().message.as_deref(),
        Some("the name docs2 is taken")
    );
    assert!(screen(&mut app, 80, 24).contains("the name docs2 is taken"));
    app.update(ch('d'));
    assert!(screen(&mut app, 80, 24).contains("Delete docs? No turns yet."));
    assert_eq!(
        requests(&app.update(ch('y'))),
        [Request::Kill {
            session: "docs".into()
        }]
    );
}

#[test]
fn a_question_closes_the_list() {
    let mut app = listing();
    app.update(live(SessionEvent::Question(Box::new(permission()))));
    assert!(app.session_list.is_none());
    // And a list asked for does not open over the prompt.
    app.update(sessions_answer(three()));
    assert!(app.session_list.is_none());
}

/// The models, with `opus` as `default_model`: not the session's model.
fn default_opus() -> Vec<ModelInfo> {
    model_list()
        .into_iter()
        .map(|m| ModelInfo {
            default: m.name == "opus",
            ..m
        })
        .collect()
}

#[test]
fn new_starts_on_the_default_model() {
    let mut app = among_three();
    assert_eq!(requests(&command(&mut app, "/new")), [Request::Models]);
    app.update(models_answer(default_opus()));
    let picker = app.picker.as_ref().expect("picker");
    assert_eq!(picker.chosen().unwrap().model, "anthropic/opus");
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("Model for a new session"), "{text}");
    // No `current` mark: the new session has no model yet.
    assert!(!text.contains("current"), "{text}");
    // One Enter: the default model.
    let effects = app.update(key(KeyCode::Enter));
    assert_eq!(
        requests(&effects),
        [Request::New {
            workspace: "/ws/project".into(),
            name: None,
            model: Some("anthropic/opus".into()),
        }]
    );
    assert!(app.picker.is_none());
    // The answer switches to it, with a note: the workspace is shared.
    let effects = app.update(Input::Response {
        request: requests(&effects).remove(0),
        result: Ok(Payload::Session {
            session: Box::new(session(4, "project-2")),
        }),
    });
    assert_eq!(switches(&effects), ["project-2"]);
    app.update(attached_to(4, "project-2", Vec::new()));
    assert_eq!(
        notices(&app),
        ["note: project, docs, web also work in ~/ws/project; sessions may change the same files"]
    );
}

#[test]
fn new_with_a_name_and_another_model() {
    let mut app = among_three();
    command(&mut app, "/new spike");
    app.update(models_answer(default_opus()));
    app.update(key(KeyCode::Down));
    assert_eq!(
        requests(&app.update(key(KeyCode::Enter))),
        [Request::New {
            workspace: "/ws/project".into(),
            name: Some("spike".into()),
            model: Some("local/qwen".into()),
        }]
    );
    // Esc creates nothing.
    let mut app = among_three();
    command(&mut app, "/new");
    app.update(models_answer(default_opus()));
    assert!(requests(&app.update(key(KeyCode::Esc))).is_empty());
    assert!(app.picker.is_none());
}

#[test]
fn n_in_the_list_asks_workspace_and_name_then_the_model() {
    let mut app = listing();
    app.update(ch('n'));
    assert!(screen(&mut app, 80, 24).contains("Workspace: ~/ws/project"));
    app.update(key(KeyCode::Enter));
    typed(&mut app, "side");
    let effects = app.update(key(KeyCode::Enter));
    assert_eq!(requests(&effects), [Request::Models]);
    assert!(app.session_list.is_none());
    app.update(models_answer(default_opus()));
    let effects = app.update(key(KeyCode::Enter));
    let sent = requests(&effects);
    let [Request::New { name, model, .. }] = sent.as_slice() else {
        panic!("{effects:?}");
    };
    assert_eq!(name.as_deref(), Some("side"));
    assert_eq!(model.as_deref(), Some("anthropic/opus"));
}

#[test]
fn the_list_opens_when_the_session_ended() {
    let mut app = among_three();
    let effects = app.update(live(SessionEvent::Ended {
        reason: "killed".into(),
    }));
    assert_eq!(requests(&effects), [Request::List]);
    app.update(sessions_answer(vec![session(2, "docs"), session(3, "web")]));
    let list = app.session_list.as_ref().expect("list");
    assert!(list.exit_on_esc);
    // The list keeps up while it is open.
    app.update(Input::Tick);
    assert_eq!(requests(&app.update(Input::Tick)), [Request::List]);
    // Another session: attached, and the TUI goes on.
    assert_eq!(switches(&app.update(key(KeyCode::Enter))), ["docs"]);
    app.update(attached_to(2, "docs", Vec::new()));
    assert!(app.ended.is_none());
    assert!(exit(&app.update(ch('x'))).is_none());

    // Esc leaves, as the session ended.
    let mut app = among_three();
    app.update(live(SessionEvent::Ended {
        reason: "killed".into(),
    }));
    app.update(sessions_answer(Vec::new()));
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("no sessions"), "{text}");
    assert!(text.contains("n new ─ Esc exit"), "{text}");
    assert_eq!(
        exit(&app.update(key(KeyCode::Esc))),
        Some(Exit::Ended {
            name: "project".into(),
            reason: "killed".into()
        })
    );
}

#[test]
fn a_stopping_server_opens_no_list() {
    let mut app = among_three();
    app.update(Input::Server(ServerMessage::Notice {
        notice: Notice::Stopping,
    }));
    let effects = app.update(live(SessionEvent::Ended {
        reason: "the server stopped".into(),
    }));
    assert!(requests(&effects).is_empty());
}

#[test]
fn attach_without_a_name_chooses_in_the_list() {
    let mut app = app();
    app.sessions = three();
    app.choose_session();
    assert!(app.session_list.as_ref().unwrap().exit_on_esc);
    assert_eq!(exit(&app.update(key(KeyCode::Esc))), detached("project"));
}

/// Sessions of all states, made at fixed ages before now.
fn various() -> Vec<SessionInfo> {
    let now = crate::agent::now_secs();
    vec![
        SessionInfo {
            created: now - 12 * 60,
            updated: 0,
            turns: 0,
            state: State::Working,
            ..session(1, "project")
        },
        SessionInfo {
            created: now - 2 * 3600,
            updated: 0,
            turns: 0,
            model: "local/qwen".into(),
            ..session(2, "project-2")
        },
        SessionInfo {
            created: now - 3 * 60,
            updated: 0,
            turns: 0,
            state: State::Waiting,
            workspace: "~/ws/docs".into(),
            model: "anthropic/opus".into(),
            question: Some(permission()),
            ..session(3, "docs")
        },
        SessionInfo {
            created: now - 30 * 86_400,
            updated: now - 12 * 86_400,
            turns: 14,
            usage: TokenTotals {
                input_tokens: 200_000,
                output_tokens: 10_000,
            },
            state: State::Saved,
            workspace: "~/ws/old".into(),
            ..session(4, "old")
        },
    ]
}

fn listing_various(unicode: bool) -> App {
    let mut app = App::new(
        settings(unicode, true),
        attachment(Vec::new()),
        Instant::now(),
    );
    app.update(ctrl('s'));
    app.update(sessions_answer(various()));
    app.update(key(KeyCode::Down));
    app
}

#[test]
fn frame_session_list() {
    insta::assert_snapshot!(screen(&mut listing_various(true), 80, 24));
}

#[test]
fn frame_session_list_ascii() {
    let text = screen(&mut listing_various(false), 80, 24);
    assert!(text.is_ascii(), "{text}");
    insta::assert_snapshot!(text);
}

#[test]
fn frame_session_list_narrow() {
    let text = screen(&mut listing_various(true), 44, 12);
    assert!(!text.contains("USED"), "{text}");
    insta::assert_snapshot!(text);
}

#[test]
fn frame_session_list_marked_and_filtered() {
    let mut app = listing_various(true);
    app.update(ch(' '));
    app.update(ch('/'));
    typed(&mut app, "project");
    app.update(key(KeyCode::Enter));
    let text = screen(&mut app, 80, 24);
    assert!(
        text.contains("filter: project ─ 2 of 4 ─ 1 marked"),
        "{text}"
    );
    insta::assert_snapshot!(text);
}

#[test]
fn frame_session_list_filter_without_match() {
    let mut app = listing_various(true);
    app.update(ch('/'));
    typed(&mut app, "saved >30d");
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("no session matches"), "{text}");
    assert!(text.contains("Filter: saved >30d"), "{text}");
}

#[test]
fn frame_session_list_delete_one() {
    let mut app = listing_various(true);
    app.update(key(KeyCode::Down));
    app.update(key(KeyCode::Down));
    app.update(ch('d'));
    let text = screen(&mut app, 80, 24);
    assert!(
        text.contains("Delete old? 14 turns, 210k tokens. [y] yes  [n] no"),
        "{text}"
    );
    insta::assert_snapshot!(text);
}

#[test]
fn frame_session_list_delete_several() {
    let mut app = listing_various(true);
    app.update(ch('a'));
    app.update(ch('d'));
    let text = screen(&mut app, 80, 24);
    assert!(
        text.contains("Delete 4 sessions? 1 is working, 1 waits for an answer."),
        "{text}"
    );
    insta::assert_snapshot!(text);
}

#[test]
fn deleting_marked_sessions_sends_one_kill_each() {
    let mut app = listing_various(true);
    app.update(ch('a'));
    app.update(ch('d'));
    let kills = requests(&app.update(ch('y')));
    let names: Vec<&str> = kills
        .iter()
        .map(|r| match r {
            Request::Kill { session } => session.as_str(),
            other => panic!("{other:?}"),
        })
        .collect();
    assert_eq!(names, ["project", "project-2", "docs", "old"]);
    // The result line waits for all answers; then the list is asked again.
    let answer = |name: &str, result: Result<Payload, String>| Input::Response {
        request: Request::Kill {
            session: name.into(),
        },
        result,
    };
    for name in ["project-2", "docs", "old"] {
        let effects = app.update(answer(name, Ok(Payload::Done)));
        assert!(requests(&effects).is_empty());
    }
    assert!(!screen(&mut app, 80, 24).contains("deleted"));
    let effects = app.update(answer("project", Err("no session named project".into())));
    assert_eq!(requests(&effects), [Request::List]);
    let text = screen(&mut app, 80, 24);
    assert!(
        text.contains("deleted 3 sessions; project: no session named project"),
        "{text}"
    );
    insta::assert_snapshot!(text);
    // The attached session among them: the list stays open.
    assert!(app.session_list.is_some());
}

#[test]
fn frame_session_list_rename() {
    let mut app = listing_various(true);
    app.update(ch('r'));
    insta::assert_snapshot!(screen(&mut app, 80, 24));
}

#[test]
fn frame_picker_for_a_new_session() {
    let mut app = among_three();
    command(&mut app, "/new");
    app.update(models_answer(default_opus()));
    insta::assert_snapshot!(screen(&mut app, 80, 24));
}

// Argument completion.

fn values(app: &App) -> Vec<String> {
    app.arguments().into_iter().map(|v| v.value).collect()
}

#[test]
fn attach_offers_the_other_sessions() {
    let mut app = among_three();
    // Tab in the command list completes `/attach ` and opens the values.
    typed(&mut app, "/att");
    app.update(key(KeyCode::Tab));
    assert_eq!(app.editor.text(), "/attach ");
    assert_eq!(app.argument_list, Some(0));
    assert_eq!(values(&app), ["docs", "web"]);
    typed(&mut app, "w");
    assert_eq!(values(&app), ["web"]);
    // Enter runs it with the selected value.
    app.update(key(KeyCode::Backspace));
    app.update(key(KeyCode::Down));
    assert_eq!(switches(&app.update(key(KeyCode::Enter))), ["web"]);
}

#[test]
fn tab_completes_a_value_and_esc_closes_the_list() {
    let mut app = among_three();
    typed(&mut app, "/attach d");
    assert_eq!(app.argument_list, Some(0));
    app.update(key(KeyCode::Tab));
    assert_eq!(app.editor.text(), "/attach docs");
    assert!(app.argument_list.is_none());
    // Esc closes it, and it stays closed while the text is edited.
    let mut app = among_three();
    typed(&mut app, "/attach ");
    let effects = app.update(key(KeyCode::Esc));
    assert!(app.argument_list.is_none());
    assert!(requests(&effects).is_empty());
    typed(&mut app, "x");
    assert!(app.argument_list.is_none());
    // A second word closes it too.
    let mut app = among_three();
    typed(&mut app, "/attach docs ");
    assert!(app.argument_list.is_none());
}

#[test]
fn model_offers_the_models_from_the_server() {
    let mut app = app();
    typed(&mut app, "/model");
    let effects = app.update(ch(' '));
    // Not known yet: asked for once, without opening the picker.
    assert_eq!(requests(&effects), [Request::Models]);
    assert!(requests(&app.update(ch('q'))).is_empty());
    app.update(models_answer(model_list()));
    assert!(app.picker.is_none());
    assert_eq!(values(&app), ["local/qwen"]);
    app.update(key(KeyCode::Backspace));
    typed(&mut app, "claude-o");
    assert_eq!(values(&app), ["anthropic/opus"]);
    assert_eq!(
        requests(&app.update(key(KeyCode::Enter))),
        [Request::SetModel {
            session: "project".into(),
            model: "anthropic/opus".into()
        }]
    );
}

#[test]
fn with_no_match_enter_runs_the_text() {
    let mut app = among_three();
    typed(&mut app, "/attach zzz");
    assert!(values(&app).is_empty());
    assert!(screen(&mut app, 80, 24).contains("no match"));
    app.update(key(KeyCode::Enter));
    assert_eq!(
        notices(&app),
        ["no session named zzz; /sessions lists them"]
    );
}

#[test]
fn frame_argument_list() {
    let mut app = among_three();
    typed(&mut app, "/attach ");
    insta::assert_snapshot!(screen(&mut app, 80, 24));
}

// Settings applied live (RFC 0006).

fn tui_with(f: impl FnOnce(&mut crate::config::Tui)) -> crate::config::Tui {
    let mut tui = app().settings.tui.clone();
    f(&mut tui);
    tui
}

#[test]
fn applied_settings_change_the_conversation() {
    let long = "x\n".repeat(30);
    let mut app = app_with(vec![
        SessionEvent::ToolCall(call("c1", "run_command", CallState::Running)),
        SessionEvent::ToolCall(done(call("c1", "run_command", CallState::Running), &long)),
    ]);
    let before = screen(&mut app, 80, 40);
    assert!(before.contains("▸ run_command"), "{before}");
    let effects = app.apply(tui_with(|t| {
        t.expand_tool_calls = true;
        t.tool_output_lines = 5;
        t.symbols = crate::config::Symbols::Ascii;
        t.scroll_lines = 7;
    }));
    assert!(effects.is_empty());
    let after = screen(&mut app, 80, 40);
    assert!(after.contains("v run_command"), "{after}");
    assert!(after.contains("25 more lines"), "{after}");
    assert!(after.is_ascii(), "{after}");
    assert_eq!(app.settings.scroll_lines, 7);
}

#[test]
fn mouse_and_frame_rate_go_to_the_shell() {
    let mut app = app();
    assert_eq!(app.frame_time(), Duration::from_secs(1) / 30);
    let effects = app.apply(tui_with(|t| {
        t.mouse = false;
        t.max_fps = 10;
    }));
    assert_eq!(effects, [Effect::Mouse(false)]);
    assert_eq!(app.frame_time(), Duration::from_millis(100));
    // The same again: nothing to do.
    assert!(app.apply(app.settings.tui.clone()).is_empty());
}

#[test]
fn timestamps_take_the_local_offset() {
    let mut app = app();
    app.apply(tui_with(|t| t.timestamps = true));
    assert_eq!(app.settings.timestamps, Some(3600));
    app.apply(tui_with(|t| t.timestamps = false));
    assert_eq!(app.settings.timestamps, None);
}

#[test]
fn a_reload_applies_the_file() {
    let mut app = app();
    let effects = app.update(Input::Server(ServerMessage::Notice {
        notice: Notice::ConfigReloaded,
    }));
    assert!(effects.contains(&Effect::ReloadConfig));
    let reloaded = super::app::Reloaded {
        tui: tui_with(|t| t.theme = "nord".into()),
        themes: app.settings.theming.themes.clone(),
        warnings: vec![warning("x")],
        file: super::app::FileTui::default(),
    };
    app.update(Input::Reloaded(Box::new(Ok(reloaded))));
    assert_eq!(app.settings.tui.theme, "nord");
    assert_eq!(app.settings.theming.warnings, [warning("x")]);
    assert_eq!(
        app.settings.look.styles.background,
        Some(Color::Indexed(237))
    );
    // A file that does not load: the settings stay, and the status says why.
    app.update(Input::Reloaded(Box::new(Err(
        "user.toml:3: tui.max_fps: 500 is not in 1–120".into(),
    ))));
    assert_eq!(app.settings.tui.theme, "nord");
    let text = screen(&mut app, 120, 24);
    assert!(text.contains("configuration: user.toml:3"), "{text}");
}

#[test]
fn a_theme_with_a_background_paints_the_screen() {
    let mut app = app();
    app.apply(tui_with(|t| t.theme = "atom-one-dark".into()));
    let terminal = screen_of(&mut app, 80, 24);
    let buffer = terminal.backend().buffer();
    // #282c34 as the nearest of 256 colours, everywhere.
    let bg = app.settings.look.styles.background.unwrap();
    assert!(
        buffer
            .content
            .iter()
            .all(|c| c.bg == bg || c.bg != Color::Reset)
    );
    assert!(buffer.content.iter().all(|c| c.bg != Color::Reset));
    assert!(buffer.content.iter().all(|c| c.fg != Color::Reset));
}

fn warning(name: &str) -> crate::config::ThemeWarning {
    crate::config::ThemeWarning {
        name: name.into(),
        message: format!("themes/{name}.toml:1: base: broken"),
    }
}

// The theme list and the settings box (RFC 0006).

use crate::config::edit::Setting;

fn saves(effects: &[Effect]) -> Vec<Vec<(&'static str, Option<Setting>)>> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Save { changes, .. } => Some(changes.clone()),
            _ => None,
        })
        .collect()
}

fn theme_saved(name: &str) -> Vec<Vec<(&'static str, Option<Setting>)>> {
    vec![vec![("theme", Some(Setting::Text(name.into())))]]
}

#[test]
fn theme_list_previews_and_esc_goes_back() {
    let mut app = app();
    command(&mut app, "/theme");
    let list = app.theme_list.as_ref().expect("list");
    assert_eq!(list.preview(), Some("terminal"));
    app.update(key(KeyCode::Down));
    assert_eq!(app.settings.tui.theme, "dark");
    assert_eq!(app.settings.look.styles.text.fg, Some(Color::Indexed(252)));
    typed(&mut app, "nord");
    assert_eq!(app.settings.tui.theme, "nord");
    let effects = app.update(key(KeyCode::Esc));
    assert!(saves(&effects).is_empty());
    assert!(app.theme_list.is_none());
    assert_eq!(app.settings.tui.theme, "terminal");
    assert_eq!(app.settings.look.styles.background, None);
}

#[test]
fn enter_takes_the_theme_and_saves_it_as_the_default() {
    let mut app = app();
    command(&mut app, "/theme");
    typed(&mut app, "gruv");
    let effects = app.update(key(KeyCode::Enter));
    assert_eq!(saves(&effects), theme_saved("gruvbox-dark"));
    let Some(Effect::Save { path, .. }) = effects.iter().find(|e| matches!(e, Effect::Save { .. }))
    else {
        panic!("{effects:?}");
    };
    assert_eq!(
        path,
        std::path::Path::new("/home/u/.config/clai/config.toml")
    );
    assert!(app.theme_list.is_none());
    assert_eq!(app.settings.tui.theme, "gruvbox-dark");
    app.update(Input::Saved(Ok(None)));
    assert!(screen(&mut app, 100, 24).contains("theme gruvbox-dark saved as the default"));
    assert_eq!(app.settings.theming.file.tui.theme, "gruvbox-dark");
    // A failed save is shown; the theme stays for this run.
    let mut app = self::app();
    command(&mut app, "/theme dark");
    app.update(Input::Saved(Err(
        "the file changed since it was read; open it again".into(),
    )));
    assert!(kinds(&app).contains(&Kind::Error(
        "not saved: the file changed since it was read; open it again".into()
    )));
    assert_eq!(app.settings.tui.theme, "dark");
}

#[test]
fn theme_by_name() {
    let mut app = app();
    assert_eq!(
        saves(&command(&mut app, "/theme nord")),
        theme_saved("nord")
    );
    assert!(app.editor.is_empty());
    let effects = command(&mut app, "/theme neon");
    assert!(saves(&effects).is_empty());
    assert_eq!(notices(&app), ["unknown theme neon; /theme lists them"]);
    assert_eq!(app.editor.text(), "/theme neon");
}

#[test]
fn a_broken_theme_file_cannot_be_taken() {
    let mut app = app();
    app.settings.theming.warnings = vec![warning("bad")];
    command(&mut app, "/theme");
    typed(&mut app, "bad");
    // The last theme that could be shown stays: `b` found gruvbox-dark.
    assert_eq!(
        app.theme_list.as_ref().unwrap().chosen().unwrap().name,
        "bad"
    );
    assert_eq!(app.settings.tui.theme, "gruvbox-dark");
    assert!(saves(&app.update(key(KeyCode::Enter))).is_empty());
    assert!(app.theme_list.is_some());
    let text = screen(&mut app, 100, 30);
    assert!(text.contains("themes/bad.toml:1: base: broken"), "{text}");
    // `/theme bad` neither.
    let mut app = self::app();
    app.settings.theming.warnings = vec![warning("bad")];
    assert!(saves(&command(&mut app, "/theme bad")).is_empty());
}

#[test]
fn theme_names_in_the_argument_list() {
    let mut app = app();
    typed(&mut app, "/theme so");
    assert_eq!(values(&app), ["solarized-light"]);
}

#[test]
fn clai_theme_wins_until_a_theme_is_taken() {
    let mut app = app();
    app.settings.theming.theme_env = Some("light".into());
    let reloaded = || super::app::Reloaded {
        tui: tui_with(|t| t.theme = "light".into()),
        themes: crate::config::themes::file_themes(&[]).0,
        warnings: Vec::new(),
        file: super::app::FileTui::default(),
    };
    app.update(Input::Reloaded(Box::new(Ok(reloaded()))));
    assert_eq!(app.settings.tui.theme, "light");
    command(&mut app, "/theme nord");
    app.update(Input::Reloaded(Box::new(Ok(reloaded()))));
    assert_eq!(app.settings.tui.theme, "nord");
}

/// The settings box on a file that sets `mouse = false`.
fn settings_open() -> App {
    let mut app = app();
    app.settings.theming.file.tui.mouse = false;
    app.settings.theming.file.keys = vec!["mouse".into()];
    app.apply(tui_with(|t| t.mouse = false));
    command(&mut app, "/settings");
    assert!(app.settings_box.is_some());
    app
}

fn select_row(app: &mut App, key_name: &str) {
    for _ in 0..10 {
        app.update(key(KeyCode::Up));
    }
    for _ in 0..10 {
        if app.settings_box.as_ref().unwrap().row().unwrap().key == key_name {
            return;
        }
        app.update(key(KeyCode::Down));
    }
    panic!("no row {key_name}");
}

#[test]
fn settings_preview_and_save() {
    let mut app = settings_open();
    select_row(&mut app, "mouse");
    let effects = app.update(key(KeyCode::Enter));
    assert!(effects.contains(&Effect::Mouse(true)));
    select_row(&mut app, "expand_tool_calls");
    app.update(key(KeyCode::Right));
    assert!(app.settings.expand_tool_calls);
    let effects = app.update(ch('s'));
    assert_eq!(
        saves(&effects),
        [vec![
            ("mouse", Some(Setting::Bool(true))),
            ("expand_tool_calls", Some(Setting::Bool(true)))
        ]]
    );
    app.update(Input::Saved(Ok(None)));
    let settings = app.settings_box.as_ref().expect("still open");
    assert!(settings.changes().is_empty());
    assert!(screen(&mut app, 100, 30).contains("saved"));
    assert!(app.settings.theming.file.tui.expand_tool_calls);
    assert!(
        app.settings
            .theming
            .file
            .keys
            .contains(&"expand_tool_calls".to_owned())
    );
    // Nothing changed now: Esc closes at once.
    app.update(key(KeyCode::Esc));
    assert!(app.settings_box.is_none());
}

#[test]
fn settings_discarded_go_back_to_the_file() {
    let mut app = settings_open();
    select_row(&mut app, "mouse");
    app.update(key(KeyCode::Enter));
    app.update(key(KeyCode::Esc));
    assert!(screen(&mut app, 100, 30).contains("save changes? [s] save  [n] discard"));
    let effects = app.update(ch('n'));
    assert!(app.settings_box.is_none());
    assert!(effects.contains(&Effect::Mouse(false)));
    assert!(!app.settings.tui.mouse);
}

#[test]
fn the_theme_row_opens_the_theme_list() {
    let mut app = settings_open();
    app.update(key(KeyCode::Enter));
    assert!(app.theme_list.as_ref().unwrap().for_settings);
    typed(&mut app, "nord");
    // Enter only sets the row; `s` in the box saves it.
    let effects = app.update(key(KeyCode::Enter));
    assert!(saves(&effects).is_empty());
    assert!(app.theme_list.is_none());
    assert_eq!(app.settings.tui.theme, "nord");
    assert_eq!(app.settings_box.as_ref().unwrap().current.theme, "nord");
    assert_eq!(saves(&app.update(ch('s'))), theme_saved("nord"));
}

#[test]
fn a_question_closes_the_settings_and_undoes_the_preview() {
    let mut app = settings_open();
    select_row(&mut app, "mouse");
    app.update(key(KeyCode::Enter));
    let effects = app.update(live(SessionEvent::Question(Box::new(permission()))));
    assert!(app.settings_box.is_none());
    assert!(effects.contains(&Effect::Mouse(false)));
    let mut app = self::app();
    command(&mut app, "/theme");
    app.update(key(KeyCode::Down));
    app.update(live(SessionEvent::Question(Box::new(permission()))));
    assert!(app.theme_list.is_none());
    assert_eq!(app.settings.tui.theme, "terminal");
}

#[test]
fn a_failed_save_shows_in_the_box() {
    let mut app = settings_open();
    select_row(&mut app, "timestamps");
    app.update(key(KeyCode::Enter));
    app.update(ch('s'));
    app.update(Input::Saved(Err("tui.theme: unknown theme `x`".into())));
    let settings = app.settings_box.as_ref().unwrap();
    assert_eq!(
        settings.message.as_deref(),
        Some("tui.theme: unknown theme `x`")
    );
    assert!(!settings.changes().is_empty());
}

#[test]
fn frame_settings() {
    let mut app = settings_open();
    select_row(&mut app, "mouse");
    app.update(key(KeyCode::Enter));
    insta::assert_snapshot!(screen(&mut app, 80, 24));
}

#[test]
fn frame_settings_ascii_editing_a_number() {
    let mut app = App::new(
        settings(false, true),
        attachment(Vec::new()),
        Instant::now(),
    );
    command(&mut app, "/settings");
    select_row(&mut app, "max_fps");
    app.update(key(KeyCode::Enter));
    let text = screen(&mut app, 80, 24);
    assert!(text.is_ascii(), "{text}");
    insta::assert_snapshot!(text);
}

#[test]
fn frame_settings_with_clai_theme() {
    let mut app = settings_open();
    app.settings.theming.theme_env = Some("nord".into());
    insta::assert_snapshot!(screen(&mut app, 80, 24));
}

#[test]
fn frame_theme_list() {
    let mut app = app();
    app.settings.theming.warnings = vec![warning("bad")];
    command(&mut app, "/theme");
    app.update(key(KeyCode::Down));
    insta::assert_snapshot!(screen(&mut app, 80, 30));
}

#[test]
fn frame_theme_list_ascii() {
    let mut app = App::new(
        settings(false, true),
        attachment(Vec::new()),
        Instant::now(),
    );
    command(&mut app, "/theme");
    let text = screen(&mut app, 80, 30);
    assert!(text.is_ascii(), "{text}");
    insta::assert_snapshot!(text);
}

/// One frame per built-in and bundled theme: the styles of the cells, not
/// only the text, so that a palette change shows.
#[test]
fn frame_of_each_theme() {
    for theme in [
        "dark",
        "light",
        "atom-one-dark",
        "nord",
        "gruvbox-dark",
        "solarized-light",
    ] {
        let mut app = app_with(vec![
            SessionEvent::UserMessage { text: "hi".into() },
            SessionEvent::TextDone {
                text: "**done** `x`".into(),
            },
            SessionEvent::ToolCall(done(call("c1", "read_file", CallState::Running), "1 line")),
            SessionEvent::Error {
                message: "oops".into(),
            },
        ]);
        app.apply(tui_with(|t| t.theme = theme.into()));
        let terminal = screen_of(&mut app, 40, 12);
        insta::assert_debug_snapshot!(format!("theme_{theme}"), terminal.backend().buffer());
    }
}

/// The shell's save: written, checked by loading, refused when invalid.
#[test]
fn the_shell_saves_into_a_real_user_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.toml");
    let text = "# mine\ndefault_model = \"a/m\"\n\n[providers.a]\napi = \"anthropic\"\n\n[providers.a.models.m]\nid = \"m\"\ncontext_window = 1000\n";
    std::fs::write(&path, text).unwrap();
    std::fs::create_dir(dir.path().join("themes")).unwrap();
    std::fs::write(dir.path().join("themes/mine.toml"), "base = \"light\"\n").unwrap();
    let options = crate::config::LoadOptions {
        config_file: Some(path.clone()),
        workspace: dir.path().to_path_buf(),
        env: crate::config::Environment::default(),
    };
    let stamp = crate::config::edit::stamp(&path);
    let theme = |name: &str| vec![("theme", Some(Setting::Text(name.into())))];
    let new = save(&options, &path, stamp, &theme("mine")).unwrap();
    let written = std::fs::read_to_string(&path).unwrap();
    assert!(written.starts_with("# mine\n"), "{written}");
    assert!(written.ends_with("[tui]\ntheme = \"mine\"\n"), "{written}");
    // A theme the loader does not know is refused, and nothing is written.
    let e = save(&options, &path, new, &theme("neon")).unwrap_err();
    assert!(e.contains("unknown theme `neon`"), "{e}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), written);
}

#[test]
fn opening_reads_the_theme_files_again() {
    let mut app = app();
    assert!(command(&mut app, "/theme").contains(&Effect::ReloadConfig));
    app.update(key(KeyCode::Down));
    let mut themes = app.settings.theming.themes.clone();
    themes.insert(
        "fresh".into(),
        crate::config::Theme {
            base: crate::config::BuiltinTheme::Light,
            colours: Vec::new(),
            code: None,
            source: crate::config::ThemeSource::File("themes/fresh.toml".into()),
        },
    );
    app.update(Input::Reloaded(Box::new(Ok(super::app::Reloaded {
        tui: app.settings.theming.file.tui.clone(),
        themes,
        warnings: Vec::new(),
        file: super::app::FileTui::default(),
    }))));
    let list = app.theme_list.as_ref().unwrap();
    assert!(list.visible().iter().any(|e| e.name == "fresh"));
    // The selection and its preview stay.
    assert_eq!(list.preview(), Some("dark"));
    assert_eq!(app.settings.tui.theme, "dark");
    assert!(command(&mut self::app(), "/settings").contains(&Effect::ReloadConfig));
}

/// Two calls in one answer of the model: the first denied with a message,
/// then the second asks at once. Its prompt must take keys.
#[test]
fn after_a_message_the_next_question_takes_keys() {
    let mut app = asked();
    app.update(ch('e'));
    typed(&mut app, "use b.rs");
    let effects = app.update(key(KeyCode::Enter));
    let request = requests(&effects).remove(0);
    let first = permission();
    app.update(live(SessionEvent::QuestionClosed {
        id: first.id.clone(),
        closed: Closed::Answered {
            answer: Answer::DenyMessage {
                text: "use b.rs".into(),
            },
            by: "tui pts/3".into(),
        },
    }));
    app.update(live(SessionEvent::ToolCall(call(
        "c2",
        "edit_file",
        CallState::Waiting,
    ))));
    let mut second = permission();
    second.id = "q2".into();
    if let QuestionKind::Permission(p) = &mut second.kind {
        p.call.id = "c2".into();
    }
    app.update(live(SessionEvent::Question(Box::new(second))));
    // The answer to the first question comes back only now.
    app.update(Input::Response {
        request,
        result: Ok(Payload::Done),
    });
    let effects = app.update(ch('y'));
    assert_eq!(
        requests(&effects),
        [Request::Answer {
            session: "project".into(),
            question: "q2".into(),
            answer: Answer::Allow,
        }]
    );
}

/// A session waiting on a question can be left for another one: the
/// question stays open on the server, and nothing is answered.
#[test]
fn a_waiting_session_can_be_left() {
    let mut app = asked();
    app.update(sessions_answer(three()));
    let effects = app.update(ctrl('n'));
    assert_eq!(switches(&effects), ["docs"]);
    assert!(
        !requests(&effects)
            .iter()
            .any(|r| matches!(r, Request::Answer { .. }))
    );
    app.update(attached_to(2, "docs", Vec::new()));
    assert!(app.prompt.is_none());
    assert_eq!(switches(&app.update(ctrl('p'))), ["project"]);
}

/// `Ctrl-S` opens the session list over the prompt; its keys go to the
/// list, not to the question.
#[test]
fn the_session_list_opens_over_a_prompt() {
    let mut app = asked();
    let effects = app.update(ctrl('s'));
    assert_eq!(requests(&effects), [Request::List]);
    app.update(sessions_answer(three()));
    assert!(app.session_list.is_some());
    assert!(app.prompt.is_some());
    // The arrow moves in the list and answers nothing.
    let effects = app.update(key(KeyCode::Down));
    assert!(requests(&effects).is_empty());
    let effects = app.update(key(KeyCode::Enter));
    assert_eq!(switches(&effects), ["docs"]);
    assert!(
        !requests(&effects)
            .iter()
            .any(|r| matches!(r, Request::Answer { .. }))
    );
    // `Esc` closes the list, and the prompt takes keys again.
    let mut app = asked();
    app.update(ctrl('s'));
    app.update(sessions_answer(three()));
    app.update(key(KeyCode::Esc));
    assert!(app.session_list.is_none());
    assert_eq!(
        requests(&app.update(ch('y'))),
        [Request::Answer {
            session: "project".into(),
            question: permission().id,
            answer: Answer::Allow,
        }]
    );
}

// The status box (RFC 0007).

const CREATED: u64 = 1_793_491_200;

/// A status after a turn with tool calls and questions, `state` now.
fn status_of(state: State) -> crate::protocol::SessionStatus {
    use crate::agent::{ToolCallEvent, TurnEnd};
    use crate::protocol::{
        ClientInfo, DirInfo, LimitsInfo, ModelStatus, SessionStatus, Stats, Status,
    };
    let t = CREATED * 1000;
    let tool = |name: &str, state: CallState| {
        SessionEvent::ToolCall(ToolCallEvent {
            id: "c".into(),
            name: name.into(),
            state,
            summary: None,
            arguments: serde_json::Value::Null,
            output: None,
            result: None,
        })
    };
    let log = [
        (
            t,
            SessionEvent::State {
                state: State::Working,
            },
        ),
        (t, SessionEvent::UserMessage { text: "go".into() }),
        (
            t + 2_000,
            SessionEvent::Usage {
                input_tokens: 2_100_000,
                output_tokens: 84_300,
                context_tokens: 170_000,
                model: String::new(),
            },
        ),
        (t + 3_000, tool("read_file", CallState::Done)),
        (t + 3_000, tool("read_file", CallState::Done)),
        (t + 4_000, tool("edit_file", CallState::Denied)),
        (
            t + 4_000,
            SessionEvent::QuestionClosed {
                id: "q1".into(),
                closed: Closed::Answered {
                    answer: Answer::Allow,
                    by: "tui pts/3".into(),
                },
            },
        ),
        (
            t + 5_000,
            SessionEvent::Retry {
                attempt: 1,
                max: 3,
                reason: "HTTP 503".into(),
            },
        ),
        (
            t + 60_000,
            SessionEvent::TurnEnd(TurnEnd {
                reason: EndReason::Done,
                requests: 1,
                tool_calls: 3,
                input_tokens: 0,
                output_tokens: 0,
            }),
        ),
        (t + 60_000, SessionEvent::State { state: State::Idle }),
        (
            t + 70_000,
            SessionEvent::ModelChanged {
                from: "anthropic/haiku".into(),
                model: "anthropic/sonnet".into(),
            },
        ),
    ];
    let mut stats = Stats::of_log(log.iter().map(|(at, e)| (Some(*at), e)));
    if state == State::Working {
        stats.count(&SessionEvent::State { state }, Some(t + 100_000));
        stats.count(
            &SessionEvent::UserMessage {
                text: "more".into(),
            },
            Some(t + 100_000),
        );
    }
    SessionStatus {
        session: SessionInfo {
            state,
            created: CREATED,
            updated: 0,
            turns: 0,
            queued: 1,
            ..info()
        },
        stats,
        clients: vec![
            ClientInfo {
                name: "chat pts/5".into(),
                answers: false,
            },
            ClientInfo {
                name: "tui pts/3".into(),
                answers: true,
            },
        ],
        dir: Some("~/.local/state/clai/sessions/1".into()),
        size: Some(1_234_567),
        extra_dirs: vec![DirInfo {
            path: "~/ws/shared-lib".into(),
            read_only: false,
            exists: false,
        }],
        model: Some(ModelStatus {
            info: ModelInfo {
                model: "anthropic/sonnet".into(),
                provider: "anthropic".into(),
                name: "sonnet".into(),
                id: "claude-sonnet-5-5".into(),
                context_window: 200_000,
                default: true,
                key: "env".into(),
            },
            api: "anthropic".into(),
            base_url: "https://api.anthropic.com".into(),
            temperature: None,
            max_output_tokens: None,
            max_retries: 3,
        }),
        limits: Some(LimitsInfo {
            max_requests: 50,
            max_tool_calls: 120,
        }),
        now: t + 7_800_000,
        server: Status {
            pid: 4242,
            sessions: 3,
            running: 2,
            busy: 1,
            clients: 3,
            version: "0.4.0".into(),
        },
    }
}

fn status_answer(status: crate::protocol::SessionStatus) -> Input {
    Input::Response {
        request: Request::SessionStatus {
            session: status.session.name.clone(),
        },
        result: Ok(Payload::SessionStatus {
            status: Box::new(status),
        }),
    }
}

fn status_requests(effects: &[Effect]) -> Vec<Request> {
    requests(effects)
        .into_iter()
        .filter(|r| matches!(r, Request::SessionStatus { .. }))
        .collect()
}

#[test]
fn status_opens_the_box_and_asks() {
    let mut app = app();
    let effects = command(&mut app, "/status");
    assert_eq!(
        status_requests(&effects),
        [Request::SessionStatus {
            session: "project".into()
        }]
    );
    assert!(app.editor.is_empty());
    assert!(screen(&mut app, 80, 24).contains("asking the server"));
    app.update(status_answer(status_of(State::Idle)));
    let text = screen(&mut app, 80, 50);
    assert!(text.contains("Tool calls"), "{text}");
    // Keys go to the box: `q` closes it and types nothing.
    app.update(ch('q'));
    assert!(app.status_box.is_none());
    assert!(app.editor.is_empty());
}

#[test]
fn the_status_box() {
    let mut app = app();
    command(&mut app, "/status");
    app.update(status_answer(status_of(State::Working)));
    insta::assert_snapshot!(screen(&mut app, 80, 56));
}

#[test]
fn the_status_box_in_ascii_and_scrolled() {
    let mut app = App::new(
        settings(false, false),
        attachment(Vec::new()),
        Instant::now(),
    );
    command(&mut app, "/status");
    app.update(status_answer(status_of(State::Idle)));
    let first = screen(&mut app, 80, 24);
    assert!(first.contains("Session"), "{first}");
    assert!(!first.contains("Server"), "{first}");
    app.update(key(KeyCode::End));
    let last = screen(&mut app, 80, 24);
    assert!(last.contains("Server"), "{last}");
    assert!(last.contains("0.4.0 - pid 4242"), "{last}");
    assert!(last.contains("Esc close"), "{last}");
    app.update(key(KeyCode::Home));
    assert_eq!(app.status_box.as_ref().unwrap().top, 0);
}

#[test]
fn the_status_box_refreshes_while_the_session_works() {
    let mut app = app();
    command(&mut app, "/status");
    // Not twice at a time.
    assert!(status_requests(&app.update(Input::Tick)).is_empty());
    app.update(status_answer(status_of(State::Working)));
    assert_eq!(status_requests(&app.update(Input::Tick)).len(), 1);
    assert!(status_requests(&app.update(Input::Tick)).is_empty());
    app.update(status_answer(status_of(State::Idle)));
    // Idle: no polling, until an event changes it.
    assert!(status_requests(&app.update(Input::Tick)).is_empty());
    app.update(live(SessionEvent::Usage {
        input_tokens: 1,
        output_tokens: 1,
        context_tokens: 2,
        model: String::new(),
    }));
    assert_eq!(status_requests(&app.update(Input::Tick)).len(), 1);
    // A failed refresh keeps what is shown.
    app.update(Input::Response {
        request: Request::SessionStatus {
            session: "project".into(),
        },
        result: Err("busy".into()),
    });
    assert!(app.status_box.as_ref().unwrap().status.is_some());
}

#[test]
fn status_of_another_session() {
    let mut app = among_three();
    let effects = command(&mut app, "/status docs");
    assert_eq!(
        status_requests(&effects),
        [Request::SessionStatus {
            session: "docs".into()
        }]
    );
    // An unknown name stays in the input.
    app.update(key(KeyCode::Esc));
    let effects = command(&mut app, "/status nope");
    assert!(status_requests(&effects).is_empty());
    assert_eq!(app.editor.text(), "/status nope");
    // A first answer that fails closes the box with the error.
    app.editor.clear();
    command(&mut app, "/status web");
    app.update(Input::Response {
        request: Request::SessionStatus {
            session: "web".into(),
        },
        result: Err("no session named web".into()),
    });
    assert!(app.status_box.is_none());
    assert!(kinds(&app).contains(&Kind::Error("no session named web".into())));
}

#[test]
fn the_status_box_closes_on_a_question_and_a_switch() {
    let mut app = among_three();
    command(&mut app, "/status");
    app.update(status_answer(status_of(State::Idle)));
    app.update(live(SessionEvent::ToolCall(call(
        "c1",
        "edit_file",
        CallState::Waiting,
    ))));
    app.update(live(SessionEvent::Question(Box::new(permission()))));
    assert!(app.status_box.is_none());

    let mut app = among_three();
    command(&mut app, "/status");
    app.update(ctrl('n'));
    app.update(attached_to(2, "docs", Vec::new()));
    assert!(app.status_box.is_none());
}

#[test]
fn status_completes_session_names_with_the_attached_one() {
    let mut app = among_three();
    typed(&mut app, "/status ");
    let values: Vec<String> = app.arguments().into_iter().map(|v| v.value).collect();
    assert_eq!(values, ["project", "docs", "web"]);
}

#[test]
fn a_retry_is_a_notice() {
    let mut app = app();
    app.update(live(SessionEvent::Retry {
        attempt: 2,
        max: 3,
        reason: "rate limited".into(),
    }));
    assert!(kinds(&app).contains(&Kind::Notice("retrying (2/3): rate limited".into())));
}

// Questions from the model.

fn ask_question(two: bool) -> Question {
    let mut questions = vec![json!({
        "question": "Which database should the cache use?", "header": "Database",
        "options": [
            {"label": "SQLite (recommended)", "description": "One file next to the binary, no server."},
            {"label": "Postgres", "description": "Shared with the API; needs the server running."}
        ]
    })];
    if two {
        questions.push(json!({
            "question": "Which features should the first version have?", "header": "Features",
            "options": [{"label": "Search"}, {"label": "Export"}, {"label": "Import"}],
            "multiple": true
        }));
    }
    let arguments = json!({
        "questions": questions,
        "context": "Both work with the current code; the choice decides the **migration**."
    });
    let asked = crate::tools::ask_user::check(&arguments).unwrap();
    Question {
        id: "q3".to_owned(),
        session: SessionRef::default(),
        opened: 0,
        turn: TurnInfo {
            number: 1,
            requests: 1,
            tool_calls: 1,
        },
        summary: crate::tools::ask_user::summary(&asked.questions),
        choices: crate::agent::ASK_USER_CHOICES.to_vec(),
        kind: QuestionKind::AskUser(Box::new(crate::agent::AskUser {
            call: crate::agent::AskCall {
                tool: "ask_user".into(),
                id: "c9".into(),
                arguments,
            },
            questions: asked.questions,
            context: asked.context,
            text: None,
        })),
    }
}

fn ask(mut app: App, two: bool) -> App {
    let mut waiting = call("c9", "ask_user", CallState::Waiting);
    waiting.summary = Some(
        if two {
            "Database, Features"
        } else {
            "Database"
        }
        .into(),
    );
    waiting.arguments = json!({});
    app.update(live(SessionEvent::ToolCall(waiting)));
    app.update(live(SessionEvent::Question(Box::new(ask_question(two)))));
    app
}

fn user_answer(selected: &[usize], text: Option<&str>) -> crate::tools::UserAnswer {
    crate::tools::UserAnswer {
        selected: selected.to_vec(),
        text: text.map(str::to_owned),
    }
}

#[test]
fn one_question_enter_sends_the_selected_option() {
    let mut app = ask(super::tests::app(), false);
    assert!(app.prompt.as_ref().is_some_and(|p| p.is_ask()));
    assert!(requests(&app.update(key(KeyCode::Down))).is_empty());
    assert_eq!(
        answer_of(&app.update(key(KeyCode::Enter))),
        Answer::Reply {
            answers: vec![user_answer(&[1], None)]
        }
    );
    // A digit selects; then Enter sends it.
    let mut app = ask(super::tests::app(), false);
    assert!(requests(&app.update(ch('1'))).is_empty());
    assert_eq!(
        answer_of(&app.update(key(KeyCode::Enter))),
        Answer::Reply {
            answers: vec![user_answer(&[0], None)]
        }
    );
}

#[test]
fn a_typed_answer() {
    let mut app = ask(super::tests::app(), false);
    app.update(ch('3'));
    typed(&mut app, "Redis");
    assert_eq!(
        answer_of(&app.update(key(KeyCode::Enter))),
        Answer::Reply {
            answers: vec![user_answer(&[], Some("Redis"))]
        }
    );
    // Ctrl-D while typing does not detach.
    let mut app = ask(super::tests::app(), false);
    app.update(ch('3'));
    typed(&mut app, "ab");
    app.update(key(KeyCode::Left));
    assert!(exit(&app.update(ctrl('d'))).is_none());
    assert_eq!(
        answer_of(&app.update(key(KeyCode::Enter))),
        Answer::Reply {
            answers: vec![user_answer(&[], Some("ab"))]
        }
    );
}

#[test]
fn several_questions_tick_tab_and_review() {
    let mut app = ask(super::tests::app(), true);
    // First question: Postgres, then on to the second.
    app.update(key(KeyCode::Down));
    assert!(requests(&app.update(key(KeyCode::Enter))).is_empty());
    // Tick Search and Import, add a text.
    app.update(ch(' '));
    app.update(ch('3'));
    app.update(ch('4'));
    typed(&mut app, "keep the old format");
    app.update(key(KeyCode::Enter));
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("Question ─ review"), "{text}");
    assert!(text.contains("Postgres"), "{text}");
    assert!(
        text.contains("Search, Import; \"keep the old format\""),
        "{text}"
    );
    // Back to the first question with Shift-Tab, which keeps the choice.
    app.update(key(KeyCode::BackTab));
    app.update(key(KeyCode::BackTab));
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("1/2 Database"), "{text}");
    app.update(key(KeyCode::Tab));
    app.update(key(KeyCode::Tab));
    assert_eq!(
        answer_of(&app.update(key(KeyCode::Enter))),
        Answer::Reply {
            answers: vec![
                user_answer(&[1], None),
                user_answer(&[0, 2], Some("keep the old format"))
            ]
        }
    );
}

#[test]
fn the_review_sends_only_complete_answers() {
    let mut app = ask(super::tests::app(), true);
    app.update(key(KeyCode::Tab));
    app.update(key(KeyCode::Tab));
    // Nothing chosen: Enter goes to the first question without an answer.
    assert!(requests(&app.update(key(KeyCode::Enter))).is_empty());
    assert_eq!(app.prompt.as_ref().unwrap().ask.as_ref().unwrap().page, 0);
}

#[test]
fn esc_declines_and_asks_first_when_something_is_chosen() {
    let mut app = ask(super::tests::app(), true);
    assert_eq!(answer_of(&app.update(key(KeyCode::Esc))), Answer::Decline);

    let mut app = ask(super::tests::app(), true);
    app.update(key(KeyCode::Enter));
    assert!(requests(&app.update(key(KeyCode::Esc))).is_empty());
    let text = screen(&mut app, 80, 24);
    assert!(text.contains("Skip the questions"), "{text}");
    assert!(requests(&app.update(ch('n'))).is_empty());
    app.update(key(KeyCode::Esc));
    assert_eq!(answer_of(&app.update(ch('y'))), Answer::Decline);

    let mut app = ask(super::tests::app(), true);
    app.update(key(KeyCode::Enter));
    assert_eq!(answer_of(&app.update(ctrl('c'))), Answer::Cancel);
}

#[test]
fn the_question_box_closes_when_another_client_answers() {
    let mut app = ask(super::tests::app(), false);
    app.update(live(SessionEvent::QuestionClosed {
        id: "q3".into(),
        closed: Closed::Answered {
            answer: Answer::Decline,
            by: "clai chat".into(),
        },
    }));
    assert!(app.prompt.is_none());
    assert_eq!(notices(&app), ["answered by clai chat: decline"]);
    assert!(!screen(&mut app, 80, 24).contains("Question ─"));
}

#[test]
fn a_rejected_reply_shows_in_the_box() {
    let mut app = ask(super::tests::app(), false);
    let effects = app.update(key(KeyCode::Enter));
    let request = requests(&effects).remove(0);
    app.update(Input::Response {
        request,
        result: Err("answer 1: option 0 does not exist".into()),
    });
    assert!(screen(&mut app, 80, 24).contains("not accepted: answer 1: option 0 does not exist"));
    assert!(!requests(&app.update(key(KeyCode::Enter))).is_empty());
}

#[test]
fn frame_question_from_the_model() {
    let mut app = ask(idle_session(), false);
    insta::assert_snapshot!(screen(&mut app, 80, 24));
}

#[test]
fn frame_question_from_the_model_multiple() {
    let mut app = ask(idle_session(), true);
    app.update(key(KeyCode::Enter));
    app.update(ch(' '));
    app.update(ch('3'));
    insta::assert_snapshot!(screen(&mut app, 80, 24));
}

#[test]
fn frame_question_from_the_model_narrow_and_ascii() {
    let mut app = ask(idle_session(), true);
    insta::assert_snapshot!("question_from_the_model_narrow", screen(&mut app, 40, 12));
    let app = App::new(settings(false, true), attachment(vec![]), Instant::now());
    let mut app = ask(app, true);
    let text = screen(&mut app, 80, 24);
    assert!(text.is_ascii(), "{text}");
    insta::assert_snapshot!("question_from_the_model_ascii", text);
}

// The activity line.

fn state_event(state: State) -> Input {
    live(SessionEvent::State { state })
}

/// A turn that started 14 s ago, with 1.2k tokens of text so far.
fn thinking() -> App {
    let mut app = app();
    app.update(state_event(State::Working));
    app.update(live(SessionEvent::UserMessage {
        text: "fix the failing test".into(),
    }));
    app.update(live(SessionEvent::TextDelta {
        text: "The test fails because `parse` drops the last line. Let me look.".into(),
    }));
    app.update(live(SessionEvent::TextDone {
        text: "The test fails because `parse` drops the last line. Let me look.".into(),
    }));
    app.update(live(SessionEvent::Usage {
        input_tokens: 4000,
        output_tokens: 1200,
        context_tokens: 5200,
        model: String::new(),
    }));
    app.update(live(SessionEvent::ToolCall(call(
        "c1",
        "read_file",
        CallState::Running,
    ))));
    app.update(live(SessionEvent::ToolCall(done(
        call("c1", "read_file", CallState::Running),
        "a\nb",
    ))));
    app.now += Duration::from_secs(14);
    app
}

fn activity_row(app: &mut App, width: u16, height: u16) -> Option<String> {
    screen(app, width, height)
        .lines()
        .find(|l| {
            l.contains('(') && (l.starts_with(['·', '✢', '✳', '✶', '✻', '✽', '|', '/', '-', '\\']))
        })
        .map(|l| l.trim_end().to_owned())
}

#[test]
fn frame_activity_thinking() {
    let mut app = thinking();
    insta::assert_snapshot!(screen(&mut app, 80, 24));
}

#[test]
fn the_activity_line_comes_with_the_turn_and_goes_with_it() {
    let mut app = app();
    assert_eq!(activity_row(&mut app, 80, 24), None);
    app.update(state_event(State::Working));
    let row = activity_row(&mut app, 80, 24).unwrap();
    // No text yet: no tokens; the time from the start.
    assert!(row.ends_with("(0s · Esc cancel)"), "{row}");
    let mut app = thinking();
    let row = activity_row(&mut app, 80, 24).unwrap();
    assert!(row.ends_with("(14s · ↓ 1.2k tokens · Esc cancel)"), "{row}");
    app.update(state_event(State::Idle));
    assert_eq!(activity_row(&mut app, 80, 24), None);
    // The conversation has its rows back.
    assert!(!screen(&mut app, 80, 24).contains("tokens"));
}

#[test]
fn the_activity_line_names_running_tools() {
    let mut app = thinking();
    let mut command = call("c2", "run_command", CallState::Running);
    command.summary = Some("cargo test".into());
    app.update(live(SessionEvent::ToolCall(command)));
    let row = activity_row(&mut app, 80, 24).unwrap();
    assert!(row.contains(" Running cargo test… ("), "{row}");
    for id in ["c3", "c4"] {
        app.update(live(SessionEvent::ToolCall(call(
            id,
            "edit_file",
            CallState::Pending,
        ))));
    }
    insta::assert_snapshot!(screen(&mut app, 80, 24));
}

#[test]
fn the_activity_line_after_a_retry_and_a_cancel() {
    let mut app = thinking();
    app.update(live(SessionEvent::Retry {
        attempt: 2,
        max: 3,
        reason: "rate limited".into(),
    }));
    let row = activity_row(&mut app, 80, 24).unwrap();
    assert!(row.contains(" Retrying (2/3): rate limited… (14s"), "{row}");
    app.update(live(SessionEvent::TextDelta { text: "ok".into() }));
    let row = activity_row(&mut app, 80, 24).unwrap();
    assert!(!row.contains("Retrying"), "{row}");
    let effects = app.update(key(KeyCode::Esc));
    assert!(matches!(
        requests(&effects).as_slice(),
        [Request::Cancel { .. }]
    ));
    let row = activity_row(&mut app, 80, 24).unwrap();
    // The hint is gone: Esc did its work.
    assert!(row.ends_with("Cancelling… (14s · ↓ 1.2k tokens)"), "{row}");
}

#[test]
fn the_esc_hint_is_shown_only_when_esc_cancels() {
    let mut app = thinking();
    assert!(
        activity_row(&mut app, 80, 24)
            .unwrap()
            .contains("Esc cancel")
    );
    typed(&mut app, "next");
    assert!(
        !activity_row(&mut app, 80, 24)
            .unwrap()
            .contains("Esc cancel")
    );
    app.update(ctrl('u'));
    app.update(key(KeyCode::Tab));
    assert_eq!(app.focus, Focus::Conversation);
    assert!(
        !activity_row(&mut app, 80, 24)
            .unwrap()
            .contains("Esc cancel")
    );
}

#[test]
fn the_activity_line_hides_while_a_prompt_is_open() {
    let mut app = thinking();
    app.update(state_event(State::Waiting));
    app.update(live(SessionEvent::ToolCall(call(
        "c1",
        "edit_file",
        CallState::Waiting,
    ))));
    app.update(live(SessionEvent::Question(Box::new(permission()))));
    assert!(app.prompt.is_some());
    assert_eq!(activity_row(&mut app, 80, 24), None);
    app.update(live(SessionEvent::QuestionClosed {
        id: "q1".into(),
        closed: Closed::Answered {
            answer: Answer::Allow,
            by: "tui pts/3".into(),
        },
    }));
    app.update(state_event(State::Working));
    let row = activity_row(&mut app, 80, 24).unwrap();
    // The same turn: its time goes on.
    assert!(row.contains("(14s"), "{row}");
}

#[test]
fn the_activity_line_drops_parts_on_a_narrow_or_low_screen() {
    let mut app = thinking();
    let verb = app.activity.verb;
    let row = activity_row(&mut app, 40, 24).unwrap();
    assert!(
        row.ends_with(&format!("{verb}… (14s · ↓ 1.2k tokens)")),
        "{row}"
    );
    assert!(!row.contains("Esc"), "{row}");
    app.update(live(SessionEvent::Retry {
        attempt: 2,
        max: 3,
        reason: "the provider is overloaded, try again later".into(),
    }));
    // The time goes last; then the text is cut.
    let row = activity_row(&mut app, 40, 24).unwrap();
    assert_eq!(row, "· Retrying (2/3): the provider is overl…");
    // Too low: no line.
    assert_eq!(activity_row(&mut app, 80, 13), None);
    assert!(!screen(&mut app, 80, 13).contains("Retrying"));
}

#[test]
fn frame_activity_ascii_and_plain() {
    let mut app = App::new(
        settings(false, false),
        attachment(Vec::new()),
        Instant::now(),
    );
    app.update(state_event(State::Working));
    app.update(live(SessionEvent::TextDelta {
        text: "x".repeat(4800),
    }));
    let mut tui = app.settings.tui.clone();
    tui.activity = crate::config::ActivityLine::Plain;
    app.apply(tui);
    app.now += Duration::from_secs(125);
    let row = activity_row(&mut app, 80, 24).unwrap();
    assert_eq!(row, "| Writing... (2m 05s - v 1.2k tokens - Esc cancel)");
    app.update(live(SessionEvent::ToolCall(call(
        "c1",
        "read_file",
        CallState::Running,
    ))));
    app.update(Input::Frame);
    let row = activity_row(&mut app, 80, 24).unwrap();
    assert_eq!(
        row,
        "/ Reading src/parse.rs... (2m 05s - v 1.2k tokens - Esc cancel)"
    );
}

#[test]
fn activity_off_shows_no_line_but_spinners_still_move() {
    let mut app = thinking();
    let mut tui = app.settings.tui.clone();
    tui.activity = crate::config::ActivityLine::Off;
    app.apply(tui);
    assert_eq!(activity_row(&mut app, 80, 24), None);
    // Nothing moves: no frames.
    assert!(!app.animating());
    assert!(app.update(Input::Frame).is_empty());
    app.update(live(SessionEvent::ToolCall(call(
        "c2",
        "read_file",
        CallState::Running,
    ))));
    assert!(app.animating());
    let before = screen(&mut app, 80, 24);
    assert_eq!(app.update(Input::Frame), [Effect::Redraw]);
    assert_ne!(screen(&mut app, 80, 24), before, "the spinner moved");
    // The second tick does not move it again.
    let before = screen(&mut app, 80, 24);
    app.update(Input::Tick);
    assert_eq!(screen(&mut app, 80, 24), before);
}

#[test]
fn frames_run_only_while_the_session_works() {
    let mut app = app();
    assert!(!app.animating());
    assert!(app.update(Input::Frame).is_empty());
    app.update(state_event(State::Working));
    assert!(app.animating());
    let glyph = |app: &mut App| activity_row(app, 80, 24).unwrap().chars().next();
    let first = glyph(&mut app);
    assert_eq!(app.update(Input::Frame), [Effect::Redraw]);
    assert_ne!(glyph(&mut app), first);
    app.update(state_event(State::Idle));
    assert!(!app.animating());
    assert!(app.update(Input::Frame).is_empty());
}

#[test]
fn a_quiet_model_turns_the_glyph_to_the_warning_colour() {
    let mut app = thinking();
    let glyph_colour = |app: &mut App| {
        let terminal = screen_of(app, 80, 24);
        let buffer = terminal.backend().buffer().clone();
        (0..24)
            .map(|y| buffer[(0, y)].clone())
            .find(|c| c.symbol() == "·")
            .map(|c| c.fg)
    };
    let accent = app.settings.look.styles.accent.fg;
    let warning = app.settings.look.styles.warning.fg;
    assert_ne!(accent, warning);
    assert_eq!(glyph_colour(&mut app), accent);
    app.now += Duration::from_secs(30);
    assert_eq!(glyph_colour(&mut app), warning);
    // The time stays.
    assert!(activity_row(&mut app, 80, 24).unwrap().contains("(44s"));
}

#[test]
fn an_attach_to_a_working_session_takes_the_start_from_the_server() {
    let working = SessionInfo {
        state: State::Working,
        ..info()
    };
    let mut app = App::new(
        settings(true, true),
        Attachment {
            session: working.clone(),
            sessions: vec![working],
            backlog: vec![
                replayed(SessionEvent::State {
                    state: State::Working,
                }),
                replayed(SessionEvent::UserMessage { text: "go".into() }),
            ],
        },
        Instant::now() + Duration::from_secs(3 * 3600),
    );
    // The time is left out until the answer.
    let row = activity_row(&mut app, 80, 24).unwrap();
    assert!(row.ends_with("(Esc cancel)"), "{row}");
    let effects = app.update(Input::Tick);
    assert_eq!(
        status_requests(&effects),
        [Request::SessionStatus {
            session: "project".into()
        }]
    );
    // Once only.
    assert!(status_requests(&app.update(Input::Frame)).is_empty());
    app.update(status_answer(status_of(State::Working)));
    // No box opened for it.
    assert!(app.status_box.is_none());
    let row = activity_row(&mut app, 80, 24).unwrap();
    // The turn started at t + 100 s; the server's clock says t + 7800 s.
    assert!(row.contains("(2h 08m · Esc cancel)"), "{row}");
}

fn global_answer(range: crate::protocol::Range, stats: crate::protocol::GlobalStats) -> Input {
    Input::Response {
        request: Request::GlobalStats {
            range,
            offset: 3_600,
        },
        result: Ok(Payload::GlobalStats {
            stats: Box::new(stats),
        }),
    }
}

fn global_requests(effects: &[Effect]) -> Vec<Request> {
    requests(effects)
        .into_iter()
        .filter(|r| matches!(r, Request::GlobalStats { .. }))
        .collect()
}

fn week_stats() -> crate::protocol::GlobalStats {
    crate::text::global::tests::week()
}

#[test]
fn stats_opens_the_global_tab_and_asks() {
    use crate::protocol::Range;
    let mut app = app();
    let effects = command(&mut app, "/stats");
    assert_eq!(
        requests(&effects),
        [Request::GlobalStats {
            range: Range::Week,
            offset: 3_600
        }]
    );
    assert!(app.editor.is_empty());
    assert!(screen(&mut app, 80, 24).contains("counting the logs"));
    // Not twice at a time, and no polling.
    assert!(global_requests(&app.update(Input::Tick)).is_empty());
    app.update(global_answer(Range::Week, week_stats()));
    assert!(global_requests(&app.update(Input::Tick)).is_empty());
    let text = screen(&mut app, 80, 50);
    assert!(text.contains("12 active of 31"), "{text}");
    // The Session tab asks for the status once it is shown.
    let effects = app.update(key(KeyCode::Tab));
    assert_eq!(
        status_requests(&effects),
        [Request::SessionStatus {
            session: "project".into()
        }]
    );
    app.update(status_answer(status_of(State::Idle)));
    assert!(screen(&mut app, 80, 50).contains("Tool calls"));
    // Back again: the answer is kept.
    assert!(requests(&app.update(key(KeyCode::Left))).is_empty());
    assert!(screen(&mut app, 80, 50).contains("12 active of 31"));
}

#[test]
fn stats_with_a_range() {
    use crate::protocol::Range;
    let mut app = app();
    let effects = command(&mut app, "/stats 30d");
    assert_eq!(
        global_requests(&effects),
        [Request::GlobalStats {
            range: Range::Month,
            offset: 3_600
        }]
    );
    // A wrong range stays in the input.
    let mut app = self::app();
    let effects = command(&mut app, "/stats 2w");
    assert!(requests(&effects).is_empty());
    assert_eq!(app.editor.text(), "/stats 2w");
    assert!(notices(&app).contains(&"usage: /stats [7d|30d|all]".to_owned()));
    // The argument is completed.
    let mut app = self::app();
    typed(&mut app, "/stats ");
    let values: Vec<String> = app.arguments().into_iter().map(|v| v.value).collect();
    assert_eq!(values, ["7d", "30d", "all"]);
}

#[test]
fn ranges_and_counting_again() {
    use crate::protocol::Range;
    let mut app = app();
    command(&mut app, "/stats");
    app.update(global_answer(Range::Week, week_stats()));
    app.update(key(KeyCode::Down));
    // `a`: all time, asked once, from the first line.
    let effects = app.update(ch('a'));
    assert_eq!(
        global_requests(&effects),
        [Request::GlobalStats {
            range: Range::All,
            offset: 3_600
        }]
    );
    let open = app.status_box.as_ref().unwrap();
    assert_eq!((open.range, open.top), (Range::All, 0));
    // `7` again: the kept answer, no request.
    assert!(global_requests(&app.update(ch('7'))).is_empty());
    // `r` goes round.
    assert_eq!(global_requests(&app.update(ch('r'))).len(), 1);
    assert_eq!(app.status_box.as_ref().unwrap().range, Range::Month);
    app.update(ch('7'));
    // `R` counts again, once at a time.
    assert_eq!(global_requests(&app.update(ch('R'))).len(), 1);
    assert!(global_requests(&app.update(ch('R'))).is_empty());
    // A refresh that fails keeps what is shown.
    app.update(Input::Response {
        request: Request::GlobalStats {
            range: Range::Week,
            offset: 0,
        },
        result: Err("busy".into()),
    });
    assert!(screen(&mut app, 80, 50).contains("12 active of 31"));
    // Range keys do nothing on the Session tab.
    app.update(key(KeyCode::Tab));
    assert!(global_requests(&app.update(ch('a'))).is_empty());
    assert_eq!(app.status_box.as_ref().unwrap().range, Range::Week);
}

#[test]
fn a_first_global_answer_that_fails_closes_the_box() {
    use crate::protocol::Range;
    let mut app = app();
    command(&mut app, "/stats");
    app.update(Input::Response {
        request: Request::GlobalStats {
            range: Range::Week,
            offset: 0,
        },
        result: Err("cannot count".into()),
    });
    assert!(app.status_box.is_none());
    assert!(kinds(&app).contains(&Kind::Error("cannot count".into())));
}

#[test]
fn an_old_answer_says_its_age() {
    use crate::protocol::Range;
    let mut app = app();
    command(&mut app, "/stats");
    app.update(global_answer(Range::Week, week_stats()));
    assert!(!screen(&mut app, 80, 50).contains(" old "));
    app.now += Duration::from_secs(150);
    let text = screen(&mut app, 80, 50);
    assert!(text.contains("Global ─ 2m 30s old"), "{text}");
}

#[test]
fn frame_global_tab() {
    use crate::protocol::Range;
    let mut app = app();
    command(&mut app, "/stats");
    app.update(global_answer(Range::Week, week_stats()));
    insta::assert_snapshot!(screen(&mut app, 80, 40));
}

#[test]
fn frame_global_tab_ascii() {
    use crate::protocol::Range;
    let mut app = App::new(
        settings(false, false),
        attachment(Vec::new()),
        Instant::now(),
    );
    command(&mut app, "/stats");
    app.update(global_answer(Range::Week, week_stats()));
    insta::assert_snapshot!(screen(&mut app, 80, 40));
}

#[test]
fn frame_global_tab_narrow() {
    use crate::protocol::Range;
    let mut app = app();
    command(&mut app, "/stats");
    app.update(global_answer(Range::Week, week_stats()));
    insta::assert_snapshot!(screen(&mut app, 42, 40));
}
