//! Secrets hidden from the model (RFC 0030; see `spec/agent-loop.md`,
//! "Secrets"): API keys, tokens, passwords and private keys in text bound
//! for a provider become placeholders like `[secret: anthropic key #3f9a]`.
//! The values of the placeholders a session has seen stay in memory, so
//! file edits can put them back.

#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::sync::Mutex;

use regex::Regex;
use serde_json::Value;

use crate::config::{KeySource, Provider, SecretsSettings};
use crate::sync::lock;

/// Known values shorter than this are not hidden: they would match words.
const MIN_KNOWN: usize = 8;
/// Assignment values shorter than this are not hidden.
const MIN_ASSIGNED: usize = 12;
/// Bits per character an assignment value needs to count as random.
const MIN_ENTROPY: f64 = 3.5;

/// Name endings of environment variables whose values are secret.
const ENV_SUFFIXES: [&str; 8] = [
    "_KEY",
    "_TOKEN",
    "_SECRET",
    "_PASSWORD",
    "_PASS",
    "_PAT",
    "_CREDENTIALS",
    "_APIKEY",
];
/// Environment variables whose values are secret, whatever their ending.
const ENV_NAMES: [&str; 6] = [
    "GITHUB_TOKEN",
    "GH_TOKEN",
    "NPM_TOKEN",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "HF_TOKEN",
];
/// Variables that end like a secret but hold none.
const ENV_NOT_SECRET: [&str; 3] = ["SSH_AUTH_SOCK", "GPG_AGENT_INFO", "KEYTIMEOUT"];

/// Key formats: kind, pattern, and the group that is the secret (0: all).
const FORMATS: [(&str, &str, usize); 14] = [
    ("anthropic key", r"\bsk-ant-[A-Za-z0-9_-]{32,}", 0),
    (
        "openai key",
        r"\bsk-(?:proj-|svcacct-|admin-)?[A-Za-z0-9_-]{32,}",
        0,
    ),
    ("github token", r"\bgh[pousr]_[A-Za-z0-9]{36,}", 0),
    ("github token", r"\bgithub_pat_[A-Za-z0-9_]{50,}", 0),
    ("gitlab token", r"\bglpat-[A-Za-z0-9_-]{20,}", 0),
    ("aws access key", r"\b(?:AKIA|ASIA)[A-Z0-9]{16}\b", 0),
    ("google api key", r"\bAIza[A-Za-z0-9_-]{35}", 0),
    ("slack token", r"\bxox[abposr]-[A-Za-z0-9-]{10,}", 0),
    ("stripe key", r"\b(?:sk|rk)_live_[A-Za-z0-9]{16,}", 0),
    ("huggingface token", r"\bhf_[A-Za-z0-9]{34,}", 0),
    (
        "private key",
        r"(?s)-----BEGIN[A-Z0-9 ]*PRIVATE KEY-----.*?(?:-----END[A-Z0-9 ]*PRIVATE KEY-----|\z)",
        0,
    ),
    (
        "jwt",
        r"\beyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
        0,
    ),
    (
        "url password",
        r"\b[A-Za-z][A-Za-z0-9+.-]*://[^\s:/@]+:([^\s/@]+)@",
        1,
    ),
    ("token", r"(?i)\bbearer\s+([A-Za-z0-9._~+/=-]{12,})", 1),
];

/// `name = value` with a name that says it is secret; group 1 the name,
/// group 2 the value.
const ASSIGNMENT: &str = r#"(?i)\b([A-Za-z0-9_.-]*(?:key|token|secret|password|passwd|pwd|credential|auth)[A-Za-z0-9_.-]*)["']?\s*(?:=|:|=>)\s*["']?([^\s"'`,;)\]}>]+)"#;

/// A placeholder in text, as [`Redactor`] writes it.
const PLACEHOLDER: &str = r"\[secret: [^\]\[#]+ #[0-9a-f]{4}\]";

/// Hides secrets in text for one session.
pub struct Redactor {
    /// Exact values, with their kinds, longest first.
    known: Vec<(String, String)>,
    formats: Vec<(String, Regex, usize)>,
    assignment: Option<Regex>,
    allow: Vec<String>,
    placeholder: Option<Regex>,
    /// The values of the placeholders hidden so far.
    seen: Mutex<HashMap<String, String>>,
}

impl std::fmt::Debug for Redactor {
    /// Never shows a value.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Redactor")
            .field("known", &self.known.len())
            .field("formats", &self.formats.len())
            .finish_non_exhaustive()
    }
}

/// Text with its secrets hidden.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Redacted {
    pub text: String,
    /// The placeholders put in, in order; the same one may come twice.
    pub hidden: Vec<String>,
}

/// One match before the matches are chosen.
struct Hit {
    start: usize,
    end: usize,
    kind: String,
}

