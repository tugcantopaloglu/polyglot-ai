//! Client connection handling

#![allow(dead_code)]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use quinn::{ClientConfig as QuinnClientConfig, Endpoint, Connection};
use tokio::sync::mpsc;
use tracing::{info, error};

use polyglot_common::{
    ClientMessage, ServerMessage, Tool, SyncMode, ExportFormat,
    encode_message, decode_message, PROTOCOL_VERSION, MAX_MESSAGE_SIZE,
};

use crate::config::ConnectionSettings;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Connected,
    Authenticated,
    Error,
}

pub struct ClientConnection {
    endpoint: Endpoint,
    connection: Option<Connection>,
    send_stream: Option<quinn::SendStream>,
    recv_stream: Option<quinn::RecvStream>,
    state: ConnectionState,
    session_id: Option<String>,
    cert_fingerprint: String,
}

impl ClientConnection {
    pub async fn new(settings: &ConnectionSettings) -> Result<Self> {
        let _client_config = configure_quic_client(settings)?;
        let endpoint = Endpoint::client("0.0.0.0:0".parse()?)?;

        let cert_pem = std::fs::read(&settings.cert_path)?;
        let certs = rustls_pemfile::certs(&mut cert_pem.as_slice())
            .collect::<Result<Vec<_>, _>>()?;
        let fingerprint = if !certs.is_empty() {
            polyglot_common::crypto::sha256_hex(&certs[0])
        } else {
            String::new()
        };

        Ok(Self {
            endpoint,
            connection: None,
            send_stream: None,
            recv_stream: None,
            state: ConnectionState::Disconnected,
            session_id: None,
            cert_fingerprint: fingerprint,
        })
    }

    pub async fn connect(&mut self, settings: &ConnectionSettings) -> Result<()> {
        self.state = ConnectionState::Connecting;

        let addr: SocketAddr = settings.server_address.parse()
            .or_else(|_| {
                use std::net::ToSocketAddrs;
                settings.server_address.to_socket_addrs()?.next()
                    .ok_or_else(|| anyhow::anyhow!("Could not resolve address"))
            })?;

        let client_config = configure_quic_client(settings)?;
        self.endpoint.set_default_client_config(client_config);

        let connection = self.endpoint
            .connect(addr, "polyglot-ai")?
            .await
            .context("Failed to connect to server")?;

        info!("Connected to {}", addr);

        let (send, recv) = connection.open_bi().await?;

        self.connection = Some(connection);
        self.send_stream = Some(send);
        self.recv_stream = Some(recv);
        self.state = ConnectionState::Connected;

        self.handshake().await?;

        self.authenticate().await?;

        Ok(())
    }

    async fn handshake(&mut self) -> Result<()> {
        let msg = ClientMessage::Handshake {
            version: PROTOCOL_VERSION,
            client_id: format!("polyglot-client-{}", env!("CARGO_PKG_VERSION")),
        };

        self.send_message(&msg).await?;

        match self.recv_message().await? {
            ServerMessage::HandshakeAck { version, server_id } => {
                info!("Handshake successful. Server: {}", server_id);
                if version != PROTOCOL_VERSION {
                    return Err(anyhow::anyhow!(
                        "Protocol version mismatch. Server: {}, Client: {}",
                        version, PROTOCOL_VERSION
                    ));
                }
                Ok(())
            }
            ServerMessage::Error { code, message } => {
                Err(anyhow::anyhow!("Handshake failed: {} - {}", code, message))
            }
            _ => Err(anyhow::anyhow!("Unexpected response to handshake")),
        }
    }

    async fn authenticate(&mut self) -> Result<()> {
        let msg = ClientMessage::Auth {
            cert_fingerprint: self.cert_fingerprint.clone(),
        };

        self.send_message(&msg).await?;

        match self.recv_message().await? {
            ServerMessage::AuthResult { success, session_id, user, error } => {
                if success {
                    self.session_id = session_id;
                    self.state = ConnectionState::Authenticated;
                    if let Some(user) = user {
                        info!("Authenticated as: {}", user.username);
                    }
                    Ok(())
                } else {
                    self.state = ConnectionState::Error;
                    Err(anyhow::anyhow!("Authentication failed: {}", error.unwrap_or_default()))
                }
            }
            _ => Err(anyhow::anyhow!("Unexpected response to auth")),
        }
    }

