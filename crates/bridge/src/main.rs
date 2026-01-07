mod dashboard;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use futures::{SinkExt, StreamExt};
use http::{Request, Response, StatusCode};
use mdns_sd::{ServiceDaemon, ServiceInfo};
use parking_lot::RwLock;
use quinn::{ClientConfig as QuinnClientConfig, Endpoint};
use rcgen::{CertificateParams, DnType, SanType};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::Command;
use tokio_rustls::TlsAcceptor;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use polyglot_common::{
    decode_message, encode_message,
    ClientMessage, ServerMessage,
    ErrorCode, OutputType, Tool, ToolInfo, ToolHealthInfo, CacheStats, ExportFormat,
    PROTOCOL_VERSION, MAX_MESSAGE_SIZE,
    RateLimiter, RateLimitConfig,
    ResponseCache, CacheConfig,
    QuotaTracker, QuotaConfig,
    HealthChecker, HealthCheckConfig,
    MetricsCollector,
    ContextWindowManager, ContextWindowConfig,
    Database, AuditLogEntry, StoredSession,
};

#[derive(Parser, Debug)]
#[command(name = "polyglot-bridge")]
#[command(about = "Polyglot-AI WebSocket bridge for mobile clients")]
struct Cli {
    /// Optional config file (TOML)
    #[arg(long)]
    config: Option<PathBuf>,

    /// Write an example config and exit
    #[arg(long)]
    generate_config: Option<PathBuf>,

    /// Print a QR payload for this bridge and exit
    #[arg(long, default_value_t = false)]
    print_qr: bool,

    /// WebSocket listen address
    #[arg(long, default_value = "0.0.0.0:8787")]
    listen: String,

    /// Polyglot server address (QUIC)
    #[arg(long, default_value = "127.0.0.1:4433")]
    server: String,

    /// Bridge mode: server (QUIC) or local (polyglot-local)
    #[arg(long, default_value = "server", value_parser = ["server", "local"])]
    mode: String,

    /// polyglot-local binary (used in local mode)
    #[arg(long, default_value = "polyglot-local")]
    local_bin: String,

    /// Client certificate for QUIC auth
    #[arg(long, default_value = "./certs/client.crt")]
    cert: PathBuf,

    /// Client key for QUIC auth
    #[arg(long, default_value = "./certs/client.key")]
    key: PathBuf,

    /// CA certificate
    #[arg(long, default_value = "./certs/ca.crt")]
    ca: PathBuf,

    /// Optional shared token to secure the bridge
    #[arg(long)]
    token: Option<String>,

    /// Enable TLS for the WebSocket listener (wss://)
    #[arg(long, default_value_t = false)]
    tls: bool,

    /// TLS certificate path
    #[arg(long, default_value = "./certs/bridge.crt")]
    tls_cert: PathBuf,

    /// TLS key path
    #[arg(long, default_value = "./certs/bridge.key")]
    tls_key: PathBuf,

    /// Auto-generate TLS cert/key if missing
    #[arg(long, default_value_t = true)]
    tls_generate: bool,

    /// Broadcast mDNS service for LAN discovery
    #[arg(long, default_value_t = true)]
    mdns: bool,

    /// mDNS service name
    #[arg(long, default_value = "polyglot-bridge")]
    mdns_name: String,

    /// Hostname or IP for QR payloads
    #[arg(long)]
    qr_host: Option<String>,

    /// Rclone remote for Drive sync (example: gdrive:polyglot-ai)
    #[arg(long)]
    drive_remote: Option<String>,

    /// Local path for Drive sync
    #[arg(long)]
    drive_path: Option<PathBuf>,

    /// QUIC idle timeout in seconds
    #[arg(long, default_value_t = 30)]
    timeout: u64,

    /// Maximum requests per minute per client
    #[arg(long, default_value_t = 100)]
    rate_limit: u32,

    /// Maximum connections per IP address
    #[arg(long, default_value_t = 10)]
    max_connections_per_ip: u32,

    /// Maximum prompt length in characters
    #[arg(long, default_value_t = 100000)]
    max_prompt_length: usize,

    /// Enable response caching
    #[arg(long, default_value_t = false)]
    enable_cache: bool,

    /// Cache TTL in seconds
    #[arg(long, default_value_t = 3600)]
    cache_ttl: u64,

    /// Token expiry in hours (0 = no expiry)
    #[arg(long, default_value_t = 24)]
    token_expiry_hours: u64,

    /// Enable admin dashboard
    #[arg(long, default_value_t = false)]
    dashboard: bool,

    /// Dashboard listen address
    #[arg(long, default_value = "127.0.0.1:8788")]
    dashboard_listen: String,

    /// Dashboard authentication token (optional)
    #[arg(long)]
    dashboard_token: Option<String>,

    /// Enable auto failover to alternative tools when primary is unhealthy
    #[arg(long, default_value_t = true)]
    auto_failover: bool,

    /// Enable request audit logging to database
    #[arg(long, default_value_t = false)]
    audit_log: bool,

    /// Database path for persistent storage
    #[arg(long, default_value = "./bridge-data/polyglot.db")]
    database: PathBuf,

    #[arg(short, long)]
    verbose: bool,
}

/// Shared bridge state for rate limiting, quotas, metrics, etc.
struct BridgeState {
    rate_limiter: RateLimiter,
    quota_tracker: QuotaTracker,
    health_checker: HealthChecker,
    metrics: MetricsCollector,
    response_cache: ResponseCache<String, String>,
    context_manager: ContextWindowManager,
    config: BridgeConfig,
    token_sessions: RwLock<HashMap<String, TokenSession>>,
    database: Option<Database>,
}

#[derive(Debug, Clone)]
struct TokenSession {
    token: String,
    created_at: chrono::DateTime<chrono::Utc>,
    expires_at: chrono::DateTime<chrono::Utc>,
    last_used: chrono::DateTime<chrono::Utc>,
}

impl BridgeState {
    fn new(config: BridgeConfig) -> Arc<Self> {
        // Initialize database if audit logging is enabled
        let database = if config.audit_log {
            // Create parent directory if needed
            if let Some(parent) = config.database_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            match Database::open(&config.database_path) {
                Ok(db) => {
                    info!("Opened database at {:?}", config.database_path);
                    Some(db)
                }
                Err(e) => {
                    warn!("Failed to open database: {}, audit logging disabled", e);
                    None
                }
            }
        } else {
            None
        };

        Arc::new(Self {
            rate_limiter: RateLimiter::new(RateLimitConfig {
                max_requests: config.rate_limit,
                window_seconds: 60,
                max_connections_per_ip: config.max_connections_per_ip,
                cleanup_interval_seconds: 300,
            }),
            quota_tracker: QuotaTracker::new(QuotaConfig::default()),
            health_checker: HealthChecker::new(HealthCheckConfig::default()),
            metrics: MetricsCollector::new(),
            response_cache: ResponseCache::new(CacheConfig {
                max_entries: 1000,
                ttl_seconds: config.cache_ttl,
                max_memory_bytes: 100 * 1024 * 1024,
            }),
            context_manager: ContextWindowManager::new(ContextWindowConfig {
                max_tokens: 128000,
                response_reserve: 4000,
                estimation_method: polyglot_common::TokenEstimationMethod::CharDivide4,
            }),
            token_sessions: RwLock::new(HashMap::new()),
            database,
            config,
        })
    }

