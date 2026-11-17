//! Copied text to the clipboard (see `spec/tui.md`, "Copy"): the terminal's
//! OSC 52 sequence, which also works over SSH, and a clipboard command
//! (`wl-copy`, `xclip`, `xsel`, `pbcopy`) when the session is local. No
//! crate: OSC 52 is one escape sequence, and the commands are processes.
//!
//! `Ctrl-V` reads the clipboard the same way (RFC 0034): the terminal passes
//! only text, so an image comes from `wl-paste`, `xclip` or `osascript`.

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

/// Whether `name` is an executable file in a directory of `path` (`PATH`).
pub fn on_path(name: &str, path: Option<&std::ffi::OsStr>) -> bool {
    use std::os::unix::fs::PermissionsExt;
    let Some(path) = path else {
        return false;
    };
    std::env::split_paths(path).any(|dir| {
        Path::new(&dir)
            .join(name)
            .metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    })
}

use crate::image::base64;

/// What `Ctrl-V` found in the clipboard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Clip {
    Image(Vec<u8>),
    Text(String),
    Empty,
}

/// The program that reads the clipboard of this desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reader {
    Wayland,
    X11,
    MacOs,
}

/// The image types taken, best first.
const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/webp", "image/gif"];

/// How long one clipboard program may take.
const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// The reader for `tui.clipboard`, or why the clipboard cannot be read.
pub fn reader(
    mode: Clipboard,
    getenv: impl Fn(&str) -> Option<String>,
    found: impl Fn(&str) -> bool,
) -> Result<Reader, String> {
    match mode {
        Clipboard::Off | Clipboard::Osc52 => {
            return Err(format!(
                "tui.clipboard = \"{}\" does not read the clipboard; use /image PATH",
                mode.name()
            ));
        }
        Clipboard::Auto if remote(&getenv) => {
            return Err("no access to the local clipboard over SSH; use /image PATH".to_owned());
        }
        Clipboard::Auto | Clipboard::Command => {}
    }
    let set = |name: &str| getenv(name).is_some_and(|v| !v.is_empty());
    if set("WAYLAND_DISPLAY") && found("wl-paste") {
        Ok(Reader::Wayland)
    } else if set("DISPLAY") && found("xclip") {
        Ok(Reader::X11)
    } else if cfg!(target_os = "macos") && found("osascript") {
        Ok(Reader::MacOs)
    } else {
        Err("install wl-clipboard or xclip to paste images".to_owned())
    }
}

/// Where `name` is in `path`, as an absolute path.
fn which(name: &str, path: Option<&std::ffi::OsStr>) -> Option<std::path::PathBuf> {
    std::env::split_paths(path?)
        .map(|dir| dir.join(name))
        .find(|p| on_path(name, p.parent().map(Path::as_os_str)))
}

/// Runs a clipboard program; its standard output, or `None` when it fails
/// (an empty clipboard makes `wl-paste` fail).
async fn output(
    path: Option<&std::ffi::OsStr>,
    program: &str,
    args: &[&str],
) -> Result<Option<Vec<u8>>, String> {
    let exe = which(program, path).ok_or_else(|| format!("{program}: not found"))?;
    let run = tokio::process::Command::new(exe)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(READ_TIMEOUT, run).await {
        Err(_) => Err(format!("{program} did not answer within 2 s")),
        Ok(Err(e)) => Err(format!("{program}: {e}")),
        Ok(Ok(out)) if out.status.success() => Ok(Some(out.stdout)),
        Ok(Ok(_)) => Ok(None),
    }
}

/// The first image type of a list of types, one per line.
fn image_type(types: &[u8]) -> Option<&'static str> {
    let types = String::from_utf8_lossy(types);
    let listed: Vec<&str> = types.lines().map(str::trim).collect();
    IMAGE_TYPES.into_iter().find(|t| listed.contains(t))
}

fn text_clip(bytes: Option<Vec<u8>>) -> Clip {
    match bytes {
        Some(bytes) if !bytes.is_empty() => {
            Clip::Text(String::from_utf8_lossy(&bytes).into_owned())
        }
        _ => Clip::Empty,
    }
}

