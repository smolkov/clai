//! Sessions as text: the table of `clai ls`, also `/sessions` in `clai chat`,
//! state names and ages, also in the TUI's session list.

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
            usage: Default::default(),
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
