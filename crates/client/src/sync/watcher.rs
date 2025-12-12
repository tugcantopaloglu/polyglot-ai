//! File system watcher for client-side sync

#![allow(dead_code)]

use std::path::PathBuf;
use std::sync::Arc;
use parking_lot::RwLock;
use tokio::sync::mpsc;
use notify::{Watcher, RecursiveMode, Event, EventKind};
use tracing::warn;
use super::SyncError;

#[derive(Debug, Clone)]
pub enum FileChange {
    Created(PathBuf),
    Modified(PathBuf),
    Deleted(PathBuf),
    Renamed { from: PathBuf, to: PathBuf },
}

pub struct FileWatcher {
    watcher: Option<notify::RecommendedWatcher>,
    watched_paths: Arc<RwLock<Vec<PathBuf>>>,
    debounce_ms: u64,
}

impl FileWatcher {
    pub fn new(debounce_ms: u64) -> Self {
        Self {
            watcher: None,
            watched_paths: Arc::new(RwLock::new(Vec::new())),
            debounce_ms,
        }
    }

    pub fn start(&mut self, paths: Vec<PathBuf>, tx: mpsc::Sender<FileChange>) -> Result<(), SyncError> {
        let _debounce_ms = self.debounce_ms;

        let watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            match res {
                Ok(event) => {
                    let changes = event_to_changes(event);
                    for change in changes {
                        if tx.blocking_send(change).is_err() {
                            break;
                        }
                    }
                }
                Err(e) => {
                    warn!("Watch error: {}", e);
                }
            }
        }).map_err(|e| SyncError::WatchError(e.to_string()))?;

        self.watcher = Some(watcher);

        for path in &paths {
            self.add_path(path)?;
        }

        *self.watched_paths.write() = paths;

        Ok(())
    }

    pub fn add_path(&mut self, path: &PathBuf) -> Result<(), SyncError> {
        if let Some(ref mut watcher) = self.watcher {
            watcher.watch(path, RecursiveMode::Recursive)
                .map_err(|e| SyncError::WatchError(e.to_string()))?;
            self.watched_paths.write().push(path.clone());
        }
        Ok(())
    }

    pub fn remove_path(&mut self, path: &PathBuf) -> Result<(), SyncError> {
        if let Some(ref mut watcher) = self.watcher {
            watcher.unwatch(path)
                .map_err(|e| SyncError::WatchError(e.to_string()))?;
            self.watched_paths.write().retain(|p| p != path);
        }
        Ok(())
    }

    pub fn stop(&mut self) {
        self.watcher = None;
        self.watched_paths.write().clear();
    }

    pub fn watched_paths(&self) -> Vec<PathBuf> {
        self.watched_paths.read().clone()
    }

    pub fn is_watching(&self) -> bool {
        self.watcher.is_some()
    }
}

fn event_to_changes(event: Event) -> Vec<FileChange> {
    let mut changes = Vec::new();

    match event.kind {
        EventKind::Create(_) => {
            for path in event.paths {
                changes.push(FileChange::Created(path));
            }
        }
        EventKind::Modify(_) => {
            for path in event.paths {
                changes.push(FileChange::Modified(path));
            }
        }
        EventKind::Remove(_) => {
            for path in event.paths {
                changes.push(FileChange::Deleted(path));
            }
        }
        _ => {
            if event.paths.len() >= 2 {
                changes.push(FileChange::Renamed {
                    from: event.paths[0].clone(),
                    to: event.paths[1].clone(),
                });
            }
        }
    }

    changes
}

pub fn compute_file_hash(path: &PathBuf) -> Result<String, SyncError> {
    use xxhash_rust::xxh3::xxh3_64;

    let content = std::fs::read(path)?;
    let hash = xxh3_64(&content);
    Ok(format!("{:016x}", hash))
}

pub fn list_files(root: &PathBuf, ignore_patterns: &[String]) -> Result<Vec<PathBuf>, SyncError> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files, ignore_patterns)?;
    Ok(files)
}

fn collect_files(
    root: &PathBuf,
    current: &PathBuf,
    files: &mut Vec<PathBuf>,
    ignore_patterns: &[String],
) -> Result<(), SyncError> {
    if !current.exists() {
        return Ok(());
    }

    for entry in std::fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();

        let relative = path.strip_prefix(root)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        if should_ignore(&relative, ignore_patterns) {
            continue;
        }

        if path.is_dir() {
            collect_files(root, &path, files, ignore_patterns)?;
        } else {
            files.push(path);
        }
    }

    Ok(())
}

fn should_ignore(path: &str, patterns: &[String]) -> bool {
    for pattern in patterns {
        if pattern.starts_with('*') {
            let suffix = &pattern[1..];
            if path.ends_with(suffix) {
                return true;
            }
        } else if path.contains(pattern) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_ignore() {
        let patterns = vec![
            ".git".to_string(),
            "node_modules".to_string(),
            "*.pyc".to_string(),
        ];

        assert!(should_ignore(".git/config", &patterns));
        assert!(should_ignore("foo/node_modules/bar", &patterns));
        assert!(should_ignore("test.pyc", &patterns));
        assert!(!should_ignore("src/main.rs", &patterns));
    }
}