/// Reads the clipboard: an image if it has one, else its text. `path` is
/// `PATH`; `temp` a directory for the file `osascript` writes.
pub async fn read(
    reader: Reader,
    path: Option<&std::ffi::OsStr>,
    temp: &Path,
) -> Result<Clip, String> {
    match reader {
        Reader::Wayland => {
            let types = output(path, "wl-paste", &["--list-types"]).await?;
            match types.as_deref().and_then(image_type) {
                Some(t) => Ok(output(path, "wl-paste", &["--no-newline", "--type", t])
                    .await?
                    .map_or(Clip::Empty, Clip::Image)),
                None if types.is_some() => Ok(text_clip(
                    output(path, "wl-paste", &["--no-newline"]).await?,
                )),
                None => Ok(Clip::Empty),
            }
        }
        Reader::X11 => {
            let clipboard = ["-selection", "clipboard"];
            let targets = output(
                path,
                "xclip",
                &[&clipboard[..], &["-t", "TARGETS", "-o"]].concat(),
            )
            .await?;
            match targets.as_deref().and_then(image_type) {
                Some(t) => Ok(
                    output(path, "xclip", &[&clipboard[..], &["-t", t, "-o"]].concat())
                        .await?
                        .map_or(Clip::Empty, Clip::Image),
                ),
                None => Ok(text_clip(
                    output(path, "xclip", &[&clipboard[..], &["-o"]].concat()).await?,
                )),
            }
        }
        Reader::MacOs => {
            let info = output(path, "osascript", &["-e", "clipboard info"]).await?;
            let has_png = info
                .as_deref()
                .is_some_and(|i| String::from_utf8_lossy(i).contains("PNGf"));
            if !has_png {
                return Ok(text_clip(output(path, "pbpaste", &[]).await?));
            }
            let file = temp.join(format!("codeagent-paste-{}.png", std::process::id()));
            let script = [
                format!(
                    "set f to open for access POSIX file \"{}\" with write permission",
                    file.display()
                ),
                "write (the clipboard as «class PNGf») to f".to_owned(),
                "close access f".to_owned(),
            ];
            let args: Vec<&str> = script.iter().flat_map(|s| ["-e", s.as_str()]).collect();
            let written = output(path, "osascript", &args).await;
            let bytes = std::fs::read(&file);
            let _ = std::fs::remove_file(&file);
            written?;
            bytes
                .map(Clip::Image)
                .map_err(|e| format!("reading the pasted image: {e}"))
        }
    }
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
        if cfg!(target_os = "macos") {
            // macOS always has `pbcopy`.
            assert_eq!(none.command, Some(vec!["pbcopy".to_owned()]));
        } else {
            assert_eq!(none.command, None);
            assert!(copy(&none, "x").is_err());
        }
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

    /// A directory of fake clipboard programs: each prints its file.
    fn fake(programs: &[(&str, &str)]) -> tempfile::TempDir {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        for (name, script) in programs {
            let path = dir.path().join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        dir
    }

    #[test]
    fn the_reader_of_the_desktop() {
        let all = |_: &str| true;
        let wayland = [("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0")];
        assert_eq!(
            reader(Clipboard::Auto, env(&wayland), all),
            Ok(Reader::Wayland)
        );
        assert_eq!(
            reader(Clipboard::Auto, env(&wayland), |n| n != "wl-paste"),
            Ok(Reader::X11)
        );
        let ssh = [("DISPLAY", ":0"), ("SSH_TTY", "/dev/pts/1")];
        let e = reader(Clipboard::Auto, env(&ssh), all).unwrap_err();
        assert!(e.contains("over SSH"), "{e}");
        assert_eq!(reader(Clipboard::Command, env(&ssh), all), Ok(Reader::X11));
        let e = reader(Clipboard::Osc52, env(&wayland), all).unwrap_err();
        assert!(e.contains("tui.clipboard = \"osc52\""), "{e}");
        if !cfg!(target_os = "macos") {
            let e = reader(Clipboard::Auto, env(&[]), all).unwrap_err();
            assert_eq!(e, "install wl-clipboard or xclip to paste images");
        }
    }

    #[tokio::test]
    async fn wayland_images_text_and_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let image = fake(&[(
            "wl-paste",
            r#"case "$1" in --list-types) printf 'text/plain\nimage/jpeg\nimage/png\n';; *) [ "$3" = image/png ] && printf PNGDATA;; esac"#,
        )]);
        let path = image.path().as_os_str();
        assert_eq!(
            read(Reader::Wayland, Some(path), tmp.path()).await,
            Ok(Clip::Image(b"PNGDATA".to_vec()))
        );
        let text = fake(&[(
            "wl-paste",
            r#"case "$1" in --list-types) echo text/plain;; *) printf 'some text';; esac"#,
        )]);
        assert_eq!(
            read(Reader::Wayland, Some(text.path().as_os_str()), tmp.path()).await,
            Ok(Clip::Text("some text".into()))
        );
        let empty = fake(&[("wl-paste", "echo 'Nothing is copied' >&2; exit 1")]);
        assert_eq!(
            read(Reader::Wayland, Some(empty.path().as_os_str()), tmp.path()).await,
            Ok(Clip::Empty)
        );
    }

    #[tokio::test]
    async fn x11_and_a_slow_program() {
        let tmp = tempfile::tempdir().unwrap();
        let image = fake(&[(
            "xclip",
            r#"if [ "$4" = TARGETS ]; then printf 'TARGETS\nimage/gif\n'; elif [ "$4" = image/gif ]; then printf GIF; fi"#,
        )]);
        assert_eq!(
            read(Reader::X11, Some(image.path().as_os_str()), tmp.path()).await,
            Ok(Clip::Image(b"GIF".to_vec()))
        );
        let slow = fake(&[("xclip", "sleep 5")]);
        let e = read(Reader::X11, Some(slow.path().as_os_str()), tmp.path())
            .await
            .unwrap_err();
        assert_eq!(e, "xclip did not answer within 2 s");
        let missing = tempfile::tempdir().unwrap();
        let e = read(Reader::X11, Some(missing.path().as_os_str()), tmp.path())
            .await
            .unwrap_err();
        assert_eq!(e, "xclip: not found");
    }
}
