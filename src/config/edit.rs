//! Writing `[tui]` keys into the user file, for the TUI's `/settings` and
//! `/theme` (see `spec/config.md`, "TUI"). Everything else in the file stays
//! as it is, comments and order included.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use toml_edit::{DocumentMut, Item, Table, Value};

/// A file's modification time and size, as the server polls it.
pub type Stamp = (SystemTime, u64);

/// The stamp of a file, through links; `None` if it cannot be read.
pub fn stamp(path: &Path) -> Option<Stamp> {
    let meta = fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// A value of a `[tui]` key as written in the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Setting {
    Bool(bool),
    Int(i64),
    Text(String),
}

impl Setting {
    fn value(&self) -> Value {
        match self {
            Setting::Bool(b) => Value::from(*b),
            Setting::Int(n) => Value::from(*n),
            Setting::Text(t) => Value::from(t.as_str()),
        }
    }
}

/// The keys a TOML text sets in `[tui]`; none if it does not parse.
pub fn tui_keys(text: &str) -> Vec<String> {
    let Ok(doc) = text.parse::<DocumentMut>() else {
        return Vec::new();
    };
    doc.get("tui")
        .and_then(Item::as_table_like)
        .map(|t| t.iter().map(|(k, _)| k.to_owned()).collect())
        .unwrap_or_default()
}

/// Sets (`Some`) or removes (`None`) keys of `[tui]` in a TOML text, and
/// creates the table at the end if it is missing. A value keeps the comment
/// after it on its line.
pub fn set_tui(text: &str, changes: &[(&str, Option<Setting>)]) -> Result<String, String> {
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| format!("cannot read the file: {}", e.message()))?;
    if !doc.contains_key("tui") {
        if changes.iter().all(|(_, v)| v.is_none()) {
            return Ok(text.to_owned());
        }
        doc.insert("tui", Item::Table(Table::new()));
    }
    let table = doc
        .get_mut("tui")
        .and_then(Item::as_table_like_mut)
        .ok_or("tui is not a table")?;
    for (key, setting) in changes {
        match setting {
            None => {
                table.remove(key);
            }
            Some(setting) => {
                let mut value = setting.value();
                if let Some(old) = table.get_mut(key).and_then(Item::as_value_mut) {
                    *value.decor_mut() = old.decor().clone();
                    *old = value;
                } else {
                    table.insert(key, Item::Value(value));
                }
            }
        }
    }
    Ok(doc.to_string())
}

/// Writes `changes` into the user file at `path`.
///
/// - `expected` is the stamp of the file when its values were read: if the
///   file changed since, nothing is written, so no change is lost silently.
/// - `check` gets the new text and must accept it, as loading would.
/// - The file is replaced atomically (a temporary file next to it, then a
///   rename) with its mode kept; a link is written through, to its target.
pub fn save(
    path: &Path,
    expected: Option<Stamp>,
    changes: &[(&str, Option<Setting>)],
    check: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(), String> {
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if expected.is_some() && stamp(&target) != expected {
        return Err("the file changed since it was read; open it again".to_owned());
    }
    let text = fs::read_to_string(&target).map_err(|e| format!("cannot read: {e}"))?;
    let new = set_tui(&text, changes)?;
    if new == text {
        return Ok(());
    }
    check(&new)?;
    let mode = fs::metadata(&target)
        .map_err(|e| format!("cannot read: {e}"))?
        .permissions();
    let temporary = temporary(&target);
    let written = (|| {
        let mut file = fs::File::create(&temporary)?;
        file.set_permissions(mode)?;
        file.write_all(new.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, &target)
    })();
    if let Err(e) = written {
        let _ = fs::remove_file(&temporary);
        return Err(format!("cannot write {}: {e}", target.display()));
    }
    Ok(())
}

/// `.config.toml.clai-1234` next to the file.
fn temporary(target: &Path) -> PathBuf {
    let name = target
        .file_name()
        .map_or_else(|| "config.toml".into(), |n| n.to_string_lossy());
    target.with_file_name(format!(".{name}.clai-{}", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    const FILE: &str = r#"# my configuration
default_model = "local/qwen"

[tui]
# dark at night
theme = "dark"   # or light
mouse = false

[providers.local]
api = "openai-compatible"
"#;

    #[test]
    fn values_change_in_place_and_comments_stay() {
        let new = set_tui(
            FILE,
            &[
                ("theme", Some(Setting::Text("nord".into()))),
                ("mouse", None),
                ("max_fps", Some(Setting::Int(60))),
                ("timestamps", Some(Setting::Bool(true))),
            ],
        )
        .unwrap();
        assert_eq!(
            new,
            r#"# my configuration
default_model = "local/qwen"

[tui]
# dark at night
theme = "nord"   # or light
max_fps = 60
timestamps = true

[providers.local]
api = "openai-compatible"
"#
        );
    }

    #[test]
    fn the_keys_of_tui() {
        assert_eq!(tui_keys(FILE), ["theme", "mouse"]);
        assert_eq!(tui_keys("tui = { max_fps = 9 }"), ["max_fps"]);
        assert!(tui_keys("x = 1").is_empty());
        assert!(tui_keys("[tui").is_empty());
    }

    #[test]
    fn a_missing_table_is_added_at_the_end() {
        let file = "default_model = \"a/b\"\n";
        let new = set_tui(file, &[("theme", Some(Setting::Text("nord".into())))]).unwrap();
        assert_eq!(new, "default_model = \"a/b\"\n\n[tui]\ntheme = \"nord\"\n");
        // Only removals: nothing to add.
        assert_eq!(set_tui(file, &[("theme", None)]).unwrap(), file);
        // An inline table is changed as it is.
        let new = set_tui("tui = { mouse = false }\n", &[("mouse", None)]).unwrap();
        assert_eq!(new, "tui = {}\n");
        assert!(set_tui("tui = 1\n", &[("mouse", None)]).is_err());
        assert!(set_tui("[tui\n", &[]).is_err());
    }

    #[test]
    fn save_writes_atomically_and_keeps_the_mode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, FILE).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let link = dir.path().join("link.toml");
        symlink(&path, &link).unwrap();
        let before = stamp(&link);
        let mut checked = String::new();
        save(
            &link,
            before,
            &[("mouse", Some(Setting::Bool(true)))],
            |text| {
                checked = text.to_owned();
                Ok(())
            },
        )
        .unwrap();
        let written = fs::read_to_string(&path).unwrap();
        assert!(written.contains("mouse = true"), "{written}");
        assert_eq!(checked, written);
        // Through the link: it is still a link.
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        // No temporary file left.
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 2);
    }

    #[test]
    fn save_refuses_a_changed_file_and_a_rejected_text() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, FILE).unwrap();
        let old = stamp(&path);
        fs::write(&path, format!("{FILE}\n# edited meanwhile\n")).unwrap();
        let e = save(&path, old, &[("mouse", None)], |_| Ok(())).unwrap_err();
        assert!(e.contains("changed since it was read"), "{e}");
        let now = stamp(&path);
        let e = save(&path, now, &[("mouse", None)], |_| {
            Err("tui.theme: unknown".into())
        })
        .unwrap_err();
        assert_eq!(e, "tui.theme: unknown");
        assert!(fs::read_to_string(&path).unwrap().contains("mouse = false"));
    }
}
