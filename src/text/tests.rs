//! Tool lines, question text and line answers.

use std::io::{Cursor, Write};
use std::sync::{Arc, Mutex};

use serde_json::json;

use crate::agent::{
    Answer, Asker, CallInfo, CallResult, CallState, EndReason, LIMIT_CHOICES, Limit,
    PERMISSION_CHOICES, Permission, Question, QuestionKind, Reply, SessionEvent, ToolCallEvent,
    TurnEnd, TurnInfo,
};
use crate::config::{Action, Symbols, Tool};
use crate::permissions::{Decision, PartDecision, Reason};
use crate::tools::{Details, PathDetails};

use super::*;

fn call(
    id: &str,
    name: &str,
    state: CallState,
    summary: Option<&str>,
    result: &str,
) -> SessionEvent {
    SessionEvent::ToolCall(ToolCallEvent {
        id: id.into(),
        name: name.into(),
        state,
        summary: summary.map(str::to_owned),
        arguments: json!({"path": "src/x.rs", "command": "rm -rf /"}),
        output: None,
        result: (!result.is_empty()).then(|| CallResult {
            text: result.into(),
            is_error: state != CallState::Done,
        }),
    })
}

fn end(reason: EndReason) -> SessionEvent {
    SessionEvent::TurnEnd(TurnEnd {
        reason,
        requests: 1,
        tool_calls: 0,
        input_tokens: 0,
        output_tokens: 0,
    })
}

fn render(events: &[SessionEvent], unicode: bool) -> (String, Option<String>) {
    let mut r = Renderer::new(unicode);
    let text: String = events.iter().map(|e| r.event(e)).collect();
    (text, r.answer().map(str::to_owned))
}

#[test]
fn tool_lines_and_final_answer() {
    let events = [
        SessionEvent::TextDone {
            text: "Let me look.".into(),
        },
        call("c1", "read_file", CallState::Pending, None, ""),
        call(
            "c1",
            "read_file",
            CallState::Running,
            Some("src/main.rs (lines 1–3)"),
            "",
        ),
        call(
            "c1",
            "read_file",
            CallState::Done,
            None,
            "     1→a\n     2→b\n     3→c",
        ),
        call(
            "c2",
            "run_command",
            CallState::Running,
            Some("cargo test"),
            "",
        ),
        call(
            "c2",
            "run_command",
            CallState::Error,
            None,
            "test x ... FAILED\n\nexit code 101",
        ),
        call(
            "c3",
            "run_command",
            CallState::Denied,
            None,
            "denied by permission rule: deny run_command(rm)",
        ),
        SessionEvent::Notice {
            text: "rules added: allow run_command(cargo test)".into(),
        },
        SessionEvent::TextDone {
            text: "The test fails.".into(),
        },
        SessionEvent::TextDone {
            text: "Fix it?".into(),
        },
        end(EndReason::Done),
    ];
    let (text, answer) = render(&events, true);
    assert_eq!(
        text,
        "Let me look.\n\
         ✓ read_file src/main.rs (lines 1–3) — 3 lines\n\
         ✗ run_command cargo test — exit code 101\n\
         ⊘ run_command rm -rf / — denied by permission rule: deny run_command(rm)\n\
         clai: rules added: allow run_command(cargo test)\n"
    );
    assert_eq!(answer.as_deref(), Some("The test fails.\nFix it?"));

    let (text, _) = render(&events, false);
    assert!(
        text.contains("+ read_file src/main.rs (lines 1–3) - 3 lines\n"),
        "{text}"
    );
    assert!(text.contains("x run_command cargo test - exit code 101\n"));
    assert!(text.contains("- run_command rm -rf / - denied"));
}

#[test]
fn text_of_a_turn_that_did_not_finish_goes_to_stderr() {
    let events = [
        SessionEvent::TextDone {
            text: "Half".into(),
        },
        SessionEvent::Error {
            message: "connection reset".into(),
        },
        end(EndReason::Failed),
    ];
    let (text, answer) = render(&events, true);
    assert_eq!(text, "clai: error: connection reset\nHalf\n");
    assert_eq!(answer, None);
}

#[test]
fn long_results_are_cut() {
    let long = "y".repeat(500);
    let (text, _) = render(
        &[call(
            "c1",
            "write_file",
            CallState::Error,
            Some("a.txt"),
            &long,
        )],
        true,
    );
    let line = text.lines().next().unwrap();
    assert!(line.chars().count() < 160, "{line}");
    assert!(line.ends_with('…'));
}

