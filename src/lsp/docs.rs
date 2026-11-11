//! Open files and positions.
//!
//! [`Docs`] keeps the files a server has open, with the size and
//! modification time they had when last sent, and decides which
//! `didOpen`, `didChange`, `didSave` and `didClose` notifications keep the
//! server in sync with the disk. It does no I/O itself; the server reads
//! the files and sends what it returns.
//!
//! Positions: the model names a symbol on a 1-based line; LSP counts
//! characters in UTF-16 code units by default, or UTF-8 bytes when the
//! server chose that encoding.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use indexmap::IndexMap;
use serde_json::{Value, json};

/// At most this many files stay open per server.
pub const MAX_OPEN: usize = 50;

/// How the server counts characters in a line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Encoding {
    Utf8,
    #[default]
    Utf16,
    Utf32,
}

impl Encoding {
    /// From the server's `positionEncoding`; UTF-16 when absent or unknown.
    pub fn from_kind(kind: Option<&str>) -> Encoding {
        match kind {
            Some("utf-8") => Encoding::Utf8,
            Some("utf-32") => Encoding::Utf32,
            _ => Encoding::Utf16,
        }
    }

    fn units(self, c: char) -> usize {
        match self {
            Encoding::Utf8 => c.len_utf8(),
            Encoding::Utf16 => c.len_utf16(),
            Encoding::Utf32 => 1,
        }
    }
}

/// Characters that make up a word, for finding a symbol as a whole word.
fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The character column (0-based) of the `occurrence`-th (1-based)
/// whole-word match of `symbol` in `line`. A symbol that does not start or
/// end with a word character (an operator) matches without that boundary.
pub fn find_symbol(line: &str, symbol: &str, occurrence: usize) -> Option<usize> {
    if symbol.is_empty() || occurrence == 0 {
        return None;
    }
    let starts_word = symbol.chars().next().is_some_and(is_word);
    let ends_word = symbol.chars().next_back().is_some_and(is_word);
    let mut found = 0;
    for (byte, _) in line.match_indices(symbol) {
        let before = line.get(..byte).and_then(|s| s.chars().next_back());
        let after = line
            .get(byte + symbol.len()..)
            .and_then(|s| s.chars().next());
        if starts_word && before.is_some_and(is_word) {
            continue;
        }
        if ends_word && after.is_some_and(is_word) {
            continue;
        }
        found += 1;
        if found == occurrence {
            return line.get(..byte).map(|s| s.chars().count());
        }
    }
    None
}

/// The LSP `character` of a character column of `line`. A column past the
/// end gives the end of the line.
pub fn to_lsp_character(line: &str, column: usize, encoding: Encoding) -> u32 {
    let units: usize = line.chars().take(column).map(|c| encoding.units(c)).sum();
    u32::try_from(units).unwrap_or(u32::MAX)
}

/// The character column of an LSP `character` of `line`. A position inside
/// a character counts as that character; past the end gives the end.
pub fn from_lsp_character(line: &str, character: u32, encoding: Encoding) -> usize {
    let target = usize::try_from(character).unwrap_or(usize::MAX);
    let mut units = 0;
    for (column, c) in line.chars().enumerate() {
        if units >= target {
            return column;
        }
        units += encoding.units(c);
        if units > target {
            return column;
        }
    }
    line.chars().count()
}

/// The lines of a text as LSP counts them: split at `\n`, `\r\n` and `\r`.
pub fn lines(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    loop {
        match rest.find(['\n', '\r']) {
            Some(i) => {
                out.push(rest.get(..i).unwrap_or(""));
                let skip = if rest.get(i..i + 2) == Some("\r\n") {
                    2
                } else {
                    1
                };
                rest = rest.get(i + skip..).unwrap_or("");
            }
            None => {
                out.push(rest);
                return out;
            }
        }
    }
}

/// `file://` URI of an absolute path, with bytes outside the unreserved
/// set and `/` percent-encoded.
pub fn path_to_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'.' | b'_' | b'~') {
            uri.push(char::from(b));
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

/// The path of a `file://` URI; `None` for other schemes or a bad
/// encoding.
pub fn uri_to_path(uri: &str) -> Option<PathBuf> {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let rest = uri.strip_prefix("file://")?;
    // `file://host/path`: only an empty host or `localhost`.
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let bytes = rest.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%' {
            let hex = rest.get(i + 1..i + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b);
            i += 1;
        }
    }
    Some(PathBuf::from(OsString::from_vec(out)))
}

