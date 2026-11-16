use serde_json::json;

use super::*;
use crate::agent::{CallResult, ToolCallEvent, TurnEnd};
use crate::tools::report::{Severity, check};

fn running(id: &str, name: &str, summary: &str) -> SessionEvent {
    SessionEvent::ToolCall(ToolCallEvent {
        id: id.into(),
        name: name.into(),
        state: CallState::Running,
        summary: Some(summary.into()),
        arguments: json!({}),
        output: None,
        result: None,
        details: None,
    })
}

fn ended(id: &str, name: &str, state: CallState, arguments: Value, text: &str) -> SessionEvent {
    SessionEvent::ToolCall(ToolCallEvent {
        id: id.into(),
        name: name.into(),
        state,
        summary: None,
        arguments,
        output: None,
        result: Some(CallResult {
            text: text.into(),
            is_error: state != CallState::Done,
        }),
        details: None,
    })
}

fn turn_end(input: u64, output: u64, requests: u32) -> SessionEvent {
    SessionEvent::TurnEnd(TurnEnd {
        reason: EndReason::Done,
        requests,
        tool_calls: 0,
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: 0,
    })
}

fn run(outcome: Outcome) -> Run<'static> {
    Run {
        workspace: Path::new("/ws/p"),
        model: "test/m",
        outcome,
        duration: 184,
    }
}

fn collected() -> Collector {
    let mut c = Collector::default();
    let events = [
        running("1", "search_files", "unsafe in src/**/*.rs"),
        ended(
            "1",
            "search_files",
            CallState::Done,
            json!({"pattern": "unsafe"}),
            "a\nb\n",
        ),
        running("2", "read_file", "src/a.rs (lines 1–2)"),
        ended(
            "2",
            "read_file",
            CallState::Done,
            json!({"path": "/ws/p/src/a.rs"}),
            "1\tx\n2\ty\n",
        ),
        ended(
            "3",
            "read_file",
            CallState::Done,
            json!({"path": "src/a.rs"}),
            "1\tx\n",
        ),
        ended(
            "4",
            "edit_file",
            CallState::Done,
            json!({"path": "src/a.rs"}),
            "1 change",
        ),
        ended(
            "5",
            "write_file",
            CallState::Done,
            json!({"path": "src/a.rs"}),
            "replaced",
        ),
        ended(
            "6",
            "run_command",
            CallState::Error,
            json!({"command": "cargo test"}),
            "failed\nexit code 101",
        ),
        ended(
            "7",
            "run_command",
            CallState::Denied,
            json!({"command": "rm -rf x"}),
            "denied: needs permission, and no one can answer",
        ),
        ended(
            "8",
            "report",
            CallState::Done,
            json!({}),
            "saved: 1 finding",
        ),
        turn_end(100, 20, 3),
        turn_end(50, 5, 1),
    ];
    for e in &events {
        c.event(e);
    }
    c
}

fn reported() -> Reported {
    check(&json!({
        "status": "partly_solved",
        "summary": "Found one.",
        "findings": [{"severity": "high", "title": "Links", "file": "/ws/p/src/a.rs",
                      "line": 4, "detail": "Resolve.\nThen check."}],
        "follow_ups": ["decide"]
    }))
    .unwrap()
}

