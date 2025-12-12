//! Polyglot-AI Server
//!
//! A high-performance server that aggregates multiple AI coding CLI tools
//! and provides unified access through a secure QUIC connection.

mod config;
mod auth;
mod tools;
mod sync;
mod usage;
mod protocol;

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use quinn::{Endpoint, ServerConfig as QuinnServerConfig};
use tokio::sync::mpsc;
#[cfg(not(unix))]
use tokio::signal;
use tracing::{info, error, debug};
use tracing_subscriber::EnvFilter;
use uuid::Uuid;

use polyglot_common::{
    ClientMessage, ServerMessage, Tool,
    ErrorCode, ToolInfo, SwitchReason,
    PROTOCOL_VERSION,
};

use config::ServerConfig;
use auth::{SessionManager, UserManager};
use tools::{ToolManager, ToolRequest, ToolOutput};
use sync::SyncManager;
use usage::UsageTracker;
use protocol::{StreamReader, StreamWriter};

#[derive(Parser)]
#[command(name = "polyglot-server")]
#[command(about = "Polyglot-AI Server - Unified AI CLI Gateway")]
#[command(long_about = "
A high-performance server that aggregates multiple AI coding CLI tools
and provides unified access through a secure QUIC connection.

Made by Tugcan Topaloglu
")]
#[command(version)]
#[command(author = "Tugcan Topaloglu")]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    #[arg(short, long, global = true)]
    config: Option<PathBuf>,

    #[arg(short, long, global = true)]
    verbose: bool,
}

#[derive(Subcommand)]
enum Commands {
    Start,

    AddUser {
        username: String,

        #[arg(long)]
        admin: bool,
    },

    RemoveUser {
        username: String,
    },

    ListUsers,

    Invite {
        #[arg(short, long, default_value = "24")]
        expiry: u32,

        #[arg(short, long, default_value = "1")]
        uses: u32,

        #[arg(long)]
        admin: bool,
    },

    Usage {
        #[arg(long)]
        detailed: bool,
    },

    Info,

    GenerateConfig {
        #[arg(short, long, default_value = "server.toml")]
        output: PathBuf,
    },

