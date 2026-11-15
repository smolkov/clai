//! `codeagent skills`: list the skills the current workspace would find (see
//! `spec/skills.md`).

use std::path::Path;

use clap::Args;
use serde_json::json;

use super::{Context, Execute};
use crate::error::Result;
use crate::output::{Align, columns, note, write_stdout};
use crate::skills::{self, Found};

#[derive(Debug, Args)]
pub struct Skills {
    /// Print a JSON object instead of the table.
    #[arg(long)]
    pub json: bool,
}

impl Execute for Skills {
    fn name(&self) -> &'static str {
        "skills"
    }

    fn run(&self, ctx: &Context) -> Result<()> {
        let workspace = ctx.workspace(None)?;
        let options = ctx.load_options(workspace.clone());
        let config = crate::config::load(&options)?;
        let found = skills::find_for(&config, &workspace, &options.env);
        let home = options.env.home.as_deref();
        if self.json {
            return write_stdout(&render_json(&found));
        }
        for warning in &found.warnings {
            note(&format!("warning: {warning}"));
        }
        write_stdout(&render_table(&found, &workspace, home))
    }
}

/// One line per skill: `NAME  SOURCE  PATH  DESCRIPTION`; a shadowed skill
/// says by which one instead of its description. Without skills, the
/// directories searched.
pub fn render_table(found: &Found, workspace: &Path, home: Option<&Path>) -> String {
    let show = |path: &Path| skills::display(path, workspace, home);
    if found.skills.is_empty() {
        let mut out = String::from("no skills found; searched:\n");
        for dir in &found.dirs {
            out.push_str(&format!("  {}\n", show(&dir.path)));
        }
        return out;
    }
    let rows: Vec<Vec<String>> = found
        .skills
        .iter()
        .map(|s| {
            let about = match &s.shadowed_by {
                Some(by) => format!("shadowed by {}", show(by)),
                None => s.description.clone(),
            };
            vec![
                s.name.clone(),
                s.source.name().to_owned(),
                show(&s.dir),
                about,
            ]
        })
        .collect();
    let mut out = String::new();
    for line in columns(&rows, &[Align::Left, Align::Left, Align::Left]) {
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// `{"skills": [...], "dirs": [...], "warnings": [...]}` with absolute paths.
pub fn render_json(found: &Found) -> String {
    let skills: Vec<serde_json::Value> = found
        .skills
        .iter()
        .map(|s| {
            json!({
                "name": s.name,
                "description": s.description,
                "dir": s.dir,
                "source": s.source,
                "shadowed_by": s.shadowed_by,
            })
        })
        .collect();
    let dirs: Vec<serde_json::Value> = found
        .dirs
        .iter()
        .map(|d| json!({ "path": d.path, "source": d.source }))
        .collect();
    let value = json!({
        "skills": skills,
        "dirs": dirs,
        "warnings": found.warnings,
    });
    format!(
        "{}\n",
        serde_json::to_string_pretty(&value).unwrap_or_default()
    )
}