    /// Get tool with automatic failover if enabled and tool is unhealthy
    fn get_tool_with_failover(&self, requested: Tool) -> Tool {
        if !self.config.auto_failover {
            return requested;
        }

        self.health_checker
            .get_tool_with_fallback(requested, Tool::all())
            .unwrap_or(requested)
    }

    /// Log an audit entry to the database
    fn log_audit(&self, entry: &AuditLogEntry) {
        if let Some(ref db) = self.database {
            if let Err(e) = db.log_audit(entry) {
                warn!("Failed to log audit entry: {}", e);
            }
        }
    }

    /// Save a session to persistent storage
    fn save_session(&self, session: &StoredSession) {
        if let Some(ref db) = self.database {
            if let Err(e) = db.save_session(session) {
                warn!("Failed to save session: {}", e);
            }
        }
    }

    /// Load a session from persistent storage
    fn load_session(&self, session_id: &str) -> Option<StoredSession> {
        if let Some(ref db) = self.database {
            match db.get_session(session_id) {
                Ok(Some(session)) => return Some(session),
                Ok(None) => return None,
                Err(e) => {
                    warn!("Failed to load session {}: {}", session_id, e);
                    return None;
                }
            }
        }
        None
    }

    /// Delete a session from persistent storage
    fn delete_session(&self, session_id: &str) {
        if let Some(ref db) = self.database {
            if let Err(e) = db.delete_session(session_id) {
                warn!("Failed to delete session {}: {}", session_id, e);
            }
        }
    }

    /// Cleanup expired sessions
    fn cleanup_expired_sessions(&self) -> u64 {
        if let Some(ref db) = self.database {
            match db.cleanup_expired_sessions() {
                Ok(count) => return count,
                Err(e) => {
                    warn!("Failed to cleanup sessions: {}", e);
                }
            }
        }
        0
    }

    fn check_rate_limit(&self, ip: &str) -> Result<(), ServerMessage> {
        if !self.rate_limiter.check(ip).is_allowed() {
            return Err(ServerMessage::Error {
                code: ErrorCode::RateLimited,
                message: "Rate limit exceeded. Please slow down.".to_string(),
            });
        }
        Ok(())
    }

    fn check_connection_rate(&self, ip: &str) -> Result<(), ServerMessage> {
        if !self.rate_limiter.check_connection(ip).is_allowed() {
            return Err(ServerMessage::Error {
                code: ErrorCode::ConnectionRateLimited,
                message: "Too many connection attempts. Please wait.".to_string(),
            });
        }
        Ok(())
    }

    fn validate_prompt(&self, prompt: &str) -> Result<(), ServerMessage> {
        if prompt.len() > self.config.max_prompt_length {
            return Err(ServerMessage::Error {
                code: ErrorCode::PromptTooLong,
                message: format!(
                    "Prompt too long: {} chars (max {})",
                    prompt.len(),
                    self.config.max_prompt_length
                ),
            });
        }

        if !self.context_manager.validate_prompt(prompt).is_valid() {
            return Err(ServerMessage::Error {
                code: ErrorCode::PromptTooLong,
                message: "Prompt exceeds context window limit".to_string(),
            });
        }

        Ok(())
    }

    fn check_quota(&self, user_id: &str) -> Result<(), ServerMessage> {
        if !self.quota_tracker.check(user_id).is_allowed() {
            return Err(ServerMessage::Error {
                code: ErrorCode::QuotaExceeded,
                message: "Usage quota exceeded".to_string(),
            });
        }
        Ok(())
    }

    fn validate_token(&self, token: &str) -> bool {
        if self.config.token_expiry_hours == 0 {
            return true; // No expiry
        }

        let sessions = self.token_sessions.read();
        if let Some(session) = sessions.get(token) {
            if Utc::now() < session.expires_at {
                return true;
            }
        }
        false
    }

    fn create_token_session(&self, token: &str) -> TokenSession {
        let now = Utc::now();
        let expires_at = now + chrono::Duration::hours(self.config.token_expiry_hours as i64);
        let session = TokenSession {
            token: token.to_string(),
            created_at: now,
            expires_at,
            last_used: now,
        };

        let mut sessions = self.token_sessions.write();
        sessions.insert(token.to_string(), session.clone());
        session
    }

    fn refresh_token(&self, old_token: &str) -> Option<TokenSession> {
        let mut sessions = self.token_sessions.write();
        if sessions.remove(old_token).is_some() {
            let new_token = polyglot_common::crypto::generate_token();
            let now = Utc::now();
            let session = TokenSession {
                token: new_token.clone(),
                created_at: now,
                expires_at: now + chrono::Duration::hours(self.config.token_expiry_hours as i64),
                last_used: now,
            };
            sessions.insert(new_token, session.clone());
            return Some(session);
        }
        None
    }

    fn get_health_status(&self) -> Vec<ToolHealthInfo> {
        self.health_checker.get_status()
    }

    fn get_cache_stats(&self) -> CacheStats {
        self.response_cache.stats()
    }
}

#[derive(Clone)]
struct BridgeConfig {
    mode: BridgeMode,
    listen: String,
    server: String,
    cert: PathBuf,
    key: PathBuf,
    ca: PathBuf,
    token: Option<String>,
    timeout: u64,
    local_bin: String,
    tls_enabled: bool,
    tls_cert: PathBuf,
    tls_key: PathBuf,
    tls_generate: bool,
    mdns_enabled: bool,
    mdns_name: String,
    qr_host: Option<String>,
    drive_remote: Option<String>,
    drive_path: PathBuf,
    // Security & feature settings
    rate_limit: u32,
    max_connections_per_ip: u32,
    max_prompt_length: usize,
    enable_cache: bool,
    cache_ttl: u64,
    token_expiry_hours: u64,
    auto_failover: bool,
    audit_log: bool,
    database_path: PathBuf,
}

