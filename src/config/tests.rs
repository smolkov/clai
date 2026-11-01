use super::*;

const BASE: &str = r#"
default_model = "anthropic/sonnet"

[providers.anthropic]
api = "anthropic"
api_key_env = "ANTHROPIC_API_KEY"

[providers.anthropic.models.sonnet]
id = "claude-sonnet-5-5"
context_window = 200000
max_output_tokens = 64000

[providers.anthropic.models.opus]
id = "claude-opus-5-5"
context_window = 200000

[providers.local]
api = "openai-compatible"
base_url = "http://localhost:11434/v1"

[providers.local.models.qwen]
id = "qwen3-coder:30b"
context_window = 32768
"#;

fn load_with(user: &str, project: Option<&str>, env: &Environment) -> Result<Config, ConfigErrors> {
    let user = FileCtx {
        path: Path::new("user.toml"),
        text: user,
    };
    let project = project.map(|text| FileCtx {
        path: Path::new("project.toml"),
        text,
    });
    build(user, project, env)
}

fn ok(user: &str) -> Config {
    load_with(user, None, &Environment::default()).unwrap_or_else(|e| panic!("{e}"))
}

/// The error messages, one string per error.
fn errs(user: &str, project: Option<&str>) -> Vec<String> {
    errs_env(user, project, &Environment::default())
}

fn errs_env(user: &str, project: Option<&str>, env: &Environment) -> Vec<String> {
    match load_with(user, project, env) {
        Ok(_) => panic!("loaded without errors"),
        Err(e) => e.0.iter().map(ToString::to_string).collect(),
    }
}

fn with(extra: &str) -> String {
    format!("{BASE}\n{extra}")
}

#[test]
fn example_is_valid() {
    let config = ok(EXAMPLE);
    assert_eq!(config.default_model.to_string(), "anthropic/sonnet");
}

#[test]
fn base_config() {
    let config = ok(BASE);
    assert_eq!(
        config.providers.keys().collect::<Vec<_>>(),
        ["anthropic", "local"]
    );
    let anthropic = &config.providers["anthropic"];
    assert_eq!(anthropic.api, Api::Anthropic);
    assert_eq!(anthropic.base_url, "https://api.anthropic.com");
    assert_eq!(anthropic.key, KeySource::Env("ANTHROPIC_API_KEY".into()));
    assert_eq!(anthropic.timeout, Duration::from_secs(300));
    assert_eq!(anthropic.max_retries, 3);
    assert_eq!(
        anthropic.models.keys().collect::<Vec<_>>(),
        ["sonnet", "opus"]
    );
    let sonnet = &anthropic.models["sonnet"];
    assert_eq!(sonnet.id, "claude-sonnet-5-5");
    assert_eq!(sonnet.display_name, "sonnet");
    assert_eq!(sonnet.max_output_tokens, Some(64000));
    assert_eq!(config.providers["local"].key, KeySource::None);
    assert_eq!(config.tui, Tui::default());
    assert_eq!(config.permissions, Permissions::default());
}

#[test]
fn model_references() {
    let config = ok(BASE);
    let r = |s| config.resolve_model(s);
    assert_eq!(r("local/qwen").unwrap().to_string(), "local/qwen");
    assert_eq!(r("opus").unwrap().to_string(), "anthropic/opus");
    assert!(
        r("gpt")
            .unwrap_err()
            .contains("configured: anthropic/sonnet")
    );
    assert!(r("local/opus").unwrap_err().contains("unknown model"));
    let (provider, model) = config.model(&r("qwen").unwrap()).unwrap();
    assert_eq!(
        (provider.name.as_str(), model.id.as_str()),
        ("local", "qwen3-coder:30b")
    );
}

#[test]
fn ambiguous_short_form() {
    let config = ok(&with(
        "[providers.other]\napi = \"anthropic\"\n[providers.other.models.opus]\nid = \"x\"\ncontext_window = 1\n",
    ));
    let err = config.resolve_model("opus").unwrap_err();
    assert!(err.contains("anthropic/opus, other/opus"), "{err}");
}

