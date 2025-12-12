use std::path::PathBuf;
use anyhow::{Result, Context};
use polyglot_common::Tool;

#[allow(dead_code)]
pub struct EnvironmentManager {
    tools_dir: PathBuf,
    auto_install: bool,
}

#[allow(dead_code)]
impl EnvironmentManager {
    pub fn new(tools_dir: PathBuf, auto_install: bool) -> Self {
        Self { tools_dir, auto_install }
    }

    pub fn get_tool_path(&self, tool: Tool) -> PathBuf {
        let tool_name = match tool {
            Tool::Claude => "claude",
            Tool::Gemini => "gemini",
            Tool::Codex => "codex",
            Tool::Copilot => "gh",
            Tool::Perplexity => "pplx",
            Tool::Cursor => "cursor-agent",
            Tool::Ollama => "ollama",
        };

        #[cfg(windows)]
        let exe_name = format!("{}.exe", tool_name);
        #[cfg(not(windows))]
        let exe_name = tool_name.to_string();

        self.tools_dir.join(tool.as_str()).join("bin").join(exe_name)
    }

    pub fn is_installed(&self, tool: Tool) -> bool {
        self.get_tool_path(tool).exists()
    }

    pub fn get_tool_dir(&self, tool: Tool) -> PathBuf {
        self.tools_dir.join(tool.as_str())
    }

    pub fn init(&self) -> Result<()> {
        std::fs::create_dir_all(&self.tools_dir)
            .context("Failed to create tools directory")?;
        Ok(())
    }

    pub fn get_install_command(&self, tool: Tool) -> Option<String> {
        match tool {
            Tool::Claude => Some("npm install -g @anthropic-ai/claude-code".to_string()),
            Tool::Gemini => Some("pip install google-generativeai".to_string()),
            Tool::Codex => Some("pip install openai".to_string()),
            Tool::Copilot => Some("gh extension install github/gh-copilot".to_string()),
            Tool::Perplexity => None,
            Tool::Cursor => Some("curl https://cursor.com/install -fsS | bash".to_string()),
            Tool::Ollama => Some(Self::ollama_install_command()),
        }
    }

    fn ollama_install_command() -> String {
        #[cfg(target_os = "macos")]
        return "brew install ollama".to_string();

        #[cfg(target_os = "linux")]
        return "curl -fsSL https://ollama.ai/install.sh | sh".to_string();

        #[cfg(target_os = "windows")]
        return "winget install Ollama.Ollama".to_string();
    }

    pub fn is_in_system_path(&self, tool: Tool) -> bool {
        let tool_name = match tool {
            Tool::Claude => "claude",
            Tool::Gemini => "gemini",
            Tool::Codex => "codex",
            Tool::Copilot => "gh",
            Tool::Perplexity => "pplx",
            Tool::Cursor => "cursor-agent",
            Tool::Ollama => "ollama",
        };

        which::which(tool_name).is_ok()
    }

    pub fn resolve_tool_path(&self, tool: Tool, use_isolated: bool) -> String {
        if use_isolated && self.is_installed(tool) {
            self.get_tool_path(tool).to_string_lossy().to_string()
        } else {
            match tool {
                Tool::Claude => "claude".to_string(),
                Tool::Gemini => "gemini".to_string(),
                Tool::Codex => "codex".to_string(),
                Tool::Copilot => "gh".to_string(),
                Tool::Perplexity => "pplx".to_string(),
                Tool::Cursor => "cursor-agent".to_string(),
                Tool::Ollama => "ollama".to_string(),
            }
        }
    }

    pub fn list_installed(&self) -> Vec<(Tool, PathBuf)> {
        Tool::all()
            .iter()
            .filter(|t| self.is_installed(**t))
            .map(|t| (*t, self.get_tool_path(*t)))
            .collect()
    }

    pub fn status(&self) -> Vec<ToolStatus> {
        Tool::all()
            .iter()
            .map(|tool| ToolStatus {
                tool: *tool,
                isolated_installed: self.is_installed(*tool),
                system_available: self.is_in_system_path(*tool),
                isolated_path: if self.is_installed(*tool) {
                    Some(self.get_tool_path(*tool))
                } else {
                    None
                },
            })
            .collect()
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct ToolStatus {
    pub tool: Tool,
    pub isolated_installed: bool,
    pub system_available: bool,
    pub isolated_path: Option<PathBuf>,
}

#[allow(dead_code)]
impl ToolStatus {
    pub fn display(&self) -> String {
        let isolated = if self.isolated_installed { "✓" } else { "✗" };
        let system = if self.system_available { "✓" } else { "✗" };
        format!(
            "{:<15} Isolated: {}  System: {}",
            self.tool.display_name(),
            isolated,
            system
        )
    }
}