    pub async fn send_message(&mut self, msg: &ClientMessage) -> Result<()> {
        let send = self.send_stream.as_mut()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;

        let data = encode_message(msg)?;
        if data.len() > MAX_MESSAGE_SIZE {
            return Err(anyhow::anyhow!("Message too large"));
        }

        let mut buf = Vec::with_capacity(4 + data.len());
        buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
        buf.extend_from_slice(&data);

        send.write_all(&buf).await?;
        Ok(())
    }

    pub async fn recv_message(&mut self) -> Result<ServerMessage> {
        let recv = self.recv_stream.as_mut()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;

        let mut len_buf = [0u8; 4];
        recv.read_exact(&mut len_buf).await?;
        let len = u32::from_be_bytes(len_buf) as usize;

        if len > MAX_MESSAGE_SIZE {
            return Err(anyhow::anyhow!("Message too large"));
        }

        let mut buf = vec![0u8; len];
        recv.read_exact(&mut buf).await?;

        let msg = decode_message(&buf)?;
        Ok(msg)
    }

    pub async fn prompt(&mut self, message: &str, tool: Option<Tool>) -> Result<ServerMessage> {
        let msg = ClientMessage::Prompt {
            tool,
            message: message.to_string(),
            working_dir: std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
        };

        self.send_message(&msg).await?;
        self.recv_message().await
    }

    pub async fn prompt_streaming(
        &mut self,
        message: &str,
        tool: Option<Tool>,
        response_tx: mpsc::Sender<ServerMessage>,
    ) -> Result<()> {
        let msg = ClientMessage::Prompt {
            tool,
            message: message.to_string(),
            working_dir: std::env::current_dir().ok().map(|p| p.to_string_lossy().to_string()),
        };

        self.send_message(&msg).await?;

        loop {
            let response = self.recv_message().await?;
            let is_done = matches!(&response,
                ServerMessage::ToolResponse { done: true, .. } |
                ServerMessage::Error { .. }
            );

            response_tx.send(response).await
                .map_err(|_| anyhow::anyhow!("Response channel closed"))?;

            if is_done {
                break;
            }
        }

        Ok(())
    }

    pub async fn try_recv_message(&mut self) -> Result<Option<ServerMessage>> {
        let recv = self.recv_stream.as_mut()
            .ok_or_else(|| anyhow::anyhow!("Not connected"))?;

        let mut len_buf = [0u8; 4];
        match tokio::time::timeout(Duration::from_millis(10), recv.read_exact(&mut len_buf)).await {
            Ok(Ok(_)) => {
                let len = u32::from_be_bytes(len_buf) as usize;
                if len > MAX_MESSAGE_SIZE {
                    return Err(anyhow::anyhow!("Message too large"));
                }

                let mut buf = vec![0u8; len];
                recv.read_exact(&mut buf).await?;

                let msg = decode_message(&buf)?;
                Ok(Some(msg))
            }
            Ok(Err(e)) => Err(e.into()),
            Err(_) => Ok(None),
        }
    }

    pub async fn usage(&mut self) -> Result<ServerMessage> {
        self.send_message(&ClientMessage::Usage).await?;
        self.recv_message().await
    }

    pub async fn list_tools(&mut self) -> Result<ServerMessage> {
        self.send_message(&ClientMessage::ListTools).await?;
        self.recv_message().await
    }

    pub async fn select_tool(&mut self, tool: Tool) -> Result<ServerMessage> {
        self.send_message(&ClientMessage::SelectTool { tool }).await?;
        self.recv_message().await
    }

    pub async fn sync(&mut self, path: &str, mode: SyncMode) -> Result<ServerMessage> {
        self.send_message(&ClientMessage::SyncRequest {
            path: path.to_string(),
            mode,
        }).await?;
        self.recv_message().await
    }

    pub async fn ping(&mut self) -> Result<Duration> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_millis() as u64;

        self.send_message(&ClientMessage::Ping { timestamp }).await?;

