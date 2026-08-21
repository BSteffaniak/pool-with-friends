#![cfg_attr(feature = "fail-on-warnings", deny(warnings))]
#![warn(clippy::all, clippy::pedantic, clippy::nursery, clippy::cargo)]
#![allow(clippy::multiple_crate_versions)]
//! Authoritative match command processing and recovery boundaries.

use std::collections::{BTreeMap, BTreeSet};

use pwmtf_game_domain::{MatchCommandResult, MatchState, Player};
use pwmtf_protocol::{CommandEnvelope, CommandId, SnapshotEnvelope};
use thiserror::Error;

mod identity;
mod identity_store;
mod lobby;
mod migrations;
mod profile_store;
mod session_store;
mod social;

pub use identity::{
    GoogleIdentity, IdentityError, IdentityJournal, IdentityJournalError, IdentityService,
    IdentityTransition, Session, SessionTokenHash,
};
pub use identity_store::{IdentityStoreError, account_for_google_identity, link_google_identity};

pub use lobby::{
    LobbyError, LobbyId, LobbyJournal, LobbyJournalError, LobbyRecord, LobbyService, LobbyStatus,
    LobbyTransition,
};
pub use migrations::{migrate, migrations};
pub use profile_store::{ProfileStoreError, account_for_handle, assign_handle, handle_for_account};
pub use session_store::{
    SessionStoreError, insert_session, resolve_session as resolve_stored_session,
    revoke_session as revoke_stored_session,
};
pub use social::{
    Challenge, ChallengeId, Handle, Invitation, InvitationId, InvitationTokenHash, SocialError,
    SocialJournal, SocialJournalError, SocialService, SocialTransition,
};

/// Stable authenticated account identifier supplied by the identity boundary.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct AccountId(u128);

impl AccountId {
    /// Creates an account identifier from the verified identity projection.
    #[must_use]
    pub const fn new(value: u128) -> Self {
        Self(value)
    }

    /// Returns the stable numeric identity.
    #[must_use]
    pub const fn value(self) -> u128 {
        self.0
    }
}

/// Authoritative absolute deadline in server-clock milliseconds.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DeadlineMillis(u64);

impl DeadlineMillis {
    /// Creates an absolute server-clock deadline.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the absolute millisecond value.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Stable identity of one scheduled turn deadline.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DeadlineId {
    /// Match revision for which this deadline was created.
    pub revision: u64,
    /// Player whose turn expires.
    pub player: Player,
}

/// Durable current deadline record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScheduledDeadline {
    /// Stable exact-once identity.
    pub id: DeadlineId,
    /// Absolute server-clock due time.
    pub due_at: DeadlineMillis,
}

/// Stable identity of one authorized live connection.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ConnectionId(u128);

impl ConnectionId {
    /// Creates a connection identifier.
    #[must_use]
    pub const fn new(value: u128) -> Self {
        Self(value)
    }
}

/// Authorized live match subscription registry.
#[derive(Default)]
pub struct SubscriptionRegistry {
    connections: BTreeMap<ConnectionId, AccountId>,
    subscriptions: BTreeMap<MatchId, BTreeSet<ConnectionId>>,
}

impl SubscriptionRegistry {
    /// Registers an authenticated connection.
    pub fn connect(&mut self, connection: ConnectionId, account: AccountId) {
        self.connections.insert(connection, account);
    }

    /// Removes a connection and all of its subscriptions.
    pub fn disconnect(&mut self, connection: ConnectionId) {
        self.connections.remove(&connection);
        for connections in self.subscriptions.values_mut() {
            connections.remove(&connection);
        }
        self.subscriptions
            .retain(|_, connections| !connections.is_empty());
    }

