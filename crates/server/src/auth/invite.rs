//! Invite code management for user registration

use std::collections::HashMap;
use std::sync::Arc;
use parking_lot::RwLock;
use chrono::{DateTime, Utc, Duration};
use serde::{Deserialize, Serialize};
use super::AuthError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteCode {
    pub code: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub max_uses: u32,
    pub uses: u32,
    pub is_admin: bool,
    pub created_by: Option<String>,
}

impl InviteCode {
    pub fn is_valid(&self) -> bool {
        self.expires_at > Utc::now() && self.uses < self.max_uses
    }

    pub fn remaining_uses(&self) -> u32 {
        if self.uses >= self.max_uses {
            0
        } else {
            self.max_uses - self.uses
        }
    }
}

pub struct InviteManager {
    invites: Arc<RwLock<HashMap<String, InviteCode>>>,
}

impl InviteManager {
    pub fn new() -> Self {
        Self {
            invites: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn generate_invite(
        &self,
        expiry_hours: u32,
        max_uses: u32,
        is_admin: bool,
        created_by: Option<String>,
    ) -> InviteCode {
        let code = Self::generate_code();
        let now = Utc::now();
        let expires_at = now + Duration::hours(expiry_hours as i64);

        let invite = InviteCode {
            code: code.clone(),
            created_at: now,
            expires_at,
            max_uses,
            uses: 0,
            is_admin,
            created_by,
        };

        self.invites.write().insert(code, invite.clone());
        invite
    }

    fn generate_code() -> String {
        use rand::Rng;
        const CHARSET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
        let mut rng = rand::rng();
        (0..8)
            .map(|_| {
                let idx = rng.random_range(0..CHARSET.len());
                CHARSET[idx] as char
            })
            .collect()
    }

    pub fn use_invite(&self, code: &str) -> Result<InviteCode, AuthError> {
        let mut invites = self.invites.write();
        let invite = invites.get_mut(code).ok_or(AuthError::InvalidCredentials)?;

        if !invite.is_valid() {
            return Err(AuthError::InvalidCredentials);
        }

        invite.uses += 1;
        let result = invite.clone();

        if invite.uses >= invite.max_uses {
            drop(invites);
            self.invites.write().remove(code);
        }

        Ok(result)
    }

    pub fn validate_invite(&self, code: &str) -> Result<InviteCode, AuthError> {
        let invites = self.invites.read();
        let invite = invites.get(code).ok_or(AuthError::InvalidCredentials)?;

        if !invite.is_valid() {
            return Err(AuthError::InvalidCredentials);
        }

        Ok(invite.clone())
    }

    pub fn list_invites(&self) -> Vec<InviteCode> {
        self.invites
            .read()
            .values()
            .filter(|i| i.is_valid())
            .cloned()
            .collect()
    }

    pub fn revoke_invite(&self, code: &str) -> Result<(), AuthError> {
        if self.invites.write().remove(code).is_some() {
            Ok(())
        } else {
            Err(AuthError::InvalidCredentials)
        }
    }

    pub fn cleanup_expired(&self) {
        let now = Utc::now();
        self.invites.write().retain(|_, invite| invite.expires_at > now);
    }

    pub fn active_count(&self) -> usize {
        self.invites.read().values().filter(|i| i.is_valid()).count()
    }
}

impl Default for InviteManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invite_lifecycle() {
        let manager = InviteManager::new();

        let invite = manager.generate_invite(24, 2, false, Some("admin".to_string()));
        assert!(invite.is_valid());
        assert_eq!(invite.remaining_uses(), 2);

        let validated = manager.validate_invite(&invite.code).unwrap();
        assert_eq!(validated.uses, 0);

        let used = manager.use_invite(&invite.code).unwrap();
        assert_eq!(used.uses, 1);

        let used2 = manager.use_invite(&invite.code).unwrap();
        assert_eq!(used2.uses, 2);

        assert!(manager.use_invite(&invite.code).is_err());
    }

    #[test]
    fn test_revoke_invite() {
        let manager = InviteManager::new();
        let invite = manager.generate_invite(24, 1, false, None);

        manager.revoke_invite(&invite.code).unwrap();
        assert!(manager.validate_invite(&invite.code).is_err());
    }
}