    GenerateCerts {
        #[arg(short, long, default_value = "./certs")]
        output: PathBuf,

        #[arg(long, default_value = "polyglot-ai")]
        cn: String,
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

struct ServerState {
    config: ServerConfig,
    session_manager: SessionManager,
    user_manager: UserManager,
    #[allow(dead_code)]
    invite_manager: auth::InviteManager,
    tool_manager: ToolManager,
    sync_manager: SyncManager,
    #[allow(dead_code)]
    usage_tracker: UsageTracker,
    shutdown: AtomicBool,
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

    let config_path = cli.config.unwrap_or_else(ServerConfig::default_path);
    let config = if config_path.exists() {
        ServerConfig::load(&config_path)?
    } else {
        info!("No config file found, using defaults");
        ServerConfig::default()
    };

    match cli.command {
        Commands::Start => start_server(config).await,
        Commands::AddUser { username, admin } => add_user(&config, &username, admin),
        Commands::RemoveUser { username } => remove_user(&config, &username),
        Commands::ListUsers => list_users(&config),
        Commands::Invite { expiry, uses, admin } => generate_invite(&config, expiry, uses, admin),
        Commands::Usage { detailed } => show_usage(&config, detailed),
        Commands::Info => show_server_info(&config),
        Commands::GenerateConfig { output } => generate_config(&output),
        Commands::GenerateCerts { output, cn } => generate_certs(&output, &cn),
        Commands::Update { check_only, force } => run_update(check_only, force).await,
    }
}

fn print_startup_banner() {
    let version = env!("CARGO_PKG_VERSION");

    println!();
    println!("\x1b[36m  ____       _             _       _        _    ___ \x1b[0m");
    println!("\x1b[36m |  _ \\ ___ | |_   _  __ _| | ___ | |_     / \\  |_ _|\x1b[0m");
    println!("\x1b[36m | |_) / _ \\| | | | |/ _` | |/ _ \\| __|   / _ \\  | | \x1b[0m");
    println!("\x1b[36m |  __/ (_) | | |_| | (_| | | (_) | |_   / ___ \\ | | \x1b[0m");
    println!("\x1b[36m |_|   \\___/|_|\\__, |\\__, |_|\\___/ \\__| /_/   \\_\\___|\x1b[0m");
    println!("\x1b[36m               |___/ |___/                           \x1b[0m");
    println!();
    println!("                  \x1b[32mv{}\x1b[0m  \x1b[33mSERVER MODE\x1b[0m", version);
    println!();
    println!("          \x1b[37mMade by Tugcan Topaloglu @tugcantopaloglu\x1b[0m");
    println!();
}

async fn start_server(config: ServerConfig) -> Result<()> {
    print_startup_banner();

    info!("Starting Polyglot-AI Server v{}", env!("CARGO_PKG_VERSION"));

    let jwt_secret = config.auth.jwt_secret.clone()
        .unwrap_or_else(|| polyglot_common::crypto::random_token(32));

    let session_manager = SessionManager::new(jwt_secret, config.auth.session_expiry_hours);
    let user_manager = UserManager::new(&config.storage.db_path)?;
    let invite_manager = auth::InviteManager::new();
    let tool_manager = ToolManager::new(&config.tools);
    let sync_manager = SyncManager::new(config.storage.sync_dir.clone());
    let usage_tracker = UsageTracker::new(&config.storage.db_path)?;

    let available = tool_manager.available_tools().await;
    info!("Available tools: {:?}", available);

    let user_count = user_manager.user_count().unwrap_or(0);
    if user_count == 0 {
        let initial_invite = invite_manager.generate_invite(168, 1, true, Some("system".to_string()));
        println!();
        println!("╔══════════════════════════════════════════════════════════════════╗");
        println!("║                    FIRST-TIME SETUP                              ║");
        println!("╠══════════════════════════════════════════════════════════════════╣");
        println!("║  No users found. Use this invite code to create admin account:  ║");
        println!("║                                                                  ║");
        println!("║    INVITE CODE: {:^8}                                       ║", initial_invite.code);
        println!("║    Expires: {} (7 days)                          ║", initial_invite.expires_at.format("%Y-%m-%d %H:%M"));
        println!("║                                                                  ║");
        println!("╚══════════════════════════════════════════════════════════════════╝");
        println!();
    }

    let state = Arc::new(ServerState {
        config: config.clone(),
        session_manager,
        user_manager,
        invite_manager,
        tool_manager,
        sync_manager,
        usage_tracker,
        shutdown: AtomicBool::new(false),
    });

    let server_config = configure_quic_server(&config)?;

    let addr: SocketAddr = config.server.bind_address.parse()
        .context("Invalid bind address")?;

    let endpoint = Endpoint::server(server_config, addr)?;

    print_connection_info(&config, &addr);

    let shutdown_state = state.clone();
    tokio::spawn(async move {
        if let Err(e) = shutdown_signal().await {
            error!("Error waiting for shutdown signal: {}", e);
        }
        info!("Shutdown signal received, stopping server...");
        shutdown_state.shutdown.store(true, Ordering::SeqCst);
    });

    loop {
        tokio::select! {
            incoming = endpoint.accept() => {
                if let Some(incoming) = incoming {
                    let state = state.clone();
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(incoming, state).await {
                            error!("Connection error: {}", e);
                        }
                    });
                } else {
                    break;
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if state.shutdown.load(Ordering::SeqCst) {
                    info!("Shutting down server...");
                    break;
                }
            }
        }
    }

    endpoint.close(0u32.into(), b"server shutdown");
    state.tool_manager.cancel_all().await;
    info!("Server stopped");

    Ok(())
}

async fn shutdown_signal() -> Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm = signal(SignalKind::terminate())?;
        let mut sigint = signal(SignalKind::interrupt())?;
        tokio::select! {
            _ = sigterm.recv() => {},
            _ = sigint.recv() => {},
        }
    }
    #[cfg(not(unix))]
    {
        signal::ctrl_c().await?;
    }
    Ok(())
}

fn configure_quic_server(config: &ServerConfig) -> Result<QuinnServerConfig> {
    let cert_path = &config.auth.cert_path;
    let key_path = &config.auth.key_path;

    let cert_pem = std::fs::read(cert_path)
        .with_context(|| format!("Failed to read certificate: {:?}", cert_path))?;
    let key_pem = std::fs::read(key_path)
        .with_context(|| format!("Failed to read private key: {:?}", key_path))?;

    let certs = rustls_pemfile::certs(&mut cert_pem.as_slice())
        .collect::<Result<Vec<_>, _>>()
        .context("Failed to parse certificates")?;

    let key = rustls_pemfile::private_key(&mut key_pem.as_slice())
        .context("Failed to parse private key")?
        .context("No private key found")?;

    let mut tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .context("Failed to create TLS config")?;

    tls_config.alpn_protocols = vec![b"polyglot-ai".to_vec()];

    let mut server_config = QuinnServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)?
    ));

    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(
        Duration::from_secs(config.server.idle_timeout).try_into()?
    ));
    server_config.transport_config(Arc::new(transport));

    Ok(server_config)
}