#[derive(Copy, Clone, Debug)]
enum BridgeMode {
    Server,
    Local,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BridgeConfigFile {
    listen: Option<String>,
    server: Option<String>,
    mode: Option<String>,
    local_bin: Option<String>,
    cert: Option<PathBuf>,
    key: Option<PathBuf>,
    ca: Option<PathBuf>,
    token: Option<String>,
    timeout: Option<u64>,
    tls_enabled: Option<bool>,
    tls_cert: Option<PathBuf>,
    tls_key: Option<PathBuf>,
    tls_generate: Option<bool>,
    mdns_enabled: Option<bool>,
    mdns_name: Option<String>,
    qr_host: Option<String>,
    drive_remote: Option<String>,
    drive_path: Option<PathBuf>,
}

#[derive(Copy, Clone)]
enum Codec {
    Json,
    Msgpack,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BridgeControl {
    Status,
    DriveSync { direction: Option<String> },
    DriveStatus,
    QrPayload,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum BridgeEvent {
    Status {
        mode: String,
        server: String,
        listen: String,
        tls_enabled: bool,
        mdns_enabled: bool,
        drive_remote: Option<String>,
        last_sync: Option<String>,
        uptime_seconds: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        active_connections: Option<u32>,
        #[serde(skip_serializing_if = "Option::is_none")]
        total_requests: Option<u64>,
    },
    DriveSyncResult {
        ok: bool,
        message: String,
        finished_at: String,
    },
    DriveStatus {
        configured: bool,
        remote: Option<String>,
        local_path: String,
        last_sync: Option<String>,
    },
    QrPayload {
        payload: String,
    },
}

type WsStream<S> = tokio_tungstenite::WebSocketStream<S>;
type WsWrite<S> = futures::stream::SplitSink<WsStream<S>, tokio_tungstenite::tungstenite::Message>;

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

    if let Some(path) = cli.generate_config.as_ref() {
        write_example_config(path)?;
        println!("Generated bridge config at {:?}", path);
        return Ok(());
    }

    let config = if let Some(path) = cli.config.as_ref() {
        load_config(path)?
    } else {
        BridgeConfig::from_cli(&cli)
    };

    if cli.print_qr {
        print_qr_payload(&config)?;
        return Ok(());
    }

    let start_time = Instant::now();
    let _mdns = if config.mdns_enabled {
        Some(start_mdns(&config)?)
    } else {
        None
    };

    let listener = TcpListener::bind(&config.listen).await
        .with_context(|| format!("Failed to bind to {}", config.listen))?;

    let tls_acceptor = if config.tls_enabled {
        Some(build_tls_acceptor(&config)?)
    } else {
        None
    };

    // Create shared bridge state
    let state = BridgeState::new(config.clone());

    // Spawn background cleanup task for rate limiter
    let cleanup_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(300));
        loop {
            interval.tick().await;
            cleanup_state.rate_limiter.cleanup();
        }
    });

    // Spawn background cleanup task for sessions
    let session_cleanup_state = state.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3600)); // Every hour
        loop {
            interval.tick().await;
            let cleaned = session_cleanup_state.cleanup_expired_sessions();
            if cleaned > 0 {
                info!("Cleaned up {} expired sessions", cleaned);
            }
        }
    });

    let scheme = if config.tls_enabled { "wss" } else { "ws" };
    info!(
        "Polyglot bridge listening on {}://{}/ws (mode: {:?})",
        scheme,
        config.listen,
        config.mode
    );
    info!(
        "Security: rate_limit={}/min, max_conn_per_ip={}, cache={}",
        config.rate_limit,
        config.max_connections_per_ip,
        if config.enable_cache { "enabled" } else { "disabled" }
    );

    // Start dashboard if enabled
    if cli.dashboard {
        let dashboard_config = dashboard::DashboardConfig {
            enabled: true,
            listen: cli.dashboard_listen.clone(),
            require_auth: cli.dashboard_token.is_some(),
            auth_token: cli.dashboard_token.clone(),
        };
        let dashboard_state = state.clone();
        tokio::spawn(async move {
            if let Err(e) = dashboard::start_dashboard(dashboard_config, dashboard_state).await {
                error!("Dashboard error: {}", e);
            }
        });
    }

    loop {
        let (stream, addr) = listener.accept().await?;
        let config = config.clone();
        let tls_acceptor = tls_acceptor.clone();
        let started = start_time;
        let state = state.clone();

        // Check connection rate limit
        let ip = addr.ip().to_string();
        if let Err(msg) = state.check_connection_rate(&ip) {
            warn!("Connection rate limited for {}", ip);
            continue;
        }

        state.metrics.connection_opened();

        tokio::spawn(async move {
            let result = handle_socket(stream, addr, config, tls_acceptor, started, state.clone()).await;
            state.metrics.connection_closed();
            if let Err(err) = result {
                error!("Connection {} failed: {}", addr, err);
            }
        });
    }
}

async fn handle_socket(
    stream: tokio::net::TcpStream,
    addr: SocketAddr,
    config: BridgeConfig,
    tls_acceptor: Option<TlsAcceptor>,
    started: Instant,
    state: Arc<BridgeState>,
) -> Result<()> {
    let token_required = config.token.clone();

    let ws_stream = if let Some(acceptor) = tls_acceptor {
        let tls_stream = acceptor.accept(stream).await?;
        accept_ws(tls_stream, token_required).await?
    } else {
        accept_ws(stream, token_required).await?
    };

    info!("WebSocket client connected: {}", addr);
    let result = match config.mode {
        BridgeMode::Server => handle_server_bridge(ws_stream, &config, started, state.clone()).await,
        BridgeMode::Local => handle_local_bridge(ws_stream, &config, started, state.clone(), &addr.ip().to_string()).await,
    };

    info!("WebSocket client disconnected: {}", addr);
    result
}

async fn accept_ws<S>(
    stream: S,
    token_required: Option<String>,
) -> Result<WsStream<S>>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let ws_stream = tokio_tungstenite::accept_hdr_async(stream, move |req: &Request<()>, mut resp: Response<()>| {
        // Add CORS headers for cross-origin requests
        resp.headers_mut().insert(
            "Access-Control-Allow-Origin",
            "*".parse().unwrap(),
        );
        resp.headers_mut().insert(
            "Access-Control-Allow-Methods",
            "GET, POST, OPTIONS".parse().unwrap(),
        );
        resp.headers_mut().insert(
            "Access-Control-Allow-Headers",
            "Content-Type, Authorization, X-Requested-With".parse().unwrap(),
        );
        resp.headers_mut().insert(
            "Access-Control-Max-Age",
            "86400".parse().unwrap(),
        );

        if req.uri().path() != "/ws" {
            *resp.status_mut() = StatusCode::NOT_FOUND;
            return Ok(resp);
        }

        if let Some(expected) = token_required.as_ref() {
            let provided = extract_token(req.uri().query());
            if provided.as_deref() != Some(expected.as_str()) {
                *resp.status_mut() = StatusCode::UNAUTHORIZED;
                return Ok(resp);
            }
        }

        Ok(resp)
    }).await?;

    Ok(ws_stream)
}

async fn handle_server_bridge<S>(
    ws_stream: WsStream<S>,
    config: &BridgeConfig,
    started: Instant,
    state: Arc<BridgeState>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut quic = QuicBridge::connect(config).await?;
    let (mut ws_write, mut ws_read) = ws_stream.split();

    let mut codec = Codec::Json;
    let mut codec_set = false;
    let mut last_sync: Option<chrono::DateTime<chrono::Utc>> = None;

    loop {
        tokio::select! {
            ws_msg = ws_read.next() => {
                let ws_msg = match ws_msg {
                    Some(Ok(message)) => message,
                    Some(Err(err)) => return Err(err.into()),
                    None => break,
                };

                match ws_msg {
                    tokio_tungstenite::tungstenite::Message::Text(text) => {
                        if !codec_set {
                            codec = Codec::Json;
                            codec_set = true;
                        }
                        if let Some(control) = parse_bridge_control(&text) {
                            handle_bridge_control(
                                control,
                                config,
                                &mut ws_write,
                                started,
                                &mut last_sync,
                                state.clone(),
                            ).await?;
                        } else {
                            let client_msg: ClientMessage = serde_json::from_str(&text)
                                .with_context(|| "Failed to parse JSON message")?;
                            quic.send_message(&client_msg).await?;
                        }
                    }
                    tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                        if !codec_set {
                            codec = Codec::Msgpack;
                            codec_set = true;
                        }
                        let client_msg: ClientMessage = rmp_serde::from_slice(&bytes)
                            .with_context(|| "Failed to parse msgpack message")?;
                        quic.send_message(&client_msg).await?;
                    }
                    tokio_tungstenite::tungstenite::Message::Ping(payload) => {
                        ws_write.send(tokio_tungstenite::tungstenite::Message::Pong(payload)).await?;
                    }
                    tokio_tungstenite::tungstenite::Message::Close(_) => {
                        break;
                    }
                    _ => {}
                }
            }
            server_msg = quic.recv_message() => {
                let server_msg = server_msg?;
                send_ws_message(&mut ws_write, codec, &server_msg).await?;
            }
        }
    }

    Ok(())
}

