//! Exact-handle challenges and private invitation lifecycle.

use std::collections::BTreeMap;

use crate::{AccountId, LobbyId, Participants};
use thiserror::Error;

/// Stable public handle normalized by the profile boundary.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Handle(String);

impl Handle {
    /// Creates a bounded stable handle.
    ///
    /// # Errors
    ///
    /// Returns [`SocialError::InvalidHandle`] unless the handle has 3–24 ASCII
    /// lowercase letters, digits, or underscores and starts with a letter.
    pub fn new(value: &str) -> Result<Self, SocialError> {
        let bytes = value.as_bytes();
        if !(3..=24).contains(&bytes.len())
            || !bytes[0].is_ascii_lowercase()
            || !bytes
                .iter()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'_')
        {
            return Err(SocialError::InvalidHandle);
        }
        Ok(Self(value.to_owned()))
    }

    /// Returns the exact public handle.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Stable challenge identifier.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ChallengeId(u128);

impl ChallengeId {
    /// Creates a challenge identifier.
    #[must_use]
    pub const fn new(value: u128) -> Self {
        Self(value)
    }
}

/// Stable invitation identifier.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct InvitationId(u128);

impl InvitationId {
    /// Creates an invitation identifier.
    #[must_use]
    pub const fn new(value: u128) -> Self {
        Self(value)
    }
}

/// Fixed-size hash of a secret invitation token.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct InvitationTokenHash([u8; 32]);

impl InvitationTokenHash {
    /// Creates a hash produced by the secret-custody adapter.
    #[must_use]
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

/// Pending exact-handle challenge.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Challenge {
    /// Stable challenge identifier.
    pub id: ChallengeId,
    /// Challenger.
    pub from: AccountId,
    /// Challenged account.
    pub to: AccountId,
}

/// Pending private invitation. Raw tokens never enter this record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Invitation {
    /// Stable invitation identifier.
    pub id: InvitationId,
    /// Invitation creator.
    pub creator: AccountId,
    /// Hash of the opaque token.
    pub token_hash: InvitationTokenHash,
    /// Absolute expiration in server-clock milliseconds.
    pub expires_at: u64,
    /// Whether the invitation was revoked.
    pub revoked: bool,
    /// Lobby created by the one successful redemption.
    pub redeemed_lobby: Option<LobbyId>,
}

/// Atomic durable social transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SocialTransition {
    /// Challenge was created.
    ChallengeCreated(Challenge),
    /// Challenge acceptance created a lobby.
    ChallengeAccepted {
        challenge_id: ChallengeId,
        lobby_id: LobbyId,
    },
    /// Invitation was created with hash-only secret custody.
    InvitationCreated(Invitation),
    /// Invitation was revoked.
    InvitationRevoked { invitation_id: InvitationId },
    /// Invitation redemption created a lobby.
    InvitationRedeemed {
        invitation_id: InvitationId,
        redeemer: AccountId,
        lobby_id: LobbyId,
    },
}

/// Atomic durable social adapter.
pub trait SocialJournal {
    /// Commits a social transition before publication.
    ///
    /// # Errors
    ///
    /// Returns [`SocialJournalError`] when durability cannot be established.
    fn commit(&mut self, transition: SocialTransition) -> Result<(), SocialJournalError>;
}

/// Secret-safe durable social error.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("social journal commit failed")]
pub struct SocialJournalError;

/// Exact-handle challenge and private invitation service.
pub struct SocialService<J> {
    journal: J,
    handles: BTreeMap<Handle, AccountId>,
    challenges: BTreeMap<ChallengeId, Challenge>,
    invitations: BTreeMap<InvitationId, Invitation>,
    invitation_by_hash: BTreeMap<InvitationTokenHash, InvitationId>,
}

impl<J: SocialJournal> SocialService<J> {
    /// Creates an empty social service.
    #[must_use]
    pub const fn new(journal: J) -> Self {
        Self {
            journal,
            handles: BTreeMap::new(),
            challenges: BTreeMap::new(),
            invitations: BTreeMap::new(),
            invitation_by_hash: BTreeMap::new(),
        }
    }

