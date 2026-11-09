//! `ask_user`: questions with options from the model to the user (see
//! `spec/tools.md`, "ask_user"). Pure: the agent opens the question and
//! waits for the answer; this checks the arguments and the answers and
//! makes the result text.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::provider::ToolSpec;

pub const NAME: &str = "ask_user";

pub const MAX_QUESTIONS: usize = 4;
pub const MIN_OPTIONS: usize = 2;
pub const MAX_OPTIONS: usize = 6;
const MAX_QUESTION: usize = 500;
const MAX_HEADER: usize = 16;
const MAX_LABEL: usize = 60;
const MAX_DESCRIPTION: usize = 300;
const MAX_CONTEXT: usize = 4000;
pub const MAX_TEXT: usize = 4000;

/// The result when the user declines.
pub const DECLINED: &str =
    "the user declined to answer; decide yourself, or ask in your final answer";
/// The result when no one can answer.
pub const NOT_AVAILABLE: &str = "ask_user is not available: no one can answer; decide yourself and say what you chose in \
     your answer";

/// One checked question.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskQuestion {
    pub question: String,
    /// Short label, at most 16 characters.
    pub header: String,
    pub options: Vec<AskOption>,
    #[serde(default)]
    pub multiple: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskOption {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// The answer to one question: chosen options by index and a typed text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserAnswer {
    #[serde(default)]
    pub selected: Vec<usize>,
    #[serde(default)]
    pub text: Option<String>,
}

impl UserAnswer {
    /// The typed text, trimmed, if not empty.
    pub fn typed(&self) -> Option<&str> {
        self.text
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
    }
}

