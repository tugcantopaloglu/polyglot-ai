mod config;
mod tools;
mod tui;
mod history;
mod plugins;
mod environment;

use std::path::PathBuf;
use std::io::{self, Write};

use anyhow::Result;
use clap::{Parser, Subcommand};
use tokio::sync::mpsc;
use tracing::error;
use tracing_subscriber::EnvFilter;

use polyglot_common::Tool;
use config::LocalConfig;
use tools::{LocalToolManager, ToolOutput, TaggedOutput};
use tui::{App, AppAction, OutputType};
use history::HistoryManager;

#[derive(Parser)]
#[command(name = "polyglot-local")]
#[command(about = "Polyglot-AI Local - Standalone AI CLI Aggregator")]
#[command(long_about = "
Run multiple AI coding assistants locally without a server.
Supports Claude Code, Gemini CLI, Codex CLI, and GitHub Copilot CLI.
Automatically switches between tools when rate limits are hit.

Made by Tugcan Topaloglu
")]
#[command(version)]
#[command(author = "Tugcan Topaloglu")]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    #[arg(short, long, global = true)]
    verbose: bool,

    #[arg(long, global = true)]
    no_tui: bool,

    #[arg(short, long, global = true)]
    project: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Commands {
    Chat,

    Ask {
        prompt: String,

        #[arg(short, long)]
        tool: Option<String>,

        #[arg(long)]
        with_context: bool,
    },

    Tools,

    Usage,

    History {
        #[arg(short, long, default_value = "10")]
        limit: usize,

        #[arg(short, long)]
        search: Option<String>,
    },

    Init {
        #[arg(short, long, default_value = "polyglot-local.toml")]
        output: PathBuf,
    },

    Doctor,

    Env,

    /// Check for updates and optionally install them
    Update {
        /// Just check for updates without installing
        #[arg(long)]
        check_only: bool,

        /// Force update even if on latest version
        #[arg(long)]
        force: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("warn")
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();

    let config_path = cli.config.unwrap_or_else(LocalConfig::default_path);
    let config = if config_path.exists() {
        LocalConfig::load(&config_path)?
    } else {
        LocalConfig::default()
    };

    let mut history_manager = HistoryManager::new(None)?;
    if let Some(ref project) = cli.project {
        history_manager.set_project(Some(project.to_string_lossy().to_string()));
    } else {
        if let Ok(cwd) = std::env::current_dir() {
            history_manager.set_project(Some(cwd.to_string_lossy().to_string()));
        }
    }

    let tool_manager = LocalToolManager::new(&config);

    match cli.command {
        None | Some(Commands::Chat) => {
            if cli.no_tui {
                run_simple_cli(tool_manager, &config, history_manager).await
            } else {
                run_tui(tool_manager, &config, history_manager).await
            }
        }
        Some(Commands::Ask { prompt, tool, with_context }) => {
            run_single_prompt(tool_manager, &prompt, tool, with_context, &mut history_manager).await
        }
        Some(Commands::Tools) => {
            list_tools(tool_manager).await
        }
        Some(Commands::Usage) => {
            show_usage(&tool_manager)
        }
        Some(Commands::History { limit, search }) => {
            show_history(&history_manager, limit, search)
        }
        Some(Commands::Init { output }) => {
            generate_config(&output)
        }
        Some(Commands::Doctor) => {
            run_doctor(tool_manager).await
        }
        Some(Commands::Env) => {
            show_environment(&tool_manager, &config)
        }
        Some(Commands::Update { check_only, force }) => {
            run_update(check_only, force).await
        }
    }
}

async fn show_splash_screen<B: ratatui::backend::Backend>(terminal: &mut ratatui::Terminal<B>) -> Result<()> {
    use std::time::Duration;
    use ratatui::{
        layout::{Constraint, Direction, Layout, Alignment},
        style::{Color, Modifier, Style},
        text::{Line, Span},
        widgets::{Paragraph, Clear},
    };

    let version = env!("CARGO_PKG_VERSION");

    let logo = vec![
        r"  ____       _             _       _        _    ___ ",
        r" |  _ \ ___ | |_   _  __ _| | ___ | |_     / \  |_ _|",
        r" | |_) / _ \| | | | |/ _` | |/ _ \| __|   / _ \  | | ",
        r" |  __/ (_) | | |_| | (_| | | (_) | |_   / ___ \ | | ",
        r" |_|   \___/|_|\__, |\__, |_|\___/ \__| /_/   \_\___|",
        r"               |___/ |___/                           ",
    ];

    let frames = [
        ("█", Color::Cyan),
        ("▓", Color::Blue),
        ("▒", Color::Magenta),
        ("░", Color::LightBlue),
    ];

    for phase in 0..12 {
        terminal.draw(|f| {
            let area = f.area();
            f.render_widget(Clear, area);

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Percentage(25),
                    Constraint::Length(8),
                    Constraint::Length(3),
                    Constraint::Length(2),
                    Constraint::Length(3),
                    Constraint::Min(0),
                ])
                .split(area);

            let logo_lines: Vec<Line> = logo.iter().enumerate().map(|(i, line)| {
                let visible_chars = if phase < 6 {
                    (line.len() * (phase + 1)) / 6
                } else {
                    line.len()
                };

                let color = if phase >= 6 {
                    Color::Cyan
                } else {
                    frames[(phase + i) % frames.len()].1
                };

                Line::from(Span::styled(
                    &line[..visible_chars.min(line.len())],
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ))
            }).collect();

            let logo_paragraph = Paragraph::new(logo_lines)
                .alignment(Alignment::Center);
            f.render_widget(logo_paragraph, chunks[1]);

            if phase >= 4 {
                let version_style = if phase >= 8 {
                    Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::DarkGray)
                };

                let version_text = Line::from(vec![
                    Span::styled(format!("v{}", version), version_style),
                    Span::raw("  "),
                    Span::styled("LOCAL MODE", Style::default().fg(Color::Yellow)),
                ]);

                let version_paragraph = Paragraph::new(version_text)
                    .alignment(Alignment::Center);
                f.render_widget(version_paragraph, chunks[2]);
            }

            if phase >= 6 {
                let author_style = if phase >= 10 {
                    Style::default().fg(Color::White)
                } else {
                    Style::default().fg(Color::DarkGray)
                };

                let author_text = Line::from(Span::styled(
                    "Made by Tugcan Topaloglu @tugcantopaloglu",
                    author_style,
                ));

                let author_paragraph = Paragraph::new(author_text)
                    .alignment(Alignment::Center);
                f.render_widget(author_paragraph, chunks[3]);
            }

            if phase >= 8 {
                let progress = ((phase - 8) * 25).min(100);
                let bar_width = (area.width as usize).saturating_sub(20);
                let filled = (bar_width * progress) / 100;
                let empty = bar_width.saturating_sub(filled);

                let bar = format!(
                    "[{}{}] {}%",
                    "█".repeat(filled),
                    "░".repeat(empty),
                    progress
                );

                let bar_text = Line::from(Span::styled(bar, Style::default().fg(Color::Cyan)));
                let bar_paragraph = Paragraph::new(bar_text)
                    .alignment(Alignment::Center);
                f.render_widget(bar_paragraph, chunks[4]);
            }
        })?;

        tokio::time::sleep(Duration::from_millis(80)).await;
    }

    terminal.draw(|f| {
        let area = f.area();
        f.render_widget(Clear, area);

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Percentage(25),
                Constraint::Length(8),
                Constraint::Length(3),
                Constraint::Length(2),
                Constraint::Length(3),
                Constraint::Min(0),
            ])
            .split(area);

        let logo_lines: Vec<Line> = logo.iter().map(|line| {
            Line::from(Span::styled(
                *line,
                Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
            ))
        }).collect();

        let logo_paragraph = Paragraph::new(logo_lines)
            .alignment(Alignment::Center);
        f.render_widget(logo_paragraph, chunks[1]);

        let version_text = Line::from(vec![
            Span::styled(format!("v{}", version), Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
            Span::raw("  "),
            Span::styled("LOCAL MODE", Style::default().fg(Color::Yellow)),
        ]);
        let version_paragraph = Paragraph::new(version_text)
            .alignment(Alignment::Center);
        f.render_widget(version_paragraph, chunks[2]);

        let author_text = Line::from(Span::styled(
            "Made by Tugcan Topaloglu @tugcantopaloglu",
            Style::default().fg(Color::White),
        ));
        let author_paragraph = Paragraph::new(author_text)
            .alignment(Alignment::Center);
        f.render_widget(author_paragraph, chunks[3]);

        let bar_width = (area.width as usize).saturating_sub(20);
        let bar = format!("[{}] 100%", "█".repeat(bar_width));
        let bar_text = Line::from(Span::styled(bar, Style::default().fg(Color::Green)));
        let bar_paragraph = Paragraph::new(bar_text)
            .alignment(Alignment::Center);
        f.render_widget(bar_paragraph, chunks[4]);
    })?;

    tokio::time::sleep(Duration::from_millis(400)).await;

    Ok(())
}