/// The LSP `languageId` of a file, from its extension.
pub fn language_id(path: &Path) -> String {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
    match ext {
        "rs" => "rust",
        "go" => "go",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        "py" | "pyi" => "python",
        "ts" => "typescript",
        "tsx" => "typescriptreact",
        "js" => "javascript",
        "jsx" => "javascriptreact",
        other => other,
    }
    .to_string()
}

/// What identifies the content of a file on disk without reading it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    pub size: u64,
    pub modified: Option<SystemTime>,
}

impl Stamp {
    pub fn of(meta: &std::fs::Metadata) -> Stamp {
        Stamp {
            size: meta.len(),
            modified: meta.modified().ok(),
        }
    }
}

/// A notification to send to the server: method and params.
pub type Notification = (&'static str, Value);

#[derive(Debug, Clone)]
struct Open {
    version: i32,
    stamp: Stamp,
}

/// The open files of one server, least recently used first.
#[derive(Debug, Default)]
pub struct Docs {
    open: IndexMap<PathBuf, Open>,
    /// Whether the server wants `didSave`.
    pub send_save: bool,
}

impl Docs {
    pub fn new(send_save: bool) -> Docs {
        Docs {
            open: IndexMap::new(),
            send_save,
        }
    }

    /// The open files, least recently used first.
    pub fn paths(&self) -> Vec<PathBuf> {
        self.open.keys().cloned().collect()
    }

    pub fn is_open(&self, path: &Path) -> bool {
        self.open.contains_key(path)
    }

    /// The version last sent for an open file.
    pub fn version(&self, path: &Path) -> Option<i32> {
        self.open.get(path).map(|o| o.version)
    }

    /// The stamp last sent for an open file.
    pub fn stamp(&self, path: &Path) -> Option<Stamp> {
        self.open.get(path).map(|o| o.stamp)
    }

    /// A file is about to be asked about, with its stamp and text on disk:
    /// opened if it is not, changed if it differs from what was sent, and
    /// made the most recently used. Opening the 51st file closes the least
    /// recently used one.
    pub fn use_file(&mut self, path: &Path, stamp: Stamp, text: &str) -> Vec<Notification> {
        if let Some(index) = self.open.get_index_of(path) {
            let last = self.open.len().saturating_sub(1);
            self.open.move_index(index, last);
            return self.changed(path, stamp, text);
        }
        let mut out = Vec::new();
        while self.open.len() >= MAX_OPEN {
            let Some((old, _)) = self.open.shift_remove_index(0) else {
                break;
            };
            out.push(close(&old));
        }
        self.open
            .insert(path.to_path_buf(), Open { version: 1, stamp });
        out.push((
            "textDocument/didOpen",
            json!({"textDocument": {
                "uri": path_to_uri(path),
                "languageId": language_id(path),
                "version": 1,
                "text": text,
            }}),
        ));
        out
    }

    /// An open file has `stamp` on disk: `didChange` with the full text if
    /// it differs from what was sent. `text` is read only then.
    pub fn changed(&mut self, path: &Path, stamp: Stamp, text: &str) -> Vec<Notification> {
        let Some(open) = self.open.get_mut(path) else {
            return Vec::new();
        };
        if open.stamp == stamp {
            return Vec::new();
        }
        open.stamp = stamp;
        open.version = open.version.saturating_add(1);
        vec![change(path, open.version, text)]
    }

    /// `edit_file` or `write_file` wrote an open file: `didChange` and
    /// `didSave` at once, also when the stamp looks the same (an edit in
    /// the same second with the same size).
    pub fn saved(&mut self, path: &Path, stamp: Stamp, text: &str) -> Vec<Notification> {
        let Some(open) = self.open.get_mut(path) else {
            return Vec::new();
        };
        open.stamp = stamp;
        open.version = open.version.saturating_add(1);
        let mut out = vec![change(path, open.version, text)];
        if self.send_save {
            out.push((
                "textDocument/didSave",
                json!({"textDocument": {"uri": path_to_uri(path)}}),
            ));
        }
        out
    }

    /// An open file is gone from disk: `didClose`.
    pub fn removed(&mut self, path: &Path) -> Vec<Notification> {
        if self.open.shift_remove(path).is_some() {
            vec![close(path)]
        } else {
            Vec::new()
        }
    }
}

fn change(path: &Path, version: i32, text: &str) -> Notification {
    (
        "textDocument/didChange",
        json!({
            "textDocument": {"uri": path_to_uri(path), "version": version},
            "contentChanges": [{"text": text}],
        }),
    )
}

fn close(path: &Path) -> Notification {
    (
        "textDocument/didClose",
        json!({"textDocument": {"uri": path_to_uri(path)}}),
    )
}
