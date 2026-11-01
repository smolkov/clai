//! Checks against a configuration: rule order, boundary, commands.

use std::fs;
use std::path::Path;

use crate::config::permissions::parse_for_test;
use crate::config::{Access, Action, ExtraDir, Permissions, Rule, Tool};

use super::*;

const CONFIG: &str = r#"
[permissions]
read_file = "allow"

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
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("ws");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "").unwrap();
        fs::create_dir_all(tmp.path().join("docs")).unwrap();
        fs::create_dir_all(tmp.path().join("lib")).unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(&home).unwrap();
        let perms = parse_for_test(CONFIG, Some(&home));
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
        "rule ask edit(src/main.rs) (config.toml:14)"
    );
    let d = f.path(Tool::EditFile, "tests/a.rs");
    assert_eq!(d.reason.to_string(), "default for edit_file");
    let d = f.path(Tool::ReadFile, "config/a.key");
    assert_eq!(
        d.denial(),
        "denied by permission rule: deny read(config/*.key): Keys stay private."
    );
    assert_eq!(d.message(), Some("Keys stay private."));
    let d = f.path(Tool::ReadFile, "/etc/passwd");
    assert_eq!(d.denial(), "denied: /etc/passwd is outside the workspace");
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
