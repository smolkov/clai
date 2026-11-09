//! The filter of the session list (see `spec/tui.md`, "Sessions"): words
//! that each must match a session, by text or by the time since its last
//! use.

use crate::protocol::SessionInfo;
use crate::text::sessions::{last_used, state_name};

/// One word of the filter.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Word {
    /// `>7d`: not used for more than that many seconds.
    Older(u64),
    /// `<1d`: used within that many seconds.
    Newer(u64),
    /// A part of the name, workspace, model or state, in lower case.
    Text(String),
}

fn parse(word: &str) -> Word {
    let time = |rest: &str| -> Option<u64> {
        let unit = match rest.chars().last()? {
            's' => 1,
            'm' => 60,
            'h' => 3600,
            'd' => 86_400,
            'w' => 7 * 86_400,
            _ => return None,
        };
        let number: u64 = rest.get(..rest.len() - 1)?.parse().ok()?;
        number.checked_mul(unit)
    };
    if let Some(rest) = word.strip_prefix('>')
        && let Some(secs) = time(rest)
    {
        return Word::Older(secs);
    }
    if let Some(rest) = word.strip_prefix('<')
        && let Some(secs) = time(rest)
    {
        return Word::Newer(secs);
    }
    Word::Text(word.to_lowercase())
}

/// A parsed filter text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Filter {
    words: Vec<Word>,
}

impl Filter {
    pub fn new(text: &str) -> Filter {
        Filter {
            words: text.split_whitespace().map(parse).collect(),
        }
    }

    /// Every word matches `session` at `now` (seconds since 1970).
    pub fn matches(&self, session: &SessionInfo, now: u64) -> bool {
        let idle = now.saturating_sub(last_used(session));
        self.words.iter().all(|word| match word {
            Word::Older(secs) => idle > *secs,
            Word::Newer(secs) => idle < *secs,
            Word::Text(text) => [
                session.name.as_str(),
                session.workspace.as_str(),
                session.model.as_str(),
                state_name(session.state),
            ]
            .iter()
            .any(|field| field.to_lowercase().contains(text)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::State;

    const DAY: u64 = 86_400;

    fn session(name: &str, state: State, updated: u64) -> SessionInfo {
        SessionInfo {
            id: 1,
            name: name.to_owned(),
            workspace: "~/ws/Project".to_owned(),
            path: "/home/u/ws/Project".into(),
            model: "anthropic/sonnet".to_owned(),
            state,
            created: 1,
            updated,
            turns: 0,
            queued: 0,
            question: None,
            usage: Default::default(),
        }
    }

    fn shows(text: &str, s: &SessionInfo) -> bool {
        Filter::new(text).matches(s, 100 * DAY)
    }

    #[test]
    fn words() {
        assert_eq!(parse(">7d"), Word::Older(7 * DAY));
        assert_eq!(parse("<12h"), Word::Newer(12 * 3600));
        assert_eq!(parse(">30m"), Word::Older(1800));
        assert_eq!(parse(">2w"), Word::Older(14 * DAY));
        assert_eq!(parse(">7x"), Word::Text(">7x".into()));
        assert_eq!(parse(">d"), Word::Text(">d".into()));
        assert_eq!(parse("Docs"), Word::Text("docs".into()));
        assert_eq!(Filter::new("  "), Filter::default());
    }

    #[test]
    fn text_matches_name_workspace_model_state_ignoring_case() {
        let s = session("Docs-2", State::Saved, 99 * DAY);
        for word in [
            "docs",
            "DOCS-2",
            "project",
            "~/ws",
            "sonnet",
            "anthropic/",
            "saved",
        ] {
            assert!(shows(word, &s), "{word}");
        }
        assert!(!shows("idle", &s));
        assert!(!shows("qwen", &s));
    }

    #[test]
    fn times_count_from_the_last_use() {
        let old = session("old", State::Saved, 90 * DAY);
        let fresh = session("fresh", State::Idle, 100 * DAY - 60);
        assert!(shows(">7d", &old));
        assert!(!shows(">7d", &fresh));
        assert!(shows("<1h", &fresh));
        assert!(!shows("<1h", &old));
        // Without `updated` (an older server): the creation time.
        let unknown = session("unknown", State::Saved, 0);
        assert!(shows(">7d", &unknown));
    }

    #[test]
    fn every_word_must_match() {
        let old = session("old", State::Saved, 90 * DAY);
        assert!(shows("saved >7d", &old));
        assert!(!shows("idle >7d", &old));
        // A bad time word matches as text and hides everything.
        assert!(!shows(">7x", &old));
        assert!(shows("", &old));
    }
}
