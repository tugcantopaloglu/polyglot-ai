//! Persistent storage using SQLite
//!
//! Provides storage for:
//! - User quotas
//! - Session data
//! - Audit logs
//! - API keys (encrypted)
//! - Cache entries

use std::path::Path;
use std::sync::Arc;

use chrono::Utc;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};

use crate::Tool;

/// Database connection wrapper
pub struct Database {
    conn: Arc<Mutex<rusqlite::Connection>>,
}

impl Database {
    /// Open or create a database at the given path
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let conn = rusqlite::Connection::open(path)
            .map_err(|e| StorageError::ConnectionFailed(e.to_string()))?;

        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };

        db.initialize_schema()?;
        Ok(db)
    }

    /// Open an in-memory database (for testing)
    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn = rusqlite::Connection::open_in_memory()
            .map_err(|e| StorageError::ConnectionFailed(e.to_string()))?;

        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };

        db.initialize_schema()?;
        Ok(db)
    }

    fn initialize_schema(&self) -> Result<(), StorageError> {
        let conn = self.conn.lock();

        conn.execute_batch(
            r#"
            -- Quota tracking
            CREATE TABLE IF NOT EXISTS quotas (
                user_id TEXT PRIMARY KEY,
                daily_requests INTEGER DEFAULT 0,
                monthly_requests INTEGER DEFAULT 0,
                daily_tokens INTEGER DEFAULT 0,
                monthly_tokens INTEGER DEFAULT 0,
                daily_reset TEXT,
                monthly_reset TEXT,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT DEFAULT CURRENT_TIMESTAMP
            );

            -- Session persistence
            CREATE TABLE IF NOT EXISTS sessions (
                session_id TEXT PRIMARY KEY,
                user_id TEXT,
                tool TEXT,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP,
                last_active TEXT DEFAULT CURRENT_TIMESTAMP,
                expires_at TEXT,
                metadata TEXT
            );

            -- Audit log
            CREATE TABLE IF NOT EXISTS audit_log (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT DEFAULT CURRENT_TIMESTAMP,
                user_id TEXT,
                action TEXT NOT NULL,
                tool TEXT,
                prompt_hash TEXT,
                response_tokens INTEGER,
                latency_ms INTEGER,
                success INTEGER,
                error_message TEXT,
                ip_address TEXT,
                metadata TEXT
            );

            -- API keys (encrypted)
            CREATE TABLE IF NOT EXISTS api_keys (
                key_id TEXT PRIMARY KEY,
                tool TEXT NOT NULL,
                encrypted_key BLOB NOT NULL,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP,
                last_used TEXT,
                is_active INTEGER DEFAULT 1
            );

            -- Response cache
            CREATE TABLE IF NOT EXISTS cache (
                cache_key TEXT PRIMARY KEY,
                tool TEXT NOT NULL,
                prompt_hash TEXT NOT NULL,
                response TEXT NOT NULL,
                tokens INTEGER,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP,
                expires_at TEXT,
                hit_count INTEGER DEFAULT 0
            );

            -- Webhooks
            CREATE TABLE IF NOT EXISTS webhooks (
                webhook_id TEXT PRIMARY KEY,
                url TEXT NOT NULL,
                events TEXT NOT NULL,
                secret TEXT,
                is_active INTEGER DEFAULT 1,
                created_at TEXT DEFAULT CURRENT_TIMESTAMP,
                last_triggered TEXT,
                failure_count INTEGER DEFAULT 0
            );

            -- Indexes
            CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit_log(timestamp);
            CREATE INDEX IF NOT EXISTS idx_audit_user ON audit_log(user_id);
            CREATE INDEX IF NOT EXISTS idx_sessions_user ON sessions(user_id);
            CREATE INDEX IF NOT EXISTS idx_cache_expires ON cache(expires_at);
            "#,
        )
        .map_err(|e| StorageError::SchemaError(e.to_string()))?;

        Ok(())
    }

    // =========================================================================
    // Quota Operations
    // =========================================================================

    pub fn get_quota(&self, user_id: &str) -> Result<Option<StoredQuota>, StorageError> {
        let conn = self.conn.lock();

        let mut stmt = conn
            .prepare("SELECT * FROM quotas WHERE user_id = ?")
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        let result = stmt.query_row([user_id], |row| {
            Ok(StoredQuota {
                user_id: row.get(0)?,
                daily_requests: row.get(1)?,
                monthly_requests: row.get(2)?,
                daily_tokens: row.get(3)?,
                monthly_tokens: row.get(4)?,
                daily_reset: row.get(5)?,
                monthly_reset: row.get(6)?,
            })
        });

        match result {
            Ok(quota) => Ok(Some(quota)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::QueryError(e.to_string())),
        }
    }

    pub fn save_quota(&self, quota: &StoredQuota) -> Result<(), StorageError> {
        let conn = self.conn.lock();

        conn.execute(
            r#"
            INSERT INTO quotas (user_id, daily_requests, monthly_requests, daily_tokens, monthly_tokens, daily_reset, monthly_reset, updated_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, CURRENT_TIMESTAMP)
            ON CONFLICT(user_id) DO UPDATE SET
                daily_requests = ?2,
                monthly_requests = ?3,
                daily_tokens = ?4,
                monthly_tokens = ?5,
                daily_reset = ?6,
                monthly_reset = ?7,
                updated_at = CURRENT_TIMESTAMP
            "#,
            rusqlite::params![
                quota.user_id,
                quota.daily_requests,
                quota.monthly_requests,
                quota.daily_tokens,
                quota.monthly_tokens,
                quota.daily_reset,
                quota.monthly_reset,
            ],
        )
        .map_err(|e| StorageError::WriteError(e.to_string()))?;

        Ok(())
    }

    pub fn increment_quota(&self, user_id: &str, requests: u64, tokens: u64) -> Result<(), StorageError> {
        let conn = self.conn.lock();

        conn.execute(
            r#"
            INSERT INTO quotas (user_id, daily_requests, monthly_requests, daily_tokens, monthly_tokens, daily_reset, monthly_reset)
            VALUES (?1, ?2, ?2, ?3, ?3, datetime('now', '+1 day'), datetime('now', '+30 days'))
            ON CONFLICT(user_id) DO UPDATE SET
                daily_requests = daily_requests + ?2,
                monthly_requests = monthly_requests + ?2,
                daily_tokens = daily_tokens + ?3,
                monthly_tokens = monthly_tokens + ?3,
                updated_at = CURRENT_TIMESTAMP
            "#,
            rusqlite::params![user_id, requests, tokens],
        )
        .map_err(|e| StorageError::WriteError(e.to_string()))?;

        Ok(())
    }

    pub fn reset_daily_quotas(&self) -> Result<u64, StorageError> {
        let conn = self.conn.lock();

        let affected = conn.execute(
            r#"
            UPDATE quotas
            SET daily_requests = 0, daily_tokens = 0, daily_reset = datetime('now', '+1 day')
            WHERE datetime(daily_reset) <= datetime('now')
            "#,
            [],
        )
        .map_err(|e| StorageError::WriteError(e.to_string()))?;

        Ok(affected as u64)
    }

    pub fn reset_monthly_quotas(&self) -> Result<u64, StorageError> {
        let conn = self.conn.lock();

        let affected = conn.execute(
            r#"
            UPDATE quotas
            SET monthly_requests = 0, monthly_tokens = 0, monthly_reset = datetime('now', '+30 days')
            WHERE datetime(monthly_reset) <= datetime('now')
            "#,
            [],
        )
        .map_err(|e| StorageError::WriteError(e.to_string()))?;

        Ok(affected as u64)
    }

    // =========================================================================
    // Session Operations
    // =========================================================================

    pub fn get_session(&self, session_id: &str) -> Result<Option<StoredSession>, StorageError> {
        let conn = self.conn.lock();

        let mut stmt = conn
            .prepare("SELECT * FROM sessions WHERE session_id = ?")
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        let result = stmt.query_row([session_id], |row| {
            Ok(StoredSession {
                session_id: row.get(0)?,
                user_id: row.get(1)?,
                tool: row.get(2)?,
                created_at: row.get(3)?,
                last_active: row.get(4)?,
                expires_at: row.get(5)?,
                metadata: row.get(6)?,
            })
        });

        match result {
            Ok(session) => Ok(Some(session)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::QueryError(e.to_string())),
        }
    }

    pub fn save_session(&self, session: &StoredSession) -> Result<(), StorageError> {
        let conn = self.conn.lock();

        conn.execute(
            r#"
            INSERT INTO sessions (session_id, user_id, tool, created_at, last_active, expires_at, metadata)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ON CONFLICT(session_id) DO UPDATE SET
                last_active = ?5,
                tool = ?3,
                metadata = ?7
            "#,
            rusqlite::params![
                session.session_id,
                session.user_id,
                session.tool,
                session.created_at,
                session.last_active,
                session.expires_at,
                session.metadata,
            ],
        )
        .map_err(|e| StorageError::WriteError(e.to_string()))?;

        Ok(())
    }

    pub fn delete_session(&self, session_id: &str) -> Result<bool, StorageError> {
        let conn = self.conn.lock();

        let affected = conn
            .execute("DELETE FROM sessions WHERE session_id = ?", [session_id])
            .map_err(|e| StorageError::WriteError(e.to_string()))?;

        Ok(affected > 0)
    }

    pub fn cleanup_expired_sessions(&self) -> Result<u64, StorageError> {
        let conn = self.conn.lock();

        let affected = conn.execute(
            "DELETE FROM sessions WHERE datetime(expires_at) <= datetime('now')",
            [],
        )
        .map_err(|e| StorageError::WriteError(e.to_string()))?;

        Ok(affected as u64)
    }

    // =========================================================================
    // Audit Log Operations
    // =========================================================================

    pub fn log_audit(&self, entry: &AuditLogEntry) -> Result<i64, StorageError> {
        let conn = self.conn.lock();

        conn.execute(
            r#"
            INSERT INTO audit_log (timestamp, user_id, action, tool, prompt_hash, response_tokens, latency_ms, success, error_message, ip_address, metadata)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            "#,
            rusqlite::params![
                entry.timestamp,
                entry.user_id,
                entry.action,
                entry.tool,
                entry.prompt_hash,
                entry.response_tokens,
                entry.latency_ms,
                entry.success,
                entry.error_message,
                entry.ip_address,
                entry.metadata,
            ],
        )
        .map_err(|e| StorageError::WriteError(e.to_string()))?;

        Ok(conn.last_insert_rowid())
    }

    pub fn get_audit_logs(&self, limit: u32, offset: u32) -> Result<Vec<AuditLogEntry>, StorageError> {
        let conn = self.conn.lock();

        let mut stmt = conn
            .prepare("SELECT * FROM audit_log ORDER BY timestamp DESC LIMIT ? OFFSET ?")
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        let rows = stmt
            .query_map([limit, offset], |row| {
                Ok(AuditLogEntry {
                    id: Some(row.get(0)?),
                    timestamp: row.get(1)?,
                    user_id: row.get(2)?,
                    action: row.get(3)?,
                    tool: row.get(4)?,
                    prompt_hash: row.get(5)?,
                    response_tokens: row.get(6)?,
                    latency_ms: row.get(7)?,
                    success: row.get(8)?,
                    error_message: row.get(9)?,
                    ip_address: row.get(10)?,
                    metadata: row.get(11)?,
                })
            })
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row.map_err(|e| StorageError::QueryError(e.to_string()))?);
        }

        Ok(entries)
    }

    pub fn get_audit_logs_for_user(&self, user_id: &str, limit: u32) -> Result<Vec<AuditLogEntry>, StorageError> {
        let conn = self.conn.lock();

        let mut stmt = conn
            .prepare("SELECT * FROM audit_log WHERE user_id = ? ORDER BY timestamp DESC LIMIT ?")
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        let rows = stmt
            .query_map(rusqlite::params![user_id, limit], |row| {
                Ok(AuditLogEntry {
                    id: Some(row.get(0)?),
                    timestamp: row.get(1)?,
                    user_id: row.get(2)?,
                    action: row.get(3)?,
                    tool: row.get(4)?,
                    prompt_hash: row.get(5)?,
                    response_tokens: row.get(6)?,
                    latency_ms: row.get(7)?,
                    success: row.get(8)?,
                    error_message: row.get(9)?,
                    ip_address: row.get(10)?,
                    metadata: row.get(11)?,
                })
            })
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row.map_err(|e| StorageError::QueryError(e.to_string()))?);
        }

        Ok(entries)
    }

    // =========================================================================
    // API Key Operations
    // =========================================================================

    pub fn save_api_key(&self, key: &StoredApiKey) -> Result<(), StorageError> {
        let conn = self.conn.lock();

        conn.execute(
            r#"
            INSERT INTO api_keys (key_id, tool, encrypted_key, created_at, is_active)
            VALUES (?1, ?2, ?3, ?4, ?5)
            ON CONFLICT(key_id) DO UPDATE SET
                encrypted_key = ?3,
                is_active = ?5
            "#,
            rusqlite::params![
                key.key_id,
                key.tool,
                key.encrypted_key,
                key.created_at,
                key.is_active,
            ],
        )
        .map_err(|e| StorageError::WriteError(e.to_string()))?;

        Ok(())
    }

    pub fn get_api_key(&self, tool: &str) -> Result<Option<StoredApiKey>, StorageError> {
        let conn = self.conn.lock();

        let mut stmt = conn
            .prepare("SELECT * FROM api_keys WHERE tool = ? AND is_active = 1 ORDER BY created_at DESC LIMIT 1")
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        let result = stmt.query_row([tool], |row| {
            Ok(StoredApiKey {
                key_id: row.get(0)?,
                tool: row.get(1)?,
                encrypted_key: row.get(2)?,
                created_at: row.get(3)?,
                last_used: row.get(4)?,
                is_active: row.get(5)?,
            })
        });

        match result {
            Ok(key) => Ok(Some(key)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::QueryError(e.to_string())),
        }
    }

    pub fn deactivate_api_key(&self, key_id: &str) -> Result<bool, StorageError> {
        let conn = self.conn.lock();

        let affected = conn
            .execute("UPDATE api_keys SET is_active = 0 WHERE key_id = ?", [key_id])
            .map_err(|e| StorageError::WriteError(e.to_string()))?;

        Ok(affected > 0)
    }

    // =========================================================================
    // Cache Operations
    // =========================================================================

    pub fn get_cached_response(&self, cache_key: &str) -> Result<Option<CachedResponse>, StorageError> {
        let conn = self.conn.lock();

        let mut stmt = conn
            .prepare("SELECT * FROM cache WHERE cache_key = ? AND datetime(expires_at) > datetime('now')")
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        let result = stmt.query_row([cache_key], |row| {
            Ok(CachedResponse {
                cache_key: row.get(0)?,
                tool: row.get(1)?,
                prompt_hash: row.get(2)?,
                response: row.get(3)?,
                tokens: row.get(4)?,
                created_at: row.get(5)?,
                expires_at: row.get(6)?,
                hit_count: row.get(7)?,
            })
        });

        match result {
            Ok(cached) => {
                // Increment hit count
                let _ = conn.execute(
                    "UPDATE cache SET hit_count = hit_count + 1 WHERE cache_key = ?",
                    [cache_key],
                );
                Ok(Some(cached))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StorageError::QueryError(e.to_string())),
        }
    }

    pub fn save_cached_response(&self, cached: &CachedResponse) -> Result<(), StorageError> {
        let conn = self.conn.lock();

        conn.execute(
            r#"
            INSERT INTO cache (cache_key, tool, prompt_hash, response, tokens, created_at, expires_at, hit_count)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
            ON CONFLICT(cache_key) DO UPDATE SET
                response = ?4,
                tokens = ?5,
                expires_at = ?7,
                hit_count = 0
            "#,
            rusqlite::params![
                cached.cache_key,
                cached.tool,
                cached.prompt_hash,
                cached.response,
                cached.tokens,
                cached.created_at,
                cached.expires_at,
                cached.hit_count,
            ],
        )
        .map_err(|e| StorageError::WriteError(e.to_string()))?;

        Ok(())
    }

    pub fn cleanup_expired_cache(&self) -> Result<u64, StorageError> {
        let conn = self.conn.lock();

        let affected = conn.execute(
            "DELETE FROM cache WHERE datetime(expires_at) <= datetime('now')",
            [],
        )
        .map_err(|e| StorageError::WriteError(e.to_string()))?;

        Ok(affected as u64)
    }

    // =========================================================================
    // Webhook Operations
    // =========================================================================

    pub fn save_webhook(&self, webhook: &StoredWebhook) -> Result<(), StorageError> {
        let conn = self.conn.lock();

        conn.execute(
            r#"
            INSERT INTO webhooks (webhook_id, url, events, secret, is_active, created_at)
            VALUES (?1, ?2, ?3, ?4, ?5, ?6)
            ON CONFLICT(webhook_id) DO UPDATE SET
                url = ?2,
                events = ?3,
                secret = ?4,
                is_active = ?5
            "#,
            rusqlite::params![
                webhook.webhook_id,
                webhook.url,
                webhook.events,
                webhook.secret,
                webhook.is_active,
                webhook.created_at,
            ],
        )
        .map_err(|e| StorageError::WriteError(e.to_string()))?;

        Ok(())
    }

    pub fn get_active_webhooks(&self) -> Result<Vec<StoredWebhook>, StorageError> {
        let conn = self.conn.lock();

        let mut stmt = conn
            .prepare("SELECT * FROM webhooks WHERE is_active = 1")
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        let rows = stmt
            .query_map([], |row| {
                Ok(StoredWebhook {
                    webhook_id: row.get(0)?,
                    url: row.get(1)?,
                    events: row.get(2)?,
                    secret: row.get(3)?,
                    is_active: row.get(4)?,
                    created_at: row.get(5)?,
                    last_triggered: row.get(6)?,
                    failure_count: row.get(7)?,
                })
            })
            .map_err(|e| StorageError::QueryError(e.to_string()))?;

        let mut webhooks = Vec::new();
        for row in rows {
            webhooks.push(row.map_err(|e| StorageError::QueryError(e.to_string()))?);
        }

        Ok(webhooks)
    }

    pub fn update_webhook_status(&self, webhook_id: &str, success: bool) -> Result<(), StorageError> {
        let conn = self.conn.lock();

        if success {
            conn.execute(
                "UPDATE webhooks SET last_triggered = CURRENT_TIMESTAMP, failure_count = 0 WHERE webhook_id = ?",
                [webhook_id],
            )
        } else {
            conn.execute(
                "UPDATE webhooks SET failure_count = failure_count + 1 WHERE webhook_id = ?",
                [webhook_id],
            )
        }
        .map_err(|e| StorageError::WriteError(e.to_string()))?;

        Ok(())
    }

    pub fn disable_failing_webhooks(&self, max_failures: u32) -> Result<u64, StorageError> {
        let conn = self.conn.lock();

        let affected = conn.execute(
            "UPDATE webhooks SET is_active = 0 WHERE failure_count >= ?",
            [max_failures],
        )
        .map_err(|e| StorageError::WriteError(e.to_string()))?;

        Ok(affected as u64)
    }
}

