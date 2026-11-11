//! Writing the user file: `[tui]` keys for the TUI's `/settings` and
//! `/theme`, and `default_model` for the model picker, `/model --default` and
//! `clai models --set-default` (see `spec/config.md`, "Writing the user
//! file"). Everything else in the file stays as it is, comments and order
//! included.

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

/// Sets a text key at the top level of a TOML text, e.g. `default_model`.
/// An existing key keeps its place and the comment after it; a new one goes
/// before the first table, where TOML wants top-level keys.
pub fn set_top(text: &str, key: &str, value: &str) -> Result<String, String> {
    let mut doc: DocumentMut = text
        .parse()
        .map_err(|e: toml_edit::TomlError| format!("cannot read the file: {}", e.message()))?;
    let mut new = Value::from(value);
    match doc.get_mut(key) {
        Some(Item::Value(old)) => {
            *new.decor_mut() = old.decor().clone();
            *old = new;
        }
        Some(_) => return Err(format!("{key} is not a value")),
        None => {
            doc.insert(key, Item::Value(new));
        }
    }
    Ok(doc.to_string())
}

/// Writes `changes` into `[tui]` of the user file at `path`; see
/// [`save_with`].
pub fn save(
    path: &Path,
    expected: Option<Stamp>,
    changes: &[(&str, Option<Setting>)],
    check: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(), String> {
    save_with(path, expected, |text| set_tui(text, changes), check)
}

/// Writes the text `change` makes of the user file at `path`.
///
/// - `expected` is the stamp of the file when its values were read: if the
///   file changed since, nothing is written, so no change is lost silently.
///   `None` skips this, for a change made from the text as it is now.
/// - `check` gets the new text and must accept it, as loading would.
/// - The file is replaced atomically (a temporary file next to it, then a
///   rename) with its mode kept; a link is written through, to its target.
pub fn save_with(
    path: &Path,
    expected: Option<Stamp>,
    change: impl FnOnce(&str) -> Result<String, String>,
    check: impl FnOnce(&str) -> Result<(), String>,
) -> Result<(), String> {
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if expected.is_some() && stamp(&target) != expected {
        return Err("the file changed since it was read; open it again".to_owned());
    }
    let text = fs::read_to_string(&target).map_err(|e| format!("cannot read: {e}"))?;
    let new = change(&text)?;
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
    fn a_top_level_key_changes_in_place() {
        let file =
            "# mine\ndefault_model = \"local/qwen\"   # the small one\n\n[tui]\ntheme = \"dark\"\n";
        let new = set_top(file, "default_model", "anthropic/opus").unwrap();
        assert_eq!(
            new,
            "# mine\ndefault_model = \"anthropic/opus\"   # the small one\n\n[tui]\ntheme = \"dark\"\n"
        );
    }

    #[test]
    fn a_new_top_level_key_goes_before_the_tables() {
        let file =
            "# mine\n\n[tui]\ntheme = \"dark\"\n\n[providers.local]\napi = \"openai-compatible\"\n";
        let new = set_top(file, "default_model", "local/qwen").unwrap();
        let key = new.find("default_model").unwrap();
        assert!(key < new.find("[tui]").unwrap(), "{new}");
        // It is a top-level key, not one of [tui].
        let doc: DocumentMut = new.parse().unwrap();
        assert_eq!(doc["default_model"].as_str(), Some("local/qwen"));
        assert!(doc["tui"].get("default_model").is_none());
        assert_eq!(doc["tui"]["theme"].as_str(), Some("dark"));
        assert_eq!(
            set_top("", "default_model", "a/b").unwrap(),
            "default_model = \"a/b\"\n"
        );
        assert!(set_top("[default_model]\nx = 1\n", "default_model", "a/b").is_err());
        assert!(set_top("[tui\n", "default_model", "a/b").is_err());
    }

    #[test]
    fn save_with_skips_the_stamp_check_without_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        fs::write(&path, FILE).unwrap();
        save_with(
            &path,
            None,
            |text| set_top(text, "default_model", "local/other"),
            |_| Ok(()),
        )
        .unwrap();
        let written = fs::read_to_string(&path).unwrap();
        assert!(
            written.contains("default_model = \"local/other\""),
            "{written}"
        );
        assert!(written.contains("# dark at night"), "{written}");
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
