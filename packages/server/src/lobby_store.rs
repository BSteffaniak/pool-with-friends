//! Switchy persistence for waiting-lobby lifecycle transitions.

use crate::{
    AccountId, ConnectionId, LobbyId, LobbyRecord, LobbyStatus, MatchId, Participants,
    ScheduledDeadline,
};
use pwmtf_game_domain::MatchState;
use switchy_database::{Database, DatabaseValue, query::FilterableQuery as _};
use thiserror::Error;

const LOBBY_CONNECTION_LEASE_MS: u64 = 15_000;

/// Registers a participant's authorized live lobby connection and transactionally
/// removes expired leases for that lobby.
///
/// # Errors
///
/// Returns [`LobbyStoreError`] for missing/consumed lobbies, unauthorized
/// participants, duplicate connections, malformed records, or database failures.
pub async fn connect_lobby(
    db: &dyn Database,
    lobby_id: LobbyId,
    actor: AccountId,
    connection: ConnectionId,
    now: u64,
) -> Result<LobbyRecord, LobbyStoreError> {
    let tx = db.begin_transaction().await?;
    let record = load_required_lobby(&*tx, lobby_id).await?;
    require_waiting_member(record, actor)?;
    let cutoff = to_i64(now.saturating_sub(LOBBY_CONNECTION_LEASE_MS))?;
    tx.delete("lobby_readiness")
        .where_eq("lobby_id", lobby_id.value().to_string())
        .where_lt("last_seen_at_ms", cutoff)
        .execute(&*tx)
        .await?;
    tx.insert("lobby_readiness")
        .value("lobby_id", lobby_id.value().to_string())
        .value("account_id", actor.value().to_string())
        .value("connection_id", connection.value().to_string())
        .value("ready", 0_i64)
        .value("last_seen_at_ms", to_i64(now)?)
        .execute(&*tx)
        .await?;
    tx.commit().await?;
    Ok(record)
}

/// Removes one exact live lobby connection and clears the participant's ready
/// state only after their last connection disappears.
///
/// # Errors
///
/// Returns [`LobbyStoreError`] for missing/consumed lobbies, unauthorized or
/// mismatched connections, malformed records, or database failures.
pub async fn disconnect_lobby(
    db: &dyn Database,
    lobby_id: LobbyId,
    actor: AccountId,
    connection: ConnectionId,
) -> Result<LobbyRecord, LobbyStoreError> {
    let tx = db.begin_transaction().await?;
    let record = load_required_lobby(&*tx, lobby_id).await?;
    require_waiting_member(record, actor)?;
    let rows = tx
        .select("lobby_readiness")
        .where_eq("connection_id", connection.value().to_string())
        .where_eq("lobby_id", lobby_id.value().to_string())
        .where_eq("account_id", actor.value().to_string())
        .execute(&*tx)
        .await?;
    if rows.len() != 1 {
        return Err(LobbyStoreError::ConnectionNotFound);
    }
    tx.delete("lobby_readiness")
        .where_eq("connection_id", connection.value().to_string())
        .execute(&*tx)
        .await?;
    tx.commit().await?;
    Ok(record)
}

