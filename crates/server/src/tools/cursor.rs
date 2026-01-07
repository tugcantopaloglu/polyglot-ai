//! Cursor Agent CLI adapter
//! Cursor Agent CLI (cursor-agent) from https://cursor.com/cli
//! Requires WSL on Windows, runs natively on macOS and Linux

use async_trait::async_trait;
use tokio::process::Command;
use tokio::io::{BufReader, AsyncBufReadExt};
use tokio::sync::mpsc;
use std::process::Stdio;
use std::sync::Arc;
use parking_lot::Mutex;
use polyglot_common::Tool;
use super::{ToolAdapter, ToolError, ToolOutput, ToolRequest, is_rate_limit_message};

pub struct CursorAdapter {
    path: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    current_process: Arc<Mutex<Option<u32>>>,
}

impl CursorAdapter {
    pub fn new(path: String, args: Vec<String>, env: Vec<(String, String)>) -> Self {
        Self {
            path,
            args,
            env,
            current_process: Arc::new(Mutex::new(None)),
        }
    }

    #[cfg(windows)]
    pub fn for_current_platform(env: Vec<(String, String)>) -> Self {
        Self::new(
            "wsl".to_string(),
            vec!["cursor-agent".to_string()],
            env,
        )
    }

    #[cfg(not(windows))]
    pub fn for_current_platform(env: Vec<(String, String)>) -> Self {
        Self::new(
            "cursor-agent".to_string(),
            vec![],
            env,
        )
    }
}

#[async_trait]
impl ToolAdapter for CursorAdapter {
    fn tool(&self) -> Tool {
        Tool::Cursor
    }

    async fn is_available(&self) -> bool {
        let mut cmd = Command::new(&self.path);

        for arg in &self.args {
            cmd.arg(arg);
        }
        cmd.arg("--version");

        cmd.stdout(Stdio::null())
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
        let mut cmd = Command::new(&self.path);

        for arg in &self.args {
            cmd.arg(arg);
        }

        cmd.arg("-p");
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
            let mut rate_limited = false;

            while let Ok(Some(line)) = lines.next_line().await {
                if is_rate_limit_message(&line) {
                    rate_limited = true;
                }
                if output_tx_stderr.send(ToolOutput::Stderr(line)).await.is_err() {
                    break;
                }
            }

            rate_limited
        });

        let (_, rate_limited) = tokio::try_join!(stdout_handle, stderr_handle)
            .map_err(|e| ToolError::ExecutionFailed(e.to_string()))?;

        *self.current_process.lock() = None;

        let status = child.wait().await?;

        if rate_limited {
            output_tx.send(ToolOutput::RateLimited).await.ok();
            return Err(ToolError::RateLimited);
        }

        if status.success() {
            output_tx.send(ToolOutput::Done { tokens: None }).await.ok();
        } else {
            output_tx.send(ToolOutput::Error(format!(
                "Cursor CLI exited with code: {:?}",
                status.code()
            ))).await.ok();
        }

        Ok(())
    }

    async fn cancel(&self) -> Result<(), ToolError> {
        if let Some(pid) = *self.current_process.lock() {
            #[cfg(unix)]
            {
                use nix::sys::signal::{kill, Signal};
                use nix::unistd::Pid;
                let _ = kill(Pid::from_raw(pid as i32), Signal::SIGTERM);
            }
            #[cfg(windows)]
            {
                let _ = std::process::Command::new("taskkill")
                    .args(["/PID", &pid.to_string(), "/F"])
                    .output();
            }
        }
        Ok(())
    }

    fn get_command(&self, request: &ToolRequest) -> String {
        let mut parts = vec![self.path.clone()];
        parts.extend(self.args.clone());
        parts.push("--prompt".to_string());
        parts.push(format!("\"{}\"", request.message.chars().take(50).collect::<String>()));
        parts.join(" ")
    }
}
