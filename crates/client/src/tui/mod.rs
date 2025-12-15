//! Terminal User Interface for Polyglot-AI client

#![allow(dead_code)]

mod views;

use std::io::{self, Stdout};
use std::time::Duration;

use anyhow::Result;
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame, Terminal,
};
use polyglot_common::{Tool, ToolUsage};

pub struct App {
    pub input: String,
    pub cursor_position: usize,
    pub output: Vec<OutputLine>,
    pub current_tool: Option<Tool>,
    pub connected: bool,
    pub tools: Vec<(Tool, bool)>,
    pub usage: Vec<ToolUsage>,
    pub view: View,
    pub should_quit: bool,
    pub status: String,
    pub scroll_offset: usize,
}

#[derive(Clone)]
pub struct OutputLine {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub line_type: OutputType,
    pub content: String,
}

#[derive(Clone, Copy, PartialEq)]
pub enum OutputType {
    User,
    Assistant,
    System,
    Error,
}

#[derive(Clone, Copy, PartialEq)]
pub enum View {
    Chat,
    Tools,
    Usage,
    Help,
    About,
}

impl Default for App {
    fn default() -> Self {
        Self {
            input: String::new(),
            cursor_position: 0,
            output: Vec::new(),
            current_tool: None,
            connected: false,
            tools: Vec::new(),
            usage: Vec::new(),
            view: View::Chat,
            should_quit: false,
            status: "Disconnected".to_string(),
            scroll_offset: 0,
        }
    }
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_output(&mut self, line_type: OutputType, content: String) {
        self.output.push(OutputLine {
            timestamp: chrono::Utc::now(),
            line_type,
            content,
        });

        if self.output.len() > 0 {
            self.scroll_offset = self.output.len().saturating_sub(1);
        }
    }

    pub fn handle_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<AppAction> {
        match (code, modifiers) {
            (KeyCode::Char('c'), KeyModifiers::CONTROL) |
            (KeyCode::Char('q'), KeyModifiers::CONTROL) => {
                self.should_quit = true;
                Some(AppAction::Quit)
            }

            (KeyCode::F(1), _) => {
                self.view = View::Chat;
                None
            }
            (KeyCode::F(2), _) => {
                self.view = View::Tools;
                None
            }
            (KeyCode::F(3), _) => {
                self.view = View::Usage;
                None
            }
            (KeyCode::F(4), _) => {
                self.view = View::Help;
                None
            }
            (KeyCode::F(5), _) => {
                self.view = View::About;
                None
            }

            (KeyCode::PageUp, _) => {
                self.scroll_offset = self.scroll_offset.saturating_sub(10);
                None
            }
            (KeyCode::PageDown, _) => {
                self.scroll_offset = (self.scroll_offset + 10).min(self.output.len().saturating_sub(1));
                None
            }

            _ if self.view == View::Chat => self.handle_chat_key(code, modifiers),

            _ => None,
        }
    }

    fn handle_chat_key(&mut self, code: KeyCode, _modifiers: KeyModifiers) -> Option<AppAction> {
        match code {
            KeyCode::Enter => {
                if !self.input.is_empty() {
                    let input = self.input.clone();
                    self.input.clear();
                    self.cursor_position = 0;

                    if input.starts_with('/') {
                        return Some(self.handle_command(&input));
                    }

                    self.add_output(OutputType::User, input.clone());
                    return Some(AppAction::SendPrompt(input));
                }
                None
            }

            KeyCode::Char(c) => {
                self.input.insert(self.cursor_position, c);
                self.cursor_position += 1;
                None
            }

            KeyCode::Backspace => {
                if self.cursor_position > 0 {
                    self.cursor_position -= 1;
                    self.input.remove(self.cursor_position);
                }
                None
            }

            KeyCode::Delete => {
                if self.cursor_position < self.input.len() {
                    self.input.remove(self.cursor_position);
                }
                None
            }

            KeyCode::Left => {
                self.cursor_position = self.cursor_position.saturating_sub(1);
                None
            }

            KeyCode::Right => {
                self.cursor_position = (self.cursor_position + 1).min(self.input.len());
                None
            }

            KeyCode::Home => {
                self.cursor_position = 0;
                None
            }

            KeyCode::End => {
                self.cursor_position = self.input.len();
                None
            }

            _ => None,
        }
    }