async fn handle_connection(
    incoming: quinn::Incoming,
    state: Arc<ServerState>,
) -> Result<()> {
    let connection = incoming.await?;
    let remote_addr = connection.remote_address();
    info!("New connection from {}", remote_addr);

    let (mut send, mut recv) = connection.accept_bi().await?;

    let mut reader = StreamReader::new();
    let mut writer = StreamWriter::new();
    let mut session_id: Option<Uuid> = None;
    let mut current_tool: Option<Tool> = None;

    let (response_tx, mut response_rx) = mpsc::channel::<ServerMessage>(100);

    let mut buf = [0u8; 8192];

    loop {
        tokio::select! {
            result = recv.read(&mut buf) => {
                match result {
                    Ok(Some(n)) => {
                        reader.push(&buf[..n]);

                        while let Some(msg) = reader.try_read()? {
                            handle_message(
                                msg,
                                &state,
                                &mut session_id,
                                &mut current_tool,
                                response_tx.clone(),
                            ).await?;
                        }
                    }
                    Ok(None) => {
                        debug!("Stream closed by client");
                        break;
                    }
                    Err(e) => {
                        error!("Read error: {}", e);
                        break;
                    }
                }
            }
            Some(response) = response_rx.recv() => {
                writer.queue(&response)?;
                if writer.has_pending() {
                    let data = writer.take();
                    send.write_all(&data).await?;
                }
            }
        }
    }

    if let Some(sid) = session_id {
        state.session_manager.remove_session(sid);
    }

    info!("Connection closed: {}", remote_addr);
    Ok(())
}

