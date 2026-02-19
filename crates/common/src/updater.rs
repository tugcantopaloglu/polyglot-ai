//! Self-update functionality for Polyglot-AI binaries
//!
//! Provides safe auto-update with backup and rollback capabilities.

use std::path::{Path, PathBuf};
use std::fs;
use std::io::{self, Write};
use serde::{Deserialize, Serialize};

/// GitHub release information
#[derive(Debug, Clone, Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
    pub name: String,
    pub body: Option<String>,
    pub published_at: String,
    pub assets: Vec<GitHubAsset>,
    pub prerelease: bool,
    pub draft: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GitHubAsset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
    pub content_type: String,
}

/// Update check result
#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub release_notes: Option<String>,
    pub download_url: Option<String>,
    pub asset_name: Option<String>,
}

/// Update status for tracking progress
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateStatus {
    pub phase: UpdatePhase,
    pub message: String,
    pub progress: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpdatePhase {
    Checking,
    Downloading,
    Backing,
    Installing,
    Verifying,
    Complete,
    RollingBack,
    Failed,
}

/// Backup information for rollback
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupInfo {
    pub original_path: PathBuf,
    pub backup_path: PathBuf,
    pub version: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Update configuration
#[derive(Debug, Clone)]
pub struct UpdateConfig {
    pub github_repo: String,
    pub current_version: String,
    pub binary_name: String,
    pub check_prerelease: bool,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            github_repo: "tugcantopaloglu/polyglot-ai".to_string(),
            current_version: env!("CARGO_PKG_VERSION").to_string(),
            binary_name: "polyglot".to_string(),
            check_prerelease: false,
        }
    }
}

/// Compare semantic versions
/// Returns: Greater if a > b, Less if a < b, Equal if a == b
pub fn version_compare(a: &str, b: &str) -> std::cmp::Ordering {
    let parse = |v: &str| -> Vec<u32> {
        v.trim_start_matches('v')
            .split('.')
            .filter_map(|s| s.split('-').next()?.parse().ok())
            .collect()
    };

    let a_parts = parse(a);
    let b_parts = parse(b);

    for i in 0..3 {
        let a_val = a_parts.get(i).copied().unwrap_or(0);
        let b_val = b_parts.get(i).copied().unwrap_or(0);
        match a_val.cmp(&b_val) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    std::cmp::Ordering::Equal
}

/// Get the platform-specific asset name (matches Rust target triples)
pub fn get_platform_asset_name(binary_name: &str) -> String {
    let target = if cfg!(target_os = "windows") {
        if cfg!(target_arch = "x86_64") {
            "x86_64-pc-windows-msvc"
        } else if cfg!(target_arch = "aarch64") {
            "aarch64-pc-windows-msvc"
        } else {
            "x86_64-pc-windows-msvc"
        }
    } else if cfg!(target_os = "macos") {
        if cfg!(target_arch = "x86_64") {
            "x86_64-apple-darwin"
        } else if cfg!(target_arch = "aarch64") {
            "aarch64-apple-darwin"
        } else {
            "x86_64-apple-darwin"
        }
    } else {
        if cfg!(target_arch = "x86_64") {
            "x86_64-unknown-linux-gnu"
        } else if cfg!(target_arch = "aarch64") {
            "aarch64-unknown-linux-gnu"
        } else {
            "x86_64-unknown-linux-gnu"
        }
    };

    let ext = if cfg!(target_os = "windows") { ".exe" } else { "" };

    format!("{}-{}{}", binary_name, target, ext)
}

/// Get the backup directory path
pub fn get_backup_dir() -> PathBuf {
    if let Some(data_dir) = directories::ProjectDirs::from("ai", "polyglot", "polyglot") {
        data_dir.data_dir().join("backups")
    } else {
        PathBuf::from(".polyglot-backups")
    }
}

/// Create a backup of the current binary
pub fn create_backup(binary_path: &Path, version: &str) -> io::Result<BackupInfo> {
    let backup_dir = get_backup_dir();
    fs::create_dir_all(&backup_dir)?;

    let binary_name = binary_path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    let timestamp = chrono::Utc::now();
    let backup_name = format!(
        "{}.{}.backup",
        binary_name,
        timestamp.format("%Y%m%d_%H%M%S")
    );
    let backup_path = backup_dir.join(&backup_name);

    fs::copy(binary_path, &backup_path)?;

    // Save backup metadata
    let info = BackupInfo {
        original_path: binary_path.to_path_buf(),
        backup_path: backup_path.clone(),
        version: version.to_string(),
        timestamp,
    };

    let metadata_path = backup_dir.join(format!("{}.json", backup_name));
    let metadata = serde_json::to_string_pretty(&info)
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    fs::write(&metadata_path, metadata)?;

    Ok(info)
}

/// Restore from backup
pub fn restore_backup(backup_info: &BackupInfo) -> io::Result<()> {
    if !backup_info.backup_path.exists() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "Backup file not found",
        ));
    }

    // On Windows, we may need to rename the current binary first
    #[cfg(windows)]
    {
        let old_path = backup_info.original_path.with_extension("exe.old");
        if backup_info.original_path.exists() {
            fs::rename(&backup_info.original_path, &old_path)?;
        }
        fs::copy(&backup_info.backup_path, &backup_info.original_path)?;
        let _ = fs::remove_file(&old_path);
    }

    #[cfg(not(windows))]
    {
        fs::copy(&backup_info.backup_path, &backup_info.original_path)?;
        
        // Restore executable permissions
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&backup_info.original_path)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&backup_info.original_path, perms)?;
        }
    }

    Ok(())
}