    /// Authorizes a connection to subscribe to one match.
    ///
    /// # Errors
    ///
    /// Returns [`SubscriptionError`] when the connection is unauthenticated or
    /// its account is not one of the match participants.
    pub fn subscribe(
        &mut self,
        connection: ConnectionId,
        match_id: MatchId,
        participants: Participants,
    ) -> Result<(), SubscriptionError> {
        let account = self
            .connections
            .get(&connection)
            .copied()
            .ok_or(SubscriptionError::Unauthenticated)?;
        participants
            .player_for(account)
            .ok_or(SubscriptionError::Unauthorized)?;
        self.subscriptions
            .entry(match_id)
            .or_default()
            .insert(connection);
        Ok(())
    }

    /// Returns subscribed connections in stable identifier order.
    #[must_use]
    pub fn subscribers(&self, match_id: MatchId) -> Vec<ConnectionId> {
        self.subscriptions
            .get(&match_id)
            .map_or_else(Vec::new, |connections| {
                connections.iter().copied().collect()
            })
    }
}

/// Live subscription authorization failure.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum SubscriptionError {
    /// Connection has no authenticated account.
    #[error("connection is not authenticated")]
    Unauthenticated,
    /// Authenticated account is not a match participant.
    #[error("connection is not authorized for match")]
    Unauthorized,
}

/// Snapshot publication failure.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum SnapshotError {
    /// Match does not exist.
    #[error("match not found")]
    MatchNotFound,
    /// Canonical snapshot exceeds the protocol bound.
    #[error("canonical snapshot exceeds protocol bound")]
    TooLarge,
}

/// Stable match identifier.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct MatchId(u128);

impl MatchId {
    /// Creates a match identifier.
    #[must_use]
    pub const fn new(value: u128) -> Self {
        Self(value)
    }
}

/// Authorized match participant mapping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Participants {
    /// Account occupying player one.
    pub player_one: AccountId,
    /// Account occupying player two.
    pub player_two: AccountId,
}

impl Participants {
    const fn player_for(self, account: AccountId) -> Option<Player> {
        if account.0 == self.player_one.0 {
            Some(Player::One)
        } else if account.0 == self.player_two.0 {
            Some(Player::Two)
        } else {
            None
        }
    }
}

/// Canonical command record that must become durable before acknowledgement.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AcceptedCommand {
    /// Match receiving the command.
    pub match_id: MatchId,
    /// New canonical revision after acceptance.
    pub revision: u64,
    /// Idempotency key.
    pub command_id: CommandId,
    /// Authenticated actor.
    pub actor: AccountId,
    /// Exact accepted wire frame.
    pub frame: Vec<u8>,
    /// Canonical command result needed for identical duplicate acknowledgement after recovery.
    pub result: MatchCommandResult,
    /// Complete resulting canonical aggregate snapshot.
    pub snapshot: Vec<u8>,
    /// Result checksum.
    pub checksum: u64,
    /// Next authoritative deadline, absent after completion.
    pub deadline: Option<ScheduledDeadline>,
}

/// Durable command journal adapter.
///
/// Implementations must commit the record atomically before returning success.
pub trait CommandJournal {
    /// Persists one accepted command and resulting canonical state.
    ///
    /// # Errors
    ///
    /// Returns an adapter-defined error when durability cannot be established.
    fn commit(&mut self, command: AcceptedCommand) -> Result<(), JournalError>;

    /// Loads durable accepted commands for one match in ascending revision order.
    ///
    /// # Errors
    ///
    /// Returns an adapter-defined error when durable records cannot be read or
    /// validated.
    fn load(&self, match_id: MatchId) -> Result<Vec<AcceptedCommand>, JournalError>;
}

/// Durable journal failure without backend-sensitive details.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
#[error("canonical command journal commit failed")]
pub struct JournalError;

/// Authoritative response for an accepted or previously accepted command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandAcknowledgement {
    /// Canonical revision after the command.
    pub revision: u64,
    /// Stable checksum after applying the command.
    pub checksum: u64,
    /// Canonical command result.
    pub result: MatchCommandResult,
    /// Whether this response was recovered from the idempotency cache.
    pub duplicate: bool,
    /// Next authoritative deadline, absent after completion.
    pub deadline: Option<ScheduledDeadline>,
}

