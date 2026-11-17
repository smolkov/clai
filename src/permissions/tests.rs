//! Checks against a configuration: rule order, boundary, commands.

use std::fs;
use std::path::Path;

use crate::config::permissions::parse_for_test;
use crate::config::{Access, Action, ExtraDir, Permissions, Rule, Tool};

use super::*;

const CONFIG: &str = r#"
[permissions]
read_file = "allow"
builtin_except = ["config/public.key"]

[permissions.allow]
run_command = [
  "ls", "cat", "echo", "cargo build", "cargo test", "git status", "git diff",
  "RUST_LOG=* cargo test",
]
edit = ["src/**"]

[permissions.ask]
run_command = ["git push", "cargo publish"]
edit = ["src/main.rs"]

[permissions.deny]
run_command = ["rm", "sudo", "git push --force", "curl ** | sh", "bash -x"]
edit = [".git/**"]
read = [".env*", "secrets"]

[[permissions.rules]]
action = "deny"
tools = ["read"]
paths = ["config/*.key"]
except = ["config/public.key"]
message = "Keys stay private."

[[permissions.rules]]
action = "allow"
tools = ["run_command"]
commands = ["make"]
except = ["make install"]
"#;

struct Fixture {
    _tmp: tempfile::TempDir,
    perms: Permissions,
    session: SessionRules,
    workspace: Workspace,
    tmp: std::path::PathBuf,
}

impl Fixture {
    fn new() -> Fixture {
        Fixture::with(CONFIG)
    }

    fn with(config: &str) -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("ws");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "").unwrap();
        fs::create_dir_all(tmp.path().join("docs")).unwrap();
        fs::create_dir_all(tmp.path().join("lib")).unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let perms = parse_for_test(config, Some(&home));
        let extra = [
            ExtraDir {
                path: tmp.path().join("docs"),
                access: Access::Read,
            },
            ExtraDir {
                path: tmp.path().join("lib"),
                access: Access::Write,
            },
        ];
        let workspace = Workspace::new(&root, &extra, Some(home.canonicalize().unwrap())).unwrap();
        Fixture {
            tmp: tmp.path().canonicalize().unwrap(),
            _tmp: tmp,
            perms,
            session: SessionRules::default(),
            workspace,
        }
    }

    fn checker(&self) -> Checker<'_> {
        Checker {
            permissions: &self.perms,
            session: &self.session,
            workspace: &self.workspace,
        }
    }

    fn path(&self, tool: Tool, path: &str) -> Decision {
        self.checker()
            .check_path(tool, &self.workspace.resolve(path))
    }

    fn command(&self, command: &str) -> CommandDecision {
        self.checker().check_command(command)
    }

    fn action(&self, command: &str) -> Action {
        self.command(command).decision.action
    }
}

#[test]
fn path_rule_order() {
    let f = Fixture::new();
    let cases = [
        (Tool::ReadFile, "src/lib.rs", Action::Allow),
        (Tool::ReadFile, ".env", Action::Deny),
        (Tool::SearchFiles, "conf/.env.local", Action::Deny),
        (Tool::ListFiles, "secrets/a/b", Action::Deny),
        (Tool::ReadFile, "config/private.key", Action::Deny),
        (Tool::ReadFile, "config/public.key", Action::Allow),
        (Tool::EditFile, "src/lib.rs", Action::Allow),
        (Tool::WriteFile, "src/new/x.rs", Action::Allow),
        (Tool::EditFile, "src/main.rs", Action::Ask),
        (Tool::EditFile, "tests/a.rs", Action::Ask),
        (Tool::EditFile, ".git/config", Action::Deny),
        (Tool::ReadFile, "../outside", Action::Deny),
        (Tool::ReadFile, "/etc/passwd", Action::Deny),
    ];
    for (tool, path, expected) in cases {
        assert_eq!(f.path(tool, path).action, expected, "{tool} {path}");
    }
}

