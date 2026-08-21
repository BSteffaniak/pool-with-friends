//! Durable waiting-lobby lifecycle and connection-aware readiness.

use std::collections::{BTreeMap, BTreeSet};

use crate::{AccountId, ConnectionId, MatchId, Participants};
use thiserror::Error;

/// Stable waiting-lobby identifier.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct LobbyId(u128);

impl LobbyId {
    /// Creates a lobby identifier.
    #[must_use]
    pub const fn new(value: u128) -> Self {
        Self(value)
    }

    /// Returns the stable numeric identifier.
    #[must_use]
    pub const fn value(self) -> u128 {
        self.0
    }
}

/// Durable waiting-lobby lifecycle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LobbyStatus {
    /// Lobby is waiting for both participants to connect and ready.
    Waiting,
    /// Lobby atomically created this match.
    Started { match_id: MatchId },
    /// Lobby was cancelled before match start.
    Cancelled { by: AccountId },
}

/// Complete durable lobby record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LobbyRecord {
    /// Stable lobby identifier.
    pub id: LobbyId,
    /// Exactly two authorized members.
    pub participants: Participants,
    /// Durable lifecycle status.
    pub status: LobbyStatus,
}

/// Durable lobby transition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LobbyTransition {
    /// A non-expiring waiting lobby was created.
    Created(LobbyRecord),
    /// Waiting lobby atomically started one match.
    Started {
        /// Lobby that started.
        lobby_id: LobbyId,
        /// Newly created match.
        match_id: MatchId,
    },
    /// Waiting lobby was cancelled.
    Cancelled {
        /// Lobby that was cancelled.
        lobby_id: LobbyId,
        /// Participant who cancelled.
        by: AccountId,
    },
}

/// Atomic durable lobby journal.
pub trait LobbyJournal {
    /// Commits one lifecycle transition before it becomes visible.
    ///
    /// # Errors
    ///
    /// Returns [`LobbyJournalError`] when durability cannot be established.
    fn commit(&mut self, transition: LobbyTransition) -> Result<(), LobbyJournalError>;
}

/// Secret-safe durable lobby journal error.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("lobby journal commit failed")]
pub struct LobbyJournalError;

struct LobbyRuntime {
    record: LobbyRecord,
    connections: BTreeMap<AccountId, BTreeSet<ConnectionId>>,
    ready: BTreeSet<AccountId>,
}

/// Durable lobby service with ephemeral connection and readiness tracking.
pub struct LobbyService<J> {
    journal: J,
    lobbies: BTreeMap<LobbyId, LobbyRuntime>,
}

impl<J: LobbyJournal> LobbyService<J> {
    /// Creates an empty lobby service.
    #[must_use]
    pub const fn new(journal: J) -> Self {
        Self {
            journal,
            lobbies: BTreeMap::new(),
        }
    }

    /// Creates a durable non-expiring waiting lobby with exactly two members.
    ///
    /// # Errors
    ///
    /// Returns [`LobbyError`] for duplicate identifiers, duplicate members, or
    /// journal failure. Journal failure publishes no lobby.
    pub fn create(
        &mut self,
        id: LobbyId,
        participants: Participants,
    ) -> Result<LobbyRecord, LobbyError> {
        if self.lobbies.contains_key(&id) {
            return Err(LobbyError::AlreadyExists);
        }
        if participants.player_one == participants.player_two {
            return Err(LobbyError::DuplicateParticipant);
        }
        let record = LobbyRecord {
            id,
            participants,
            status: LobbyStatus::Waiting,
        };
        self.journal.commit(LobbyTransition::Created(record))?;
        self.lobbies.insert(
            id,
            LobbyRuntime {
                record,
                connections: BTreeMap::new(),
                ready: BTreeSet::new(),
            },
        );
        Ok(record)
    }

    /// Returns the durable lobby record.
    #[must_use]
    pub fn lobby(&self, id: LobbyId) -> Option<LobbyRecord> {
        self.lobbies.get(&id).map(|runtime| runtime.record)
    }

    /// Registers one authorized live lobby connection.
    ///
    /// # Errors
    ///
    /// Returns [`LobbyError`] when the lobby is unknown, no longer waiting, or
    /// the account is not a participant.
    pub fn connect(
        &mut self,
        id: LobbyId,
        account: AccountId,
        connection: ConnectionId,
    ) -> Result<(), LobbyError> {
        let runtime = self.waiting_mut(id)?;
        authorize(runtime.record.participants, account)?;
        runtime
            .connections
            .entry(account)
            .or_default()
            .insert(connection);
        Ok(())
    }

