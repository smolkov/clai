//! The system prompt (see `spec/agent-loop.md`, "System prompt").

use std::time::{SystemTime, UNIX_EPOCH};

use crate::config::Access;
use crate::permissions::Workspace;

const TEMPLATE: &str = include_str!("system_prompt.md");

/// The system prompt for a workspace on a day (`2026-11-01`).
pub fn system_prompt(workspace: &Workspace, today: &str) -> String {
    let extra: Vec<String> = workspace
        .extra_dirs()
        .iter()
        .map(|dir| {
            let access = match dir.access {
                Access::Read => "read only",
                Access::Write => "read and write",
            };
            format!("{} ({access})", workspace.tilde(&dir.path))
        })
        .collect();
    let extra = if extra.is_empty() {
        "none".to_owned()
    } else {
        extra.join(", ")
    };
    TEMPLATE
        .replace("{workspace}", &workspace.root().display().to_string())
        .replace("{extra_dirs}", &extra)
        .replace("{platform}", std::env::consts::OS)
        .replace("{date}", today)
}

/// Today's date in UTC, `YYYY-MM-DD`.
pub fn today() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    date_of(secs)
}

/// Seconds since 1970 now.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Milliseconds since 1970, for the times of logged events.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// The UTC date of a time in seconds since 1970 (H. Hinnant's
/// `civil_from_days`).
pub fn date_of(secs: u64) -> String {
    let days = i64::try_from(secs / 86_400).unwrap_or(0) + 719_468;
    let era = days.div_euclid(146_097);
    let doe = days.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ExtraDir;

    #[test]
    fn dates() {
        assert_eq!(date_of(0), "1970-01-01");
        assert_eq!(date_of(951_782_400), "2000-02-29");
        assert_eq!(date_of(1_793_491_200), "2026-11-01");
        assert_eq!(date_of(1_793_491_200 + 86_399), "2026-11-01");
    }

    #[test]
    fn prompt_has_the_values() {
        let tmp = tempfile::tempdir().unwrap();
        let docs = tmp.path().join("docs");
        std::fs::create_dir(&docs).unwrap();
        let extra = [ExtraDir {
            path: docs.clone(),
            access: Access::Read,
        }];
        let ws = Workspace::new(tmp.path(), &extra, None).unwrap();
        let prompt = system_prompt(&ws, "2026-11-01");
        assert!(prompt.contains(&format!("Workspace: {}", ws.root().display())));
        assert!(prompt.contains("(read only)"), "{prompt}");
        assert!(prompt.contains("Today: 2026-11-01"));
        assert!(!prompt.contains('{'), "{prompt}");

        let ws = Workspace::new(tmp.path(), &[], None).unwrap();
        assert!(system_prompt(&ws, "x").contains("you may use: none"));
    }
}