/// Authoritative command rejection.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum CommandError {
    /// Match identifier is unknown.
    #[error("match not found")]
    MatchNotFound,
    /// Authenticated account is not a participant.
    #[error("account is not authorized for match")]
    Unauthorized,
    /// Command belongs to the other participant's turn or identity.
    #[error("account cannot apply this command")]
    WrongActor,
    /// Expected revision does not match canonical state.
    #[error("stale match revision: expected {expected}, actual {actual}")]
    StaleRevision {
        /// Revision in the command.
        expected: u64,
        /// Current canonical revision.
        actual: u64,
    },
    /// Same idempotency key was reused with different bytes or actor.
    #[error("idempotency key conflicts with an existing command")]
    IdempotencyConflict,
    /// Canonical domain rejected the transition.
    #[error("canonical match transition rejected")]
    Domain,
    /// Durable journal failed; no acknowledgement was produced.
    #[error(transparent)]
    Journal(#[from] JournalError),
}

/// Durable recovery failure.
#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum RecoveryError {
    /// Durable records could not be loaded.
    #[error(transparent)]
    Journal(#[from] JournalError),
    /// Revisions are not contiguous from one.
    #[error("durable command revisions are invalid")]
    InvalidRevision,
    /// A command id appears more than once in the journal.
    #[error("durable journal contains duplicate command ids")]
    DuplicateCommand,
    /// Accepted wire frame is malformed or incompatible.
    #[error("durable journal contains an invalid command frame")]
    InvalidFrame,
    /// Canonical snapshot is malformed or incompatible.
    #[error("durable journal contains an invalid canonical snapshot")]
    InvalidSnapshot,
    /// Snapshot checksum does not match the durable record.
    #[error("durable canonical snapshot checksum mismatch")]
    ChecksumMismatch,
}

struct MatchRuntime {
    participants: Participants,
    revision: u64,
    state: MatchState,
    deadline: Option<ScheduledDeadline>,
    accepted: BTreeMap<CommandId, StoredAcknowledgement>,
}

#[derive(Clone)]
struct StoredAcknowledgement {
    actor: AccountId,
    frame: Vec<u8>,
    acknowledgement: CommandAcknowledgement,
}

/// In-process authoritative match service with injected durability.
pub struct MatchService<J> {
    journal: J,
    matches: BTreeMap<MatchId, MatchRuntime>,
}

impl<J: CommandJournal> MatchService<J> {
    /// Creates an empty authoritative service.
    #[must_use]
    pub const fn new(journal: J) -> Self {
        Self {
            journal,
            matches: BTreeMap::new(),
        }
    }

    /// Registers a canonically initialized match.
    ///
    /// Returns the previous state only if the identifier was already present.
    pub fn insert_match(
        &mut self,
        match_id: MatchId,
        participants: Participants,
        state: MatchState,
    ) -> Option<MatchState> {
        self.matches
            .insert(
                match_id,
                MatchRuntime {
                    participants,
                    revision: 0,
                    state,
                    deadline: None,
                    accepted: BTreeMap::new(),
                },
            )
            .map(|runtime| runtime.state)
    }

    /// Returns the current canonical revision and state.
    #[must_use]
    pub fn match_state(&self, match_id: MatchId) -> Option<(u64, &MatchState)> {
        self.matches
            .get(&match_id)
            .map(|runtime| (runtime.revision, &runtime.state))
    }