async fn run_tui(tool_manager: LocalToolManager, _config: &LocalConfig, mut history_manager: HistoryManager) -> Result<()> {
    use std::time::Duration;
    use crossterm::{
        event::{self, DisableMouseCapture, EnableMouseCapture, Event},
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    };
    use ratatui::{backend::CrosstermBackend, Terminal};

    enable_raw_mode()?;

    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::{
            GetConsoleMode, SetConsoleMode, GetStdHandle,
            STD_INPUT_HANDLE, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT,
            ENABLE_PROCESSED_INPUT,
        };
        use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
        unsafe {
            let handle = GetStdHandle(STD_INPUT_HANDLE);
            if handle != INVALID_HANDLE_VALUE as _ && !handle.is_null() {
                let mut mode: u32 = 0;
                if GetConsoleMode(handle, &mut mode) != 0 {
                    mode &= !(ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT | ENABLE_PROCESSED_INPUT);
                    let _ = SetConsoleMode(handle, mode);
                }
            }
        }
    }
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    show_splash_screen(&mut terminal).await?;

    let mut app = App::new();
    app.add_output(OutputType::System, "Welcome to Polyglot-AI Local!".to_string());
    app.add_output(OutputType::System, "Type your message and press Enter to send.".to_string());
    app.add_output(OutputType::System, "Use /help for commands, /update to check for updates, Ctrl+Q to quit.".to_string());

    // Check for updates in background
    tokio::spawn(async {
        if let Some(msg) = check_updates_on_startup().await {
            // We can't directly add to app here, so we'll print to stderr
            // The user will see it on next interaction
            eprintln!("\n{}", msg);
        }
    });

    if let Ok(history) = history_manager.get_accessible_history() {
        app.history = history;
    }

    let available = tool_manager.check_available().await;
    if available.is_empty() {
        app.add_output(OutputType::Error, "No AI tools found! Run 'polyglot-local doctor' to check.".to_string());
    } else {
        let tool_names: Vec<_> = available.iter().map(|t| t.display_name()).collect();
        app.add_output(OutputType::System, format!("Available tools: {}", tool_names.join(", ")));
        app.current_tool = Some(available[0]);
        app.tools = available.iter().map(|t| (*t, true)).collect();

        history_manager.set_tool(available[0]);
    }

    let (response_tx, mut response_rx) = mpsc::channel::<ToolOutput>(100);
    let (multi_tx, mut multi_rx) = mpsc::channel::<TaggedOutput>(100);

    let result = async {
        loop {
            terminal.draw(|f| tui::draw_ui(f, &app))?;

            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(50)) => {
                    if event::poll(Duration::from_millis(0))? {
                        if let Event::Key(key) = event::read()? {
                            if key.kind != event::KeyEventKind::Press {
                                continue;
                            }
                            if let Some(action) = app.handle_key(key.code, key.modifiers) {
                                match action {
                                    AppAction::Quit => break,
                                    AppAction::SendPrompt(message) => {
                                        history_manager.add_user_message(message.clone());
                                        app.current_response.clear();

                                        let prompt_with_context = if let Some(context) = history_manager.get_transfer_context() {
                                            if !context.summary.is_empty() && history_manager.current_session().messages.len() <= 2 {
                                                context.as_prompt_prefix()
                                            } else {
                                                message.clone()
                                            }
                                        } else {
                                            message.clone()
                                        };

                                        let tx = response_tx.clone();
                                        let tool = app.current_tool;

                                        let mut tm = tool_manager.clone();
                                        tokio::spawn(async move {
                                            if let Err(e) = tm.execute_streaming(&prompt_with_context, tool, tx.clone()).await {
                                                let _ = tx.send(ToolOutput::Error(format!("Tool execution error: {}", e))).await;
                                            }
                                        });
                                    }
                                    AppAction::SwitchTool(tool) => {
                                        if tool_manager.is_available(tool).await {
                                            app.current_tool = Some(tool);
                                            history_manager.set_tool(tool);

                                            history_manager.auto_summarize();

                                            app.add_output(OutputType::System,
                                                format!("Switched to {} (context preserved)", tool.display_name()));
                                        } else {
                                            app.add_output(OutputType::Error,
                                                format!("{} is not available", tool.display_name()));
                                        }
                                    }
                                    AppAction::RequestUsage => {
                                        let stats = tool_manager.get_usage();
                                        app.usage = stats;
                                        app.view = tui::View::Usage;
                                    }
                                    AppAction::RequestTools => {
                                        let available = tool_manager.check_available().await;
                                        app.tools = Tool::all().iter()
                                            .map(|t| (*t, available.contains(t)))
                                            .collect();
                                        app.view = tui::View::Tools;
                                    }
                                    AppAction::RequestHistory => {
                                        app.history_search.clear();
                                        if let Ok(history) = history_manager.get_accessible_history() {
                                            app.history = history;
                                            app.history_selected = 0;
                                        }
                                        app.view = tui::View::History;
                                    }
                                    AppAction::SearchHistory(query) => {
                                        if let Ok(results) = history_manager.search(&query) {
                                            app.history = results;
                                            app.history_selected = 0;
                                        }
                                        app.view = tui::View::History;
                                    }
                                    AppAction::NewChat => {
                                        if let Some(context) = history_manager.transfer_to_new_session() {
                                            app.output.clear();
                                            app.scroll_offset = 0;
                                            app.add_output(OutputType::System,
                                                "New chat started with context from previous session.".to_string());
                                            if !context.summary.is_empty() {
                                                app.add_output(OutputType::System,
                                                    format!("Context: {}", polyglot_common::truncate_smart(&context.summary, 100)));
                                            }
                                        } else {
                                            history_manager.new_session();
                                            app.output.clear();
                                            app.scroll_offset = 0;
                                            app.add_output(OutputType::System, "New chat started.".to_string());
                                        }
                                        app.view = tui::View::Chat;
                                    }
                                    AppAction::ResumeSession(session_id) => {
                                        match history_manager.resume_session(session_id) {
                                            Ok(session) => {
                                                app.output.clear();
                                                app.scroll_offset = 0;
                                                app.add_output(OutputType::System,
                                                    format!("Resumed session from {}",
                                                        session.updated_at.format("%Y-%m-%d %H:%M")));

                                                for msg in session.last_messages(6) {
                                                    let output_type = match msg.role {
                                                        polyglot_common::MessageRole::User => OutputType::User,
                                                        polyglot_common::MessageRole::Assistant => OutputType::Assistant,
                                                        polyglot_common::MessageRole::System => OutputType::System,
                                                    };
                                                    app.add_output(output_type, msg.content.clone());
                                                }

                                                if let Some(tool) = session.tool {
                                                    app.current_tool = Some(tool);
                                                }
                                            }
                                            Err(e) => {
                                                app.add_output(OutputType::Error,
                                                    format!("Failed to resume session: {}", e));
                                            }
                                        }
                                        app.view = tui::View::Chat;
                                    }
                                    AppAction::EnableMultiModel(tools) => {
                                        app.multi_model.enable(tools.clone());
                                        let names: Vec<_> = tools.iter().map(|t| t.display_name()).collect();
                                        app.add_output(OutputType::System,
                                            format!("Multi-model mode enabled with: {}", names.join(", ")));
                                    }
                                    AppAction::DisableMultiModel => {
                                        app.multi_model.disable();
                                    }
                                    AppAction::ToggleMultiTool(tool) => {
                                        app.multi_model.toggle_tool(tool);
                                    }
                                    AppAction::SendMultiPrompt(message, tools) => {
                                        history_manager.add_user_message(message.clone());
                                        app.multi_model.clear_responses();

                                        let tx = multi_tx.clone();
                                        let mut tm = tool_manager.clone();
                                        let prompt = message.clone();
                                        let selected_tools = tools.clone();

                                        tokio::spawn(async move {
                                            if let Err(e) = tm.execute_multi_streaming(&prompt, selected_tools.clone(), tx.clone()).await {
                                                for tool in selected_tools {
                                                    let _ = tx.send(TaggedOutput {
                                                        tool,
                                                        output: ToolOutput::Error(format!("Execution error: {}", e)),
                                                    }).await;
                                                }
                                            }
                                        });
                                    }
                                    AppAction::CheckUpdate => {
                                        match check_for_updates_github("polyglot-local").await {
                                            Ok(info) => {
                                                if info.update_available {
                                                    app.add_output(OutputType::System,
                                                        format!("Update available: v{} -> v{}", info.current_version, info.latest_version));
                                                    app.add_output(OutputType::System,
                                                        "   Use /update to install.".to_string());
                                                } else {
                                                    app.add_output(OutputType::System,
                                                        format!("You are running the latest version (v{})", info.current_version));
                                                }
                                            }
                                            Err(e) => {
                                                app.add_output(OutputType::Error, format!("Failed to check for updates: {}", e));
                                            }
                                        }
                                    }
                                    AppAction::PerformUpdate => {
                                        match check_for_updates_github("polyglot-local").await {
                                            Ok(info) => {
                                                if info.update_available {
                                                    app.add_output(OutputType::System,
                                                        format!("Updating: v{} -> v{}", info.current_version, info.latest_version));
                                                    match perform_update_tui(&info, &mut app).await {
                                                        Ok(true) => {
                                                            app.add_output(OutputType::System, "Update installed! Please restart the application.".to_string());
                                                            app.should_quit = true;
                                                        }
                                                        Ok(false) => {}
                                                        Err(e) => {
                                                            app.add_output(OutputType::Error, format!("Update failed: {}", e));
                                                        }
                                                    }
                                                } else {
                                                    app.add_output(OutputType::System,
                                                        format!("You are running the latest version (v{})", info.current_version));
                                                }
                                            }
                                            Err(e) => {
                                                app.add_output(OutputType::Error, format!("Failed to check for updates: {}", e));
                                            }
                                        }
                                    }
                                    AppAction::None => {}
                                }
                            }
                        }
                    }
                }

                Some(tagged) = multi_rx.recv() => {
                    let tool = tagged.tool;
                    match tagged.output {
                        ToolOutput::Stdout(line) => {
                            app.multi_model.add_line(tool, line);
                        }
                        ToolOutput::Stderr(line) => {
                            app.multi_model.add_line(tool, format!("[stderr] {}", line));
                        }
                        ToolOutput::Done { tool, tokens: _ } => {
                            app.multi_model.mark_done(tool);

                            if app.multi_model.all_done() {
                                let mut combined = String::new();
                                for t in &app.multi_model.selected_tools {
                                    if let Some(lines) = app.multi_model.responses.get(t) {
                                        combined.push_str(&format!("\n--- {} ---\n", t.display_name()));
                                        combined.push_str(&lines.join("\n"));
                                    }
                                }
                                if !combined.is_empty() {
                                    history_manager.add_assistant_message(combined.trim().to_string());
                                }
                                app.add_output(OutputType::System, "All models completed.".to_string());
                            }
                        }
                        ToolOutput::Error(e) => {
                            app.multi_model.add_line(tool, format!("[ERROR] {}", e));
                            app.multi_model.mark_done(tool);
                        }
                        ToolOutput::RateLimited { tool, next_tool: _ } => {
                            app.multi_model.add_line(tool, "[Rate limited]".to_string());
                            app.multi_model.mark_done(tool);
                        }
                    }
                }

                Some(output) = response_rx.recv() => {
                    match output {
                        ToolOutput::Stdout(line) => {
                            app.current_response.push_str(&line);
                            app.current_response.push('\n');
                            app.add_output(OutputType::Assistant, line);
                        }
                        ToolOutput::Stderr(line) => {
                            app.add_output(OutputType::System, format!("[stderr] {}", line));
                        }
                        ToolOutput::Done { tool, tokens } => {
                            if !app.current_response.is_empty() {
                                history_manager.add_assistant_message(app.current_response.trim().to_string());
                                app.current_response.clear();

                                history_manager.auto_summarize();
                            }

                            if let Some(t) = tokens {
                                app.add_output(OutputType::System, format!("({} - {} tokens)", tool.display_name(), t));
                            }
                        }
                        ToolOutput::Error(e) => {
                            app.add_output(OutputType::Error, e);
                        }
                        ToolOutput::RateLimited { tool, next_tool } => {
                            history_manager.auto_summarize();

                            app.add_output(OutputType::System,
                                format!("{} rate limited. Switching to {} (context preserved)...",
                                    tool.display_name(),
                                    next_tool.map(|t| t.display_name()).unwrap_or("none")));
                            if let Some(next) = next_tool {
                                app.current_tool = Some(next);
                                history_manager.set_tool(next);
                            }
                        }
                    }
                }
            }

            if app.should_quit {
                break;
            }
        }

        let _ = history_manager.save_current();

        Ok::<(), anyhow::Error>(())
    }.await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