    /// Removes one connection and clears readiness only after the participant
    /// loses every authorized lobby connection.
    ///
    /// # Errors
    ///
    /// Returns [`LobbyError`] when the lobby is unknown or account unauthorized.
    pub fn disconnect(
        &mut self,
        id: LobbyId,
        account: AccountId,
        connection: ConnectionId,
    ) -> Result<(), LobbyError> {
        let runtime = self.waiting_mut(id)?;
        authorize(runtime.record.participants, account)?;
        if let Some(connections) = runtime.connections.get_mut(&account) {
            connections.remove(&connection);
            if connections.is_empty() {
                runtime.connections.remove(&account);
                runtime.ready.remove(&account);
            }
        }
        Ok(())
    }

    /// Marks a connected participant ready.
    ///
    /// # Errors
    ///
    /// Returns [`LobbyError`] when membership/lifecycle checks fail or the
    /// participant has no authorized connection.
    pub fn ready(&mut self, id: LobbyId, account: AccountId) -> Result<(), LobbyError> {
        let runtime = self.waiting_mut(id)?;
        authorize(runtime.record.participants, account)?;
        if !runtime.connections.contains_key(&account) {
            return Err(LobbyError::NotConnected);
        }
        runtime.ready.insert(account);
        Ok(())
    }

    /// Atomically starts a match only while both members are connected and ready.
    ///
    /// Duplicate start attempts cannot produce another match.
    ///
    /// # Errors
    ///
    /// Returns [`LobbyError`] when preconditions fail or the durable transition
    /// cannot be committed. Journal failure leaves the lobby waiting.
    pub fn start(&mut self, id: LobbyId, match_id: MatchId) -> Result<LobbyRecord, LobbyError> {
        {
            let runtime = self.waiting_mut(id)?;
            for account in [
                runtime.record.participants.player_one,
                runtime.record.participants.player_two,
            ] {
                if !runtime.connections.contains_key(&account) {
                    return Err(LobbyError::NotConnected);
                }
                if !runtime.ready.contains(&account) {
                    return Err(LobbyError::NotReady);
                }
            }
        }
        self.journal.commit(LobbyTransition::Started {
            lobby_id: id,
            match_id,
        })?;
        let runtime = self.waiting_mut(id)?;
        runtime.record.status = LobbyStatus::Started { match_id };
        Ok(runtime.record)
    }

    /// Cancels a waiting lobby by either participant.
    ///
    /// # Errors
    ///
    /// Returns [`LobbyError`] for lifecycle, authorization, or journal failure.
    pub fn cancel(&mut self, id: LobbyId, by: AccountId) -> Result<LobbyRecord, LobbyError> {
        {
            let runtime = self.waiting_mut(id)?;
            authorize(runtime.record.participants, by)?;
        }
        self.journal
            .commit(LobbyTransition::Cancelled { lobby_id: id, by })?;
        let runtime = self.waiting_mut(id)?;
        runtime.record.status = LobbyStatus::Cancelled { by };
        Ok(runtime.record)
    }

    fn waiting_mut(&mut self, id: LobbyId) -> Result<&mut LobbyRuntime, LobbyError> {
        let runtime = self.lobbies.get_mut(&id).ok_or(LobbyError::NotFound)?;
        if runtime.record.status != LobbyStatus::Waiting {
            return Err(LobbyError::NotWaiting);
        }
        Ok(runtime)
    }
}

/// Waiting-lobby transition rejection.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum LobbyError {
    /// Lobby identifier already exists.
    #[error("lobby already exists")]
    AlreadyExists,
    /// Lobby does not exist.
    #[error("lobby not found")]
    NotFound,
    /// Both seats reference the same account.
    #[error("lobby requires two distinct participants")]
    DuplicateParticipant,
    /// Account is not a lobby member.
    #[error("account is not authorized for lobby")]
    Unauthorized,
    /// Lobby is no longer waiting.
    #[error("lobby is not waiting")]
    NotWaiting,
    /// Participant has no authorized live connection.
    #[error("participant is not connected")]
    NotConnected,
    /// Both participants have not readied.
    #[error("lobby is not ready to start")]
    NotReady,
    /// Durable transition failed.
    #[error(transparent)]
    Journal(#[from] LobbyJournalError),
}