    /// Returns a complete canonical snapshot for initial subscription or reconnect.
    ///
    /// # Errors
    ///
    /// Returns [`SnapshotError`] when the match is unknown or its canonical
    /// snapshot exceeds the protocol bound.
    pub fn snapshot(&self, match_id: MatchId) -> Result<SnapshotEnvelope, SnapshotError> {
        let runtime = self
            .matches
            .get(&match_id)
            .ok_or(SnapshotError::MatchNotFound)?;
        SnapshotEnvelope::new(
            runtime.revision,
            runtime.state.checksum(),
            runtime.state.to_bytes(),
        )
        .map_err(|_| SnapshotError::TooLarge)
    }

    /// Returns the current authoritative deadline.
    #[must_use]
    pub fn deadline(&self, match_id: MatchId) -> Option<ScheduledDeadline> {
        self.matches
            .get(&match_id)
            .and_then(|runtime| runtime.deadline)
    }

    /// Applies a due deadline exactly once when its revision/player identity is current.
    ///
    /// Stale or already-applied deadline identities are harmless no-ops.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError`] if the synthetic timeout cannot be durably accepted.
    pub fn apply_due_deadline(
        &mut self,
        match_id: MatchId,
        deadline: ScheduledDeadline,
        now: DeadlineMillis,
    ) -> Result<Option<CommandAcknowledgement>, CommandError> {
        let runtime = self
            .matches
            .get(&match_id)
            .ok_or(CommandError::MatchNotFound)?;
        if runtime.deadline != Some(deadline)
            || deadline.due_at > now
            || runtime.revision != deadline.id.revision
            || runtime.state.active_player() != deadline.id.player
        {
            return Ok(None);
        }
        let actor = match deadline.id.player {
            Player::One => runtime.participants.player_one,
            Player::Two => runtime.participants.player_two,
        };
        let mut id = [0_u8; 16];
        id[..8].copy_from_slice(&deadline.id.revision.to_be_bytes());
        id[8] = match deadline.id.player {
            Player::One => 1,
            Player::Two => 2,
        };
        let envelope = CommandEnvelope::new(
            deadline.id.revision,
            CommandId::new(id),
            pwmtf_game_domain::VersionedMatchCommand::new(pwmtf_game_domain::MatchCommand::Timeout),
        );
        self.apply_at(match_id, actor, envelope, now).map(Some)
    }

    /// Restores one match from its durable canonical command records.
    ///
    /// The latest record's complete snapshot becomes canonical state; every
    /// record is validated for contiguous revisions, match identity, unique
    /// idempotency keys, and snapshot checksum before publication.
    ///
    /// # Errors
    ///
    /// Returns [`RecoveryError`] when durable records cannot be loaded or fail
    /// canonical integrity checks.
    pub fn recover_match(
        &mut self,
        match_id: MatchId,
        participants: Participants,
    ) -> Result<Option<u64>, RecoveryError> {
        let records = self.journal.load(match_id)?;
        if records.is_empty() {
            return Ok(None);
        }
        let mut accepted = BTreeMap::new();
        let mut previous_revision = 0;
        let mut state = None;
        let mut records_last_deadline = None;
        for record in records {
            if record.match_id != match_id || record.revision != previous_revision + 1 {
                return Err(RecoveryError::InvalidRevision);
            }
            if accepted.contains_key(&record.command_id) {
                return Err(RecoveryError::DuplicateCommand);
            }
            CommandEnvelope::from_bytes(&record.frame).map_err(|_| RecoveryError::InvalidFrame)?;
            let restored = MatchState::from_bytes(&record.snapshot)
                .map_err(|_| RecoveryError::InvalidSnapshot)?;
            if restored.checksum() != record.checksum {
                return Err(RecoveryError::ChecksumMismatch);
            }
            let acknowledgement = CommandAcknowledgement {
                revision: record.revision,
                checksum: record.checksum,
                result: record.result.clone(),
                duplicate: false,
                deadline: record.deadline,
            };
            accepted.insert(
                record.command_id,
                StoredAcknowledgement {
                    actor: record.actor,
                    frame: record.frame,
                    acknowledgement,
                },
            );
            previous_revision = record.revision;
            records_last_deadline = record.deadline;
            state = Some(restored);
        }
        let state = state.ok_or(RecoveryError::InvalidSnapshot)?;
        self.matches.insert(
            match_id,
            MatchRuntime {
                participants,
                revision: previous_revision,
                state,
                deadline: records_last_deadline,
                accepted,
            },
        );
        Ok(Some(previous_revision))
    }

