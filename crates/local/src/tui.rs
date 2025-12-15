//! Terminal UI for Polyglot-AI Local

use std::collections::HashMap;
use crossterm::event::{KeyCode, KeyModifiers};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};
use unicode_width::UnicodeWidthChar;

use polyglot_common::{Tool, ToolUsage, HistoryEntry};

#[derive(Clone, Default)]
pub struct MultiModelState {
    pub enabled: bool,
    pub selected_tools: Vec<Tool>,
    pub responses: HashMap<Tool, Vec<String>>,
    pub completed: HashMap<Tool, bool>,
}

impl MultiModelState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn enable(&mut self, tools: Vec<Tool>) {
        self.enabled = true;
        self.selected_tools = tools;
        self.clear_responses();
    }

    pub fn disable(&mut self) {
        self.enabled = false;
        self.selected_tools.clear();
        self.clear_responses();
    }

    pub fn clear_responses(&mut self) {
        self.responses.clear();
        self.completed.clear();
        for tool in &self.selected_tools {
            self.responses.insert(*tool, Vec::new());
            self.completed.insert(*tool, false);
        }
    }

    pub fn add_line(&mut self, tool: Tool, line: String) {
        if let Some(lines) = self.responses.get_mut(&tool) {
            lines.push(line);
        }
    }

    pub fn mark_done(&mut self, tool: Tool) {
        self.completed.insert(tool, true);
    }

    pub fn all_done(&self) -> bool {
        !self.selected_tools.is_empty() && self.completed.values().all(|&v| v)
    }

    pub fn toggle_tool(&mut self, tool: Tool) {
        if let Some(pos) = self.selected_tools.iter().position(|t| *t == tool) {
            self.selected_tools.remove(pos);
        } else {
            self.selected_tools.push(tool);
        }
    }
}

pub struct App {
    pub input: String,
    pub cursor_position: usize,
    pub cursor_display_pos: usize,
    pub output: Vec<OutputLine>,
    pub current_tool: Option<Tool>,
    pub tools: Vec<(Tool, bool)>,
    pub usage: Vec<ToolUsage>,
    pub history: Vec<HistoryEntry>,
    pub history_selected: usize,
    pub history_search: String,
    pub view: View,
    pub should_quit: bool,
    pub scroll_offset: usize,
    pub current_response: String,
    pub multi_model: MultiModelState,
}

#[derive(Clone)]
pub struct OutputLine {
    #[allow(dead_code)]
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
    History,
    Help,
    MultiSelect,
    About,
}

#[derive(Debug, Clone)]
pub enum AppAction {
    None,
    Quit,
    SendPrompt(String),
    SendMultiPrompt(String, Vec<Tool>),
    SwitchTool(Tool),
    RequestUsage,
    RequestTools,
    RequestHistory,
    SearchHistory(String),
    NewChat,
    ResumeSession(uuid::Uuid),
    EnableMultiModel(Vec<Tool>),
    DisableMultiModel,
    ToggleMultiTool(Tool),
    CheckUpdate,
    PerformUpdate,
}

impl Default for App {
    fn default() -> Self {
        Self {
            input: String::new(),
            cursor_position: 0,
            cursor_display_pos: 0,
            output: Vec::new(),
            current_tool: None,
            tools: Vec::new(),
            usage: Vec::new(),
            history: Vec::new(),
            history_selected: 0,
            history_search: String::new(),
            view: View::Chat,
            should_quit: false,
            scroll_offset: 0,
            current_response: String::new(),
            multi_model: MultiModelState::new(),
        }
    }
}

impl App {
    pub fn new() -> Self {
        Self::default()
    }

    fn update_cursor_display_pos(&mut self) {
        self.cursor_display_pos = self.input.chars()
            .take(self.cursor_position)
            .map(|c| c.width().unwrap_or(0))
            .sum();
    }

