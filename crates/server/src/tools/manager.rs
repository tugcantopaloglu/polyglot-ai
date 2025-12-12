//! Tool manager with rotation logic

use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;
use chrono::Utc;
use tokio::sync::mpsc;
use polyglot_common::{Tool, ToolUsage, RotationStrategy};
use super::{ToolAdapter, ToolError, ToolOutput, ToolRequest};
use super::{ClaudeAdapter, GeminiAdapter, CodexAdapter, CopilotAdapter, CursorAdapter, OllamaAdapter};
use crate::config::ToolsSettings;

struct ToolManagerInner {
    adapters: HashMap<Tool, Arc<dyn ToolAdapter>>,
    usage: RwLock<HashMap<Tool, ToolUsage>>,
    rotation_strategy: RotationStrategy,
    switch_delay: u8,
    default_tool: Tool,
    current_tool: RwLock<Tool>,
}

#[derive(Clone)]
pub struct ToolManager {
    inner: Arc<ToolManagerInner>,
}

impl ToolManager {
    pub fn new(config: &ToolsSettings) -> Self {
        let mut adapters: HashMap<Tool, Arc<dyn ToolAdapter>> = HashMap::new();
        let mut usage: HashMap<Tool, ToolUsage> = HashMap::new();

        if let Some(ref claude_config) = config.claude {
            if claude_config.enabled {
                adapters.insert(
                    Tool::Claude,
                    Arc::new(ClaudeAdapter::new(
                        claude_config.path.clone(),
                        claude_config.args.clone(),
                        claude_config.env.clone(),
                    )),
                );
                usage.insert(Tool::Claude, ToolUsage::new(Tool::Claude));
            }
        }

        if let Some(ref gemini_config) = config.gemini {
            if gemini_config.enabled {
                adapters.insert(
                    Tool::Gemini,
                    Arc::new(GeminiAdapter::new(
                        gemini_config.path.clone(),
                        gemini_config.args.clone(),
                        gemini_config.env.clone(),
                    )),
                );
                usage.insert(Tool::Gemini, ToolUsage::new(Tool::Gemini));
            }
        }

        if let Some(ref codex_config) = config.codex {
            if codex_config.enabled {
                adapters.insert(
                    Tool::Codex,
                    Arc::new(CodexAdapter::new(
                        codex_config.path.clone(),
                        codex_config.args.clone(),
                        codex_config.env.clone(),
                    )),
                );
                usage.insert(Tool::Codex, ToolUsage::new(Tool::Codex));
            }
        }

        if let Some(ref copilot_config) = config.copilot {
            if copilot_config.enabled {
                adapters.insert(
                    Tool::Copilot,
                    Arc::new(CopilotAdapter::new(
                        copilot_config.path.clone(),
                        copilot_config.args.clone(),
                        copilot_config.env.clone(),
                    )),
                );
                usage.insert(Tool::Copilot, ToolUsage::new(Tool::Copilot));
            }
        }

        if let Some(ref cursor_config) = config.cursor {
            if cursor_config.enabled {
                adapters.insert(
                    Tool::Cursor,
                    Arc::new(CursorAdapter::new(
                        cursor_config.path.clone(),
                        cursor_config.args.clone(),
                        cursor_config.env.clone(),
                    )),
                );
                usage.insert(Tool::Cursor, ToolUsage::new(Tool::Cursor));
            }
        }

        if let Some(ref ollama_config) = config.ollama {
            if ollama_config.enabled {
                let model = ollama_config.args.get(1)
                    .cloned()
                    .unwrap_or_else(|| "codellama".to_string());
                adapters.insert(
                    Tool::Ollama,
                    Arc::new(OllamaAdapter::new(
                        ollama_config.path.clone(),
                        model,
                        ollama_config.args.clone(),
                        ollama_config.env.clone(),
                    )),
                );
                usage.insert(Tool::Ollama, ToolUsage::new(Tool::Ollama));
            }
        }

        Self {
            inner: Arc::new(ToolManagerInner {
                adapters,
                usage: RwLock::new(usage),
                rotation_strategy: config.rotation_strategy,
                switch_delay: config.switch_delay,
                default_tool: config.default_tool,
                current_tool: RwLock::new(config.default_tool),
            }),
        }
    }