#[test]
fn syntax_error_has_line() {
    let e = errs("default_model = \n", None);
    assert_eq!(e.len(), 1);
    assert!(e[0].starts_with("user.toml:1: "), "{e:?}");
}

#[test]
fn unknown_key_is_error() {
    let e = errs(
        &with("[providers.local.models.qwen2]\nid = \"q\"\ncontext_window = 1\ncontxt = 3\n"),
        None,
    );
    assert!(e[0].contains("unknown field `contxt`"), "{e:?}");
    assert!(e[0].starts_with("user.toml:28: "), "{e:?}");
}

#[test]
fn missing_context_window() {
    let e = errs(&with("[providers.local.models.big]\nid = \"b\"\n"), None);
    assert_eq!(
        e,
        ["user.toml:25: providers.local.models.big: missing context_window"]
    );
}

#[test]
fn no_provider_and_no_model() {
    assert_eq!(
        errs("default_model = \"a/b\"\n", None)[0],
        "user.toml: no provider is defined"
    );
    let e = errs(
        "default_model = \"a/b\"\n[providers.a]\napi = \"anthropic\"\n",
        None,
    );
    assert_eq!(e[0], "user.toml: no model is defined");
}

#[test]
fn default_model_missing_or_unknown() {
    let without = BASE.replace("default_model = \"anthropic/sonnet\"", "");
    assert_eq!(errs(&without, None), ["user.toml: missing default_model"]);
    let unknown = BASE.replace("anthropic/sonnet", "anthropic/haiku");
    let e = errs(&unknown, None);
    assert_eq!(e.len(), 1);
    assert!(
        e[0].starts_with("user.toml:2: default_model: unknown model `anthropic/haiku`"),
        "{e:?}"
    );
}

#[test]
fn provider_errors() {
    let e = errs(
        &with(
            r#"
[providers.bad]
api = "gemini"

[providers.both]
api = "anthropic"
api_key_env = "A"
api_key_cmd = "b"

[providers.open]
api = "openai-compatible"

[providers.url]
api = "anthropic"
base_url = "localhost"
timeout_s = 0

[providers.noapi]
"#,
        ),
        None,
    );
    let expect = [
        "providers.bad.api: unknown api `gemini`",
        "providers.both: set only one of api_key_env and api_key_cmd",
        "providers.open: missing base_url (required for openai-compatible)",
        "providers.url.base_url: `localhost` must start with http:// or https://",
        "providers.url.timeout_s: 0 is not in 1–86400",
        "providers.noapi: missing api",
    ];
    assert_eq!(e.len(), expect.len(), "{e:?}");
    for (got, want) in e.iter().zip(expect) {
        assert!(got.contains(want), "{got} does not contain {want}");
    }
}

#[test]
fn name_with_slash() {
    let e = errs(&with("[providers.\"a/b\"]\napi = \"anthropic\"\n"), None);
    assert!(e[0].contains("must not be empty or contain `/`"), "{e:?}");
}

#[test]
fn model_errors() {
    let e = errs(
        &with(
            "[providers.local.models.m]\ncontext_window = 0\ntemperature = 3.5\nmax_output_tokens = 0\n",
        ),
        None,
    );
    let joined = e.join("\n");
    for want in [
        "providers.local.models.m: missing id",
        "providers.local.models.m.context_window: 0 is not in",
        "providers.local.models.m.max_output_tokens: 0 is not in",
        "providers.local.models.m.temperature: 3.5 is not in 0–2",
    ] {
        assert!(joined.contains(want), "{joined}\nmissing: {want}");
    }
}

#[test]
fn invalid_model_does_not_cascade() {
    // `local/qwen` is invalid, but default_model naming it is not a second error.
    let text = BASE
        .replace(
            "default_model = \"anthropic/sonnet\"",
            "default_model = \"local/qwen\"",
        )
        .replace("context_window = 32768", "");
    let e = errs(&text, None);
    assert_eq!(e.len(), 1, "{e:?}");
}

