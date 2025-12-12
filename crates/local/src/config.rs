//! Configuration for Polyglot-AI Local

use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use polyglot_common::{Tool, RotationStrategy};
use crate::plugins::PluginConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocalConfig {
    #[serde(default)]
    pub tools: ToolsConfig,

    #[serde(default)]
    pub ui: UiConfig,

    #[serde(default)]
    pub isolation: IsolationConfig,

    #[serde(default)]
    pub plugins: Vec<PluginConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolsConfig {
    #[serde(default = "default_tool")]
    pub default_tool: Tool,

    #[serde(default)]
    pub rotation_strategy: RotationStrategy,

    #[serde(default = "default_switch_delay")]
    pub switch_delay: u8,

    pub claude: Option<ToolConfig>,

    pub gemini: Option<ToolConfig>,

    pub codex: Option<ToolConfig>,

    pub copilot: Option<ToolConfig>,

    pub perplexity: Option<ToolConfig>,

    pub cursor: Option<ToolConfig>,

    pub ollama: Option<ToolConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(default)]
    pub path: String,

    #[serde(default)]
    pub args: Vec<String>,

    #[serde(default)]
    pub env: Vec<(String, String)>,

    #[serde(default)]
    pub use_isolated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsolationConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "default_tools_dir")]
    pub tools_dir: std::path::PathBuf,

    #[serde(default)]
    pub auto_install: bool,
}

fn default_tools_dir() -> std::path::PathBuf {
    directories::BaseDirs::new()
        .map(|d| d.data_dir().join("polyglot").join("tools"))
        .unwrap_or_else(|| std::path::PathBuf::from(".polyglot/tools"))
}

impl Default for IsolationConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            tools_dir: default_tools_dir(),
            auto_install: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiConfig {
    #[serde(default = "default_true")]
    pub tui_enabled: bool,

    #[serde(default = "default_true")]
    pub show_timestamps: bool,

    #[serde(default = "default_theme")]
    pub theme: String,
}

fn default_tool() -> Tool {
    Tool::Claude
}

fn default_switch_delay() -> u8 {
    3
}

fn default_true() -> bool {
    true
}

fn default_theme() -> String {
    "default".to_string()
}

impl Default for LocalConfig {
    fn default() -> Self {
        Self {
            tools: ToolsConfig::default(),
            ui: UiConfig::default(),
            isolation: IsolationConfig::default(),
            plugins: Vec::new(),
        }
    }
}

impl Default for ToolsConfig {
    fn default() -> Self {
        Self {
            default_tool: Tool::Claude,
            rotation_strategy: RotationStrategy::OnLimit,
            switch_delay: 3,
            claude: Some(ToolConfig {
                enabled: true,
                path: "claude".to_string(),
                args: vec![],
                env: vec![],
                use_isolated: false,
            }),
            gemini: Some(ToolConfig {
                enabled: true,
                path: "gemini".to_string(),
                args: vec![],
                env: vec![],
                use_isolated: false,
            }),
            codex: Some(ToolConfig {
                enabled: true,
                path: "codex".to_string(),
                args: vec![],
                env: vec![],
                use_isolated: false,
            }),
            copilot: Some(ToolConfig {
                enabled: true,
                path: "gh".to_string(),
                args: vec!["copilot".to_string()],
                env: vec![],
                use_isolated: false,
            }),
            perplexity: Some(ToolConfig {
                enabled: true,
                path: "pplx".to_string(),
                args: vec![],
                env: vec![],
                use_isolated: false,
            }),
            cursor: Some(ToolConfig {
                enabled: true,
                path: "cursor-agent".to_string(),
                args: vec![],
                env: vec![],
                use_isolated: false,
            }),
            ollama: Some(ToolConfig {
                enabled: true,
                path: "ollama".to_string(),
                args: vec![],
                env: vec![],
                use_isolated: false,
            }),
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            tui_enabled: true,
            show_timestamps: true,
            theme: "default".to_string(),
        }
    }
}

impl LocalConfig {
    pub fn default_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("polyglot-ai")
            .join("local.toml")
    }

    pub fn load(path: &PathBuf) -> anyhow::Result<Self> {
        let content = std::fs::read_to_string(path)?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }

    #[allow(dead_code)]
    pub fn save(&self, path: &PathBuf) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(self)?;
        std::fs::write(path, content)?;
        Ok(())
    }
}

pub fn generate_example_config() -> String {
    r#"# Polyglot-AI Local Configuration

[tools]
# Default tool to use when starting
default_tool = "claude"

# Strategy when rate limited: "on_limit", "round_robin", or "priority"
rotation_strategy = "on_limit"

# Seconds to show notification before auto-switching tools
switch_delay = 3

[tools.claude]
enabled = true
path = "claude"
args = []

[tools.gemini]
enabled = true
path = "gemini"
args = []

[tools.codex]
enabled = true
path = "codex"
args = []

[tools.copilot]
enabled = true
path = "gh"
args = ["copilot"]

[tools.perplexity]
enabled = true
path = "pplx"
args = []
# Cursor Agent CLI - Requires WSL on Windows
# Install: curl https://cursor.com/install -fsS | bash (inside WSL on Windows)
[tools.cursor]
enabled = true
# Windows: runs through WSL
# path = "wsl"
# args = ["cursor-agent"]
# Unix/Mac: runs directly
path = "cursor-agent"
args = []
[ui]
# Enable terminal UI (set to false for simple CLI mode)
tui_enabled = true

# Show timestamps in chat output
show_timestamps = true

# Color theme
theme = "default"

# Custom Plugins
# ===============
# Plugins allow you to add custom AI tools without modifying code.
# Uncomment and modify the examples below to add your own.

# Example: Ollama (local LLM)
# [[plugins]]
# name = "ollama"
# display_name = "Ollama (Local)"
# plugin_type = "cli"
# enabled = true
# priority = 10
# command = "ollama"
# args = ["run", "codellama"]
# prompt_placeholder = "{prompt}"
# timeout = 300

# Example: Custom Python script
# [[plugins]]
# name = "my-ai"
# display_name = "My Custom AI"
# plugin_type = "script"
# enabled = true
# priority = 50
# command = "~/.polyglot-ai/plugins/my_ai.py"
# args = ["--prompt", "{prompt}"]
# interpreter = "python3"
# timeout = 120

# Example: HTTP API
# [[plugins]]
# name = "my-api"
# display_name = "My API"
# plugin_type = "http"
# enabled = true
# priority = 30
# command = "https://api.example.com/chat"
# http_method = "POST"
# headers = { "Authorization" = "Bearer YOUR_TOKEN" }
# body_template = '{"prompt": "{prompt}"}'
# timeout = 60
"#.to_string()
}
