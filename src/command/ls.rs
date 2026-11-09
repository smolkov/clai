//! `clai ls` (see `spec/command.md`).

use clap::Args;
use serde_json::{Value, json};

use super::{Context, Execute};
use crate::agent::{QuestionKind, now_secs};
use crate::client::with_client;
use crate::error::Result;
use crate::output::write_stdout;
use crate::protocol::SessionInfo;
use crate::text::sessions::{state_name, table};

#[derive(Debug, Args)]
pub struct Ls {
    /// Print a JSON array instead of the table.
    #[arg(long)]
    pub json: bool,
}

impl Execute for Ls {
    fn name(&self) -> &'static str {
        "ls"
    }

    fn run(&self, ctx: &Context) -> Result<()> {
        let config = ctx.config();
        let sessions = with_client(config, true, async |client| client.list().await)?;
        let text = if self.json {
            let list: Vec<Value> = sessions.iter().map(json_of).collect();
            format!("{}\n", Value::Array(list))
        } else {
            table(&sessions, now_secs())
        };
        if sessions.is_empty() && !self.json {
            return Ok(());
        }
        write_stdout(&text)
    }
}

fn json_of(s: &SessionInfo) -> Value {
    let question = s.question.as_ref().map(|q| {
        json!({
            "id": q.id,
            "kind": match q.kind {
                QuestionKind::Permission(_) => "permission",
                QuestionKind::Limit(_) => "limit",
                QuestionKind::AskUser(_) => "ask_user",
            },
            "summary": q.summary,
        })
    });
    json!({
        "name": s.name,
        "workspace": s.path,
        "model": s.model,
        "state": state_name(s.state),
        "created": s.created,
        "queued": s.queued,
        "question": question,
        "input_tokens": s.usage.input_tokens,
        "output_tokens": s.usage.output_tokens,
    })
}