#[test]
fn key_cmd() {
    let config = ok(&BASE.replace(
        "api_key_env = \"ANTHROPIC_API_KEY\"",
        "api_key_cmd = \"pass show anthropic\"",
    ));
    assert_eq!(
        config.providers["anthropic"].key,
        KeySource::Cmd("pass show anthropic".into())
    );
}

#[test]
fn project_overrides_default_model_and_commit() {
    let config = load_with(
        &with("[commit]\nmodel = \"opus\"\ntrailer = \"T\"\n"),
        Some("default_model = \"local/qwen\"\n[commit]\nmodel = \"local/qwen\"\n"),
        &Environment::default(),
    )
    .unwrap();
    assert_eq!(config.default_model.to_string(), "local/qwen");
    assert_eq!(config.commit.model.unwrap().to_string(), "local/qwen");
    assert_eq!(config.commit.trailer.as_deref(), Some("T"));
    assert_eq!(config.project_file, Some(PathBuf::from("project.toml")));
}

#[test]
fn project_may_not_define_user_tables() {
    let e = errs(
        BASE,
        Some(
            "[providers.evil]\napi = \"anthropic\"\n[tui]\nmouse = false\n[themes.x]\nbase = \"dark\"\n",
        ),
    );
    assert_eq!(e.len(), 3, "{e:?}");
    assert!(
        e[0].starts_with("project.toml:1: providers: only allowed in the user file (user.toml)"),
        "{e:?}"
    );
    assert!(e[1].contains("tui: only allowed"), "{e:?}");
    assert!(e[2].contains("themes: only allowed"), "{e:?}");
}

#[test]
fn project_unknown_key() {
    let e = errs(BASE, Some("[agent]\nmax = 3\n"));
    assert!(
        e[0].starts_with("project.toml:1: unknown field `agent`"),
        "{e:?}"
    );
}

#[test]
fn env_overrides() {
    let env = Environment {
        model: Some("qwen".into()),
        theme: Some("light".into()),
        ..Environment::default()
    };
    let config = load_with(BASE, Some("default_model = \"opus\"\n"), &env).unwrap();
    assert_eq!(config.default_model.to_string(), "local/qwen");
    assert_eq!(config.tui.theme, "light");
}

#[test]
fn env_errors() {
    let env = Environment {
        model: Some("nope".into()),
        theme: Some("neon".into()),
        ..Environment::default()
    };
    let e = errs_env(BASE, None, &env);
    assert!(
        e[0].starts_with("CLAI_MODEL: unknown model `nope`"),
        "{e:?}"
    );
    assert_eq!(e[1], "CLAI_THEME: unknown theme `neon`");
}

/// Rules as `action label(patterns)`, in load order.
fn rule_texts(config: &Config) -> Vec<String> {
    config
        .permissions
        .rules
        .iter()
        .map(ToString::to_string)
        .collect()
}

#[test]
fn permissions_user_file() {
    let env = Environment {
        home: Some(PathBuf::from("/home/u")),
        ..Environment::default()
    };
    let config = load_with(
        &with(
            r#"
[permissions]
read_file = "ask"
run_command = "deny"
extra_dirs = ["~/ws/shared-lib", { path = "/opt/docs", access = "read" }]

[permissions.allow]
run_command = ["cargo test", "ls*"]
edit = ["src/**"]

[permissions.ask]
run_command = ["git push"]

[permissions.deny]
run_command = ["rm", "curl ** | sh"]
read = [".env*", "~/.ssh/**"]

[[permissions.rules]]
action = "deny"
tools = ["read"]
paths = ["secrets/**"]
except = ["secrets/README.md"]
message = "no secrets"

[[permissions.rules]]
action = "allow"
tools = ["run_command"]
commands = ["cargo"]
except = ["cargo publish"]
"#,
        ),
        None,
        &env,
    )
    .unwrap();
    let p = &config.permissions;
    assert_eq!(p.action(Tool::ReadFile), Action::Ask);
    assert_eq!(p.action(Tool::RunCommand), Action::Deny);
    assert_eq!(p.action(Tool::EditFile), Action::Ask);
    assert_eq!(p.action(Tool::ListFiles), Action::Allow);
    assert_eq!(
        rule_texts(&config),
        [
            "allow run_command(cargo test)",
            "allow run_command(ls*)",
            "allow edit(src/**)",
            "ask run_command(git push)",
            "deny run_command(rm)",
            "deny run_command(curl ** | sh)",
            "deny read(.env*)",
            "deny read(~/.ssh/**)",
            "deny read(secrets/**)",
            "allow run_command(cargo)",
        ]
    );
    let detailed = &p.rules[8];
    assert_eq!(detailed.tools.len(), 3);
    assert_eq!(detailed.except[0].text(), "secrets/README.md");
    assert_eq!(detailed.message.as_deref(), Some("no secrets"));
    assert_eq!(p.rules[4].source.to_string(), "user.toml:39");
    assert_eq!(
        p.extra_dirs,
        [
            ExtraDir {
                path: PathBuf::from("/home/u/ws/shared-lib"),
                access: Access::Write
            },
            ExtraDir {
                path: PathBuf::from("/opt/docs"),
                access: Access::Read
            }
        ]
    );
}