    /// Registers a unique stable handle.
    ///
    /// # Errors
    ///
    /// Returns [`SocialError::HandleTaken`] when already assigned.
    pub fn register_handle(
        &mut self,
        handle: Handle,
        account: AccountId,
    ) -> Result<(), SocialError> {
        match self.handles.entry(handle) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(account);
                Ok(())
            }
            std::collections::btree_map::Entry::Occupied(_) => Err(SocialError::HandleTaken),
        }
    }

    /// Creates a challenge addressed by exact handle.
    ///
    /// # Errors
    ///
    /// Returns [`SocialError`] for unknown handles, self-challenges, duplicate
    /// identifiers, or journal failure.
    pub fn challenge(
        &mut self,
        id: ChallengeId,
        from: AccountId,
        exact_handle: &Handle,
    ) -> Result<Challenge, SocialError> {
        if self.challenges.contains_key(&id) {
            return Err(SocialError::AlreadyExists);
        }
        let to = self
            .handles
            .get(exact_handle)
            .copied()
            .ok_or(SocialError::HandleNotFound)?;
        if from == to {
            return Err(SocialError::SelfChallenge);
        }
        let challenge = Challenge { id, from, to };
        self.journal
            .commit(SocialTransition::ChallengeCreated(challenge))?;
        self.challenges.insert(id, challenge);
        Ok(challenge)
    }

    /// Accepts a challenge and returns the exact lobby membership to create.
    ///
    /// Removal and lobby-creation intent are committed atomically in one social
    /// transition, preventing duplicate acceptance.
    ///
    /// # Errors
    ///
    /// Returns [`SocialError`] for missing challenge, wrong actor, or journal failure.
    pub fn accept_challenge(
        &mut self,
        id: ChallengeId,
        actor: AccountId,
        lobby_id: LobbyId,
    ) -> Result<Participants, SocialError> {
        let challenge = *self.challenges.get(&id).ok_or(SocialError::NotFound)?;
        if challenge.to != actor {
            return Err(SocialError::Unauthorized);
        }
        self.journal.commit(SocialTransition::ChallengeAccepted {
            challenge_id: id,
            lobby_id,
        })?;
        self.challenges.remove(&id);
        Ok(Participants {
            player_one: challenge.from,
            player_two: challenge.to,
        })
    }

    /// Creates an expiring private invitation storing only a token hash.
    ///
    /// # Errors
    ///
    /// Returns [`SocialError`] for duplicate ids/hashes, non-future expiry, or journal failure.
    pub fn create_invitation(
        &mut self,
        id: InvitationId,
        creator: AccountId,
        token_hash: InvitationTokenHash,
        expires_at: u64,
        now: u64,
    ) -> Result<Invitation, SocialError> {
        if expires_at <= now {
            return Err(SocialError::InvalidExpiration);
        }
        if self.invitations.contains_key(&id) || self.invitation_by_hash.contains_key(&token_hash) {
            return Err(SocialError::AlreadyExists);
        }
        let invitation = Invitation {
            id,
            creator,
            token_hash,
            expires_at,
            revoked: false,
            redeemed_lobby: None,
        };
        self.journal
            .commit(SocialTransition::InvitationCreated(invitation))?;
        self.invitations.insert(id, invitation);
        self.invitation_by_hash.insert(token_hash, id);
        Ok(invitation)
    }

    /// Revokes an unused invitation.
    ///
    /// # Errors
    ///
    /// Returns [`SocialError`] for missing, unauthorized, used, or journal failure.
    pub fn revoke_invitation(
        &mut self,
        id: InvitationId,
        actor: AccountId,
    ) -> Result<(), SocialError> {
        let invitation = self
            .invitations
            .get(&id)
            .copied()
            .ok_or(SocialError::NotFound)?;
        if invitation.creator != actor {
            return Err(SocialError::Unauthorized);
        }
        if invitation.redeemed_lobby.is_some() {
            return Err(SocialError::AlreadyUsed);
        }
        self.journal
            .commit(SocialTransition::InvitationRevoked { invitation_id: id })?;
        let stored = self.invitations.get_mut(&id).ok_or(SocialError::NotFound)?;
        stored.revoked = true;
        Ok(())
    }

    /// Redeems an invitation hash exactly once and returns lobby membership.
    ///
    /// # Errors
    ///
    /// Returns [`SocialError`] for unknown, expired, revoked, used, self-redeemed,
    /// or journal failure.
    pub fn redeem_invitation(
        &mut self,
        token_hash: InvitationTokenHash,
        redeemer: AccountId,
        lobby_id: LobbyId,
        now: u64,
    ) -> Result<Participants, SocialError> {
        let id = *self
            .invitation_by_hash
            .get(&token_hash)
            .ok_or(SocialError::NotFound)?;
        let invitation = self
            .invitations
            .get(&id)
            .copied()
            .ok_or(SocialError::NotFound)?;
        if invitation.creator == redeemer {
            return Err(SocialError::SelfInvitation);
        }
        if invitation.revoked {
            return Err(SocialError::Revoked);
        }
        if invitation.expires_at <= now {
            return Err(SocialError::Expired);
        }
        if invitation.redeemed_lobby.is_some() {
            return Err(SocialError::AlreadyUsed);
        }
        self.journal.commit(SocialTransition::InvitationRedeemed {
            invitation_id: id,
            redeemer,
            lobby_id,
        })?;
        let stored = self.invitations.get_mut(&id).ok_or(SocialError::NotFound)?;
        stored.redeemed_lobby = Some(lobby_id);
        Ok(Participants {
            player_one: invitation.creator,
            player_two: redeemer,
        })
    }
}