    pub async fn available_tools(&self) -> Vec<Tool> {
        let mut available = Vec::new();
        for (tool, adapter) in &self.inner.adapters {
            if adapter.is_available().await {
                available.push(*tool);
            }
        }
        available
    }

    pub fn current_tool(&self) -> Tool {
        *self.inner.current_tool.read()
    }

    pub fn set_current_tool(&self, tool: Tool) -> Result<(), ToolError> {
        if !self.inner.adapters.contains_key(&tool) {
            return Err(ToolError::NotAvailable(tool));
        }
        *self.inner.current_tool.write() = tool;
        Ok(())
    }

    pub fn get_usage(&self) -> Vec<ToolUsage> {
        self.inner.usage.read().values().cloned().collect()
    }

    pub fn switch_delay(&self) -> u8 {
        self.inner.switch_delay
    }

    pub async fn execute(
        &self,
        tool: Option<Tool>,
        request: ToolRequest,
        output_tx: mpsc::Sender<ToolOutput>,
    ) -> Result<Tool, ToolError> {
        let tool = tool.unwrap_or_else(|| self.current_tool());

        let adapter = self.inner.adapters.get(&tool)
            .ok_or(ToolError::NotAvailable(tool))?;

        {
            let mut usage = self.inner.usage.write();
            if let Some(stats) = usage.get_mut(&tool) {
                stats.requests += 1;
                stats.last_used = Some(Utc::now());
            }
        }

        let (internal_tx, mut internal_rx) = mpsc::channel::<ToolOutput>(100);
        let output_tx_clone = output_tx.clone();
        let inner_clone = self.inner.clone();
        let tool_clone = tool;

        let monitor_handle = tokio::spawn(async move {
            let mut rate_limited = false;
            let mut tokens = None;

            while let Some(output) = internal_rx.recv().await {
                match &output {
                    ToolOutput::RateLimited => {
                        rate_limited = true;
                        let mut usage = inner_clone.usage.write();
                        if let Some(stats) = usage.get_mut(&tool_clone) {
                            stats.rate_limit_hits += 1;
                            stats.is_available = false;
                        }
                    }
                    ToolOutput::Done { tokens: t } => {
                        tokens = *t;
                        if let Some(count) = t {
                            let mut usage = inner_clone.usage.write();
                            if let Some(stats) = usage.get_mut(&tool_clone) {
                                stats.tokens_used += count;
                            }
                        }
                    }
                    ToolOutput::Error(_) => {
                        let mut usage = inner_clone.usage.write();
                        if let Some(stats) = usage.get_mut(&tool_clone) {
                            stats.errors += 1;
                        }
                    }
                    _ => {}
                }

                if output_tx_clone.send(output).await.is_err() {
                    break;
                }
            }

            (rate_limited, tokens)
        });

        let result = adapter.execute(request, internal_tx).await;

        let (rate_limited, _tokens) = monitor_handle.await.unwrap_or((false, None));

        if rate_limited {
            return Err(ToolError::RateLimited);
        }

        result.map(|_| tool)
    }

    pub async fn get_next_tool(&self, current: Tool) -> Option<Tool> {
        let available = self.available_tools().await;

        match self.inner.rotation_strategy {
            RotationStrategy::OnLimit | RotationStrategy::Priority => {
                let priorities = [Tool::Claude, Tool::Gemini, Tool::Codex, Tool::Copilot, Tool::Perplexity, Tool::Cursor];
                for tool in priorities {
                    if tool != current && available.contains(&tool) {
                        let usage = self.inner.usage.read();
                        if let Some(stats) = usage.get(&tool) {
                            if stats.is_available {
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
                    if available.contains(&next_tool) {
                        return Some(next_tool);
                    }
                }
            }
        }

        None
    }

    pub async fn cancel_all(&self) {
        for adapter in self.inner.adapters.values() {
            let _ = adapter.cancel().await;
        }
    }

    pub fn reset_availability(&self) {
        let mut usage = self.inner.usage.write();
        for stats in usage.values_mut() {
            stats.is_available = true;
        }
    }
}