    pub fn add_output(&mut self, line_type: OutputType, content: String) {
        self.output.push(OutputLine {
            timestamp: chrono::Utc::now(),
            line_type,
            content,
        });

        if !self.output.is_empty() {
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

            (KeyCode::F(1), _) => { self.view = View::Chat; None }
            (KeyCode::F(2), _) => { self.view = View::Tools; Some(AppAction::RequestTools) }
            (KeyCode::F(3), _) => { self.view = View::Usage; Some(AppAction::RequestUsage) }
            (KeyCode::F(4), _) => { self.view = View::History; Some(AppAction::RequestHistory) }
            (KeyCode::F(5), _) => { self.view = View::Help; None }
            (KeyCode::F(6), _) => { self.view = View::MultiSelect; Some(AppAction::RequestTools) }
            (KeyCode::F(7), _) => { self.view = View::About; None }

            (KeyCode::Char('n'), KeyModifiers::CONTROL) => {
                Some(AppAction::NewChat)
            }

            (KeyCode::Char('m'), KeyModifiers::CONTROL) => {
                if self.multi_model.enabled {
                    self.multi_model.disable();
                    self.add_output(OutputType::System, "Multi-model mode disabled.".to_string());
                    Some(AppAction::DisableMultiModel)
                } else {
                    self.view = View::MultiSelect;
                    Some(AppAction::RequestTools)
                }
            }

            (KeyCode::PageUp, _) => {
                if self.view == View::History {
                    self.history_selected = self.history_selected.saturating_sub(1);
                } else {
                    self.scroll_offset = self.scroll_offset.saturating_sub(10);
                }
                None
            }
            (KeyCode::PageDown, _) => {
                if self.view == View::History {
                    self.history_selected = (self.history_selected + 1).min(self.history.len().saturating_sub(1));
                } else {
                    self.scroll_offset = (self.scroll_offset + 10).min(self.output.len().saturating_sub(1));
                }
                None
            }

            (KeyCode::Up, _) if self.view == View::History => {
                self.history_selected = self.history_selected.saturating_sub(1);
                None
            }
            (KeyCode::Down, _) if self.view == View::History => {
                self.history_selected = (self.history_selected + 1).min(self.history.len().saturating_sub(1));
                None
            }
            (KeyCode::Enter, _) if self.view == View::History => {
                if let Some(entry) = self.history.get(self.history_selected) {
                    self.view = View::Chat;
                    return Some(AppAction::ResumeSession(entry.session_id));
                }
                None
            }
            (KeyCode::Esc, _) if self.view == View::History => {
                if !self.history_search.is_empty() {
                    self.history_search.clear();
                    Some(AppAction::RequestHistory)
                } else {
                    self.view = View::Chat;
                    None
                }
            }

            _ if self.view == View::MultiSelect => self.handle_multi_select_key(code),

            _ if self.view == View::Chat => self.handle_chat_key(code, modifiers),

            _ => None,
        }
    }

    fn handle_multi_select_key(&mut self, code: KeyCode) -> Option<AppAction> {
        match code {
            KeyCode::Char('1') => Some(AppAction::ToggleMultiTool(Tool::Claude)),
            KeyCode::Char('2') => Some(AppAction::ToggleMultiTool(Tool::Gemini)),
            KeyCode::Char('3') => Some(AppAction::ToggleMultiTool(Tool::Codex)),
            KeyCode::Char('4') => Some(AppAction::ToggleMultiTool(Tool::Copilot)),
            KeyCode::Char('5') => Some(AppAction::ToggleMultiTool(Tool::Perplexity)),
            KeyCode::Char('6') => Some(AppAction::ToggleMultiTool(Tool::Cursor)),
            KeyCode::Char('7') => Some(AppAction::ToggleMultiTool(Tool::Ollama)),
            KeyCode::Enter => {
                if self.multi_model.selected_tools.len() >= 2 {
                    let tools = self.multi_model.selected_tools.clone();
                    self.multi_model.enable(tools.clone());
                    self.view = View::Chat;
                    let names: Vec<_> = tools.iter().map(|t| t.display_name()).collect();
                    self.add_output(OutputType::System,
                        format!("Multi-model mode enabled with: {}", names.join(", ")));
                    Some(AppAction::EnableMultiModel(tools))
                } else {
                    self.add_output(OutputType::Error, "Select at least 2 tools for multi-model mode.".to_string());
                    None
                }
            }
            KeyCode::Esc => {
                self.view = View::Chat;
                None
            }
            _ => None,
        }
    }