/// Social transition rejection.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum SocialError {
    /// Handle does not meet canonical syntax.
    #[error("handle is invalid")]
    InvalidHandle,
    /// Handle is already assigned.
    #[error("handle is unavailable")]
    HandleTaken,
    /// Exact handle does not exist.
    #[error("handle not found")]
    HandleNotFound,
    /// Stable identifier or token hash already exists.
    #[error("record already exists")]
    AlreadyExists,
    /// Record does not exist.
    #[error("record not found")]
    NotFound,
    /// Actor is not authorized for the transition.
    #[error("operation is not authorized")]
    Unauthorized,
    /// Account challenged itself.
    #[error("cannot challenge self")]
    SelfChallenge,
    /// Invitation creator attempted redemption.
    #[error("cannot redeem own invitation")]
    SelfInvitation,
    /// Expiration is not in the future.
    #[error("invitation expiration is invalid")]
    InvalidExpiration,
    /// Invitation expired.
    #[error("invitation expired")]
    Expired,
    /// Invitation was revoked.
    #[error("invitation revoked")]
    Revoked,
    /// Invitation or challenge was already consumed.
    #[error("record already used")]
    AlreadyUsed,
    /// Durable journal failure.
    #[error(transparent)]
    Journal(#[from] SocialJournalError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct MemoryJournal(Vec<SocialTransition>);
    impl SocialJournal for MemoryJournal {
        fn commit(&mut self, transition: SocialTransition) -> Result<(), SocialJournalError> {
            self.0.push(transition);
            Ok(())
        }
    }

    #[test]
    fn exact_handle_challenge_accepts_once() {
        let mut service = SocialService::new(MemoryJournal::default());
        let handle = Handle::new("bob_2").unwrap();
        service
            .register_handle(handle.clone(), AccountId::new(2))
            .unwrap();
        service
            .challenge(ChallengeId::new(1), AccountId::new(1), &handle)
            .unwrap();
        assert_eq!(
            service.accept_challenge(ChallengeId::new(1), AccountId::new(2), LobbyId::new(5)),
            Ok(Participants {
                player_one: AccountId::new(1),
                player_two: AccountId::new(2)
            })
        );
        assert_eq!(
            service.accept_challenge(ChallengeId::new(1), AccountId::new(2), LobbyId::new(6)),
            Err(SocialError::NotFound)
        );
    }

    #[test]
    fn invitation_is_hash_only_expiring_revocable_and_single_use() {
        let mut service = SocialService::new(MemoryJournal::default());
        let hash = InvitationTokenHash::new([7; 32]);
        service
            .create_invitation(InvitationId::new(1), AccountId::new(1), hash, 100, 0)
            .unwrap();
        assert_eq!(
            service.redeem_invitation(hash, AccountId::new(2), LobbyId::new(9), 50),
            Ok(Participants {
                player_one: AccountId::new(1),
                player_two: AccountId::new(2)
            })
        );
        assert_eq!(
            service.redeem_invitation(hash, AccountId::new(3), LobbyId::new(10), 51),
            Err(SocialError::AlreadyUsed)
        );

        let revoked = InvitationTokenHash::new([8; 32]);
        service
            .create_invitation(InvitationId::new(2), AccountId::new(1), revoked, 100, 0)
            .unwrap();
        service
            .revoke_invitation(InvitationId::new(2), AccountId::new(1))
            .unwrap();
        assert_eq!(
            service.redeem_invitation(revoked, AccountId::new(2), LobbyId::new(11), 50),
            Err(SocialError::Revoked)
        );
    }
}
