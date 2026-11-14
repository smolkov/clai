use std::fs;
use std::path::{Path, PathBuf};

use super::*;

/// Writes `<dir>/<name>/SKILL.md` with that description.
fn skill(dir: &Path, name: &str, description: &str) -> PathBuf {
    let path = dir.join(name);
    fs::create_dir_all(&path).unwrap();
    fs::write(
        path.join("SKILL.md"),
        format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n"),
    )
    .unwrap();
    path
}

struct Places {
    _tmp: tempfile::TempDir,
    home: PathBuf,
    ws: PathBuf,
}

fn places() -> Places {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("home");
    let ws = tmp.path().join("ws");
    fs::create_dir_all(&home).unwrap();
    fs::create_dir_all(&ws).unwrap();
    Places {
        _tmp: tmp,
        home,
        ws,
    }
}

fn find_in(p: &Places, config: &Skills) -> Found {
    find(config, &p.ws, Some(&p.home), None)
}

#[test]
fn the_order_of_all_directories() {
    let p = places();
    let a = p.home.join("a");
    let b = p.home.join("b");
    let dirs = [
        a.clone(),
        b.clone(),
        p.ws.join(".opencode/skills"),
        p.ws.join(".claude/skills"),
        p.ws.join(".agents/skills"),
        p.home.join(".config/opencode/skills"),
        p.home.join(".claude/skills"),
        p.home.join(".agents/skills"),
    ];
    // Each directory has `same` and one skill of its own.
    for (i, dir) in dirs.iter().enumerate() {
        skill(dir, "same", &format!("from {i}"));
        skill(dir, &format!("own-{i}"), "own");
    }
    let config = Skills {
        dirs: vec![a.clone(), b],
        default_dirs: true,
    };
    let found = find_in(&p, &config);
    assert_eq!(
        found
            .dirs
            .iter()
            .map(|d| d.path.clone())
            .collect::<Vec<_>>(),
        dirs
    );
    assert_eq!(
        found.dirs.iter().map(|d| d.source).collect::<Vec<_>>(),
        [
            Source::Config,
            Source::Config,
            Source::Project,
            Source::Project,
            Source::Project,
            Source::User,
            Source::User,
            Source::User
        ]
    );
    assert_eq!(found.get("same").unwrap().description, "from 0");
    let same: Vec<&Skill> = found.skills.iter().filter(|s| s.name == "same").collect();
    assert_eq!(same.len(), 8);
    for s in same.iter().skip(1) {
        assert_eq!(s.shadowed_by.as_deref(), Some(a.join("same").as_path()));
    }
    assert_eq!(found.names().len(), 9);
    assert!(found.warnings.is_empty(), "{:?}", found.warnings);
}

#[test]
fn only_configured_directories() {
    let p = places();
    skill(&p.home.join(".claude/skills"), "pdf", "PDF");
    let team = p.home.join("team");
    skill(&team, "api", "API");
    let config = Skills {
        dirs: vec![team],
        default_dirs: false,
    };
    let found = find_in(&p, &config);
    assert_eq!(found.names(), ["api"]);
    assert_eq!(found.dirs.len(), 1);
}

#[test]
fn xdg_config_home_moves_opencode_only() {
    let p = places();
    let xdg = p.home.join("xdg");
    let dirs = search_dirs(&Skills::default(), &p.ws, Some(&p.home), Some(&xdg));
    let paths: Vec<PathBuf> = dirs.into_iter().map(|d| d.path).collect();
    assert!(paths.contains(&xdg.join("opencode/skills")));
    assert!(paths.contains(&p.home.join(".claude/skills")));
    assert!(!paths.contains(&p.home.join(".config/opencode/skills")));
}

#[test]
fn missing_directories() {
    let p = places();
    let config = Skills {
        dirs: vec![p.home.join("gone")],
        default_dirs: true,
    };
    let found = find_in(&p, &config);
    assert!(found.skills.is_empty());
    assert_eq!(found.warnings, ["~/gone: not found"]);
}

