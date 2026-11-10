//! Copied text to the clipboard (see `spec/tui.md`, "Copy"): the terminal's
//! OSC 52 sequence, which also works over SSH, and a clipboard command
//! (`wl-copy`, `xclip`, `xsel`, `pbcopy`) when the session is local. No
//! crate: OSC 52 is one escape sequence, and the commands are processes.

use std::io::Write;
use std::path::Path;

use crate::config::Clipboard;

/// The most text OSC 52 carries: many terminals refuse longer sequences.
pub const OSC52_MAX: usize = 100_000;

/// How a copy goes out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Plan {
    pub osc52: bool,
    /// The command and its arguments, which read the text on standard input.
    pub command: Option<Vec<String>>,
}

impl Plan {
    /// For `tui.clipboard`: `getenv` reads the environment, `found` says
    /// whether a program is on `PATH`.
    pub fn new(
        mode: Clipboard,
        getenv: impl Fn(&str) -> Option<String>,
        found: impl Fn(&str) -> bool,
    ) -> Plan {
        let command = || command(&getenv, &found);
        match mode {
            Clipboard::Off => Plan {
                osc52: false,
                command: None,
            },
            Clipboard::Osc52 => Plan {
                osc52: true,
                command: None,
            },
            Clipboard::Command => Plan {
                osc52: false,
                command: command(),
            },
            Clipboard::Auto => Plan {
                osc52: true,
                // Over SSH the command would fill the remote clipboard.
                command: if remote(&getenv) { None } else { command() },
            },
        }
    }
}

fn remote(getenv: &impl Fn(&str) -> Option<String>) -> bool {
    ["SSH_TTY", "SSH_CONNECTION"]
        .iter()
        .any(|name| getenv(name).is_some_and(|v| !v.is_empty()))
}

/// The clipboard command of the desktop, if one is installed.
fn command(
    getenv: &impl Fn(&str) -> Option<String>,
    found: &impl Fn(&str) -> bool,
) -> Option<Vec<String>> {
    let set = |name: &str| getenv(name).is_some_and(|v| !v.is_empty());
    let candidates: [(bool, &[&str]); 4] = [
        (set("WAYLAND_DISPLAY"), &["wl-copy"]),
        (set("DISPLAY"), &["xclip", "-selection", "clipboard"]),
        (set("DISPLAY"), &["xsel", "--clipboard", "--input"]),
        (cfg!(target_os = "macos"), &["pbcopy"]),
    ];
    candidates
        .iter()
        .find(|(usable, words)| *usable && words.first().is_some_and(|w| found(w)))
        .map(|(_, words)| words.iter().map(|w| (*w).to_owned()).collect())
}

/// Whether `name` is an executable file in a directory of `PATH`.
pub fn on_path(name: &str) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        Path::new(&dir)
            .join(name)
            .metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let symbol = |n: u32| char::from(BASE64.get((n & 63) as usize).copied().unwrap_or(b'A'));
    for chunk in bytes.chunks(3) {
        let b = |i: usize| u32::from(chunk.get(i).copied().unwrap_or(0));
        let n = (b(0) << 16) | (b(1) << 8) | b(2);
        out.push(symbol(n >> 18));
        out.push(symbol(n >> 12));
        out.push(if chunk.len() > 1 { symbol(n >> 6) } else { '=' });
        out.push(if chunk.len() > 2 { symbol(n) } else { '=' });
    }
    out
}

/// The text, cut at [`OSC52_MAX`] bytes on a character boundary, and
/// whether it was cut.
pub fn cut(text: &str) -> (&str, bool) {
    if text.len() <= OSC52_MAX {
        return (text, false);
    }
    let mut end = OSC52_MAX;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text.get(..end).unwrap_or_default(), true)
}

/// The OSC 52 sequence that puts `text` in the clipboard.
pub fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