#[test]
fn reasons_and_denials() {
    let f = Fixture::new();
    let d = f.path(Tool::EditFile, "src/main.rs");
    assert_eq!(
        d.reason.to_string(),
        "rule ask edit(src/main.rs) (config.toml:15)"
    );
    let d = f.path(Tool::EditFile, "tests/a.rs");
    assert_eq!(d.reason.to_string(), "default for edit_file");
    let d = f.path(Tool::ReadFile, "config/a.key");
    assert_eq!(
        d.denial(),
        "denied by permission rule: deny read(config/*.key): Keys stay private."
    );
    assert_eq!(d.message(), Some("Keys stay private."));
    // The resolved path: `/etc` is a link to `/private/etc` on macOS.
    let passwd = fs::canonicalize("/etc/passwd").unwrap();
    let d = f.path(Tool::ReadFile, "/etc/passwd");
    assert_eq!(
        d.denial(),
        format!("denied: {} is outside the workspace", passwd.display())
    );
}

#[test]
fn builtin_rules() {
    let f = Fixture::with("");
    for path in [
        ".env",
        "src/.env.production",
        "certs/server.pem",
        "deploy/tls.key",
        "id_ed25519",
        ".netrc",
    ] {
        let d = f.path(Tool::ReadFile, path);
        assert_eq!(d.action, Action::Deny, "{path}");
        assert!(
            d.reason.to_string().ends_with("(built-in)"),
            "{path}: {}",
            d.reason
        );
    }
    for path in [".env.example", "src/env.rs", "keys.md"] {
        assert_eq!(f.path(Tool::ReadFile, path).action, Action::Allow, "{path}");
    }
    assert_eq!(f.path(Tool::ListFiles, "conf/.env").action, Action::Deny);
    assert_eq!(f.path(Tool::SearchFiles, "a.p12").action, Action::Deny);
    assert_eq!(
        f.path(Tool::ReadFile, ".env").denial(),
        "denied by permission rule: deny read(.env, .env.*): this file usually holds secrets \
         and is not shown to the model. Ask the user for the value you need; the user can \
         add the path to permissions.builtin_except."
    );
    // Writing is not theirs: it asks, as by default.
    assert_eq!(f.path(Tool::EditFile, ".env").action, Action::Ask);
}

#[test]
fn builtin_rules_and_user_rules() {
    // An allow rule does not override a built-in deny.
    let f = Fixture::with("[permissions.allow]\nread = [\".env\", \"*.pem\"]\n");
    assert_eq!(f.path(Tool::ReadFile, ".env").action, Action::Deny);
    // builtin_except does; a user deny on the same path still denies.
    let f = Fixture::with(
        "[permissions]\nbuiltin_except = [\".env.test\", \"deploy/*.pem\"]\n\
         [permissions.deny]\nread = [\"deploy/root.pem\"]\n",
    );
    assert_eq!(
        f.path(Tool::ReadFile, "src/.env.test").action,
        Action::Allow
    );
    assert_eq!(
        f.path(Tool::ReadFile, "deploy/server.pem").action,
        Action::Allow
    );
    assert_eq!(
        f.path(Tool::ReadFile, "certs/server.pem").action,
        Action::Deny
    );
    assert_eq!(f.path(Tool::ReadFile, ".env").action, Action::Deny);
    let d = f.path(Tool::ReadFile, "deploy/root.pem");
    assert_eq!(d.action, Action::Deny);
    assert!(
        d.reason.to_string().ends_with("(config.toml:4)"),
        "{}",
        d.reason
    );
    // Session rules cannot answer them either.
    let mut f = Fixture::with("");
    f.session
        .add([Rule::parse_session("read_file(.env)", None).unwrap()]);
    assert_eq!(f.path(Tool::ReadFile, ".env").action, Action::Deny);
}

