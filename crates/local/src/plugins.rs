#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use tokio::process::Command;
use tokio::io::{BufReader, AsyncBufReadExt};
use tokio::sync::mpsc;
use chrono::Utc;

use crate::tools::ToolOutput;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum PluginType {
    Cli,
    Http,
    Script,
}

impl Default for PluginType {
    fn default() -> Self {
        PluginType::Cli
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
}

impl Default for HttpMethod {
    fn default() -> Self {
        HttpMethod::Post
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginConfig {
    pub name: String,

    #[serde(default)]
    pub display_name: Option<String>,

    #[serde(default)]
    pub plugin_type: PluginType,

    #[serde(default = "default_true")]
    pub enabled: bool,

    #[serde(default = "default_priority")]
    pub priority: u8,

    pub command: String,

    #[serde(default)]
    pub args: Vec<String>,

    #[serde(default = "default_prompt_placeholder")]
    pub prompt_placeholder: String,

    #[serde(default)]
    pub env: HashMap<String, String>,

    #[serde(default)]
    pub http_method: HttpMethod,

    #[serde(default)]
    pub headers: HashMap<String, String>,

    #[serde(default)]
    pub body_template: Option<String>,

    #[serde(default)]
    pub response_path: Option<String>,

    #[serde(default)]
    pub interpreter: Option<String>,

    #[serde(default)]
    pub working_dir: Option<PathBuf>,

    #[serde(default = "default_timeout")]
    pub timeout: u64,
}

fn default_true() -> bool { true }
fn default_priority() -> u8 { 50 }
fn default_prompt_placeholder() -> String { "{prompt}".to_string() }
fn default_timeout() -> u64 { 120 }

impl PluginConfig {
    pub fn display_name(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.name)
    }
}

#[derive(Debug, Clone)]
pub struct PluginUsage {
    pub name: String,
    pub requests: u64,
    pub errors: u64,
    pub last_used: Option<chrono::DateTime<chrono::Utc>>,
    pub is_available: bool,
}

impl PluginUsage {
    pub fn new(name: String) -> Self {
        Self {
            name,
            requests: 0,
            errors: 0,
            last_used: None,
            is_available: true,
        }
    }
}

pub struct PluginManager {
    plugins: HashMap<String, PluginConfig>,
    usage: HashMap<String, PluginUsage>,
}

impl PluginManager {
    pub fn new(plugins: Vec<PluginConfig>) -> Self {
        let mut plugin_map = HashMap::new();
        let mut usage_map = HashMap::new();

        for plugin in plugins {
            if plugin.enabled {
                let name = plugin.name.clone();
                usage_map.insert(name.clone(), PluginUsage::new(name.clone()));
                plugin_map.insert(name, plugin);
            }
        }

        Self {
            plugins: plugin_map,
            usage: usage_map,
        }
    }

    pub fn list_plugins(&self) -> Vec<&PluginConfig> {
        self.plugins.values().collect()
    }

    pub fn has_plugin(&self, name: &str) -> bool {
        self.plugins.contains_key(name)
    }

    pub fn get_plugin(&self, name: &str) -> Option<&PluginConfig> {
        self.plugins.get(name)
    }

    pub async fn is_available(&self, name: &str) -> bool {
        let plugin = match self.plugins.get(name) {
            Some(p) => p,
            None => return false,
        };

        match plugin.plugin_type {
            PluginType::Cli => {
                let result = Command::new(&plugin.command)
                    .arg("--version")
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .await;
                result.map(|s| s.success()).unwrap_or(false)
            }
            PluginType::Script => {
                let path = PathBuf::from(&plugin.command);
                path.exists()
            }
            PluginType::Http => {
                true
            }
        }
    }

