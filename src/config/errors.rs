//! Configuration errors with their location.

use std::fmt;
use std::ops::Range;
use std::path::{Path, PathBuf};

/// Where a configuration value came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Origin {
    /// A config file, with the line of the value when it is known.
    File { path: PathBuf, line: Option<usize> },
    /// An environment variable.
    Env(&'static str),
}

/// One problem in the configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError {
    pub origin: Origin,
    /// Dotted key, e.g. `providers.local.models.qwen`.
    pub key: Option<String>,
    pub message: String,
}

impl fmt::Display for ConfigError {
    /// `config.toml:14: providers.local: missing api`
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.origin {
            Origin::File {
                path,
                line: Some(line),
            } => write!(f, "{}:{line}: ", path.display())?,
            Origin::File { path, line: None } => write!(f, "{}: ", path.display())?,
            Origin::Env(name) => write!(f, "{name}: ")?,
        }
        if let Some(key) = &self.key {
            write!(f, "{key}: ")?;
        }
        f.write_str(&self.message)
    }
}

/// All problems found while loading the configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigErrors(pub Vec<ConfigError>);

impl fmt::Display for ConfigErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, error) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("\n")?;
            }
            write!(f, "{error}")?;
        }
        Ok(())
    }
}

/// A config file being checked: turns byte spans into line numbers.
#[derive(Clone, Copy)]
pub(crate) struct FileCtx<'a> {
    pub path: &'a Path,
    pub text: &'a str,
}

impl FileCtx<'_> {
    pub fn error(
        &self,
        span: Option<Range<usize>>,
        key: Option<String>,
        message: impl Into<String>,
    ) -> ConfigError {
        ConfigError {
            origin: Origin::File {
                path: self.path.to_owned(),
                line: span.and_then(|s| self.line_of(s.start)),
            },
            key,
            message: message.into(),
        }
    }

    /// 1-based line of a byte offset.
    fn line_of(&self, offset: usize) -> Option<usize> {
        self.text
            .get(..offset)
            .map(|before| before.matches('\n').count() + 1)
    }

    /// Parses the file as `T`; syntax and type errors become a `ConfigError`.
    pub fn parse<T: serde::de::DeserializeOwned>(&self) -> Result<T, ConfigError> {
        toml::from_str(self.text).map_err(|e| self.error(e.span(), None, e.message().trim()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_with_line_and_key() {
        let ctx = FileCtx {
            path: Path::new("c.toml"),
            text: "a\nb\nc",
        };
        let err = ctx.error(Some(4..5), Some("x.y".into()), "bad");
        assert_eq!(err.to_string(), "c.toml:3: x.y: bad");
    }

    #[test]
    fn display_env() {
        let err = ConfigError {
            origin: Origin::Env("CLAI_MODEL"),
            key: None,
            message: "unknown model".into(),
        };
        assert_eq!(err.to_string(), "CLAI_MODEL: unknown model");
    }

    #[test]
    fn offset_past_end_has_no_line() {
        let ctx = FileCtx {
            path: Path::new("c.toml"),
            text: "a",
        };
        let err = ctx.error(Some(99..100), None, "bad");
        assert_eq!(err.to_string(), "c.toml: bad");
    }
}