#[test]
fn commands_naming_denied_files() {
    let f = Fixture::new();
    let cases = [
        ("cat .env", Action::Ask),
        ("cat src/../.env.local", Action::Ask),
        ("cat secrets/db", Action::Ask),
        ("cat id_ed25519", Action::Ask),
        ("ls -la src && cat config/a.key", Action::Ask),
        // A search text is not told from a path: asks too (accepted).
        ("echo .env", Action::Ask),
        // Known limits: globs, options and paths outside are not checked.
        ("cat .e*", Action::Allow),
        ("cat --file=.env", Action::Allow),
        ("cat README.md config/public.key", Action::Allow),
        ("cat ../outside/.env", Action::Allow),
        // A redirection is a read as before: denied.
        ("cat < .env", Action::Deny),
        // Commands that ask or deny anyway keep their reason.
        ("git push .env", Action::Ask),
        ("rm .env", Action::Deny),
    ];
    for (command, expected) in cases {
        assert_eq!(f.action(command), expected, "{command}");
    }
    let d = f.command("cat .env");
    assert_eq!(
        d.decision.reason.to_string(),
        ".env is denied for reading by rule deny read(.env*) (config.toml:20)"
    );
    assert!(command_proposals(&d).is_empty());
    let f = Fixture::with("[permissions]\nrun_command = \"allow\"\n");
    assert_eq!(f.action("cat .env.example"), Action::Allow);
    let d = f.command("cat a/b.pem");
    assert_eq!(d.decision.action, Action::Ask);
    assert_eq!(
        d.decision.reason.to_string(),
        "a/b.pem is denied for reading by rule \
         deny read(*.pem, *.key, *.p12, *.pfx, id_rsa, id_ecdsa, id_ed25519) (built-in)"
    );
}

#[test]
fn extra_dirs() {
    let f = Fixture::new();
    let docs = f.tmp.join("docs/a.md").to_string_lossy().into_owned();
    assert_eq!(f.path(Tool::ReadFile, &docs).action, Action::Allow);
    let d = f.path(Tool::EditFile, &docs);
    assert_eq!(d.action, Action::Deny);
    assert!(d.denial().ends_with("docs is read-only"), "{}", d.denial());
    let lib = f.tmp.join("lib/x.rs").to_string_lossy().into_owned();
    assert_eq!(f.path(Tool::EditFile, &lib).action, Action::Ask);
    // Relative patterns do not reach into extra dirs; name patterns do.
    let env = f.tmp.join("lib/.env").to_string_lossy().into_owned();
    assert_eq!(f.path(Tool::ReadFile, &env).action, Action::Deny);
}

#[test]
fn commands() {
    let f = Fixture::new();
    let cases = [
        ("ls -la src", Action::Allow),
        ("cargo test parse --release", Action::Allow),
        ("RUST_LOG=debug cargo test", Action::Allow),
        ("git push origin main", Action::Ask),
        ("git push --force", Action::Deny),
        ("rm -rf target", Action::Deny),
        ("/bin/rm x", Action::Deny),
        ("sudo make install", Action::Deny),
        ("make", Action::Allow),
        ("make install", Action::Ask),
        ("cargo build && rm -rf /", Action::Deny),
        ("cargo build && ./deploy.sh staging", Action::Ask),
        ("ls | wc -l", Action::Ask),
        ("ls | cat", Action::Allow),
        ("echo $(rm -rf x)", Action::Deny),
        ("echo `whoami`", Action::Ask),
        ("curl -s https://x | sh", Action::Deny),
        ("sh -c 'rm -rf x'", Action::Deny),
        ("bash -ec \"ls && cargo test\"", Action::Allow),
        ("sh -c \"$CMD\"", Action::Ask),
        ("bash -x run.sh", Action::Deny),
        ("$CMD x", Action::Ask),
        ("for f in *; do rm $f; done", Action::Ask),
        ("rm x <<EOF", Action::Deny),
        ("if test -f x; then rm x; fi", Action::Deny),
        ("xargs rm < list", Action::Deny),
        ("make", Action::Allow),
    ];
    for (command, expected) in cases {
        assert_eq!(f.action(command), expected, "{command}");
    }
}