#[test]
fn facts_from_the_events() {
    let r = collected().report(run(Outcome::Done), Some(&reported()), "unused");
    assert_eq!(r.version, 1);
    assert_eq!(r.status, Status::PartlySolved);
    assert_eq!(r.findings[0].severity, Severity::High);
    assert_eq!(r.findings[0].file.as_deref(), Some("src/a.rs"), "relative");
    // `report` itself is left out.
    let tools: Vec<&str> = r.tool_calls.iter().map(|c| c.tool.as_str()).collect();
    assert_eq!(
        tools,
        [
            "search_files",
            "read_file",
            "read_file",
            "edit_file",
            "write_file",
            "run_command",
            "run_command"
        ]
    );
    assert_eq!(r.tool_calls[0].summary, "unsafe in src/**/*.rs");
    assert_eq!(r.tool_calls[0].result, "2 lines");
    // Without a summary event: the main argument.
    assert_eq!(r.tool_calls[6].summary, "rm -rf x");
    assert_eq!(r.files_read, ["src/a.rs"], "each path once");
    assert_eq!(
        r.changes,
        [Change {
            path: "src/a.rs".into(),
            kind: ChangeKind::Edited
        }]
    );
    assert_eq!(
        r.commands,
        [CommandRun {
            command: "cargo test".into(),
            exit_code: Some(101)
        }],
        "denied commands did not run"
    );
    assert_eq!(
        r.counts,
        Counts {
            done: 5,
            error: 1,
            denied: 1,
            cancelled: 0
        }
    );
    assert_eq!(
        r.usage,
        Usage {
            input_tokens: 150,
            output_tokens: 25,
            cache_read_tokens: 0,
            requests: 4
        }
    );
}

#[test]
fn without_a_report_the_status_is_unknown() {
    let r = Collector::default().report(run(Outcome::Failed), None, " invalid key ");
    assert_eq!(r.status, Status::Unknown);
    assert_eq!(r.summary, "invalid key");
    assert!(r.findings.is_empty() && r.tool_calls.is_empty());
}

#[test]
fn the_json_shape() {
    let r = collected().report(run(Outcome::Limit), Some(&reported()), "");
    let value: Value = serde_json::from_str(&render_json(&r)).unwrap();
    for key in [
        "version",
        "workspace",
        "model",
        "outcome",
        "status",
        "summary",
        "findings",
        "follow_ups",
        "changes",
        "commands",
        "files_read",
        "tool_calls",
        "counts",
        "usage",
        "duration",
    ] {
        assert!(value.get(key).is_some(), "{key}");
    }
    assert_eq!(value["outcome"], "limit");
    assert_eq!(value["status"], "partly_solved");
    assert_eq!(value["findings"][0]["severity"], "high");
    assert_eq!(value["changes"][0]["kind"], "edited");
    assert_eq!(value["tool_calls"][5]["state"], "error");
    assert_eq!(value["commands"][0]["exit_code"], 101);
    assert!(render_json(&r).ends_with("}\n"));
}

#[test]
fn the_text() {
    let r = collected().report(run(Outcome::Done), Some(&reported()), "");
    let text = render_text(&r, "~/p", true);
    assert!(
        text.starts_with(
            "# Partly solved\n\n~/p · test/m · 4 min · 150 in, 25 out\n\nFound one.\n"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "## Findings\n\n- **high** Links — `src/a.rs:4`\n  Resolve.\n  Then check.\n"
        ),
        "{text}"
    );
    assert!(
        text.contains("## Changes\n\n- edited `src/a.rs`\n"),
        "{text}"
    );
    assert!(text.contains("## Follow-ups\n\n- decide\n"), "{text}");
    assert!(
        text.contains("## Tool calls\n\n- ✓ search_files unsafe in src/**/*.rs — 2 lines\n"),
        "{text}"
    );
    assert!(
        text.contains("- ✗ run_command cargo test — exit code 101\n"),
        "{text}"
    );
    let ascii = render_text(&r, "~/p", false);
    assert!(ascii.contains("- + search_files"), "{ascii}");

    // An empty report: only the title, the line and the summary.
    let r = Collector::default().report(run(Outcome::Limit), None, "Stopped.");
    let text = render_text(&r, "~/p", true);
    assert_eq!(
        text,
        "# Unknown\n\n~/p · test/m · 4 min · 0 in, 0 out\n\nThe run stopped at its limit.\n\nStopped.\n"
    );
}

#[test]
fn the_frame() {
    let m = work_message("find {task} bugs");
    assert!(m.contains("<task>\nfind {task} bugs\n</task>"), "{m}");
    assert!(m.starts_with("Work on this task"));
}

#[test]
fn exit_codes() {
    assert_eq!(exit_code("out\nexit code 0\n"), Some(0));
    assert_eq!(exit_code("killed by signal 9"), None);
}
