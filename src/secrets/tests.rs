use super::*;

fn redactor() -> Redactor {
    Redactor::new(&SecretsSettings::default(), Vec::new())
}

/// The text with its secrets hidden by the default redactor.
fn hide(text: &str) -> String {
    redactor().hide(text)
}

/// A value of `n` characters that looks random: letters and digits.
fn random(n: usize) -> String {
    let chars = b"aZ3kQ9mX2pL7vB1nR8tY4wC6hJ0sD5fG";
    (0..n)
        .map(|i| char::from(chars[(i * 7 + i / 3) % chars.len()]))
        .collect()
}

#[test]
fn key_formats_are_hidden() {
    let cases = [
        (format!("sk-ant-api03-{}", random(90)), "anthropic key"),
        (format!("sk-proj-{}", random(60)), "openai key"),
        (format!("ghp_{}", random(36)), "github token"),
        (format!("github_pat_{}", random(60)), "github token"),
        (format!("glpat-{}", random(20)), "gitlab token"),
        ("AKIAIOSFODNN7EXAMPLE".to_owned(), "aws access key"),
        (format!("AIza{}", random(35)), "google api key"),
        (format!("xoxb-{}", random(30)), "slack token"),
        (format!("sk_live_{}", random(24)), "stripe key"),
        (format!("hf_{}", random(34)), "huggingface token"),
        (
            format!("eyJ{}.eyJ{}.{}", random(20), random(30), random(40)),
            "jwt",
        ),
    ];
    for (value, kind) in cases {
        let text = format!("the key is {value} here");
        let hidden = hide(&text);
        assert_eq!(
            hidden,
            format!("the key is {} here", placeholder(kind, &value)),
            "{kind}"
        );
        assert!(!hidden.contains(&value));
    }
}

#[test]
fn private_keys_and_url_passwords() {
    let block = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\nQyNTUxOQAAACD\n-----END OPENSSH PRIVATE KEY-----";
    let text = format!("before\n{block}\nafter");
    assert_eq!(
        hide(&text),
        format!("before\n{}\nafter", placeholder("private key", block))
    );
    // A block cut off, as while output streams.
    assert!(!hide("-----BEGIN RSA PRIVATE KEY-----\nMIIEow").contains("MIIEow"));
    assert_eq!(
        hide("git clone https://bob:s3cretPa55@example.com/x.git"),
        format!(
            "git clone https://bob:{}@example.com/x.git",
            placeholder("url password", "s3cretPa55")
        )
    );
    assert_eq!(
        hide("postgres://app:$DB_PASS@db/app"),
        "postgres://app:$DB_PASS@db/app"
    );
}

#[test]
fn assignments_with_random_values() {
    let value = random(32);
    for text in [
        format!("api_key = \"{value}\""),
        format!("password: {value}"),
        format!("\"auth_token\": \"{value}\","),
        format!("export MY_SECRET={value}"),
        format!("Authorization: Bearer {value}"),
    ] {
        let hidden = hide(&text);
        assert!(!hidden.contains(&value), "{text} -> {hidden}");
        assert!(hidden.contains("[secret: "), "{hidden}");
    }
    // Names, references and stand-ins stay.
    for text in [
        "api_key_env = \"ANTHROPIC_API_KEY\"",
        "token = \"$GITHUB_TOKEN\"",
        "password: ${{ secrets.DB_PASSWORD }}",
        "api_key = \"<your-key-here>\"",
        "password = \"changeme\"",
        "secret_key = os.environ[\"SECRET_KEY\"]",
        "token = 'xxxxxxxxxxxxxxxx'",
        "auth: true",
        "key = \"short1\"",
        "keywords = [\"cli\", \"agent\"]",
        "password_file = /etc/app/password.txt",
    ] {
        assert_eq!(hide(text), text);
    }
    let off = Redactor::new(
        &SecretsSettings {
            assignments: false,
            ..SecretsSettings::default()
        },
        Vec::new(),
    );
    let text = format!("password: {value}");
    assert_eq!(off.hide(&text), text);
}

#[test]
fn lookalikes_stay_visible() {
    for text in [
        "commit 3f9a1c2e8b7d6f5a4e3d2c1b0a9f8e7d6c5b4a39",
        "checksum = \"6f1f2d4c5b9e8a7f3e2d1c0b9a8f7e6d5c4b3a2f1e0d9c8b7a6f5e4d3c2b1a0\"",
        "id 550e8400-e29b-41d4-a716-446655440000",
        "the sk-learn library and ghp_short",
        "data: aGVsbG8gd29ybGQgdGhpcyBpcyBiYXNlNjQ=",
        "AKIA is a prefix",
    ] {
        assert_eq!(hide(text), text);
    }
}