    fn handle_command(&mut self, input: &str) -> AppAction {
        let parts: Vec<&str> = input[1..].split_whitespace().collect();
        match parts.first().map(|s| *s) {
            Some("usage") => {
                self.view = View::Usage;
                AppAction::RequestUsage
            }
            Some("tools") => {
                self.view = View::Tools;
                AppAction::RequestTools
            }
            Some("switch") => {
                if let Some(tool_name) = parts.get(1) {
                    if let Ok(tool) = tool_name.parse::<Tool>() {
                        return AppAction::SwitchTool(tool);
                    }
                }
                self.add_output(OutputType::Error, "Usage: /switch <tool>".to_string());
                AppAction::None
            }
            Some("help") => {
                self.view = View::Help;
                AppAction::None
            }
            Some("quit") | Some("exit") => {
                self.should_quit = true;
                AppAction::Quit
            }
            Some("clear") => {
                self.output.clear();
                self.scroll_offset = 0;
                AppAction::None
            }
            Some("sync") => {
                let path = parts.get(1).unwrap_or(&".");
                AppAction::Sync(path.to_string())
            }
            Some("about") => {
                self.view = View::About;
                AppAction::None
            }
            Some("update") => {
                self.add_output(OutputType::System, "Checking for updates...".to_string());
                AppAction::CheckUpdate
            }
            _ => {
                self.add_output(OutputType::Error, format!("Unknown command: {}", input));
                AppAction::None
            }
        }
    }

    pub fn set_connected(&mut self, connected: bool, tool: Option<Tool>) {
        self.connected = connected;
        self.current_tool = tool;
        self.status = if connected {
            format!("Connected | Tool: {}", tool.map(|t| t.display_name()).unwrap_or("None"))
        } else {
            "Disconnected".to_string()
        };
    }
}

#[derive(Debug)]
pub enum AppAction {
    None,
    Quit,
    SendPrompt(String),
    RequestUsage,
    RequestTools,
    SwitchTool(Tool),
    Sync(String),
    CheckUpdate,
}

pub fn run_tui(app: &mut App) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_event_loop(&mut terminal, app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|f| draw_ui(f, app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if let Some(_action) = app.handle_key(key.code, key.modifiers) {
                }
            }
        }

        if app.should_quit {
            break;
        }
    }

    Ok(())
}

pub fn draw_ui(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(f.area());

    draw_header(f, chunks[0], app);

    match app.view {
        View::Chat => draw_chat_view(f, chunks[1], app),
        View::Tools => draw_tools_view(f, chunks[1], app),
        View::Usage => draw_usage_view(f, chunks[1], app),
        View::Help => draw_help_view(f, chunks[1], app),
        View::About => draw_about_view(f, chunks[1]),
    }

    if app.view == View::Chat {
        draw_input(f, chunks[2], app);
    } else {
        let block = Block::default().borders(Borders::ALL);
        f.render_widget(block, chunks[2]);
    }

    draw_status_bar(f, chunks[3], app);
}