async fn run_simple_cli(tool_manager: LocalToolManager, _config: &LocalConfig, mut history_manager: HistoryManager) -> Result<()> {
    println!("Polyglot-AI Local v{}", env!("CARGO_PKG_VERSION"));
    println!("Type your message and press Enter. Use /quit to exit.\n");

    let available = tool_manager.check_available().await;
    if available.is_empty() {
        println!("Warning: No AI tools found! Run 'polyglot-local doctor' to check.\n");
    } else {
        let tool_names: Vec<_> = available.iter().map(|t| t.display_name()).collect();
        println!("Available: {}\n", tool_names.join(", "));
    }

    let mut current_tool = available.first().copied();
    if let Some(tool) = current_tool {
        history_manager.set_tool(tool);
    }

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        let tool_name = current_tool.map(|t| t.display_name()).unwrap_or("none");
        print!("[{}] > ", tool_name);
        stdout.flush()?;

        let mut input = String::new();
        if stdin.read_line(&mut input)? == 0 {
            break;
        }

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        if input.starts_with('/') {
            match input {
                "/quit" | "/exit" | "/q" => break,
                "/new" | "/n" => {
                    if let Some(context) = history_manager.transfer_to_new_session() {
                        println!("\nNew chat started with context.");
                        if !context.summary.is_empty() {
                            println!("Context: {}\n", polyglot_common::truncate_smart(&context.summary, 100));
                        }
                    } else {
                        history_manager.new_session();
                        println!("\nNew chat started.\n");
                    }
                }
                "/history" | "/h" => {
                    println!("\nRecent sessions:");
                    if let Ok(history) = history_manager.get_accessible_history() {
                        for (i, entry) in history.iter().take(10).enumerate() {
                            let tool_name = entry.tool.map(|t| t.as_str()).unwrap_or("?");
                            println!("  {}. [{}] {} ({} msgs)",
                                i + 1,
                                tool_name,
                                polyglot_common::truncate_smart(&entry.preview, 50),
                                entry.message_count);
                        }
                    }
                    println!();
                }
                "/tools" => {
                    println!("\nAvailable tools:");
                    for tool in Tool::all() {
                        let status = if tool_manager.is_available(*tool).await { "[OK]" } else { "[--]" };
                        let current = if Some(*tool) == current_tool { " (current)" } else { "" };
                        println!("  {} {}{}", status, tool.display_name(), current);
                    }
                    println!();
                }
                "/usage" => {
                    println!("\nUsage Statistics:");
                    for stat in tool_manager.get_usage() {
                        println!("  {}:", stat.tool.display_name());
                        println!("    Requests: {}", stat.requests);
                        println!("    Tokens:   {}", stat.tokens_used);
                        println!("    Errors:   {}", stat.errors);
                    }
                    println!();
                }
                "/help" => {
                    println!("\nCommands:");
                    println!("  /tools          - List available tools");
                    println!("  /switch <tool>  - Switch tool (claude, gemini, codex, copilot)");
                    println!("  /new            - Start new chat with context transfer");
                    println!("  /history        - Show recent chat history");
                    println!("  /search <query> - Search chat history");
                    println!("  /title <name>   - Set current session title");
                    println!("  /usage          - Show usage statistics");
                    println!("  /quit           - Exit");
                    println!();
                }
                _ if input.starts_with("/search ") => {
                    let query = input.strip_prefix("/search ").unwrap().trim();
                    println!("\nSearch results for: \"{}\"\n", query);
                    if let Ok(results) = history_manager.search(query) {
                        if results.is_empty() {
                            println!("  No matching sessions found.");
                        } else {
                            for (i, entry) in results.iter().take(10).enumerate() {
                                let tool_name = entry.tool.map(|t| t.as_str()).unwrap_or("?");
                                println!("  {}. [{}] {} ({} msgs)",
                                    i + 1,
                                    tool_name,
                                    polyglot_common::truncate_smart(&entry.title, 40),
                                    entry.message_count);
                            }
                        }
                    }
                    println!();
                }
                _ if input.starts_with("/title ") => {
                    let title = input.strip_prefix("/title ").unwrap().trim();
                    history_manager.set_session_title(title.to_string());
                    println!("Session title set: {}\n", title);
                }
                _ if input.starts_with("/switch ") => {
                    let tool_name = input.strip_prefix("/switch ").unwrap().trim();
                    match tool_name.parse::<Tool>() {
                        Ok(tool) => {
                            if tool_manager.is_available(tool).await {
                                current_tool = Some(tool);
                                history_manager.set_tool(tool);
                                history_manager.auto_summarize();
                                println!("Switched to {} (context preserved)\n", tool.display_name());
                            } else {
                                println!("Error: {} is not available\n", tool.display_name());
                            }
                        }
                        Err(_) => println!("Unknown tool: {}\n", tool_name),
                    }
                }
                _ => println!("Unknown command. Type /help for help.\n"),
            }
            continue;
        }

        history_manager.add_user_message(input.to_string());

        let (tx, mut rx) = mpsc::channel(100);
        let prompt = input.to_string();
        let tool = current_tool;

        let mut tm = tool_manager.clone();
        let handle = tokio::spawn(async move {
            tm.execute_streaming(&prompt, tool, tx).await
        });

        let mut response_buffer = String::new();

        while let Some(output) = rx.recv().await {
            match output {
                ToolOutput::Stdout(line) => {
                    response_buffer.push_str(&line);
                    response_buffer.push('\n');
                    println!("{}", line);
                }
                ToolOutput::Stderr(line) => {
                    eprintln!("[stderr] {}", line);
                }
                ToolOutput::Done { tool, tokens } => {
                    if !response_buffer.is_empty() {
                        history_manager.add_assistant_message(response_buffer.trim().to_string());
                        history_manager.auto_summarize();
                    }

                    if let Some(t) = tokens {
                        println!("\n({} - {} tokens)", tool.display_name(), t);
                    }
                }
                ToolOutput::Error(e) => {
                    eprintln!("Error: {}", e);
                }
                ToolOutput::RateLimited { tool, next_tool } => {
                    history_manager.auto_summarize();
                    println!("\n{} rate limited.", tool.display_name());
                    if let Some(next) = next_tool {
                        println!("Switching to {} (context preserved)...", next.display_name());
                        current_tool = Some(next);
                        history_manager.set_tool(next);
                    }
                }
            }
        }

        handle.await??;
        println!();
    }

    println!("\nGoodbye!");
    Ok(())
}