#[test]
fn permissions_errors() {
    let e = errs(
        &with(
            r#"
[permissions]
extra_dirs = ["rel", { path = "/x", access = "all" }]

[permissions.allow]
bash = ["ls"]
run_command = ["git 'x", "curl ** | sh"]
edit = ["src/", "", "a["]
"#,
        ),
        None,
    );
    let expected = [
        "permissions.allow.bash: unknown tool or group `bash`",
        "permissions.allow.run_command: pattern `git 'x`: unbalanced quotes",
        "permissions.allow.run_command: pattern `curl ** | sh`: `|` is only allowed in deny and ask rules",
        "permissions.allow.edit: pattern `src/` ends in `/`; use `src/**` for a directory",
        "permissions.allow.edit: empty pattern",
        "permissions.allow.edit: pattern `a[`",
        "permissions.extra_dirs: `rel` must be an absolute path or start with `~/`",
        "permissions.extra_dirs: unknown access `all`; use read or write",
    ];
    assert_eq!(e.len(), expected.len(), "{e:#?}");
    for (e, expected) in e.iter().zip(expected) {
        assert!(e.contains(expected), "{e}\n  does not contain\n{expected}");
    }
    let e = errs(&with("[permissions]\nshell = \"allow\"\n"), None);
    assert!(e[0].contains("unknown field `shell`"), "{e:?}");
    let e = errs(&with("[permissions]\nread_file = \"maybe\"\n"), None);
    assert!(e[0].contains("unknown variant `maybe`"), "{e:?}");
    // The old list form is a type error.
    let e = errs(&with("[permissions]\nallow = [\"read_file\"]\n"), None);
    assert!(e[0].contains("invalid type"), "{e:?}");
    let e = errs(&with("[permissions.deny]\nrun_command = \"rm\"\n"), None);
    assert!(e[0].contains("invalid type"), "{e:?}");
}

#[test]
fn detailed_rule_errors() {
    let cases = [
        ("tools = [\"read\"]", "missing action"),
        ("action = \"deny\"", "missing tools"),
        ("action = \"deny\"\ntools = []", "missing tools"),
        (
            "action = \"deny\"\ntools = [\"rm\"]",
            "unknown tool or group `rm`",
        ),
        (
            "action = \"deny\"\ntools = [\"read\", \"run_command\"]",
            "mixes file tools and run_command",
        ),
        (
            "action = \"deny\"\ntools = [\"run_command\"]\npaths = [\"x\"]",
            "`paths` is for file tools",
        ),
        (
            "action = \"deny\"\ntools = [\"edit\"]\ncommands = [\"x\"]",
            "`commands` is for run_command",
        ),
        (
            "action = \"allow\"\ntools = [\"edit\"]\nmessage = \"m\"",
            "`message` is not allowed with action = \"allow\"",
        ),
        (
            "action = \"allow\"\ntools = [\"run_command\"]\ncommands = [\"a | b\"]",
            "`|` is only allowed in deny and ask rules",
        ),
        (
            "action = \"deny\"\ntools = [\"edit\"]\nexcept = [\"x/\"]",
            "ends in `/`",
        ),
        (
            "action = \"deny\"\ntools = [\"edit\"]\ncolour = 1",
            "unknown field `colour`",
        ),
    ];
    for (body, expected) in cases {
        let e = errs(&with(&format!("[[permissions.rules]]\n{body}\n")), None);
        assert!(
            e.iter().any(|e| e.contains(expected)),
            "{body}: {e:?} does not contain {expected}"
        );
    }
}