/// Refreshes one exact live lobby connection lease. A heartbeat received after
/// expiry reconnects the lease but clears stale readiness, requiring a new
/// explicit ready action.
///
/// # Errors
///
/// Returns [`LobbyStoreError`] for missing/consumed lobbies, unauthorized or
/// mismatched connections, overflow, malformed records, or database failures.
pub async fn heartbeat_lobby(
    db: &dyn Database,
    lobby_id: LobbyId,
    actor: AccountId,
    connection: ConnectionId,
    now: u64,
) -> Result<LobbyRecord, LobbyStoreError> {
    let tx = db.begin_transaction().await?;
    let record = load_required_lobby(&*tx, lobby_id).await?;
    require_waiting_member(record, actor)?;
    let rows = tx
        .select("lobby_readiness")
        .where_eq("connection_id", connection.value().to_string())
        .where_eq("lobby_id", lobby_id.value().to_string())
        .where_eq("account_id", actor.value().to_string())
        .execute(&*tx)
        .await?;
    let [row] = rows.as_slice() else {
        return Err(LobbyStoreError::ConnectionNotFound);
    };
    let cutoff = to_i64(now.saturating_sub(LOBBY_CONNECTION_LEASE_MS))?;
    let expired = integer(row, "last_seen_at_ms")? < cutoff;
    let mut update = tx
        .update("lobby_readiness")
        .value("last_seen_at_ms", to_i64(now)?);
    if expired {
        update = update.value("ready", 0_i64);
    }
    let updated = update
        .where_eq("connection_id", connection.value().to_string())
        .where_eq("lobby_id", lobby_id.value().to_string())
        .where_eq("account_id", actor.value().to_string())
        .execute(&*tx)
        .await?;
    if updated.len() != 1 {
        return Err(LobbyStoreError::ConnectionNotFound);
    }
    tx.commit().await?;
    Ok(record)
}

/// Marks every live connection for a participant ready and removes that
/// participant's expired leases in the same transaction.
///
/// # Errors
///
/// Returns [`LobbyStoreError`] unless the actor has a live connection in the
/// waiting lobby at `now`, or persistence fails.
pub async fn ready_lobby(
    db: &dyn Database,
    lobby_id: LobbyId,
    actor: AccountId,
    now: u64,
) -> Result<LobbyRecord, LobbyStoreError> {
    let tx = db.begin_transaction().await?;
    let record = load_required_lobby(&*tx, lobby_id).await?;
    require_waiting_member(record, actor)?;
    let cutoff = to_i64(now.saturating_sub(LOBBY_CONNECTION_LEASE_MS))?;
    let rows = tx
        .select("lobby_readiness")
        .where_eq("lobby_id", lobby_id.value().to_string())
        .where_eq("account_id", actor.value().to_string())
        .where_gte("last_seen_at_ms", cutoff)
        .execute(&*tx)
        .await?;
    if rows.is_empty() {
        return Err(LobbyStoreError::NotConnected);
    }
    let updated = tx
        .update("lobby_readiness")
        .value("ready", 1_i64)
        .where_eq("lobby_id", lobby_id.value().to_string())
        .where_eq("account_id", actor.value().to_string())
        .where_gte("last_seen_at_ms", cutoff)
        .execute(&*tx)
        .await?;
    if updated.len() != rows.len() {
        return Err(LobbyStoreError::Malformed);
    }
    tx.delete("lobby_readiness")
        .where_eq("lobby_id", lobby_id.value().to_string())
        .where_eq("account_id", actor.value().to_string())
        .where_lt("last_seen_at_ms", cutoff)
        .execute(&*tx)
        .await?;
    tx.commit().await?;
    Ok(record)
}

/// Returns whether both participants have at least one live ready connection.
///
/// # Errors
///
/// Returns [`LobbyStoreError`] for missing, consumed, malformed, or database records.
pub async fn lobby_ready(
    db: &dyn Database,
    lobby_id: LobbyId,
    now: u64,
) -> Result<bool, LobbyStoreError> {
    let record = load_required_lobby(db, lobby_id).await?;
    if record.status != LobbyStatus::Waiting {
        return Err(LobbyStoreError::NotWaiting);
    }
    let cutoff = to_i64(now.saturating_sub(LOBBY_CONNECTION_LEASE_MS))?;
    let rows = db
        .select("lobby_readiness")
        .where_eq("lobby_id", lobby_id.value().to_string())
        .where_eq("ready", 1_i64)
        .where_gte("last_seen_at_ms", cutoff)
        .execute(db)
        .await?;
    let mut player_one = false;
    let mut player_two = false;
    for row in rows {
        let account = AccountId::new(parse_u128(&row, "account_id")?);
        if account != record.participants.player_one && account != record.participants.player_two {
            return Err(LobbyStoreError::Malformed);
        }
        player_one |= account == record.participants.player_one;
        player_two |= account == record.participants.player_two;
    }
    Ok(player_one && player_two)
}