async fn handle_message(
    msg: ClientMessage,
    state: &Arc<ServerState>,
    session_id: &mut Option<Uuid>,
    current_tool: &mut Option<Tool>,
    response_tx: mpsc::Sender<ServerMessage>,
) -> Result<()> {
    match msg {
        ClientMessage::Handshake { version, client_id } => {
            if version != PROTOCOL_VERSION {
                response_tx.send(ServerMessage::Error {
                    code: ErrorCode::ProtocolMismatch,
                    message: format!(
                        "Protocol version mismatch. Server: {}, Client: {}",
                        PROTOCOL_VERSION, version
                    ),
                }).await.ok();
                return Ok(());
            }

            debug!("Handshake from client: {}", client_id);
            response_tx.send(ServerMessage::HandshakeAck {
                version: PROTOCOL_VERSION,
                server_id: format!("polyglot-server-{}", env!("CARGO_PKG_VERSION")),
            }).await.ok();
        }

        ClientMessage::Auth { cert_fingerprint } => {
            match state.user_manager.get_user_by_fingerprint(&cert_fingerprint) {
                Ok(user) => {
                    let (session, _token) = state.session_manager.create_session(user.id)?;
                    *session_id = Some(session.id);
                    *current_tool = Some(state.config.tools.default_tool);

                    state.user_manager.update_last_login(user.id)?;

                    response_tx.send(ServerMessage::AuthResult {
                        success: true,
                        session_id: Some(session.id.to_string()),
                        user: Some(user),
                        error: None,
                    }).await.ok();
                }
                Err(_) => {
                    if state.user_manager.is_single_user_mode()? {
                        let user = state.user_manager.create_user("default", true)?;
                        state.user_manager.set_user_fingerprint(user.id, &cert_fingerprint)?;

                        let (session, _token) = state.session_manager.create_session(user.id)?;
                        *session_id = Some(session.id);
                        *current_tool = Some(state.config.tools.default_tool);

                        response_tx.send(ServerMessage::AuthResult {
                            success: true,
                            session_id: Some(session.id.to_string()),
                            user: Some(user),
                            error: None,
                        }).await.ok();
                    } else {
                        response_tx.send(ServerMessage::AuthResult {
                            success: false,
                            session_id: None,
                            user: None,
                            error: Some("Unknown certificate".to_string()),
                        }).await.ok();
                    }
                }
            }
        }

        ClientMessage::Prompt { tool, message, working_dir } => {
            let tool = tool.or(*current_tool).unwrap_or(state.config.tools.default_tool);

            let request = ToolRequest {
                message,
                working_dir,
                context_files: Vec::new(),
            };

            let (tool_tx, mut tool_rx) = mpsc::channel::<ToolOutput>(100);

            let tool_manager = state.tool_manager.clone();
            let response_tx_clone = response_tx.clone();
            let switch_delay = state.config.tools.switch_delay;

            tokio::spawn(async move {
                let execute_handle = tokio::spawn({
                    let tool_manager = tool_manager.clone();
                    async move {
                        tool_manager.execute(Some(tool), request, tool_tx).await
                    }
                });

                while let Some(output) = tool_rx.recv().await {
                    match output {
                        ToolOutput::Stdout(line) => {
                            response_tx_clone.send(ServerMessage::ToolResponse {
                                tool,
                                content: line,
                                done: false,
                                tokens: None,
                            }).await.ok();
                        }
                        ToolOutput::Stderr(line) => {
                            response_tx_clone.send(ServerMessage::ToolOutput {
                                tool,
                                output_type: polyglot_common::OutputType::Stderr,
                                content: line,
                            }).await.ok();
                        }
                        ToolOutput::Done { tokens } => {
                            response_tx_clone.send(ServerMessage::ToolResponse {
                                tool,
                                content: String::new(),
                                done: true,
                                tokens,
                            }).await.ok();
                        }
                        ToolOutput::Error(e) => {
                            response_tx_clone.send(ServerMessage::Error {
                                code: ErrorCode::ToolError,
                                message: e,
                            }).await.ok();
                        }
                        ToolOutput::RateLimited => {
                            if let Some(next_tool) = tool_manager.get_next_tool(tool).await {
                                response_tx_clone.send(ServerMessage::ToolSwitchNotice {
                                    from: tool,
                                    to: next_tool,
                                    reason: SwitchReason::RateLimit,
                                    countdown: switch_delay,
                                }).await.ok();
                            } else {
                                response_tx_clone.send(ServerMessage::Error {
                                    code: ErrorCode::RateLimited,
                                    message: "Rate limited and no alternative tools available".to_string(),
                                }).await.ok();
                            }
                        }
                    }
                }

                if let Err(e) = execute_handle.await {
                    error!("Tool execution task failed: {}", e);
                }
            });
        }

        ClientMessage::Usage => {
            let stats = state.tool_manager.get_usage();
            let session = session_id
                .and_then(|sid| state.session_manager.get_session(sid).ok());

            response_tx.send(ServerMessage::UsageStats {
                stats,
                session_start: session.map(|s| s.created_at).unwrap_or_else(chrono::Utc::now),
            }).await.ok();
        }

        ClientMessage::SelectTool { tool } => {
            match state.tool_manager.set_current_tool(tool) {
                Ok(_) => {
                    let from_tool = current_tool.unwrap_or(tool);
                    *current_tool = Some(tool);
                    if let Some(sid) = session_id {
                        let _ = state.session_manager.set_current_tool(*sid, tool);
                    }
                    response_tx.send(ServerMessage::ToolSwitched {
                        from: from_tool,
                        to: tool,
                        reason: SwitchReason::UserRequest,
                    }).await.ok();
                }
                Err(_) => {
                    response_tx.send(ServerMessage::Error {
                        code: ErrorCode::ToolNotAvailable,
                        message: format!("Tool {} is not available", tool),
                    }).await.ok();
                }
            }
        }

        ClientMessage::ListTools => {
            let available = state.tool_manager.available_tools().await;
            let tools: Vec<ToolInfo> = Tool::all()
                .iter()
                .map(|t| ToolInfo {
                    tool: *t,
                    enabled: state.tool_manager.get_usage()
                        .iter()
                        .find(|u| u.tool == *t)
                        .map(|u| u.is_available)
                        .unwrap_or(false),
                    available: available.contains(t),
                    priority: match t {
                        Tool::Claude => 1,
                        Tool::Gemini => 2,
                        Tool::Codex => 3,
                        Tool::Copilot => 4,
                        Tool::Perplexity => 5,
                        Tool::Cursor => 6,
                        Tool::Ollama => 7,
                    },
                })
                .collect();

            response_tx.send(ServerMessage::ToolList {
                tools,
                current: *current_tool,
            }).await.ok();
        }

        ClientMessage::SyncRequest { path, mode } => {
            let sync_dir = state.sync_manager.user_sync_dir(
                &session_id.map(|s| s.to_string()).unwrap_or_default()
            );
            let full_path = sync_dir.join(&path);

            match state.sync_manager.list_files(&full_path) {
                Ok(files) => {
                    response_tx.send(ServerMessage::SyncResponse { files, mode }).await.ok();
                }
                Err(e) => {
                    response_tx.send(ServerMessage::Error {
                        code: ErrorCode::SyncError,
                        message: e.to_string(),
                    }).await.ok();
                }
            }
        }

        ClientMessage::Ping { timestamp } => {
            response_tx.send(ServerMessage::Pong {
                timestamp,
                server_time: chrono::Utc::now().timestamp_millis() as u64,
            }).await.ok();
        }

        ClientMessage::VersionCheck => {
            let server_version = env!("CARGO_PKG_VERSION").to_string();
            let update_available = check_for_updates(&state.config.updates).await;

            response_tx.send(ServerMessage::VersionInfo {
                server_version,
                protocol_version: PROTOCOL_VERSION,
                min_client_version: state.config.updates.min_client_version.clone(),
                update_available,
                update_url: state.config.updates.client_download_url.clone(),
                update_message: state.config.updates.update_message.clone(),
            }).await.ok();
        }

        ClientMessage::Disconnect => {
            if let Some(sid) = session_id.take() {
                state.session_manager.remove_session(sid);
            }
        }

        _ => {
            response_tx.send(ServerMessage::Error {
                code: ErrorCode::InvalidMessage,
                message: "Unhandled message type".to_string(),
            }).await.ok();
        }
    }

    Ok(())
}

