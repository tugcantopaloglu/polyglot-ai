//! User management

use std::path::Path;
use std::sync::Mutex;
use rusqlite::{Connection, params};
use uuid::Uuid;
use chrono::Utc;
use polyglot_common::User;
use super::AuthError;

pub struct UserManager {
    conn: Mutex<Connection>,
}

impl UserManager {
    pub fn new(db_path: &Path) -> Result<Self, AuthError> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent).ok();
        }

        let conn = Connection::open(db_path)?;
        let manager = Self { conn: Mutex::new(conn) };
        manager.init_schema()?;
        Ok(manager)
    }

    fn init_schema(&self) -> Result<(), AuthError> {
        let conn = self.conn.lock().map_err(|_| AuthError::LockError)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS users (
                id TEXT PRIMARY KEY,
                username TEXT UNIQUE NOT NULL,
                cert_fingerprint TEXT,
                created_at TEXT NOT NULL,
                last_login TEXT,
                is_admin INTEGER NOT NULL DEFAULT 0
            );

            CREATE INDEX IF NOT EXISTS idx_users_username ON users(username);
            CREATE INDEX IF NOT EXISTS idx_users_fingerprint ON users(cert_fingerprint);
            "
        )?;
        Ok(())
    }

    pub fn create_user(&self, username: &str, is_admin: bool) -> Result<User, AuthError> {
        if self.get_user_by_username(username).is_ok() {
            return Err(AuthError::UserExists);
        }

        let user = User::new(username.to_string(), is_admin);
        let conn = self.conn.lock().map_err(|_| AuthError::LockError)?;

        conn.execute(
            "INSERT INTO users (id, username, created_at, is_admin) VALUES (?1, ?2, ?3, ?4)",
            params![
                user.id.to_string(),
                user.username,
                user.created_at.to_rfc3339(),
                user.is_admin as i32
            ],
        )?;

        Ok(user)
    }

    pub fn get_user(&self, id: Uuid) -> Result<User, AuthError> {
        let conn = self.conn.lock().map_err(|_| AuthError::LockError)?;
        let mut stmt = conn.prepare(
            "SELECT id, username, created_at, last_login, is_admin FROM users WHERE id = ?1"
        )?;

        let user = stmt.query_row([id.to_string()], |row| {
            Ok(User {
                id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                username: row.get(1)?,
                created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(2)?)
                    .unwrap()
                    .with_timezone(&Utc),
                last_login: row.get::<_, Option<String>>(3)?
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc)),
                is_admin: row.get::<_, i32>(4)? != 0,
            })
        }).map_err(|_| AuthError::UserNotFound)?;

        Ok(user)
    }

    pub fn get_user_by_username(&self, username: &str) -> Result<User, AuthError> {
        let conn = self.conn.lock().map_err(|_| AuthError::LockError)?;
        let mut stmt = conn.prepare(
            "SELECT id, username, created_at, last_login, is_admin FROM users WHERE username = ?1"
        )?;

        let user = stmt.query_row([username], |row| {
            Ok(User {
                id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                username: row.get(1)?,
                created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(2)?)
                    .unwrap()
                    .with_timezone(&Utc),
                last_login: row.get::<_, Option<String>>(3)?
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc)),
                is_admin: row.get::<_, i32>(4)? != 0,
            })
        }).map_err(|_| AuthError::UserNotFound)?;

        Ok(user)
    }

    pub fn get_user_by_fingerprint(&self, fingerprint: &str) -> Result<User, AuthError> {
        let conn = self.conn.lock().map_err(|_| AuthError::LockError)?;
        let mut stmt = conn.prepare(
            "SELECT id, username, created_at, last_login, is_admin FROM users WHERE cert_fingerprint = ?1"
        )?;

        let user = stmt.query_row([fingerprint], |row| {
            Ok(User {
                id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                username: row.get(1)?,
                created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(2)?)
                    .unwrap()
                    .with_timezone(&Utc),
                last_login: row.get::<_, Option<String>>(3)?
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc)),
                is_admin: row.get::<_, i32>(4)? != 0,
            })
        }).map_err(|_| AuthError::UserNotFound)?;

        Ok(user)
    }

    pub fn set_user_fingerprint(&self, user_id: Uuid, fingerprint: &str) -> Result<(), AuthError> {
        let conn = self.conn.lock().map_err(|_| AuthError::LockError)?;
        let rows = conn.execute(
            "UPDATE users SET cert_fingerprint = ?1 WHERE id = ?2",
            params![fingerprint, user_id.to_string()],
        )?;

        if rows == 0 {
            return Err(AuthError::UserNotFound);
        }

        Ok(())
    }

    pub fn update_last_login(&self, user_id: Uuid) -> Result<(), AuthError> {
        let now = Utc::now().to_rfc3339();
        let conn = self.conn.lock().map_err(|_| AuthError::LockError)?;
        let rows = conn.execute(
            "UPDATE users SET last_login = ?1 WHERE id = ?2",
            params![now, user_id.to_string()],
        )?;

        if rows == 0 {
            return Err(AuthError::UserNotFound);
        }

        Ok(())
    }

    pub fn delete_user(&self, user_id: Uuid) -> Result<(), AuthError> {
        let conn = self.conn.lock().map_err(|_| AuthError::LockError)?;
        let rows = conn.execute(
            "DELETE FROM users WHERE id = ?1",
            params![user_id.to_string()],
        )?;

        if rows == 0 {
            return Err(AuthError::UserNotFound);
        }

        Ok(())
    }

    pub fn list_users(&self) -> Result<Vec<User>, AuthError> {
        let conn = self.conn.lock().map_err(|_| AuthError::LockError)?;
        let mut stmt = conn.prepare(
            "SELECT id, username, created_at, last_login, is_admin FROM users ORDER BY username"
        )?;

        let users = stmt.query_map([], |row| {
            Ok(User {
                id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap(),
                username: row.get(1)?,
                created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(2)?)
                    .unwrap()
                    .with_timezone(&Utc),
                last_login: row.get::<_, Option<String>>(3)?
                    .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                    .map(|dt| dt.with_timezone(&Utc)),
                is_admin: row.get::<_, i32>(4)? != 0,
            })
        })?.collect::<Result<Vec<_>, _>>()?;

        Ok(users)
    }

    pub fn user_count(&self) -> Result<u32, AuthError> {
        let conn = self.conn.lock().map_err(|_| AuthError::LockError)?;
        let count: u32 = conn.query_row(
            "SELECT COUNT(*) FROM users",
            [],
            |row| row.get(0),
        )?;
        Ok(count)
    }

    pub fn is_single_user_mode(&self) -> Result<bool, AuthError> {
        let count = self.user_count()?;
        Ok(count <= 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_db() -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!("polyglot_test_{}.db", Uuid::new_v4()));
        path
    }

    #[test]
    fn test_user_crud() {
        let db_path = temp_db();
        let manager = UserManager::new(&db_path).unwrap();

        let user = manager.create_user("testuser", false).unwrap();
        assert_eq!(user.username, "testuser");
        assert!(!user.is_admin);

        let fetched = manager.get_user(user.id).unwrap();
        assert_eq!(fetched.username, "testuser");

        let fetched2 = manager.get_user_by_username("testuser").unwrap();
        assert_eq!(fetched2.id, user.id);

        manager.set_user_fingerprint(user.id, "abc123").unwrap();
        let by_fp = manager.get_user_by_fingerprint("abc123").unwrap();
        assert_eq!(by_fp.id, user.id);

        manager.update_last_login(user.id).unwrap();
        let updated = manager.get_user(user.id).unwrap();
        assert!(updated.last_login.is_some());

        let users = manager.list_users().unwrap();
        assert_eq!(users.len(), 1);

        manager.delete_user(user.id).unwrap();
        assert!(manager.get_user(user.id).is_err());

        std::fs::remove_file(&db_path).ok();
    }
}
