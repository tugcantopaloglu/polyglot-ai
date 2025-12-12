//! Server configuration

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use polyglot_common::{AuthMode, RotationStrategy, Tool};
use anyhow::{Context, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub server: ServerSettings,
    pub auth: AuthSettings,
    pub tools: ToolsSettings,
    pub storage: StorageSettings,
    #[serde(default)]
    pub updates: UpdateSettings,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            server: ServerSettings::default(),
            auth: AuthSettings::default(),
            tools: ToolsSettings::default(),
            storage: StorageSettings::default(),
            updates: UpdateSettings::default(),
        }
    }
}

impl ServerConfig {
    pub fn load(path: &PathBuf) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {:?}", path))?;
        let config: Self = toml::from_str(&content)
            .with_context(|| "Failed to parse config file")?;
        Ok(config)
    }

    pub fn save(&self, path: &PathBuf) -> Result<()> {
        let content = toml::to_string_pretty(self)
            .with_context(|| "Failed to serialize config")?;
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write config file: {:?}", path))?;
        Ok(())
    }

    pub fn default_path() -> PathBuf {
        if let Some(config_dir) = directories::ProjectDirs::from("ai", "polyglot", "polyglot-server") {
            config_dir.config_dir().join("server.toml")
        } else {
            PathBuf::from("server.toml")
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerSettings {
    pub bind_address: String,
    pub max_connections: u32,
    pub idle_timeout: u64,
    pub verbose: bool,
}

impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            bind_address: "0.0.0.0:4433".to_string(),
            max_connections: 100,
            idle_timeout: 300,
            verbose: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthSettings {
    pub mode: AuthMode,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub ca_path: PathBuf,
    pub jwt_secret: Option<String>,
    pub session_expiry_hours: u32,
}

impl Default for AuthSettings {
    fn default() -> Self {
        Self {
            mode: AuthMode::SingleUser,
            cert_path: PathBuf::from("./certs/server.crt"),
            key_path: PathBuf::from("./certs/server.key"),
            ca_path: PathBuf::from("./certs/ca.crt"),
            jwt_secret: None,
            session_expiry_hours: 24,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsSettings {
    pub rotation_strategy: RotationStrategy,
    pub default_tool: Tool,
    pub switch_delay: u8,
    pub claude: Option<ToolInstanceConfig>,
    pub gemini: Option<ToolInstanceConfig>,
    pub codex: Option<ToolInstanceConfig>,
    pub copilot: Option<ToolInstanceConfig>,
    pub cursor: Option<ToolInstanceConfig>,
    pub ollama: Option<ToolInstanceConfig>,
}

impl Default for ToolsSettings {
    fn default() -> Self {
        Self {
            rotation_strategy: RotationStrategy::OnLimit,
            default_tool: Tool::Claude,
            switch_delay: 3,
            claude: Some(ToolInstanceConfig::default_claude()),
            gemini: Some(ToolInstanceConfig::default_gemini()),
            codex: Some(ToolInstanceConfig::default_codex()),
            copilot: Some(ToolInstanceConfig::default_copilot()),
            cursor: Some(ToolInstanceConfig::default_cursor()),
            ollama: Some(ToolInstanceConfig::default_ollama()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolInstanceConfig {
    pub enabled: bool,
    pub path: String,
    pub priority: u8,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Vec<(String, String)>,
}

impl ToolInstanceConfig {
    pub fn default_claude() -> Self {
        Self {
            enabled: true,
            path: "claude".to_string(),
            priority: 1,
            args: vec![],
            env: vec![],
        }
    }

    pub fn default_gemini() -> Self {
        Self {
            enabled: true,
            path: "gemini".to_string(),
            priority: 2,
            args: vec![],
            env: vec![],
        }
    }

    pub fn default_codex() -> Self {
        Self {
            enabled: true,
            path: "codex".to_string(),
            priority: 3,
            args: vec![],
            env: vec![],
        }
    }

    pub fn default_copilot() -> Self {
        Self {
            enabled: true,
            path: "gh".to_string(),
            priority: 4,
            args: vec!["copilot".to_string()],
            env: vec![],
        }
    }

    pub fn default_cursor() -> Self {
        #[cfg(windows)]
        {
            Self {
                enabled: true,
                path: "wsl".to_string(),
                priority: 5,
                args: vec!["cursor-agent".to_string()],
                env: vec![],
            }
        }
        #[cfg(not(windows))]
        {
            Self {
                enabled: true,
                path: "cursor-agent".to_string(),
                priority: 5,
                args: vec![],
                env: vec![],
            }
        }
    }

    pub fn default_ollama() -> Self {
        Self {
            enabled: true,
            path: "ollama".to_string(),
            priority: 7,
            args: vec!["run".to_string(), "codellama".to_string()],
            env: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageSettings {
    pub db_path: PathBuf,
    pub api_keys_path: PathBuf,
    pub sync_dir: PathBuf,
}

impl Default for StorageSettings {
    fn default() -> Self {
        Self {
            db_path: PathBuf::from("./data/polyglot.db"),
            api_keys_path: PathBuf::from("./data/keys.enc"),
            sync_dir: PathBuf::from("./data/sync"),
        }
    }
}

/// Settings for seamless updates and graceful shutdown
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateSettings {
    /// Enable automatic update checking
    pub check_updates: bool,
    /// URL to check for updates (GitHub releases API)
    pub update_check_url: String,
    /// Minimum required client version (clients below this get update warning)
    pub min_client_version: Option<String>,
    /// Graceful shutdown timeout in seconds (drain connections before stopping)
    pub graceful_shutdown_timeout: u32,
    /// Message to show clients when update is available
    pub update_message: Option<String>,
    /// Download URL for client updates
    pub client_download_url: Option<String>,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            check_updates: true,
            update_check_url: "https://api.github.com/repos/tugcantopaloglu/selfhosted-ai-code-platform/releases/latest".to_string(),
            min_client_version: None,
            graceful_shutdown_timeout: 30,
            update_message: None,
            client_download_url: Some("https://github.com/tugcantopaloglu/selfhosted-ai-code-platform/releases".to_string()),
        }
    }
}

pub fn generate_example_config() -> String {
    let config = ServerConfig::default();
    toml::to_string_pretty(&config).expect("Failed to serialize default config")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config_serialization() {
        let config = ServerConfig::default();
        let serialized = toml::to_string_pretty(&config).unwrap();
        let deserialized: ServerConfig = toml::from_str(&serialized).unwrap();
        assert_eq!(config.server.bind_address, deserialized.server.bind_address);
    }
}
