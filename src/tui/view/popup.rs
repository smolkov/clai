//! Boxes over the screen: help and the confirmation of `/kill`.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::tui::app::{App, Dialog};
use crate::tui::commands::COMMANDS;
use crate::tui::style::Look;

const KEYS: &str = "\
Enter            send the message
Alt-Enter        new line (Shift-Enter where the terminal reports it)
Up / Down        earlier messages; select an entry in the conversation
Tab              switch between input and conversation
Enter            expand a tool call (conversation)
/                list the commands; type to search, Esc closes the list
Ctrl-O           choose the model
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

pub fn draw(app: &App, frame: &mut Frame<'_>, size: Rect) {
    let Some(dialog) = app.dialog else {
        return;
    };
    let (title, text) = match dialog {
        Dialog::Help => (" Help ".to_owned(), help()),
        Dialog::ConfirmKill => (
            " Kill session ".to_owned(),
            format!(
                "End session {} and delete its files?\n\n[y] yes  [n] no",
                app.session.name
            ),
        ),
    };
    let rows = u16::try_from(text.lines().count()).unwrap_or(u16::MAX);
    let cells = text.lines().map(|l| l.chars().count()).max().unwrap_or(0);
    let cells = u16::try_from(cells).unwrap_or(u16::MAX);
    let area = open(frame, size, &title, cells, rows, &app.settings.look);
    frame.render_widget(Paragraph::new(text), area);
}

/// Clears a box for `cells` x `rows` of text in the middle of the screen, as
/// far as it fits, and returns the area for the text.
pub fn open(
    frame: &mut Frame<'_>,
    size: Rect,
    title: &str,
    cells: u16,
    rows: u16,
    look: &Look,
) -> Rect {
    let width = cells.saturating_add(4).min(size.width);
    let height = rows.saturating_add(2).min(size.height);
    let area = Rect::new(
        size.x + (size.width - width) / 2,
        size.y + (size.height - height) / 2,
        width,
        height,
    );
    let block = Block::new()
        .borders(Borders::ALL)
        .border_set(look.symbols.border)
        .border_style(look.styles.border)
        .title(title.to_owned());
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    Rect::new(
        inner.x.saturating_add(1),
        inner.y,
        inner.width.saturating_sub(1),
        inner.height,
    )
}