    fn handle_chat_key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<AppAction> {
        match code {
            KeyCode::Esc => {
                self.input.clear();
                self.cursor_position = 0;
                None
            }

            KeyCode::Enter => {
                if !self.input.is_empty() {
                    let input = self.input.clone();
                    self.input.clear();
                    self.cursor_position = 0;

                    if input.starts_with('/') {
                        return Some(self.handle_command(&input));
                    }

                    self.add_output(OutputType::User, input.clone());

                    if self.multi_model.enabled && self.multi_model.selected_tools.len() >= 2 {
                        self.multi_model.clear_responses();
                        let tools = self.multi_model.selected_tools.clone();
                        return Some(AppAction::SendMultiPrompt(input, tools));
                    }

                    return Some(AppAction::SendPrompt(input));
                }
                None
            }

            KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.clear();
                self.cursor_position = 0;
                None
            }

            KeyCode::Char(c) => {
                let byte_pos = self.input.char_indices()
                    .nth(self.cursor_position)
                    .map(|(i, _)| i)
                    .unwrap_or(self.input.len());
                self.input.insert(byte_pos, c);
                self.cursor_position += 1;
                self.update_cursor_display_pos();
                None
            }

            KeyCode::Backspace => {
                if self.cursor_position > 0 {
                    self.cursor_position -= 1;
                    if let Some((byte_pos, ch)) = self.input.char_indices().nth(self.cursor_position) {
                        self.input.drain(byte_pos..byte_pos + ch.len_utf8());
                    }
                    self.update_cursor_display_pos();
                }
                None
            }

            KeyCode::Delete => {
                let char_count = self.input.chars().count();
                if self.cursor_position < char_count {
                    if let Some((byte_pos, ch)) = self.input.char_indices().nth(self.cursor_position) {
                        self.input.drain(byte_pos..byte_pos + ch.len_utf8());
                    }
                }
                None
            }

            KeyCode::Left => {
                self.cursor_position = self.cursor_position.saturating_sub(1);
                self.update_cursor_display_pos();
                None
            }

            KeyCode::Right => {
                let char_count = self.input.chars().count();
                self.cursor_position = (self.cursor_position + 1).min(char_count);
                self.update_cursor_display_pos();
                None
            }

            KeyCode::Home => {
                self.cursor_position = 0;
                self.update_cursor_display_pos();
                None
            }

            KeyCode::End => {
                self.cursor_position = self.input.chars().count();
                self.update_cursor_display_pos();
                None
            }

            _ => None,
        }
    }

    fn handle_command(&mut self, input: &str) -> AppAction {
        let command_str = input.strip_prefix('/').unwrap_or(input);
        let parts: Vec<&str> = command_str.split_whitespace().collect();
        match parts.first().copied() {
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
                self.add_output(OutputType::Error, "Usage: /switch <claude|gemini|codex|copilot>".to_string());
                AppAction::None
            }
            Some("help") => {
                self.view = View::Help;
                AppAction::None
            }
            Some("history") | Some("h") => {
                self.view = View::History;
                AppAction::RequestHistory
            }
            Some("search") | Some("s") => {
                if parts.len() > 1 {
                    let query = parts[1..].join(" ");
                    self.history_search = query.clone();
                    self.view = View::History;
                    AppAction::SearchHistory(query)
                } else {
                    self.add_output(OutputType::Error, "Usage: /search <query>".to_string());
                    AppAction::None
                }
            }
            Some("new") | Some("n") => {
                AppAction::NewChat
            }
            Some("title") => {
                if parts.len() > 1 {
                    let title = parts[1..].join(" ");
                    self.add_output(OutputType::System, format!("Session title set: {}", title));
                } else {
                    self.add_output(OutputType::Error, "Usage: /title <name>".to_string());
                }
                AppAction::None
            }
            Some("quit") | Some("exit") | Some("q") => {
                self.should_quit = true;
                AppAction::Quit
            }
            Some("clear") => {
                self.output.clear();
                self.scroll_offset = 0;
                AppAction::None
            }
            Some("multi") => {
                if parts.len() > 1 {
                    let mut tools = Vec::new();
                    for name in &parts[1..] {
                        if let Ok(tool) = name.parse::<Tool>() {
                            tools.push(tool);
                        } else {
                            self.add_output(OutputType::Error, format!("Unknown tool: {}", name));
                            return AppAction::None;
                        }
                    }
                    if tools.len() >= 2 {
                        self.multi_model.enable(tools.clone());
                        let names: Vec<_> = tools.iter().map(|t| t.display_name()).collect();
                        self.add_output(OutputType::System,
                            format!("Multi-model mode enabled with: {}", names.join(", ")));
                        return AppAction::EnableMultiModel(tools);
                    } else {
                        self.add_output(OutputType::Error, "Specify at least 2 tools.".to_string());
                        return AppAction::None;
                    }
                }
                self.view = View::MultiSelect;
                AppAction::RequestTools
            }
            Some("single") => {
                self.multi_model.disable();
                self.add_output(OutputType::System, "Multi-model mode disabled. Back to single tool mode.".to_string());
                AppAction::DisableMultiModel
            }
            Some("about") => {
                self.view = View::About;
                AppAction::None
            }
            Some("update") => {
                self.add_output(OutputType::System, "Checking for updates...".to_string());
                AppAction::PerformUpdate
            }
            _ => {
                self.add_output(OutputType::Error, format!("Unknown command: {}", input));
                AppAction::None
            }
        }
    }
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
        View::Chat => {
            if app.multi_model.enabled && !app.multi_model.responses.is_empty() {
                draw_multi_model_view(f, chunks[1], app);
            } else {
                draw_chat_view(f, chunks[1], app);
            }
        }
        View::Tools => draw_tools_view(f, chunks[1], app),
        View::Usage => draw_usage_view(f, chunks[1], app),
        View::History => draw_history_view(f, chunks[1], app),
        View::Help => draw_help_view(f, chunks[1]),
        View::MultiSelect => draw_multi_select_view(f, chunks[1], app),
        View::About => draw_about_view(f, chunks[1]),
    }

    if app.view == View::Chat || app.view == View::MultiSelect {
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
        if app.view == View::History { "[F4 History]" } else { " F4 History " },
        if app.view == View::Help { "[F5 Help]" } else { " F5 Help " },
        if app.view == View::MultiSelect { "[F6 Multi]" } else { " F6 Multi " },
        if app.view == View::About { "[F7 About]" } else { " F7 About " },
    ];

    let multi_indicator = if app.multi_model.enabled {
        format!(" [MULTI: {}]", app.multi_model.selected_tools.len())
    } else {
        String::new()
    };

    let header = Line::from(vec![
        Span::styled("Polyglot-AI Local ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw("| "),
        Span::raw(tabs.join(" ")),
        Span::styled(multi_indicator, Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
    ]);

    let paragraph = Paragraph::new(header).style(Style::default().bg(Color::DarkGray));
    f.render_widget(paragraph, area);
}

fn draw_chat_view(f: &mut Frame, area: Rect, app: &App) {
    let visible_height = area.height.saturating_sub(2) as usize;
    let start = app.scroll_offset.saturating_sub(visible_height);

    let items: Vec<ListItem> = app.output
        .iter()
        .skip(start)
        .take(visible_height + 1)
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

fn draw_help_view(f: &mut Frame, area: Rect) {
    let help_text = vec![
        Line::from(Span::styled("Commands:", Style::default().add_modifier(Modifier::BOLD))),
        Line::from("  /usage      - Show usage statistics"),
        Line::from("  /tools      - Show available tools"),
        Line::from("  /history    - Show chat history"),
        Line::from("  /new        - Start new chat (with context transfer)"),
        Line::from("  /switch <t> - Switch tool (claude, gemini, codex, copilot, perplexity, cursor, ollama)"),
        Line::from("  /multi      - Open multi-model selection (query multiple AIs at once)"),
        Line::from("  /multi <t1> <t2> ... - Enable multi-model with specific tools"),
        Line::from("  /single     - Return to single-tool mode"),
        Line::from("  /update     - Check for updates"),
        Line::from("  /clear      - Clear chat output"),
        Line::from("  /about      - About Polyglot-AI"),
        Line::from("  /help       - Show this help"),
        Line::from("  /quit       - Exit"),
        Line::from(""),
        Line::from(Span::styled("Keyboard Shortcuts:", Style::default().add_modifier(Modifier::BOLD))),
        Line::from("  F1-F5       - Switch views (Chat, Tools, Usage, History, Help)"),
        Line::from("  F6          - Multi-model tool selection"),
        Line::from("  F7          - About"),
        Line::from("  Ctrl+M      - Toggle multi-model mode"),
        Line::from("  Ctrl+N      - New chat with context transfer"),
        Line::from("  Ctrl+C/Q    - Quit"),
        Line::from("  PageUp/Down - Scroll output / navigate history"),
        Line::from("  Enter       - Send message / select history item"),
        Line::from(""),
        Line::from(Span::styled("Multi-Model Mode:", Style::default().add_modifier(Modifier::BOLD))),
        Line::from("  Query multiple AI tools simultaneously and compare responses"),
        Line::from("  Press 1-7 in selection screen to toggle tools, Enter to confirm"),
        Line::from("  Responses displayed side-by-side in real-time"),
        Line::from(""),
        Line::from(Span::styled("Features:", Style::default().add_modifier(Modifier::BOLD))),
        Line::from("  - Automatic tool switching on rate limit"),
        Line::from("  - Direct local execution (no server needed)"),
        Line::from("  - Context transfer between chats and tools"),
        Line::from("  - Chat history: 5 recent global + all project chats"),
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

fn draw_history_view(f: &mut Frame, area: Rect, app: &App) {
    let title = if app.history_search.is_empty() {
        "Chat History - Enter=resume, Ctrl+N=new, /search <query>".to_string()
    } else {
        format!("Search: \"{}\" - Esc to clear", app.history_search)
    };

    if app.history.is_empty() {
        let message = if app.history_search.is_empty() {
            "No chat history found. Start chatting to build up your history!"
        } else {
            "No sessions match your search."
        };
        let text = vec![
            Line::from(""),
            Line::from(Span::styled(message, Style::default().fg(Color::Gray))),
        ];
        let paragraph = Paragraph::new(text)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: true });
        f.render_widget(paragraph, area);
        return;
    }

    let items: Vec<ListItem> = app.history
        .iter()
        .enumerate()
        .map(|(i, entry)| {
            let is_selected = i == app.history_selected;
            let tool_name = entry.tool.map(|t| t.as_str()).unwrap_or("?");
            let project = entry.project_path.as_deref()
                .map(|p| {
                    std::path::Path::new(p)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or(p)
                })
                .unwrap_or("global");

            let time_ago = format_time_ago(entry.updated_at);
            let session_title = polyglot_common::truncate_smart(&entry.title, 35);

            let style = if is_selected {
                Style::default().bg(Color::DarkGray).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };

            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("[{}] ", tool_name),
                    Style::default().fg(Color::Cyan)
                ),
                Span::styled(
                    session_title,
                    Style::default().add_modifier(Modifier::BOLD)
                ),
                Span::styled(
                    format!(" ({}) ", project),
                    Style::default().fg(Color::Yellow)
                ),
                Span::styled(
                    format!("{} • {} msgs", time_ago, entry.message_count),
                    Style::default().fg(Color::Gray)
                ),
            ])).style(style)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default()
            .borders(Borders::ALL)
            .title(title));
    f.render_widget(list, area);
}