async fn run_single_prompt(
    mut tool_manager: LocalToolManager,
    prompt: &str,
    tool: Option<String>,
    with_context: bool,
    history_manager: &mut HistoryManager,
) -> Result<()> {
    let tool = match tool {
        Some(name) => Some(name.parse::<Tool>()
            .map_err(|_| anyhow::anyhow!("Unknown tool: {}", name))?),
        None => None,
    };

    let full_prompt = if with_context {
        if let Some(context) = history_manager.get_transfer_context() {
            context.as_prompt_prefix()
        } else {
            prompt.to_string()
        }
    } else {
        prompt.to_string()
    };

    history_manager.add_user_message(prompt.to_string());
    if let Some(t) = tool {
        history_manager.set_tool(t);
    }

    let (tx, mut rx) = mpsc::channel(100);

    let prompt_clone = full_prompt.clone();
    let handle = tokio::spawn(async move {
        tool_manager.execute_streaming(&prompt_clone, tool, tx).await
    });

    let mut response_buffer = String::new();

    while let Some(output) = rx.recv().await {
        match output {
            ToolOutput::Stdout(line) => {
                response_buffer.push_str(&line);
                response_buffer.push('\n');
                println!("{}", line);
            }
            ToolOutput::Stderr(line) => {
                eprintln!("{}", line);
            }
            ToolOutput::Done { .. } => {
                if !response_buffer.is_empty() {
                    history_manager.add_assistant_message(response_buffer.trim().to_string());
                }
            }
            ToolOutput::Error(e) => {
                eprintln!("Error: {}", e);
                std::process::exit(1);
            }
            ToolOutput::RateLimited { tool, next_tool } => {
                eprintln!("\n{} rate limited.", tool.display_name());
                if let Some(next) = next_tool {
                    eprintln!("Consider switching to: {}", next.display_name());
                }
            }
        }
    }

    handle.await??;
    Ok(())
}

