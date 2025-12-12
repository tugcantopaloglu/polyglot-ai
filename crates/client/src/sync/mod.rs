//! Client-side file synchronization

#![allow(dead_code)]

mod watcher;

use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SyncError {
    #[error("File not found: {0}")]
    FileNotFound(PathBuf),
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    #[error("Watch error: {0}")]
    WatchError(String),
}
