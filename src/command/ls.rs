//! `clai ls` (see `spec/command.md`).

use std::path::Path;

use clap::Args;
use serde_json::{Value, json};

use crate::agent::{QuestionKind, State, now_secs};
use crate::client::with_client;
use crate::error::Result;
use crate::output::write_stdout;
use crate::protocol::SessionInfo;

#[derive(Debug, Args)]
pub struct Ls {
    /// Print a JSON array instead of the table.
    #[arg(long)]
    pub json: bool,
}

impl Ls {
    pub fn run(&self, config: Option<&Path>) -> Result<()> {
        let sessions = with_client(config, true, async |client| client.list().await)?;
        let text = if self.json {
            let list: Vec<Value> = sessions.iter().map(json_of).collect();
            format!("{}\n", Value::Array(list))
        } else {
            table(&sessions, now_secs())
        };
        if sessions.is_empty() && !self.json {
            return Ok(());
        }
        write_stdout(&text)
    }
}

fn state_name(state: State) -> &'static str {
    match state {
        State::Idle => "idle",
        State::Working => "working",
        State::Waiting => "waiting",
    }
}

fn json_of(s: &SessionInfo) -> Value {
    let question = s.question.as_ref().map(|q| {
        json!({
            "id": q.id,
            "kind": match q.kind {
                QuestionKind::Permission(_) => "permission",
                QuestionKind::Limit(_) => "limit",
            },
            "summary": q.summary,
        })
    });
    json!({
        "name": s.name,
        "workspace": s.path,
        "model": s.model,
        "state": state_name(s.state),
        "created": s.created,
        "queued": s.queued,
        "question": question,
    })
}

/// One line per session: name, workspace, model, state, age, and what a
/// waiting session asks.
pub fn table(sessions: &[SessionInfo], now: u64) -> String {
    let rows: Vec<[String; 5]> = sessions
        .iter()
        .map(|s| {
            [
                s.name.clone(),
                s.workspace.clone(),
                s.model.clone(),
                state_name(s.state).to_owned(),
                age(now.saturating_sub(s.created)),
            ]
        })
        .collect();
    let width = |i: usize| {
        rows.iter()
            .filter_map(|r| r.get(i))
            .map(|c| c.chars().count())
            .max()
            .unwrap_or(0)
    };
    let widths = [width(0), width(1), width(2), width(3)];
    let mut out = String::new();
    for (row, session) in rows.iter().zip(sessions) {
        let [name, workspace, model, state, age] = row;
        let [w0, w1, w2, w3] = widths;
        let mut line = format!("{name:<w0$}  {workspace:<w1$}  {model:<w2$}  {state:<w3$}  {age}");
        if let Some(q) = &session.question {
            line.push_str(&format!("  ? {}", q.summary));
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// `45s`, `12m`, `3h`, `2d`.
pub fn age(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86_400 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn info(name: &str, state: State, created: u64) -> SessionInfo {
        SessionInfo {
            id: 1,
            name: name.into(),
            workspace: "~/ws/clai".into(),
            path: PathBuf::from("/home/u/ws/clai"),
            model: "anthropic/sonnet".into(),
            state,
            created,
            queued: 0,
            question: None,
        }
    }

    #[test]
    fn ages() {
        assert_eq!(age(0), "0s");
        assert_eq!(age(59), "59s");
        assert_eq!(age(61), "1m");
        assert_eq!(age(7300), "2h");
        assert_eq!(age(200_000), "2d");
    }

    #[test]
    fn lines() {
        let sessions = [
            info("clai", State::Working, 1000),
            info("docs-2", State::Idle, 400),
        ];
        assert_eq!(
            table(&sessions, 1030),
            "clai    ~/ws/clai  anthropic/sonnet  working  30s\n\
             docs-2  ~/ws/clai  anthropic/sonnet  idle     10m\n"
        );
    }
}
