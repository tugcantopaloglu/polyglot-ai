//! Gemini CLI adapter

use async_trait::async_trait;
use tokio::process::Command;
use tokio::io::{BufReader, AsyncBufReadExt};
use tokio::sync::mpsc;
use std::process::Stdio;
use std::sync::Arc;
use parking_lot::Mutex;
use polyglot_common::Tool;
use super::{ToolAdapter, ToolError, ToolOutput, ToolRequest, is_rate_limit_message};

pub struct GeminiAdapter {
    path: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    current_process: Arc<Mutex<Option<u32>>>,
}

impl GeminiAdapter {
    pub fn new(path: String, args: Vec<String>, env: Vec<(String, String)>) -> Self {
        Self {
            path,
            args,
            env,
            current_process: Arc::new(Mutex::new(None)),
        }
    }
}

#[async_trait]
impl ToolAdapter for GeminiAdapter {
    fn tool(&self) -> Tool {
        Tool::Gemini
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

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.stdin(Stdio::null());

        let mut child = cmd.spawn()?;

        if let Some(pid) = child.id() {
            *self.current_process.lock() = Some(pid);
        }

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
            let mut rate_limited = false;

            while let Ok(Some(line)) = lines.next_line().await {
                if is_rate_limit_message(&line) {
                    rate_limited = true;
                    let _ = output_tx_stderr.send(ToolOutput::RateLimited).await;
                } else {
                    let _ = output_tx_stderr.send(ToolOutput::Stderr(line)).await;
                }
            }

            rate_limited
        });

        let status = child.wait().await?;

        let _ = stdout_handle.await;
        let rate_limited = stderr_handle.await.unwrap_or(false);

        *self.current_process.lock() = None;

        if rate_limited {
            output_tx.send(ToolOutput::RateLimited).await.ok();
            return Err(ToolError::RateLimited);
        }

        if status.success() {
            output_tx.send(ToolOutput::Done { tokens: None }).await.ok();
            Ok(())
        } else {
            let error_msg = format!("Gemini exited with code: {:?}", status.code());
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
        let mut parts = vec![self.path.clone()];
        parts.extend(self.args.clone());
        parts.push("-p".to_string());
        parts.push(format!("\"{}\"", request.message));
        parts.join(" ")
    }
}
