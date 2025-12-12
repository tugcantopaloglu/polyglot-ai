//! Client configuration

#![allow(dead_code)]

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use polyglot_common::SyncMode;
use anyhow::{Context, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    pub connection: ConnectionSettings,
    pub sync: SyncSettings,
    pub ui: UiSettings,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            connection: ConnectionSettings::default(),
            sync: SyncSettings::default(),
            ui: UiSettings::default(),
        }
    }
}

impl ClientConfig {
    pub fn load(path: &PathBuf) -> Result<Self> {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {:?}", path))?;
        let config: Self = toml::from_str(&content)
            .with_context(|| "Failed to parse config file")?;
        Ok(config)
    }

    pub fn save(&self, path: &PathBuf) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)
            .with_context(|| "Failed to serialize config")?;
        std::fs::write(path, content)
            .with_context(|| format!("Failed to write config file: {:?}", path))?;
        Ok(())
    }

    pub fn default_path() -> PathBuf {
        if let Some(config_dir) = directories::ProjectDirs::from("ai", "polyglot", "polyglot-client") {
            config_dir.config_dir().join("client.toml")
        } else {
            PathBuf::from("client.toml")
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionSettings {
    pub server_address: String,
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
    pub ca_path: PathBuf,
    pub timeout: u64,
    pub auto_reconnect: bool,
}

impl Default for ConnectionSettings {
    fn default() -> Self {
        Self {
            server_address: "localhost:4433".to_string(),
            cert_path: PathBuf::from("./certs/client.crt"),
            key_path: PathBuf::from("./certs/client.key"),
            ca_path: PathBuf::from("./certs/ca.crt"),
            timeout: 30,
            auto_reconnect: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSettings {
    pub default_mode: SyncMode,
    pub sync_paths: Vec<PathBuf>,
    pub ignore_patterns: Vec<String>,
    pub debounce_ms: u64,
}

impl Default for SyncSettings {
    fn default() -> Self {
        Self {
            default_mode: SyncMode::OnDemand,
            sync_paths: vec![PathBuf::from(".")],
            ignore_patterns: vec![
                ".git".to_string(),
                "node_modules".to_string(),
                "target".to_string(),
                ".env".to_string(),
                "*.pyc".to_string(),
                "__pycache__".to_string(),
            ],
            debounce_ms: 500,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSettings {
    pub tui_enabled: bool,
    pub theme: String,
    pub show_timestamps: bool,
    pub history_size: usize,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            tui_enabled: true,
            theme: "default".to_string(),
            show_timestamps: true,
            history_size: 1000,
        }
    }
}

pub fn generate_example_config() -> String {
    let config = ClientConfig::default();
    toml::to_string_pretty(&config).expect("Failed to serialize default config")
}