async fn handle_local_bridge<S>(
    ws_stream: WsStream<S>,
    config: &BridgeConfig,
    started: Instant,
    state: Arc<BridgeState>,
    client_ip: &str,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (mut ws_write, mut ws_read) = ws_stream.split();
    let mut codec = Codec::Json;
    let mut codec_set = false;
    let mut current_tool: Option<Tool> = None;
    let mut env_entries: Vec<(String, String)> = Vec::new();
    let mut last_sync: Option<chrono::DateTime<chrono::Utc>> = None;
    let user_id = client_ip.to_string(); // Use IP as user ID for quota tracking

    while let Some(ws_msg) = ws_read.next().await {
        let ws_msg = ws_msg?;

        // Check rate limit for each message
        if let Err(err_msg) = state.check_rate_limit(client_ip) {
            send_ws_message(&mut ws_write, codec, &err_msg).await?;
            continue;
        }

        let client_msg = match ws_msg {
            tokio_tungstenite::tungstenite::Message::Text(text) => {
                if !codec_set {
                    codec = Codec::Json;
                    codec_set = true;
                }
                if let Some(control) = parse_bridge_control(&text) {
                    handle_bridge_control(
                        control,
                        config,
                        &mut ws_write,
                        started,
                        &mut last_sync,
                        state.clone(),
                    ).await?;
                    None
                } else {
                    Some(serde_json::from_str::<ClientMessage>(&text)?)
                }
            }
            tokio_tungstenite::tungstenite::Message::Binary(bytes) => {
                if !codec_set {
                    codec = Codec::Msgpack;
                    codec_set = true;
                }
                Some(rmp_serde::from_slice::<ClientMessage>(&bytes)?)
            }
            tokio_tungstenite::tungstenite::Message::Ping(payload) => {
                ws_write.send(tokio_tungstenite::tungstenite::Message::Pong(payload)).await?;
                None
            }
            tokio_tungstenite::tungstenite::Message::Close(_) => {
                break;
            }
            _ => None,
        };

        let Some(client_msg) = client_msg else { continue };

        match client_msg {
            ClientMessage::Handshake { version, .. } => {
                if version != PROTOCOL_VERSION {
                    let err = ServerMessage::Error {
                        code: ErrorCode::ProtocolMismatch,
                        message: format!(
                            "Protocol version mismatch. Server: {}, Client: {}",
                            PROTOCOL_VERSION, version
                        ),
                    };
                    send_ws_message(&mut ws_write, codec, &err).await?;
                    continue;
                }

                let response = ServerMessage::HandshakeAck {
                    version: PROTOCOL_VERSION,
                    server_id: "polyglot-bridge-local".to_string(),
                };
                send_ws_message(&mut ws_write, codec, &response).await?;
            }
            ClientMessage::Auth { .. } => {
                let response = ServerMessage::AuthResult {
                    success: true,
                    session_id: Some("local".to_string()),
                    user: None,
                    error: None,
                };
                send_ws_message(&mut ws_write, codec, &response).await?;
            }
            ClientMessage::SetEnv { entries } => {
                // Validate and filter environment variables for security
                env_entries = entries.into_iter()
                    .filter(|(key, value)| {
                        // Reject keys with dangerous characters or patterns
                        !key.is_empty() &&
                        key.chars().all(|c| c.is_alphanumeric() || c == '_') &&
                        !key.starts_with("LD_") &&
                        !key.starts_with("DYLD_") &&
                        key != "PATH" &&
                        // Reject values with shell injection characters
                        !value.contains('\0') &&
                        !value.contains('`') &&
                        !value.contains("$(")
                    })
                    .collect();
                let response = ServerMessage::EnvAck {
                    applied: env_entries.len() as u32,
                };
                send_ws_message(&mut ws_write, codec, &response).await?;
            }
            ClientMessage::SelectTool { tool } => {
                let previous = current_tool.unwrap_or(tool);
                current_tool = Some(tool);
                let response = ServerMessage::ToolSwitched {
                    from: previous,
                    to: tool,
                    reason: polyglot_common::SwitchReason::UserRequest,
                };
                send_ws_message(&mut ws_write, codec, &response).await?;
            }
            ClientMessage::Prompt { tool, message, working_dir } => {
                // Validate prompt length
                if let Err(err_msg) = state.validate_prompt(&message) {
                    send_ws_message(&mut ws_write, codec, &err_msg).await?;
                    continue;
                }

                // Check quota
                if let Err(err_msg) = state.check_quota(&user_id) {
                    send_ws_message(&mut ws_write, codec, &err_msg).await?;
                    continue;
                }

                let requested_tool = tool.or(current_tool).unwrap_or(Tool::Claude);
                let use_tool_flag = tool.is_some() || current_tool.is_some();

                // Auto failover: get healthy tool if enabled
                let selected_tool = state.get_tool_with_failover(requested_tool);
                if selected_tool != requested_tool {
                    info!("Tool failover: {} -> {}", requested_tool, selected_tool);
                }
                current_tool = Some(selected_tool);

                // Check cache first if enabled
                if config.enable_cache {
                    let cache_key = format!("{}:{}", selected_tool.as_str(), &message);
                    if let Some(cached) = state.response_cache.get(&cache_key) {
                        let response = ServerMessage::ToolResponse {
                            tool: selected_tool,
                            content: cached,
                            done: true,
                            tokens: None,
                        };
                        send_ws_message(&mut ws_write, codec, &response).await?;
                        continue;
                    }
                }

                let start_time = std::time::Instant::now();
                let result = handle_prompt_local(
                    &mut ws_write,
                    codec,
                    config,
                    selected_tool,
                    &message,
                    use_tool_flag,
                    working_dir.as_deref(),
                    &env_entries,
                ).await;

                let latency_ms_u64 = start_time.elapsed().as_millis() as u64;
                let success = result.is_ok();

                // Audit logging
                let audit_entry = AuditLogEntry::new("prompt")
                    .with_user(&user_id)
                    .with_tool(selected_tool)
                    .with_latency(latency_ms_u64)
                    .with_ip(client_ip);
                let audit_entry = if !success {
                    audit_entry.with_error("Request failed")
                } else {
                    audit_entry
                };
                state.log_audit(&audit_entry);

                if let Err(e) = result {
                    state.health_checker.record_failure(selected_tool);
                    return Err(e);
                }

                // Record metrics
                let latency_ms = start_time.elapsed().as_millis() as u32;
                state.metrics.record_request(selected_tool, true, latency_ms);
                state.health_checker.record_success(selected_tool, latency_ms);
                state.quota_tracker.record_usage(&user_id, 0); // TODO: actual token count
            }
            ClientMessage::ListTools => {
                let tools = list_local_tools(&config.local_bin).await?;
                let response = ServerMessage::ToolList {
                    tools,
                    current: current_tool,
                };
                send_ws_message(&mut ws_write, codec, &response).await?;
            }
            ClientMessage::Usage => {
                let response = ServerMessage::UsageStats {
                    stats: Vec::new(),
                    session_start: Utc::now(),
                };
                send_ws_message(&mut ws_write, codec, &response).await?;
            }
            ClientMessage::Ping { timestamp } => {
                let response = ServerMessage::Pong {
                    timestamp,
                    server_time: Utc::now().timestamp_millis() as u64,
                };
                send_ws_message(&mut ws_write, codec, &response).await?;
            }
            ClientMessage::VersionCheck => {
                let response = ServerMessage::VersionInfo {
                    server_version: "local".to_string(),
                    protocol_version: PROTOCOL_VERSION,
                    min_client_version: None,
                    update_available: false,
                    update_url: None,
                    update_message: None,
                };
                send_ws_message(&mut ws_write, codec, &response).await?;
            }
            ClientMessage::Disconnect => break,
            ClientMessage::HealthCheck => {
                let tools = state.get_health_status();
                let server_healthy = state.health_checker.all_healthy();
                let response = ServerMessage::HealthStatus {
                    tools,
                    server_healthy,
                    uptime_seconds: started.elapsed().as_secs(),
                };
                send_ws_message(&mut ws_write, codec, &response).await?;
            }
            ClientMessage::QuotaCheck => {
                let status = state.quota_tracker.get_status(&user_id);
                let response = ServerMessage::QuotaInfo {
                    daily_limit: status.daily_limit,
                    daily_used: status.daily_used,
                    monthly_limit: status.monthly_limit,
                    monthly_used: status.monthly_used,
                    reset_at: Some(status.daily_reset),
                };
                send_ws_message(&mut ws_write, codec, &response).await?;
            }
            ClientMessage::RefreshToken { current_token } => {
                if let Some(new_session) = state.refresh_token(&current_token) {
                    let response = ServerMessage::TokenRefreshed {
                        new_token: new_session.token,
                        expires_at: new_session.expires_at,
                    };
                    send_ws_message(&mut ws_write, codec, &response).await?;
                } else {
                    let response = ServerMessage::Error {
                        code: ErrorCode::TokenExpired,
                        message: "Token not found or already expired".to_string(),
                    };
                    send_ws_message(&mut ws_write, codec, &response).await?;
                }
            }
            ClientMessage::ExportHistory { session_id: _, format } => {
                // In local mode, we don't have access to full history
                // Return empty export
                let response = ServerMessage::HistoryExport {
                    format,
                    data: match format {
                        ExportFormat::Json => "[]".to_string(),
                        ExportFormat::Markdown => "# No history available in bridge mode".to_string(),
                        ExportFormat::Html => "<html><body><p>No history available in bridge mode</p></body></html>".to_string(),
                    },
                    session_count: 0,
                };
                send_ws_message(&mut ws_write, codec, &response).await?;
            }
            ClientMessage::GetMetrics => {
                let cache_stats = state.get_cache_stats();
                let metrics = state.metrics.get_metrics(cache_stats);
                let response = ServerMessage::Metrics {
                    active_connections: metrics.active_connections,
                    total_requests: metrics.total_requests,
                    requests_per_minute: metrics.requests_per_minute,
                    tool_stats: metrics.tool_stats,
                    cache_stats: metrics.cache_stats,
                    uptime_seconds: metrics.uptime_seconds,
                };
                send_ws_message(&mut ws_write, codec, &response).await?;
            }
            _ => {
                let response = ServerMessage::Error {
                    code: ErrorCode::InvalidMessage,
                    message: "Message not supported in local mode.".to_string(),
                };
                send_ws_message(&mut ws_write, codec, &response).await?;
            }
        }
    }

    Ok(())
}

