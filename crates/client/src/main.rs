//! Polyglot-AI Client
//!
//! A client for connecting to Polyglot-AI server and interacting with
//! multiple AI coding assistants through a unified interface.

mod config;
mod connection;
mod sync;
mod tui;

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing::info;
use tracing_subscriber::EnvFilter;

use polyglot_common::{Tool, SyncMode, ServerMessage};
use config::ClientConfig;
use connection::ClientConnection;
use tui::{App, AppAction, OutputType};

#[derive(Parser)]
#[command(name = "polyglot")]
#[command(about = "Polyglot-AI Client - Connect to AI CLI Gateway")]
#[command(long_about = "
A client for connecting to Polyglot-AI server and interacting with
multiple AI coding assistants through a unified interface.

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
    server: Option<String>,

    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    Connect,

    Prompt {
        message: String,

        #[arg(short, long)]
        tool: Option<String>,
    },

    Sync {
        #[arg(default_value = ".")]
        path: String,

        #[arg(short, long, default_value = "ondemand")]
        mode: String,
    },

    Usage,

    Tools,

    Switch {
        tool: String,
    },

    GenerateConfig {
        #[arg(short, long, default_value = "client.toml")]
        output: PathBuf,
    },

    GenerateCerts {
        #[arg(short, long, default_value = "./certs")]
        output: PathBuf,

        #[arg(long, default_value = "polyglot-client")]
        cn: String,

        #[arg(long)]
        ca_cert: PathBuf,

        #[arg(long)]
        ca_key: PathBuf,
    },

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
        EnvFilter::new("info")
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .init();

    let config_path = cli.config.unwrap_or_else(ClientConfig::default_path);
    let mut config = if config_path.exists() {
        ClientConfig::load(&config_path)?
    } else {
        ClientConfig::default()
    };

    if let Some(server) = cli.server {
        config.connection.server_address = server;
    }

    match cli.command {
        Some(Commands::Connect) | None => run_interactive(&config).await,
        Some(Commands::Prompt { message, tool }) => run_prompt(&config, &message, tool).await,
        Some(Commands::Sync { path, mode }) => run_sync(&config, &path, &mode).await,
        Some(Commands::Usage) => run_usage(&config).await,
        Some(Commands::Tools) => run_tools(&config).await,
        Some(Commands::Switch { tool }) => run_switch(&config, &tool).await,
        Some(Commands::GenerateConfig { output }) => generate_config(&output),
        Some(Commands::GenerateCerts { output, cn, ca_cert, ca_key }) => {
            generate_certs(&output, &cn, &ca_cert, &ca_key)
        }
        Some(Commands::Update { check_only, force }) => run_update("polyglot", check_only, force).await,
    }
}