fn require_waiting_member(record: LobbyRecord, actor: AccountId) -> Result<(), LobbyStoreError> {
    if record.status != LobbyStatus::Waiting {
        return Err(LobbyStoreError::NotWaiting);
    }
    if actor != record.participants.player_one && actor != record.participants.player_two {
        return Err(LobbyStoreError::Unauthorized);
    }
    Ok(())
}

/// Atomically starts one waiting lobby only while both participants have live,
/// ready connections.
///
/// # Errors
///
/// Returns [`LobbyStoreError`] for missing/consumed/unready lobbies, expired
/// connection leases, mismatched deadline identity, duplicate match identifiers,
/// malformed records, numeric overflow, or database failures.
pub async fn start_ready_lobby(
    db: &dyn Database,
    lobby_id: LobbyId,
    match_id: MatchId,
    state: &MatchState,
    deadline: ScheduledDeadline,
    now: u64,
) -> Result<LobbyRecord, LobbyStoreError> {
    let tx = db.begin_transaction().await?;
    let record = load_required_lobby(&*tx, lobby_id).await?;
    require_waiting_member(record, record.participants.player_one)?;
    let cutoff = to_i64(now.saturating_sub(LOBBY_CONNECTION_LEASE_MS))?;
    let rows = tx
        .select("lobby_readiness")
        .where_eq("lobby_id", lobby_id.value().to_string())
        .where_eq("ready", 1_i64)
        .where_gte("last_seen_at_ms", cutoff)
        .execute(&*tx)
        .await?;
    let mut player_one = false;
    let mut player_two = false;
    for row in rows {
        let account = AccountId::new(parse_u128(&row, "account_id")?);
        if account != record.participants.player_one && account != record.participants.player_two {
            return Err(LobbyStoreError::Malformed);
        }
        player_one |= account == record.participants.player_one;
        player_two |= account == record.participants.player_two;
    }
    if !player_one || !player_two {
        return Err(LobbyStoreError::NotReady);
    }
    start_lobby_in_transaction(&*tx, record, match_id, state, Some(deadline)).await?;
    tx.commit().await?;
    Ok(LobbyRecord {
        status: LobbyStatus::Started { match_id },
        ..record
    })
}

async fn start_lobby_in_transaction(
    tx: &dyn Database,
    record: LobbyRecord,
    match_id: MatchId,
    state: &MatchState,
    deadline: Option<ScheduledDeadline>,
) -> Result<(), LobbyStoreError> {
    if deadline
        .is_some_and(|value| value.id.revision != 0 || value.id.player != state.active_player())
    {
        return Err(LobbyStoreError::InvalidDeadline);
    }
    let snapshot = encode_bytes(&state.to_bytes());
    let mut insert = tx
        .insert("matches")
        .value("match_id", match_id.value().to_string())
        .value(
            "player_one_id",
            record.participants.player_one.value().to_string(),
        )
        .value(
            "player_two_id",
            record.participants.player_two.value().to_string(),
        )
        .value("canonical_revision", 0_i64)
        .value("canonical_snapshot", snapshot)
        .value("canonical_checksum", checksum_i64(state.checksum()))
        .value(
            "match_configuration",
            encode_bytes(&state.configuration().to_bytes()),
        );
    insert = match deadline {
        Some(value) => insert
            .value("deadline_revision", to_i64(value.id.revision)?)
            .value("deadline_player", encode_player(value.id.player))
            .value("deadline_at_ms", to_i64(value.due_at.value())?),
        None => insert
            .value("deadline_revision", DatabaseValue::Null)
            .value("deadline_player", DatabaseValue::Null)
            .value("deadline_at_ms", DatabaseValue::Null),
    };
    insert.execute(tx).await?;
    let updated = tx
        .update("waiting_lobbies")
        .value("status", format!("started:{}", match_id.value()))
        .value("match_id", match_id.value().to_string())
        .where_eq("lobby_id", record.id.value().to_string())
        .where_eq("status", "waiting")
        .execute(tx)
        .await?;
    if updated.len() != 1 {
        return Err(LobbyStoreError::NotWaiting);
    }
    Ok(())
}