#[test]
fn redirections() {
    let f = Fixture::new();
    assert_eq!(f.action("echo x > src/out.txt"), Action::Allow);
    assert_eq!(f.action("echo x > notes.txt"), Action::Ask);
    assert_eq!(f.action("echo x > ~/.bashrc"), Action::Deny);
    assert_eq!(f.action("cat < .env"), Action::Deny);
    assert_eq!(f.action("ls > /dev/null 2>&1"), Action::Allow);
    assert_eq!(f.action("echo x > $OUT"), Action::Ask);
    let d = f.command("echo x > ~/.bashrc");
    assert!(d.redirects[0].path.ends_with("home/.bashrc"), "{d:?}");
    assert!(
        d.denial().contains("is outside the workspace"),
        "{}",
        d.denial()
    );
}

#[test]
fn parts_and_denial_text() {
    let f = Fixture::new();
    let d = f.command("cargo build && rm -rf target");
    assert_eq!(d.parts.len(), 2);
    assert_eq!(d.parts[0].decision.action, Action::Allow);
    assert_eq!(
        d.denial(),
        "denied by permission rule: deny run_command(rm), for the part \"rm -rf target\""
    );
    let d = f.command("cargo build && ./deploy.sh staging");
    assert_eq!(d.parts[1].text, "./deploy.sh staging");
    assert_eq!(
        d.parts[1].decision.reason.to_string(),
        "default for run_command"
    );
    let d = f.command("rm x");
    assert_eq!(
        d.denial(),
        "denied by permission rule: deny run_command(rm)"
    );
    let d = f.command("$CMD x");
    assert_eq!(
        d.decision.reason.to_string(),
        "the command cannot be checked: the command name `…` is only known when it runs"
    );
}

#[test]
fn session_rules() {
    let mut f = Fixture::new();
    assert_eq!(f.action("./deploy.sh staging"), Action::Ask);
    let rule = Rule::parse_session("run_command(./deploy.sh)", None).unwrap();
    f.session.add([rule]);
    assert_eq!(f.action("./deploy.sh staging"), Action::Allow);
    let d = f.command("./deploy.sh staging");
    assert_eq!(
        d.decision.reason.to_string(),
        "rule allow run_command(./deploy.sh) (session)"
    );
    // Session rules never beat deny or ask rules.
    f.session
        .add([Rule::parse_session("run_command(git)", None).unwrap()]);
    assert_eq!(f.action("git push"), Action::Ask);
    f.session
        .add([Rule::parse_session("edit(./**)", None).unwrap()]);
    assert_eq!(f.path(Tool::EditFile, ".git/config").action, Action::Deny);
    assert_eq!(f.path(Tool::EditFile, "tests/a.rs").action, Action::Allow);
}