    pub async fn execute_cli(
        &mut self,
        name: &str,
        prompt: &str,
        output_tx: mpsc::Sender<ToolOutput>,
    ) -> Result<()> {
        let plugin = self.plugins.get(name)
            .ok_or_else(|| anyhow::anyhow!("Plugin not found: {}", name))?
            .clone();

        if let Some(usage) = self.usage.get_mut(name) {
            usage.requests += 1;
            usage.last_used = Some(Utc::now());
        }

        let mut cmd = match plugin.plugin_type {
            PluginType::Script => {
                let interpreter = plugin.interpreter.as_deref().unwrap_or("python");
                let mut c = Command::new(interpreter);
                c.arg(&plugin.command);
                c
            }
            _ => Command::new(&plugin.command),
        };

        for arg in &plugin.args {
            let resolved = arg.replace(&plugin.prompt_placeholder, prompt);
            cmd.arg(resolved);
        }

        if !plugin.args.iter().any(|a| a.contains(&plugin.prompt_placeholder)) {
            cmd.arg(prompt);
        }

        for (key, value) in &plugin.env {
            cmd.env(key, value);
        }

        if let Some(ref dir) = plugin.working_dir {
            cmd.current_dir(dir);
        } else if let Ok(cwd) = std::env::current_dir() {
            cmd.current_dir(cwd);
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());

        let mut child = cmd.spawn()
            .context(format!("Failed to start plugin: {}", name))?;

        let stdout = child.stdout.take().expect("stdout not captured");
        let stderr = child.stderr.take().expect("stderr not captured");

        let output_tx_stdout = output_tx.clone();
        let output_tx_stderr = output_tx.clone();

        let stdout_handle = tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                if output_tx_stdout.send(ToolOutput::Stdout(line)).await.is_err() {
                    break;
                }
            }
        });

        let stderr_handle = tokio::spawn(async move {
            let reader = BufReader::new(stderr);
            let mut lines = reader.lines();

            while let Ok(Some(line)) = lines.next_line().await {
                output_tx_stderr.send(ToolOutput::Stderr(line)).await.ok();
            }
        });

        let status = child.wait().await?;

        let _ = stdout_handle.await;
        let _ = stderr_handle.await;

        let plugin_name = name.to_string();
        if status.success() {
            output_tx.send(ToolOutput::Done {
                tool: polyglot_common::Tool::Claude,
                tokens: None
            }).await.ok();
        } else {
            if let Some(usage) = self.usage.get_mut(&plugin_name) {
                usage.errors += 1;
            }
            output_tx.send(ToolOutput::Error(
                format!("Plugin {} exited with code: {:?}", plugin_name, status.code())
            )).await.ok();
        }

        Ok(())
    }

    pub async fn execute_http(
        &mut self,
        name: &str,
        prompt: &str,
        output_tx: mpsc::Sender<ToolOutput>,
    ) -> Result<()> {
        let plugin = self.plugins.get(name)
            .ok_or_else(|| anyhow::anyhow!("Plugin not found: {}", name))?
            .clone();

        if let Some(usage) = self.usage.get_mut(name) {
            usage.requests += 1;
            usage.last_used = Some(Utc::now());
        }

        let url = plugin.command.replace(&plugin.prompt_placeholder, prompt);

        let mut cmd = Command::new("curl");
        cmd.arg("-s");

        match plugin.http_method {
            HttpMethod::Post => {
                cmd.arg("-X").arg("POST");
            }
            HttpMethod::Get => {
            }
        }

        for (key, value) in &plugin.headers {
            cmd.arg("-H").arg(format!("{}: {}", key, value));
        }

        if let Some(ref body_template) = plugin.body_template {
            let body = body_template.replace(&plugin.prompt_placeholder, prompt);
            cmd.arg("-d").arg(body);
        }

        cmd.arg(&url);
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());

        let output = cmd.output().await
            .context(format!("Failed to execute HTTP plugin: {}", name))?;

        let response = String::from_utf8_lossy(&output.stdout);

        for line in response.lines() {
            output_tx.send(ToolOutput::Stdout(line.to_string())).await.ok();
        }

        if output.status.success() {
            output_tx.send(ToolOutput::Done {
                tool: polyglot_common::Tool::Claude,
                tokens: None
            }).await.ok();
        } else {
            let plugin_name = name.to_string();
            if let Some(usage) = self.usage.get_mut(&plugin_name) {
                usage.errors += 1;
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            output_tx.send(ToolOutput::Error(stderr.to_string())).await.ok();
        }

        Ok(())
    }

    pub async fn execute(
        &mut self,
        name: &str,
        prompt: &str,
        output_tx: mpsc::Sender<ToolOutput>,
    ) -> Result<()> {
        let plugin_type = self.plugins.get(name)
            .map(|p| p.plugin_type.clone())
            .ok_or_else(|| anyhow::anyhow!("Plugin not found: {}", name))?;

        match plugin_type {
            PluginType::Cli | PluginType::Script => {
                self.execute_cli(name, prompt, output_tx).await
            }
            PluginType::Http => {
                self.execute_http(name, prompt, output_tx).await
            }
        }
    }

    pub fn get_usage(&self) -> Vec<&PluginUsage> {
        self.usage.values().collect()
    }
}