/// Returns durable lobbies belonging to one participant in stable identifier order.
///
/// # Errors
///
/// Returns [`LobbyStoreError`] for malformed records or database failures.
pub async fn lobbies_for_account(
    db: &dyn Database,
    account: AccountId,
) -> Result<Vec<LobbyRecord>, LobbyStoreError> {
    let account_id = account.value().to_string();
    let player_one = db
        .select("waiting_lobbies")
        .where_eq("player_one_id", account_id.clone())
        .execute(db)
        .await?;
    let player_two = db
        .select("waiting_lobbies")
        .where_eq("player_two_id", account_id)
        .execute(db)
        .await?;
    let mut records = player_one
        .iter()
        .chain(&player_two)
        .map(decode_lobby)
        .collect::<Result<Vec<_>, _>>()?;
    records.sort_by_key(|record| record.id);
    records.dedup_by_key(|record| record.id);
    if records.iter().any(|record| {
        account != record.participants.player_one && account != record.participants.player_two
    }) {
        return Err(LobbyStoreError::Malformed);
    }
    Ok(records)
}

/// Loads one durable waiting-lobby record.
///
/// # Errors
///
/// Returns [`LobbyStoreError`] for malformed stored records or database
/// failures.
pub async fn load_lobby(
    db: &dyn Database,
    id: LobbyId,
) -> Result<Option<LobbyRecord>, LobbyStoreError> {
    let rows = db
        .select("waiting_lobbies")
        .where_eq("lobby_id", id.value().to_string())
        .execute(db)
        .await?;
    match rows.as_slice() {
        [] => Ok(None),
        [row] => Ok(Some(decode_lobby(row)?)),
        _ => Err(LobbyStoreError::Malformed),
    }
}

/// Test-only primitive that atomically starts one waiting lobby and creates its
/// initial canonical match without requiring connection readiness.
///
/// Production entry points must use [`start_ready_lobby`]. This helper exists
/// only to exercise lower-level atomic persistence and cancellation behavior.
///
/// # Errors
///
/// Returns [`LobbyStoreError`] for missing or consumed lobbies, mismatched
/// deadline identity, duplicate match identifiers, malformed records, numeric
/// overflow, or database failures.
#[cfg(test)]
async fn start_lobby_for_test(
    db: &dyn Database,
    lobby_id: LobbyId,
    match_id: MatchId,
    state: &MatchState,
    deadline: Option<ScheduledDeadline>,
) -> Result<LobbyRecord, LobbyStoreError> {
    if deadline
        .is_some_and(|value| value.id.revision != 0 || value.id.player != state.active_player())
    {
        return Err(LobbyStoreError::InvalidDeadline);
    }
    let tx = db.begin_transaction().await?;
    let record = load_required_lobby(&*tx, lobby_id).await?;
    if record.status != LobbyStatus::Waiting {
        return Err(LobbyStoreError::NotWaiting);
    }

    let snapshot = encode_bytes(&state.to_bytes());
    let mut insert = tx
        .insert("matches")
        .value("match_id", match_id.value().to_string())
        .value(
            "player_one_id",
            record.participants.player_one.value().to_string(),
        )
        .value(
            "player_two_id",
            record.participants.player_two.value().to_string(),
        )
        .value("canonical_revision", 0_i64)
        .value("canonical_snapshot", snapshot)
        .value("canonical_checksum", checksum_i64(state.checksum()))
        .value(
            "match_configuration",
            encode_bytes(&state.configuration().to_bytes()),
        );
    insert = match deadline {
        Some(value) => insert
            .value("deadline_revision", to_i64(value.id.revision)?)
            .value("deadline_player", encode_player(value.id.player))
            .value("deadline_at_ms", to_i64(value.due_at.value())?),
        None => insert
            .value("deadline_revision", DatabaseValue::Null)
            .value("deadline_player", DatabaseValue::Null)
            .value("deadline_at_ms", DatabaseValue::Null),
    };
    insert.execute(&*tx).await?;

    let status = format!("started:{}", match_id.value());
    let updated = tx
        .update("waiting_lobbies")
        .value("status", status)
        .value("match_id", match_id.value().to_string())
        .where_eq("lobby_id", lobby_id.value().to_string())
        .where_eq("status", "waiting")
        .execute(&*tx)
        .await?;
    if updated.len() != 1 {
        return Err(LobbyStoreError::NotWaiting);
    }
    tx.commit().await?;
    Ok(LobbyRecord {
        status: LobbyStatus::Started { match_id },
        ..record
    })
}

