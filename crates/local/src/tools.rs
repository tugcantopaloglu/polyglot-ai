//! Local tool execution without network

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;

use parking_lot::RwLock;
use tokio::process::Command;
use tokio::io::{BufReader, AsyncBufReadExt};
use tokio::sync::mpsc;
use chrono::Utc;

use polyglot_common::{Tool, ToolUsage, RotationStrategy};
use crate::config::{LocalConfig, ToolConfig};
use crate::environment::EnvironmentManager;
use crate::sandbox::{SandboxConfig as SandboxSettings};

#[derive(Debug, Clone)]
pub enum ToolOutput {
    Stdout(String),
    Stderr(String),
    Done { tool: Tool, tokens: Option<u64> },
    Error(String),
    RateLimited { tool: Tool, next_tool: Option<Tool> },
}

#[derive(Debug, Clone)]
pub struct TaggedOutput {
    pub tool: Tool,
    pub output: ToolOutput,
}

struct LocalToolManagerInner {
    configs: HashMap<Tool, ToolConfig>,
    usage: RwLock<HashMap<Tool, ToolUsage>>,
    rotation_strategy: RotationStrategy,
    #[allow(dead_code)]
    switch_delay: u8,
    #[allow(dead_code)]
    default_tool: Tool,
    environment: EnvironmentManager,
    sandbox: SandboxSettings,
    force_isolated: bool,
}

#[derive(Clone)]
pub struct LocalToolManager {
    inner: Arc<LocalToolManagerInner>,
}

impl LocalToolManager {
    pub fn new(config: &LocalConfig) -> Self {
        let mut configs = HashMap::new();
        let mut usage = HashMap::new();

        let mut sandbox_settings = SandboxSettings {
            enabled: config.sandbox.enabled,
            sandbox_root: config.sandbox.sandbox_root.clone(),
            allowed_read_paths: config.sandbox.allowed_read_paths.clone(),
            allowed_write_paths: config.sandbox.allowed_write_paths.clone(),
            max_memory_mb: config.sandbox.max_memory_mb,
            max_cpu_percent: config.sandbox.max_cpu_percent,
            network_access: config.sandbox.get_network_policy(),
            env_whitelist: config.sandbox.env_whitelist.clone(),
        };

        if let Ok(current_dir) = std::env::current_dir() {
            if !sandbox_settings.allowed_read_paths.contains(&current_dir) {
                sandbox_settings.allowed_read_paths.push(current_dir.clone());
            }
            if !sandbox_settings.allowed_write_paths.contains(&current_dir) {
                sandbox_settings.allowed_write_paths.push(current_dir);
            }
        }

        if sandbox_settings.allowed_read_paths.is_empty() {
            sandbox_settings.allowed_read_paths.push(sandbox_settings.sandbox_root.clone());
        }

        if sandbox_settings.allowed_write_paths.is_empty() {
            sandbox_settings.allowed_write_paths.push(sandbox_settings.get_workspace_dir());
            sandbox_settings.allowed_write_paths.push(sandbox_settings.get_temp_dir());
        }

        if let Err(e) = sandbox_settings.init_directories() {
            eprintln!("Warning: Failed to initialize sandbox directories: {}", e);
        }

        let environment = EnvironmentManager::new(
            config.isolation.tools_dir.clone(),
            config.isolation.auto_install,
        );

        if let Some(ref c) = config.tools.claude {
            if c.enabled {
                configs.insert(Tool::Claude, c.clone());
                usage.insert(Tool::Claude, ToolUsage::new(Tool::Claude));
            }
        }

        if let Some(ref c) = config.tools.gemini {
            if c.enabled {
                configs.insert(Tool::Gemini, c.clone());
                usage.insert(Tool::Gemini, ToolUsage::new(Tool::Gemini));
            }
        }

        if let Some(ref c) = config.tools.codex {
            if c.enabled {
                configs.insert(Tool::Codex, c.clone());
                usage.insert(Tool::Codex, ToolUsage::new(Tool::Codex));
            }
        }

        if let Some(ref c) = config.tools.copilot {
            if c.enabled {
                configs.insert(Tool::Copilot, c.clone());
                usage.insert(Tool::Copilot, ToolUsage::new(Tool::Copilot));
            }
        }

        if let Some(ref c) = config.tools.perplexity {
            if c.enabled {
                configs.insert(Tool::Perplexity, c.clone());
                usage.insert(Tool::Perplexity, ToolUsage::new(Tool::Perplexity));
            }
        }

        if let Some(ref c) = config.tools.cursor {
            if c.enabled {
                configs.insert(Tool::Cursor, c.clone());
                usage.insert(Tool::Cursor, ToolUsage::new(Tool::Cursor));
            }
        }

        if let Some(ref c) = config.tools.ollama {
            if c.enabled {
                configs.insert(Tool::Ollama, c.clone());
                usage.insert(Tool::Ollama, ToolUsage::new(Tool::Ollama));
            }
        }

        Self {
            inner: Arc::new(LocalToolManagerInner {
                configs,
                usage: RwLock::new(usage),
                rotation_strategy: config.tools.rotation_strategy,
                switch_delay: config.tools.switch_delay,
                default_tool: config.tools.default_tool,
                environment,
                sandbox: sandbox_settings,
                force_isolated: config.isolation.force_isolated,
            }),
        }
    }