fn show_history(history_manager: &HistoryManager, limit: usize, search: Option<String>) -> Result<()> {
    println!("Polyglot-AI Local - Chat History\n");

    let history = if let Some(ref query) = search {
        println!("Search results for: \"{}\"\n", query);
        history_manager.search(query)?
    } else {
        history_manager.get_accessible_history()?
    };

    if history.is_empty() {
        if search.is_some() {
            println!("No sessions match your search.");
        } else {
            println!("No chat history found.");
            println!("Start chatting to build up your history!");
        }
        println!();
        return Ok(());
    }

    if search.is_some() {
        for entry in history.iter().take(limit) {
            print_history_entry(entry);
        }
        println!();
        return Ok(());
    }

    let project_path = history_manager.current_project();
    let (project_sessions, global_sessions): (Vec<_>, Vec<_>) = history.iter()
        .partition(|e| e.project_path.as_deref() == project_path);

    if !project_sessions.is_empty() {
        println!("Current Project Sessions:");
        for entry in project_sessions.iter().take(limit) {
            print_history_entry(entry);
        }
        println!();
    }

    if !global_sessions.is_empty() {
        println!("Recent Global Sessions:");
        for entry in global_sessions.iter().take(5) {
            print_history_entry(entry);
        }
        println!();
    }

    println!("Use 'polyglot-local chat' and /history to resume a session.");
    println!("Use 'polyglot-local history --search <query>' to search.");
    Ok(())
}