    /// Authenticates authorization, checks revision/idempotency, executes the
    /// canonical transition, durably journals it, and only then acknowledges.
    ///
    /// # Errors
    ///
    /// Returns [`CommandError`] for unknown matches, unauthorized/wrong actors,
    /// stale revisions, conflicting idempotency reuse, domain rejection, or a
    /// journal failure. Journal failure leaves in-memory canonical state and
    /// revision unchanged.
    pub fn apply(
        &mut self,
        match_id: MatchId,
        actor: AccountId,
        envelope: CommandEnvelope,
    ) -> Result<CommandAcknowledgement, CommandError> {
        self.apply_at(match_id, actor, envelope, DeadlineMillis::new(0))
    }

    /// Applies a command and schedules the next turn deadline from authoritative time.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::apply`].
    pub fn apply_at(
        &mut self,
        match_id: MatchId,
        actor: AccountId,
        envelope: CommandEnvelope,
        now: DeadlineMillis,
    ) -> Result<CommandAcknowledgement, CommandError> {
        let runtime = self
            .matches
            .get_mut(&match_id)
            .ok_or(CommandError::MatchNotFound)?;
        let player = runtime
            .participants
            .player_for(actor)
            .ok_or(CommandError::Unauthorized)?;
        let frame = envelope.to_bytes();
        if let Some(stored) = runtime.accepted.get(&envelope.command_id) {
            if stored.actor == actor && stored.frame == frame {
                let mut acknowledgement = stored.acknowledgement.clone();
                acknowledgement.duplicate = true;
                return Ok(acknowledgement);
            }
            return Err(CommandError::IdempotencyConflict);
        }
        if envelope.expected_revision != runtime.revision {
            return Err(CommandError::StaleRevision {
                expected: envelope.expected_revision,
                actual: runtime.revision,
            });
        }
        if command_requires_active_player(envelope.command.command)
            && runtime.state.active_player() != player
        {
            return Err(CommandError::WrongActor);
        }
        if let pwmtf_game_domain::MatchCommand::Concede {
            player: command_player,
        } = envelope.command.command
            && command_player != player
        {
            return Err(CommandError::WrongActor);
        }

        let mut candidate = runtime.state.clone();
        let result = candidate
            .apply_command(envelope.command)
            .map_err(|_| CommandError::Domain)?;
        let revision = runtime.revision.saturating_add(1);
        let checksum = candidate.checksum();
        let deadline = if matches!(
            candidate.status(),
            pwmtf_game_domain::MatchStatus::InProgress
        ) {
            Some(ScheduledDeadline {
                id: DeadlineId {
                    revision,
                    player: candidate.active_player(),
                },
                due_at: DeadlineMillis::new(now.value().saturating_add(30_000)),
            })
        } else {
            None
        };
        let acknowledgement = CommandAcknowledgement {
            revision,
            checksum,
            result: result.clone(),
            duplicate: false,
            deadline,
        };
        self.journal.commit(AcceptedCommand {
            match_id,
            revision,
            command_id: envelope.command_id,
            actor,
            frame: frame.clone(),
            result,
            snapshot: candidate.to_bytes(),
            checksum,
            deadline,
        })?;
        runtime.state = candidate;
        runtime.revision = revision;
        runtime.deadline = deadline;
        runtime.accepted.insert(
            envelope.command_id,
            StoredAcknowledgement {
                actor,
                frame,
                acknowledgement: acknowledgement.clone(),
            },
        );
        Ok(acknowledgement)
    }
}

