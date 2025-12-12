//! Real-time file synchronization using file system watcher

use std::path::PathBuf;
use std::sync::Arc;
use std::collections::HashMap;
use parking_lot::RwLock;
use tokio::sync::mpsc;
use notify::{Watcher, RecursiveMode, Event, EventKind};
use super::SyncError;

#[derive(Debug, Clone)]
pub enum FileChange {
    Created(PathBuf),
    Modified(PathBuf),
    Deleted(PathBuf),
    Renamed { from: PathBuf, to: PathBuf },
}

pub struct RealtimeWatcher {
    watchers: Arc<RwLock<HashMap<String, notify::RecommendedWatcher>>>,
}

impl RealtimeWatcher {
    pub fn new() -> Self {
        Self {
            watchers: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn watch(
        &self,
        session_id: &str,
        path: PathBuf,
        change_tx: mpsc::Sender<FileChange>,
    ) -> Result<(), SyncError> {
        let session_id = session_id.to_string();

        let mut watcher = notify::recommended_watcher(move |res: Result<Event, notify::Error>| {
            if let Ok(event) = res {
                let changes = Self::event_to_changes(event);
                for change in changes {
                    let _ = change_tx.blocking_send(change);
                }
            }
        }).map_err(|e| SyncError::IoError(std::io::Error::new(
            std::io::ErrorKind::Other,
            e.to_string(),
        )))?;

        watcher.watch(&path, RecursiveMode::Recursive)
            .map_err(|e| SyncError::IoError(std::io::Error::new(
                std::io::ErrorKind::Other,
                e.to_string(),
            )))?;

        self.watchers.write().insert(session_id, watcher);
        Ok(())
    }

    pub fn unwatch(&self, session_id: &str) {
        self.watchers.write().remove(session_id);
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
            EventKind::Any => {
                if event.paths.len() >= 2 {
                    changes.push(FileChange::Renamed {
                        from: event.paths[0].clone(),
                        to: event.paths[1].clone(),
                    });
                }
            }
            _ => {}
        }

        changes
    }

    pub fn active_count(&self) -> usize {
        self.watchers.read().len()
    }
}

impl Default for RealtimeWatcher {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ChangeDebouncer {
    pending: Arc<RwLock<HashMap<PathBuf, std::time::Instant>>>,
    debounce_ms: u64,
}

impl ChangeDebouncer {
    pub fn new(debounce_ms: u64) -> Self {
        Self {
            pending: Arc::new(RwLock::new(HashMap::new())),
            debounce_ms,
        }
    }

    pub fn should_process(&self, path: &PathBuf) -> bool {
        let now = std::time::Instant::now();
        let mut pending = self.pending.write();

        if let Some(last_time) = pending.get(path) {
            if now.duration_since(*last_time).as_millis() < self.debounce_ms as u128 {
                return false;
            }
        }

        pending.insert(path.clone(), now);
        true
    }

    pub fn cleanup(&self) {
        let now = std::time::Instant::now();
        let threshold = std::time::Duration::from_millis(self.debounce_ms * 2);
        self.pending.write().retain(|_, time| now.duration_since(*time) < threshold);
    }
}
