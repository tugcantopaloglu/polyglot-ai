//! Wire protocol messages for client-server communication

use serde::{Deserialize, Serialize};
use crate::models::*;

pub const PROTOCOL_VERSION: u8 = 1;

pub const MAX_MESSAGE_SIZE: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ClientMessage {
    Handshake {
        version: u8,
        client_id: String,
    },

    Auth {
        cert_fingerprint: String,
    },

    Prompt {
        tool: Option<Tool>,
        message: String,
        working_dir: Option<String>,
    },

    SyncRequest {
        path: String,
        mode: SyncMode,
    },

    FileChunk {
        path: String,
        offset: u64,
        total_size: u64,
        data: Vec<u8>,
        is_last: bool,
    },

    FileRequest {
        path: String,
    },

    ResolveConflict {
        path: String,
        resolution: ConflictResolution,
    },

    Usage,

    SelectTool {
        tool: Tool,
    },

    ListTools,

    Cancel,

    Disconnect,

    Ping {
        timestamp: u64,
    },

    /// Check for server version and updates
    VersionCheck,

    /// Set environment variables for this session (BYOK relay)
    SetEnv {
        entries: Vec<(String, String)>,
    },

    /// Request tool health status
    HealthCheck,

    /// Request usage quota information
    QuotaCheck,

    /// Refresh authentication token
    RefreshToken {
        current_token: String,
    },

    /// Export conversation history
    ExportHistory {
        session_id: Option<String>,
        format: ExportFormat,
    },

    /// Request server metrics (admin only)
    GetMetrics,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ServerMessage {
    HandshakeAck {
        version: u8,
        server_id: String,
    },

    AuthResult {
        success: bool,
        session_id: Option<String>,
        user: Option<User>,
        error: Option<String>,
    },

    ToolResponse {
        tool: Tool,
        content: String,
        done: bool,
        tokens: Option<u64>,
    },

    ToolOutput {
        tool: Tool,
        output_type: OutputType,
        content: String,
    },

    SyncResponse {
        files: Vec<FileInfo>,
        mode: SyncMode,
    },

    FileChunk {
        path: String,
        offset: u64,
        total_size: u64,
        data: Vec<u8>,
        is_last: bool,
    },

    SyncComplete {
        path: String,
        files_synced: u32,
        bytes_transferred: u64,
    },

    ConflictDetected {
        conflict: FileConflict,
    },

    UsageStats {
        stats: Vec<ToolUsage>,
        session_start: chrono::DateTime<chrono::Utc>,
    },

    ToolList {
        tools: Vec<ToolInfo>,
        current: Option<Tool>,
    },

    ToolSwitched {
        from: Tool,
        to: Tool,
        reason: SwitchReason,
    },

    ToolSwitchNotice {
        from: Tool,
        to: Tool,
        reason: SwitchReason,
        countdown: u8,
    },

    Error {
        code: ErrorCode,
        message: String,
    },

    Pong {
        timestamp: u64,
        server_time: u64,
    },

    /// Server version information for update checking
    VersionInfo {
        server_version: String,
        protocol_version: u8,
        min_client_version: Option<String>,
        update_available: bool,
        update_url: Option<String>,
        update_message: Option<String>,
    },

    /// Server is about to shutdown gracefully
    ServerShutdown {
        reason: String,
        /// Seconds until shutdown
        countdown: u32,
    },

    /// Acknowledge environment variable updates
    EnvAck {
        applied: u32,
    },

    /// Tool health status response
    HealthStatus {
        tools: Vec<ToolHealthInfo>,
        server_healthy: bool,
        uptime_seconds: u64,
    },

    /// Usage quota information
    QuotaInfo {
        daily_limit: Option<u64>,
        daily_used: u64,
        monthly_limit: Option<u64>,
        monthly_used: u64,
        reset_at: Option<chrono::DateTime<chrono::Utc>>,
    },

    /// Token refresh result
    TokenRefreshed {
        new_token: String,
        expires_at: chrono::DateTime<chrono::Utc>,
    },

    /// Exported history data
    HistoryExport {
        format: ExportFormat,
        data: String,
        session_count: u32,
    },

    /// Server metrics (admin only)
    Metrics {
        active_connections: u32,
        total_requests: u64,
        requests_per_minute: f64,
        tool_stats: Vec<ToolMetrics>,
        cache_stats: CacheStats,
        uptime_seconds: u64,
    },

    /// Streaming chunk for real-time responses
    StreamChunk {
        tool: Tool,
        content: String,
        sequence: u32,
        is_final: bool,
    },
}

/// Export format for conversation history
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExportFormat {
    Json,
    Markdown,
    Html,
}

/// Tool health information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolHealthInfo {
    pub tool: Tool,
    pub healthy: bool,
    pub last_check: chrono::DateTime<chrono::Utc>,
    pub latency_ms: Option<u32>,
    pub error_rate: f32,
    pub consecutive_failures: u32,
}