fn add_user(config: &ServerConfig, username: &str, admin: bool) -> Result<()> {
    let user_manager = UserManager::new(&config.storage.db_path)?;
    let user = user_manager.create_user(username, admin)?;
    println!("Created user: {} (ID: {})", user.username, user.id);
    if admin {
        println!("User is an administrator");
    }
    Ok(())
}

fn remove_user(config: &ServerConfig, username: &str) -> Result<()> {
    let user_manager = UserManager::new(&config.storage.db_path)?;
    let user = user_manager.get_user_by_username(username)?;
    user_manager.delete_user(user.id)?;
    println!("Removed user: {}", username);
    Ok(())
}

fn list_users(config: &ServerConfig) -> Result<()> {
    let user_manager = UserManager::new(&config.storage.db_path)?;
    let users = user_manager.list_users()?;

    if users.is_empty() {
        println!("No users registered");
        return Ok(());
    }

    println!("{:<36} {:<20} {:<10} {:<20}", "ID", "Username", "Admin", "Last Login");
    println!("{}", "-".repeat(90));

    for user in users {
        let last_login = user.last_login
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "Never".to_string());

        println!(
            "{:<36} {:<20} {:<10} {:<20}",
            user.id,
            user.username,
            if user.is_admin { "Yes" } else { "No" },
            last_login
        );
    }

    Ok(())
}

fn show_usage(config: &ServerConfig, detailed: bool) -> Result<()> {
    let tracker = UsageTracker::new(&config.storage.db_path)?;
    let stats = tracker.get_all_stats()?;

    println!("\nUsage Statistics");
    println!("{}", "=".repeat(70));

    for stat in &stats {
        println!("\n{}", stat.tool.display_name());
        println!("{}", "-".repeat(40));
        println!("  Requests:      {}", stat.requests);
        println!("  Tokens Used:   {}", stat.tokens_used);
        println!("  Errors:        {}", stat.errors);
        println!("  Rate Limits:   {}", stat.rate_limit_hits);
        if let Some(last) = stat.last_used {
            println!("  Last Used:     {}", last.format("%Y-%m-%d %H:%M:%S"));
        }
    }

    if detailed {
        println!("\n\nDaily Statistics (Last 7 Days)");
        println!("{}", "=".repeat(70));

        let today = chrono::Utc::now();
        let week_ago = today - chrono::Duration::days(7);
        let daily = tracker.get_daily_stats(
            &week_ago.format("%Y-%m-%d").to_string(),
            &today.format("%Y-%m-%d").to_string(),
        )?;

        println!("{:<12} {:<15} {:<10} {:<10} {:<10}", "Date", "Tool", "Requests", "Tokens", "Errors");
        println!("{}", "-".repeat(60));

        for stat in daily {
            println!(
                "{:<12} {:<15} {:<10} {:<10} {:<10}",
                stat.date,
                stat.tool.display_name(),
                stat.total_requests,
                stat.total_tokens,
                stat.total_errors
            );
        }
    }

    Ok(())
}