fn draw_multi_select_view(f: &mut Frame, area: Rect, app: &App) {
    let mut items = vec![
        ListItem::new(Line::from(vec![
            Span::styled("SELECT TOOLS FOR MULTI-MODEL MODE", Style::default().add_modifier(Modifier::BOLD)),
        ])),
        ListItem::new(""),
        ListItem::new("Press number keys to toggle tools, Enter to confirm, Esc to cancel:"),
        ListItem::new(""),
    ];

    let all_tools = [
        (Tool::Claude, "1"),
        (Tool::Gemini, "2"),
        (Tool::Codex, "3"),
        (Tool::Copilot, "4"),
        (Tool::Perplexity, "5"),
        (Tool::Cursor, "6"),
        (Tool::Ollama, "7"),
    ];

    for (tool, key) in all_tools {
        let is_selected = app.multi_model.selected_tools.contains(&tool);
        let is_available = app.tools.iter().any(|(t, avail)| *t == tool && *avail);

        let checkbox = if is_selected { "[✓]" } else { "[ ]" };
        let status = if is_available { "" } else { " (unavailable)" };

        let style = if is_selected {
            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
        } else if is_available {
            Style::default().fg(Color::White)
        } else {
            Style::default().fg(Color::DarkGray)
        };

        items.push(ListItem::new(Line::from(vec![
            Span::styled(format!("  {} {} {}{}", key, checkbox, tool.display_name(), status), style),
        ])));
    }

    items.push(ListItem::new(""));
    items.push(ListItem::new(Line::from(vec![
        Span::styled(
            format!("Selected: {} tools", app.multi_model.selected_tools.len()),
            Style::default().fg(Color::Yellow),
        ),
    ])));

    if app.multi_model.selected_tools.len() < 2 {
        items.push(ListItem::new(Line::from(vec![
            Span::styled("(Select at least 2 tools to enable multi-model mode)", Style::default().fg(Color::Red)),
        ])));
    }

    let list = List::new(items)
        .block(Block::default()
            .borders(Borders::ALL)
            .title("Multi-Model Selection"));
    f.render_widget(list, area);
}

