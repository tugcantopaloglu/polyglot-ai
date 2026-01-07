//! Ollama adapter for local LLM support
//! Ollama runs LLMs locally: https://ollama.ai

use async_trait::async_trait;
use tokio::process::Command;
use tokio::io::{BufReader, AsyncBufReadExt};
use tokio::sync::mpsc;
use std::process::Stdio;
use std::sync::Arc;
use parking_lot::Mutex;
use polyglot_common::Tool;
use super::{ToolAdapter, ToolError, ToolOutput, ToolRequest};

pub struct OllamaAdapter {
    path: String,
    model: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    current_process: Arc<Mutex<Option<u32>>>,
}

impl OllamaAdapter {
    pub fn new(path: String, model: String, args: Vec<String>, env: Vec<(String, String)>) -> Self {
        Self {
            path,
            model,
            args,
            env,
            current_process: Arc::new(Mutex::new(None)),
        }
    }

    pub fn with_default_model(env: Vec<(String, String)>) -> Self {
        Self::new(
            "ollama".to_string(),
            "codellama".to_string(),
            vec![],
            env,
        )
    }

    async fn is_model_downloaded(&self) -> Result<bool, ToolError> {
        let output = Command::new(&self.path)
            .arg("list")
            .output()
            .await?;

        if !output.status.success() {
            return Ok(false);
        }

        let list_output = String::from_utf8_lossy(&output.stdout);
        Ok(list_output.lines().any(|line| line.starts_with(&self.model)))
    }
}

#[async_trait]
impl ToolAdapter for OllamaAdapter {
    fn tool(&self) -> Tool {
        Tool::Ollama
    }

    async fn is_available(&self) -> bool {
        Command::new(&self.path)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    async fn execute(
        &self,
        request: ToolRequest,
        output_tx: mpsc::Sender<ToolOutput>,
    ) -> Result<(), ToolError> {
        let is_downloaded = self.is_model_downloaded().await.unwrap_or(false);

        if !is_downloaded {
            output_tx.send(ToolOutput::Stderr(format!("⚠️  Model '{}' is not downloaded.", self.model))).await.ok();
            output_tx.send(ToolOutput::Stderr("".to_string())).await.ok();
            output_tx.send(ToolOutput::Stderr("This will download the model, which may:".to_string())).await.ok();
            output_tx.send(ToolOutput::Stderr("  - Take several minutes to complete".to_string())).await.ok();
            output_tx.send(ToolOutput::Stderr("  - Use significant bandwidth".to_string())).await.ok();
            output_tx.send(ToolOutput::Stderr("  - Require several GB of disk space".to_string())).await.ok();
            output_tx.send(ToolOutput::Stderr("".to_string())).await.ok();
            output_tx.send(ToolOutput::Stderr("Press Ctrl+C within 5 seconds to cancel...".to_string())).await.ok();

            for i in (1..=5).rev() {
                tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                output_tx.send(ToolOutput::Stderr(format!("Starting download in {}...", i))).await.ok();
            }

            output_tx.send(ToolOutput::Stderr("".to_string())).await.ok();
            output_tx.send(ToolOutput::Stderr("Starting model download...".to_string())).await.ok();
        }

        let mut cmd = Command::new(&self.path);

        cmd.arg("run");
        cmd.arg(&self.model);

        for arg in &self.args {
            cmd.arg(arg);
        }

        cmd.arg(&request.message);

        if let Some(ref dir) = request.working_dir {
            cmd.current_dir(dir);
        }

        for (key, value) in &self.env {
            cmd.env(key, value);
        }
        for (key, value) in &request.env {
            cmd.env(key, value);
        }

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());

        let mut child = cmd.spawn()?;

        if let Some(pid) = child.id() {
            *self.current_process.lock() = Some(pid);
        }

        let stdout = child.stdout.take()
            .ok_or_else(|| ToolError::ExecutionFailed("Failed to capture stdout".to_string()))?;
        let stderr = child.stderr.take()
            .ok_or_else(|| ToolError::ExecutionFailed("Failed to capture stderr".to_string()))?;

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
                let _ = output_tx_stderr.send(ToolOutput::Stderr(line)).await;
            }
        });

        let status = child.wait().await?;

        let _ = stdout_handle.await;
        let _ = stderr_handle.await;

        *self.current_process.lock() = None;

        if status.success() {
            output_tx.send(ToolOutput::Done { tokens: None }).await.ok();
            Ok(())
        } else {
            let error_msg = format!("Ollama exited with code: {:?}", status.code());
            output_tx.send(ToolOutput::Error(error_msg.clone())).await.ok();
            Err(ToolError::ExecutionFailed(error_msg))
        }
    }

    async fn cancel(&self) -> Result<(), ToolError> {
        let pid = *self.current_process.lock();
        if let Some(pid) = pid {
            #[cfg(unix)]
            {
                use nix::sys::signal::{kill, Signal};
                use nix::unistd::Pid;
                let _ = kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
            }
            #[cfg(windows)]
            {
                let _ = Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/F"])
                    .status()
                    .await;
            }
        }
        *self.current_process.lock() = None;
        Ok(())
    }

    fn get_command(&self, request: &ToolRequest) -> String {
        let mut parts = vec![self.path.clone(), "run".to_string(), self.model.clone()];
        parts.extend(self.args.clone());
        parts.push(format!("\"{}\"", request.message));
        parts.join(" ")
    }
}