async fn send_ws_message<S>(
    ws_write: &mut WsWrite<S>,
    codec: Codec,
    message: &ServerMessage,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let outgoing = match codec {
        Codec::Json => {
            let json = serde_json::to_string(message)?;
            tokio_tungstenite::tungstenite::Message::Text(json)
        }
        Codec::Msgpack => {
            let bytes = rmp_serde::to_vec(message)?;
            tokio_tungstenite::tungstenite::Message::Binary(bytes)
        }
    };

    ws_write.send(outgoing).await?;
    Ok(())
}

async fn handle_prompt_local<S>(
    ws_write: &mut WsWrite<S>,
    codec: Codec,
    config: &BridgeConfig,
    tool: Tool,
    prompt: &str,
    use_tool_flag: bool,
    working_dir: Option<&str>,
    env_entries: &[(String, String)],
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let mut cmd = Command::new(&config.local_bin);
    cmd.arg("--no-tui")
        .arg("ask")
        .arg(prompt)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    if let Some(path) = working_dir {
        cmd.arg("--project").arg(path);
    }

    for (key, value) in env_entries {
        cmd.arg("--env").arg(format!("{}={}", key, value));
    }

    if use_tool_flag {
        cmd.arg("--tool").arg(tool.as_str());
    }

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            let msg = ServerMessage::Error {
                code: ErrorCode::ToolError,
                message: format!("Failed to start {}: {}", config.local_bin, err),
            };
            send_ws_message(ws_write, codec, &msg).await?;
            return Ok(());
        }
    };

    let stdout = child.stdout.take().ok_or_else(|| anyhow::anyhow!("stdout not captured"))?;
    let stderr = child.stderr.take().ok_or_else(|| anyhow::anyhow!("stderr not captured"))?;

    let mut stdout_lines = BufReader::new(stdout).lines();
    let mut stderr_lines = BufReader::new(stderr).lines();

    let mut stdout_done = false;
    let mut stderr_done = false;

    while !(stdout_done && stderr_done) {
        tokio::select! {
            line = stdout_lines.next_line(), if !stdout_done => {
                match line? {
                    Some(text) => {
                        let msg = ServerMessage::ToolResponse {
                            tool,
                            content: text,
                            done: false,
                            tokens: None,
                        };
                        send_ws_message(ws_write, codec, &msg).await?;
                    }
                    None => stdout_done = true,
                }
            }
            line = stderr_lines.next_line(), if !stderr_done => {
                match line? {
                    Some(text) => {
                        let msg = ServerMessage::ToolOutput {
                            tool,
                            output_type: OutputType::Stderr,
                            content: text,
                        };
                        send_ws_message(ws_write, codec, &msg).await?;
                    }
                    None => stderr_done = true,
                }
            }
        }
    }

    let status = child.wait().await?;
    if !status.success() {
        let msg = ServerMessage::Error {
            code: ErrorCode::ToolError,
            message: format!("polyglot-local exited with {}", status),
        };
        send_ws_message(ws_write, codec, &msg).await?;
    }

    let done = ServerMessage::ToolResponse {
        tool,
        content: String::new(),
        done: true,
        tokens: None,
    };
    send_ws_message(ws_write, codec, &done).await?;

    Ok(())
}