fn print_history_entry(entry: &polyglot_common::HistoryEntry) {
    let tool_name = entry.tool.map(|t| t.as_str()).unwrap_or("?");
    let project = entry.project_path.as_deref()
        .map(|p| {
            std::path::Path::new(p)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("project")
        })
        .unwrap_or("global");

    let title = polyglot_common::truncate_smart(&entry.title, 35);

    println!("  \x1b[1m{}\x1b[0m", title);
    println!("    [{}] {} • {} messages",
        tool_name,
        project,
        entry.message_count
    );
}

async fn list_tools(tool_manager: LocalToolManager) -> Result<()> {
    println!("Polyglot-AI Local - Available Tools\n");

    for tool in Tool::all() {
        let available = tool_manager.is_available(*tool).await;
        let status = if available { "[OK]" } else { "[--]" };
        let color = if available { "\x1b[32m" } else { "\x1b[31m" };
        println!("{}{} {}\x1b[0m", color, status, tool.display_name());
    }

    println!();
    Ok(())
}

fn show_usage(tool_manager: &LocalToolManager) -> Result<()> {
    println!("Polyglot-AI Local - Usage Statistics\n");

    let stats = tool_manager.get_usage();
    if stats.is_empty() {
        println!("No usage data yet.");
        return Ok(());
    }

    for stat in stats {
        println!("{}:", stat.tool.display_name());
        println!("  Requests:    {}", stat.requests);
        println!("  Tokens:      {}", stat.tokens_used);
        println!("  Errors:      {}", stat.errors);
        println!("  Rate Limits: {}", stat.rate_limit_hits);
        if let Some(last) = stat.last_used {
            println!("  Last Used:   {}", last.format("%Y-%m-%d %H:%M:%S"));
        }
        println!();
    }

    Ok(())
}

fn generate_config(output: &PathBuf) -> Result<()> {
    let content = config::generate_example_config();
    std::fs::write(output, content)?;
    println!("Generated configuration file: {:?}", output);
    println!("\nEdit this file to customize tool paths and settings.");
    Ok(())
}

async fn run_doctor(tool_manager: LocalToolManager) -> Result<()> {
    println!("Polyglot-AI Local - System Check\n");
    println!("Checking installed tools...\n");

    let mut all_ok = false;

    for tool in Tool::all() {
        print!("  {} ... ", tool.display_name());
        io::stdout().flush()?;

        if tool_manager.is_available(*tool).await {
            println!("\x1b[32mOK\x1b[0m");
            all_ok = true;
        } else {
            println!("\x1b[31mNot Found\x1b[0m");
        }
    }

    println!();

    if all_ok {
        println!("\x1b[32mAt least one tool is available. You're ready to go!\x1b[0m");
    } else {
        println!("\x1b[31mNo tools found!\x1b[0m");
        println!("\nTo install tools, run one of:");
        println!("  Linux/macOS: ./scripts/install-tools.sh");
        println!("  Windows:     .\\scripts\\install-tools.ps1");
    }

    println!("\nFor more information, see the README.md file.");
    Ok(())
}