#[test]
fn project_permissions_only_stricter() {
    let config = load_with(
        &with("[permissions]\nedit_file = \"allow\"\n[permissions.ask]\nread_file = [\"a\"]\n"),
        Some(
            "[permissions]\nedit_file = \"ask\"\nread_file = \"deny\"\n[permissions.ask]\nread_file = [\"b\"]\n[permissions.deny]\nwrite_file = [\"**\"]\n[[permissions.rules]]\naction = \"deny\"\ntools = [\"run_command\"]\ncommands = [\"rm\"]\nmessage = \"no\"\n",
        ),
        &Environment::default(),
    )
    .unwrap();
    let p = &config.permissions;
    assert_eq!(p.action(Tool::EditFile), Action::Ask);
    assert_eq!(p.action(Tool::ReadFile), Action::Deny);
    assert_eq!(
        rule_texts(&config),
        [
            "ask read_file(a)",
            "ask read_file(b)",
            "deny write_file(**)",
            "deny run_command(rm)"
        ]
    );
    assert_eq!(p.rules[1].source.to_string(), "project.toml:5");

    // A stricter user value stays: project "ask" does not loosen user "deny".
    let config = load_with(
        &with("[permissions]\nrun_command = \"deny\"\n"),
        Some("[permissions]\nrun_command = \"ask\"\n"),
        &Environment::default(),
    )
    .unwrap();
    assert_eq!(config.permissions.action(Tool::RunCommand), Action::Deny);
}

#[test]
fn project_permissions_may_not_loosen() {
    let e = errs(
        BASE,
        Some(
            "[permissions]\nrun_command = \"allow\"\nextra_dirs = [\"/x\"]\n[permissions.allow]\nread_file = [\"x\"]\n[[permissions.rules]]\naction = \"allow\"\ntools = [\"edit\"]\n",
        ),
    );
    assert_eq!(e.len(), 4, "{e:?}");
    assert_eq!(
        e[0],
        "project.toml:2: permissions.run_command: a project file may not set a tool to allow"
    );
    assert!(
        e[1].starts_with("project.toml:")
            && e[1].contains("permissions.allow: a project file may not add allow rules"),
        "{e:?}"
    );
    assert_eq!(
        e[2],
        "project.toml:3: permissions.extra_dirs: a project file may not set extra_dirs"
    );
    assert!(
        e[3].contains("permissions.rules: a project file may not add allow rules"),
        "{e:?}"
    );
}

#[test]
fn tui_and_themes() {
    let config = ok(&with(
        r##"
[tui]
theme = "solar"
symbols = "ascii"
mouse = false
max_fps = 60

[themes.solar]
base = "dark"
user = "cyan"
accent = "#e5a50a"
border = 240
code = "Solarized (dark)"
"##,
    ));
    assert_eq!(config.tui.theme, "solar");
    assert_eq!(config.tui.symbols, Symbols::Ascii);
    assert!(!config.tui.mouse);
    assert_eq!(config.tui.max_fps, 60);
    assert_eq!(config.tui.scroll_lines, 3);
    let solar = &config.themes["solar"];
    assert_eq!(solar.base, BuiltinTheme::Dark);
    assert_eq!(
        solar.colours,
        [
            ("user", Colour::Named("cyan")),
            ("accent", Colour::Rgb(0xe5, 0xa5, 0x0a)),
            ("border", Colour::Index(240)),
        ]
    );
    assert_eq!(solar.code.as_deref(), Some("Solarized (dark)"));
}

