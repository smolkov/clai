//! Boxes over the screen: help and the confirmation of `/kill`.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;

use crate::tui::app::{App, Dialog};
use crate::tui::commands::COMMANDS;

const KEYS: &str = "\
Enter            send the message
Alt-Enter        new line (Shift-Enter where the terminal reports it)
Up / Down        earlier messages; select an entry in the conversation
Tab              switch between input and conversation
Enter            expand a tool call (conversation)
o                open a tool call in the details view (conversation)
y / Y            copy the entry / its code block (conversation)
Ctrl-Y           copy the last answer
Ctrl-T           show or hide the tool calls of the last turn
Ctrl-V / Alt-V   paste an image (a screenshot) or text from the clipboard
drag             select and copy; click a tool call to expand it;
                 Shift-drag uses the terminal's own selection
/                list the commands; type to search, Esc closes the list
Ctrl-O           choose the model
Ctrl-S           sessions: attach, new, rename, kill
Ctrl-N / Ctrl-P  next / previous session
PgUp / PgDn      scroll; Ctrl-Home / Ctrl-End: start / end
Esc / Ctrl-C     cancel the turn; Ctrl-C without one clears the input
Ctrl-C twice, or Ctrl-D with empty input: detach
";

/// The keys and the commands that work.
fn help() -> String {
    let built = || COMMANDS.iter().filter(|c| c.built);
    let width = built()
        .map(|c| c.shown().chars().count())
        .max()
        .unwrap_or(0);
    let mut text = format!("{KEYS}\n");
    for command in built() {
        let shown = command.shown();
        let description = command.description;
        text.push_str(&format!("{shown:<width$}  {description}\n"));
    }
    let escape = "//text";
    text.push_str(&format!(
        "{escape:<width$}  Send a message that starts with /\n"
    ));
    text.push_str("\nAny key closes this.");
    text
}

/// The title and text of the open dialog.
fn dialog(app: &App) -> Option<(String, String)> {
    let dialog = app.dialog?;
    Some(match dialog {
        Dialog::Help => (" Help ".to_owned(), help()),
        Dialog::ConfirmKill => (
            " Delete session ".to_owned(),
            format!(
                "{}\n\n[y] yes  [n] no",
                crate::text::sessions::confirm_delete(&[&app.session])
            ),
        ),
    })
}

/// The rows the dialog wants (RFC 0024).
pub fn wanted(app: &App) -> usize {
    dialog(app).map_or(0, |(_, text)| text.lines().count())
}

pub fn draw(app: &App, frame: &mut Frame<'_>, area: Rect) {
    let Some((title, text)) = dialog(app) else {
        return;
    };
    let area = super::open_panel(frame, area, (Line::raw(title), None), &app.settings.look);
    frame.render_widget(Paragraph::new(text), area);
}