impl Redactor {
    /// From `[secrets]`, the configured providers and the environment.
    /// `None` with `redact = false`.
    pub fn from_config<'a>(
        settings: &SecretsSettings,
        providers: impl IntoIterator<Item = &'a Provider>,
        vars: impl IntoIterator<Item = (String, String)>,
    ) -> Option<Redactor> {
        if !settings.redact {
            return None;
        }
        let vars: Vec<(String, String)> = vars.into_iter().collect();
        let mut known = Vec::new();
        for provider in providers {
            match &provider.key {
                KeySource::Value(key) => {
                    known.push((format!("{} key", provider.name), key.expose().to_owned()))
                }
                KeySource::Env(name) => {
                    if let Some((_, value)) = vars.iter().find(|(n, _)| n == name) {
                        known.push((format!("env {name}"), value.clone()));
                    }
                }
                KeySource::None | KeySource::Cmd(_) => {}
            }
        }
        for (name, value) in &vars {
            if secret_env_name(name) || settings.values_env.contains(name) {
                known.push((format!("env {name}"), value.clone()));
            }
        }
        Some(Redactor::new(settings, known))
    }

    /// With exact values `known` as (kind, value).
    pub fn new(settings: &SecretsSettings, known: Vec<(String, String)>) -> Redactor {
        let mut known: Vec<(String, String)> = known
            .into_iter()
            .filter(|(_, value)| {
                let v = value.trim();
                v.len() >= MIN_KNOWN && !v.starts_with('/') && !v.starts_with('~')
            })
            .map(|(kind, value)| (kind, value.trim().to_owned()))
            .collect();
        known.sort_by_key(|(_, value)| std::cmp::Reverse(value.len()));
        known.dedup_by(|a, b| a.1 == b.1);
        let mut formats: Vec<(String, Regex, usize)> = FORMATS
            .iter()
            .filter_map(|(kind, pattern, group)| {
                Regex::new(pattern)
                    .ok()
                    .map(|re| ((*kind).to_owned(), re, *group))
            })
            .collect();
        // Checked when the configuration is loaded.
        formats.extend(
            settings
                .patterns
                .iter()
                .filter_map(|p| Regex::new(p).ok())
                .map(|re| ("secret".to_owned(), re, 0)),
        );
        Redactor {
            known,
            formats,
            assignment: settings
                .assignments
                .then(|| Regex::new(ASSIGNMENT).ok())
                .flatten(),
            allow: settings.allow.clone(),
            placeholder: Regex::new(PLACEHOLDER).ok(),
            seen: Mutex::new(HashMap::new()),
        }
    }

    /// `text` with every secret replaced by its placeholder.
    pub fn redact(&self, text: &str) -> Redacted {
        let mut hits = self.hits(text);
        if hits.is_empty() {
            return Redacted {
                text: text.to_owned(),
                hidden: Vec::new(),
            };
        }
        hits.sort_by_key(|h| h.start);
        let mut out = String::with_capacity(text.len());
        let mut hidden = Vec::new();
        let mut at = 0;
        let mut seen = lock(&self.seen);
        for hit in hits {
            let (Some(before), Some(value)) =
                (text.get(at..hit.start), text.get(hit.start..hit.end))
            else {
                continue;
            };
            let placeholder = placeholder(&hit.kind, value);
            out.push_str(before);
            out.push_str(&placeholder);
            seen.insert(placeholder.clone(), value.to_owned());
            hidden.push(placeholder);
            at = hit.end;
        }
        out.push_str(text.get(at..).unwrap_or(""));
        Redacted { text: out, hidden }
    }

    /// The text with the secrets hidden, and nothing else.
    pub fn hide(&self, text: &str) -> String {
        self.redact(text).text
    }

    /// Every string in a JSON value, redacted.
    pub fn redact_json(&self, value: &Value) -> Value {
        match value {
            Value::String(s) => Value::String(self.hide(s)),
            Value::Array(items) => {
                Value::Array(items.iter().map(|v| self.redact_json(v)).collect())
            }
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(k, v)| (k.clone(), self.redact_json(v)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    /// `text` with the placeholders this session has seen replaced by their
    /// values; the placeholders it has not seen, as they are.
    pub fn restore(&self, text: &str) -> Restored {
        let Some(re) = &self.placeholder else {
            return Restored {
                text: text.to_owned(),
                unknown: Vec::new(),
            };
        };
        let seen = lock(&self.seen);
        let mut unknown = Vec::new();
        let restored = re.replace_all(text, |caps: &regex::Captures<'_>| {
            let found = caps.get(0).map_or("", |m| m.as_str());
            match seen.get(found) {
                Some(value) => value.clone(),
                None => {
                    unknown.push(found.to_owned());
                    found.to_owned()
                }
            }
        });
        Restored {
            text: restored.into_owned(),
            unknown,
        }
    }

    /// The matches to replace, without overlaps: known values first, then
    /// key formats, then assignments.
    fn hits(&self, text: &str) -> Vec<Hit> {
        let mut chosen: Vec<Hit> = Vec::new();
        let take = |hit: Hit, chosen: &mut Vec<Hit>| {
            let free = chosen
                .iter()
                .all(|c| hit.end <= c.start || hit.start >= c.end);
            let value = text.get(hit.start..hit.end).unwrap_or("");
            if free && hit.end > hit.start && !self.allow.iter().any(|a| a == value) {
                chosen.push(hit);
            }
        };
        for (kind, value) in &self.known {
            for (start, _) in text.match_indices(value.as_str()) {
                take(
                    Hit {
                        start,
                        end: start + value.len(),
                        kind: kind.clone(),
                    },
                    &mut chosen,
                );
            }
        }
        for (kind, re, group) in &self.formats {
            for caps in re.captures_iter(text) {
                let Some(m) = caps.get(*group) else { continue };
                if *group > 0 && looks_like_reference(m.as_str()) {
                    continue;
                }
                take(
                    Hit {
                        start: m.start(),
                        end: m.end(),
                        kind: kind.clone(),
                    },
                    &mut chosen,
                );
            }
        }
        if let Some(re) = &self.assignment {
            for caps in re.captures_iter(text) {
                let (Some(name), Some(m)) = (caps.get(1), caps.get(2)) else {
                    continue;
                };
                if !looks_random(m.as_str()) {
                    continue;
                }
                let kind = assignment_kind(name.as_str());
                take(
                    Hit {
                        start: m.start(),
                        end: m.end(),
                        kind: kind.to_owned(),
                    },
                    &mut chosen,
                );
            }
        }
        chosen
    }
}

/// What [`Redactor::restore`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Restored {
    pub text: String,
    /// Placeholders whose values the session does not know.
    pub unknown: Vec<String>,
}