    fn get_tool_path(&self, tool: Tool) -> String {
        let config = match self.inner.configs.get(&tool) {
            Some(c) => c,
            None => return tool.as_str().to_string(),
        };

        if self.inner.force_isolated || config.use_isolated {
            self.inner.environment.resolve_tool_path(tool, true)
        } else {
            config.path.clone()
        }
    }

    pub fn environment(&self) -> &EnvironmentManager {
        &self.inner.environment
    }

    pub async fn is_available(&self, tool: Tool) -> bool {
        if !self.inner.configs.contains_key(&tool) {
            return false;
        }

        let tool_path = self.get_tool_path(tool);

        let result = Command::new(&tool_path)
            .args(if tool == Tool::Copilot {
                vec!["--version"]
            } else {
                vec!["--version"]
            })
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await;

        result.map(|s| s.success()).unwrap_or(false)
    }

    pub async fn check_available(&self) -> Vec<Tool> {
        let mut available = Vec::new();

        for tool in Tool::all() {
            if self.is_available(*tool).await {
                available.push(*tool);
            }
        }

        available
    }

    pub async fn execute_streaming(
        &mut self,
        prompt: &str,
        tool: Option<Tool>,
        output_tx: mpsc::Sender<ToolOutput>,
    ) -> anyhow::Result<()> {
        let tool = tool.unwrap_or(self.inner.default_tool);

        let config = self.inner.configs.get(&tool)
            .ok_or_else(|| anyhow::anyhow!("{} is not configured", tool.display_name()))?
            .clone();

        {
            let mut usage = self.inner.usage.write();
            if let Some(stats) = usage.get_mut(&tool) {
                stats.requests += 1;
                stats.last_used = Some(Utc::now());
            }
        }

        let tool_path = self.get_tool_path(tool);
        let mut cmd = Command::new(&tool_path);

        for arg in &config.args {
            cmd.arg(arg);
        }

        match tool {
            Tool::Claude => {
                cmd.arg("--print");
                cmd.arg(prompt);
            }
            Tool::Gemini => {
                cmd.arg("--prompt");
                cmd.arg(prompt);
            }
            Tool::Codex => {
                cmd.arg("--query");
                cmd.arg(prompt);
            }
            Tool::Copilot => {
                cmd.arg("suggest");
                cmd.arg(prompt);
            }
            Tool::Perplexity => {
                cmd.arg(prompt);
            }
            Tool::Cursor => {
                cmd.arg("-p");
                cmd.arg(prompt);
            }
            Tool::Ollama => {
                if config.args.is_empty() {
                    cmd.arg("run");
                    cmd.arg("codellama");
                }
                cmd.arg(prompt);
            }
        }

        let mut filtered_env = self.inner.sandbox.filter_env_vars(&config.env);
        self.inner.sandbox.add_tool_env_vars(&mut filtered_env, tool);

        cmd.env_clear();
        for (key, value) in &filtered_env {
            cmd.env(key, value);
        }

        let working_dir = std::env::current_dir().unwrap_or_else(|_| self.inner.sandbox.get_workspace_dir());
        cmd.current_dir(&working_dir);

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());

        #[cfg(unix)]
        crate::sandbox::unix::apply_resource_limits(&mut cmd, &self.inner.sandbox);

