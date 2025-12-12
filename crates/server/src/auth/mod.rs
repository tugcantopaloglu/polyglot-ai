//! Authentication and session management

#![allow(dead_code)]

mod session;
mod users;
mod invite;

pub use session::*;
pub use users::*;
pub use invite::*;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("Invalid credentials")]
    InvalidCredentials,
    #[error("Session expired")]
    SessionExpired,
    #[error("Session not found")]
    SessionNotFound,
    #[error("User not found")]
    UserNotFound,
    #[error("User already exists")]
    UserExists,
    #[error("Permission denied")]
    PermissionDenied,
    #[error("Lock error")]
    LockError,
    #[error("Database error: {0}")]
    DatabaseError(#[from] rusqlite::Error),
    #[error("JWT error: {0}")]
    JwtError(#[from] jsonwebtoken::errors::Error),
}
