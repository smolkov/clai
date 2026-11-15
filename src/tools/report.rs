//! `report`: the model hands in the result of `clai run --report` (see
//! `spec/tools.md`, "report", and RFC 0027). Pure: the agent keeps the
//! report; this checks the arguments and makes the spec.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::provider::ToolSpec;

pub const NAME: &str = "report";

const MAX_SUMMARY: usize = 4000;
const MAX_FINDINGS: usize = 200;
const MAX_TITLE: usize = 200;
const MAX_DETAIL: usize = 4000;
const MAX_FILE: usize = 1000;
const MAX_FOLLOW_UPS: usize = 50;
const MAX_FOLLOW_UP: usize = 1000;

/// The result of a call where the tool is not offered.
pub const NOT_AVAILABLE: &str = "report is not available here; answer in text";

/// How the task went, in the model's view; `unknown` when it gave no
/// report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Solved,
    PartlySolved,
    NotSolved,
    Unknown,
}

impl Status {
    /// `partly solved`.
    pub fn text(self) -> &'static str {
        match self {
            Status::Solved => "solved",
            Status::PartlySolved => "partly solved",
            Status::NotSolved => "not solved",
            Status::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Critical,
    High,
    Medium,
    Low,
    Info,
}

impl Severity {
    const ALL: [Severity; 5] = [
        Severity::Critical,
        Severity::High,
        Severity::Medium,
        Severity::Low,
        Severity::Info,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Severity::Critical => "critical",
            Severity::High => "high",
            Severity::Medium => "medium",
            Severity::Low => "low",
            Severity::Info => "info",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub severity: Severity,
    pub title: String,
    pub file: Option<String>,
    pub line: Option<u64>,
    pub detail: Option<String>,
}

/// What the model reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reported {
    pub status: Status,
    pub summary: String,
    pub findings: Vec<Finding>,
    pub follow_ups: Vec<String>,
}

impl Reported {
    /// `1 finding`, `3 findings`, `no findings`.
    pub fn findings_text(&self) -> String {
        match self.findings.len() {
            0 => "no findings".to_owned(),
            1 => "1 finding".to_owned(),
            n => format!("{n} findings"),
        }
    }
}

#[derive(Deserialize)]
struct Args {
    status: String,
    summary: String,
    #[serde(default)]
    findings: Vec<FindingIn>,
    #[serde(default)]
    follow_ups: Vec<String>,
}

#[derive(Deserialize)]
struct FindingIn {
    severity: String,
    title: String,
    #[serde(default)]
    file: Option<String>,
    #[serde(default)]
    line: Option<u64>,
    #[serde(default)]
    detail: Option<String>,
}

fn too_long(what: &str, text: &str, max: usize) -> Result<(), String> {
    let n = text.chars().count();
    if n > max {
        Err(format!("{what} has {n} characters; at most {max}"))
    } else {
        Ok(())
    }
}

/// A trimmed text, `None` when empty.
fn optional(text: Option<String>) -> Option<String> {
    text.map(|t| t.trim().to_owned()).filter(|t| !t.is_empty())
}

/// Checks the arguments of a call.
pub fn check(arguments: &Value) -> Result<Reported, String> {
    if let Value::String(raw) = arguments {
        return Err(format!(
            "the arguments of {NAME} are not valid JSON: {}",
            super::cut_for_model(raw)
        ));
    }
    let args: Args = serde_json::from_value(arguments.clone())
        .map_err(|e| format!("invalid arguments for {NAME}: {e}"))?;
    let status = match args.status.trim() {
        "solved" => Status::Solved,
        "partly_solved" => Status::PartlySolved,
        "not_solved" => Status::NotSolved,
        other => {
            return Err(format!(
                "status `{other}` is not one of solved, partly_solved, not_solved"
            ));
        }
    };
    let summary = args.summary.trim().to_owned();
    if summary.is_empty() {
        return Err("the summary is empty".to_owned());
    }
    too_long("summary", &summary, MAX_SUMMARY)?;
    let n = args.findings.len();
    if n > MAX_FINDINGS {
        return Err(format!("findings has {n} entries; at most {MAX_FINDINGS}"));
    }
    let mut findings = Vec::new();
    for (i, f) in args.findings.into_iter().enumerate() {
        let at = format!("finding {}", i + 1);
        let severity = Severity::ALL
            .into_iter()
            .find(|s| s.name() == f.severity.trim())
            .ok_or_else(|| {
                format!(
                    "{at}: severity `{}` is not one of critical, high, medium, low, info",
                    f.severity.trim()
                )
            })?;
        let title = f.title.trim().to_owned();
        if title.is_empty() {
            return Err(format!("{at}: the title is empty"));
        }
        too_long(&format!("{at}: title"), &title, MAX_TITLE)?;
        let file = optional(f.file);
        if let Some(file) = &file {
            too_long(&format!("{at}: file"), file, MAX_FILE)?;
        }
        if f.line == Some(0) {
            return Err(format!("{at}: lines are counted from 1"));
        }
        let detail = optional(f.detail);
        if let Some(detail) = &detail {
            too_long(&format!("{at}: detail"), detail, MAX_DETAIL)?;
        }
        findings.push(Finding {
            severity,
            title,
            file,
            line: f.line,
            detail,
        });
    }
    let n = args.follow_ups.len();
    if n > MAX_FOLLOW_UPS {
        return Err(format!(
            "follow_ups has {n} entries; at most {MAX_FOLLOW_UPS}"
        ));
    }
    let mut follow_ups = Vec::new();
    for (i, item) in args.follow_ups.into_iter().enumerate() {
        let item = item.trim().to_owned();
        if item.is_empty() {
            continue;
        }
        too_long(&format!("follow-up {}", i + 1), &item, MAX_FOLLOW_UP)?;
        follow_ups.push(item);
    }
    Ok(Reported {
        status,
        summary,
        findings,
        follow_ups,
    })
}

/// The result of a valid call. Its first line is the short result of the
/// tool line.
pub fn result_text(reported: &Reported) -> String {
    format!(
        "saved: {}\nEnd with a short answer; call {NAME} again only if the result changes.",
        reported.findings_text()
    )
}

/// The spec sent to the model.
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: NAME.into(),
        description: "Hand in the result of your work on the task, once, when you are done: \
                      the status, a summary, the findings the task asked for, and what is \
                      left to do. Then end with a short answer. Call it again only if the \
                      result changes; the last call counts."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "status": {
                    "type": "string",
                    "enum": ["solved", "partly_solved", "not_solved"],
                    "description": "Whether the task is done."
                },
                "summary": {
                    "type": "string",
                    "description": "What you did and the outcome, in a few sentences."
                },
                "findings": {
                    "type": "array",
                    "maxItems": MAX_FINDINGS,
                    "description": "Problems or facts the task asked for; empty if none.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "severity": {"type": "string", "enum": ["critical", "high", "medium", "low", "info"]},
                            "title": {"type": "string", "description": "One line."},
                            "file": {"type": "string", "description": "Path relative to the workspace."},
                            "line": {"type": "integer", "minimum": 1},
                            "detail": {"type": "string", "description": "What it is, why it matters, how to fix it."}
                        },
                        "required": ["severity", "title"]
                    }
                },
                "follow_ups": {
                    "type": "array",
                    "maxItems": MAX_FOLLOW_UPS,
                    "items": {"type": "string"},
                    "description": "What is left to do, one item each."
                }
            },
            "required": ["status", "summary"]
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn finding(severity: &str) -> Value {
        json!({"severity": severity, "title": "Path check follows links",
               "file": " src/a.rs ", "line": 41, "detail": " Resolve it. "})
    }

    #[test]
    fn a_valid_report() {
        let r = check(&json!({
            "status": "partly_solved",
            "summary": " Found two. ",
            "findings": [finding("high"), {"severity": "info", "title": "x", "file": ""}],
            "follow_ups": ["decide", " "],
            "extra": true
        }))
        .unwrap();
        assert_eq!(r.status, Status::PartlySolved);
        assert_eq!(r.summary, "Found two.");
        assert_eq!(r.findings.len(), 2);
        assert_eq!(r.findings[0].severity, Severity::High);
        assert_eq!(r.findings[0].file.as_deref(), Some("src/a.rs"));
        assert_eq!(r.findings[0].line, Some(41));
        assert_eq!(r.findings[0].detail.as_deref(), Some("Resolve it."));
        assert_eq!(r.findings[1].file, None, "empty: none");
        assert_eq!(r.follow_ups, ["decide"]);
        assert_eq!(r.findings_text(), "2 findings");
        assert!(result_text(&r).starts_with("saved: 2 findings\n"));
        // Only status and summary are needed.
        let r = check(&json!({"status": "solved", "summary": "Done."})).unwrap();
        assert!(r.findings.is_empty());
        assert_eq!(r.findings_text(), "no findings");
    }

    #[test]
    fn wrong_calls_name_the_problem() {
        let cases = [
            (json!({"summary": "x"}), "invalid arguments for report"),
            (
                json!({"status": "unknown", "summary": "x"}),
                "status `unknown` is not one of",
            ),
            (
                json!({"status": "solved", "summary": " "}),
                "the summary is empty",
            ),
            (
                json!({"status": "solved", "summary": "x", "findings": [finding("urgent")]}),
                "finding 1: severity `urgent` is not one of",
            ),
            (
                json!({"status": "solved", "summary": "x", "findings": [{"severity": "low", "title": ""}]}),
                "finding 1: the title is empty",
            ),
            (
                json!({"status": "solved", "summary": "x", "findings": [{"severity": "low", "title": "t", "line": 0}]}),
                "finding 1: lines are counted from 1",
            ),
            (
                json!({"status": "solved", "summary": "x".repeat(4001)}),
                "summary has 4001 characters; at most 4000",
            ),
            (json!("{not json"), "are not valid JSON"),
        ];
        for (args, expected) in cases {
            let e = check(&args).unwrap_err();
            assert!(e.contains(expected), "{args}: {e}");
        }
    }

    #[test]
    fn the_spec_lists_the_statuses() {
        let spec = spec();
        assert_eq!(spec.name, NAME);
        assert_eq!(
            spec.input_schema["properties"]["status"]["enum"],
            json!(["solved", "partly_solved", "not_solved"])
        );
    }
}