fn draw_multi_model_view(f: &mut Frame, area: Rect, app: &App) {
    let num_tools = app.multi_model.selected_tools.len();
    if num_tools == 0 {
        draw_chat_view(f, area, app);
        return;
    }

    let constraints: Vec<Constraint> = (0..num_tools)
        .map(|_| Constraint::Ratio(1, num_tools as u32))
        .collect();

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(constraints)
        .split(area);

    for (i, tool) in app.multi_model.selected_tools.iter().enumerate() {
        let responses = app.multi_model.responses.get(tool)
            .map(|v| v.as_slice())
            .unwrap_or(&[]);

        let is_done = app.multi_model.completed.get(tool).copied().unwrap_or(false);

        let status = if is_done { " ✓" } else { " ..." };
        let title_style = if is_done {
            Style::default().fg(Color::Green)
        } else {
            Style::default().fg(Color::Yellow)
        };

        let title = format!("{}{}", tool.display_name(), status);

        let content: Vec<Line> = responses.iter()
            .flat_map(|line| {
                let max_width = (chunks[i].width.saturating_sub(2)) as usize;
                if line.len() > max_width && max_width > 0 {
                    line.chars()
                        .collect::<Vec<_>>()
                        .chunks(max_width)
                        .map(|chunk| Line::from(chunk.iter().collect::<String>()))
                        .collect::<Vec<_>>()
                } else {
                    vec![Line::from(line.clone())]
                }
            })
            .collect();

        let paragraph = Paragraph::new(content)
            .block(Block::default()
                .borders(Borders::ALL)
                .title(Span::styled(title, title_style))
                .border_style(if is_done {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default().fg(Color::Yellow)
                }))
            .wrap(Wrap { trim: false })
            .scroll((0, 0));

        f.render_widget(paragraph, chunks[i]);
    }
}

