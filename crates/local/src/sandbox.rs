use std::path::{Path, PathBuf};
use std::collections::HashSet;
use anyhow::{Result, bail, Context};
use polyglot_common::Tool;

#[derive(Debug, Clone)]
pub struct SandboxConfig {
    pub enabled: bool,
    pub sandbox_root: PathBuf,
    pub allowed_read_paths: Vec<PathBuf>,
    pub allowed_write_paths: Vec<PathBuf>,
    pub max_memory_mb: Option<u64>,
    pub max_cpu_percent: Option<u8>,
    pub network_access: NetworkPolicy,
    pub env_whitelist: HashSet<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkPolicy {
    Deny,
    AllowLocalhost,
    AllowAll,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        let sandbox_root = directories::BaseDirs::new()
            .map(|d| d.data_dir().join("polyglot").join("sandbox"))
            .unwrap_or_else(|| PathBuf::from(".polyglot/sandbox"));

        let mut env_whitelist = HashSet::new();
        env_whitelist.insert("PATH".to_string());
        env_whitelist.insert("HOME".to_string());
        env_whitelist.insert("USER".to_string());
        env_whitelist.insert("LANG".to_string());
        env_whitelist.insert("LC_ALL".to_string());
        env_whitelist.insert("TERM".to_string());
        env_whitelist.insert("PWD".to_string());

        Self {
            enabled: true,
            sandbox_root: sandbox_root.clone(),
            allowed_read_paths: vec![sandbox_root.clone()],
            allowed_write_paths: vec![
                sandbox_root.join("workspace"),
                sandbox_root.join("temp"),
            ],
            max_memory_mb: Some(4096),
            max_cpu_percent: Some(80),
            network_access: NetworkPolicy::AllowAll,
            env_whitelist,
        }
    }
}

impl SandboxConfig {
    pub fn init_directories(&self) -> Result<()> {
        std::fs::create_dir_all(&self.sandbox_root)
            .context("Failed to create sandbox root")?;

        std::fs::create_dir_all(self.sandbox_root.join("workspace"))
            .context("Failed to create workspace directory")?;

        std::fs::create_dir_all(self.sandbox_root.join("temp"))
            .context("Failed to create temp directory")?;

        std::fs::create_dir_all(self.sandbox_root.join("tools"))
            .context("Failed to create tools directory")?;

        std::fs::create_dir_all(self.sandbox_root.join("cache"))
            .context("Failed to create cache directory")?;

        Ok(())
    }

    pub fn get_workspace_dir(&self) -> PathBuf {
        self.sandbox_root.join("workspace")
    }

    pub fn get_temp_dir(&self) -> PathBuf {
        self.sandbox_root.join("temp")
    }

    pub fn get_tools_dir(&self) -> PathBuf {
        self.sandbox_root.join("tools")
    }

    pub fn get_cache_dir(&self) -> PathBuf {
        self.sandbox_root.join("cache")
    }

    pub fn validate_path_read(&self, path: &Path) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let canonical = path.canonicalize()
            .unwrap_or_else(|_| path.to_path_buf());

        for allowed in &self.allowed_read_paths {
            if canonical.starts_with(allowed) {
                return Ok(());
            }
        }

        bail!("Access denied: path '{}' is outside sandbox read boundaries", path.display())
    }

    pub fn validate_path_write(&self, path: &Path) -> Result<()> {
        if !self.enabled {
            return Ok(());
        }

        let canonical = path.canonicalize()
            .unwrap_or_else(|_| path.to_path_buf());

        for allowed in &self.allowed_write_paths {
            if canonical.starts_with(allowed) {
                return Ok(());
            }
        }

        bail!("Access denied: path '{}' is outside sandbox write boundaries", path.display())
    }

    pub fn filter_env_vars(&self, env: &[(String, String)]) -> Vec<(String, String)> {
        if !self.enabled {
            return env.to_vec();
        }

        env.iter()
            .filter(|(key, _)| self.env_whitelist.contains(key))
            .cloned()
            .collect()
    }

    pub fn add_tool_env_vars(&self, env: &mut Vec<(String, String)>, tool: Tool) {
        env.push(("POLYGLOT_SANDBOX".to_string(), "1".to_string()));
        env.push(("POLYGLOT_TOOL".to_string(), tool.as_str().to_string()));
        env.push(("POLYGLOT_WORKSPACE".to_string(), self.get_workspace_dir().to_string_lossy().to_string()));
        env.push(("POLYGLOT_TOOLS_DIR".to_string(), self.get_tools_dir().to_string_lossy().to_string()));
        env.push(("POLYGLOT_CACHE_DIR".to_string(), self.get_cache_dir().to_string_lossy().to_string()));

        if let Ok(current_dir) = std::env::current_dir() {
            env.push(("POLYGLOT_PROJECT_DIR".to_string(), current_dir.to_string_lossy().to_string()));
        }

        env.push(("TMPDIR".to_string(), self.get_temp_dir().to_string_lossy().to_string()));
        env.push(("TEMP".to_string(), self.get_temp_dir().to_string_lossy().to_string()));
        env.push(("TMP".to_string(), self.get_temp_dir().to_string_lossy().to_string()));
    }
}

#[cfg(unix)]
pub mod unix {
    use super::*;
    use std::os::unix::process::CommandExt;
    use tokio::process::Command;

    pub fn apply_resource_limits(cmd: &mut Command, config: &SandboxConfig) {
        if !config.enabled {
            return;
        }

        let _ = (config.max_cpu_percent, config.network_access);

        if let Some(max_mem_mb) = config.max_memory_mb {
            let max_mem_bytes = max_mem_mb * 1024 * 1024;
            unsafe {
                use libc::{setrlimit, rlimit, RLIMIT_AS};
                let limit = rlimit {
                    rlim_cur: max_mem_bytes,
                    rlim_max: max_mem_bytes,
                };
                cmd.pre_exec(move || {
                    setrlimit(RLIMIT_AS, &limit);
                    Ok(())
                });
            }
        }
    }
}

#[cfg(windows)]
pub mod windows {
    use super::*;
    use tokio::process::Command;

    pub fn apply_resource_limits(_cmd: &mut Command, config: &SandboxConfig) {
        if !config.enabled {
            return;
        }

        let _ = (config.max_memory_mb, config.max_cpu_percent, config.network_access);
    }
}