#[test]
fn tui_errors() {
    let e = errs(
        &with(
            r#"
[tui]
theme = "neon"
max_fps = 500

[themes.dark]
base = "light"

[themes.a]
base = "b"

[themes.c]
text = "purple"
"#,
        ),
        None,
    );
    let joined = e.join("\n");
    for want in [
        "themes.dark: a custom theme may not use the name of a built-in theme",
        "themes.a.base: base `b` is not a built-in theme",
        "themes.c.text: unknown colour `purple`",
        "tui.theme: unknown theme `neon`",
        "tui.max_fps: 500 is not in 1–120",
    ] {
        assert!(joined.contains(want), "{joined}\nmissing: {want}");
    }
    let e = errs(&with("[themes.x]\nshadow = \"red\"\n"), None);
    assert!(e[0].contains("unknown field `shadow`"), "{e:?}");
}

#[test]
fn user_path() {
    let env = Environment {
        xdg_config_home: Some(PathBuf::from("/x")),
        home: Some(PathBuf::from("/home/u")),
        ..Environment::default()
    };
    assert_eq!(
        user_config_path(&env),
        Some(PathBuf::from("/x/clai/config.toml"))
    );
    let env = Environment {
        xdg_config_home: Some(PathBuf::from("relative")),
        home: Some(PathBuf::from("/home/u")),
        ..Environment::default()
    };
    assert_eq!(
        user_config_path(&env),
        Some(PathBuf::from("/home/u/.config/clai/config.toml"))
    );
    assert_eq!(user_config_path(&Environment::default()), None);
}

#[test]
fn limits() {
    assert_eq!(ok(BASE).limits, Limits::default());
    let user = with("[limits]\nmax_requests = 20\nmax_tool_calls = 300\n");
    let config = ok(&user);
    assert_eq!(config.limits.max_requests, 20);
    assert_eq!(config.limits.max_tool_calls, 300);

    // A project file only lowers them.
    let project = "[limits]\nmax_requests = 40\nmax_tool_calls = 100\n";
    let config = load_with(&user, Some(project), &Environment::default()).unwrap();
    assert_eq!(config.limits.max_requests, 20);
    assert_eq!(config.limits.max_tool_calls, 100);

    assert_eq!(
        errs(
            &with("[limits]\nmax_requests = 0\nmax_tool_calls = 10001\n"),
            None
        ),
        [
            "user.toml:26: limits.max_requests: 0 is not in 1–10000",
            "user.toml:27: limits.max_tool_calls: 10001 is not in 1–10000",
        ]
    );
    assert_eq!(
        errs(BASE, Some("[limits]\nmax_turns = 3\n")),
        ["project.toml:2: unknown field `max_turns`, expected `max_requests` or `max_tool_calls`"]
    );
}

#[test]
fn sample_configs() {
    let full = include_str!("../../tests/configs/valid/full.toml");
    let project = include_str!("../../tests/configs/project/config.toml");
    let env = Environment {
        home: Some(PathBuf::from("/home/u")),
        ..Environment::default()
    };
    let config = load_with(full, None, &env).unwrap();
    assert_eq!(config.providers.len(), 3);
    assert_eq!(config.limits, Limits::default());
    assert_eq!(config.tui.theme, "solar");
    assert_eq!(config.permissions.extra_dirs.len(), 2);
    let rules = rule_texts(&config);
    assert!(
        rules.contains(&"allow run_command(cargo test)".to_owned()),
        "{rules:?}"
    );
    assert!(
        rules.contains(&"deny run_command(curl ** | sh)".to_owned()),
        "{rules:?}"
    );

    let config = load_with(full, Some(project), &env).unwrap();
    assert_eq!(config.limits.max_requests, 30);
    assert_eq!(config.limits.max_tool_calls, 120);
    assert_eq!(
        config.commit.trailer.as_deref(),
        Some("Signed-off-by: Project Bot <bot@example.com>")
    );
    let rules = rule_texts(&config);
    assert!(
        rules.contains(&"deny run_command(cargo publish)".to_owned()),
        "{rules:?}"
    );
}