async fn list_local_tools(local_bin: &str) -> Result<Vec<ToolInfo>> {
    let output = Command::new(local_bin)
        .arg("--no-tui")
        .arg("tools")
        .output()
        .await;

    let mut availability = std::collections::HashMap::new();
    for tool in Tool::all() {
        availability.insert(*tool, false);
    }

    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let cleaned = strip_ansi(&stdout);
        for line in cleaned.lines() {
            if let Some((status, name)) = parse_tool_line(line) {
                if let Some(tool) = tool_from_display(name) {
                    availability.insert(tool, status);
                }
            }
        }
    }

    let mut tools = Vec::new();
    for (index, tool) in Tool::all().iter().enumerate() {
        tools.push(ToolInfo {
            tool: *tool,
            enabled: true,
            available: *availability.get(tool).unwrap_or(&false),
            priority: (index + 1) as u8,
        });
    }

    Ok(tools)
}

fn parse_tool_line(line: &str) -> Option<(bool, &str)> {
    let trimmed = line.trim();
    let status = if trimmed.starts_with("[OK]") {
        true
    } else if trimmed.starts_with("[--]") {
        false
    } else {
        return None;
    };
    let name = trimmed.trim_start_matches("[OK]")
        .trim_start_matches("[--]")
        .trim();
    Some((status, name))
}

fn tool_from_display(name: &str) -> Option<Tool> {
    let lowered = name.to_lowercase();
    if lowered.contains("claude") {
        Some(Tool::Claude)
    } else if lowered.contains("gemini") {
        Some(Tool::Gemini)
    } else if lowered.contains("codex") {
        Some(Tool::Codex)
    } else if lowered.contains("copilot") {
        Some(Tool::Copilot)
    } else if lowered.contains("perplexity") {
        Some(Tool::Perplexity)
    } else if lowered.contains("cursor") {
        Some(Tool::Cursor)
    } else if lowered.contains("ollama") {
        Some(Tool::Ollama)
    } else {
        None
    }
}

fn strip_ansi(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                while let Some(next) = chars.next() {
                    if next == 'm' {
                        break;
                    }
                }
                continue;
            }
        }
        output.push(ch);
    }

    output
}

fn parse_bridge_control(text: &str) -> Option<BridgeControl> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    if value.get("type").is_none() {
        return None;
    }
    serde_json::from_value(value).ok()
}

async fn handle_bridge_control<S>(
    control: BridgeControl,
    config: &BridgeConfig,
    ws_write: &mut WsWrite<S>,
    started: Instant,
    last_sync: &mut Option<chrono::DateTime<chrono::Utc>>,
    state: Arc<BridgeState>,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    match control {
        BridgeControl::Status => {
            let metrics = state.metrics.get_metrics(state.get_cache_stats());
            let event = BridgeEvent::Status {
                mode: bridge_mode_label(config.mode),
                server: config.server.clone(),
                listen: config.listen.clone(),
                tls_enabled: config.tls_enabled,
                mdns_enabled: config.mdns_enabled,
                drive_remote: config.drive_remote.clone(),
                last_sync: last_sync.map(|t| t.to_rfc3339()),
                uptime_seconds: started.elapsed().as_secs(),
                active_connections: Some(metrics.active_connections),
                total_requests: Some(metrics.total_requests),
            };
            send_bridge_event(ws_write, event).await?;
        }
        BridgeControl::DriveSync { direction } => {
            let result = run_drive_sync(config, direction.as_deref()).await;
            let finished = Utc::now();
            if result.is_ok() {
                *last_sync = Some(finished);
            }
            let event = BridgeEvent::DriveSyncResult {
                ok: result.is_ok(),
                message: result.unwrap_or_else(|e| e.to_string()),
                finished_at: finished.to_rfc3339(),
            };
            send_bridge_event(ws_write, event).await?;
        }
        BridgeControl::DriveStatus => {
            let event = BridgeEvent::DriveStatus {
                configured: config.drive_remote.is_some(),
                remote: config.drive_remote.clone(),
                local_path: config.drive_path.to_string_lossy().to_string(),
                last_sync: last_sync.map(|t| t.to_rfc3339()),
            };
            send_bridge_event(ws_write, event).await?;
        }
        BridgeControl::QrPayload => {
            let payload = build_qr_payload(config)?;
            let event = BridgeEvent::QrPayload { payload };
            send_bridge_event(ws_write, event).await?;
        }
    }

    Ok(())
}

async fn send_bridge_event<S>(ws_write: &mut WsWrite<S>, event: BridgeEvent) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let payload = serde_json::to_string(&event)?;
    ws_write.send(tokio_tungstenite::tungstenite::Message::Text(payload)).await?;
    Ok(())
}

fn validate_rclone_path(path: &str) -> bool {
    // Reject paths with shell injection patterns
    !path.contains('`') &&
    !path.contains("$(") &&
    !path.contains('\0') &&
    !path.contains(';') &&
    !path.contains('|') &&
    !path.contains('&') &&
    !path.contains('\n') &&
    !path.contains('\r')
}

async fn run_drive_sync(config: &BridgeConfig, direction: Option<&str>) -> Result<String> {
    let remote = config.drive_remote.as_ref()
        .ok_or_else(|| anyhow::anyhow!("Drive remote not configured"))?;

    // Validate remote path for security
    if !validate_rclone_path(remote) {
        return Err(anyhow::anyhow!("Invalid characters in drive remote path"));
    }

    let local_path = config.drive_path.to_string_lossy();
    if !validate_rclone_path(&local_path) {
        return Err(anyhow::anyhow!("Invalid characters in local drive path"));
    }

    std::fs::create_dir_all(&config.drive_path)
        .with_context(|| format!("Failed to create {:?}", config.drive_path))?;

    let (source, target) = match direction.unwrap_or("upload") {
        "download" => (remote.as_str(), local_path.as_ref()),
        _ => (local_path.as_ref(), remote.as_str()),
    };

    let output = Command::new("rclone")
        .arg("sync")
        .arg(source)
        .arg(target)
        .output()
        .await
        .context("Failed to run rclone")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow::anyhow!("rclone failed: {}", stderr.trim()));
    }

    Ok("Sync complete".to_string())
}

fn bridge_mode_label(mode: BridgeMode) -> String {
    match mode {
        BridgeMode::Server => "server".to_string(),
        BridgeMode::Local => "local".to_string(),
    }
}

struct QuicBridge {
    _endpoint: Endpoint,
    send: quinn::SendStream,
    recv: quinn::RecvStream,
}

impl QuicBridge {
    async fn connect(config: &BridgeConfig) -> Result<Self> {
        let addr: SocketAddr = config.server.parse()
            .or_else(|_| {
                use std::net::ToSocketAddrs;
                config.server.to_socket_addrs()?.next()
                    .ok_or_else(|| anyhow::anyhow!("Could not resolve address"))
            })?;

        let mut endpoint = Endpoint::client("0.0.0.0:0".parse()?)?;
        let client_config = configure_quic_client(config)?;
        endpoint.set_default_client_config(client_config);

        let connection = endpoint
            .connect(addr, "polyglot-ai")?
            .await
            .context("Failed to connect to server")?;

        let (send, recv) = connection.open_bi().await?;

        Ok(Self {
            _endpoint: endpoint,
            send,
            recv,
        })
    }