async fn run_interactive(config: &ClientConfig) -> Result<()> {
    info!("Starting Polyglot-AI Client v{}", env!("CARGO_PKG_VERSION"));

    let mut app = App::new();
    app.add_output(OutputType::System, "Welcome to Polyglot-AI!".to_string());
    app.add_output(OutputType::System, format!("Connecting to {}...", config.connection.server_address));

    let mut conn = ClientConnection::new(&config.connection).await?;

    match conn.connect(&config.connection).await {
        Ok(_) => {
            app.set_connected(true, Some(Tool::Claude));
            app.add_output(OutputType::System, "Connected successfully!".to_string());

            if let Ok(ServerMessage::ToolList { tools, current }) = conn.list_tools().await {
                app.tools = tools.iter().map(|t| (t.tool, t.available)).collect();
                app.current_tool = current;
            }

            // Check for updates
            if let Ok(ServerMessage::VersionInfo {
                server_version,
                min_client_version,
                update_available,
                update_url,
                update_message,
                ..
            }) = conn.check_version().await {
                let client_version = env!("CARGO_PKG_VERSION");

                // Check if client version is below minimum required
                if let Some(min_version) = min_client_version {
                    if version_compare(client_version, &min_version) == std::cmp::Ordering::Less {
                        app.add_output(OutputType::Error,
                            format!("⚠️  Your client (v{}) is outdated. Minimum required: v{}", client_version, min_version));
                        if let Some(url) = &update_url {
                            app.add_output(OutputType::System, format!("   Download update: {}", url));
                        }
                    }
                }

                // Show update notification if available
                if update_available {
                    app.add_output(OutputType::System,
                        format!("🆕 Server update available (v{}) - Your client: v{}", server_version, client_version));
                    if let Some(msg) = update_message {
                        app.add_output(OutputType::System, format!("   {}", msg));
                    }
                    if let Some(url) = update_url {
                        app.add_output(OutputType::System, format!("   Download: {}", url));
                    }
                }
            }
        }
        Err(e) => {
            app.add_output(OutputType::Error, format!("Connection failed: {}", e));
        }
    }

    if config.ui.tui_enabled {
        run_tui_interactive(&mut conn, &mut app, config).await?;
    } else {
        run_cli_loop(&mut conn, &mut app).await?;
    }

    conn.disconnect().await?;

    Ok(())
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
                    Span::styled("CLIENT MODE", Style::default().fg(Color::Magenta)),
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

                let bar_text = Line::from(Span::styled(bar, Style::default().fg(Color::Magenta)));
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
            Span::styled("CLIENT MODE", Style::default().fg(Color::Magenta)),
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

async fn run_tui_interactive(
    conn: &mut ClientConnection,
    app: &mut App,
    _config: &ClientConfig,
) -> Result<()> {
    use std::io;
    use std::time::Duration;
    use crossterm::{
        event::{self, DisableMouseCapture, EnableMouseCapture, Event},
        execute,
        terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    };
    use ratatui::{backend::CrosstermBackend, Terminal};
    use tokio::sync::mpsc;

    enable_raw_mode()?;

    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Console::{
            GetConsoleMode, SetConsoleMode, GetStdHandle,
            STD_INPUT_HANDLE, ENABLE_ECHO_INPUT, ENABLE_LINE_INPUT,
        };
        unsafe {
            let handle = GetStdHandle(STD_INPUT_HANDLE);
            let mut mode: u32 = 0;
            if GetConsoleMode(handle, &mut mode) != 0 {
                mode &= !(ENABLE_ECHO_INPUT | ENABLE_LINE_INPUT);
                SetConsoleMode(handle, mode);
            }
        }
    }

    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    show_splash_screen(&mut terminal).await?;

    // Check for updates in background on startup
    let update_check = tokio::spawn(async move {
        check_updates_on_startup().await
    });

    let (response_tx, mut response_rx) = mpsc::channel::<ServerMessage>(100);

    // Check if update notification is available
    if let Ok(Some(update_msg)) = update_check.await {
        app.add_output(OutputType::System, update_msg);
    }

    let result = async {
        loop {
            terminal.draw(|f| tui::draw_ui(f, app))?;

            tokio::select! {
                _ = tokio::time::sleep(Duration::from_millis(50)) => {
                    if event::poll(Duration::from_millis(0))? {
                        if let Event::Key(key) = event::read()? {
                            if let Some(action) = app.handle_key(key.code, key.modifiers) {
                                match action {
                                    AppAction::Quit => break,
                                    AppAction::SendPrompt(message) => {
                                        let tx = response_tx.clone();
                                        if let Err(e) = conn.prompt_streaming(&message, app.current_tool, tx).await {
                                            app.add_output(OutputType::Error, format!("Error: {}", e));
                                        }
                                    }
                                    AppAction::RequestUsage => {
                                        match conn.usage().await {
                                            Ok(ServerMessage::UsageStats { stats, .. }) => {
                                                app.usage = stats;
                                            }
                                            Ok(ServerMessage::Error { message, .. }) => {
                                                app.add_output(OutputType::Error, message);
                                            }
                                            Err(e) => {
                                                app.add_output(OutputType::Error, format!("Error: {}", e));
                                            }
                                            _ => {}
                                        }
                                    }
                                    AppAction::RequestTools => {
                                        match conn.list_tools().await {
                                            Ok(ServerMessage::ToolList { tools, current }) => {
                                                app.tools = tools.iter().map(|t| (t.tool, t.available)).collect();
                                                app.current_tool = current;
                                            }
                                            Err(e) => {
                                                app.add_output(OutputType::Error, format!("Error: {}", e));
                                            }
                                            _ => {}
                                        }
                                    }
                                    AppAction::SwitchTool(tool) => {
                                        match conn.select_tool(tool).await {
                                            Ok(ServerMessage::ToolSwitched { to, .. }) => {
                                                app.current_tool = Some(to);
                                                app.add_output(OutputType::System, format!("Switched to {}", to.display_name()));
                                            }
                                            Ok(ServerMessage::Error { message, .. }) => {
                                                app.add_output(OutputType::Error, message);
                                            }
                                            Err(e) => {
                                                app.add_output(OutputType::Error, format!("Error: {}", e));
                                            }
                                            _ => {}
                                        }
                                    }
                                    AppAction::Sync(path) => {
                                        match conn.sync(&path, SyncMode::OnDemand).await {
                                            Ok(ServerMessage::SyncResponse { files, .. }) => {
                                                app.add_output(OutputType::System, format!("Synced {} files", files.len()));
                                            }
                                            Ok(ServerMessage::Error { message, .. }) => {
                                                app.add_output(OutputType::Error, message);
                                            }
                                            Err(e) => {
                                                app.add_output(OutputType::Error, format!("Error: {}", e));
                                            }
                                            _ => {}
                                        }
                                    }
                                    AppAction::CheckUpdate => {
                                        app.add_output(OutputType::System, "Checking for updates...".to_string());
                                        match check_for_updates_github("polyglot").await {
                                            Ok(info) => {
                                                if info.update_available {
                                                    app.add_output(OutputType::System, format!(
                                                        "⬆ Update available: v{} → v{}", info.current_version, info.latest_version
                                                    ));
                                                    app.add_output(OutputType::System, 
                                                        "Run 'polyglot update' from terminal to install".to_string()
                                                    );
                                                } else {
                                                    app.add_output(OutputType::System, "✓ You are running the latest version!".to_string());
                                                }
                                            }
                                            Err(e) => {
                                                app.add_output(OutputType::Error, format!("Update check failed: {}", e));
                                            }
                                        }
                                    }
                                    AppAction::None => {}
                                }
                            }
                        }
                    }
                }

                Some(response) = response_rx.recv() => {
                    match response {
                        ServerMessage::ToolResponse { tool: _, content, done, tokens } => {
                            if !content.is_empty() {
                                app.add_output(OutputType::Assistant, content);
                            }
                            if done {
                                if let Some(t) = tokens {
                                    app.add_output(OutputType::System, format!("(tokens: {})", t));
                                }
                            }
                        }
                        ServerMessage::ToolOutput { content, .. } => {
                            app.add_output(OutputType::System, format!("[stderr] {}", content));
                        }
                        ServerMessage::ToolSwitchNotice { from, to, reason, countdown } => {
                            app.add_output(
                                OutputType::System,
                                format!(
                                    "{} hit {}. Switching to {} in {}s...",
                                    from.display_name(),
                                    reason,
                                    to.display_name(),
                                    countdown
                                ),
                            );
                        }
                        ServerMessage::Error { code, message } => {
                            app.add_output(OutputType::Error, format!("{}: {}", code, message));
                        }
                        _ => {}
                    }
                }
            }

            if app.should_quit {
                break;
            }
        }

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

async fn run_cli_loop(conn: &mut ClientConnection, app: &mut App) -> Result<()> {
    use std::io::{self, BufRead, Write};

    let stdin = io::stdin();
    let mut stdout = io::stdout();

    loop {
        print!("> ");
        stdout.flush()?;

        let mut input = String::new();
        if stdin.lock().read_line(&mut input)? == 0 {
            break;
        }

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        if input.starts_with('/') {
            match input {
                "/quit" | "/exit" => break,
                "/usage" => {
                    if let Ok(resp) = conn.usage().await {
                        println!("{:?}", resp);
                    }
                }
                "/tools" => {
                    if let Ok(resp) = conn.list_tools().await {
                        println!("{:?}", resp);
                    }
                }
                "/help" => {
                    println!("Commands:");
                    println!("  /usage  - Show usage statistics");
                    println!("  /tools  - List available tools");
                    println!("  /quit   - Exit");
                }
                _ => {
                    println!("Unknown command: {}", input);
                }
            }
            continue;
        }

        match conn.prompt(input, app.current_tool).await {
            Ok(ServerMessage::ToolResponse { tool, content, done: _, tokens }) => {
                println!("[{}] {}", tool.display_name(), content);
                if let Some(t) = tokens {
                    println!("(tokens: {})", t);
                }
            }
            Ok(ServerMessage::Error { code, message }) => {
                println!("Error: {} - {}", code, message);
            }
            Ok(other) => {
                println!("Response: {:?}", other);
            }
            Err(e) => {
                println!("Error: {}", e);
            }
        }
    }

    Ok(())
}

async fn run_prompt(config: &ClientConfig, message: &str, tool: Option<String>) -> Result<()> {
    let tool = tool.map(|t| t.parse::<Tool>()).transpose()
        .map_err(|e| anyhow::anyhow!("Invalid tool: {}", e))?;

    let mut conn = ClientConnection::new(&config.connection).await?;
    conn.connect(&config.connection).await?;

    match conn.prompt(message, tool).await {
        Ok(ServerMessage::ToolResponse { tool: _, content, done: _, tokens }) => {
            println!("{}", content);
            if let Some(t) = tokens {
                eprintln!("(tokens used: {})", t);
            }
        }
        Ok(ServerMessage::Error { code, message }) => {
            eprintln!("Error: {} - {}", code, message);
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("Unexpected response");
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }

    conn.disconnect().await?;
    Ok(())
}

async fn run_sync(config: &ClientConfig, path: &str, mode: &str) -> Result<()> {
    let sync_mode = match mode.to_lowercase().as_str() {
        "realtime" => SyncMode::Realtime,
        "ondemand" | "on-demand" => SyncMode::OnDemand,
        _ => {
            eprintln!("Invalid sync mode. Use 'realtime' or 'ondemand'");
            std::process::exit(1);
        }
    };

    let mut conn = ClientConnection::new(&config.connection).await?;
    conn.connect(&config.connection).await?;

    match conn.sync(path, sync_mode).await {
        Ok(ServerMessage::SyncResponse { files, mode }) => {
            println!("Sync mode: {:?}", mode);
            println!("Files: {}", files.len());
            for file in files {
                println!("  {} ({} bytes)", file.path, file.size);
            }
        }
        Ok(ServerMessage::Error { code, message }) => {
            eprintln!("Error: {} - {}", code, message);
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("Unexpected response");
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }

    conn.disconnect().await?;
    Ok(())
}

async fn run_usage(config: &ClientConfig) -> Result<()> {
    let mut conn = ClientConnection::new(&config.connection).await?;
    conn.connect(&config.connection).await?;

    match conn.usage().await {
        Ok(ServerMessage::UsageStats { stats, session_start }) => {
            println!("Usage Statistics");
            println!("Session started: {}", session_start);
            println!();

            for stat in stats {
                println!("{}", stat.tool.display_name());
                println!("  Requests:      {}", stat.requests);
                println!("  Tokens Used:   {}", stat.tokens_used);
                println!("  Errors:        {}", stat.errors);
                println!("  Rate Limits:   {}", stat.rate_limit_hits);
                if let Some(last) = stat.last_used {
                    println!("  Last Used:     {}", last);
                }
                println!();
            }
        }
        Ok(ServerMessage::Error { code, message }) => {
            eprintln!("Error: {} - {}", code, message);
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("Unexpected response");
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }

    conn.disconnect().await?;
    Ok(())
}

async fn run_tools(config: &ClientConfig) -> Result<()> {
    let mut conn = ClientConnection::new(&config.connection).await?;
    conn.connect(&config.connection).await?;

    match conn.list_tools().await {
        Ok(ServerMessage::ToolList { tools, current }) => {
            println!("Available Tools:");
            for tool_info in tools {
                let status = if tool_info.available { "[OK]" } else { "[--]" };
                let current_marker = if Some(tool_info.tool) == current { " (current)" } else { "" };
                println!("  {} {}{}", status, tool_info.tool.display_name(), current_marker);
            }
        }
        Ok(ServerMessage::Error { code, message }) => {
            eprintln!("Error: {} - {}", code, message);
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("Unexpected response");
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }

    conn.disconnect().await?;
    Ok(())
}

async fn run_switch(config: &ClientConfig, tool_name: &str) -> Result<()> {
    let tool: Tool = tool_name.parse()
        .map_err(|e| anyhow::anyhow!("Invalid tool: {}", e))?;

    let mut conn = ClientConnection::new(&config.connection).await?;
    conn.connect(&config.connection).await?;

    match conn.select_tool(tool).await {
        Ok(ServerMessage::ToolSwitched { from, to, reason }) => {
            println!("Switched from {} to {} (reason: {})", from, to, reason);
        }
        Ok(ServerMessage::Error { code, message }) => {
            eprintln!("Error: {} - {}", code, message);
            std::process::exit(1);
        }
        Ok(_) => {
            eprintln!("Unexpected response");
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(1);
        }
    }

    conn.disconnect().await?;
    Ok(())
}

fn generate_config(output: &PathBuf) -> Result<()> {
    let config_content = config::generate_example_config();
    std::fs::write(output, config_content)?;
    println!("Generated example config: {:?}", output);
    Ok(())
}

fn generate_certs(output: &PathBuf, cn: &str, ca_cert_path: &PathBuf, ca_key_path: &PathBuf) -> Result<()> {
    use rcgen::{CertificateParams, KeyPair, DnType, Issuer};

    std::fs::create_dir_all(output)?;

    let ca_cert_pem = std::fs::read_to_string(ca_cert_path)?;
    let ca_key_pem = std::fs::read_to_string(ca_key_path)?;

    let ca_key_pair = KeyPair::from_pem(&ca_key_pem)?;
    let issuer = Issuer::from_ca_cert_pem(&ca_cert_pem, ca_key_pair)?;

    let mut client_params = CertificateParams::default();
    client_params.distinguished_name.push(DnType::CommonName, cn.to_string());

    let client_key_pair = KeyPair::generate()?;
    let client_cert = client_params.signed_by(&client_key_pair, &issuer)?;

    std::fs::write(output.join("client.crt"), client_cert.pem())?;
    std::fs::write(output.join("client.key"), client_key_pair.serialize_pem())?;

    println!("Generated client certificates in {:?}", output);
    println!("  client.crt - Client certificate");
    println!("  client.key - Client private key");

    Ok(())
}

/// Simple semantic version comparison
fn version_compare(a: &str, b: &str) -> std::cmp::Ordering {
    polyglot_common::version_compare(a, b)
}

/// Check for updates and optionally install them
async fn run_update(binary_name: &str, check_only: bool, force: bool) -> Result<()> {
    use polyglot_common::updater::*;
    use std::io::Write;

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

    let update_info = check_for_updates_github(binary_name).await?;

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
        println!("\nRun 'polyglot update' to install the update.");
        return Ok(());
    }

    println!();
    print!("Do you want to install the update? [y/N]: ");
    std::io::stdout().flush()?;

    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;

    if !input.trim().eq_ignore_ascii_case("y") {
        println!("Update cancelled.");
        return Ok(());
    }

    perform_update(&update_info).await
}

/// Check for updates from GitHub releases
async fn check_for_updates_github(binary_name: &str) -> Result<polyglot_common::updater::UpdateInfo> {
    use polyglot_common::updater::*;

    let client = reqwest::Client::builder()
        .user_agent("polyglot-ai-updater")
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let url = "https://api.github.com/repos/tugcantopaloglu/selfhosted-ai-code-platform/releases/latest";

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
    use tracing::error;

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
        anyhow::bail!("Download failed: HTTP {}", response.status());
    }

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
        let old_path = current_exe.with_extension("exe.old");
        std::fs::rename(&current_exe, &old_path)?;
        std::fs::rename(&temp_path, &current_exe)?;
        let _ = std::fs::remove_file(&old_path);
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

/// Check for updates on startup (returns notification string if update available)
async fn check_updates_on_startup() -> Option<String> {
    match check_for_updates_github("polyglot").await {
        Ok(info) => {
            if info.update_available {
                Some(format!("⬆ Update available: v{} → v{}. Use /update to check.", 
                    info.current_version, info.latest_version))
            } else {
                None
            }
        }
        _ => None,
    }
}