        #[cfg(windows)]
        crate::sandbox::windows::apply_resource_limits(&mut cmd, &self.inner.sandbox);

        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                output_tx.send(ToolOutput::Error(format!("Failed to start {}: {}", tool.display_name(), e))).await.ok();
                return Err(e.into());
            }
        };

        let stdout = child.stdout.take().expect("stdout not captured");
        let stderr = child.stderr.take().expect("stderr not captured");

        let output_tx_stdout = output_tx.clone();
        let output_tx_stderr = output_tx.clone();
        let inner = self.inner.clone();

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
            let mut rate_limited = false;

            while let Ok(Some(line)) = lines.next_line().await {
                let lower = line.to_lowercase();
                if lower.contains("rate limit") ||
                   lower.contains("too many requests") ||
                   lower.contains("quota exceeded") ||
                   lower.contains("429")
                {
                    rate_limited = true;
                    let next_tool = get_next_tool(&inner, tool);
                    output_tx_stderr.send(ToolOutput::RateLimited {
                        tool,
                        next_tool,
                    }).await.ok();

                    let mut usage = inner.usage.write();
                    if let Some(stats) = usage.get_mut(&tool) {
                        stats.rate_limit_hits += 1;
                        stats.is_available = false;
                    }
                } else {
                    output_tx_stderr.send(ToolOutput::Stderr(line)).await.ok();
                }
            }

            rate_limited
        });

        let status = child.wait().await?;

        let _ = stdout_handle.await;
        let rate_limited = stderr_handle.await.unwrap_or(false);

        if rate_limited {
            return Ok(());
        }

        if status.success() {
            output_tx.send(ToolOutput::Done { tool, tokens: None }).await.ok();
        } else {
            {
                let mut usage = self.inner.usage.write();
                if let Some(stats) = usage.get_mut(&tool) {
                    stats.errors += 1;
                }
            }
            output_tx.send(ToolOutput::Error(
                format!("{} exited with code: {:?}", tool.display_name(), status.code())
            )).await.ok();
        }

        Ok(())
    }

    pub fn get_usage(&self) -> Vec<ToolUsage> {
        self.inner.usage.read().values().cloned().collect()
    }

    #[allow(dead_code)]
    pub fn default_tool(&self) -> Tool {
        self.inner.default_tool
    }

    pub async fn execute_multi_streaming(
        &mut self,
        prompt: &str,
        tools: Vec<Tool>,
        output_tx: mpsc::Sender<TaggedOutput>,
    ) -> anyhow::Result<()> {
        use tokio::task::JoinSet;

        let mut join_set: JoinSet<anyhow::Result<()>> = JoinSet::new();

        for tool in tools {
            if !self.inner.configs.contains_key(&tool) {
                let tx = output_tx.clone();
                let _ = tx.send(TaggedOutput {
                    tool,
                    output: ToolOutput::Error(format!("{} is not configured", tool.display_name())),
                }).await;
                continue;
            }

            let tx = output_tx.clone();
            let prompt = prompt.to_string();
            let mut tm = self.clone();

            join_set.spawn(async move {
                let (tool_tx, mut tool_rx) = mpsc::channel::<ToolOutput>(100);

                let exec_tool = tool;
                let exec_prompt = prompt.clone();
                let exec_handle = tokio::spawn(async move {
                    tm.execute_streaming(&exec_prompt, Some(exec_tool), tool_tx).await
                });

                while let Some(output) = tool_rx.recv().await {
                    if tx.send(TaggedOutput { tool, output }).await.is_err() {
                        break;
                    }
                }

                exec_handle.await??;
                Ok(())
            });
        }

        while let Some(result) = join_set.join_next().await {
            let _ = result;
        }

        Ok(())
    }

    #[allow(dead_code)]
    pub fn configured_tools(&self) -> Vec<Tool> {
        self.inner.configs.keys().copied().collect()
    }
}

fn get_next_tool(inner: &LocalToolManagerInner, current: Tool) -> Option<Tool> {
    let usage = inner.usage.read();

    match inner.rotation_strategy {
        RotationStrategy::OnLimit | RotationStrategy::Priority => {
            let priorities = [Tool::Claude, Tool::Gemini, Tool::Codex, Tool::Copilot, Tool::Perplexity, Tool::Cursor];
            for tool in priorities {
                if tool != current {
                    if let Some(stats) = usage.get(&tool) {
                        if stats.is_available && inner.configs.contains_key(&tool) {
                            return Some(tool);
                        }
                    }
                }
            }
        }
        RotationStrategy::RoundRobin => {
            let all_tools = [Tool::Claude, Tool::Gemini, Tool::Codex, Tool::Copilot, Tool::Perplexity, Tool::Cursor];
            let current_idx = all_tools.iter().position(|t| *t == current).unwrap_or(0);

            for i in 1..all_tools.len() {
                let next_idx = (current_idx + i) % all_tools.len();
                let next_tool = all_tools[next_idx];
                if inner.configs.contains_key(&next_tool) {
                    if let Some(stats) = usage.get(&next_tool) {
                        if stats.is_available {
                            return Some(next_tool);
                        }
                    }
                }
            }
        }
    }

    None
}