/// Delete old backups, keeping only the most recent N
pub fn cleanup_old_backups(keep_count: usize) -> io::Result<()> {
    let backup_dir = get_backup_dir();
    if !backup_dir.exists() {
        return Ok(());
    }

    let mut backups: Vec<_> = fs::read_dir(&backup_dir)?
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|ext| ext == "backup").unwrap_or(false))
        .collect();

    backups.sort_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
    backups.reverse();

    for entry in backups.into_iter().skip(keep_count) {
        let path = entry.path();
        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(path.with_extension("backup.json"));
    }

    Ok(())
}

/// Get the path of the currently running executable
pub fn get_current_exe() -> io::Result<PathBuf> {
    std::env::current_exe()
}

/// Verify a binary is valid by checking it can be executed
pub fn verify_binary(path: &Path) -> bool {
    #[cfg(windows)]
    {
        // On Windows, check the PE header
        if let Ok(data) = fs::read(path) {
            data.len() > 2 && data[0] == b'M' && data[1] == b'Z'
        } else {
            false
        }
    }
    
    #[cfg(not(windows))]
    {
        // On Unix, check ELF header or try to get version
        if let Ok(data) = fs::read(path) {
            data.len() > 4 && &data[0..4] == b"\x7fELF"
        } else {
            false
        }
    }
}

/// Format bytes to human readable size
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;

    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} bytes", bytes)
    }
}

/// Print update status to stdout
pub fn print_status(status: &UpdateStatus) {
    let icon = match status.phase {
        UpdatePhase::Checking => "🔍",
        UpdatePhase::Downloading => "⬇️",
        UpdatePhase::Backing => "💾",
        UpdatePhase::Installing => "📦",
        UpdatePhase::Verifying => "✅",
        UpdatePhase::Complete => "🎉",
        UpdatePhase::RollingBack => "⏪",
        UpdatePhase::Failed => "❌",
    };

    if let Some(progress) = status.progress {
        print!("\r{} {} [{}%]", icon, status.message, progress);
        io::stdout().flush().ok();
    } else {
        println!("{} {}", icon, status.message);
    }
}

/// Check for updates from GitHub releases.
/// `binary_name` is the binary to look for in release assets (e.g. "polyglot", "polyglot-local", "polyglot-server").
pub async fn check_for_updates_github(binary_name: &str) -> anyhow::Result<UpdateInfo> {
    let client = reqwest::Client::builder()
        .user_agent("polyglot-ai-updater")
        .timeout(std::time::Duration::from_secs(30))
        .build()?;

    let url = "https://api.github.com/repos/tugcantopaloglu/polyglot-ai/releases/latest";

    let response = client.get(url).send().await?;

    if !response.status().is_success() {
        anyhow::bail!("Failed to check for updates: HTTP {}", response.status());
    }

    let release: GitHubRelease = response.json().await?;

    let current_version = env!("CARGO_PKG_VERSION");
    let latest_version = release.tag_name.trim_start_matches('v').to_string();

    let update_available = version_compare(&latest_version, current_version) == std::cmp::Ordering::Greater;

    let asset_name = get_platform_asset_name(binary_name);
    let (download_url, found_asset) = release.assets.iter()
        .find(|a| a.name == asset_name || a.name.contains(&asset_name.replace(".exe", "")))
        .map(|a| (Some(a.browser_download_url.clone()), Some(a.name.clone())))
        .unwrap_or((None, None));

    Ok(UpdateInfo {
        current_version: current_version.to_string(),
        latest_version,
        update_available,
        release_notes: release.body,
        download_url,
        asset_name: found_asset,
    })
}