/// Per-tool metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolMetrics {
    pub tool: Tool,
    pub total_requests: u64,
    pub successful_requests: u64,
    pub failed_requests: u64,
    pub avg_latency_ms: u32,
    pub rate_limit_hits: u64,
}

/// Cache statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheStats {
    pub entries: u64,
    pub hits: u64,
    pub misses: u64,
    pub hit_rate: f32,
    pub memory_bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputType {
    Stdout,
    Stderr,
    Status,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInfo {
    pub tool: Tool,
    pub enabled: bool,
    pub available: bool,
    pub priority: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SwitchReason {
    RateLimit,
    UserRequest,
    ToolError,
    ToolUnavailable,
}

impl std::fmt::Display for SwitchReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SwitchReason::RateLimit => write!(f, "rate limit reached"),
            SwitchReason::UserRequest => write!(f, "user request"),
            SwitchReason::ToolError => write!(f, "tool error"),
            SwitchReason::ToolUnavailable => write!(f, "tool unavailable"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(u32)]
pub enum ErrorCode {
    Unknown = 0,
    AuthFailed = 1,
    SessionExpired = 2,
    InvalidMessage = 3,
    ToolNotAvailable = 4,
    ToolError = 5,
    SyncError = 6,
    FileNotFound = 7,
    PermissionDenied = 8,
    RateLimited = 9,
    ServerOverloaded = 10,
    ProtocolMismatch = 11,
    PromptTooLong = 12,
    QuotaExceeded = 13,
    TokenExpired = 14,
    ConnectionRateLimited = 15,
}

impl std::fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ErrorCode::Unknown => write!(f, "Unknown error"),
            ErrorCode::AuthFailed => write!(f, "Authentication failed"),
            ErrorCode::SessionExpired => write!(f, "Session expired"),
            ErrorCode::InvalidMessage => write!(f, "Invalid message"),
            ErrorCode::ToolNotAvailable => write!(f, "Tool not available"),
            ErrorCode::ToolError => write!(f, "Tool error"),
            ErrorCode::SyncError => write!(f, "Sync error"),
            ErrorCode::FileNotFound => write!(f, "File not found"),
            ErrorCode::PermissionDenied => write!(f, "Permission denied"),
            ErrorCode::RateLimited => write!(f, "Rate limited"),
            ErrorCode::ServerOverloaded => write!(f, "Server overloaded"),
            ErrorCode::ProtocolMismatch => write!(f, "Protocol version mismatch"),
            ErrorCode::PromptTooLong => write!(f, "Prompt exceeds maximum length"),
            ErrorCode::QuotaExceeded => write!(f, "Usage quota exceeded"),
            ErrorCode::TokenExpired => write!(f, "Authentication token expired"),
            ErrorCode::ConnectionRateLimited => write!(f, "Too many connection attempts"),
        }
    }
}

pub fn encode_message<T: Serialize>(msg: &T) -> Result<Vec<u8>, rmp_serde::encode::Error> {
    rmp_serde::to_vec(msg)
}

pub fn decode_message<'a, T: Deserialize<'a>>(data: &'a [u8]) -> Result<T, rmp_serde::decode::Error> {
    rmp_serde::from_slice(data)
}

pub fn frame_message(data: &[u8]) -> Vec<u8> {
    let len = data.len() as u32;
    let mut framed = Vec::with_capacity(4 + data.len());
    framed.extend_from_slice(&len.to_be_bytes());
    framed.extend_from_slice(data);
    framed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_client_message() {
        let msg = ClientMessage::Prompt {
            tool: Some(Tool::Claude),
            message: "Hello, world!".to_string(),
            working_dir: Some("/home/user/project".to_string()),
        };

        let encoded = encode_message(&msg).unwrap();
        let decoded: ClientMessage = decode_message(&encoded).unwrap();

        match decoded {
            ClientMessage::Prompt { tool, message, .. } => {
                assert_eq!(tool, Some(Tool::Claude));
                assert_eq!(message, "Hello, world!");
            }
            _ => panic!("Wrong message type"),
        }
    }

    #[test]
    fn test_encode_decode_server_message() {
        let msg = ServerMessage::ToolResponse {
            tool: Tool::Gemini,
            content: "Test response".to_string(),
            done: true,
            tokens: Some(42),
        };

        let encoded = encode_message(&msg).unwrap();
        let decoded: ServerMessage = decode_message(&encoded).unwrap();

        match decoded {
            ServerMessage::ToolResponse { tool, content, done, tokens } => {
                assert_eq!(tool, Tool::Gemini);
                assert_eq!(content, "Test response");
                assert!(done);
                assert_eq!(tokens, Some(42));
            }
            _ => panic!("Wrong message type"),
        }
    }
}
