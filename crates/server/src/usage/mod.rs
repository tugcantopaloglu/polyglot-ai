//! Usage tracking module

#![allow(dead_code)]

mod stats;

pub use stats::*;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum UsageError {
    #[error("Database error: {0}")]
    DatabaseError(#[from] rusqlite::Error),
    #[error("Tool not found: {0}")]
    ToolNotFound(String),
    #[error("Lock error")]
    LockError,
}