pub fn example_plugins() -> Vec<PluginConfig> {
    vec![
        PluginConfig {
            name: "openai".to_string(),
            display_name: Some("OpenAI GPT".to_string()),
            plugin_type: PluginType::Cli,
            enabled: false,
            priority: 10,
            command: "openai".to_string(),
            args: vec!["api".to_string(), "chat.completions.create".to_string(),
                       "-m".to_string(), "gpt-4".to_string(),
                       "-g".to_string(), "user".to_string(), "{prompt}".to_string()],
            prompt_placeholder: "{prompt}".to_string(),
            env: HashMap::new(),
            http_method: HttpMethod::Post,
            headers: HashMap::new(),
            body_template: None,
            response_path: None,
            interpreter: None,
            working_dir: None,
            timeout: 120,
        },

        PluginConfig {
            name: "ollama".to_string(),
            display_name: Some("Ollama (Local LLM)".to_string()),
            plugin_type: PluginType::Cli,
            enabled: false,
            priority: 20,
            command: "ollama".to_string(),
            args: vec!["run".to_string(), "codellama".to_string()],
            prompt_placeholder: "{prompt}".to_string(),
            env: HashMap::new(),
            http_method: HttpMethod::Post,
            headers: HashMap::new(),
            body_template: None,
            response_path: None,
            interpreter: None,
            working_dir: None,
            timeout: 300,
        },

        PluginConfig {
            name: "custom-ai".to_string(),
            display_name: Some("Custom AI Script".to_string()),
            plugin_type: PluginType::Script,
            enabled: false,
            priority: 100,
            command: "~/.polyglot-ai/plugins/my_ai.py".to_string(),
            args: vec!["--prompt".to_string(), "{prompt}".to_string()],
            prompt_placeholder: "{prompt}".to_string(),
            env: HashMap::new(),
            http_method: HttpMethod::Post,
            headers: HashMap::new(),
            body_template: None,
            response_path: None,
            interpreter: Some("python3".to_string()),
            working_dir: None,
            timeout: 120,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plugin_config_display_name() {
        let plugin = PluginConfig {
            name: "test".to_string(),
            display_name: Some("Test Plugin".to_string()),
            plugin_type: PluginType::Cli,
            enabled: true,
            priority: 50,
            command: "echo".to_string(),
            args: vec![],
            prompt_placeholder: "{prompt}".to_string(),
            env: HashMap::new(),
            http_method: HttpMethod::Post,
            headers: HashMap::new(),
            body_template: None,
            response_path: None,
            interpreter: None,
            working_dir: None,
            timeout: 120,
        };

        assert_eq!(plugin.display_name(), "Test Plugin");
    }

    #[test]
    fn test_plugin_manager_creation() {
        let plugins = vec![
            PluginConfig {
                name: "test1".to_string(),
                display_name: None,
                plugin_type: PluginType::Cli,
                enabled: true,
                priority: 10,
                command: "echo".to_string(),
                args: vec![],
                prompt_placeholder: "{prompt}".to_string(),
                env: HashMap::new(),
                http_method: HttpMethod::Post,
                headers: HashMap::new(),
                body_template: None,
                response_path: None,
                interpreter: None,
                working_dir: None,
                timeout: 120,
            },
            PluginConfig {
                name: "test2".to_string(),
                display_name: None,
                plugin_type: PluginType::Cli,
                enabled: false,
                priority: 20,
                command: "echo".to_string(),
                args: vec![],
                prompt_placeholder: "{prompt}".to_string(),
                env: HashMap::new(),
                http_method: HttpMethod::Post,
                headers: HashMap::new(),
                body_template: None,
                response_path: None,
                interpreter: None,
                working_dir: None,
                timeout: 120,
            },
        ];

        let manager = PluginManager::new(plugins);

        assert!(manager.has_plugin("test1"));
        assert!(!manager.has_plugin("test2"));
        assert_eq!(manager.list_plugins().len(), 1);
    }
}
