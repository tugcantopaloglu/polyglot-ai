use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use chrono::Utc;
use clap::Parser;
use futures::{SinkExt, StreamExt};
use http::{Request, Response, StatusCode};
use quinn::{ClientConfig as QuinnClientConfig, Endpoint};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::net::TcpListener;
use tokio::process::Command;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use polyglot_common::{
    decode_message, encode_message,
    ClientMessage, ServerMessage,
    ErrorCode, OutputType, Tool, ToolInfo, PROTOCOL_VERSION,
    MAX_MESSAGE_SIZE,
};

#[derive(Parser, Debug)]
#[command(name = "polyglot-bridge")]
#[command(about = "Polyglot-AI WebSocket bridge for mobile clients")]
struct Cli {
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

    /// QUIC idle timeout in seconds
    #[arg(long, default_value_t = 30)]
    timeout: u64,

    #[arg(short, long)]
    verbose: bool,
}

#[derive(Clone)]
struct BridgeConfig {
    mode: BridgeMode,
    server: String,
    cert: PathBuf,
    key: PathBuf,
    ca: PathBuf,
    token: Option<String>,
    timeout: u64,
    local_bin: String,
}

#[derive(Copy, Clone, Debug)]
enum BridgeMode {
    Server,
    Local,
}

#[derive(Copy, Clone)]
enum Codec {
    Json,
    Msgpack,
}

type WsStream = tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>;
type WsWrite = futures::stream::SplitSink<WsStream, tokio_tungstenite::tungstenite::Message>;

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

    let mode = if cli.mode == "local" {
        BridgeMode::Local
    } else {
        BridgeMode::Server
    };

    let config = BridgeConfig {
        mode,
        server: cli.server,
        cert: cli.cert,
        key: cli.key,
        ca: cli.ca,
        token: cli.token,
        timeout: cli.timeout,
        local_bin: cli.local_bin,
    };

    let listener = TcpListener::bind(&cli.listen).await
        .with_context(|| format!("Failed to bind to {}", cli.listen))?;

    info!(
        "Polyglot bridge listening on ws://{}/ws (mode: {:?})",
        cli.listen,
        mode
    );

    loop {
        let (stream, addr) = listener.accept().await?;
        let config = config.clone();

        tokio::spawn(async move {
            if let Err(err) = handle_socket(stream, addr, config).await {
                error!("Connection {} failed: {}", addr, err);
            }
        });
    }
}

async fn handle_socket(stream: tokio::net::TcpStream, addr: SocketAddr, config: BridgeConfig) -> Result<()> {
    let token_required = config.token.clone();

    let ws_stream = tokio_tungstenite::accept_hdr_async(stream, move |req: &Request<()>, mut resp: Response<()>| {
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

    info!("WebSocket client connected: {}", addr);
    let result = match config.mode {
        BridgeMode::Server => handle_server_bridge(ws_stream, &config).await,
        BridgeMode::Local => handle_local_bridge(ws_stream, &config).await,
    };

    info!("WebSocket client disconnected: {}", addr);
    result
}

async fn handle_server_bridge(ws_stream: WsStream, config: &BridgeConfig) -> Result<()> {
    let mut quic = QuicBridge::connect(config).await?;
    let (mut ws_write, mut ws_read) = ws_stream.split();

    let mut codec = Codec::Json;
    let mut codec_set = false;

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
                        let client_msg: ClientMessage = serde_json::from_str(&text)
                            .with_context(|| "Failed to parse JSON message")?;
                        quic.send_message(&client_msg).await?;
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

async fn handle_local_bridge(ws_stream: WsStream, config: &BridgeConfig) -> Result<()> {
    let (mut ws_write, mut ws_read) = ws_stream.split();
    let mut codec = Codec::Json;
    let mut codec_set = false;
    let mut current_tool: Option<Tool> = None;

    while let Some(ws_msg) = ws_read.next().await {
        let ws_msg = ws_msg?;
        let client_msg = match ws_msg {
            tokio_tungstenite::tungstenite::Message::Text(text) => {
                if !codec_set {
                    codec = Codec::Json;
                    codec_set = true;
                }
                Some(serde_json::from_str::<ClientMessage>(&text)?)
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
                let selected_tool = tool.or(current_tool).unwrap_or(Tool::Claude);
                let use_tool_flag = tool.is_some() || current_tool.is_some();
                current_tool = Some(selected_tool);
                handle_prompt_local(
                    &mut ws_write,
                    codec,
                    config,
                    selected_tool,
                    &message,
                    use_tool_flag,
                    working_dir.as_deref(),
                ).await?;
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

async fn send_ws_message(ws_write: &mut WsWrite, codec: Codec, message: &ServerMessage) -> Result<()> {
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

async fn handle_prompt_local(
    ws_write: &mut WsWrite,
    codec: Codec,
    config: &BridgeConfig,
    tool: Tool,
    prompt: &str,
    use_tool_flag: bool,
    working_dir: Option<&str>,
) -> Result<()> {
    let mut cmd = Command::new(&config.local_bin);
    cmd.arg("--no-tui")
        .arg("ask")
        .arg(prompt)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    if let Some(path) = working_dir {
        cmd.arg("--project").arg(path);
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
            if let Ok(value) = u8::from_str_radix(&input[i + 1..i + 3], 16) {
                output.push(value);
                i += 3;
                continue;
            }
        }
        output.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&output).to_string()
}
