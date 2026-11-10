//! The status row of the footer: the current state, turn counters and key
//! hints.

use crate::agent::State;
use crate::tools::group;
use crate::tui::app::App;
use crate::tui::keys::Focus;

fn plural(n: u32, one: &str) -> String {
    let count = group(usize::try_from(n).unwrap_or(usize::MAX));
    if n == 1 {
        format!("{count} {one}")
    } else {
        format!("{count} {one}s")
    }
}

pub fn text(app: &App) -> String {
    let symbols = &app.settings.look.symbols;
    if let Some(reason) = &app.ended {
        return format!(
            "session ended: {reason} {} any key exits",
            symbols.separator
        );
    }
    let mut parts: Vec<String> = Vec::new();
    if app.lost_since.is_some() {
        parts.push("server disconnected, retrying".to_owned());
    } else if let Some(name) = &app.switching {
        parts.push(format!("attaching {name}{}", symbols.more));
    } else {
        parts.push(state(app));
    }
    if let Some(message) = &app.message {
        parts.push(message.clone());
    }
    if app.new_output {
        parts.push(format!("{} new output", symbols.new_output));
    }
    let queued = app.conversation.queued.len();
    if queued > 0 {
        parts.push(format!("{queued} queued"));
    }
    if matches!(app.session.state, State::Working | State::Waiting) {
        parts.push("Esc cancel".to_owned());
    }
    if app.focus == Focus::Conversation {
        if app.selected_is_call() {
            parts.push("Enter expand".to_owned());
            parts.push("o open".to_owned());
        }
        parts.push("y copy".to_owned());
        parts.push("? help".to_owned());
        if app.settings.tui.mouse {
            parts.push("drag copy".to_owned());
        }
    } else {
        parts.push("/help".to_owned());
    }
    parts.push("Ctrl-D detach".to_owned());
    parts.join(&format!(" {} ", symbols.separator))
}

/// `idle`, `asking`, `running tool… 2 requests, 1 tool call`.
fn state(app: &App) -> String {
    let more = app.settings.look.symbols.more;
    if app.prompt.is_some() {
        return "asking".to_owned();
    }
    match app.session.state {
        State::Idle => "idle".to_owned(),
        State::Saved => "saved".to_owned(),
        State::Waiting => "waiting".to_owned(),
        State::Working => {
            let what = if app.conversation.running_tool() {
                "running tool"
            } else {
                "waiting for model"
            };
            format!(
                "{what}{more} {}, {}",
                plural(app.requests, "request"),
                plural(app.tool_calls, "tool call")
            )
        }
    }
}