/// Perform the actual update: backup, download, verify, replace binary.
pub async fn perform_update(update_info: &UpdateInfo) -> anyhow::Result<()> {
    let download_url = update_info.download_url.as_ref()
        .ok_or_else(|| anyhow::anyhow!("No download URL available for your platform"))?;

    let current_exe = get_current_exe()?;
    let current_version = env!("CARGO_PKG_VERSION");

    // Phase 1: Create backup
    print_status(&UpdateStatus {
        phase: UpdatePhase::Backing,
        message: "Creating backup...".to_string(),
        progress: None,
    });

    let backup_info = create_backup(&current_exe, current_version)?;
    println!("  Backup saved to: {:?}", backup_info.backup_path);

    // Phase 2: Download new version
    print_status(&UpdateStatus {
        phase: UpdatePhase::Downloading,
        message: format!("Downloading v{}...", update_info.latest_version),
        progress: Some(0),
    });

    let client = reqwest::Client::builder()
        .user_agent("polyglot-ai-updater")
        .timeout(std::time::Duration::from_secs(300))
        .build()?;

    let response = client.get(download_url).send().await?;

    if !response.status().is_success() {
        restore_backup(&backup_info)?;
        anyhow::bail!("Download failed: HTTP {}", response.status());
    }

    let new_binary = response.bytes().await?;

    println!();
    print_status(&UpdateStatus {
        phase: UpdatePhase::Downloading,
        message: format!("Downloaded {}", format_bytes(new_binary.len() as u64)),
        progress: Some(100),
    });

    // Phase 3: Install new version
    print_status(&UpdateStatus {
        phase: UpdatePhase::Installing,
        message: "Installing update...".to_string(),
        progress: None,
    });

    let temp_path = current_exe.with_extension("new");

    if let Err(e) = fs::write(&temp_path, &new_binary) {
        tracing::error!("Failed to write new binary: {}", e);
        restore_backup(&backup_info)?;
        print_status(&UpdateStatus {
            phase: UpdatePhase::Failed,
            message: format!("Failed to write new binary: {}", e),
            progress: None,
        });
        return Err(e.into());
    }

    // Phase 4: Verify the new binary
    print_status(&UpdateStatus {
        phase: UpdatePhase::Verifying,
        message: "Verifying new binary...".to_string(),
        progress: None,
    });

    if !verify_binary(&temp_path) {
        let _ = fs::remove_file(&temp_path);
        restore_backup(&backup_info)?;
        print_status(&UpdateStatus {
            phase: UpdatePhase::Failed,
            message: "Downloaded binary is invalid!".to_string(),
            progress: None,
        });
        anyhow::bail!("Downloaded binary failed verification");
    }

    // Phase 5: Replace the current binary
    #[cfg(windows)]
    {
        let old_path = current_exe.with_extension("exe.old");
        if old_path.exists() {
            let _ = fs::remove_file(&old_path);
        }
        fs::rename(&current_exe, &old_path)?;
        fs::rename(&temp_path, &current_exe)?;
        let _ = fs::remove_file(&old_path);
    }

    #[cfg(not(windows))]
    {
        fs::rename(&temp_path, &current_exe)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&current_exe)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&current_exe, perms)?;
        }
    }

    let _ = cleanup_old_backups(3);

    println!();
    print_status(&UpdateStatus {
        phase: UpdatePhase::Complete,
        message: format!("Successfully updated to v{}!", update_info.latest_version),
        progress: None,
    });

    println!();
    println!("\x1b[32m✓ Update complete! Please restart the application.\x1b[0m");

    Ok(())
}

/// Check for updates on startup (returns notification string if update available).
pub async fn check_updates_on_startup(binary_name: &str) -> Option<String> {
    match check_for_updates_github(binary_name).await {
        Ok(info) if info.update_available => {
            Some(format!(
                "Update available: v{} -> v{}",
                info.current_version, info.latest_version
            ))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_version_compare() {
        assert_eq!(version_compare("1.0.0", "1.0.0"), std::cmp::Ordering::Equal);
        assert_eq!(version_compare("1.0.1", "1.0.0"), std::cmp::Ordering::Greater);
        assert_eq!(version_compare("1.0.0", "1.0.1"), std::cmp::Ordering::Less);
        assert_eq!(version_compare("2.0.0", "1.9.9"), std::cmp::Ordering::Greater);
        assert_eq!(version_compare("v1.0.0", "1.0.0"), std::cmp::Ordering::Equal);
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(500), "500 bytes");
        assert_eq!(format_bytes(1024), "1.00 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.00 MB");
    }
}