// =========================================================================
// Data Types
// =========================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredQuota {
    pub user_id: String,
    pub daily_requests: u64,
    pub monthly_requests: u64,
    pub daily_tokens: u64,
    pub monthly_tokens: u64,
    pub daily_reset: String,
    pub monthly_reset: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredSession {
    pub session_id: String,
    pub user_id: Option<String>,
    pub tool: Option<String>,
    pub created_at: String,
    pub last_active: String,
    pub expires_at: Option<String>,
    pub metadata: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLogEntry {
    pub id: Option<i64>,
    pub timestamp: String,
    pub user_id: Option<String>,
    pub action: String,
    pub tool: Option<String>,
    pub prompt_hash: Option<String>,
    pub response_tokens: Option<u64>,
    pub latency_ms: Option<u64>,
    pub success: bool,
    pub error_message: Option<String>,
    pub ip_address: Option<String>,
    pub metadata: Option<String>,
}

impl AuditLogEntry {
    pub fn new(action: &str) -> Self {
        Self {
            id: None,
            timestamp: Utc::now().to_rfc3339(),
            user_id: None,
            action: action.to_string(),
            tool: None,
            prompt_hash: None,
            response_tokens: None,
            latency_ms: None,
            success: true,
            error_message: None,
            ip_address: None,
            metadata: None,
        }
    }

    pub fn with_user(mut self, user_id: &str) -> Self {
        self.user_id = Some(user_id.to_string());
        self
    }

    pub fn with_tool(mut self, tool: Tool) -> Self {
        self.tool = Some(tool.as_str().to_string());
        self
    }

    pub fn with_latency(mut self, latency_ms: u64) -> Self {
        self.latency_ms = Some(latency_ms);
        self
    }

    pub fn with_error(mut self, message: &str) -> Self {
        self.success = false;
        self.error_message = Some(message.to_string());
        self
    }

    pub fn with_ip(mut self, ip: &str) -> Self {
        self.ip_address = Some(ip.to_string());
        self
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredApiKey {
    pub key_id: String,
    pub tool: String,
    pub encrypted_key: Vec<u8>,
    pub created_at: String,
    pub last_used: Option<String>,
    pub is_active: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CachedResponse {
    pub cache_key: String,
    pub tool: String,
    pub prompt_hash: String,
    pub response: String,
    pub tokens: Option<u64>,
    pub created_at: String,
    pub expires_at: String,
    pub hit_count: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredWebhook {
    pub webhook_id: String,
    pub url: String,
    pub events: String, // Comma-separated event types
    pub secret: Option<String>,
    pub is_active: bool,
    pub created_at: String,
    pub last_triggered: Option<String>,
    pub failure_count: u32,
}

// =========================================================================
// Errors
// =========================================================================

#[derive(Debug, Clone)]
pub enum StorageError {
    ConnectionFailed(String),
    SchemaError(String),
    QueryError(String),
    WriteError(String),
    NotFound,
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConnectionFailed(msg) => write!(f, "Database connection failed: {}", msg),
            Self::SchemaError(msg) => write!(f, "Schema initialization failed: {}", msg),
            Self::QueryError(msg) => write!(f, "Query error: {}", msg),
            Self::WriteError(msg) => write!(f, "Write error: {}", msg),
            Self::NotFound => write!(f, "Record not found"),
        }
    }
}

impl std::error::Error for StorageError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_database_creation() {
        let db = Database::open_in_memory().unwrap();
        assert!(db.get_quota("test_user").unwrap().is_none());
    }

    #[test]
    fn test_quota_operations() {
        let db = Database::open_in_memory().unwrap();

        // Increment quota
        db.increment_quota("user1", 1, 100).unwrap();
        db.increment_quota("user1", 1, 50).unwrap();

        let quota = db.get_quota("user1").unwrap().unwrap();
        assert_eq!(quota.daily_requests, 2);
        assert_eq!(quota.daily_tokens, 150);
    }

    #[test]
    fn test_audit_log() {
        let db = Database::open_in_memory().unwrap();

        let entry = AuditLogEntry::new("prompt")
            .with_user("user1")
            .with_tool(Tool::Claude)
            .with_latency(150);

        let id = db.log_audit(&entry).unwrap();
        assert!(id > 0);

        let logs = db.get_audit_logs(10, 0).unwrap();
        assert_eq!(logs.len(), 1);
        assert_eq!(logs[0].action, "prompt");
    }

    #[test]
    fn test_session_operations() {
        let db = Database::open_in_memory().unwrap();

        let session = StoredSession {
            session_id: "sess1".to_string(),
            user_id: Some("user1".to_string()),
            tool: Some("claude".to_string()),
            created_at: Utc::now().to_rfc3339(),
            last_active: Utc::now().to_rfc3339(),
            expires_at: None,
            metadata: None,
        };

        db.save_session(&session).unwrap();

        let loaded = db.get_session("sess1").unwrap().unwrap();
        assert_eq!(loaded.user_id, Some("user1".to_string()));
    }
}