#[test]
fn symbols_from_the_locale() {
    let env = |pairs: &'static [(&'static str, &'static str)]| {
        move |name: &str| {
            pairs
                .iter()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| (*v).to_owned())
        }
    };
    assert!(unicode(Symbols::Auto, env(&[("LANG", "en_US.UTF-8")])));
    assert!(!unicode(Symbols::Auto, env(&[("LANG", "C")])));
    assert!(!unicode(
        Symbols::Auto,
        env(&[("LC_ALL", "C"), ("LANG", "en_US.UTF-8")])
    ));
    assert!(!unicode(Symbols::Auto, env(&[])));
    assert!(unicode(Symbols::Unicode, env(&[])));
    assert!(!unicode(Symbols::Ascii, env(&[("LANG", "en_US.UTF-8")])));
}

fn path(p: &str) -> PathDetails {
    PathDetails {
        written: p.into(),
        resolved: p.into(),
        area: "workspace".into(),
    }
}

fn permission(details: Details, proposals: Vec<Vec<String>>) -> Question {
    Question {
        id: "q1".into(),
        session: Default::default(),
        opened: 0,
        turn: TurnInfo {
            number: 1,
            requests: 1,
            tool_calls: 1,
        },
        summary: "write_file: a.txt (2 lines)".into(),
        choices: PERMISSION_CHOICES.to_vec(),
        kind: QuestionKind::Permission(Box::new(Permission {
            call: CallInfo {
                tool: Tool::WriteFile,
                id: "c1".into(),
                arguments: json!({}),
            },
            details,
            reason: Reason::Default {
                tool: Tool::WriteFile,
            },
            reason_text: "default for write_file".into(),
            proposals,
            text: Some("I will write it.\nThen test.".into()),
        })),
    }
}

fn new_file(lines: usize) -> Details {
    let content: Vec<String> = (1..=lines).map(|i| format!("line {i}")).collect();
    Details::WriteFile {
        path: path("a.txt"),
        exists: false,
        content: Some(crate::tools::Text::new(content.join("\n"))),
        diff: None,
        lines,
        created_dirs: vec![],
    }
}

#[test]
fn write_question_text() {
    let text = render_question(&permission(new_file(2), vec![]), "~/ws", true);
    assert_eq!(
        text,
        "── clai · ~/ws · q1 ──\n\
         write_file: a.txt (2 lines)\n  \
         new file a.txt (2 lines)\n  \
         asks     default for write_file\n    \
         line 1\n    \
         line 2\n  \
         model    \"I will write it.\"\n\
         [y] allow  [a] always  [n] deny  [e] deny with message  [c] cancel turn\n"
    );
    let long = render_question(&permission(new_file(45), vec![]), "~/ws", false);
    assert!(long.contains("    line 40\n... 5 more lines\n"), "{long}");
    assert!(!long.contains("line 41"));
    assert!(long.starts_with("-- clai - ~/ws - q1 --\n"));
}

#[test]
fn command_question_text() {
    let part = |text: &str, action: Action, reason: Reason| PartDecision {
        text: text.into(),
        words: vec![],
        assignments: 0,
        decision: Decision { action, reason },
    };
    let details = Details::RunCommand {
        command: "cargo build && ./deploy.sh staging".into(),
        cwd: "~/ws".into(),
        timeout_s: 120,
        env: vec![],
        parts: vec![
            part(
                "cargo build",
                Action::Allow,
                Reason::Rule {
                    rule: "allow run_command(cargo build)".into(),
                    source: "config.toml:28".into(),
                    message: None,
                },
            ),
            part(
                "./deploy.sh staging",
                Action::Ask,
                Reason::Default {
                    tool: Tool::RunCommand,
                },
            ),
        ],
        redirects: vec![],
    };
    let text = render_question(&permission(details, vec![]), "~/ws", true);
    assert!(text.contains("  in       ~/ws, timeout 120 s\n"), "{text}");
    assert!(
        text.contains(
            "  part 1   cargo build          allow  rule allow run_command(cargo build) (config.toml:28)\n"
        ),
        "{text}"
    );
    assert!(
        text.contains("  part 2   ./deploy.sh staging  ask    default for run_command\n"),
        "{text}"
    );
}

/// A writer the test can read back.
#[derive(Clone, Default)]
struct Shared(Arc<Mutex<Vec<u8>>>);