    async fn send_message(&mut self, msg: &ClientMessage) -> Result<()> {
        let data = encode_message(msg)?;
        if data.len() > MAX_MESSAGE_SIZE {
            return Err(anyhow::anyhow!("Message too large"));
        }

        let mut buf = Vec::with_capacity(4 + data.len());
        buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
        buf.extend_from_slice(&data);

        self.send.write_all(&buf).await?;
        Ok(())
    }

    async fn recv_message(&mut self) -> Result<ServerMessage> {
        let mut len_buf = [0u8; 4];
        self.recv.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;

        if len > MAX_MESSAGE_SIZE {
            return Err(anyhow::anyhow!("Message too large"));
        }

        let mut buf = vec![0u8; len];
        self.recv.read_exact(&mut buf).await?;

        let msg = decode_message(&buf)?;
        Ok(msg)
    }
}

fn configure_quic_client(config: &BridgeConfig) -> Result<QuinnClientConfig> {
    let cert_pem = std::fs::read(&config.cert)
        .with_context(|| format!("Failed to read client certificate: {:?}", config.cert))?;
    let key_pem = std::fs::read(&config.key)
        .with_context(|| format!("Failed to read client key: {:?}", config.key))?;
    let ca_pem = std::fs::read(&config.ca)
        .with_context(|| format!("Failed to read CA certificate: {:?}", config.ca))?;

    let certs = rustls_pemfile::certs(&mut cert_pem.as_slice())
        .collect::<Result<Vec<_>, _>>()?;
    let key = rustls_pemfile::private_key(&mut key_pem.as_slice())?
        .ok_or_else(|| anyhow::anyhow!("No private key found"))?;

    let mut roots = rustls::RootCertStore::empty();
    for cert in rustls_pemfile::certs(&mut ca_pem.as_slice()) {
        roots.add(cert?)?;
    }

    let tls_config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_client_auth_cert(certs, key)?;

    let mut client_config = QuinnClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)?
    ));

    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(
        Duration::from_secs(config.timeout).try_into()?
    ));
    client_config.transport_config(Arc::new(transport));

    Ok(client_config)
}

fn extract_token(query: Option<&str>) -> Option<String> {
    let query = query?;
    for pair in query.split('&') {
        let mut iter = pair.splitn(2, '=');
        let key = iter.next()?;
        if key == "token" {
            let raw = iter.next().unwrap_or("");
            return Some(percent_decode(raw));
        }
    }
    None
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            // Safely extract hex digits as bytes and convert to string
            let hex_bytes = &bytes[i + 1..i + 3];
            if hex_bytes.iter().all(|b| b.is_ascii_hexdigit()) {
                // Safe to convert since we verified ASCII hex digits
                if let Ok(hex_str) = std::str::from_utf8(hex_bytes) {
                    if let Ok(value) = u8::from_str_radix(hex_str, 16) {
                        output.push(value);
                        i += 3;
                        continue;
                    }
                }
            }
        }
        output.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&output).to_string()
}

impl BridgeConfig {
    fn from_cli(cli: &Cli) -> Self {
        let mode = if cli.mode == "local" {
            BridgeMode::Local
        } else {
            BridgeMode::Server
        };

        Self {
            mode,
            listen: cli.listen.clone(),
            server: cli.server.clone(),
            cert: cli.cert.clone(),
            key: cli.key.clone(),
            ca: cli.ca.clone(),
            token: cli.token.clone(),
            timeout: cli.timeout,
            local_bin: cli.local_bin.clone(),
            tls_enabled: cli.tls,
            tls_cert: cli.tls_cert.clone(),
            tls_key: cli.tls_key.clone(),
            tls_generate: cli.tls_generate,
            mdns_enabled: cli.mdns,
            mdns_name: cli.mdns_name.clone(),
            qr_host: cli.qr_host.clone(),
            drive_remote: cli.drive_remote.clone(),
            drive_path: cli.drive_path.clone().unwrap_or_else(|| PathBuf::from("./bridge-sync")),
            rate_limit: cli.rate_limit,
            max_connections_per_ip: cli.max_connections_per_ip,
            max_prompt_length: cli.max_prompt_length,
            enable_cache: cli.enable_cache,
            cache_ttl: cli.cache_ttl,
            token_expiry_hours: cli.token_expiry_hours,
            auto_failover: cli.auto_failover,
            audit_log: cli.audit_log,
            database_path: cli.database.clone(),
        }
    }
}

fn load_config(path: &Path) -> Result<BridgeConfig> {
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file {:?}", path))?;
    let parsed: BridgeConfigFile = toml::from_str(&raw)
        .with_context(|| "Failed to parse config TOML")?;

    let mut config = BridgeConfig {
        mode: BridgeMode::Server,
        listen: "0.0.0.0:8787".to_string(),
        server: "127.0.0.1:4433".to_string(),
        cert: PathBuf::from("./certs/client.crt"),
        key: PathBuf::from("./certs/client.key"),
        ca: PathBuf::from("./certs/ca.crt"),
        token: None,
        timeout: 30,
        local_bin: "polyglot-local".to_string(),
        tls_enabled: false,
        tls_cert: PathBuf::from("./certs/bridge.crt"),
        tls_key: PathBuf::from("./certs/bridge.key"),
        tls_generate: true,
        mdns_enabled: true,
        mdns_name: "polyglot-bridge".to_string(),
        qr_host: None,
        drive_remote: None,
        drive_path: PathBuf::from("./bridge-sync"),
        rate_limit: 100,
        max_connections_per_ip: 10,
        max_prompt_length: 100000,
        enable_cache: false,
        cache_ttl: 3600,
        token_expiry_hours: 24,
        auto_failover: true,
        audit_log: false,
        database_path: PathBuf::from("./bridge-data/polyglot.db"),
    };

    if let Some(listen) = parsed.listen {
        config.listen = listen;
    }
    if let Some(server) = parsed.server {
        config.server = server;
    }
    if let Some(mode) = parsed.mode {
        config.mode = if mode == "local" { BridgeMode::Local } else { BridgeMode::Server };
    }
    if let Some(local_bin) = parsed.local_bin {
        config.local_bin = local_bin;
    }
    if let Some(cert) = parsed.cert {
        config.cert = cert;
    }
    if let Some(key) = parsed.key {
        config.key = key;
    }
    if let Some(ca) = parsed.ca {
        config.ca = ca;
    }
    if let Some(token) = parsed.token {
        let trimmed = token.trim();
        if !trimmed.is_empty() {
            config.token = Some(trimmed.to_string());
        }
    }
    if let Some(timeout) = parsed.timeout {
        config.timeout = timeout;
    }
    if let Some(tls_enabled) = parsed.tls_enabled {
        config.tls_enabled = tls_enabled;
    }
    if let Some(tls_cert) = parsed.tls_cert {
        config.tls_cert = tls_cert;
    }
    if let Some(tls_key) = parsed.tls_key {
        config.tls_key = tls_key;
    }
    if let Some(tls_generate) = parsed.tls_generate {
        config.tls_generate = tls_generate;
    }
    if let Some(mdns_enabled) = parsed.mdns_enabled {
        config.mdns_enabled = mdns_enabled;
    }
    if let Some(mdns_name) = parsed.mdns_name {
        config.mdns_name = mdns_name;
    }
    if let Some(qr_host) = parsed.qr_host {
        let trimmed = qr_host.trim();
        if !trimmed.is_empty() {
            config.qr_host = Some(trimmed.to_string());
        }
    }
    if let Some(remote) = parsed.drive_remote {
        let trimmed = remote.trim();
        if !trimmed.is_empty() {
            config.drive_remote = Some(trimmed.to_string());
        }
    }
    if let Some(path) = parsed.drive_path {
        config.drive_path = path;
    }

    Ok(config)
}