#[test]
fn skipped_files_are_warnings() {
    let p = places();
    let dir = p.ws.join(".claude/skills");
    skill(&dir, "good", "Good");
    fs::create_dir_all(dir.join("old")).unwrap();
    fs::write(dir.join("old/SKILL.md"), "---\nname: old\n---\n").unwrap();
    fs::create_dir_all(dir.join("Bad")).unwrap();
    fs::write(dir.join("Bad/SKILL.md"), "---\ndescription: d\n---\n").unwrap();
    // Not a skill: a directory without SKILL.md, a file.
    fs::create_dir_all(dir.join("notes")).unwrap();
    fs::write(dir.join("README.md"), "x").unwrap();
    let found = find_in(&p, &Skills::default());
    assert_eq!(found.names(), ["good"]);
    assert_eq!(
        found.warnings,
        [
            ".claude/skills/Bad/SKILL.md: name `Bad`: use 1 to 64 lowercase letters, digits and -, not at the start or end",
            ".claude/skills/old/SKILL.md: no description",
        ]
    );
}

#[test]
fn name_from_the_directory_or_the_file() {
    let p = places();
    let dir = p.home.join(".agents/skills");
    fs::create_dir_all(dir.join("plain")).unwrap();
    fs::write(dir.join("plain/SKILL.md"), "---\ndescription: d\n---\nbody").unwrap();
    fs::create_dir_all(dir.join("folder")).unwrap();
    fs::write(
        dir.join("folder/SKILL.md"),
        "---\nname: other\ndescription: d\n---\n",
    )
    .unwrap();
    let found = find_in(&p, &Skills::default());
    assert_eq!(found.names(), ["other", "plain"]);
    assert_eq!(found.get("plain").unwrap().body, "body");
    assert_eq!(
        found.warnings,
        [
            "~/.agents/skills/folder/SKILL.md: name `other` differs from the directory name; `other` is used"
        ]
    );
}

#[cfg(unix)]
#[test]
fn links_are_followed() {
    let p = places();
    let shared = p.home.join("shared");
    let target = skill(&shared, "release", "Release");
    let dir = p.home.join(".claude/skills");
    fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(&target, dir.join("release")).unwrap();
    // A link to the file itself.
    fs::create_dir_all(dir.join("pdf")).unwrap();
    let file = shared.join("pdf.md");
    fs::write(&file, "---\ndescription: PDF\n---\n").unwrap();
    std::os::unix::fs::symlink(&file, dir.join("pdf/SKILL.md")).unwrap();
    let found = find_in(&p, &Skills::default());
    assert_eq!(found.names(), ["pdf", "release"]);
    assert_eq!(found.get("release").unwrap().dir, dir.join("release"));
}

#[test]
fn unknown_names_list_the_found_ones() {
    let p = places();
    skill(&p.home.join(".claude/skills"), "pdf", "PDF");
    let found = find_in(&p, &Skills::default());
    assert_eq!(found.unknown("x"), "unknown skill `x`; found: pdf");
    assert_eq!(
        Found::default().unknown("x"),
        "unknown skill `x`; no skills are found"
    );
}

#[test]
fn paths_for_display() {
    let ws = Path::new("/w");
    let home = Some(Path::new("/h"));
    assert_eq!(
        display(Path::new("/w/.claude/skills"), ws, home),
        ".claude/skills"
    );
    assert_eq!(
        display(Path::new("/h/.claude/skills"), ws, home),
        "~/.claude/skills"
    );
    assert_eq!(display(Path::new("/opt/s"), ws, home), "/opt/s");
}

fn active(name: &str) -> Active {
    Active {
        name: name.into(),
        dir: PathBuf::from(format!("/s/{name}")),
        text: format!("# {name}"),
    }
}

