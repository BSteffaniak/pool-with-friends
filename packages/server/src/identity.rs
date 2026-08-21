//! Verified external identities and hash-only session custody.

use std::collections::BTreeMap;

use crate::AccountId;
use thiserror::Error;

/// Verified Google issuer and subject identity key.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct GoogleIdentity {
    issuer: String,
    subject: String,
}

impl GoogleIdentity {
    /// Creates an identity only from claims already verified by the OIDC adapter.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError::InvalidClaim`] for empty, oversized, or
    /// control-containing issuer/subject values.
    pub fn verified(issuer: &str, subject: &str) -> Result<Self, IdentityError> {
        if !valid_claim(issuer, 256) || !valid_claim(subject, 256) {
            return Err(IdentityError::InvalidClaim);
        }
        Ok(Self {
            issuer: issuer.to_owned(),
            subject: subject.to_owned(),
        })
    }

    /// Returns the verified issuer identifier.
    #[must_use]
    pub fn issuer(&self) -> &str {
        &self.issuer
    }

    /// Returns the verified provider subject identifier.
    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }
}

/// Fixed-size session-token hash. Raw session tokens never enter durable state.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct SessionTokenHash([u8; 32]);

impl SessionTokenHash {
    /// Creates a hash produced by the cryptographic secret-custody adapter.
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns the exact hash bytes.
    #[must_use]
    pub const fn bytes(self) -> [u8; 32] {
        self.0
    }
}

/// Durable hash-only session record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Session {
    /// Account authenticated by the session.
    pub account: AccountId,
    /// Absolute expiration in server-clock milliseconds.
    pub expires_at: u64,
    /// Absolute last-use time for revocation/audit behavior.
    pub last_used_at: u64,
}

/// Durable identity/session transition.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IdentityTransition {
    /// Verified external identity was linked to an account.
    IdentityLinked {
        identity: GoogleIdentity,
        account: AccountId,
    },
    /// Hash-only session was created.
    SessionCreated {
        token_hash: SessionTokenHash,
        session: Session,
    },
    /// Session was revoked.
    SessionRevoked { token_hash: SessionTokenHash },
}

/// Durable identity/session adapter.
pub trait IdentityJournal {
    /// Commits a transition before publication.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityJournalError`] when durability cannot be established.
    fn commit(&mut self, transition: IdentityTransition) -> Result<(), IdentityJournalError>;
}

/// Secret-safe identity journal error.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("identity journal commit failed")]
pub struct IdentityJournalError;

/// Verified identity and hash-only session service.
pub struct IdentityService<J> {
    journal: J,
    identities: BTreeMap<GoogleIdentity, AccountId>,
    sessions: BTreeMap<SessionTokenHash, Session>,
}

impl<J: IdentityJournal> IdentityService<J> {
    /// Creates an empty identity service.
    #[must_use]
    pub const fn new(journal: J) -> Self {
        Self {
            journal,
            identities: BTreeMap::new(),
            sessions: BTreeMap::new(),
        }
    }

    /// Links a verified Google identity to one stable account.
    ///
    /// Re-linking the same identity to the same account is idempotent.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError::IdentityConflict`] for a different account, or
    /// a journal error when durability fails.
    pub fn link_google(
        &mut self,
        identity: GoogleIdentity,
        account: AccountId,
    ) -> Result<(), IdentityError> {
        if let Some(existing) = self.identities.get(&identity) {
            return if *existing == account {
                Ok(())
            } else {
                Err(IdentityError::IdentityConflict)
            };
        }
        self.journal.commit(IdentityTransition::IdentityLinked {
            identity: identity.clone(),
            account,
        })?;
        self.identities.insert(identity, account);
        Ok(())
    }

    /// Resolves a verified external identity.
    #[must_use]
    pub fn account_for_google(&self, identity: &GoogleIdentity) -> Option<AccountId> {
        self.identities.get(identity).copied()
    }

    /// Creates an expiring session indexed only by token hash.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError`] for non-future expiry, duplicate hash, or journal failure.
    pub fn create_session(
        &mut self,
        token_hash: SessionTokenHash,
        account: AccountId,
        expires_at: u64,
        now: u64,
    ) -> Result<Session, IdentityError> {
        if expires_at <= now {
            return Err(IdentityError::InvalidExpiration);
        }
        if self.sessions.contains_key(&token_hash) {
            return Err(IdentityError::SessionConflict);
        }
        let session = Session {
            account,
            expires_at,
            last_used_at: now,
        };
        self.journal.commit(IdentityTransition::SessionCreated {
            token_hash,
            session,
        })?;
        self.sessions.insert(token_hash, session);
        Ok(session)
    }

    /// Resolves an unexpired session hash and updates in-memory last-use time.
    #[must_use]
    pub fn resolve_session(&mut self, token_hash: SessionTokenHash, now: u64) -> Option<AccountId> {
        let session = self.sessions.get_mut(&token_hash)?;
        if session.expires_at <= now {
            return None;
        }
        session.last_used_at = now;
        Some(session.account)
    }

    /// Revokes a session hash.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError::SessionNotFound`] or journal failure.
    pub fn revoke_session(&mut self, token_hash: SessionTokenHash) -> Result<(), IdentityError> {
        if !self.sessions.contains_key(&token_hash) {
            return Err(IdentityError::SessionNotFound);
        }
        self.journal
            .commit(IdentityTransition::SessionRevoked { token_hash })?;
        self.sessions.remove(&token_hash);
        Ok(())
    }
}

/// Identity/session rejection.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum IdentityError {
    /// A verified claim is structurally invalid.
    #[error("verified identity claim is invalid")]
    InvalidClaim,
    /// Identity is linked to another account.
    #[error("external identity conflict")]
    IdentityConflict,
    /// Session expiry is not in the future.
    #[error("session expiration is invalid")]
    InvalidExpiration,
    /// Session hash already exists.
    #[error("session already exists")]
    SessionConflict,
    /// Session hash does not exist.
    #[error("session not found")]
    SessionNotFound,
    /// Durable transition failed.
    #[error(transparent)]
    Journal(#[from] IdentityJournalError),
}

fn valid_claim(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct MemoryJournal(Vec<IdentityTransition>);
    impl IdentityJournal for MemoryJournal {
        fn commit(&mut self, transition: IdentityTransition) -> Result<(), IdentityJournalError> {
            self.0.push(transition);
            Ok(())
        }
    }

    #[test]
    fn verified_google_identity_is_stable_and_conflict_safe() {
        let mut service = IdentityService::new(MemoryJournal::default());
        let identity =
            GoogleIdentity::verified("https://accounts.google.com", "subject-1").unwrap();
        service
            .link_google(identity.clone(), AccountId::new(1))
            .unwrap();
        service
            .link_google(identity.clone(), AccountId::new(1))
            .unwrap();
        assert_eq!(
            service.account_for_google(&identity),
            Some(AccountId::new(1))
        );
        assert_eq!(
            service.link_google(identity, AccountId::new(2)),
            Err(IdentityError::IdentityConflict)
        );
        assert_eq!(service.journal.0.len(), 1);
    }

    #[test]
    fn sessions_are_hash_only_expiring_and_revocable() {
        let mut service = IdentityService::new(MemoryJournal::default());
        let hash = SessionTokenHash::new([9; 32]);
        service
            .create_session(hash, AccountId::new(1), 100, 0)
            .unwrap();
        assert_eq!(service.resolve_session(hash, 50), Some(AccountId::new(1)));
        assert_eq!(service.resolve_session(hash, 100), None);
        service.revoke_session(hash).unwrap();
        assert_eq!(service.resolve_session(hash, 51), None);
    }
}
