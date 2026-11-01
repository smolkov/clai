//! Path patterns of the file tools (see `spec/config.md`, "Path patterns").

use std::fmt;
use std::path::{Component, Path};

use globset::{GlobBuilder, GlobMatcher};

/// A path to check, in the forms the patterns need.
#[derive(Debug, Clone, Copy)]
pub struct PathInfo<'a> {
    /// Resolved absolute path.
    pub abs: &'a Path,
    /// Relative to the workspace, if inside it (`""` for the workspace).
    pub rel: Option<&'a Path>,
    /// Relative to the directory it lies in: the workspace or an extra dir.
    pub root_rel: &'a Path,
}

#[derive(Debug, Clone)]
enum Kind {
    /// No `/`: a file or directory name in any directory.
    Name,
    /// Relative to the workspace.
    Relative,
    /// `/…` or `~/…`.
    Absolute,
}

#[derive(Debug, Clone)]
pub struct PathPattern {
    text: String,
    kind: Kind,
    matcher: GlobMatcher,
    /// For `DIR/**`: matches `DIR` itself.
    dir_itself: Option<GlobMatcher>,
}

impl PathPattern {
    /// Parses a pattern; `home` expands `~/`.
    pub fn parse(text: &str, home: Option<&Path>) -> Result<PathPattern, String> {
        if text.is_empty() {
            return Err("empty pattern".into());
        }
        if text.len() > 1 && text.ends_with('/') {
            return Err(format!(
                "pattern `{text}` ends in `/`; use `{}/**` for a directory",
                text.trim_end_matches('/')
            ));
        }
        let (kind, glob) = if text == "~" || text.starts_with("~/") {
            let home = home.ok_or_else(|| format!("pattern `{text}`: HOME is not set"))?;
            let rest = text.get(1..).unwrap_or("").trim_start_matches('/');
            let full = home.join(rest);
            (Kind::Absolute, full.to_string_lossy().into_owned())
        } else if text.starts_with('/') {
            (Kind::Absolute, text.to_owned())
        } else if text.contains('/') {
            let rel = text.trim_start_matches("./");
            (Kind::Relative, rel.to_owned())
        } else {
            (Kind::Name, text.to_owned())
        };
        let matcher = compile(&glob, text)?;
        let dir_itself = match glob.strip_suffix("/**") {
            Some(dir) if !dir.is_empty() => Some(compile(dir, text)?),
            _ => None,
        };
        Ok(PathPattern {
            text: text.to_owned(),
            kind,
            matcher,
            dir_itself,
        })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// True if the pattern matches the path or a directory it lies in.
    pub fn matches(&self, path: PathInfo<'_>) -> bool {
        match self.kind {
            Kind::Name => path.root_rel.components().any(|c| match c {
                Component::Normal(name) => self.matcher.is_match(Path::new(name)),
                _ => false,
            }),
            Kind::Relative => match path.rel {
                Some(rel) if rel.as_os_str().is_empty() => self.matches_root(),
                Some(rel) => self.matches_with_ancestors(rel),
                None => false,
            },
            Kind::Absolute => self.matches_with_ancestors(path.abs),
        }
    }

    fn matches_with_ancestors(&self, path: &Path) -> bool {
        if self.dir_itself.as_ref().is_some_and(|m| m.is_match(path)) {
            return true;
        }
        path.ancestors()
            .filter(|p| !p.as_os_str().is_empty())
            .any(|p| self.matcher.is_match(p))
    }

    /// The workspace itself: only `**` (written `./**`) matches it.
    fn matches_root(&self) -> bool {
        self.text.trim_start_matches("./") == "**"
    }
}

impl fmt::Display for PathPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

fn compile(glob: &str, text: &str) -> Result<GlobMatcher, String> {
    GlobBuilder::new(glob)
        .literal_separator(true)
        .backslash_escape(true)
        .build()
        .map(|g| g.compile_matcher())
        .map_err(|e| format!("pattern `{text}`: {}", e.kind()))
}

/// A path written as a pattern that matches only itself: glob characters are
/// put in brackets, and a path in the workspace root gets `./`.
pub fn literal(path: &Path, relative: bool) -> String {
    let mut out = String::new();
    for c in path.to_string_lossy().chars() {
        match c {
            '*' | '?' | '[' | ']' | '{' | '}' => {
                out.push('[');
                out.push(c);
                out.push(']');
            }
            '\\' => out.push_str("[\\\\]"),
            _ => out.push(c),
        }
    }
    if relative && !out.contains('/') {
        out.insert_str(0, "./");
    }
    out
}

/// `DIR/**` for a directory, relative or absolute; `./**` for the workspace.
pub fn below(dir: &Path, relative: bool) -> String {
    if relative && dir.as_os_str().is_empty() {
        return "./**".into();
    }
    format!("{}/**", literal(dir, false))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info<'a>(abs: &'a str, rel: Option<&'a str>, root_rel: &'a str) -> PathInfo<'a> {
        PathInfo {
            abs: Path::new(abs),
            rel: rel.map(Path::new),
            root_rel: Path::new(root_rel),
        }
    }

    fn ws(rel: &'static str) -> PathInfo<'static> {
        let abs: &'static str = Box::leak(format!("/ws/{rel}").into_boxed_str());
        info(abs, Some(rel), rel)
    }

    fn p(text: &str) -> PathPattern {
        PathPattern::parse(text, Some(Path::new("/home/u"))).unwrap()
    }

    #[test]
    fn name_patterns_match_in_any_directory() {
        assert!(p(".env*").matches(ws(".env")));
        assert!(p(".env*").matches(ws("config/.env.local")));
        assert!(p("*.pem").matches(ws("a/b/key.pem")));
        assert!(!p("*.pem").matches(ws("a/b/key.pem.txt")));
        // A matching directory covers its content.
        assert!(p("secrets").matches(ws("secrets/a/b.txt")));
        assert!(!p(".env").matches(ws("env/x")));
    }

    #[test]
    fn relative_patterns_are_anchored() {
        assert!(p("src/**").matches(ws("src/a/b.rs")));
        assert!(p("src/**").matches(ws("src")));
        assert!(!p("src/**").matches(ws("x/src/a.rs")));
        assert!(p("docs/*.md").matches(ws("docs/a.md")));
        assert!(!p("docs/*.md").matches(ws("docs/x/a.md")));
        assert!(p("./Cargo.toml").matches(ws("Cargo.toml")));
        assert!(!p("./Cargo.toml").matches(ws("sub/Cargo.toml")));
        assert!(p("docs/private").matches(ws("docs/private/x.md")));
        // Never outside the workspace.
        assert!(!p("src/**").matches(info("/lib/src/a.rs", None, "src/a.rs")));
    }

    #[test]
    fn workspace_itself() {
        assert!(p("./**").matches(ws("")));
        assert!(p("./**").matches(ws("a/b")));
        assert!(!p("src/**").matches(ws("")));
    }

    #[test]
    fn absolute_and_home_patterns() {
        let ssh = info("/home/u/.ssh/id_ed25519", None, "id_ed25519");
        assert!(p("~/.ssh/**").matches(ssh));
        assert!(p("/home/u/.ssh/*").matches(ssh));
        assert!(!p("~/.ssh/**").matches(info("/home/u/ws/a", None, "a")));
    }

    #[test]
    fn errors() {
        assert!(PathPattern::parse("", None).is_err());
        let e = PathPattern::parse("src/", None).unwrap_err();
        assert!(e.contains("use `src/**`"), "{e}");
        assert!(PathPattern::parse("a[", None).is_err());
        assert!(
            PathPattern::parse("~/x", None)
                .unwrap_err()
                .contains("HOME")
        );
    }

    #[test]
    fn literals_match_only_themselves() {
        let text = literal(Path::new("src/a*b.rs"), true);
        assert_eq!(text, "src/a[*]b.rs");
        assert!(p(&text).matches(ws("src/a*b.rs")));
        assert!(!p(&text).matches(ws("src/axb.rs")));
        assert_eq!(literal(Path::new("Cargo.toml"), true), "./Cargo.toml");
        assert_eq!(below(Path::new("src"), true), "src/**");
        assert_eq!(below(Path::new("src/a"), true), "src/a/**");
        assert_eq!(below(Path::new(""), true), "./**");
        assert_eq!(below(Path::new("/opt/lib"), false), "/opt/lib/**");
        assert!(p(&below(Path::new("src"), true)).matches(ws("src/x.rs")));
    }
}