/// Carries out a copy; the error says what failed. The command runs in
/// the background, so a slow one does not hold the screen.
pub fn copy(plan: &Plan, text: &str) -> Result<(), String> {
    if plan.osc52 {
        let (text, _) = cut(text);
        let mut out = std::io::stdout();
        out.write_all(osc52(text).as_bytes())
            .and_then(|()| out.flush())
            .map_err(|e| format!("the terminal: {e}"))?;
    }
    match &plan.command {
        Some(words) => {
            let Some((program, args)) = words.split_first() else {
                return Ok(());
            };
            let mut child = tokio::process::Command::new(program)
                .args(args)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .map_err(|e| format!("{program}: {e}"))?;
            let text = text.to_owned();
            let program = program.clone();
            tokio::spawn(async move {
                use tokio::io::AsyncWriteExt;
                if let Some(mut stdin) = child.stdin.take() {
                    if let Err(e) = stdin.write_all(text.as_bytes()).await {
                        tracing::info!("{program}: {e}");
                    }
                    drop(stdin);
                }
                if let Err(e) = child.wait().await {
                    tracing::info!("{program}: {e}");
                }
            });
            Ok(())
        }
        None if !plan.osc52 => Err("no clipboard command found (wl-copy, xclip, xsel)".into()),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
        move |name| {
            vars.iter()
                .find(|(n, _)| *n == name)
                .map(|(_, v)| (*v).to_owned())
        }
    }

    #[test]
    fn the_sequence() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"f"), "Zg==");
        assert_eq!(base64(b"fo"), "Zm8=");
        assert_eq!(base64(b"foo"), "Zm9v");
        assert_eq!(base64("hé!".as_bytes()), "aMOpIQ==");
        assert_eq!(osc52("hi"), "\x1b]52;c;aGk=\x07");
    }

    #[test]
    fn long_text_is_cut_on_a_character() {
        let text = format!("{}é", "a".repeat(OSC52_MAX - 1));
        let (cut_text, was_cut) = cut(&text);
        assert!(was_cut);
        assert_eq!(cut_text.len(), OSC52_MAX - 1);
        assert_eq!(cut("short"), ("short", false));
    }

    #[test]
    fn auto_uses_a_command_only_locally() {
        let wayland = [("WAYLAND_DISPLAY", "wayland-0")];
        let all = |_: &str| true;
        let plan = Plan::new(Clipboard::Auto, env(&wayland), all);
        assert!(plan.osc52);
        assert_eq!(plan.command, Some(vec!["wl-copy".to_owned()]));
        let ssh = [("WAYLAND_DISPLAY", "wayland-0"), ("SSH_TTY", "/dev/pts/1")];
        let plan = Plan::new(Clipboard::Auto, env(&ssh), all);
        assert_eq!(plan.command, None);
        assert!(plan.osc52);
    }

    #[test]
    fn the_command_is_one_that_is_installed() {
        let x11 = [("DISPLAY", ":0")];
        let only_xsel = |name: &str| name == "xsel";
        let plan = Plan::new(Clipboard::Command, env(&x11), only_xsel);
        assert!(!plan.osc52);
        assert_eq!(plan.command.unwrap(), ["xsel", "--clipboard", "--input"]);
        let none = Plan::new(Clipboard::Command, env(&[]), |_: &str| true);
        assert_eq!(none.command, None);
        assert!(copy(&none, "x").is_err());
        let off = Plan::new(Clipboard::Off, env(&x11), |_: &str| true);
        assert_eq!(
            off,
            Plan {
                osc52: false,
                command: None
            }
        );
    }

    #[test]
    fn a_fake_command_on_path_gets_the_text() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("out");
        let script = dir.path().join("wl-copy");
        std::fs::write(&script, format!("#!/bin/sh\ncat > '{}'\n", out.display())).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let found = |name: &str| dir.path().join(name).is_file();
        let mut plan = Plan::new(Clipboard::Command, env(&[("WAYLAND_DISPLAY", "w")]), found);
        assert_eq!(plan.command, Some(vec!["wl-copy".to_owned()]));
        plan.command = Some(vec![script.display().to_string()]);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            copy(&plan, "copied text").unwrap();
            for _ in 0..200 {
                if std::fs::read_to_string(&out).is_ok_and(|t| t == "copied text") {
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            panic!("the command did not get the text");
        });
    }
}
