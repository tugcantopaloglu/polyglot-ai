//! AI tool adapters and management

#![allow(dead_code)]

mod manager;
mod claude;
mod gemini;
mod codex;
mod copilot;
mod cursor;
mod ollama;

pub use manager::*;
pub use claude::ClaudeAdapter;
pub use gemini::GeminiAdapter;
pub use codex::CodexAdapter;
pub use copilot::CopilotAdapter;
pub use cursor::CursorAdapter;
pub use ollama::OllamaAdapter;

use async_trait::async_trait;
use thiserror::Error;
use tokio::sync::mpsc;
use polyglot_common::Tool;

#[derive(Debug, Error)]
pub enum ToolError {
    #[error("Tool not available: {0}")]
    NotAvailable(Tool),
    #[error("Tool execution failed: {0}")]
    ExecutionFailed(String),
    #[error("Rate limit exceeded")]
    RateLimited,
    #[error("Tool timed out")]
    Timeout,
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Process error: {0}")]
    ProcessError(String),
}

#[derive(Debug, Clone)]
pub enum ToolOutput {
    Stdout(String),
    Stderr(String),
    Done { tokens: Option<u64> },
    Error(String),
    RateLimited,
}

#[derive(Debug, Clone)]
pub struct ToolRequest {
    pub message: String,
    pub working_dir: Option<String>,
    pub context_files: Vec<String>,
}

#[async_trait]
pub trait ToolAdapter: Send + Sync {
    fn tool(&self) -> Tool;

    async fn is_available(&self) -> bool;

    async fn execute(
        &self,
        request: ToolRequest,
        output_tx: mpsc::Sender<ToolOutput>,
    ) -> Result<(), ToolError>;

    async fn cancel(&self) -> Result<(), ToolError>;

    fn get_command(&self, request: &ToolRequest) -> String;
}

pub fn is_rate_limit_message(output: &str) -> bool {
    let patterns = [
        "rate limit",
        "too many requests",
        "quota exceeded",
        "429",
        "throttled",
        "try again later",
        "limit reached",
        "exceeded your",
    ];

    let lower = output.to_lowercase();
    patterns.iter().any(|p| lower.contains(p))
}

pub fn parse_token_count(output: &str) -> Option<u64> {
    let lower = output.to_lowercase();

    if let Some(idx) = lower.find("tokens") {
        let after = &output[idx..];
        if let Some(num_start) = after.find(|c: char| c.is_ascii_digit()) {
            let num_str: String = after[num_start..]
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            if let Ok(count) = num_str.parse() {
                return Some(count);
            }
        }
    }

    None
}
