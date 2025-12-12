//! File synchronization engine

#![allow(dead_code)]

mod realtime;
mod ondemand;

use thiserror::Error;
use std::path::PathBuf;
use polyglot_common::{FileInfo, FileConflict, SyncMode};
use chrono::{DateTime, Utc};

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("File not found: {0}")]
    FileNotFound(PathBuf),
    #[error("Permission denied: {0}")]
    PermissionDenied(PathBuf),
    #[error("Conflict detected: {0}")]
    ConflictDetected(String),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Sync aborted")]
    Aborted,
}

pub struct SyncManager {
    sync_dir: PathBuf,
    mode: SyncMode,
}

impl SyncManager {
    pub fn new(sync_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&sync_dir).ok();

        Self {
            sync_dir,
            mode: SyncMode::default(),
        }
    }

    pub fn set_mode(&mut self, mode: SyncMode) {
        self.mode = mode;
    }

    pub fn mode(&self) -> SyncMode {
        self.mode
    }

    pub fn sync_dir(&self) -> &PathBuf {
        &self.sync_dir
    }

    pub fn user_sync_dir(&self, user_id: &str) -> PathBuf {
        self.sync_dir.join(user_id)
    }

    pub fn list_files(&self, base_path: &PathBuf) -> Result<Vec<FileInfo>, SyncError> {
        let mut files = Vec::new();

        if !base_path.exists() {
            return Ok(files);
        }

        self.collect_files(base_path, base_path, &mut files)?;
        Ok(files)
    }

    fn collect_files(
        &self,
        root: &PathBuf,
        current: &PathBuf,
        files: &mut Vec<FileInfo>,
    ) -> Result<(), SyncError> {
        for entry in std::fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = entry.metadata()?;

            let relative_path = path.strip_prefix(root)
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();

            if relative_path.starts_with('.') ||
               relative_path.contains("node_modules") ||
               relative_path.contains("target") ||
               relative_path.contains(".git") {
                continue;
            }

            if metadata.is_dir() {
                files.push(FileInfo {
                    path: relative_path.clone(),
                    size: 0,
                    hash: String::new(),
                    modified_at: DateTime::<Utc>::from(metadata.modified()?),
                    is_directory: true,
                });
                self.collect_files(root, &path, files)?;
            } else {
                let hash = self.compute_file_hash(&path)?;
                files.push(FileInfo {
                    path: relative_path,
                    size: metadata.len(),
                    hash,
                    modified_at: DateTime::<Utc>::from(metadata.modified()?),
                    is_directory: false,
                });
            }
        }

        Ok(())
    }

    pub fn compute_file_hash(&self, path: &PathBuf) -> Result<String, SyncError> {
        use xxhash_rust::xxh3::xxh3_64;

        let content = std::fs::read(path)?;
        let hash = xxh3_64(&content);
        Ok(format!("{:016x}", hash))
    }

    pub fn detect_conflicts(
        &self,
        local_files: &[FileInfo],
        remote_files: &[FileInfo],
    ) -> Vec<FileConflict> {
        let mut conflicts = Vec::new();

        for local in local_files {
            if local.is_directory {
                continue;
            }

            if let Some(remote) = remote_files.iter().find(|r| r.path == local.path) {
                if !remote.is_directory && local.hash != remote.hash {
                    conflicts.push(FileConflict {
                        path: local.path.clone(),
                        local_hash: local.hash.clone(),
                        remote_hash: remote.hash.clone(),
                        local_modified: local.modified_at,
                        remote_modified: remote.modified_at,
                    });
                }
            }
        }

        conflicts
    }

    pub fn read_file(&self, path: &PathBuf) -> Result<Vec<u8>, SyncError> {
        if !path.exists() {
            return Err(SyncError::FileNotFound(path.clone()));
        }
        Ok(std::fs::read(path)?)
    }

    pub fn write_file(&self, path: &PathBuf, content: &[u8]) -> Result<(), SyncError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        Ok(())
    }

    pub fn delete_file(&self, path: &PathBuf) -> Result<(), SyncError> {
        if path.exists() {
            if path.is_dir() {
                std::fs::remove_dir_all(path)?;
            } else {
                std::fs::remove_file(path)?;
            }
        }
        Ok(())
    }

    pub fn backup_file(&self, path: &PathBuf) -> Result<PathBuf, SyncError> {
        if !path.exists() {
            return Err(SyncError::FileNotFound(path.clone()));
        }

        let backup_name = format!(
            "{}.backup.{}",
            path.file_name().unwrap_or_default().to_string_lossy(),
            chrono::Utc::now().timestamp()
        );
        let backup_path = path.with_file_name(backup_name);

        std::fs::copy(path, &backup_path)?;
        Ok(backup_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_sync_manager_creation() {
        let temp_dir = std::env::temp_dir().join("polyglot_sync_test");
        let manager = SyncManager::new(temp_dir.clone());
        assert!(temp_dir.exists());
        std::fs::remove_dir_all(&temp_dir).ok();
    }

    #[test]
    fn test_file_hash() {
        let temp_dir = std::env::temp_dir().join("polyglot_hash_test");
        std::fs::create_dir_all(&temp_dir).ok();
        let test_file = temp_dir.join("test.txt");
        std::fs::write(&test_file, "hello world").unwrap();

        let manager = SyncManager::new(temp_dir.clone());
        let hash = manager.compute_file_hash(&test_file).unwrap();
        assert!(!hash.is_empty());

        std::fs::remove_dir_all(&temp_dir).ok();
    }
}