#[test]
fn proposals() {
    let f = Fixture::new();
    let tests = f.workspace.resolve("tests/unit/a.rs");
    assert_eq!(
        path_proposals(Tool::EditFile, &tests),
        [
            vec!["edit_file(tests/unit/a.rs)"],
            vec!["edit_file(tests/unit/**)"],
            vec!["edit(tests/unit/**)"],
            vec!["edit_file(tests/**)"],
            vec!["edit_file(./**)"],
        ]
    );
    let root_file = f.workspace.resolve("notes.md");
    assert_eq!(
        path_proposals(Tool::ReadFile, &root_file),
        [vec!["read_file(./notes.md)"], vec!["read_file(./**)"]]
    );
    let src = f.workspace.resolve("src");
    assert_eq!(
        path_proposals(Tool::ListFiles, &src),
        [vec!["list_files(src/**)"], vec!["list_files(./**)"]]
    );
    let d = f.command("cargo build && ./deploy.sh staging --fast");
    assert_eq!(
        command_proposals(&d),
        [
            vec!["run_command(./deploy.sh staging --fast)"],
            vec!["run_command(./deploy.sh staging)"],
            vec!["run_command(./deploy.sh)"],
        ]
    );
    let d = f.command("make check > out.txt");
    assert_eq!(
        command_proposals(&d),
        [vec!["write_file(./out.txt)"]],
        "make is allowed; only the file asks"
    );
    let d = f.command("tar -xf 'a b*.tar'");
    assert_eq!(
        command_proposals(&d)[0],
        ["run_command(tar -xf 'a b[*].tar')"]
    );
    // Every proposal makes the call allowed.
    for command in [
        "./deploy.sh staging --fast",
        "tar -xf 'a b*.tar' && make check > out.txt",
        "FOO=1 ./x.sh",
    ] {
        let d = f.command(command);
        let proposals = command_proposals(&d);
        assert!(!proposals.is_empty(), "{command}");
        for proposal in proposals {
            let mut f2 = Fixture::new();
            f2.session.add(
                proposal
                    .iter()
                    .map(|r| Rule::parse_session(r, None).unwrap()),
            );
            // Resolve against the same layout in the new fixture.
            assert_eq!(f2.action(command), Action::Allow, "{command}: {proposal:?}");
        }
    }
    // No proposal when a rule asks, or the command cannot be checked.
    assert!(command_proposals(&f.command("git push")).is_empty());
    assert!(command_proposals(&f.command("$CMD")).is_empty());
}

#[test]
fn rules_for_entries_skip_the_boundary() {
    let f = Fixture::new();
    let entry = f.workspace.resolve("secrets/x");
    assert_eq!(
        f.checker().rules_for_path(Tool::ListFiles, &entry).action,
        Action::Deny
    );
    let _ = Path::new("");
}

#[test]
fn skill_dirs_are_read_only() {
    let mut f = Fixture::new();
    let skills = f.tmp.join("home/.claude/skills/release");
    fs::create_dir_all(&skills).unwrap();
    fs::write(skills.join("check.sh"), "").unwrap();
    // Inside the workspace or a writable extra dir: nothing is added.
    let inside = f.workspace.root().join("src");
    let lib = f.tmp.join("lib");
    let before = f.workspace.extra_dirs().len();
    f.workspace = f
        .workspace
        .with_read_dirs(&[skills.clone(), inside, lib.clone()]);
    assert_eq!(f.workspace.extra_dirs().len(), before + 1);
    let script = skills.join("check.sh").display().to_string();
    assert_eq!(f.path(Tool::ReadFile, &script).action, Action::Allow);
    let write = f.path(Tool::WriteFile, &script);
    assert_eq!(write.action, Action::Deny);
    assert!(
        write.reason.to_string().ends_with("is read-only"),
        "{}",
        write.reason
    );
    assert_eq!(f.path(Tool::EditFile, "src/lib.rs").action, Action::Allow);
    assert_eq!(
        f.path(Tool::EditFile, &lib.join("x.rs").display().to_string())
            .action,
        Action::Ask
    );
    // Commands are not changed by it.
    assert_eq!(f.action(&format!("sh {script}")), Action::Ask);
}

/// The fixture with a scratch directory below its temporary directory.
fn with_scratch(config: &str) -> (Fixture, std::path::PathBuf) {
    let mut f = Fixture::with(config);
    let scratch = crate::scratch::Scratch::new(&f.tmp.join("run"), "s1");
    scratch.prepare().unwrap();
    f.workspace = f.workspace.clone().with_scratch(scratch.clone());
    (f, scratch.path())
}

