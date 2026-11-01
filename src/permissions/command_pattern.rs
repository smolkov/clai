//! Command patterns of `run_command` (see `spec/config.md`, "Command
//! patterns"): words matched from the start of a command.

use std::fmt;

use globset::{GlobBuilder, GlobMatcher};

use super::shell::{self, Part};

/// Commands that run the command given in their arguments.
const WRAPPERS: [&str; 15] = [
    "sudo", "doas", "env", "command", "builtin", "exec", "nice", "nohup", "time", "timeout",
    "xargs", "watch", "setsid", "stdbuf", "ionice",
];

#[derive(Debug, Clone)]
enum WordPattern {
    /// `**`: any number of words.
    Any,
    Glob(GlobMatcher),
}

#[derive(Debug, Clone)]
pub struct CommandPattern {
    text: String,
    /// One entry per stage; more than one for `a | b` patterns.
    stages: Vec<Vec<WordPattern>>,
}

impl CommandPattern {
    pub fn parse(text: &str) -> Result<CommandPattern, String> {
        let words = shell::split_words(text).map_err(|e| format!("pattern `{text}`: {e}"))?;
        if words.is_empty() {
            return Err("empty pattern".into());
        }
        let mut stages = vec![Vec::new()];
        for word in &words {
            if word == "|" {
                stages.push(Vec::new());
                continue;
            }
            let pattern = if word == "**" {
                WordPattern::Any
            } else {
                let glob = GlobBuilder::new(word)
                    .literal_separator(false)
                    .backslash_escape(true)
                    .build()
                    .map_err(|e| format!("pattern `{text}`: {}", e.kind()))?;
                WordPattern::Glob(glob.compile_matcher())
            };
            if let Some(stage) = stages.last_mut() {
                stage.push(pattern);
            }
        }
        if stages.iter().any(Vec::is_empty) {
            return Err(format!("pattern `{text}`: a command is missing around `|`"));
        }
        Ok(CommandPattern {
            text: text.to_owned(),
            stages,
        })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// `a | b`: matched against pipelines, not single commands.
    pub fn is_pipeline(&self) -> bool {
        self.stages.len() > 1
    }

    /// Matches one command. `strict` is for `deny` and `ask` rules: they
    /// also match by the file name of the command, after assignments, and
    /// inside wrappers such as `sudo`. `allow` rules match as written.
    pub fn matches(&self, part: &Part) -> bool {
        match self.stages.as_slice() {
            [stage] => stage_matches(stage, part, false),
            _ => false,
        }
    }

    pub fn matches_strict(&self, part: &Part) -> bool {
        match self.stages.as_slice() {
            [stage] => stage_matches(stage, part, true),
            _ => false,
        }
    }

    /// Matches a pipeline: consecutive stages match the pattern's stages.
    pub fn matches_pipeline(&self, parts: &[&Part]) -> bool {
        let n = self.stages.len();
        if n < 2 || parts.len() < n {
            return false;
        }
        parts.windows(n).any(|window| {
            self.stages
                .iter()
                .zip(window)
                .all(|(stage, part)| stage_matches(stage, part, true))
        })
    }
}

impl fmt::Display for CommandPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

fn stage_matches(stage: &[WordPattern], part: &Part, strict: bool) -> bool {
    let all: Vec<&str> = part.words.iter().map(|w| w.text.as_str()).collect();
    if !strict {
        return words_match(stage, &all, false);
    }
    let command = all.get(part.assignments..).unwrap_or(&[]);
    if words_match(stage, &all, true) || words_match(stage, command, true) {
        return true;
    }
    // Look through wrappers: any later word may be the wrapped command.
    let is_wrapper = command
        .first()
        .is_some_and(|w| WRAPPERS.contains(&file_name(w)));
    is_wrapper
        && (1..command.len()).any(|i| words_match(stage, command.get(i..).unwrap_or(&[]), true))
}

/// Pattern words against the first words; further words are allowed.
fn words_match(pattern: &[WordPattern], words: &[&str], by_name: bool) -> bool {
    let Some((first, rest)) = pattern.split_first() else {
        return true;
    };
    match first {
        WordPattern::Any => {
            (0..=words.len()).any(|k| words_match(rest, words.get(k..).unwrap_or(&[]), false))
        }
        WordPattern::Glob(glob) => match words.split_first() {
            Some((word, others)) => {
                let hit = glob.is_match(word)
                    || (by_name
                        && !glob.glob().glob().contains('/')
                        && glob.is_match(file_name(word)));
                hit && words_match(rest, others, false)
            }
            None => false,
        },
    }
}

/// `rm` for `/bin/rm`.
pub fn file_name(word: &str) -> &str {
    word.rsplit('/').next().unwrap_or(word)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(src: &str) -> Part {
        let script = shell::parse(src).unwrap();
        script.parts.into_iter().next().unwrap()
    }

    fn pat(text: &str) -> CommandPattern {
        CommandPattern::parse(text).unwrap()
    }

    /// (pattern, command, allow matches, deny/ask matches)
    const CASES: &[(&str, &str, bool, bool)] = &[
        ("ls", "ls", true, true),
        ("ls", "ls -la src", true, true),
        ("ls", "lsof", false, false),
        ("ls", "sudo ls", false, true),
        ("ls*", "lsof -i", true, true),
        ("ls*", "lsblk", true, true),
        ("ls*", "cat ls", false, false),
        ("rm", "rm -rf /", true, true),
        ("rm", "/bin/rm x", false, true),
        ("rm", "./rm x", false, true),
        ("rm", "rmdir x", false, false),
        ("rm", "sudo rm x", false, true),
        ("rm", "sudo -u root rm x", false, true),
        ("rm", "xargs rm", false, true),
        ("rm", "env FOO=1 rm x", false, true),
        ("rm", "FOO=1 rm x", false, true),
        ("rm", "timeout 10 rm x", false, true),
        ("git push", "git push", true, true),
        ("git push", "git push -f origin", true, true),
        ("git push", "git pull", false, false),
        ("git push", "git status", false, false),
        ("git ** --force", "git push --force", true, true),
        ("git ** --force", "git push origin main --force", true, true),
        ("git ** --force", "git push -f", false, false),
        ("cargo {build,test}", "cargo build --release", true, true),
        ("cargo {build,test}", "cargo test parse", true, true),
        ("cargo {build,test}", "cargo run", false, false),
        ("*", "make", true, true),
        (
            "RUST_LOG=* cargo test",
            "RUST_LOG=debug cargo test",
            true,
            true,
        ),
        ("cargo test", "RUST_LOG=debug cargo test", false, true),
        ("/usr/bin/rm", "rm x", false, false),
        ("/usr/bin/rm", "/usr/bin/rm x", true, true),
        (
            "git commit -m 'fix parse'",
            "git commit -m \"fix parse\"",
            true,
            true,
        ),
        ("find ** -delete", "find . -name x -delete", true, true),
        ("[*]", "'*' x", true, true),
        ("[*]", "ls", false, false),
    ];

    #[test]
    fn matching_table() {
        for &(pattern, command, allow, strict) in CASES {
            let p = pat(pattern);
            let part = part(command);
            assert_eq!(p.matches(&part), allow, "allow: {pattern} vs {command}");
            assert_eq!(
                p.matches_strict(&part),
                strict,
                "strict: {pattern} vs {command}"
            );
        }
    }

    #[test]
    fn pipelines() {
        let script = shell::parse("curl -s https://x | sh").unwrap();
        let parts: Vec<&Part> = script.parts.iter().collect();
        assert!(pat("curl ** | sh").matches_pipeline(&parts));
        assert!(!pat("curl ** | bash").matches_pipeline(&parts));
        assert!(!pat("curl ** | sh").matches(parts[0]));
        let script = shell::parse("cat x | curl -d @- y | sudo sh").unwrap();
        let parts: Vec<&Part> = script.parts.iter().collect();
        assert!(pat("curl ** | sh").matches_pipeline(&parts));
    }

    #[test]
    fn errors() {
        assert!(CommandPattern::parse("").is_err());
        assert!(CommandPattern::parse("  ").is_err());
        assert!(
            CommandPattern::parse("git 'x")
                .unwrap_err()
                .contains("unbalanced")
        );
        assert!(CommandPattern::parse("| sh").is_err());
        assert!(CommandPattern::parse("a[").is_err());
    }
}