fn format_time_ago(time: chrono::DateTime<chrono::Utc>) -> String {
    let now = chrono::Utc::now();
    let duration = now.signed_duration_since(time);

    if duration.num_seconds() < 60 {
        "just now".to_string()
    } else if duration.num_minutes() < 60 {
        format!("{}m", duration.num_minutes())
    } else if duration.num_hours() < 24 {
        format!("{}h", duration.num_hours())
    } else if duration.num_days() < 7 {
        format!("{}d", duration.num_days())
    } else {
        time.format("%m/%d").to_string()
    }
}

fn draw_input(f: &mut Frame, area: Rect, app: &App) {
    let input = Paragraph::new(app.input.as_str())
        .block(Block::default().borders(Borders::ALL).title("Input"));
    f.render_widget(input, area);

    f.set_cursor_position((
        area.x + app.cursor_display_pos as u16 + 1,
        area.y + 1,
    ));
}

fn draw_status_bar(f: &mut Frame, area: Rect, app: &App) {
    let mut spans = Vec::new();

    if app.multi_model.enabled {
        let tool_names: Vec<_> = app.multi_model.selected_tools.iter()
            .map(|t| t.as_str())
            .collect();
        spans.push(Span::styled("Multi-Model: ", Style::default().fg(Color::Yellow)));
        spans.push(Span::styled(tool_names.join(", "), Style::default().fg(Color::Green)));
    } else {
        let tool_name = app.current_tool
            .map(|t| t.display_name())
            .unwrap_or("None");
        spans.push(Span::styled("Tool: ", Style::default().fg(Color::Gray)));
        spans.push(Span::styled(tool_name, Style::default().fg(Color::Cyan)));
    }

    spans.push(Span::raw(" | "));
    spans.push(Span::raw("Ctrl+M multi-mode | Ctrl+Q quit"));

    let status = Paragraph::new(Line::from(spans))
        .style(Style::default().bg(Color::DarkGray));

    f.render_widget(status, area);
}
