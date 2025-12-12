//! Usage statistics storage and retrieval

use std::path::Path;
use std::sync::Mutex;
use rusqlite::{Connection, params};
use chrono::{DateTime, Utc};
use uuid::Uuid;
use polyglot_common::{Tool, ToolUsage};
use super::UsageError;

pub struct UsageTracker {
    conn: Mutex<Connection>,
}

impl UsageTracker {
    pub fn new(db_path: &Path) -> Result<Self, UsageError> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let conn = Connection::open(db_path)?;
        let tracker = Self { conn: Mutex::new(conn) };
        tracker.init_schema()?;
        Ok(tracker)
    }

    fn init_schema(&self) -> Result<(), UsageError> {
        let conn = self.conn.lock().map_err(|_| UsageError::LockError)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS tool_usage (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                tool TEXT NOT NULL,
                user_id TEXT,
                session_id TEXT,
                timestamp TEXT NOT NULL,
                tokens_used INTEGER DEFAULT 0,
                request_type TEXT,
                success INTEGER NOT NULL,
                error_message TEXT,
                duration_ms INTEGER
            );

            CREATE INDEX IF NOT EXISTS idx_usage_tool ON tool_usage(tool);
            CREATE INDEX IF NOT EXISTS idx_usage_user ON tool_usage(user_id);
            CREATE INDEX IF NOT EXISTS idx_usage_timestamp ON tool_usage(timestamp);

            CREATE TABLE IF NOT EXISTS daily_stats (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                date TEXT NOT NULL,
                tool TEXT NOT NULL,
                total_requests INTEGER DEFAULT 0,
                total_tokens INTEGER DEFAULT 0,
                total_errors INTEGER DEFAULT 0,
                rate_limit_hits INTEGER DEFAULT 0,
                UNIQUE(date, tool)
            );

            CREATE INDEX IF NOT EXISTS idx_daily_date ON daily_stats(date);
            "
        )?;
        Ok(())
    }

    pub fn record_usage(
        &self,
        tool: Tool,
        user_id: Option<Uuid>,
        session_id: Option<Uuid>,
        tokens: u64,
        success: bool,
        error_message: Option<&str>,
        duration_ms: u64,
    ) -> Result<(), UsageError> {
        let now = Utc::now();
        let date = now.format("%Y-%m-%d").to_string();
        let conn = self.conn.lock().map_err(|_| UsageError::LockError)?;

        conn.execute(
            "INSERT INTO tool_usage (tool, user_id, session_id, timestamp, tokens_used, success, error_message, duration_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                tool.as_str(),
                user_id.map(|u| u.to_string()),
                session_id.map(|s| s.to_string()),
                now.to_rfc3339(),
                tokens as i64,
                success as i32,
                error_message,
                duration_ms as i64,
            ],
        )?;

        conn.execute(
            "INSERT INTO daily_stats (date, tool, total_requests, total_tokens, total_errors)
             VALUES (?1, ?2, 1, ?3, ?4)
             ON CONFLICT(date, tool) DO UPDATE SET
                total_requests = total_requests + 1,
                total_tokens = total_tokens + ?3,
                total_errors = total_errors + ?4",
            params![
                date,
                tool.as_str(),
                tokens as i64,
                if success { 0 } else { 1 },
            ],
        )?;

        Ok(())
    }

    pub fn record_rate_limit(&self, tool: Tool) -> Result<(), UsageError> {
        let date = Utc::now().format("%Y-%m-%d").to_string();
        let conn = self.conn.lock().map_err(|_| UsageError::LockError)?;

        conn.execute(
            "INSERT INTO daily_stats (date, tool, rate_limit_hits)
             VALUES (?1, ?2, 1)
             ON CONFLICT(date, tool) DO UPDATE SET
                rate_limit_hits = rate_limit_hits + 1",
            params![date, tool.as_str()],
        )?;

        Ok(())
    }

    pub fn get_all_stats(&self) -> Result<Vec<ToolUsage>, UsageError> {
        let mut stats = Vec::new();

        for tool in Tool::all() {
            let usage = self.get_tool_stats(*tool)?;
            stats.push(usage);
        }

        Ok(stats)
    }

    pub fn get_tool_stats(&self, tool: Tool) -> Result<ToolUsage, UsageError> {
        let conn = self.conn.lock().map_err(|_| UsageError::LockError)?;
        let mut stmt = conn.prepare(
            "SELECT
                COUNT(*) as requests,
                COALESCE(SUM(tokens_used), 0) as tokens,
                COALESCE(SUM(CASE WHEN success = 0 THEN 1 ELSE 0 END), 0) as errors,
                MAX(timestamp) as last_used
             FROM tool_usage
             WHERE tool = ?1"
        )?;

        let (requests, tokens, errors, last_used): (i64, i64, i64, Option<String>) =
            stmt.query_row([tool.as_str()], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })?;

        let rate_limits: i64 = conn.query_row(
            "SELECT COALESCE(SUM(rate_limit_hits), 0) FROM daily_stats WHERE tool = ?1",
            [tool.as_str()],
            |row| row.get(0),
        )?;

        Ok(ToolUsage {
            tool,
            requests: requests as u64,
            tokens_used: tokens as u64,
            errors: errors as u64,
            rate_limit_hits: rate_limits as u64,
            last_used: last_used.and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                .map(|dt| dt.with_timezone(&Utc)),
            is_available: true,
        })
    }

    pub fn get_user_stats(&self, user_id: Uuid) -> Result<Vec<ToolUsage>, UsageError> {
        let mut stats = Vec::new();
        let conn = self.conn.lock().map_err(|_| UsageError::LockError)?;

        for tool in Tool::all() {
            let mut stmt = conn.prepare(
                "SELECT
                    COUNT(*) as requests,
                    COALESCE(SUM(tokens_used), 0) as tokens,
                    COALESCE(SUM(CASE WHEN success = 0 THEN 1 ELSE 0 END), 0) as errors,
                    MAX(timestamp) as last_used
                 FROM tool_usage
                 WHERE tool = ?1 AND user_id = ?2"
            )?;

            let (requests, tokens, errors, last_used): (i64, i64, i64, Option<String>) =
                stmt.query_row(params![tool.as_str(), user_id.to_string()], |row| {
                    Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
                })?;

            stats.push(ToolUsage {
                tool: *tool,
                requests: requests as u64,
                tokens_used: tokens as u64,
                errors: errors as u64,
                rate_limit_hits: 0,
                last_used: last_used.and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc)),
                is_available: true,
            });
        }

        Ok(stats)
    }

    pub fn get_daily_stats(
        &self,
        start_date: &str,
        end_date: &str,
    ) -> Result<Vec<DailyStats>, UsageError> {
        let conn = self.conn.lock().map_err(|_| UsageError::LockError)?;
        let mut stmt = conn.prepare(
            "SELECT date, tool, total_requests, total_tokens, total_errors, rate_limit_hits
             FROM daily_stats
             WHERE date >= ?1 AND date <= ?2
             ORDER BY date DESC, tool"
        )?;

        let stats = stmt.query_map([start_date, end_date], |row| {
            Ok(DailyStats {
                date: row.get(0)?,
                tool: row.get::<_, String>(1)?.parse().unwrap_or(Tool::Claude),
                total_requests: row.get::<_, i64>(2)? as u64,
                total_tokens: row.get::<_, i64>(3)? as u64,
                total_errors: row.get::<_, i64>(4)? as u64,
                rate_limit_hits: row.get::<_, i64>(5)? as u64,
            })
        })?.collect::<Result<Vec<_>, _>>()?;

        Ok(stats)
    }

    pub fn cleanup_old_records(&self, days: u32) -> Result<u64, UsageError> {
        let cutoff = Utc::now() - chrono::Duration::days(days as i64);
        let cutoff_str = cutoff.to_rfc3339();
        let conn = self.conn.lock().map_err(|_| UsageError::LockError)?;

        let deleted = conn.execute(
            "DELETE FROM tool_usage WHERE timestamp < ?1",
            [cutoff_str],
        )?;

        Ok(deleted as u64)
    }
}

#[derive(Debug, Clone)]
pub struct DailyStats {
    pub date: String,
    pub tool: Tool,
    pub total_requests: u64,
    pub total_tokens: u64,
    pub total_errors: u64,
    pub rate_limit_hits: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_db() -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("polyglot_usage_test_{}.db", Uuid::new_v4()));
        path
    }

    #[test]
    fn test_usage_tracking() {
        let db_path = temp_db();
        let tracker = UsageTracker::new(&db_path).unwrap();

        tracker.record_usage(
            Tool::Claude,
            None,
            None,
            100,
            true,
            None,
            1000,
        ).unwrap();

        tracker.record_usage(
            Tool::Claude,
            None,
            None,
            50,
            false,
            Some("test error"),
            500,
        ).unwrap();

        let stats = tracker.get_tool_stats(Tool::Claude).unwrap();
        assert_eq!(stats.requests, 2);
        assert_eq!(stats.tokens_used, 150);
        assert_eq!(stats.errors, 1);

        std::fs::remove_file(&db_path).ok();
    }
}
