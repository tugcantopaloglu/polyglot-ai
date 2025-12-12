//! Domain models for Polyglot-AI

use serde::{Deserialize, Serialize};
use uuid::Uuid;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Claude,
    Gemini,
    Codex,
    Copilot,
    Perplexity,
    Cursor,
    Ollama,
}

impl Tool {
    pub fn as_str(&self) -> &'static str {
        match self {
            Tool::Claude => "claude",
            Tool::Gemini => "gemini",
            Tool::Codex => "codex",
            Tool::Copilot => "copilot",
            Tool::Perplexity => "perplexity",
            Tool::Cursor => "cursor",
            Tool::Ollama => "ollama",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Tool::Claude => "Claude Code",
            Tool::Gemini => "Gemini CLI",
            Tool::Codex => "Codex CLI",
            Tool::Copilot => "GitHub Copilot CLI",
            Tool::Perplexity => "Perplexity AI",
            Tool::Cursor => "Cursor CLI",
            Tool::Ollama => "Ollama",
        }
    }

    pub fn all() -> &'static [Tool] {
        &[Tool::Claude, Tool::Gemini, Tool::Codex, Tool::Copilot, Tool::Perplexity, Tool::Cursor, Tool::Ollama]
    }
}

impl std::fmt::Display for Tool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
}

impl std::str::FromStr for Tool {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "claude" | "claude-code" => Ok(Tool::Claude),
            "gemini" | "gemini-cli" => Ok(Tool::Gemini),
            "codex" | "codex-cli" => Ok(Tool::Codex),
            "copilot" | "github-copilot" => Ok(Tool::Copilot),
            "perplexity" | "pplx" => Ok(Tool::Perplexity),
            "cursor" | "cursor-cli" => Ok(Tool::Cursor),
            "ollama" | "ollama-local" => Ok(Tool::Ollama),
            _ => Err(format!("Unknown tool: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncMode {
    OnDemand,
    Realtime,
}

impl Default for SyncMode {
    fn default() -> Self {
        SyncMode::OnDemand
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RotationStrategy {
    OnLimit,
    RoundRobin,
    Priority,
}

impl Default for RotationStrategy {
    fn default() -> Self {
        RotationStrategy::OnLimit
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    SingleUser,
    MultiUser,
}

impl Default for AuthMode {
    fn default() -> Self {
        AuthMode::SingleUser
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: Uuid,
    pub username: String,
    pub created_at: DateTime<Utc>,
    pub last_login: Option<DateTime<Utc>>,
    pub is_admin: bool,
}

impl User {
    pub fn new(username: String, is_admin: bool) -> Self {
        Self {
            id: Uuid::new_v4(),
            username,
            created_at: Utc::now(),
            last_login: None,
            is_admin,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub user_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub current_tool: Option<Tool>,
    pub sync_mode: SyncMode,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolUsage {
    pub tool: Tool,
    pub requests: u64,
    pub tokens_used: u64,
    pub errors: u64,
    pub rate_limit_hits: u64,
    pub last_used: Option<DateTime<Utc>>,
    pub is_available: bool,
}

impl ToolUsage {
    pub fn new(tool: Tool) -> Self {
        Self {
            tool,
            requests: 0,
            tokens_used: 0,
            errors: 0,
            rate_limit_hits: 0,
            last_used: None,
            is_available: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileInfo {
    pub path: String,
    pub size: u64,
    pub hash: String,
    pub modified_at: DateTime<Utc>,
    pub is_directory: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileConflict {
    pub path: String,
    pub local_hash: String,
    pub remote_hash: String,
    pub local_modified: DateTime<Utc>,
    pub remote_modified: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConflictResolution {
    KeepLocal,
    KeepRemote,
    KeepBoth,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolConfig {
    pub tool: Tool,
    pub enabled: bool,
    pub path: String,
    pub priority: u8,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

impl ToolConfig {
    pub fn default_for(tool: Tool) -> Self {
        let (path, priority) = match tool {
            Tool::Claude => ("claude", 1),
            Tool::Gemini => ("gemini", 2),
            Tool::Codex => ("codex", 3),
            Tool::Copilot => ("gh copilot", 4),
            Tool::Perplexity => ("pplx", 5),
            Tool::Cursor => ("cursor-agent", 6),
            Tool::Ollama => ("ollama", 7),
        };

        Self {
            tool,
            enabled: true,
            path: path.to_string(),
            priority,
            args: Vec::new(),
            env: Vec::new(),
        }
    }
}