fn show_environment(tool_manager: &LocalToolManager, config: &LocalConfig) -> Result<()> {
    println!("Polyglot-AI Local - Environment Status\n");

    let isolation_status = if config.isolation.enabled {
        "\x1b[32mEnabled\x1b[0m"
    } else {
        "\x1b[33mDisabled\x1b[0m"
    };
    println!("Isolation Mode: {}", isolation_status);
    println!("Tools Directory: {:?}", config.isolation.tools_dir);
    println!("Auto-install: {}\n", config.isolation.auto_install);

    println!("{:<15} {:^12} {:^12} {:^12}", "Tool", "Isolated", "System", "Active");
    println!("{}", "-".repeat(55));

    let status_list = tool_manager.environment().status();
    for status in status_list {
        let isolated = if status.isolated_installed {
            "\x1b[32m✓\x1b[0m"
        } else {
            "\x1b[31m✗\x1b[0m"
        };
        let system = if status.system_available {
            "\x1b[32m✓\x1b[0m"
        } else {
            "\x1b[31m✗\x1b[0m"
        };

        let tool_config = match status.tool {
            polyglot_common::Tool::Claude => config.tools.claude.as_ref(),
            polyglot_common::Tool::Gemini => config.tools.gemini.as_ref(),
            polyglot_common::Tool::Codex => config.tools.codex.as_ref(),
            polyglot_common::Tool::Copilot => config.tools.copilot.as_ref(),
            polyglot_common::Tool::Perplexity => config.tools.perplexity.as_ref(),
            polyglot_common::Tool::Cursor => config.tools.cursor.as_ref(),
            polyglot_common::Tool::Ollama => config.tools.ollama.as_ref(),
        };

        let active = match tool_config {
            Some(tc) if tc.use_isolated && status.isolated_installed => "Isolated",
            Some(tc) if !tc.use_isolated && status.system_available => "System",
            Some(_) if status.system_available => "System",
            Some(_) if status.isolated_installed => "Isolated",
            _ => "-",
        };

        println!("{:<15} {:^12} {:^12} {:^12}",
            status.tool.display_name(),
            isolated,
            system,
            active
        );
    }

    println!("\n\x1b[33mTip:\x1b[0m Set 'use_isolated = true' per-tool in config to use Polyglot's");
    println!("     own tool installation instead of your system PATH.");
    println!("\n     To enable globally: set '[isolation] enabled = true' in config.");

    Ok(())
}

/// Check for updates and optionally install them
async fn run_update(check_only: bool, force: bool) -> Result<()> {
    use polyglot_common::updater::*;

    println!();
    println!("\x1b[36m  ____       _             _       _        _    ___ \x1b[0m");
    println!("\x1b[36m |  _ \\ ___ | |_   _  __ _| | ___ | |_     / \\  |_ _|\x1b[0m");
    println!("\x1b[36m | |_) / _ \\| | | | |/ _` | |/ _ \\| __|   / _ \\  | | \x1b[0m");
    println!("\x1b[36m |  __/ (_) | | |_| | (_| | | (_) | |_   / ___ \\ | | \x1b[0m");
    println!("\x1b[36m |_|   \\___/|_|\\__, |\\__, |_|\\___/ \\__| /_/   \\_\\___|\x1b[0m");
    println!("\x1b[36m               |___/ |___/                           \x1b[0m");
    println!();
    println!("                    \x1b[33mUpdate Manager\x1b[0m");
    println!();

    let current_version = env!("CARGO_PKG_VERSION");
    println!("Current version: \x1b[32mv{}\x1b[0m", current_version);
    println!();

    print_status(&UpdateStatus {
        phase: UpdatePhase::Checking,
        message: "Checking for updates...".to_string(),
        progress: None,
    });

    let update_info = check_for_updates_github("polyglot-local").await?;

    if !update_info.update_available && !force {
        println!();
        println!("\x1b[32m✓ You are running the latest version!\x1b[0m");
        return Ok(());
    }

    println!();
    if update_info.update_available {
        println!("\x1b[33m🆕 New version available: v{}\x1b[0m", update_info.latest_version);
    } else {
        println!("\x1b[33m⚠ Force update requested for v{}\x1b[0m", update_info.latest_version);
    }

    if let Some(ref notes) = update_info.release_notes {
        println!();
        println!("Release notes:");
        for line in notes.lines().take(10) {
            println!("  {}", line);
        }
        if notes.lines().count() > 10 {
            println!("  ...");
        }
    }

    if check_only {
        println!();
        if let Some(ref url) = update_info.download_url {
            println!("Download: {}", url);
        }
        println!("\nRun 'polyglot-local update' to install the update.");
        return Ok(());
    }

    println!();
    print!("Do you want to install the update? [y/N]: ");
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;

    if !input.trim().eq_ignore_ascii_case("y") {
        println!("Update cancelled.");
        return Ok(());
    }

    // Perform the update
    perform_update(&update_info).await
}

/// Check for updates from GitHub releases
async fn check_for_updates_github(binary_name: &str) -> Result<polyglot_common::updater::UpdateInfo> {
    use polyglot_common::updater::*;

    let client = reqwest::Client::builder()
        .user_agent("polyglot-ai-updater")
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let url = "https://api.github.com/repos/tugcantopaloglu/polyglot-ai/releases/latest";

    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        anyhow::bail!("Failed to check for updates: HTTP {}", response.status());
    }

    let release: GitHubRelease = response.json().await?;

    let current_version = env!("CARGO_PKG_VERSION");
    let latest_version = release.tag_name.trim_start_matches('v').to_string();

    let update_available = version_compare(&latest_version, current_version) == std::cmp::Ordering::Greater;

    let asset_name = get_platform_asset_name(binary_name);
    let (download_url, found_asset) = release.assets.iter()
        .find(|a| a.name == asset_name || a.name.contains(&asset_name.replace(".exe", "")))
        .map(|a| (Some(a.browser_download_url.clone()), Some(a.name.clone())))
        .unwrap_or((None, None));

    Ok(UpdateInfo {
        current_version: current_version.to_string(),
        latest_version,
        update_available,
        release_notes: release.body,
        download_url,
        asset_name: found_asset,
    })
}