fn draw_header(f: &mut Frame, area: Rect, app: &App) {
    let tabs = vec![
        if app.view == View::Chat { "[F1 Chat]" } else { " F1 Chat " },
        if app.view == View::Tools { "[F2 Tools]" } else { " F2 Tools " },
        if app.view == View::Usage { "[F3 Usage]" } else { " F3 Usage " },
        if app.view == View::Help { "[F4 Help]" } else { " F4 Help " },
        if app.view == View::About { "[F5 About]" } else { " F5 About " },
    ];

    let header = Line::from(vec![
        Span::styled("Polyglot-AI ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("| "),
        Span::raw(tabs.join(" ")),
    ]);

    let paragraph = Paragraph::new(header)
        .style(Style::default().bg(Color::DarkGray));
    f.render_widget(paragraph, area);
}

fn draw_chat_view(f: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app.output
        .iter()
        .skip(app.scroll_offset.saturating_sub(area.height as usize))
        .take(area.height as usize)
        .map(|line| {
            let style = match line.line_type {
                OutputType::User => Style::default().fg(Color::Green),
                OutputType::Assistant => Style::default().fg(Color::White),
                OutputType::System => Style::default().fg(Color::Yellow),
                OutputType::Error => Style::default().fg(Color::Red),
            };

            let prefix = match line.line_type {
                OutputType::User => "> ",
                OutputType::Assistant => "< ",
                OutputType::System => "* ",
                OutputType::Error => "! ",
            };

            ListItem::new(Line::from(vec![
                Span::styled(prefix, style.add_modifier(Modifier::BOLD)),
                Span::styled(&line.content, style),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Chat"));
    f.render_widget(list, area);
}

fn draw_tools_view(f: &mut Frame, area: Rect, app: &App) {
    let items: Vec<ListItem> = app.tools
        .iter()
        .map(|(tool, available)| {
            let status = if *available { "[OK]" } else { "[--]" };
            let current = if Some(*tool) == app.current_tool { " (current)" } else { "" };
            let style = if *available {
                Style::default().fg(Color::Green)
            } else {
                Style::default().fg(Color::Red)
            };

            ListItem::new(Line::from(vec![
                Span::styled(status, style),
                Span::raw(" "),
                Span::styled(tool.display_name(), Style::default().fg(Color::White)),
                Span::styled(current, Style::default().fg(Color::Yellow)),
            ]))
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title("Available Tools"));
    f.render_widget(list, area);
}

fn draw_usage_view(f: &mut Frame, area: Rect, app: &App) {
    let text: Vec<Line> = app.usage
        .iter()
        .flat_map(|stat| {
            vec![
                Line::from(Span::styled(
                    stat.tool.display_name(),
                    Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
                )),
                Line::from(format!("  Requests:    {}", stat.requests)),
                Line::from(format!("  Tokens:      {}", stat.tokens_used)),
                Line::from(format!("  Errors:      {}", stat.errors)),
                Line::from(format!("  Rate Limits: {}", stat.rate_limit_hits)),
                Line::from(""),
            ]
        })
        .collect();

    let paragraph = Paragraph::new(text)
        .block(Block::default().borders(Borders::ALL).title("Usage Statistics"))
        .wrap(Wrap { trim: true });
    f.render_widget(paragraph, area);
}

fn draw_help_view(f: &mut Frame, area: Rect, _app: &App) {
    let help_text = vec![
        Line::from(Span::styled("Commands:", Style::default().add_modifier(Modifier::BOLD))),
        Line::from("  /usage      - Show usage statistics"),
        Line::from("  /tools      - Show available tools"),
        Line::from("  /switch <t> - Switch to tool (claude, gemini, codex, copilot)"),
        Line::from("  /sync [p]   - Sync files (optional path)"),
        Line::from("  /update     - Check for updates"),
        Line::from("  /clear      - Clear chat history"),
        Line::from("  /about      - About Polyglot-AI"),
        Line::from("  /help       - Show this help"),
        Line::from("  /quit       - Exit the application"),
        Line::from(""),
        Line::from(Span::styled("Keyboard Shortcuts:", Style::default().add_modifier(Modifier::BOLD))),
        Line::from("  F1-F4       - Switch views"),
        Line::from("  Ctrl+C/Q    - Quit"),
        Line::from("  PageUp/Down - Scroll output"),
        Line::from("  Enter       - Send message"),
    ];

    let paragraph = Paragraph::new(help_text)
        .block(Block::default().borders(Borders::ALL).title("Help"))
        .wrap(Wrap { trim: true });
    f.render_widget(paragraph, area);
}

fn draw_about_view(f: &mut Frame, area: Rect) {
    let about_text = vec![
        Line::from(""),
        Line::from(Span::styled(
            r"  ____       _             _       _        _    ___ ",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(Span::styled(
            r" |  _ \ ___ | |_   _  __ _| | ___ | |_     / \  |_ _|",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(Span::styled(
            r" | |_) / _ \| | | | |/ _` | |/ _ \| __|   / _ \  | | ",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(Span::styled(
            r" |  __/ (_) | | |_| | (_| | | (_) | |_   / ___ \ | | ",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(Span::styled(
            r" |_|   \___/|_|\__, |\__, |_|\___/ \__| /_/   \_\___|",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(Span::styled(
            r"               |___/ |___/                           ",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled("  Polyglot-AI", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled("  Self-Hosted AI Code Platform", Style::default().fg(Color::White))),
        Line::from(format!("  Version: {}", env!("CARGO_PKG_VERSION"))),
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━", Style::default().fg(Color::DarkGray))),
        Line::from(""),
        Line::from(Span::styled("  AUTHOR", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(vec![
            Span::styled("  Name:    ", Style::default().fg(Color::Gray)),
            Span::styled("Tugcan Topaloglu", Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("  GitHub:  ", Style::default().fg(Color::Gray)),
            Span::styled("@tugcantopaloglu", Style::default().fg(Color::Cyan)),
        ]),
        Line::from(vec![
            Span::styled("  Website: ", Style::default().fg(Color::Gray)),
            Span::styled("https://tugcan.dev", Style::default().fg(Color::Blue)),
        ]),
        Line::from(vec![
            Span::styled("  Email:   ", Style::default().fg(Color::Gray)),
            Span::styled("tugcantopaloglu@proton.me", Style::default().fg(Color::Magenta)),
        ]),
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━", Style::default().fg(Color::DarkGray))),
        Line::from(""),
        Line::from(Span::styled("  SUPPORT", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(Span::styled("  Found a bug or have a feature request?", Style::default().fg(Color::White))),
        Line::from(""),
        Line::from(vec![
            Span::styled("  → ", Style::default().fg(Color::Green)),
            Span::styled("Please open an issue on GitHub:", Style::default().fg(Color::White)),
        ]),
        Line::from(Span::styled("    https://github.com/tugcantopaloglu/polyglot-ai/issues", Style::default().fg(Color::Cyan))),
        Line::from(""),
        Line::from(Span::styled("  → ", Style::default().fg(Color::Green)),
        ),
        Line::from(Span::styled("  Or reach out via email for direct support.", Style::default().fg(Color::White))),
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━", Style::default().fg(Color::DarkGray))),
        Line::from(""),
        Line::from(Span::styled("  LICENSE", Style::default().fg(Color::Magenta).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(Span::styled("  This project is open source and available under the MIT License.", Style::default().fg(Color::White))),
        Line::from(""),
        Line::from(""),
        Line::from(Span::styled("  Thank you for using Polyglot-AI! ❤️", Style::default().fg(Color::Red))),
        Line::from(""),
    ];

    let paragraph = Paragraph::new(about_text)
        .block(Block::default().borders(Borders::ALL).title("About"))
        .wrap(Wrap { trim: false });
    f.render_widget(paragraph, area);
}

fn draw_input(f: &mut Frame, area: Rect, app: &App) {
    let input = Paragraph::new(app.input.as_str())
        .block(Block::default().borders(Borders::ALL).title("Input"));
    f.render_widget(input, area);

    f.set_cursor_position((
        area.x + app.cursor_position as u16 + 1,
        area.y + 1,
    ));
}

fn draw_status_bar(f: &mut Frame, area: Rect, app: &App) {
    let status_style = if app.connected {
        Style::default().fg(Color::Green)
    } else {
        Style::default().fg(Color::Red)
    };

    let status = Paragraph::new(Line::from(vec![
        Span::styled(&app.status, status_style),
        Span::raw(" | "),
        Span::raw("Ctrl+Q to quit"),
    ]))
    .style(Style::default().bg(Color::DarkGray));

    f.render_widget(status, area);
}