#[test]
fn known_values_are_hidden_anywhere() {
    let r = Redactor::new(
        &SecretsSettings::default(),
        vec![
            ("env MY_SERVICE_TOKEN".into(), "plainword-but-secret".into()),
            ("env SHORT_KEY".into(), "abc".into()),
            ("env KEY_PATH".into(), "/home/u/.ssh/id".into()),
        ],
    );
    assert_eq!(
        r.hide("x=plainword-but-secretXYZ"),
        format!(
            "x={}XYZ",
            placeholder("env MY_SERVICE_TOKEN", "plainword-but-secret")
        )
    );
    assert_eq!(r.hide("abc and /home/u/.ssh/id"), "abc and /home/u/.ssh/id");
}

#[test]
fn known_values_from_the_config_and_the_environment() {
    let settings = SecretsSettings {
        values_env: vec!["ODD_NAME".into()],
        ..SecretsSettings::default()
    };
    let vars = [
        ("GITHUB_TOKEN", "tok-1234567890"),
        ("DEPLOY_API_KEY", "dep-1234567890"),
        ("ODD_NAME", "odd-1234567890"),
        ("PATH", "/usr/bin:/bin"),
        ("SSH_AUTH_SOCK", "sock-1234567890"),
        ("HOME", "/home/u"),
    ]
    .map(|(a, b)| (a.to_owned(), b.to_owned()));
    let r = Redactor::from_config(&settings, std::iter::empty(), vars).unwrap();
    let hidden = r.hide("tok-1234567890 dep-1234567890 odd-1234567890 sock-1234567890 /usr/bin");
    assert_eq!(
        hidden,
        format!(
            "{} {} {} sock-1234567890 /usr/bin",
            placeholder("env GITHUB_TOKEN", "tok-1234567890"),
            placeholder("env DEPLOY_API_KEY", "dep-1234567890"),
            placeholder("env ODD_NAME", "odd-1234567890"),
        )
    );
    let off = SecretsSettings {
        redact: false,
        ..SecretsSettings::default()
    };
    assert!(Redactor::from_config(&off, std::iter::empty(), Vec::new()).is_none());
}

#[test]
fn placeholders_are_stable_and_restorable() {
    let key = format!("ghp_{}", random(36));
    let r = redactor();
    let a = r.redact(&format!("one {key} two {key}"));
    assert_eq!(a.hidden.len(), 2);
    assert_eq!(a.hidden[0], a.hidden[1]);
    // Another redactor, as after a resume, gives the same placeholder.
    assert_eq!(redactor().hide(&key), a.hidden[0]);
    assert!(a.hidden[0].starts_with("[secret: github token #"));
    assert_eq!(count(&a.text), 2);
    // Back to the values.
    let restored = r.restore(&a.text);
    assert_eq!(restored.text, format!("one {key} two {key}"));
    assert!(restored.unknown.is_empty());
    // One the session has not seen stays, and is named.
    let restored = r.restore("x [secret: token #0000] y");
    assert_eq!(restored.text, "x [secret: token #0000] y");
    assert_eq!(restored.unknown, ["[secret: token #0000]"]);
}

#[test]
fn allowed_values_and_custom_patterns() {
    let fake = format!("ghp_{}", random(36));
    let r = Redactor::new(
        &SecretsSettings {
            allow: vec![fake.clone()],
            patterns: vec![r"acme_[a-z0-9]{8}".into()],
            ..SecretsSettings::default()
        },
        Vec::new(),
    );
    assert_eq!(r.hide(&fake), fake);
    assert_eq!(
        r.hide("id acme_abcd1234"),
        format!("id {}", placeholder("secret", "acme_abcd1234"))
    );
}

#[test]
fn json_strings_are_redacted() {
    let key = format!("glpat-{}", random(20));
    let value = serde_json::json!({"diff": [format!("+token {key}")], "n": 1});
    let hidden = redactor().redact_json(&value);
    assert!(!hidden.to_string().contains(&key));
    assert_eq!(hidden["n"], 1);
}

#[test]
fn the_debug_form_shows_no_value() {
    let r = Redactor::new(
        &SecretsSettings::default(),
        vec![("k".into(), "very-secret-value".into())],
    );
    r.hide("very-secret-value");
    assert!(!format!("{r:?}").contains("very-secret"));
}

#[test]
fn entropy_of_text() {
    assert!(entropy("aaaaaaaa") < 0.1);
    assert!(entropy(&random(32)) > MIN_ENTROPY);
}
