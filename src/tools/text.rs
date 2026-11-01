//! Text files: binary detection, line numbers, safe writing (see
//! `spec/tools.md`, "Text files").

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Bytes looked at to tell a binary file.
const SNIFF: usize = 8192;

/// A NUL byte in the first 8 KiB.
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(SNIFF).any(|&b| b == 0)
}

/// Reads the start of a file and tells whether it is binary.
pub fn file_is_binary(path: &Path) -> io::Result<bool> {
    let mut buf = Vec::with_capacity(SNIFF);
    fs::File::open(path)?
        .take(SNIFF as u64)
        .read_to_end(&mut buf)?;
    Ok(is_binary(&buf))
}

/// 1-based line of a byte offset.
pub fn line_at(text: &str, offset: usize) -> usize {
    text.get(..offset)
        .map_or(1, |before| before.matches('\n').count() + 1)
}

/// Writes a file through a temporary file in the same directory and a
/// rename, so it is never left half written. `keep` gives the permissions
/// of the file being replaced.
pub fn write_atomic(path: &Path, data: &[u8], keep: Option<fs::Permissions>) -> io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let tmp: PathBuf = dir.join(format!(".{name}.clai-{}-{n}.tmp", std::process::id()));
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        file.write_all(data)?;
        file.sync_data()?;
        if let Some(perms) = keep {
            fs::set_permissions(&tmp, perms)?;
        }
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

/// Up to three names in the directory of `path` that look like its name:
/// the same ignoring case, or at most two edits away.
pub fn similar_names(path: &Path) -> Vec<String> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name()) else {
        return Vec::new();
    };
    let name = name.to_string_lossy().to_lowercase();
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(usize, String)> = entries
        .filter_map(Result::ok)
        .filter_map(|e| {
            let other = e.file_name().to_string_lossy().into_owned();
            let d = distance(&name, &other.to_lowercase());
            (d <= 2).then_some((d, other))
        })
        .collect();
    found.sort();
    found.into_iter().take(3).map(|(_, n)| n).collect()
}

/// Edit distance (insert, delete, replace) between two names.
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            let replace = prev.get(j).copied().unwrap_or(0) + usize::from(ca != cb);
            let delete = prev.get(j + 1).copied().unwrap_or(0) + 1;
            let insert = cur.get(j).copied().unwrap_or(0) + 1;
            cur.push(replace.min(delete).min(insert));
        }
        prev = cur;
    }
    prev.last().copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn binary() {
        assert!(is_binary(b"ab\0cd"));
        assert!(!is_binary(b"plain text"));
        let mut late = vec![b'a'; SNIFF];
        late.push(0);
        assert!(!is_binary(&late));
    }

    #[test]
    fn lines() {
        assert_eq!(line_at("a\nb\nc", 0), 1);
        assert_eq!(line_at("a\nb\nc", 2), 2);
        assert_eq!(line_at("a\nb\nc", 4), 3);
    }

    #[test]
    fn atomic_write_keeps_permissions() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("run.sh");
        fs::write(&path, "old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        let keep = fs::metadata(&path).unwrap().permissions();
        write_atomic(&path, b"new", Some(keep)).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        let mode = fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755);
        // No temporary file is left.
        assert_eq!(fs::read_dir(tmp.path()).unwrap().count(), 1);
    }

    #[test]
    fn similar() {
        let tmp = tempfile::tempdir().unwrap();
        for name in ["parse.rs", "Parser.rs", "main.rs", "parse.rs.bak"] {
            fs::write(tmp.path().join(name), "").unwrap();
        }
        let names = similar_names(&tmp.path().join("pars.rs"));
        assert_eq!(names, ["parse.rs", "Parser.rs"]);
        assert_eq!(
            similar_names(&tmp.path().join("PARSE.RS")),
            ["parse.rs", "Parser.rs"]
        );
        assert!(similar_names(&tmp.path().join("zzz")).is_empty());
        assert_eq!(distance("kitten", "sitting"), 3);
    }
}