        let start = std::time::Instant::now();
        match self.recv_message().await? {
            ServerMessage::Pong { timestamp: _, server_time: _ } => {
                Ok(start.elapsed())
            }
            _ => Err(anyhow::anyhow!("Unexpected response to ping")),
        }
    }

    /// Check server version and update availability
    pub async fn check_version(&mut self) -> Result<ServerMessage> {
        self.send_message(&ClientMessage::VersionCheck).await?;
        self.recv_message().await
    }

    /// Check health status of all tools
    pub async fn health_check(&mut self) -> Result<ServerMessage> {
        self.send_message(&ClientMessage::HealthCheck).await?;
        self.recv_message().await
    }

    /// Check quota status for current user
    pub async fn quota_check(&mut self) -> Result<ServerMessage> {
        self.send_message(&ClientMessage::QuotaCheck).await?;
        self.recv_message().await
    }

    /// Get server metrics
    pub async fn get_metrics(&mut self) -> Result<ServerMessage> {
        self.send_message(&ClientMessage::GetMetrics).await?;
        self.recv_message().await
    }

    /// Export chat history in specified format
    pub async fn export_history(&mut self, format: ExportFormat) -> Result<ServerMessage> {
        self.send_message(&ClientMessage::ExportHistory { format }).await?;
        self.recv_message().await
    }

    /// Refresh authentication token
    pub async fn refresh_token(&mut self) -> Result<ServerMessage> {
        self.send_message(&ClientMessage::RefreshToken).await?;
        self.recv_message().await
    }

    pub async fn disconnect(&mut self) -> Result<()> {
        if self.state == ConnectionState::Authenticated || self.state == ConnectionState::Connected {
            let _ = self.send_message(&ClientMessage::Disconnect).await;
        }

        self.send_stream = None;
        self.recv_stream = None;

        if let Some(conn) = self.connection.take() {
            conn.close(0u32.into(), b"disconnect");
        }

        self.state = ConnectionState::Disconnected;
        self.session_id = None;

        info!("Disconnected from server");
        Ok(())
    }

    pub fn state(&self) -> ConnectionState {
        self.state
    }

    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn is_ready(&self) -> bool {
        self.state == ConnectionState::Authenticated
    }

    pub async fn reconnect(&mut self, settings: &ConnectionSettings, max_attempts: u32) -> Result<()> {
        let mut attempt = 0;
        let mut delay = Duration::from_secs(1);
        let max_delay = Duration::from_secs(60);

        while attempt < max_attempts {
            attempt += 1;
            info!("Reconnection attempt {}/{}", attempt, max_attempts);

            self.send_stream = None;
            self.recv_stream = None;
            if let Some(conn) = self.connection.take() {
                conn.close(0u32.into(), b"reconnecting");
            }
            self.state = ConnectionState::Disconnected;

            match self.connect(settings).await {
                Ok(_) => {
                    info!("Reconnected successfully");
                    return Ok(());
                }
                Err(e) => {
                    error!("Reconnection attempt {} failed: {}", attempt, e);
                    if attempt < max_attempts {
                        info!("Retrying in {:?}...", delay);
                        tokio::time::sleep(delay).await;
                        delay = std::cmp::min(delay * 2, max_delay);
                    }
                }
            }
        }

        Err(anyhow::anyhow!("Failed to reconnect after {} attempts", max_attempts))
    }

    pub async fn with_reconnect<F, Fut, T>(
        &mut self,
        settings: &ConnectionSettings,
        operation: F,
    ) -> Result<T>
    where
        F: Fn(&mut Self) -> Fut,
        Fut: std::future::Future<Output = Result<T>>,
    {
        match operation(self).await {
            Ok(result) => Ok(result),
            Err(e) => {
                if !self.is_ready() && settings.auto_reconnect {
                    info!("Connection lost, attempting to reconnect...");
                    self.reconnect(settings, 3).await?;
                    operation(self).await
                } else {
                    Err(e)
                }
            }
        }
    }

    pub async fn prompt_with_reconnect(
        &mut self,
        settings: &ConnectionSettings,
        message: &str,
        tool: Option<Tool>,
    ) -> Result<ServerMessage> {
        if !self.is_ready() && settings.auto_reconnect {
            self.reconnect(settings, 3).await?;
        }

        match self.prompt(message, tool).await {
            Ok(response) => Ok(response),
            Err(e) => {
                if settings.auto_reconnect {
                    info!("Request failed, attempting reconnect: {}", e);
                    self.reconnect(settings, 3).await?;
                    self.prompt(message, tool).await
                } else {
                    Err(e)
                }
            }
        }
    }
}

fn configure_quic_client(settings: &ConnectionSettings) -> Result<QuinnClientConfig> {
    let cert_pem = std::fs::read(&settings.cert_path)
        .with_context(|| format!("Failed to read client certificate: {:?}", settings.cert_path))?;
    let key_pem = std::fs::read(&settings.key_path)
        .with_context(|| format!("Failed to read client key: {:?}", settings.key_path))?;
    let ca_pem = std::fs::read(&settings.ca_path)
        .with_context(|| format!("Failed to read CA certificate: {:?}", settings.ca_path))?;

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
        Duration::from_secs(settings.timeout).try_into()?
    ));
    client_config.transport_config(Arc::new(transport));

    Ok(client_config)
}
