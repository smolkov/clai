//! The workspace and its extra directories, and resolving the paths of tool
//! calls (see `spec/tools.md`, "Paths").

use std::io;
use std::path::{Component, Path, PathBuf};

use crate::config::{Access, ExtraDir};

use super::PathInfo;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Workspace {
    /// Resolved (canonical) workspace directory.
    root: PathBuf,
    /// Resolved extra directories that exist.
    extra: Vec<ExtraDir>,
    home: Option<PathBuf>,
}

/// Where a resolved path lies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Area {
    /// In the workspace; the path relative to it (`""` for the workspace).
    Workspace(PathBuf),
    /// In an extra directory: its resolved path, access, and the path below.
    Extra {
        dir: PathBuf,
        access: Access,
        rel: PathBuf,
    },
    Outside,
}

/// A path of a tool call after resolving `.`, `..` and symbolic links.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub abs: PathBuf,
    pub area: Area,
}

impl Resolved {
    /// Relative to the workspace when inside it, otherwise absolute
    /// (see `spec/tools.md`, "Paths").
    pub fn display(&self) -> String {
        match &self.area {
            Area::Workspace(rel) if rel.as_os_str().is_empty() => ".".into(),
            Area::Workspace(rel) => rel.to_string_lossy().into_owned(),
            _ => self.abs.to_string_lossy().into_owned(),
        }
    }

    pub fn info(&self) -> PathInfo<'_> {
        let (rel, root_rel) = match &self.area {
            Area::Workspace(rel) => (Some(rel.as_path()), rel.as_path()),
            Area::Extra { rel, .. } => (None, rel.as_path()),
            Area::Outside => (None, self.abs.as_path()),
        };
        PathInfo {
            abs: &self.abs,
            rel,
            root_rel,
        }
    }

    pub fn is_outside(&self) -> bool {
        self.area == Area::Outside
    }

    /// The extra directory this path lies in, if it is read-only.
    pub fn read_only_dir(&self) -> Option<&Path> {
        match &self.area {
            Area::Extra {
                dir,
                access: Access::Read,
                ..
            } => Some(dir),
            _ => None,
        }
    }
}

