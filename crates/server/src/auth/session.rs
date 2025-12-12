//! Session management

use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;
use uuid::Uuid;
use chrono::{Utc, Duration};
use jsonwebtoken::{encode, decode, Header, Validation, EncodingKey, DecodingKey};
use serde::{Deserialize, Serialize};
use polyglot_common::{Session, Tool, SyncMode};
use super::AuthError;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: String,
    pub session_id: String,
    pub exp: i64,
    pub iat: i64,
}

pub struct SessionManager {
    sessions: Arc<RwLock<HashMap<Uuid, Session>>>,
    jwt_secret: String,
    expiry_hours: u32,
}

impl SessionManager {
    pub fn new(jwt_secret: String, expiry_hours: u32) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
            jwt_secret,
            expiry_hours,
        }
    }

    pub fn create_session(&self, user_id: Uuid) -> Result<(Session, String), AuthError> {
        let now = Utc::now();
        let expiry = now + Duration::hours(self.expiry_hours as i64);

        let session = Session {
            id: Uuid::new_v4(),
            user_id,
            created_at: now,
            expires_at: expiry,
            current_tool: None,
            sync_mode: SyncMode::default(),
        };

        let token = self.generate_token(&session)?;

        self.sessions.write().insert(session.id, session.clone());

        Ok((session, token))
    }

    fn generate_token(&self, session: &Session) -> Result<String, AuthError> {
        let claims = Claims {
            sub: session.user_id.to_string(),
            session_id: session.id.to_string(),
            exp: session.expires_at.timestamp(),
            iat: session.created_at.timestamp(),
        };

        let token = encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(self.jwt_secret.as_bytes()),
        )?;

        Ok(token)
    }

    pub fn validate_token(&self, token: &str) -> Result<Session, AuthError> {
        let token_data = decode::<Claims>(
            token,
            &DecodingKey::from_secret(self.jwt_secret.as_bytes()),
            &Validation::default(),
        )?;

        let session_id = Uuid::parse_str(&token_data.claims.session_id)
            .map_err(|_| AuthError::SessionNotFound)?;

        self.get_session(session_id)
    }

    pub fn get_session(&self, session_id: Uuid) -> Result<Session, AuthError> {
        let sessions = self.sessions.read();
        let session = sessions.get(&session_id).ok_or(AuthError::SessionNotFound)?;

        if session.expires_at < Utc::now() {
            drop(sessions);
            self.sessions.write().remove(&session_id);
            return Err(AuthError::SessionExpired);
        }

        Ok(session.clone())
    }

    pub fn set_current_tool(&self, session_id: Uuid, tool: Tool) -> Result<(), AuthError> {
        let mut sessions = self.sessions.write();
        let session = sessions.get_mut(&session_id).ok_or(AuthError::SessionNotFound)?;
        session.current_tool = Some(tool);
        Ok(())
    }

    pub fn set_sync_mode(&self, session_id: Uuid, mode: SyncMode) -> Result<(), AuthError> {
        let mut sessions = self.sessions.write();
        let session = sessions.get_mut(&session_id).ok_or(AuthError::SessionNotFound)?;
        session.sync_mode = mode;
        Ok(())
    }

    pub fn remove_session(&self, session_id: Uuid) {
        self.sessions.write().remove(&session_id);
    }

    pub fn cleanup_expired(&self) {
        let now = Utc::now();
        self.sessions.write().retain(|_, session| session.expires_at > now);
    }

    pub fn active_count(&self) -> usize {
        self.sessions.read().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_lifecycle() {
        let manager = SessionManager::new("test_secret".to_string(), 24);
        let user_id = Uuid::new_v4();

        let (session, token) = manager.create_session(user_id).unwrap();
        assert_eq!(session.user_id, user_id);

        let validated = manager.validate_token(&token).unwrap();
        assert_eq!(validated.id, session.id);

        manager.set_current_tool(session.id, Tool::Claude).unwrap();
        let updated = manager.get_session(session.id).unwrap();
        assert_eq!(updated.current_tool, Some(Tool::Claude));

        manager.remove_session(session.id);
        assert!(manager.get_session(session.id).is_err());
    }
}