#[test]
fn applying_a_list_keeps_the_order_of_activation() {
    let found = [active("a"), active("b"), active("c")];
    let names = ["a", "b", "c"];
    let list = |v: &[&str]| v.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    let Applied {
        active: next,
        added,
        removed,
    } = apply(&[], &list(&["c", "a"]), &found, &names).unwrap();
    assert_eq!(next, [active("c"), active("a")]);
    assert_eq!(added, [active("c"), active("a")]);
    assert!(removed.is_empty());
    let Applied {
        active: next,
        added,
        removed,
    } = apply(&next, &list(&["a", "b", "c"]), &found, &names).unwrap();
    assert_eq!(next, [active("c"), active("a"), active("b")]);
    assert_eq!(added, [active("b")]);
    assert!(removed.is_empty());
    let Applied {
        active: next,
        added,
        removed,
    } = apply(&next, &list(&["b"]), &found, &names).unwrap();
    assert_eq!(next, [active("b")]);
    assert!(added.is_empty());
    assert_eq!(removed, ["c", "a"]);
    // An active skill no longer found stays when named.
    let Applied { active: next, .. } = apply(&next, &list(&["b"]), &[], &[]).unwrap();
    assert_eq!(next, [active("b")]);
    assert_eq!(
        apply(&next, &list(&["x"]), &found, &names).unwrap_err(),
        "unknown skill `x`; found: a, b, c"
    );
}

#[test]
fn replaying_changes() {
    let a = [active("a"), active("b")];
    let none: [String; 0] = [];
    let off = ["a".to_owned()];
    let again = [active("a")];
    let list = replay([
        (&a[..], &none[..]),
        (&[][..], &off[..]),
        (&again[..], &none[..]),
    ]);
    assert_eq!(list, [active("b"), active("a")]);
}

#[test]
fn change_notice() {
    assert_eq!(
        change_text(&[active("release-check")], &["pdf".into()]),
        "skills: + release-check, \u{2212} pdf"
    );
}

#[test]
fn missing_and_changed_on_disk() {
    let p = places();
    let dir = skill(&p.home.join(".claude/skills"), "pdf", "PDF");
    let found = find_in(&p, &Skills::default());
    let mut on = found.activate("pdf").unwrap();
    assert_eq!(on.dir, dir);
    assert!(!found.missing(&on) && !found.changed_on_disk(&on));
    on.text.push_str(" old");
    assert!(found.changed_on_disk(&on));
    on.dir = PathBuf::from("/elsewhere");
    assert!(found.missing(&on) && !found.changed_on_disk(&on));
    assert_eq!(active("x").tokens(), 1);
}

#[test]
fn skills_are_found_in_deeper_directories() {
    let p = places();
    let dir = p.home.join(".claude/skills");
    let synced = dir.join("synced/5cb6fc9a_defc2ee2");
    skill(&synced, "pdf", "PDF");
    skill(&dir, "release", "Release");
    // Below a skill nothing more is looked for.
    skill(&dir.join("release/examples"), "inner", "Inner");
    // Hidden directories are passed over; too deep is not searched.
    skill(&dir.join(".cache"), "hidden", "Hidden");
    skill(&dir.join("a/b/c/d/e/f"), "too-deep", "Too deep");
    skill(&dir.join("a/b/c/d/e"), "deep", "Deep");
    let found = find_in(&p, &Skills::default());
    assert_eq!(found.names(), ["deep", "release", "pdf"]);
    assert_eq!(found.get("pdf").unwrap().dir, synced.join("pdf"));
    assert!(found.warnings.is_empty(), "{:?}", found.warnings);
}

#[cfg(unix)]
#[test]
fn a_link_loop_ends() {
    let p = places();
    let dir = p.home.join(".claude/skills");
    fs::create_dir_all(dir.join("group")).unwrap();
    std::os::unix::fs::symlink(&dir, dir.join("group/back")).unwrap();
    skill(&dir.join("group"), "pdf", "PDF");
    let found = find_in(&p, &Skills::default());
    assert_eq!(found.names(), ["pdf"]);
}
