//! Switchy persistence for waiting-lobby lifecycle transitions.

use crate::{
    AccountId, LobbyId, LobbyRecord, LobbyStatus, MatchId, Participants, ScheduledDeadline,
};
use pwmtf_game_domain::MatchState;
use switchy_database::{Database, DatabaseValue, query::FilterableQuery as _};
use thiserror::Error;

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

/// Atomically starts one waiting lobby and creates its initial canonical match.
///
/// The initial match snapshot, participants, revision, checksum, and optional
/// authoritative deadline become durable in the same transaction that consumes
/// the waiting lobby.
///
/// # Errors
///
/// Returns [`LobbyStoreError`] for missing or consumed lobbies, mismatched
/// deadline identity, duplicate match identifiers, malformed records, numeric
/// overflow, or database failures.
pub async fn start_lobby(
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
        .value("canonical_checksum", checksum_i64(state.checksum()));
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
            let started = start_lobby(
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
                start_lobby(
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
                start_lobby(&*db, LobbyId::new(2), MatchId::new(11), &state(), None).await,
                Err(LobbyStoreError::NotWaiting)
            ));
            assert!(db.select("matches").execute(&*db).await.unwrap().is_empty());
        });
    }
}