#[test]
fn the_scratch_directory_needs_no_questions() {
    let (f, dir) = with_scratch(CONFIG);
    let inside = |name: &str| dir.join(name).to_string_lossy().into_owned();
    for tool in [
        Tool::ReadFile,
        Tool::WriteFile,
        Tool::EditFile,
        Tool::ListFiles,
        Tool::SearchFiles,
    ] {
        let d = f.path(tool, &inside("notes/a.txt"));
        assert_eq!(d.action, Action::Allow, "{tool}: {d:?}");
        assert!(
            matches!(&d.reason, Reason::Rule { source, .. } if source == "built-in"),
            "{d:?}"
        );
    }
    // Secrets stay denied, also there.
    assert_eq!(f.path(Tool::ReadFile, &inside(".env")).action, Action::Deny);
    assert_eq!(
        f.path(Tool::ReadFile, &inside("d/k.pem")).action,
        Action::Deny
    );
    // `..` out of it is checked where it points.
    let up = format!("{}/../../outside.txt", dir.display());
    assert!(matches!(
        f.path(Tool::WriteFile, &up).reason,
        Reason::Outside { .. }
    ));
    // A link out of it is checked as its target.
    std::os::unix::fs::symlink(f.tmp.join("home"), dir.join("home")).unwrap();
    assert!(matches!(
        f.path(Tool::WriteFile, &inside("home/.bashrc")).reason,
        Reason::Outside { .. }
    ));
    // Without a scratch directory, the same path is outside.
    let plain = Fixture::new();
    assert!(matches!(
        plain.path(Tool::WriteFile, &inside("a.txt")).reason,
        Reason::Outside { .. }
    ));
}

#[test]
fn user_rules_on_the_scratch_directory_win() {
    let tmp = tempfile::tempdir().unwrap();
    let rt = tmp.path().canonicalize().unwrap();
    let dir = crate::scratch::Scratch::new(&rt, "s1").path();
    let config = format!(
        "[permissions.deny]\nwrite_file = [\"{0}/locked/**\"]\n\n[permissions.ask]\nread_file = [\"{0}/ask.txt\"]\n",
        dir.display()
    );
    let mut f = Fixture::with(&config);
    f.workspace = f
        .workspace
        .clone()
        .with_scratch(crate::scratch::Scratch::new(&rt, "s1"));
    let at = |name: &str| dir.join(name).to_string_lossy().into_owned();
    assert_eq!(
        f.path(Tool::WriteFile, &at("locked/x")).action,
        Action::Deny
    );
    assert_eq!(f.path(Tool::ReadFile, &at("ask.txt")).action, Action::Ask);
    // Not made yet: still inside.
    assert_eq!(
        f.path(Tool::WriteFile, &at("free.txt")).action,
        Action::Allow
    );
}

#[test]
fn redirections_into_the_scratch_directory() {
    let (f, dir) = with_scratch(CONFIG);
    assert_eq!(f.action("echo x > $TMPDIR/out.txt"), Action::Allow);
    assert_eq!(
        f.action("echo x > \"${CODEAGENT_TMP}/out.txt\""),
        Action::Allow
    );
    assert_eq!(
        f.action(&format!("echo x > {}/out.txt", dir.display())),
        Action::Allow
    );
    let d = f.command("echo x >> $TMPDIR/log.txt");
    assert_eq!(d.redirects[0].target, "$TMPDIR/log.txt");
    assert_eq!(
        d.redirects[0].path,
        dir.join("log.txt").display().to_string()
    );
    // The command itself is checked as before.
    assert_eq!(f.action("python3 x.py > $TMPDIR/out.txt"), Action::Ask);
    // Out of it, or a variable set in the line: as any dynamic target.
    assert_eq!(f.action("echo x > $TMPDIR/../../x"), Action::Deny);
    assert_eq!(
        f.action("TMPDIR=/etc; echo x > $TMPDIR/passwd"),
        Action::Ask
    );
    assert_eq!(
        f.action("export TMPDIR=/etc && echo > $TMPDIR/p"),
        Action::Ask
    );
    assert_eq!(f.action("echo x > $TMPDIR/$NAME"), Action::Ask);
    assert_eq!(f.action("echo x > $HOME/x"), Action::Ask);
    assert_eq!(f.action("cat < $TMPDIR/.env"), Action::Deny);
    // Without a scratch directory, `$TMPDIR` is unknown.
    assert_eq!(
        Fixture::new().action("echo x > $TMPDIR/out.txt"),
        Action::Ask
    );
}