/// Checked arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asked {
    pub questions: Vec<AskQuestion>,
    pub context: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    questions: Vec<QuestionIn>,
    #[serde(default)]
    context: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QuestionIn {
    question: String,
    #[serde(default)]
    header: Option<String>,
    options: Vec<OptionIn>,
    #[serde(default)]
    multiple: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OptionIn {
    label: String,
    #[serde(default)]
    description: Option<String>,
}

fn chars(text: &str) -> usize {
    text.chars().count()
}

fn too_long(what: &str, text: &str, max: usize) -> Result<(), String> {
    let n = chars(text);
    if n > max {
        Err(format!("{what} has {n} characters; at most {max}"))
    } else {
        Ok(())
    }
}

/// Cuts a text to `max` characters, ending in `…` when cut.
fn cut(text: &str, max: usize) -> String {
    if chars(text) <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out = out.trim_end().to_owned();
    out.push('…');
    out
}

/// Checks the arguments of a call.
pub fn check(arguments: &Value) -> Result<Asked, String> {
    if let Value::String(raw) = arguments {
        return Err(format!(
            "the arguments of {NAME} are not valid JSON: {}",
            super::cut_for_model(raw)
        ));
    }
    let args: Args = serde_json::from_value(arguments.clone())
        .map_err(|e| format!("invalid arguments for {NAME}: {e}"))?;
    let n = args.questions.len();
    if n == 0 || n > MAX_QUESTIONS {
        return Err(format!(
            "questions must have 1–{MAX_QUESTIONS} entries, not {n}"
        ));
    }
    let context = args
        .context
        .map(|c| c.trim().to_owned())
        .filter(|c| !c.is_empty());
    if let Some(context) = &context {
        too_long("context", context, MAX_CONTEXT)?;
    }
    let mut questions = Vec::new();
    for (i, q) in args.questions.into_iter().enumerate() {
        let at = format!("question {}", i + 1);
        let question = q.question.trim().to_owned();
        if question.is_empty() {
            return Err(format!("{at}: the question is empty"));
        }
        too_long(&at, &question, MAX_QUESTION)?;
        let header = match q
            .header
            .map(|h| h.trim().to_owned())
            .filter(|h| !h.is_empty())
        {
            Some(header) => {
                too_long(&format!("{at}: header"), &header, MAX_HEADER)?;
                header
            }
            None => cut(question.lines().next().unwrap_or(""), MAX_HEADER),
        };
        let count = q.options.len();
        if !(MIN_OPTIONS..=MAX_OPTIONS).contains(&count) {
            return Err(format!(
                "{at}: options must have {MIN_OPTIONS}–{MAX_OPTIONS} entries, not {count}"
            ));
        }
        let mut options: Vec<AskOption> = Vec::new();
        for (j, o) in q.options.into_iter().enumerate() {
            let what = format!("{at}, option {}", j + 1);
            let label = o.label.trim().to_owned();
            if label.is_empty() {
                return Err(format!("{what}: the label is empty"));
            }
            too_long(&format!("{what}: label"), &label, MAX_LABEL)?;
            if options
                .iter()
                .any(|other| other.label.to_lowercase() == label.to_lowercase())
            {
                return Err(format!("{at}: the label `{label}` is given twice"));
            }
            let description = o
                .description
                .map(|d| d.trim().to_owned())
                .filter(|d| !d.is_empty());
            if let Some(d) = &description {
                too_long(&format!("{what}: description"), d, MAX_DESCRIPTION)?;
            }
            options.push(AskOption { label, description });
        }
        questions.push(AskQuestion {
            question,
            header,
            options,
            multiple: q.multiple,
        });
    }
    Ok(Asked { questions, context })
}

/// One line for lists: `ask_user: Database, Features`.
pub fn summary(questions: &[AskQuestion]) -> String {
    let headers: Vec<&str> = questions.iter().map(|q| q.header.as_str()).collect();
    format!("{NAME}: {}", headers.join(", "))
}

/// Checks a reply against the questions.
pub fn check_answers(questions: &[AskQuestion], answers: &[UserAnswer]) -> Result<(), String> {
    if answers.len() != questions.len() {
        return Err(format!(
            "{} answers for {} questions",
            answers.len(),
            questions.len()
        ));
    }
    for (i, (q, a)) in questions.iter().zip(answers).enumerate() {
        let at = format!("answer {}", i + 1);
        for (k, &index) in a.selected.iter().enumerate() {
            if index >= q.options.len() {
                return Err(format!(
                    "{at}: option {index} does not exist; there are {}",
                    q.options.len()
                ));
            }
            if a.selected.iter().take(k).any(|&i| i == index) {
                return Err(format!("{at}: option {index} is chosen twice"));
            }
        }
        if !q.multiple && a.selected.len() > 1 {
            return Err(format!("{at}: only one option may be chosen"));
        }
        if a.selected.is_empty() && a.typed().is_none() {
            return Err(format!("{at}: neither an option nor a text"));
        }
        if let Some(text) = &a.text {
            too_long(&format!("{at}: the text"), text, MAX_TEXT)?;
        }
    }
    Ok(())
}

/// The result for the model, for answers checked by [`check_answers`].
pub fn result_text(questions: &[AskQuestion], answers: &[UserAnswer]) -> String {
    let mut out = String::from("The user answered:");
    for (i, (q, a)) in questions.iter().zip(answers).enumerate() {
        out.push_str(&format!("\n\n{}. {}", i + 1, indent(&q.question)));
        let labels: Vec<&str> = a
            .selected
            .iter()
            .filter_map(|&k| q.options.get(k))
            .map(|o| o.label.as_str())
            .collect();
        if !labels.is_empty() {
            out.push_str(&format!("\n   {}", labels.join(", ")));
        }
        if let Some(text) = a.typed() {
            let lead = if labels.is_empty() {
                "The user wrote"
            } else {
                "The user added"
            };
            out.push_str(&format!("\n   {lead}: {}", indent(text)));
        }
    }
    out
}

/// Continues a multi-line text below a `   ` indent.
fn indent(text: &str) -> String {
    text.lines().collect::<Vec<_>>().join("\n   ")
}

/// The spec sent to the model.
pub fn spec() -> ToolSpec {
    ToolSpec {
        name: NAME.into(),
        description: "Ask the user one to four questions with options and wait for the answer. \
             Use it only for a decision that is the user's and that the code and the task do \
             not settle. Put the option you recommend first, with (recommended) in its label. \
             Do not add an \"Other\" option: the user can always type an answer of their own."
            .into(),
        input_schema: json!({
            "type": "object",
            "properties": {
                "questions": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": MAX_QUESTIONS,
                    "description": "Questions asked together.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "question": {"type": "string", "maxLength": MAX_QUESTION, "description": "The full question, one or two sentences, ending with ?."},
                            "header": {"type": "string", "maxLength": MAX_HEADER, "description": "A short label, at most 16 characters."},
                            "options": {
                                "type": "array",
                                "minItems": MIN_OPTIONS,
                                "maxItems": MAX_OPTIONS,
                                "items": {
                                    "type": "object",
                                    "properties": {
                                        "label": {"type": "string", "maxLength": MAX_LABEL, "description": "The option, 1 to 5 words."},
                                        "description": {"type": "string", "maxLength": MAX_DESCRIPTION, "description": "What choosing it means."}
                                    },
                                    "required": ["label"],
                                    "additionalProperties": false
                                }
                            },
                            "multiple": {"type": "boolean", "description": "The user may choose several options. Default false."}
                        },
                        "required": ["question", "options"],
                        "additionalProperties": false
                    }
                },
                "context": {"type": "string", "maxLength": MAX_CONTEXT, "description": "Markdown shown above the questions: why you ask."}
            },
            "required": ["questions"],
            "additionalProperties": false
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(options: Value) -> Value {
        json!({"questions": [{"question": "Which database?", "options": options}]})
    }

    fn two_options() -> Value {
        json!([{"label": "SQLite", "description": "One file."}, {"label": "Postgres"}])
    }

    #[test]
    fn checks_a_good_call() {
        let asked = check(&json!({
            "questions": [
                {"question": " Which database should the cache use? ", "header": "Database",
                 "options": [{"label": "SQLite", "description": "One file."}, {"label": "Postgres", "description": " "}]},
                {"question": "Which features should the first version have?",
                 "options": [{"label": "Search"}, {"label": "Export"}, {"label": "Import"}], "multiple": true}
            ],
            "context": "Both work."
        }))
        .unwrap();
        assert_eq!(asked.context.as_deref(), Some("Both work."));
        let q = &asked.questions;
        assert_eq!(q[0].question, "Which database should the cache use?");
        assert_eq!(q[0].header, "Database");
        assert_eq!(q[0].options[1].description, None);
        assert!(!q[0].multiple);
        assert_eq!(q[1].header, "Which features…");
        assert!(q[1].multiple);
        assert_eq!(summary(q), "ask_user: Database, Which features…");
    }

    #[test]
    fn every_limit() {
        let cases = [
            (
                json!({"questions": []}),
                "questions must have 1–4 entries, not 0",
            ),
            (
                json!({"questions": vec![json!({"question": "Q?", "options": two_options()}); 5]}),
                "not 5",
            ),
            (
                one(json!([{"label": "A"}])),
                "options must have 2–6 entries, not 1",
            ),
            (one(json!(vec![json!({"label": "A"}); 7])), "not 7"),
            (
                one(json!([{"label": "A"}, {"label": " "}])),
                "option 2: the label is empty",
            ),
            (
                one(json!([{"label": "Same"}, {"label": "same"}])),
                "the label `same` is given twice",
            ),
            (
                one(json!([{"label": "A"}, {"label": "x".repeat(61)}])),
                "label has 61 characters; at most 60",
            ),
            (
                one(json!([{"label": "A", "description": "d".repeat(301)}, {"label": "B"}])),
                "description has 301 characters; at most 300",
            ),
            (
                json!({"questions": [{"question": "q".repeat(501), "options": two_options()}]}),
                "question 1 has 501 characters; at most 500",
            ),
            (
                json!({"questions": [{"question": " ", "options": two_options()}]}),
                "the question is empty",
            ),
            (
                json!({"questions": [{"question": "Q?", "header": "h".repeat(17), "options": two_options()}]}),
                "header has 17 characters; at most 16",
            ),
            (
                json!({"questions": [{"question": "Q?", "options": two_options()}], "context": "c".repeat(4001)}),
                "context has 4001 characters; at most 4000",
            ),
            (
                json!({"questions": [{"question": "Q?"}]}),
                "missing field `options`",
            ),
            (
                json!({"questions": [], "extra": 1}),
                "unknown field `extra`",
            ),
            (Value::String("{".into()), "not valid JSON"),
        ];
        for (arguments, expected) in cases {
            let e = check(&arguments).unwrap_err();
            assert!(e.contains(expected), "{arguments}: {e}");
        }
        // The limits themselves are fine.
        let six: Vec<Value> = (0..6).map(|i| json!({"label": format!("o{i}")})).collect();
        let full = json!({"question": "q".repeat(500), "header": "h".repeat(16), "options": six});
        check(&json!({"questions": vec![full; 4], "context": "c".repeat(4000)})).unwrap();
    }

    fn asked() -> Vec<AskQuestion> {
        check(&json!({"questions": [
            {"question": "Which database should the cache use?", "options": [{"label": "SQLite"}, {"label": "Postgres"}]},
            {"question": "Which features should the first version have?", "options": [{"label": "Search"}, {"label": "Export"}, {"label": "Import"}], "multiple": true}
        ]}))
        .unwrap()
        .questions
    }

    fn answer(selected: &[usize], text: Option<&str>) -> UserAnswer {
        UserAnswer {
            selected: selected.to_vec(),
            text: text.map(str::to_owned),
        }
    }

    #[test]
    fn answers_are_checked() {
        let q = asked();
        let ok = [answer(&[0], None), answer(&[0, 1], Some("x"))];
        assert_eq!(check_answers(&q, &ok), Ok(()));
        let cases = [
            (vec![answer(&[0], None)], "1 answers for 2 questions"),
            (
                vec![answer(&[2], None), answer(&[0], None)],
                "answer 1: option 2 does not exist",
            ),
            (
                vec![answer(&[0, 1], None), answer(&[0], None)],
                "answer 1: only one option",
            ),
            (
                vec![answer(&[0], None), answer(&[1, 1], None)],
                "answer 2: option 1 is chosen twice",
            ),
            (
                vec![answer(&[], Some("  ")), answer(&[0], None)],
                "answer 1: neither an option nor a text",
            ),
            (
                vec![answer(&[0], Some(&"t".repeat(4001))), answer(&[0], None)],
                "the text has 4001 characters",
            ),
        ];
        for (answers, expected) in cases {
            let e = check_answers(&q, &answers).unwrap_err();
            assert!(e.contains(expected), "{answers:?}: {e}");
        }
    }

    #[test]
    fn result_texts() {
        let q = asked();
        assert_eq!(
            result_text(
                &q,
                &[
                    answer(&[0], None),
                    answer(&[0, 1], Some("keep the old export format too."))
                ]
            ),
            "The user answered:\n\n\
             1. Which database should the cache use?\n   SQLite\n\n\
             2. Which features should the first version have?\n   Search, Export\n   \
             The user added: keep the old export format too."
        );
        assert_eq!(
            result_text(&q[..1], &[answer(&[], Some("Redis,\nif possible"))]),
            "The user answered:\n\n\
             1. Which database should the cache use?\n   The user wrote: Redis,\n   if possible"
        );
    }
}
