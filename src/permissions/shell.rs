//! A small shell parser for permission checks: splits a command line into
//! the commands it runs (see `spec/config.md`, "Commands with several
//! parts"). It does not run or expand anything. What it cannot read safely
//! is an error, and the call then asks.

/// The commands of a command line, also those inside substitutions.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Script {
    pub parts: Vec<Part>,
    /// Pipelines of two or more parts, as indexes into `parts`, in order.
    pub pipelines: Vec<Vec<usize>>,
}

/// One simple command.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Part {
    /// The command as written (without operators around it).
    pub text: String,
    /// Words with quotes removed, leading assignments included.
    pub words: Vec<Word>,
    /// How many of `words` are leading `NAME=value` assignments.
    pub assignments: usize,
    pub redirects: Vec<Redirect>,
}

impl Part {
    /// The words after the assignments: the command and its arguments.
    pub fn command(&self) -> &[Word] {
        self.words.get(self.assignments..).unwrap_or(&[])
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Word {
    pub text: String,
    /// Contains an expansion (`$X`, `$( )`, backticks), so its value is
    /// only known when the shell runs it.
    pub dynamic: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redirect {
    pub write: bool,
    pub target: Word,
}

/// Parses a command line.
pub fn parse(src: &str) -> Result<Script, String> {
    let mut lexer = Lexer {
        chars: src.char_indices().collect(),
        src,
        pos: 0,
        depth: 0,
        ends: Vec::new(),
    };
    let mut script = Script::default();
    lexer.list(&mut script, None)?;
    Ok(script)
}

/// Splits a pattern or a simple text into words, like the shell, with `|`
/// as its own word. No expansions, no other operators.
pub fn split_words(src: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = src.chars();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' | '\n' => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            '|' => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
                words.push("|".into());
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => cur.push(c),
                        None => return Err("unbalanced quotes".into()),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(c @ ('"' | '\\' | '$' | '`')) => cur.push(c),
                            Some(c) => {
                                cur.push('\\');
                                cur.push(c);
                            }
                            None => return Err("unbalanced quotes".into()),
                        },
                        Some(c) => cur.push(c),
                        None => return Err("unbalanced quotes".into()),
                    }
                }
            }
            '\\' => {
                in_word = true;
                match chars.next() {
                    Some(c) => cur.push(c),
                    None => cur.push('\\'),
                }
            }
            c => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    if in_word {
        words.push(cur);
    }
    Ok(words)
}

/// Quotes a word so that [`split_words`] gives it back unchanged.
pub fn quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| !c.is_whitespace() && !matches!(c, '\'' | '"' | '\\' | '|'));
    if plain {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

/// Words that only structure a command: skipped at the start of a part.
const SKIPPED: [&str; 10] = [
    "!", "{", "}", "do", "then", "else", "elif", "if", "while", "until",
];
/// Words that end a compound command; a part of only these runs nothing.
const CLOSING: [&str; 4] = ["fi", "done", "esac", "}"];
/// Constructs the checker does not follow.
const UNSUPPORTED: [&str; 7] = [
    "for", "case", "select", "function", "coproc", "eval", "trap",
];
const MAX_DEPTH: usize = 20;

struct Lexer<'a> {
    src: &'a str,
    chars: Vec<(usize, char)>,
    pos: usize,
    depth: usize,
    /// Closing characters of the open substitutions, innermost last.
    ends: Vec<char>,
}

/// A part being built.
#[derive(Default)]
struct Building {
    start: Option<usize>,
    end: usize,
    words: Vec<Word>,
    redirects: Vec<Redirect>,
    /// The part started with `(`: a subshell is not allowed after words.
    has_words: bool,
}