impl Workspace {
    /// Resolves the workspace and the extra directories. Extra directories
    /// that do not exist are skipped with a warning in the log.
    pub fn new(root: &Path, extra: &[ExtraDir], home: Option<PathBuf>) -> io::Result<Workspace> {
        let root = root.canonicalize()?;
        let mut dirs = Vec::new();
        for dir in extra {
            match dir.path.canonicalize() {
                Ok(path) if path.is_dir() => dirs.push(ExtraDir {
                    path,
                    access: dir.access,
                }),
                Ok(path) => {
                    tracing::warn!("extra dir {} is not a directory; skipped", path.display())
                }
                Err(e) => tracing::warn!("extra dir {}: {e}; skipped", dir.path.display()),
            }
        }
        Ok(Workspace {
            root,
            extra: dirs,
            home,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The extra directories that exist, resolved.
    pub fn extra_dirs(&self) -> &[ExtraDir] {
        &self.extra
    }

    pub fn home(&self) -> Option<&Path> {
        self.home.as_deref()
    }

    /// Resolves a path argument: relative to the workspace, or absolute.
    /// `~` is not expanded. A path that does not exist is resolved through
    /// its nearest existing parent.
    pub fn resolve(&self, path: &str) -> Resolved {
        self.resolve_in(&self.root, path)
    }

    /// Like [`Workspace::resolve`], relative to `dir`.
    pub fn resolve_in(&self, dir: &Path, path: &str) -> Resolved {
        let joined = if path.is_empty() {
            dir.to_path_buf()
        } else {
            dir.join(path)
        };
        let abs = resolve_path(&joined);
        let area = self.area_of(&abs);
        Resolved { abs, area }
    }

    /// A path that is already resolved, e.g. an entry found while walking
    /// a resolved directory (links are not followed).
    pub fn locate(&self, abs: &Path) -> Resolved {
        Resolved {
            abs: abs.to_path_buf(),
            area: self.area_of(abs),
        }
    }

    fn area_of(&self, abs: &Path) -> Area {
        if let Ok(rel) = abs.strip_prefix(&self.root) {
            return Area::Workspace(rel.to_path_buf());
        }
        for dir in &self.extra {
            if let Ok(rel) = abs.strip_prefix(&dir.path) {
                return Area::Extra {
                    dir: dir.path.clone(),
                    access: dir.access,
                    rel: rel.to_path_buf(),
                };
            }
        }
        Area::Outside
    }

    /// `~/x` for paths in the home directory, for messages.
    pub fn tilde(&self, path: &Path) -> String {
        match self.home.as_deref().and_then(|h| path.strip_prefix(h).ok()) {
            Some(rest) if rest.as_os_str().is_empty() => "~".into(),
            Some(rest) => format!("~/{}", rest.display()),
            None => path.display().to_string(),
        }
    }
}

/// Resolves an absolute path component by component, like the system
/// does: existing parts are canonicalized (symbolic links followed), `.`
/// and `..` are removed, and parts that do not exist yet are added as they
/// are. A link that points to a missing file is followed to where it points,
/// because writing through it would create that file.
fn resolve_path(path: &Path) -> PathBuf {
    resolve_depth(path, 0)
}

/// Links followed before giving up, like the system's limit.
const MAX_LINKS: usize = 40;

fn resolve_depth(path: &Path, depth: usize) -> PathBuf {
    let mut out = PathBuf::from("/");
    let mut exists = true;
    for component in path.components() {
        match component {
            Component::RootDir | Component::Prefix(_) | Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::Normal(name) => {
                let next = out.join(name);
                if !exists {
                    out = next;
                } else if let Ok(canonical) = next.canonicalize() {
                    out = canonical;
                } else if let Ok(target) = std::fs::read_link(&next) {
                    if depth >= MAX_LINKS {
                        // A loop: say "outside" by pointing at the link's target.
                        return lexical(&out.join(target));
                    }
                    out = resolve_depth(&out.join(target), depth + 1);
                } else {
                    exists = false;
                    out = next;
                }
            }
        }
    }
    out
}

fn lexical(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::symlink;

    fn setup() -> (tempfile::TempDir, Workspace) {
        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("ws");
        fs::create_dir_all(ws.join("src")).unwrap();
        fs::write(ws.join("src/a.rs"), "").unwrap();
        fs::create_dir_all(tmp.path().join("lib")).unwrap();
        fs::create_dir_all(tmp.path().join("docs")).unwrap();
        let extra = [
            ExtraDir {
                path: tmp.path().join("lib"),
                access: Access::Write,
            },
            ExtraDir {
                path: tmp.path().join("docs"),
                access: Access::Read,
            },
            ExtraDir {
                path: tmp.path().join("missing"),
                access: Access::Write,
            },
        ];
        let workspace = Workspace::new(&ws, &extra, None).unwrap();
        (tmp, workspace)
    }

    #[test]
    fn inside_and_outside() {
        let (tmp, ws) = setup();
        assert_eq!(ws.resolve("src/a.rs").display(), "src/a.rs");
        assert_eq!(ws.resolve("").display(), ".");
        assert_eq!(ws.resolve("./src/../src/a.rs").display(), "src/a.rs");
        assert_eq!(ws.resolve("new/dir/f.txt").display(), "new/dir/f.txt");
        assert!(ws.resolve("../outside").is_outside());
        assert!(ws.resolve("new/../../x").is_outside());
        assert!(ws.resolve("/etc/passwd").is_outside());
        let lib = ws.resolve(&tmp.path().join("lib/x.rs").to_string_lossy());
        assert!(matches!(
            lib.area,
            Area::Extra {
                access: Access::Write,
                ..
            }
        ));
        assert_eq!(lib.read_only_dir(), None);
        let docs = ws.resolve(&tmp.path().join("docs/a.md").to_string_lossy());
        assert!(docs.read_only_dir().is_some());
        // A missing extra dir is skipped.
        assert!(
            ws.resolve(&tmp.path().join("missing/x").to_string_lossy())
                .is_outside()
        );
    }

    #[test]
    fn symlinks_are_followed() {
        let (tmp, ws) = setup();
        symlink(tmp.path(), ws.root().join("up")).unwrap();
        symlink("/etc/passwd", ws.root().join("pw")).unwrap();
        assert!(ws.resolve("up/secret").is_outside());
        assert!(ws.resolve("pw").is_outside());
        assert!(ws.resolve("up/ws/src/a.rs").display() == "src/a.rs");
        // `..` after a link goes up from where the link points.
        symlink(tmp.path().join("lib"), ws.root().join("l")).unwrap();
        assert!(ws.resolve("l/../ws/src/a.rs").display() == "src/a.rs");
        // A dangling link is followed to its target.
        symlink(tmp.path().join("outside-new"), ws.root().join("dangling")).unwrap();
        assert!(ws.resolve("dangling").is_outside());
        symlink("src/new.rs", ws.root().join("inner")).unwrap();
        assert_eq!(ws.resolve("inner").display(), "src/new.rs");
    }

    #[test]
    fn tilde() {
        let ws = Workspace {
            root: PathBuf::from("/w"),
            extra: Vec::new(),
            home: Some(PathBuf::from("/home/u")),
        };
        assert_eq!(ws.tilde(Path::new("/home/u/ws/docs")), "~/ws/docs");
        assert_eq!(ws.tilde(Path::new("/opt")), "/opt");
    }
}
