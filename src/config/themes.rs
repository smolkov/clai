//! Themes besides the `[themes.NAME]` tables (see `spec/config.md`, "Themes"):
//! the files of the `themes/` directory next to the user file, and the themes
//! bundled with `clai`. A theme file has the keys of a table at its top level.

use std::fs;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;

use super::errors::{ConfigError, FileCtx};
use super::file::ThemeFile;
use super::tui::{BuiltinTheme, Theme, ThemeSource, build_theme};

/// The themes built into the binary, from `themes/` in the repository.
pub const BUNDLED: [(&str, &str); 4] = [
    (
        "atom-one-dark",
        include_str!("../../themes/atom-one-dark.toml"),
    ),
    ("nord", include_str!("../../themes/nord.toml")),
    (
        "gruvbox-dark",
        include_str!("../../themes/gruvbox-dark.toml"),
    ),
    (
        "solarized-light",
        include_str!("../../themes/solarized-light.toml"),
    ),
];

/// A theme file as read: its name, path and text, or why it could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeText {
    pub name: String,
    pub path: PathBuf,
    pub text: Result<String, String>,
}

/// `themes/` next to the user file.
pub fn directory(user_file: &Path) -> PathBuf {
    user_file
        .parent()
        .map_or_else(|| PathBuf::from("themes"), |dir| dir.join("themes"))
}

/// Whether a file name, without `.toml`, can be a theme name: letters,
/// digits, `-` and `_`.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The `NAME.toml` files of a directory, by name. A missing directory has
/// none; other files are ignored.
pub fn read_directory(dir: &Path) -> Vec<ThemeText> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<ThemeText> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            let name = path
                .file_name()?
                .to_str()?
                .strip_suffix(".toml")?
                .to_owned();
            if !valid_name(&name) || !path.is_file() {
                return None;
            }
            let text = fs::read_to_string(&path).map_err(|e| format!("cannot read: {e}"));
            Some(ThemeText { name, path, text })
        })
        .collect();
    files.sort_by(|a, b| a.name.cmp(&b.name));
    files
}

/// A theme file that could not be used, and why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Broken {
    pub name: String,
    pub errors: Vec<ConfigError>,
}

/// Builds one theme file. `None` with the errors if it has any.
fn build_file(name: &str, path: &Path, text: &str, source: ThemeSource) -> Result<Theme, Broken> {
    let ctx = FileCtx { path, text };
    let broken = |errors| Broken {
        name: name.to_owned(),
        errors,
    };
    let file: ThemeFile = ctx.parse().map_err(|e| broken(vec![e]))?;
    let mut errors = Vec::new();
    if BuiltinTheme::from_name(name).is_some() {
        errors.push(ctx.error(
            None,
            None,
            "a theme file may not use the name of a built-in theme",
        ));
    }
    let theme = build_theme(ctx, "", file, source, &mut errors);
    match theme {
        Some(theme) if errors.is_empty() => Ok(theme),
        _ => Err(broken(errors)),
    }
}

/// The bundled themes, then the files of the directory over them: a file
/// replaces a bundled theme of its name in its place. Broken ones are left
/// out and returned apart.
pub fn file_themes(files: &[ThemeText]) -> (IndexMap<String, Theme>, Vec<Broken>) {
    let mut themes = IndexMap::new();
    let mut broken = Vec::new();
    for (name, text) in BUNDLED {
        let path = PathBuf::from(format!("<bundled>/{name}.toml"));
        match build_file(name, &path, text, ThemeSource::Bundled) {
            Ok(theme) => {
                themes.insert(name.to_owned(), theme);
            }
            Err(b) => broken.push(b),
        }
    }
    for file in files {
        let built = match &file.text {
            Ok(text) => build_file(
                &file.name,
                &file.path,
                text,
                ThemeSource::File(file.path.clone()),
            ),
            Err(message) => Err(Broken {
                name: file.name.clone(),
                errors: vec![
                    FileCtx {
                        path: &file.path,
                        text: "",
                    }
                    .error(None, None, message.clone()),
                ],
            }),
        };
        match built {
            Ok(theme) => {
                themes.insert(file.name.clone(), theme);
            }
            Err(b) => {
                // A broken file does not fall back to the bundled theme of
                // its name: the user meant the file.
                themes.shift_remove(&file.name);
                broken.push(b);
            }
        }
    }
    (themes, broken)
}