impl Lexer<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).map(|&(_, c)| c)
    }

    fn peek_at(&self, n: usize) -> Option<char> {
        self.chars.get(self.pos + n).map(|&(_, c)| c)
    }

    fn offset(&self) -> usize {
        self.chars.get(self.pos).map_or(self.src.len(), |&(i, _)| i)
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek();
        if c.is_some() {
            self.pos += 1;
        }
        c
    }

    /// A list of commands up to `end` (`)` or a backtick) or the end of input.
    fn list(&mut self, script: &mut Script, end: Option<char>) -> Result<(), String> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err("nested too deeply".into());
        }
        if let Some(end) = end {
            self.ends.push(end);
        }
        let mut part = Building::default();
        let mut pipeline: Vec<usize> = Vec::new();
        loop {
            let Some(c) = self.peek() else {
                if end.is_some() {
                    return Err("unbalanced parentheses or backticks".into());
                }
                break;
            };
            if Some(c) == end {
                self.bump();
                break;
            }
            match c {
                ' ' | '\t' => {
                    self.bump();
                }
                '\\' if self.peek_at(1) == Some('\n') => {
                    self.pos += 2;
                }
                '#' => {
                    while self.peek().is_some_and(|c| c != '\n') {
                        self.bump();
                    }
                }
                '\n' | ';' => {
                    self.bump();
                    if c == ';' && self.peek() == Some(';') {
                        return Err("`;;` (case) is not supported".into());
                    }
                    self.finish(script, &mut part, &mut pipeline, false)?;
                }
                '&' if self.peek_at(1) == Some('>') => self.redirect(&mut part, None)?,
                '&' => {
                    self.bump();
                    if self.peek() == Some('&') {
                        self.bump();
                    }
                    self.finish(script, &mut part, &mut pipeline, false)?;
                }
                '|' => {
                    self.bump();
                    let pipe = match self.peek() {
                        Some('|') => {
                            self.bump();
                            false
                        }
                        Some('&') => {
                            self.bump();
                            true
                        }
                        _ => true,
                    };
                    if part.words.is_empty() && part.redirects.is_empty() {
                        return Err("missing command before `|`".into());
                    }
                    self.finish(script, &mut part, &mut pipeline, pipe)?;
                }
                '(' => {
                    if part.has_words {
                        return Err("function definitions are not supported".into());
                    }
                    self.bump();
                    let mut inner = Script::default();
                    self.list(&mut inner, Some(')'))?;
                    append(script, inner);
                }
                ')' => return Err("unbalanced parentheses".into()),
                '<' | '>' if self.peek_at(1) != Some('(') => self.redirect(&mut part, None)?,
                c if c.is_ascii_digit() && self.is_fd_redirect() => {
                    self.bump();
                    self.redirect(&mut part, Some(c))?;
                }
                _ => {
                    let start = self.offset();
                    let word = self.word(script)?;
                    part.start.get_or_insert(start);
                    part.end = self.offset();
                    part.words.push(word);
                    part.has_words = true;
                }
            }
        }
        self.finish(script, &mut part, &mut pipeline, false)?;
        if end.is_some() {
            self.ends.pop();
        }
        self.depth -= 1;
        Ok(())
    }

    /// A digit directly followed by `<` or `>`: a file descriptor.
    fn is_fd_redirect(&self) -> bool {
        let at_word_start = self.pos == 0
            || self
                .chars
                .get(self.pos - 1)
                .is_some_and(|&(_, c)| c.is_whitespace() || ";&|()".contains(c));
        at_word_start && matches!(self.peek_at(1), Some('<' | '>'))
    }

    fn redirect(&mut self, part: &mut Building, fd: Option<char>) -> Result<(), String> {
        let start = self.offset();
        part.start
            .get_or_insert(start.saturating_sub(usize::from(fd.is_some())));
        let first = self.bump();
        let mut write = first != Some('<');
        if first == Some('&') {
            self.bump(); // `>` of `&>`
        }
        match (first, self.peek()) {
            (Some('<'), Some('<')) => {
                self.bump();
                if self.peek() == Some('<') {
                    // Here-string: the word is data, not a file.
                    self.bump();
                    self.skip_blanks();
                    let mut ignored = Script::default();
                    self.word(&mut ignored)?;
                    part.end = self.offset();
                    return Ok(());
                }
                return Err("here documents are not supported".into());
            }
            (Some('<'), Some('>')) => {
                self.bump();
                write = true;
            }
            (Some('>' | '&'), Some('>' | '|')) => {
                self.bump();
            }
            (Some('<' | '>'), Some('&')) => {
                // Duplicating a descriptor: `2>&1`, `>&-`; `>&file` writes.
                self.bump();
                self.skip_blanks();
                let mut ignored = Script::default();
                let target = self.word(&mut ignored)?;
                part.end = self.offset();
                if !target.text.chars().all(|c| c.is_ascii_digit() || c == '-') {
                    part.redirects.push(Redirect { write, target });
                }
                return Ok(());
            }
            _ => {}
        }
        self.skip_blanks();
        if self.peek().is_none_or(|c| "\n;&|()<>".contains(c)) {
            return Err("redirection without a file".into());
        }
        let mut inner = Script::default();
        let target = self.word(&mut inner)?;
        part.end = self.offset();
        part.redirects.push(Redirect { write, target });
        Ok(())
    }

    fn skip_blanks(&mut self) {
        while matches!(self.peek(), Some(' ' | '\t')) {
            self.bump();
        }
    }

    /// One word; commands in its substitutions are added to `script`.
    fn word(&mut self, script: &mut Script) -> Result<Word, String> {
        let mut word = Word::default();
        while let Some(c) = self.peek() {
            match c {
                ' ' | '\t' | '\n' | ';' | '&' | '|' | ')' | '<' | '>' => {
                    // `<(` and `>(` inside a word are process substitutions.
                    if matches!(c, '<' | '>') && self.peek_at(1) == Some('(') {
                        self.pos += 2;
                        let mut inner = Script::default();
                        self.list(&mut inner, Some(')'))?;
                        append(script, inner);
                        word.dynamic = true;
                        continue;
                    }
                    break;
                }
                '(' => {
                    if word.text.is_empty() && !word.dynamic {
                        break;
                    }
                    return Err("function definitions are not supported".into());
                }
                '\'' => {
                    self.bump();
                    loop {
                        match self.bump() {
                            Some('\'') => break,
                            Some(c) => word.text.push(c),
                            None => return Err("unbalanced quotes".into()),
                        }
                    }
                }
                '"' => {
                    self.bump();
                    loop {
                        match self.peek() {
                            Some('"') => {
                                self.bump();
                                break;
                            }
                            Some('\\') => {
                                self.bump();
                                match self.bump() {
                                    Some(c @ ('"' | '\\' | '$' | '`')) => word.text.push(c),
                                    Some('\n') => {}
                                    Some(c) => {
                                        word.text.push('\\');
                                        word.text.push(c);
                                    }
                                    None => return Err("unbalanced quotes".into()),
                                }
                            }
                            Some('$') => self.dollar(script, &mut word)?,
                            Some('`') => self.backtick(script, &mut word)?,
                            Some(c) => {
                                self.bump();
                                word.text.push(c);
                            }
                            None => return Err("unbalanced quotes".into()),
                        }
                    }
                }
                '\\' => {
                    self.bump();
                    match self.bump() {
                        Some('\n') | None => {}
                        Some(c) => word.text.push(c),
                    }
                }
                '$' => self.dollar(script, &mut word)?,
                '`' if self.ends.last() == Some(&'`') => break,
                '`' => self.backtick(script, &mut word)?,
                c => {
                    self.bump();
                    word.text.push(c);
                }
            }
        }
        Ok(word)
    }

    /// `$…` at the current position.
    fn dollar(&mut self, script: &mut Script, word: &mut Word) -> Result<(), String> {
        self.bump();
        match self.peek() {
            Some('(') if self.peek_at(1) == Some('(') => {
                // Arithmetic: no commands, but its value is unknown.
                self.pos += 2;
                self.skip_balanced('(', ')', 2)?;
                word.dynamic = true;
            }
            Some('(') => {
                self.bump();
                let mut inner = Script::default();
                self.list(&mut inner, Some(')'))?;
                append(script, inner);
                word.dynamic = true;
            }
            Some('{') => {
                self.bump();
                self.skip_balanced('{', '}', 1)?;
                word.dynamic = true;
            }
            Some('\'') => {
                // `$'…'`: escapes are not decoded, so the value is unknown.
                self.bump();
                loop {
                    match self.bump() {
                        Some('\\') => {
                            self.bump();
                        }
                        Some('\'') => break,
                        Some(_) => {}
                        None => return Err("unbalanced quotes".into()),
                    }
                }
                word.dynamic = true;
            }
            Some(c) if c.is_ascii_alphanumeric() || "_?@*#!$-".contains(c) => {
                if c.is_ascii_alphabetic() || c == '_' {
                    while self
                        .peek()
                        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
                    {
                        self.bump();
                    }
                } else {
                    self.bump();
                }
                word.dynamic = true;
            }
            _ => word.text.push('$'),
        }
        Ok(())
    }

    fn backtick(&mut self, script: &mut Script, word: &mut Word) -> Result<(), String> {
        self.bump();
        let mut inner = Script::default();
        self.list(&mut inner, Some('`'))?;
        append(script, inner);
        word.dynamic = true;
        Ok(())
    }

    /// Skips to the matching `close`; `open` was already read `level` times.
    fn skip_balanced(&mut self, open: char, close: char, level: usize) -> Result<(), String> {
        let mut level = level;
        while level > 0 {
            match self.bump() {
                Some(c) if c == open => level += 1,
                Some(c) if c == close => level -= 1,
                Some(_) => {}
                None => return Err("unbalanced brackets".into()),
            }
        }
        Ok(())
    }

    /// Ends the current part; `pipe` says it is followed by `|`.
    fn finish(
        &self,
        script: &mut Script,
        part: &mut Building,
        pipeline: &mut Vec<usize>,
        pipe: bool,
    ) -> Result<(), String> {
        let built = std::mem::take(part);
        if let Some(done) = make_part(self.src, built)? {
            pipeline.push(script.parts.len());
            script.parts.push(done);
        }
        if !pipe {
            if pipeline.len() > 1 {
                script.pipelines.push(std::mem::take(pipeline));
            }
            pipeline.clear();
        }
        Ok(())
    }
}