impl Write for Shared {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Shared {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

async fn answer(question: &Question, input: &str) -> (Reply, String) {
    let out = Shared::default();
    let asker = LineAsker::new(
        Box::new(Cursor::new(input.to_owned())),
        Box::new(out.clone()),
        "~/ws".into(),
        true,
    );
    let reply = asker.ask(question).await;
    (reply, out.text())
}

fn answered(answer: Answer) -> Reply {
    Reply::Answer {
        answer,
        by: "terminal".into(),
    }
}

#[tokio::test]
async fn line_answers() {
    let proposals = vec![
        vec!["write_file(./a.txt)".to_owned()],
        vec!["edit(./**)".to_owned()],
    ];
    let q = permission(new_file(1), proposals);
    assert_eq!(answer(&q, "y\n").await.0, answered(Answer::Allow));
    assert_eq!(answer(&q, "N\n").await.0, answered(Answer::Deny));
    assert_eq!(answer(&q, "c\n").await.0, answered(Answer::Cancel));
    assert_eq!(
        answer(&q, "e\nuse b.txt\n").await.0,
        answered(Answer::DenyMessage {
            text: "use b.txt".into()
        })
    );
    let (reply, out) = answer(&q, "a\n\n").await;
    assert_eq!(
        reply,
        answered(Answer::AllowAlways {
            rules: vec!["write_file(./a.txt)".into()]
        })
    );
    assert!(
        out.contains("  1  write_file(./a.txt)\n  2  edit(./**)\nrule [1]> "),
        "{out}"
    );
    assert_eq!(
        answer(&q, "a\n2\n").await.0,
        answered(Answer::AllowAlways {
            rules: vec!["edit(./**)".into()]
        })
    );
    assert_eq!(
        answer(&q, "a\nwrite_file(*.txt)\n").await.0,
        answered(Answer::AllowAlways {
            rules: vec!["write_file(*.txt)".into()]
        })
    );
    // A wrong key, a wrong number, then an answer.
    let (reply, out) = answer(&q, "x\na\n9\ny\n").await;
    assert_eq!(reply, answered(Answer::Allow));
    assert!(out.contains("answer y, a, n, e or c"));
    assert!(out.contains("no rule 9"));
    // The end of input: no one answers.
    assert_eq!(answer(&q, "").await.0, Reply::NoOne);
    assert_eq!(answer(&q, "e\n").await.0, Reply::NoOne);
}

#[tokio::test]
async fn limit_answers_and_rejections() {
    let q = Question {
        id: "q2".into(),
        session: Default::default(),
        opened: 0,
        turn: TurnInfo {
            number: 1,
            requests: 50,
            tool_calls: 3,
        },
        summary: "turn limit reached (50 requests, 3 tool calls)".into(),
        choices: LIMIT_CHOICES.to_vec(),
        kind: QuestionKind::Limit(Limit {
            max_requests: 50,
            max_tool_calls: 120,
            requests: 50,
            tool_calls: 3,
        }),
    };
    let (reply, out) = answer(&q, "y\n").await;
    assert_eq!(reply, answered(Answer::Continue));
    assert_eq!(
        out,
        "── clai · ~/ws · q2 ──\n\
         turn limit reached (50 requests, 3 tool calls)\n\
         [y] continue  [n] stop  [c] cancel turn\n> "
    );
    assert_eq!(answer(&q, "n\n").await.0, answered(Answer::Stop));
    assert_eq!(answer(&q, "a\nc\n").await.0, answered(Answer::Cancel));

    let out = Shared::default();
    let asker = LineAsker::new(
        Box::new(Cursor::new(String::new())),
        Box::new(out.clone()),
        "~".into(),
        true,
    );
    asker.reject(&q, "terminal", "the rules do not allow this call");
    assert_eq!(
        out.text(),
        "  not accepted: the rules do not allow this call\n"
    );
}

#[test]
fn chat_style_has_colour_and_view() {
    let style = Style {
        name: "docs".into(),
        place: "~/ws".into(),
        unicode: true,
        colour: true,
        pager: true,
    };
    let q = permission(new_file(45), vec![]);
    assert!(has_more(&q));
    let text = render_question_styled(&q, &style);
    assert!(text.starts_with("── docs · ~/ws · q1 ──\n"), "{text}");
    assert!(text.contains("… 5 more lines, [v] view all\n"), "{text}");
    let mut dialog = Dialog::new(q.clone(), true);
    assert_eq!(dialog.line("v"), Step::View);
    let mut no_pager = Dialog::new(q, false);
    assert!(matches!(no_pager.line("v"), Step::Again(_)));

    let edit = permission(
        Details::EditFile {
            path: path("a.rs"),
            diff: crate::tools::Text::new("@@ -1 +1 @@\n-old\n+new\n".into()),
            first_line: 1,
            last_line: 1,
            occurrences: 1,
            size: 3,
            modified: None,
        },
        vec![],
    );
    assert!(!has_more(&edit));
    let text = render_question_styled(&edit, &style);
    assert!(text.contains("    \x1b[31m-old\x1b[0m\n"), "{text}");
    assert!(text.contains("    \x1b[32m+new\x1b[0m\n"), "{text}");
    assert!(text.contains("    \x1b[36m@@ -1 +1 @@\x1b[0m\n"), "{text}");
}