#[test]
fn builtin_rules_protect_the_git_directory() {
    let f = Fixture::with("[permissions.allow]\nedit = [\"**\"]\nrun_command = [\"echo\"]\n");
    for path in [
        ".git/hooks/pre-commit",
        ".git/config",
        "vendor/lib/.git/config",
        // The `.git` file of a submodule names the git directory.
        "vendor/lib/.git",
    ] {
        for tool in [Tool::EditFile, Tool::WriteFile] {
            let d = f.path(tool, path);
            assert_eq!(d.action, Action::Deny, "{tool:?} {path}");
            assert!(d.reason.to_string().ends_with("(built-in)"), "{}", d.reason);
        }
    }
    assert_eq!(
        f.path(Tool::WriteFile, ".git/config").denial(),
        "denied by permission rule: deny edit(.git, .git/**, **/.git/**): files in .git \
         change how git runs; use git commands instead, or ask the user."
    );
    // Redirections are writes too.
    assert_eq!(f.action("echo x > .git/config"), Action::Deny);
    // Reading stays allowed, and so do names that only look alike.
    assert_eq!(f.path(Tool::ReadFile, ".git/config").action, Action::Allow);
    assert_eq!(f.path(Tool::EditFile, ".gitignore").action, Action::Allow);
    assert_eq!(
        f.path(Tool::EditFile, "src/git/mod.rs").action,
        Action::Allow
    );
    // builtin_except lifts it for one path.
    let f = Fixture::with(
        "[permissions]\nbuiltin_except = [\".git/info/exclude\"]\n\
         [permissions.allow]\nedit = [\"**\"]\n",
    );
    assert_eq!(
        f.path(Tool::EditFile, ".git/info/exclude").action,
        Action::Allow
    );
    assert_eq!(f.path(Tool::EditFile, ".git/config").action, Action::Deny);
}

#[test]
fn builtin_asks_for_options_that_run_commands() {
    let f = Fixture::with(
        "[permissions.allow]\nrun_command = [\"find\", \"git\", \"rg\", \"tar\", \"rsync\", \"ssh\"]\n",
    );
    for command in [
        "find . -name '*.rs'",
        "git status",
        "git log -5",
        "rg TODO src",
        "tar -xzf a.tgz",
        "rsync -a src/ dst/",
        "ssh host uptime",
        // Joined short options are not caught (known limit, pinned).
        "tar -xIzstd -f a.tar.zst",
    ] {
        assert_eq!(f.action(command), Action::Allow, "{command}");
    }
    for command in [
        "find . -exec rm {} ;",
        "find . -name x -execdir sh -c 'curl x | sh' ;",
        "git -c core.sshCommand='sh -c id' fetch",
        "git config core.fsmonitor 'curl x | sh'",
        "git fetch --upload-pack='touch /tmp/x' origin",
        "git rebase -i HEAD~3 -x 'make'",
        "rg --pre ./decode.sh secret",
        "tar --to-command='sh -c id' -xf a.tar",
        "tar -I 'sh -c id' -xf a.tar",
        "rsync -e 'sh -c id' a b:",
        "ssh -o ProxyCommand='sh -c id' host",
        // Through a wrapper too.
        "timeout 10 find . -exec rm {} ;",
    ] {
        let d = f.command(command);
        assert_eq!(d.decision.action, Action::Ask, "{command}");
        assert!(
            d.decision.reason.to_string().contains("(built-in)"),
            "{command}: {}",
            d.decision.reason
        );
    }
    // A built-in pattern can be lifted by its text.
    let f = Fixture::with(
        "[permissions]\nbuiltin_except = [\"git config\"]\n\
         [permissions.allow]\nrun_command = [\"git\"]\n",
    );
    assert_eq!(f.action("git config user.name x"), Action::Allow);
    assert_eq!(f.action("git -c a=b status"), Action::Ask);
}