const fn command_requires_active_player(command: pwmtf_game_domain::MatchCommand) -> bool {
    matches!(
        command,
        pwmtf_game_domain::MatchCommand::PlayShot { .. }
            | pwmtf_game_domain::MatchCommand::PlaceCueBall { .. }
    )
}

#[cfg(test)]
mod tests {
    use pwmtf_game_domain::{
        MatchCommand, PhysicsProfile, RackSeed, RulesProfile, TableGeometry, VersionedMatchCommand,
    };

    use super::*;

    #[derive(Default)]
    struct MemoryJournal {
        records: Vec<AcceptedCommand>,
        fail: bool,
    }

    impl CommandJournal for MemoryJournal {
        fn commit(&mut self, command: AcceptedCommand) -> Result<(), JournalError> {
            if self.fail {
                Err(JournalError)
            } else {
                self.records.push(command);
                Ok(())
            }
        }

        fn load(&self, match_id: MatchId) -> Result<Vec<AcceptedCommand>, JournalError> {
            if self.fail {
                Err(JournalError)
            } else {
                Ok(self
                    .records
                    .iter()
                    .filter(|record| record.match_id == match_id)
                    .cloned()
                    .collect())
            }
        }
    }

    fn service() -> MatchService<MemoryJournal> {
        let mut service = MatchService::new(MemoryJournal::default());
        service.insert_match(
            MatchId::new(9),
            Participants {
                player_one: AccountId::new(1),
                player_two: AccountId::new(2),
            },
            MatchState::new(
                RulesProfile::standard(),
                PhysicsProfile::standard(),
                TableGeometry::standard(),
                RackSeed::new(42),
                Player::One,
            )
            .unwrap(),
        );
        service
    }

    fn envelope(revision: u64, id: u8, command: MatchCommand) -> CommandEnvelope {
        CommandEnvelope::new(
            revision,
            CommandId::new([id; 16]),
            VersionedMatchCommand::new(command),
        )
    }

    #[test]
    fn snapshots_support_initial_load_and_reconnect_convergence() {
        let mut service = service();
        let initial = service.snapshot(MatchId::new(9)).unwrap();
        assert_eq!(initial.revision, 0);
        assert_eq!(
            pwmtf_game_domain::MatchState::from_bytes(&initial.snapshot)
                .unwrap()
                .checksum(),
            initial.checksum
        );
        service
            .apply(
                MatchId::new(9),
                AccountId::new(1),
                envelope(0, 7, MatchCommand::Timeout),
            )
            .unwrap();
        let reconnect = service.snapshot(MatchId::new(9)).unwrap();
        assert_eq!(reconnect.revision, 1);
        assert_ne!(reconnect.checksum, initial.checksum);
        let restored = pwmtf_game_domain::MatchState::from_bytes(&reconnect.snapshot).unwrap();
        assert_eq!(restored.active_player(), Player::Two);
        assert_eq!(restored.checksum(), reconnect.checksum);
    }

    #[test]
    fn subscriptions_require_authenticated_participants_and_allow_multiple_connections() {
        let participants = Participants {
            player_one: AccountId::new(1),
            player_two: AccountId::new(2),
        };
        let mut registry = SubscriptionRegistry::default();
        assert_eq!(
            registry.subscribe(ConnectionId::new(1), MatchId::new(9), participants),
            Err(SubscriptionError::Unauthenticated)
        );
        registry.connect(ConnectionId::new(1), AccountId::new(1));
        registry.connect(ConnectionId::new(2), AccountId::new(1));
        registry.connect(ConnectionId::new(3), AccountId::new(3));
        registry
            .subscribe(ConnectionId::new(1), MatchId::new(9), participants)
            .unwrap();
        registry
            .subscribe(ConnectionId::new(2), MatchId::new(9), participants)
            .unwrap();
        assert_eq!(
            registry.subscribe(ConnectionId::new(3), MatchId::new(9), participants),
            Err(SubscriptionError::Unauthorized)
        );
        assert_eq!(
            registry.subscribers(MatchId::new(9)),
            vec![ConnectionId::new(1), ConnectionId::new(2)]
        );
        registry.disconnect(ConnectionId::new(1));
        assert_eq!(
            registry.subscribers(MatchId::new(9)),
            vec![ConnectionId::new(2)]
        );
    }

