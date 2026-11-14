//! `SKILL.md`: the frontmatter and the body, and their checks (see
//! `spec/skills.md`, "Format").
//!
//! The frontmatter is read line by line: `key: value` at the start of a
//! line, with plain, quoted and block (`>` / `|`) values. Only `name` and
//! `description` are kept; other keys, nested tables and lists are passed
//! over. No YAML crate: the two keys need no more.

/// Largest `SKILL.md`; a larger file is skipped.
pub const MAX_FILE: usize = 64 * 1024;
/// Longest description, in characters.
pub const MAX_DESCRIPTION: usize = 1024;
/// Longest name, in characters.
pub const MAX_NAME: usize = 64;

/// A parsed `SKILL.md`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// `name` of the frontmatter, if given; not checked yet.
    pub name: Option<String>,
    /// One line, whitespace runs made one space.
    pub description: String,
    /// The instructions: everything after the frontmatter.
    pub body: String,
}

/// Why a file is not a skill: the message, and the line it is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub line: Option<usize>,
    pub message: String,
}

impl Problem {
    fn at(line: usize, message: impl Into<String>) -> Problem {
        Problem {
            line: Some(line),
            message: message.into(),
        }
    }

    fn file(message: impl Into<String>) -> Problem {
        Problem {
            line: None,
            message: message.into(),
        }
    }
}

/// Parses the bytes of a `SKILL.md`.
pub fn parse(bytes: &[u8]) -> Result<Parsed, Problem> {
    if bytes.len() > MAX_FILE {
        return Err(Problem::file(format!(
            "larger than {} KiB",
            MAX_FILE / 1024
        )));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Problem::file("not UTF-8"))?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let text = text.replace("\r\n", "\n");
    let lines: Vec<&str> = text.split('\n').collect();
    if lines.first().map(|l| l.trim_end()) != Some("---") {
        return Err(Problem::at(
            1,
            "no frontmatter: the file must start with ---",
        ));
    }
    let Some(end) = lines
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, l)| l.trim_end() == "---")
        .map(|(i, _)| i)
    else {
        return Err(Problem::at(1, "the frontmatter has no closing --- line"));
    };
    let front = lines.get(1..end).unwrap_or(&[]);
    let keys = keys(front)?;
    let mut name = None;
    let mut description = None;
    for (key, value) in keys {
        match key.as_str() {
            "name" => name = Some(value.trim().to_owned()).filter(|n| !n.is_empty()),
            "description" => description = Some(value),
            _ => {}
        }
    }
    let description = description
        .map(|d| d.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|d| !d.is_empty())
        .ok_or_else(|| Problem::file("no description"))?;
    if description.chars().count() > MAX_DESCRIPTION {
        return Err(Problem::file(format!(
            "the description is longer than {MAX_DESCRIPTION} characters"
        )));
    }
    let body = lines.get(end + 1..).unwrap_or(&[]).join("\n");
    let body = body.trim_start_matches('\n').trim_end().to_owned();
    Ok(Parsed {
        name,
        description,
        body,
    })
}

/// Whether `name` follows the rule of the common format: 1 to 64
/// lowercase letters, digits and `-`, not starting or ending with `-`.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_NAME
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !name.starts_with('-')
        && !name.ends_with('-')
}

/// The top-level keys of the frontmatter with their values, in order.
/// `front` holds the lines between the two `---`; line numbers count the
/// first `---` as 1.
fn keys(front: &[&str]) -> Result<Vec<(String, String)>, Problem> {
    let mut keys = Vec::new();
    let mut i = 0;
    while let Some(line) = front.get(i) {
        let number = i + 2;
        i += 1;
        if is_blank_or_comment(line) || line.starts_with([' ', '\t']) || line.starts_with("- ") {
            // Nested values and list items belong to a key that is not read.
            continue;
        }
        let Some((key, rest)) = line.split_once(':') else {
            return Err(Problem::at(number, "expected `key: value`"));
        };
        let key = key.trim().to_owned();
        let rest = rest.trim();
        // The indented lines that follow belong to this key.
        let start = i;
        while front
            .get(i)
            .is_some_and(|l| l.starts_with([' ', '\t']) || l.trim().is_empty())
        {
            i += 1;
        }
        let more = front.get(start..i).unwrap_or(&[]);
        let value = value(rest, more, number)?;
        keys.push((key, value));
    }
    Ok(keys)
}

fn is_blank_or_comment(line: &str) -> bool {
    let t = line.trim();
    t.is_empty() || t.starts_with('#')
}

/// The value after `key:`, with the indented lines after it.
fn value(rest: &str, more: &[&str], line: usize) -> Result<String, Problem> {
    if let Some(indicator) = rest.strip_prefix(['>', '|']) {
        let folded = rest.starts_with('>');
        let indicator = strip_comment(indicator).trim();
        if !indicator
            .chars()
            .all(|c| c == '-' || c == '+' || c.is_ascii_digit())
        {
            return Err(Problem::at(line, format!("unknown block `{rest}`")));
        }
        return Ok(block(more, folded));
    }
    if let Some(inner) = rest.strip_prefix('"') {
        return double_quoted(inner).ok_or_else(|| Problem::at(line, "unterminated \"string\""));
    }
    if let Some(inner) = rest.strip_prefix('\'') {
        return single_quoted(inner).ok_or_else(|| Problem::at(line, "unterminated 'string'"));
    }
    // A plain value goes on over indented lines, joined with spaces.
    let mut parts = vec![strip_comment(rest).trim().to_owned()];
    parts.extend(
        more.iter()
            .map(|l| strip_comment(l).trim().to_owned())
            .filter(|l| !l.is_empty()),
    );
    Ok(parts
        .into_iter()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join(" "))
}