/// `[secret: KIND #ID]`, with four hex digits of a hash of the value.
pub fn placeholder(kind: &str, value: &str) -> String {
    format!("[secret: {kind} #{:04x}]", fnv1a(value) & 0xffff)
}

/// How many placeholders `text` holds.
pub fn count(text: &str) -> usize {
    // A fixed pattern; it always compiles.
    Regex::new(PLACEHOLDER).map_or(0, |re| re.find_iter(text).count())
}

/// FNV-1a, 64 bits: the same in every build.
fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn secret_env_name(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    !ENV_NOT_SECRET.contains(&upper.as_str())
        && (ENV_NAMES.contains(&upper.as_str()) || ENV_SUFFIXES.iter().any(|s| upper.ends_with(s)))
}

/// The kind of an assignment by its name: `password`, `token`, `secret`.
fn assignment_kind(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if ["password", "passwd", "pwd"]
        .iter()
        .any(|w| lower.contains(w))
    {
        "password"
    } else if lower.contains("token") {
        "token"
    } else if lower.contains("key") {
        "key"
    } else {
        "secret"
    }
}

/// A name, a reference or a stand-in rather than a value: `MY_KEY`,
/// `$TOKEN`, `${{ secrets.X }}`, `<your-key>`, `xxxx`, `changeme`.
fn looks_like_reference(value: &str) -> bool {
    let v = value.trim();
    let lower = v.to_ascii_lowercase();
    v.starts_with('$')
        || v.starts_with('<')
        || v.starts_with('{')
        || v.starts_with('%')
        || v.contains("${")
        || v.contains("{{")
        || v.chars()
            .all(|c| c == '*' || c == 'x' || c == 'X' || c == '.')
        || [
            "changeme",
            "password",
            "secret",
            "example",
            "your",
            "redacted",
            "placeholder",
            "dummy",
            "null",
            "none",
            "true",
            "false",
        ]
        .iter()
        .any(|w| lower.contains(w))
        || v.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

/// Whether an assignment value looks random enough to be a secret.
fn looks_random(value: &str) -> bool {
    if value.len() < MIN_ASSIGNED || looks_like_reference(value) {
        return false;
    }
    if value.contains("://")
        || value.starts_with('/')
        || value.starts_with("./")
        || value.starts_with('~')
        || value.contains("[secret:")
    {
        return false;
    }
    let letters = value.chars().any(|c| c.is_ascii_alphabetic());
    let digits = value.chars().any(|c| c.is_ascii_digit());
    letters && digits && entropy(value) >= MIN_ENTROPY
}

/// Shannon entropy in bits per character.
fn entropy(text: &str) -> f64 {
    let mut counts: HashMap<char, usize> = HashMap::new();
    let mut total = 0usize;
    for c in text.chars() {
        *counts.entry(c).or_default() += 1;
        total += 1;
    }
    if total == 0 {
        return 0.0;
    }
    let total = total as f64;
    counts
        .values()
        .map(|&n| {
            let p = n as f64 / total;
            -p * p.log2()
        })
        .sum()
}