fn generate_config(output: &PathBuf) -> Result<()> {
    let config_content = config::generate_example_config();
    std::fs::write(output, config_content)?;
    println!("Generated example config: {:?}", output);
    Ok(())
}

fn generate_certs(output: &PathBuf, cn: &str) -> Result<()> {
    use rcgen::{CertificateParams, KeyPair, DnType, Issuer};

    std::fs::create_dir_all(output)?;

    let mut ca_params = CertificateParams::default();
    ca_params.distinguished_name.push(DnType::CommonName, format!("{} CA", cn));
    ca_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);

    let ca_key_pair = KeyPair::generate()?;
    let ca_cert = ca_params.self_signed(&ca_key_pair)?;

    let ca_cert_pem = ca_cert.pem();
    let ca_key_pem = ca_key_pair.serialize_pem();

    let issuer = Issuer::from_params(&ca_params, ca_key_pair);

    let mut server_params = CertificateParams::default();
    server_params.distinguished_name.push(DnType::CommonName, cn.to_string());
    server_params.subject_alt_names = vec![
        rcgen::SanType::DnsName(cn.try_into()?),
        rcgen::SanType::DnsName("localhost".try_into()?),
    ];

    let server_key_pair = KeyPair::generate()?;
    let server_cert = server_params.signed_by(&server_key_pair, &issuer)?;

    std::fs::write(output.join("ca.crt"), ca_cert_pem)?;
    std::fs::write(output.join("ca.key"), ca_key_pem)?;
    std::fs::write(output.join("server.crt"), server_cert.pem())?;
    std::fs::write(output.join("server.key"), server_key_pair.serialize_pem())?;

    println!("Generated certificates in {:?}", output);
    println!("  ca.crt     - CA certificate");
    println!("  ca.key     - CA private key");
    println!("  server.crt - Server certificate");
    println!("  server.key - Server private key");

    Ok(())
}