const fn authorize(participants: Participants, account: AccountId) -> Result<(), LobbyError> {
    if account.0 == participants.player_one.0 || account.0 == participants.player_two.0 {
        Ok(())
    } else {
        Err(LobbyError::Unauthorized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct MemoryJournal {
        transitions: Vec<LobbyTransition>,
        fail: bool,
    }

    impl LobbyJournal for MemoryJournal {
        fn commit(&mut self, transition: LobbyTransition) -> Result<(), LobbyJournalError> {
            if self.fail {
                Err(LobbyJournalError)
            } else {
                self.transitions.push(transition);
                Ok(())
            }
        }
    }

    fn participants() -> Participants {
        Participants {
            player_one: AccountId::new(1),
            player_two: AccountId::new(2),
        }
    }

    #[test]
    fn lobby_starts_once_only_when_both_connected_and_ready() {
        let mut service = LobbyService::new(MemoryJournal::default());
        service.create(LobbyId::new(1), participants()).unwrap();
        service
            .connect(LobbyId::new(1), AccountId::new(1), ConnectionId::new(1))
            .unwrap();
        service
            .connect(LobbyId::new(1), AccountId::new(2), ConnectionId::new(2))
            .unwrap();
        service.ready(LobbyId::new(1), AccountId::new(1)).unwrap();
        assert_eq!(
            service.start(LobbyId::new(1), MatchId::new(9)),
            Err(LobbyError::NotReady)
        );
        service.ready(LobbyId::new(1), AccountId::new(2)).unwrap();
        assert_eq!(
            service
                .start(LobbyId::new(1), MatchId::new(9))
                .unwrap()
                .status,
            LobbyStatus::Started {
                match_id: MatchId::new(9)
            }
        );
        assert_eq!(
            service.start(LobbyId::new(1), MatchId::new(10)),
            Err(LobbyError::NotWaiting)
        );
    }

    #[test]
    fn readiness_clears_only_after_last_connection_disconnects() {
        let mut service = LobbyService::new(MemoryJournal::default());
        service.create(LobbyId::new(1), participants()).unwrap();
        service
            .connect(LobbyId::new(1), AccountId::new(1), ConnectionId::new(1))
            .unwrap();
        service
            .connect(LobbyId::new(1), AccountId::new(1), ConnectionId::new(2))
            .unwrap();
        service.ready(LobbyId::new(1), AccountId::new(1)).unwrap();
        service
            .disconnect(LobbyId::new(1), AccountId::new(1), ConnectionId::new(1))
            .unwrap();
        assert!(
            service.lobbies[&LobbyId::new(1)]
                .ready
                .contains(&AccountId::new(1))
        );
        service
            .disconnect(LobbyId::new(1), AccountId::new(1), ConnectionId::new(2))
            .unwrap();
        assert!(
            !service.lobbies[&LobbyId::new(1)]
                .ready
                .contains(&AccountId::new(1))
        );
    }

    #[test]
    fn durable_failure_publishes_no_create_start_or_cancel() {
        let mut service = LobbyService::new(MemoryJournal::default());
        service.journal.fail = true;
        assert_eq!(
            service.create(LobbyId::new(1), participants()),
            Err(LobbyError::Journal(LobbyJournalError))
        );
        assert!(service.lobby(LobbyId::new(1)).is_none());

        service.journal.fail = false;
        service.create(LobbyId::new(1), participants()).unwrap();
        service
            .connect(LobbyId::new(1), AccountId::new(1), ConnectionId::new(1))
            .unwrap();
        service
            .connect(LobbyId::new(1), AccountId::new(2), ConnectionId::new(2))
            .unwrap();
        service.ready(LobbyId::new(1), AccountId::new(1)).unwrap();
        service.ready(LobbyId::new(1), AccountId::new(2)).unwrap();
        service.journal.fail = true;
        assert_eq!(
            service.start(LobbyId::new(1), MatchId::new(9)),
            Err(LobbyError::Journal(LobbyJournalError))
        );
        assert_eq!(
            service.lobby(LobbyId::new(1)).unwrap().status,
            LobbyStatus::Waiting
        );
        assert_eq!(
            service.cancel(LobbyId::new(1), AccountId::new(1)),
            Err(LobbyError::Journal(LobbyJournalError))
        );
        assert_eq!(
            service.lobby(LobbyId::new(1)).unwrap().status,
            LobbyStatus::Waiting
        );
    }
}