    #[test]
    fn deadline_is_durable_and_applies_exactly_once() {
        let mut service = service();
        let first = service
            .apply_at(
                MatchId::new(9),
                AccountId::new(1),
                envelope(0, 5, MatchCommand::Timeout),
                DeadlineMillis::new(1_000),
            )
            .unwrap();
        let deadline = first.deadline.unwrap();
        assert_eq!(deadline.due_at, DeadlineMillis::new(31_000));
        assert_eq!(service.journal.records[0].deadline, Some(deadline));
        assert_eq!(
            service
                .apply_due_deadline(MatchId::new(9), deadline, DeadlineMillis::new(30_999))
                .unwrap(),
            None
        );
        let applied = service
            .apply_due_deadline(MatchId::new(9), deadline, DeadlineMillis::new(31_000))
            .unwrap()
            .unwrap();
        assert_eq!(applied.revision, 2);
        assert_eq!(
            service
                .match_state(MatchId::new(9))
                .unwrap()
                .1
                .active_player(),
            Player::One
        );
        assert_eq!(
            service
                .apply_due_deadline(MatchId::new(9), deadline, DeadlineMillis::new(40_000))
                .unwrap(),
            None
        );
        assert_eq!(service.journal.records.len(), 2);
    }

    #[test]
    fn recovery_restores_deadline_for_exact_once_application() {
        let mut original = service();
        let acknowledgement = original
            .apply_at(
                MatchId::new(9),
                AccountId::new(1),
                envelope(0, 6, MatchCommand::Timeout),
                DeadlineMillis::new(500),
            )
            .unwrap();
        let deadline = acknowledgement.deadline.unwrap();
        let mut recovered = MatchService::new(MemoryJournal {
            records: original.journal.records.clone(),
            fail: false,
        });
        recovered
            .recover_match(
                MatchId::new(9),
                Participants {
                    player_one: AccountId::new(1),
                    player_two: AccountId::new(2),
                },
            )
            .unwrap();
        assert_eq!(recovered.deadline(MatchId::new(9)), Some(deadline));
        assert!(
            recovered
                .apply_due_deadline(MatchId::new(9), deadline, deadline.due_at)
                .unwrap()
                .is_some()
        );
        assert_eq!(recovered.journal.records.len(), 2);
    }

    #[test]
    fn durable_recovery_restores_state_revision_and_idempotency() {
        let mut original = service();
        let command = envelope(0, 8, MatchCommand::Timeout);
        let first = original
            .apply(MatchId::new(9), AccountId::new(1), command)
            .unwrap();
        let journal = MemoryJournal {
            records: original.journal.records.clone(),
            fail: false,
        };
        let expected = original.match_state(MatchId::new(9)).unwrap().1.clone();
        let mut recovered = MatchService::new(journal);
        assert_eq!(
            recovered.recover_match(
                MatchId::new(9),
                Participants {
                    player_one: AccountId::new(1),
                    player_two: AccountId::new(2),
                },
            ),
            Ok(Some(1))
        );
        let (revision, state) = recovered.match_state(MatchId::new(9)).unwrap();
        assert_eq!(revision, 1);
        assert_eq!(state, &expected);
        let duplicate = recovered
            .apply(MatchId::new(9), AccountId::new(1), command)
            .unwrap();
        assert_eq!(duplicate.result, first.result);
        assert!(duplicate.duplicate);
        assert_eq!(recovered.journal.records.len(), 1);
    }