/// Atomically cancels one waiting lobby on behalf of a participant.
///
/// # Errors
///
/// Returns [`LobbyStoreError`] for missing or consumed lobbies, unauthorized
/// actors, malformed records, or database failures.
pub async fn cancel_lobby(
    db: &dyn Database,
    lobby_id: LobbyId,
    actor: AccountId,
) -> Result<LobbyRecord, LobbyStoreError> {
    let tx = db.begin_transaction().await?;
    let record = load_required_lobby(&*tx, lobby_id).await?;
    if record.status != LobbyStatus::Waiting {
        return Err(LobbyStoreError::NotWaiting);
    }
    if actor != record.participants.player_one && actor != record.participants.player_two {
        return Err(LobbyStoreError::Unauthorized);
    }
    let updated = tx
        .update("waiting_lobbies")
        .value("status", format!("cancelled:{}", actor.value()))
        .where_eq("lobby_id", lobby_id.value().to_string())
        .where_eq("status", "waiting")
        .execute(&*tx)
        .await?;
    if updated.len() != 1 {
        return Err(LobbyStoreError::NotWaiting);
    }
    tx.commit().await?;
    Ok(LobbyRecord {
        status: LobbyStatus::Cancelled { by: actor },
        ..record
    })
}

/// Durable waiting-lobby storage failure.
#[derive(Debug, Error)]
pub enum LobbyStoreError {
    /// Lobby does not exist.
    #[error("lobby not found")]
    NotFound,
    /// Lobby has already started or been cancelled.
    #[error("lobby is not waiting")]
    NotWaiting,
    /// Actor is not a lobby participant.
    #[error("lobby operation is not authorized")]
    Unauthorized,
    /// Participant has no live lobby connection.
    #[error("lobby participant is not connected")]
    NotConnected,
    /// Exact connection does not belong to this lobby participant.
    #[error("lobby connection not found")]
    ConnectionNotFound,
    /// Both participants do not have ready live connections.
    #[error("lobby is not ready")]
    NotReady,
    /// Initial deadline does not identify revision zero and the active player.
    #[error("initial match deadline is invalid")]
    InvalidDeadline,
    /// Stored lobby record is malformed or duplicated.
    #[error("stored lobby is malformed")]
    Malformed,
    /// Numeric value exceeds the portable database representation.
    #[error("lobby value is out of range")]
    Overflow,
    /// Database operation failed without exposing backend details to clients.
    #[error("lobby storage failed")]
    Database(#[from] switchy_database::DatabaseError),
}

async fn load_required_lobby(
    db: &dyn Database,
    id: LobbyId,
) -> Result<LobbyRecord, LobbyStoreError> {
    load_lobby(db, id).await?.ok_or(LobbyStoreError::NotFound)
}

fn decode_lobby(row: &switchy_database::Row) -> Result<LobbyRecord, LobbyStoreError> {
    let id = LobbyId::new(parse_u128(row, "lobby_id")?);
    let participants = Participants {
        player_one: AccountId::new(parse_u128(row, "player_one_id")?),
        player_two: AccountId::new(parse_u128(row, "player_two_id")?),
    };
    if participants.player_one == participants.player_two {
        return Err(LobbyStoreError::Malformed);
    }
    let status = text(row, "status")?;
    let status = if status == "waiting" {
        LobbyStatus::Waiting
    } else if let Some(value) = status.strip_prefix("started:") {
        LobbyStatus::Started {
            match_id: MatchId::new(parse_text_u128(value)?),
        }
    } else if let Some(value) = status.strip_prefix("cancelled:") {
        LobbyStatus::Cancelled {
            by: AccountId::new(parse_text_u128(value)?),
        }
    } else {
        return Err(LobbyStoreError::Malformed);
    };
    Ok(LobbyRecord {
        id,
        participants,
        status,
    })
}

fn parse_u128(row: &switchy_database::Row, column: &str) -> Result<u128, LobbyStoreError> {
    parse_text_u128(&text(row, column)?)
}

fn parse_text_u128(value: &str) -> Result<u128, LobbyStoreError> {
    value.parse().map_err(|_| LobbyStoreError::Malformed)
}

fn text(row: &switchy_database::Row, column: &str) -> Result<String, LobbyStoreError> {
    row.get(column)
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .ok_or(LobbyStoreError::Malformed)
}

fn integer(row: &switchy_database::Row, column: &str) -> Result<i64, LobbyStoreError> {
    row.get(column)
        .and_then(|value| value.as_i64())
        .ok_or(LobbyStoreError::Malformed)
}

fn to_i64(value: u64) -> Result<i64, LobbyStoreError> {
    i64::try_from(value).map_err(|_| LobbyStoreError::Overflow)
}

const fn encode_player(player: pwmtf_game_domain::Player) -> i64 {
    match player {
        pwmtf_game_domain::Player::One => 1,
        pwmtf_game_domain::Player::Two => 2,
    }
}

fn encode_bytes(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

const fn checksum_i64(value: u64) -> i64 {
    i64::from_ne_bytes(value.to_ne_bytes())
}

#[cfg(test)]
mod tests {
    use futures_lite::future::block_on;
    use pwmtf_game_domain::{
        MatchState, PhysicsProfile, Player, RackSeed, RulesProfile, TableGeometry,
    };

    use super::*;

    async fn database() -> Box<dyn Database> {
        let db = switchy_database_connection::builder()
            .turso()
            .with_in_memory()
            .build()
            .await
            .expect("in-memory Turso opens");
        crate::migrate(&*db).await.expect("schema migrates");
        db
    }

    async fn insert_lobby(db: &dyn Database, id: LobbyId) {
        db.insert("waiting_lobbies")
            .value("lobby_id", id.value().to_string())
            .value("player_one_id", "1")
            .value("player_two_id", "2")
            .value("status", "waiting")
            .value("match_id", DatabaseValue::Null)
            .execute(db)
            .await
            .unwrap();
    }

    fn state() -> MatchState {
        MatchState::new(
            RulesProfile::standard(),
            PhysicsProfile::standard(),
            TableGeometry::standard(),
            RackSeed::new(42),
            Player::One,
        )
        .unwrap()
    }

    #[test]
    fn readiness_requires_live_connections_for_both_participants() {
        block_on(async {
            let db = database().await;
            let lobby_id = LobbyId::new(3);
            insert_lobby(&*db, lobby_id).await;
            assert!(matches!(
                ready_lobby(&*db, lobby_id, AccountId::new(1), 0).await,
                Err(LobbyStoreError::NotConnected)
            ));
            connect_lobby(&*db, lobby_id, AccountId::new(1), ConnectionId::new(10), 0)
                .await
                .unwrap();
            connect_lobby(&*db, lobby_id, AccountId::new(2), ConnectionId::new(20), 0)
                .await
                .unwrap();
            ready_lobby(&*db, lobby_id, AccountId::new(1), 0)
                .await
                .unwrap();
            heartbeat_lobby(&*db, lobby_id, AccountId::new(1), ConnectionId::new(10), 5)
                .await
                .unwrap();
            assert!(!lobby_ready(&*db, lobby_id, 0).await.unwrap());
            ready_lobby(&*db, lobby_id, AccountId::new(2), 0)
                .await
                .unwrap();
            assert!(lobby_ready(&*db, lobby_id, 0).await.unwrap());
            assert!(!lobby_ready(&*db, lobby_id, 15_001).await.unwrap());
            assert!(matches!(
                ready_lobby(&*db, lobby_id, AccountId::new(2), 15_001).await,
                Err(LobbyStoreError::NotConnected)
            ));
            let state = state();
            let deadline = ScheduledDeadline {
                id: crate::DeadlineId {
                    revision: 0,
                    player: state.active_player(),
                },
                due_at: crate::DeadlineMillis::new(30_000),
            };
            assert!(matches!(
                start_ready_lobby(&*db, lobby_id, MatchId::new(29), &state, deadline, 15_001,)
                    .await,
                Err(LobbyStoreError::NotReady)
            ));
            assert!(db.select("matches").execute(&*db).await.unwrap().is_empty());
            heartbeat_lobby(
                &*db,
                lobby_id,
                AccountId::new(1),
                ConnectionId::new(10),
                15_001,
            )
            .await
            .unwrap();
            assert!(!lobby_ready(&*db, lobby_id, 15_001).await.unwrap());
            heartbeat_lobby(
                &*db,
                lobby_id,
                AccountId::new(2),
                ConnectionId::new(20),
                15_001,
            )
            .await
            .unwrap();
            assert!(!lobby_ready(&*db, lobby_id, 15_001).await.unwrap());
            ready_lobby(&*db, lobby_id, AccountId::new(2), 15_001)
                .await
                .unwrap();
            assert!(lobby_ready(&*db, lobby_id, 15_001).await.unwrap());
            let started =
                start_ready_lobby(&*db, lobby_id, MatchId::new(30), &state, deadline, 15_001)
                    .await
                    .unwrap();
            assert_eq!(
                started.status,
                LobbyStatus::Started {
                    match_id: MatchId::new(30)
                }
            );
            assert_eq!(db.select("matches").execute(&*db).await.unwrap().len(), 1);
            assert!(matches!(
                start_ready_lobby(&*db, lobby_id, MatchId::new(31), &state, deadline, 15_001,)
                    .await,
                Err(LobbyStoreError::NotWaiting)
            ));
            assert!(matches!(
                connect_lobby(&*db, lobby_id, AccountId::new(3), ConnectionId::new(30), 0).await,
                Err(LobbyStoreError::NotWaiting)
            ));
        });
    }

    #[test]
    fn last_disconnect_clears_readiness_and_exact_connection_is_required() {
        block_on(async {
            let db = database().await;
            let lobby_id = LobbyId::new(4);
            insert_lobby(&*db, lobby_id).await;
            connect_lobby(&*db, lobby_id, AccountId::new(1), ConnectionId::new(40), 0)
                .await
                .unwrap();
            connect_lobby(&*db, lobby_id, AccountId::new(1), ConnectionId::new(41), 0)
                .await
                .unwrap();
            ready_lobby(&*db, lobby_id, AccountId::new(1), 0)
                .await
                .unwrap();
            disconnect_lobby(&*db, lobby_id, AccountId::new(1), ConnectionId::new(40))
                .await
                .unwrap();
            let rows = db
                .select("lobby_readiness")
                .where_eq("connection_id", "41")
                .execute(&*db)
                .await
                .unwrap();
            assert_eq!(
                rows[0].get("ready").and_then(|value| value.as_i64()),
                Some(1)
            );
            disconnect_lobby(&*db, lobby_id, AccountId::new(1), ConnectionId::new(41))
                .await
                .unwrap();
            assert!(
                db.select("lobby_readiness")
                    .where_eq("account_id", "1")
                    .execute(&*db)
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert!(matches!(
                disconnect_lobby(&*db, lobby_id, AccountId::new(1), ConnectionId::new(40)).await,
                Err(LobbyStoreError::ConnectionNotFound)
            ));
        });
    }

    #[test]
    fn foreign_readiness_rows_fail_closed() {
        block_on(async {
            let db = database().await;
            let lobby_id = LobbyId::new(6);
            insert_lobby(&*db, lobby_id).await;
            db.insert("lobby_readiness")
                .value("lobby_id", lobby_id.value().to_string())
                .value("account_id", "3")
                .value("connection_id", "60")
                .value("ready", 1_i64)
                .value("last_seen_at_ms", 0_i64)
                .execute(&*db)
                .await
                .unwrap();
            assert!(matches!(
                lobby_ready(&*db, lobby_id, 0).await,
                Err(LobbyStoreError::Malformed)
            ));
            connect_lobby(&*db, lobby_id, AccountId::new(1), ConnectionId::new(61), 0)
                .await
                .unwrap();
            ready_lobby(&*db, lobby_id, AccountId::new(1), 0)
                .await
                .unwrap();
            assert!(matches!(
                start_ready_lobby(
                    &*db,
                    lobby_id,
                    MatchId::new(60),
                    &state(),
                    ScheduledDeadline {
                        id: crate::DeadlineId {
                            revision: 0,
                            player: Player::One,
                        },
                        due_at: crate::DeadlineMillis::new(30_000),
                    },
                    0,
                )
                .await,
                Err(LobbyStoreError::Malformed)
            ));
        });
    }

    #[test]
    fn connecting_prunes_expired_lobby_leases() {
        block_on(async {
            let db = database().await;
            let lobby_id = LobbyId::new(5);
            insert_lobby(&*db, lobby_id).await;
            connect_lobby(&*db, lobby_id, AccountId::new(1), ConnectionId::new(50), 0)
                .await
                .unwrap();
            connect_lobby(
                &*db,
                lobby_id,
                AccountId::new(2),
                ConnectionId::new(51),
                15_001,
            )
            .await
            .unwrap();
            let rows = db
                .select("lobby_readiness")
                .where_eq("lobby_id", lobby_id.value().to_string())
                .execute(&*db)
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(parse_u128(&rows[0], "connection_id").unwrap(), 51);
        });
    }

    #[test]
    fn lobby_start_atomically_persists_initial_match_once() {
        block_on(async {
            let db = database().await;
            insert_lobby(&*db, LobbyId::new(1)).await;
            let deadline = ScheduledDeadline {
                id: crate::DeadlineId {
                    revision: 0,
                    player: Player::One,
                },
                due_at: crate::DeadlineMillis::new(30_000),
            };
            let started = start_lobby_for_test(
                &*db,
                LobbyId::new(1),
                MatchId::new(9),
                &state(),
                Some(deadline),
            )
            .await
            .unwrap();
            assert_eq!(
                started.status,
                LobbyStatus::Started {
                    match_id: MatchId::new(9)
                }
            );
            assert!(matches!(
                start_lobby_for_test(
                    &*db,
                    LobbyId::new(1),
                    MatchId::new(10),
                    &state(),
                    Some(deadline)
                )
                .await,
                Err(LobbyStoreError::NotWaiting)
            ));
            assert_eq!(db.select("matches").execute(&*db).await.unwrap().len(), 1);
            assert_eq!(
                load_lobby(&*db, LobbyId::new(1)).await.unwrap(),
                Some(started)
            );
        });
    }

    #[test]
    fn lobby_cancellation_is_authorized_and_terminal() {
        block_on(async {
            let db = database().await;
            insert_lobby(&*db, LobbyId::new(2)).await;
            assert!(matches!(
                cancel_lobby(&*db, LobbyId::new(2), AccountId::new(3)).await,
                Err(LobbyStoreError::Unauthorized)
            ));
            let cancelled = cancel_lobby(&*db, LobbyId::new(2), AccountId::new(1))
                .await
                .unwrap();
            assert_eq!(
                cancelled.status,
                LobbyStatus::Cancelled {
                    by: AccountId::new(1)
                }
            );
            assert!(matches!(
                start_lobby_for_test(&*db, LobbyId::new(2), MatchId::new(11), &state(), None).await,
                Err(LobbyStoreError::NotWaiting)
            ));
            assert!(db.select("matches").execute(&*db).await.unwrap().is_empty());
        });
    }
}