fn write_example_config(path: &Path) -> Result<()> {
    let content = r#"# Polyglot bridge configuration

listen = "0.0.0.0:8787"
server = "127.0.0.1:4433"
mode = "server" # or "local"
local_bin = "polyglot-local"

cert = "./certs/client.crt"
key = "./certs/client.key"
ca = "./certs/ca.crt"

token = ""
timeout = 30

tls_enabled = false
tls_cert = "./certs/bridge.crt"
tls_key = "./certs/bridge.key"
tls_generate = true

mdns_enabled = true
mdns_name = "polyglot-bridge"
qr_host = ""

drive_remote = "" # Example: "gdrive:polyglot-ai"
drive_path = "./bridge-sync"
"#;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, content)?;
    Ok(())
}

fn build_tls_acceptor(config: &BridgeConfig) -> Result<TlsAcceptor> {
    ensure_tls_files(config)?;

    let cert_pem = std::fs::read(&config.tls_cert)
        .with_context(|| format!("Failed to read TLS cert: {:?}", config.tls_cert))?;
    let key_pem = std::fs::read(&config.tls_key)
        .with_context(|| format!("Failed to read TLS key: {:?}", config.tls_key))?;

    let certs = rustls_pemfile::certs(&mut cert_pem.as_slice())
        .collect::<Result<Vec<_>, _>>()?;
    let key = rustls_pemfile::private_key(&mut key_pem.as_slice())?
        .ok_or_else(|| anyhow::anyhow!("No private key found"))?;

    let mut server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    server_config.alpn_protocols = vec![b"http/1.1".to_vec()];

    Ok(TlsAcceptor::from(Arc::new(server_config)))
}

fn ensure_tls_files(config: &BridgeConfig) -> Result<()> {
    if config.tls_cert.exists() && config.tls_key.exists() {
        return Ok(());
    }

    if !config.tls_generate {
        anyhow::bail!("TLS cert/key missing and auto-generation disabled");
    }

    if let Some(parent) = config.tls_cert.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Some(parent) = config.tls_key.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let mut params = CertificateParams::default();
    params.distinguished_name.push(DnType::CommonName, "polyglot-bridge");
    params.subject_alt_names.push(SanType::DnsName("localhost".to_string()));
    params.subject_alt_names.push(SanType::IpAddress(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST)));

    if let Some(host) = config.qr_host.as_ref() {
        if let Ok(ip) = host.parse::<IpAddr>() {
            params.subject_alt_names.push(SanType::IpAddress(ip));
        } else {
            params.subject_alt_names.push(SanType::DnsName(host.clone()));
        }
    }

    let cert = rcgen::Certificate::from_params(params)?;
    std::fs::write(&config.tls_cert, cert.serialize_pem()?)?;
    std::fs::write(&config.tls_key, cert.serialize_private_key_pem())?;

    Ok(())
}

fn build_qr_payload(config: &BridgeConfig) -> Result<String> {
    let (_, port) = parse_listen(&config.listen)?;
    let host = if let Some(host) = config.qr_host.as_ref() {
        host.clone()
    } else if let Ok((listen_host, _)) = parse_listen(&config.listen) {
        if listen_host == "0.0.0.0" || listen_host == "127.0.0.1" {
            guess_local_ip()
                .map(|ip| ip.to_string())
                .unwrap_or_else(|| "localhost".to_string())
        } else {
            listen_host
        }
    } else {
        "localhost".to_string()
    };

    let scheme = if config.tls_enabled { "wss" } else { "ws" };
    let mut params = vec![
        format!("name={}", url_encode(&config.mdns_name)),
        format!("host={}", url_encode(&host)),
        format!("port={}", port),
        format!("tls={}", config.tls_enabled),
        format!("codec=json"),
        format!("mode=gateway"),
        format!("scheme={}", scheme),
    ];

    if let Some(token) = config.token.as_ref() {
        params.push(format!("token={}", url_encode(token)));
    }

    Ok(format!("polyglot://connect?{}", params.join("&")))
}

fn print_qr_payload(config: &BridgeConfig) -> Result<()> {
    let payload = build_qr_payload(config)?;
    println!("QR payload:\n{}", payload);

    if let Ok(code) = qrcode::QrCode::new(payload.as_bytes()) {
        let rendered = code.render::<qrcode::render::unicode::Dense1x2>().build();
        println!("\n{}\n", rendered);
    }

    Ok(())
}

fn start_mdns(config: &BridgeConfig) -> Result<ServiceDaemon> {
    let daemon = ServiceDaemon::new()
        .context("Failed to create mDNS daemon")?;

    let (host, port) = parse_listen(&config.listen)?;
    let ip = if host == "0.0.0.0" || host == "127.0.0.1" {
        guess_local_ip().unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
    } else {
        host.parse::<IpAddr>().unwrap_or(IpAddr::V4(std::net::Ipv4Addr::LOCALHOST))
    };

    let service_type = "_polyglot-bridge._tcp.local.";
    let instance_name = config.mdns_name.clone();
    let host_name = format!("{}.local.", instance_name);

    let mut properties = HashMap::new();
    properties.insert("tls".to_string(), config.tls_enabled.to_string());
    properties.insert("mode".to_string(), bridge_mode_label(config.mode));
    if config.token.is_some() {
        properties.insert("token".to_string(), "1".to_string());
    }

    let info = ServiceInfo::new(
        service_type,
        &instance_name,
        &host_name,
        ip,
        port,
        properties,
    )?;

    daemon.register(info)?;
    Ok(daemon)
}

fn parse_listen(listen: &str) -> Result<(String, u16)> {
    if let Ok(addr) = listen.parse::<SocketAddr>() {
        return Ok((addr.ip().to_string(), addr.port()));
    }
    if let Some((host, port_str)) = listen.rsplit_once(':') {
        let port = port_str.parse::<u16>()
            .context("Invalid port in listen address")?;
        return Ok((host.to_string(), port));
    }
    Ok((listen.to_string(), 8787))
}

fn guess_local_ip() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    socket.local_addr().ok().map(|addr| addr.ip())
}

fn url_encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "%20".to_string(),
            _ => format!("%{:02X}", b),
        })
        .collect::<String>()
}
