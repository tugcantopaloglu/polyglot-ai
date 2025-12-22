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
