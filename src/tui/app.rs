use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::{
    layout::{Constraint, Layout, Rect},
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Paragraph, Wrap},
    DefaultTerminal, Frame,
};
use tui_big_text::{BigText, PixelSize};

use crate::agent::Agent;
use crate::message::Content;

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone)]
struct ChatEntry {
    role: String,
    content: String,
}

#[derive(Debug, Default)]
pub struct App {
    exit: bool,
    input: String,
    cursor_position: usize,
    messages: Vec<ChatEntry>,
    status: Option<String>,
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    /// runs the application's main loop until the user quits
    pub async fn run(&mut self, terminal: &mut DefaultTerminal, agent: &mut Agent) -> io::Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.draw(frame))?;
            self.handle_events(agent).await?;
        }
        Ok(())
    }

    fn draw(&self, frame: &mut Frame) {
        let [header, body, footer] = Layout::vertical([
            Constraint::Length(7),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .areas(frame.area());

        self.render_header(frame, header);
        self.render_conversation(frame, body);
        self.render_input(frame, footer);
    }

    fn render_header(&self, frame: &mut Frame, area: Rect) {
        let [title_area, version_area] =
            Layout::vertical([Constraint::Length(5), Constraint::Length(2)]).areas(area);

        let big_text = BigText::builder()
            .pixel_size(PixelSize::Quadrant)
            .style(Style::new().fg(Color::Cyan).bold())
            .centered()
            .lines(vec!["CLAI".into()])
            .build();
        frame.render_widget(big_text, title_area);

        let version = Line::from(format!("v{VERSION}"))
            .centered()
            .style(Style::new().fg(Color::DarkGray));
        frame.render_widget(Paragraph::new(version), version_area);
    }

    fn render_conversation(&self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered().title(" Conversation ");
        let inner = block.inner(area);
        frame.render_widget(block, area);

        let lines: Vec<Line> = if self.messages.is_empty() {
            vec![Line::from(
                "Type a message below and press Enter to chat with clai.".dim(),
            )]
        } else {
            self.messages
                .iter()
                .flat_map(|entry| {
                    let (prefix, color) = match entry.role.as_str() {
                        "user" => ("You", Color::Yellow),
                        "error" => ("Error", Color::Red),
                        _ => ("Clai", Color::Cyan),
                    };
                    let mut out = vec![Line::from(Span::styled(
                        format!("{prefix}:"),
                        Style::new().fg(color).bold(),
                    ))];
                    out.extend(entry.content.lines().map(|line| Line::from(line.to_string())));
                    out.push(Line::from(""));
                    out
                })
                .collect()
        };

        let total = lines.len() as u16;
        let scroll = total.saturating_sub(inner.height);

        let paragraph = Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0));
        frame.render_widget(paragraph, inner);
    }

    fn render_input(&self, frame: &mut Frame, area: Rect) {
        let title = self.status.clone().unwrap_or_else(|| " Message ".to_string());
        let block = Block::bordered().title(title);
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(Paragraph::new(self.input.as_str()), inner);
        frame.set_cursor_position((inner.x + self.cursor_position as u16, inner.y));
    }

    async fn handle_events(&mut self, agent: &mut Agent) -> io::Result<()> {
        if event::poll(Duration::from_millis(100))? {
            // it's important to check that the event is a key press event as
            // crossterm also emits key release and repeat events on Windows.
            if let Event::Key(key_event) = event::read()? {
                if key_event.kind == KeyEventKind::Press {
                    self.handle_key_event(key_event, agent).await;
                }
            }
        }
        Ok(())
    }

    async fn handle_key_event(&mut self, key_event: KeyEvent, agent: &mut Agent) {
        match key_event.code {
            KeyCode::Esc => self.exit(),
            KeyCode::Char('c') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.exit()
            }
            KeyCode::Enter => self.submit(agent).await,
            KeyCode::Char(c) => self.insert_char(c),
            KeyCode::Backspace => self.delete_char_before(),
            KeyCode::Delete => self.delete_char_after(),
            KeyCode::Left => self.move_cursor_left(),
            KeyCode::Right => self.move_cursor_right(),
            KeyCode::Home => self.cursor_position = 0,
            KeyCode::End => self.cursor_position = self.input.chars().count(),
            _ => {}
        }
    }

    fn exit(&mut self) {
        self.exit = true;
    }

    fn byte_index(&self) -> usize {
        self.input
            .char_indices()
            .nth(self.cursor_position)
            .map(|(i, _)| i)
            .unwrap_or(self.input.len())
    }

    fn insert_char(&mut self, c: char) {
        let index = self.byte_index();
        self.input.insert(index, c);
        self.cursor_position += 1;
    }

    fn delete_char_before(&mut self) {
        if self.cursor_position == 0 {
            return;
        }
        let before: String = self.input.chars().take(self.cursor_position - 1).collect();
        let after: String = self.input.chars().skip(self.cursor_position).collect();
        self.input = before + &after;
        self.cursor_position -= 1;
    }

    fn delete_char_after(&mut self) {
        let len = self.input.chars().count();
        if self.cursor_position >= len {
            return;
        }
        let before: String = self.input.chars().take(self.cursor_position).collect();
        let after: String = self.input.chars().skip(self.cursor_position + 1).collect();
        self.input = before + &after;
    }

    fn move_cursor_left(&mut self) {
        self.cursor_position = self.cursor_position.saturating_sub(1);
    }

    fn move_cursor_right(&mut self) {
        let len = self.input.chars().count();
        self.cursor_position = (self.cursor_position + 1).min(len);
    }

    async fn submit(&mut self, agent: &mut Agent) {
        let text = self.input.trim().to_string();
        if text.is_empty() {
            return;
        }
        self.messages.push(ChatEntry {
            role: "user".to_string(),
            content: text.clone(),
        });
        self.input.clear();
        self.cursor_position = 0;
        self.status = Some(" Thinking... ".to_string());

        match agent.generate(&text).await {
            Ok(message) => {
                let content = match message.content {
                    Content::Text(text) => text,
                    Content::ToolCall(tool_call) => {
                        format!("[calling tool: {}]", tool_call.name)
                    }
                    Content::ToolResult(tool_result) => tool_result.content,
                };
                self.messages.push(ChatEntry {
                    role: "assistant".to_string(),
                    content,
                });
            }
            Err(err) => self.messages.push(ChatEntry {
                role: "error".to_string(),
                content: err.to_string(),
            }),
        }

        self.status = None;
    }
}