    #[test]
    fn corrupted_recovery_records_fail_closed() {
        let mut original = service();
        original
            .apply(
                MatchId::new(9),
                AccountId::new(1),
                envelope(0, 9, MatchCommand::Timeout),
            )
            .unwrap();
        let mut records = original.journal.records.clone();
        records[0].checksum ^= 1;
        let mut recovered = MatchService::new(MemoryJournal {
            records,
            fail: false,
        });
        assert_eq!(
            recovered.recover_match(
                MatchId::new(9),
                Participants {
                    player_one: AccountId::new(1),
                    player_two: AccountId::new(2),
                },
            ),
            Err(RecoveryError::ChecksumMismatch)
        );
        assert!(recovered.match_state(MatchId::new(9)).is_none());
    }

    #[test]
    fn acknowledgement_happens_after_durable_commit() {
        let mut service = service();
        let acknowledgement = service
            .apply(
                MatchId::new(9),
                AccountId::new(1),
                envelope(0, 1, MatchCommand::Timeout),
            )
            .unwrap();
        assert_eq!(acknowledgement.revision, 1);
        assert!(!acknowledgement.duplicate);
        assert_eq!(service.journal.records.len(), 1);
        let (revision, state) = service.match_state(MatchId::new(9)).unwrap();
        assert_eq!(revision, 1);
        assert_eq!(state.active_player(), Player::Two);
        assert_eq!(service.journal.records[0].snapshot, state.to_bytes());
    }

    #[test]
    fn journal_failure_does_not_mutate_or_acknowledge() {
        let mut service = service();
        service.journal.fail = true;
        let before = service.match_state(MatchId::new(9)).unwrap().1.clone();
        assert_eq!(
            service.apply(
                MatchId::new(9),
                AccountId::new(1),
                envelope(0, 1, MatchCommand::Timeout),
            ),
            Err(CommandError::Journal(JournalError))
        );
        let (revision, after) = service.match_state(MatchId::new(9)).unwrap();
        assert_eq!(revision, 0);
        assert_eq!(after, &before);
    }

    #[test]
    fn duplicates_are_harmless_and_conflicts_fail() {
        let mut service = service();
        let command = envelope(0, 3, MatchCommand::Timeout);
        let first = service
            .apply(MatchId::new(9), AccountId::new(1), command)
            .unwrap();
        let duplicate = service
            .apply(MatchId::new(9), AccountId::new(1), command)
            .unwrap();
        assert_eq!(first.revision, duplicate.revision);
        assert!(duplicate.duplicate);
        assert_eq!(service.journal.records.len(), 1);
        assert_eq!(
            service.apply(
                MatchId::new(9),
                AccountId::new(1),
                envelope(1, 3, MatchCommand::Timeout),
            ),
            Err(CommandError::IdempotencyConflict)
        );
    }

    #[test]
    fn authorization_revision_and_active_player_are_enforced() {
        let mut service = service();
        assert_eq!(
            service.apply(
                MatchId::new(9),
                AccountId::new(3),
                envelope(0, 1, MatchCommand::Timeout),
            ),
            Err(CommandError::Unauthorized)
        );
        assert!(matches!(
            service.apply(
                MatchId::new(9),
                AccountId::new(1),
                envelope(7, 2, MatchCommand::Timeout),
            ),
            Err(CommandError::StaleRevision { actual: 0, .. })
        ));
        assert_eq!(
            service.apply(
                MatchId::new(9),
                AccountId::new(2),
                envelope(
                    0,
                    4,
                    MatchCommand::PlayShot {
                        shot: pwmtf_game_domain::VersionedShotCommand::new(
                            pwmtf_game_domain::Aim::new(0).unwrap(),
                            pwmtf_game_domain::ShotPower::new(0).unwrap(),
                            pwmtf_game_domain::Spin::CENTER,
                        ),
                        called_pocket: None,
                    },
                ),
            ),
            Err(CommandError::WrongActor)
        );
    }
}