/// Turns the collected words into a part, or `None` if it runs nothing.
fn make_part(src: &str, built: Building) -> Result<Option<Part>, String> {
    let mut words = built.words;
    let skip = words
        .iter()
        .take_while(|w| SKIPPED.contains(&w.text.as_str()))
        .count();
    words.drain(..skip);
    if words.iter().all(|w| CLOSING.contains(&w.text.as_str())) && built.redirects.is_empty() {
        return Ok(None);
    }
    let assignments = words.iter().take_while(|w| is_assignment(&w.text)).count();
    if let Some(first) = words.get(assignments)
        && UNSUPPORTED.contains(&first.text.as_str())
    {
        return Err(format!("`{}` is not supported", first.text));
    }
    if assignments == words.len() && built.redirects.is_empty() {
        // Only assignments: nothing runs (substitutions were added already).
        return Ok(None);
    }
    let text = match built.start {
        Some(start) => src.get(start..built.end).unwrap_or("").trim().to_owned(),
        None => String::new(),
    };
    Ok(Some(Part {
        text,
        words,
        assignments,
        redirects: built.redirects,
    }))
}

fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn append(script: &mut Script, inner: Script) {
    let base = script.parts.len();
    script.parts.extend(inner.parts);
    script.pipelines.extend(
        inner
            .pipelines
            .into_iter()
            .map(|p| p.into_iter().map(|i| i + base).collect()),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The words of each part, `$` marking dynamic words.
    fn parts(src: &str) -> Vec<Vec<String>> {
        let script = parse(src).unwrap_or_else(|e| panic!("{src}: {e}"));
        script
            .parts
            .iter()
            .map(|p| {
                p.words
                    .iter()
                    .map(|w| {
                        if w.dynamic {
                            format!("${}", w.text)
                        } else {
                            w.text.clone()
                        }
                    })
                    .collect()
            })
            .collect()
    }

    fn v(list: &[&[&str]]) -> Vec<Vec<String>> {
        list.iter()
            .map(|p| p.iter().map(|w| (*w).to_owned()).collect())
            .collect()
    }

    #[test]
    fn splits_parts() {
        let cases: &[(&str, &[&[&str]])] = &[
            ("cargo test", &[&["cargo", "test"]]),
            (
                "cargo test && rm -rf /",
                &[&["cargo", "test"], &["rm", "-rf", "/"]],
            ),
            ("a; b || c & d", &[&["a"], &["b"], &["c"], &["d"]]),
            ("a | b |& c", &[&["a"], &["b"], &["c"]]),
            ("a\nb", &[&["a"], &["b"]]),
            ("(cd x && make)", &[&["cd", "x"], &["make"]]),
            ("echo $(rm -rf x)", &[&["rm", "-rf", "x"], &["echo", "$"]]),
            ("echo `id`", &[&["id"], &["echo", "$"]]),
            (
                "diff <(ls a) <(ls b)",
                &[&["ls", "a"], &["ls", "b"], &["diff", "$", "$"]],
            ),
            (
                "git commit -m \"fix parse\"",
                &[&["git", "commit", "-m", "fix parse"]],
            ),
            ("echo 'a b' c\\ d", &[&["echo", "a b", "c d"]]),
            ("echo \"$HOME/x\"", &[&["echo", "$/x"]]),
            ("echo $((1+2))", &[&["echo", "$"]]),
            ("echo ${A:-b}", &[&["echo", "$"]]),
            ("ls # comment; rm x", &[&["ls"]]),
            ("echo a\\\nb", &[&["echo", "ab"]]),
        ];
        for (src, expected) in cases {
            assert_eq!(parts(src), v(expected), "{src}");
        }
    }

    #[test]
    fn keywords_are_skipped() {
        assert_eq!(
            parts("if test -f x; then rm x; else ls; fi"),
            v(&[&["test", "-f", "x"], &["rm", "x"], &["ls"]])
        );
        assert_eq!(
            parts("while true; do sleep 1; done"),
            v(&[&["true"], &["sleep", "1"]])
        );
        assert_eq!(parts("! grep x f"), v(&[&["grep", "x", "f"]]));
        assert_eq!(parts("{ ls; }"), v(&[&["ls"]]));
    }

    #[test]
    fn assignments() {
        let script = parse("RUST_LOG=debug cargo test").unwrap();
        let part = &script.parts[0];
        assert_eq!(part.assignments, 1);
        assert_eq!(part.command()[0].text, "cargo");
        // Only assignments: nothing runs, but substitutions do.
        assert_eq!(parts("X=1"), Vec::<Vec<String>>::new());
        assert_eq!(parts("X=$(rm y)"), v(&[&["rm", "y"]]));
    }

    #[test]
    fn redirections() {
        let script = parse("echo x > ~/.bashrc 2>&1").unwrap();
        let part = &script.parts[0];
        assert_eq!(part.words.len(), 2);
        assert_eq!(
            part.redirects,
            [Redirect {
                write: true,
                target: Word {
                    text: "~/.bashrc".into(),
                    dynamic: false
                }
            }]
        );
        assert_eq!(part.text, "echo x > ~/.bashrc 2>&1");
        let script = parse("sort < in.txt >> out.txt 2> err &> all").unwrap();
        let r = &script.parts[0].redirects;
        assert_eq!(
            r.iter()
                .map(|r| (r.write, r.target.text.as_str()))
                .collect::<Vec<_>>(),
            [
                (false, "in.txt"),
                (true, "out.txt"),
                (true, "err"),
                (true, "all")
            ]
        );
        let script = parse("cat <<< hello").unwrap();
        assert!(script.parts[0].redirects.is_empty());
        let script = parse("> out").unwrap();
        assert!(script.parts[0].words.is_empty());
        assert_eq!(script.parts[0].redirects.len(), 1);
        // `2` is a word here, not a file descriptor.
        assert_eq!(parts("echo a2>x")[0], ["echo", "a2"]);
    }

    #[test]
    fn pipelines() {
        let script = parse("curl -s x | sh; ls | wc -l | cat").unwrap();
        assert_eq!(script.pipelines, [vec![0, 1], vec![2, 3, 4]]);
    }

    #[test]
    fn unsupported() {
        for src in [
            "cat <<EOF\nx\nEOF",
            "for f in *; do rm $f; done",
            "case x in a) ls;; esac",
            "f() { rm x; }",
            "eval \"$X\"",
            "echo 'x",
            "echo \"x",
            "echo $(ls",
            "ls )",
            "| ls",
            "ls >",
        ] {
            assert!(parse(src).is_err(), "{src}");
        }
    }

    #[test]
    fn part_text() {
        let script = parse("  cargo build &&   ./deploy.sh staging ; ").unwrap();
        let texts: Vec<_> = script.parts.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(texts, ["cargo build", "./deploy.sh staging"]);
    }

    #[test]
    fn split_and_quote() {
        assert_eq!(
            split_words("git ** --force").unwrap(),
            ["git", "**", "--force"]
        );
        assert_eq!(
            split_words("curl ** | sh").unwrap(),
            ["curl", "**", "|", "sh"]
        );
        assert_eq!(split_words("a 'b c' \"d\"").unwrap(), ["a", "b c", "d"]);
        assert!(split_words("a 'b").is_err());
        for w in ["plain", "a b", "it's", "x|y", ""] {
            assert_eq!(split_words(&quote(w)).unwrap(), [w], "{w}");
        }
    }
}