fn print_connection_info(config: &ServerConfig, addr: &SocketAddr) {
    println!();
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║              POLYGLOT-AI SERVER RUNNING                          ║");
    println!("╠══════════════════════════════════════════════════════════════════╣");
    println!("║  Server Address: {:^47} ║", addr);
    println!("║                                                                  ║");
    println!("║  Client Download:                                                ║");
    println!("║    GitHub: https://github.com/polyglot-ai/releases               ║");
    println!("║                                                                  ║");
    println!("║  Connect with:                                                   ║");
    println!("║    polyglot --server {}                          ║",
        format!("{:^20}", addr));
    println!("║                                                                  ║");
    println!("║  Certificates required:                                          ║");
    println!("║    - {:^55} ║", config.auth.ca_path.display());
    println!("║    - Client certificate signed by CA                             ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!();
    info!("Server listening on {}", addr);
}

fn generate_invite(config: &ServerConfig, expiry_hours: u32, max_uses: u32, is_admin: bool) -> Result<()> {
    let invite_manager = auth::InviteManager::new();
    let invite = invite_manager.generate_invite(expiry_hours, max_uses, is_admin, Some("cli".to_string()));

    println!();
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║                    INVITE CODE GENERATED                         ║");
    println!("╠══════════════════════════════════════════════════════════════════╣");
    println!("║                                                                  ║");
    println!("║    Code:    {:^8}                                           ║", invite.code);
    println!("║    Expires: {}                                   ║", invite.expires_at.format("%Y-%m-%d %H:%M UTC"));
    println!("║    Uses:    {}/{}                                               ║", invite.uses, invite.max_uses);
    println!("║    Admin:   {:^5}                                              ║", if is_admin { "Yes" } else { "No" });
    println!("║                                                                  ║");
    println!("║  Share this code with users to let them register.                ║");
    println!("║  They can use it when connecting to the server.                  ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!();

    let invite_path = config.storage.db_path.parent()
        .map(|p| p.join("pending_invites.json"))
        .unwrap_or_else(|| PathBuf::from("pending_invites.json"));

    let mut invites: Vec<auth::InviteCode> = if invite_path.exists() {
        let content = std::fs::read_to_string(&invite_path).unwrap_or_default();
        serde_json::from_str(&content).unwrap_or_default()
    } else {
        vec![]
    };
    invites.push(invite);

    if let Some(parent) = invite_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    std::fs::write(&invite_path, serde_json::to_string_pretty(&invites)?)?;

    Ok(())
}

fn show_server_info(config: &ServerConfig) -> Result<()> {
    println!();
    println!("╔══════════════════════════════════════════════════════════════════╗");
    println!("║              POLYGLOT-AI SERVER INFORMATION                      ║");
    println!("╠══════════════════════════════════════════════════════════════════╣");
    println!("║                                                                  ║");
    println!("║  Server Configuration:                                           ║");
    println!("║    Bind Address: {:^47} ║", config.server.bind_address);
    println!("║    Max Connections: {:^44} ║", config.server.max_connections);
    println!("║                                                                  ║");
    println!("║  Client Download Links:                                          ║");
    println!("║    Windows: https://github.com/polyglot-ai/releases/latest       ║");
    println!("║    macOS:   https://github.com/polyglot-ai/releases/latest       ║");
    println!("║    Linux:   https://github.com/polyglot-ai/releases/latest       ║");
    println!("║                                                                  ║");
    println!("║  Or build from source:                                           ║");
    println!("║    cargo install polyglot-client                                 ║");
    println!("║                                                                  ║");
    println!("║  Connection Steps:                                               ║");
    println!("║    1. Download the client for your platform                      ║");
    println!("║    2. Copy the CA certificate from the server                    ║");
    println!("║    3. Generate client certificate using:                         ║");
    println!("║       polyglot-server generate-certs --output ./client-certs     ║");
    println!("║    4. Connect using:                                             ║");
    println!("║       polyglot --server {} --cert client.crt     ║",
        format!("{:^12}", config.server.bind_address));
    println!("║                                                                  ║");
    println!("║  Certificates Location:                                          ║");
    println!("║    CA:     {:^53} ║", config.auth.ca_path.display());
    println!("║    Server: {:^53} ║", config.auth.cert_path.display());
    println!("║                                                                  ║");
    println!("╚══════════════════════════════════════════════════════════════════╝");
    println!();

    let invite_path = config.storage.db_path.parent()
        .map(|p| p.join("pending_invites.json"))
        .unwrap_or_else(|| PathBuf::from("pending_invites.json"));

    if invite_path.exists() {
        if let Ok(content) = std::fs::read_to_string(&invite_path) {
            if let Ok(invites) = serde_json::from_str::<Vec<auth::InviteCode>>(&content) {
                let valid_invites: Vec<_> = invites.iter().filter(|i| i.is_valid()).collect();
                if !valid_invites.is_empty() {
                    println!("Active Invite Codes:");
                    println!("{}", "-".repeat(50));
                    for inv in valid_invites {
                        println!("  Code: {}  |  Uses: {}/{}  |  Expires: {}  |  Admin: {}",
                            inv.code,
                            inv.uses,
                            inv.max_uses,
                            inv.expires_at.format("%Y-%m-%d"),
                            if inv.is_admin { "Yes" } else { "No" }
                        );
                    }
                    println!();
                }
            }
        }
    }

    Ok(())
}

/// Check GitHub releases for available updates
async fn check_for_updates(settings: &config::UpdateSettings) -> bool {
    if !settings.check_updates {
        return false;
    }

    let client = match reqwest::Client::builder()
        .user_agent("polyglot-server")
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };

    match client.get(&settings.update_check_url).send().await {
        Ok(response) => {
            if let Ok(json) = response.json::<serde_json::Value>().await {
                if let Some(tag_name) = json.get("tag_name").and_then(|t| t.as_str()) {
                    let remote_version = tag_name.trim_start_matches('v');
                    let current_version = env!("CARGO_PKG_VERSION");
                    return version_compare(remote_version, current_version) == std::cmp::Ordering::Greater;
                }
            }
        }
        Err(_) => {}
    }

    false
}

/// Simple semantic version comparison
fn version_compare(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |v: &str| -> Vec<u32> {
        v.split('.')
            .filter_map(|s| s.parse().ok())
            .collect()
    };

    let a_parts = parse(a);
    let b_parts = parse(b);

    for i in 0..3 {
        let a_val = a_parts.get(i).copied().unwrap_or(0);
        let b_val = b_parts.get(i).copied().unwrap_or(0);
        match a_val.cmp(&b_val) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

// ============================================================================
// UPDATE SYSTEM
// ============================================================================

async fn run_update(check_only: bool, force: bool) -> Result<()> {
    println!();
    println!("\x1b[36m╔══════════════════════════════════════════════════════════════════╗\x1b[0m");
    println!("\x1b[36m║                     UPDATE CHECK                                 ║\x1b[0m");
    println!("\x1b[36m╚══════════════════════════════════════════════════════════════════╝\x1b[0m");
    println!();

    let current_version = env!("CARGO_PKG_VERSION");
    println!("  Current version: \x1b[33mv{}\x1b[0m", current_version);
    println!("  Checking for updates...");
    println!();

    let update_info = check_for_updates_github("polyglot-server").await?;

    if update_info.update_available || force {
        println!("\x1b[32m  ✓ New version available: v{}\x1b[0m", update_info.latest_version);
        println!();
        if let Some(notes) = &update_info.release_notes {
            println!("  Release notes:");
            for line in notes.lines().take(10) {
                println!("    {}", line);
            }
            println!();
        }

        if check_only {
            println!("  Run without --check-only to install the update.");
            return Ok(());
        }

        if let Some(url) = &update_info.download_url {
            println!("  Download URL: {}", url);
        }
        println!();
        print!("  Do you want to install this update? [y/N]: ");
        std::io::Write::flush(&mut std::io::stdout())?;

        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;

        if input.trim().to_lowercase() == "y" {
            println!();
            perform_update(&update_info).await?;
        } else {
            println!("  Update cancelled.");
        }
    } else {
        println!("\x1b[32m  ✓ You are running the latest version!\x1b[0m");
    }

    println!();
    Ok(())
}

async fn check_for_updates_github(binary_name: &str) -> Result<polyglot_common::updater::UpdateInfo> {
    use polyglot_common::updater::*;

    let client = reqwest::Client::builder()
        .user_agent("polyglot-server")
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let url = "https://api.github.com/repos/tugcantopaloglu/selfhosted-ai-code-platform/releases/latest";

    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        anyhow::bail!("GitHub API returned status: {}", response.status());
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
    println!("    Backup created: {}", backup_info.backup_path.display());

    // Phase 2: Download new version
    print_status(&UpdateStatus {
        phase: UpdatePhase::Downloading,
        message: format!("Downloading v{}...", update_info.latest_version),
        progress: Some(0),
    });

    let client = reqwest::Client::builder()
        .user_agent("polyglot-server")
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    let response = client.get(download_url).send().await?;

    if !response.status().is_success() {
        restore_backup(&backup_info)?;
        anyhow::bail!("Failed to download update: {}", response.status());
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

    #[cfg(windows)]
    let new_exe_path = current_exe.with_extension("exe.new");
    #[cfg(not(windows))]
    let new_exe_path = current_exe.with_extension("new");

    if let Err(e) = std::fs::write(&new_exe_path, &new_binary) {
        restore_backup(&backup_info)?;
        anyhow::bail!("Failed to write new binary: {}", e);
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&new_exe_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&new_exe_path, perms)?;
    }

    // Phase 4: Verify new binary
    print_status(&UpdateStatus {
        phase: UpdatePhase::Verifying,
        message: "Verifying new binary...".to_string(),
        progress: None,
    });

    if !verify_binary(&new_exe_path) {
        let _ = std::fs::remove_file(&new_exe_path);
        restore_backup(&backup_info)?;
        anyhow::bail!("Downloaded binary failed verification");
    }

    // Replace the current executable
    #[cfg(windows)]
    {
        let old_exe = current_exe.with_extension("exe.old");
        if old_exe.exists() {
            let _ = std::fs::remove_file(&old_exe);
        }
        std::fs::rename(&current_exe, &old_exe)?;
        std::fs::rename(&new_exe_path, &current_exe)?;
        let _ = std::fs::remove_file(&old_exe);
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(&new_exe_path, &current_exe)?;
    }

    // Phase 5: Cleanup
    let _ = cleanup_old_backups(3);

    print_status(&UpdateStatus {
        phase: UpdatePhase::Complete,
        message: format!("Successfully updated to v{}!", update_info.latest_version),
        progress: None,
    });

    println!();
    println!("\x1b[32m  ╔══════════════════════════════════════════════════════════════════╗\x1b[0m");
    println!("\x1b[32m  ║                     UPDATE SUCCESSFUL!                           ║\x1b[0m");
    println!("\x1b[32m  ╠══════════════════════════════════════════════════════════════════╣\x1b[0m");
    println!("\x1b[32m  ║  Updated from v{:<10} to v{:<10}                       ║\x1b[0m",
             current_version, update_info.latest_version);
    println!("\x1b[32m  ║                                                                  ║\x1b[0m");
    println!("\x1b[32m  ║  Please restart the server to use the new version.              ║\x1b[0m");
    println!("\x1b[32m  ╚══════════════════════════════════════════════════════════════════╝\x1b[0m");
    println!();

    Ok(())
}