/// Perform the actual update with backup and rollback support
async fn perform_update(update_info: &polyglot_common::updater::UpdateInfo) -> Result<()> {
    use polyglot_common::updater::*;

    let download_url = update_info.download_url.as_ref()
        .ok_or_else(|| anyhow::anyhow!("No download URL available for your platform"))?;

    let current_exe = get_current_exe()?;
    let current_version = env!("CARGO_PKG_VERSION");

    // Phase 1: Create backup
    print_status(&UpdateStatus {
        phase: UpdatePhase::Backing,
        message: "Creating backup...".to_string(),
        progress: None,
    });

    let backup_info = create_backup(&current_exe, current_version)?;
    println!("  Backup saved to: {:?}", backup_info.backup_path);

    // Phase 2: Download new version
    print_status(&UpdateStatus {
        phase: UpdatePhase::Downloading,
        message: format!("Downloading v{}...", update_info.latest_version),
        progress: Some(0),
    });

    let client = reqwest::Client::builder()
        .user_agent("polyglot-ai-updater")
        .build()?;

    let response = client.get(download_url).send().await?;

    if !response.status().is_success() {
        // Rollback not needed yet, just fail
        anyhow::bail!("Download failed: HTTP {}", response.status());
    }

    let _total_size = response.content_length().unwrap_or(0);
    let new_binary = response.bytes().await?;

    println!();
    print_status(&UpdateStatus {
        phase: UpdatePhase::Downloading,
        message: format!("Downloaded {}", format_bytes(new_binary.len() as u64)),
        progress: Some(100),
    });

    // Phase 3: Install new version
    print_status(&UpdateStatus {
        phase: UpdatePhase::Installing,
        message: "Installing update...".to_string(),
        progress: None,
    });

    let temp_path = current_exe.with_extension("new");

    if let Err(e) = std::fs::write(&temp_path, &new_binary) {
        error!("Failed to write new binary: {}", e);
        println!();
        print_status(&UpdateStatus {
            phase: UpdatePhase::Failed,
            message: format!("Failed to write new binary: {}", e),
            progress: None,
        });
        return Err(e.into());
    }

    // Phase 4: Verify the new binary
    print_status(&UpdateStatus {
        phase: UpdatePhase::Verifying,
        message: "Verifying new binary...".to_string(),
        progress: None,
    });

    if !verify_binary(&temp_path) {
        let _ = std::fs::remove_file(&temp_path);
        println!();
        print_status(&UpdateStatus {
            phase: UpdatePhase::Failed,
            message: "Downloaded binary is invalid!".to_string(),
            progress: None,
        });
        anyhow::bail!("Downloaded binary failed verification");
    }

    // Phase 5: Replace the current binary
    #[cfg(windows)]
    {
        // On Windows, rename the running executable
        let old_path = current_exe.with_extension("exe.old");
        std::fs::rename(&current_exe, &old_path)?;
        std::fs::rename(&temp_path, &current_exe)?;
        // Schedule cleanup of old file
        let _ = std::fs::remove_file(&old_path);
    }

    #[cfg(not(windows))]
    {
        std::fs::rename(&temp_path, &current_exe)?;
        // Set executable permissions
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&current_exe)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&current_exe, perms)?;
        }
    }

    // Phase 6: Cleanup old backups
    let _ = cleanup_old_backups(3);

    println!();
    print_status(&UpdateStatus {
        phase: UpdatePhase::Complete,
        message: format!("Successfully updated to v{}!", update_info.latest_version),
        progress: None,
    });

    println!();
    println!("\x1b[32m✓ Update complete! Please restart the application.\x1b[0m");

    Ok(())
}

/// Perform update from within TUI
async fn perform_update_tui(update_info: &polyglot_common::updater::UpdateInfo, app: &mut tui::App) -> Result<bool> {
    use polyglot_common::updater::*;

    let download_url = match update_info.download_url.as_ref() {
        Some(url) => url,
        None => {
            app.add_output(tui::OutputType::Error, "No download URL available for your platform".to_string());
            return Ok(false);
        }
    };

    let current_exe = get_current_exe()?;
    let current_version = env!("CARGO_PKG_VERSION");

    app.add_output(tui::OutputType::System, "Creating backup...".to_string());
    let _backup_info = create_backup(&current_exe, current_version)?;

    app.add_output(tui::OutputType::System, format!("Downloading v{}...", update_info.latest_version));

    let client = reqwest::Client::builder()
        .user_agent("polyglot-ai-updater")
        .build()?;

    let response = client.get(download_url).send().await?;

    if !response.status().is_success() {
        anyhow::bail!("Download failed: HTTP {}", response.status());
    }

    let new_binary = response.bytes().await?;
    app.add_output(tui::OutputType::System, format!("Downloaded {} bytes", new_binary.len()));

    app.add_output(tui::OutputType::System, "Installing update...".to_string());

    let temp_path = current_exe.with_extension("new");

    std::fs::write(&temp_path, &new_binary)?;

    app.add_output(tui::OutputType::System, "Verifying binary...".to_string());

    if !verify_binary(&temp_path) {
        let _ = std::fs::remove_file(&temp_path);
        anyhow::bail!("Downloaded binary is invalid");
    }

    app.add_output(tui::OutputType::System, "Replacing binary...".to_string());

    #[cfg(windows)]
    {
        let old_path = current_exe.with_extension("old");
        let _ = std::fs::remove_file(&old_path);
        std::fs::rename(&current_exe, &old_path)?;
        std::fs::rename(&temp_path, &current_exe)?;
    }

    #[cfg(not(windows))]
    {
        std::fs::rename(&temp_path, &current_exe)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&current_exe)?.permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&current_exe, perms)?;
        }
    }

    let _ = cleanup_old_backups(3);

    app.add_output(tui::OutputType::System, format!("Successfully updated to v{}!", update_info.latest_version));

    Ok(true)
}

/// Check for updates on startup (non-blocking notification)
async fn check_updates_on_startup() -> Option<String> {
    match check_for_updates_github("polyglot-local").await {
        Ok(info) if info.update_available => {
            Some(format!(
                "🆕 Update available: v{} → v{} (run 'polyglot-local update' to install)",
                info.current_version,
                info.latest_version
            ))
        }
        _ => None,
    }
}
