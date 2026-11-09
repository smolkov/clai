//! Sessions as text: the table of `clai ls`, also `/sessions` in `clai chat`,
//! state names, ages and the delete question, also in the TUI's session list.

use crate::agent::State;
use crate::output::{Align, columns};
use crate::protocol::SessionInfo;

/// As `clai ls` and the protocol name it.
pub fn state_name(state: State) -> &'static str {
    match state {
        State::Idle => "idle",
        State::Working => "working",
        State::Waiting => "waiting",
        State::Saved => "saved",
    }
}

/// One line per session: name, workspace, model, state, age, and what a
/// waiting session asks.
pub fn table(sessions: &[SessionInfo], now: u64) -> String {
    let rows: Vec<Vec<String>> = sessions
        .iter()
        .map(|s| {
            vec![
                s.name.clone(),
                s.workspace.clone(),
                s.model.clone(),
                state_name(s.state).to_owned(),
                age(now.saturating_sub(s.created)),
            ]
        })
        .collect();
    let mut out = String::new();
    for (mut line, session) in columns(&rows, &[Align::Left; 4]).into_iter().zip(sessions) {
        if let Some(q) = &session.question {
            line.push_str(&format!("  ? {}", q.summary));
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

/// When the session was last used: `updated`, or `created` from an older
/// server that does not send it.
pub fn last_used(session: &SessionInfo) -> u64 {
    if session.updated > 0 {
        session.updated
    } else {
        session.created
    }
}

/// The question before deleting `sessions`, without the keys: what is lost.
pub fn confirm_delete(sessions: &[&SessionInfo]) -> String {
    match sessions {
        [] => String::new(),
        [one] => {
            let mut text = format!("Delete {}?", one.name);
            match one.state {
                State::Working => text.push_str(" It is working: the turn is cancelled."),
                State::Waiting => text.push_str(" Its question is closed unanswered."),
                State::Idle | State::Saved => {}
            }
            if one.turns == 0 {
                text.push_str(" No turns yet.");
            } else {
                let tokens = one.usage.input_tokens + one.usage.output_tokens;
                text.push_str(&format!(
                    " {}, {} tokens.",
                    count(one.turns, "turn", "turns"),
                    crate::text::tokens(tokens)
                ));
            }
            text
        }
        many => {
            let mut text = format!("Delete {} sessions?", many.len());
            let working = many.iter().filter(|s| s.state == State::Working).count();
            let waiting = many.iter().filter(|s| s.state == State::Waiting).count();
            let mut busy = Vec::new();
            if working > 0 {
                let verb = if working == 1 { "is" } else { "are" };
                busy.push(format!("{working} {verb} working"));
            }
            if waiting > 0 {
                let verb = if waiting == 1 { "waits" } else { "wait" };
                busy.push(format!("{waiting} {verb} for an answer"));
            }
            if !busy.is_empty() {
                text.push_str(&format!(" {}.", busy.join(", ")));
            }
            text
        }
    }
}

fn count(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
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
            updated: 0,
            turns: 0,
            queued: 0,
            question: None,
            usage: Default::default(),
        }
    }

    #[test]
    fn delete_questions() {
        let mut old = info("old", State::Saved, 0);
        old.turns = 14;
        old.usage.input_tokens = 200_000;
        old.usage.output_tokens = 10_000;
        assert_eq!(
            confirm_delete(&[&old]),
            "Delete old? 14 turns, 210k tokens."
        );
        let mut project = info("project", State::Working, 0);
        project.turns = 1;
        project.usage.input_tokens = 41_000;
        assert_eq!(
            confirm_delete(&[&project]),
            "Delete project? It is working: the turn is cancelled. 1 turn, 41k tokens."
        );
        let docs = info("docs", State::Waiting, 0);
        assert_eq!(
            confirm_delete(&[&docs]),
            "Delete docs? Its question is closed unanswered. No turns yet."
        );
        let idle = info("idle", State::Idle, 0);
        assert_eq!(
            confirm_delete(&[&old, &project, &docs, &idle, &idle]),
            "Delete 5 sessions? 1 is working, 1 waits for an answer."
        );
        assert_eq!(
            confirm_delete(&[&project, &project, &docs, &docs]),
            "Delete 4 sessions? 2 are working, 2 wait for an answer."
        );
        assert_eq!(confirm_delete(&[&old, &idle]), "Delete 2 sessions?");
    }

    #[test]
    fn last_use_falls_back_to_creation() {
        let mut s = info("s", State::Idle, 100);
        assert_eq!(last_used(&s), 100);
        s.updated = 300;
        assert_eq!(last_used(&s), 300);
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