/// A plain value ends before ` #`.
fn strip_comment(text: &str) -> &str {
    if text.trim_start().starts_with('#') {
        return "";
    }
    match text.find(" #") {
        Some(at) => text.get(..at).unwrap_or(text),
        None => text,
    }
}

/// The lines of a `>` or `|` block, without their common indentation.
fn block(lines: &[&str], folded: bool) -> String {
    let indent = lines
        .iter()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.len() - l.trim_start().len())
        .min()
        .unwrap_or(0);
    let lines: Vec<&str> = lines
        .iter()
        .map(|l| l.get(indent..).unwrap_or("").trim_end())
        .collect();
    let text = if folded {
        let mut out = String::new();
        for line in &lines {
            if line.is_empty() {
                out.push('\n');
            } else {
                if !out.is_empty() && !out.ends_with('\n') {
                    out.push(' ');
                }
                out.push_str(line);
            }
        }
        out
    } else {
        lines.join("\n")
    };
    text.trim().to_owned()
}

fn double_quoted(inner: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
    None
}

fn single_quoted(inner: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\'' {
            if chars.peek() == Some(&'\'') {
                chars.next();
                out.push('\'');
            } else {
                return Some(out);
            }
        } else {
            out.push(c);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(text: &str) -> Parsed {
        parse(text.as_bytes()).unwrap_or_else(|e| panic!("{e:?}"))
    }

    fn err(text: &str) -> Problem {
        parse(text.as_bytes()).unwrap_err()
    }

    #[test]
    fn plain_values_and_body() {
        let p = ok(
            "---\nname: release-check\ndescription: Steps before a release. # note\n---\n\n# Release\n\n1. Test\n\n",
        );
        assert_eq!(p.name.as_deref(), Some("release-check"));
        assert_eq!(p.description, "Steps before a release.");
        assert_eq!(p.body, "# Release\n\n1. Test");
    }

    #[test]
    fn quoted_values() {
        let p =
            ok("---\nname: 'pdf'\ndescription: \"Read \\\"PDF\\\" files: fill, merge\"\n---\nbody");
        assert_eq!(p.name.as_deref(), Some("pdf"));
        assert_eq!(p.description, "Read \"PDF\" files: fill, merge");
        let p = ok("---\ndescription: 'It''s here'\n---\n");
        assert_eq!(p.description, "It's here");
        assert_eq!(p.body, "");
        assert_eq!(err("---\ndescription: 'open\n---\n").line, Some(2));
    }

    #[test]
    fn blocks_become_one_line() {
        let p = ok(
            "---\ndescription: >\n  Steps before\n  a release.\n\n  Use it often.\nname: x\n---\n",
        );
        assert_eq!(p.description, "Steps before a release. Use it often.");
        let p = ok("---\ndescription: |-\n  one\n  two\n---\n");
        assert_eq!(p.description, "one two");
    }

    #[test]
    fn plain_values_go_on_over_indented_lines() {
        let p = ok("---\ndescription: Steps before\n  a release.\n---\n");
        assert_eq!(p.description, "Steps before a release.");
    }

    #[test]
    fn other_keys_lists_and_tables_are_passed_over() {
        let p = ok(
            "---\nname: a\nallowed-tools:\n- Read\n- Bash\nmetadata:\n  version: 1\n  tags: [x]\nlicense: MIT\ndescription: D\n---\n",
        );
        assert_eq!(p.description, "D");
    }

    #[test]
    fn crlf_and_bom() {
        let p = ok("\u{feff}---\r\nname: a\r\ndescription: D\r\n---\r\nline 1\r\nline 2\r\n");
        assert_eq!(p.description, "D");
        assert_eq!(p.body, "line 1\nline 2");
    }

    #[test]
    fn problems() {
        assert_eq!(err("# no frontmatter\n").line, Some(1));
        assert!(
            err("---\nname: a\ndescription: D\n")
                .message
                .contains("closing ---")
        );
        assert_eq!(err("---\nname: a\n---\n").message, "no description");
        assert_eq!(
            err("---\ndescription: \"\"\n---\n").message,
            "no description"
        );
        let bad = err("---\nname: a\njust text\n---\n");
        assert_eq!(bad.line, Some(3));
        let long = format!("---\ndescription: {}\n---\n", "x".repeat(1025));
        assert!(err(&long).message.contains("1024"));
        assert!(parse(format!("---\ndescription: {}\n---\n", "x".repeat(1024)).as_bytes()).is_ok());
        let big = format!("---\ndescription: d\n---\n{}", "x".repeat(MAX_FILE));
        assert!(err(&big).message.contains("64 KiB"));
        assert_eq!(
            parse(b"---\ndescription: \xff\n---\n").unwrap_err().message,
            "not UTF-8"
        );
    }

    #[test]
    fn names() {
        for good in ["a", "release-check", "pdf2", &"x".repeat(64)] {
            assert!(valid_name(good), "{good}");
        }
        for bad in ["", "-a", "a-", "Release", "a_b", "a b", &"x".repeat(65)] {
            assert!(!valid_name(bad), "{bad}");
        }
    }
}
