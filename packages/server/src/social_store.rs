//! Transactional Switchy persistence for challenges, invitations, and lobbies.

use crate::{
    AccountId, ChallengeId, InvitationId, InvitationToken, InvitationTokenHash, LobbyId,
    Participants, TokenError,
};
use switchy_database::{
    Database, DatabaseValue,
    query::{Expression as _, FilterableQuery as _, SortDirection},
};
use thiserror::Error;

/// Returns pending challenges addressed to an authenticated account.
///
/// Results are ordered by stable challenge identifier and expose no secrets.
///
/// # Errors
///
/// Returns [`SocialStoreError`] for malformed records or database failures.
pub async fn pending_challenges_for(
    db: &dyn Database,
    account_id: AccountId,
) -> Result<Vec<(ChallengeId, AccountId)>, SocialStoreError> {
    let rows = db
        .select("challenges")
        .where_eq("to_account_id", account_id.value().to_string())
        .where_eq("status", "pending")
        .sort("challenge_id", SortDirection::Asc)
        .execute(db)
        .await?;
    rows.iter()
        .map(|row| {
            let id = text(row, "challenge_id")?
                .parse::<u128>()
                .map(ChallengeId::new)
                .map_err(|_| SocialStoreError::Malformed)?;
            Ok((id, account(row, "from_account_id")?))
        })
        .collect()
}

/// Creates a pending exact-account challenge.
///
/// # Errors
///
/// Returns [`SocialStoreError`] for invalid participants, duplicate records, or
/// database failures.
pub async fn create_challenge(
    db: &dyn Database,
    id: ChallengeId,
    from: AccountId,
    to: AccountId,
) -> Result<(), SocialStoreError> {
    if from == to {
        return Err(SocialStoreError::SelfChallenge);
    }
    let duplicate = db
        .select("challenges")
        .where_eq("from_account_id", from.value().to_string())
        .where_eq("to_account_id", to.value().to_string())
        .where_eq("status", "pending")
        .execute(db)
        .await?;
    if !duplicate.is_empty() {
        return Err(SocialStoreError::DuplicateChallenge);
    }
    db.insert("challenges")
        .value("challenge_id", id.value().to_string())
        .value("from_account_id", from.value().to_string())
        .value("to_account_id", to.value().to_string())
        .value("status", "pending")
        .execute(db)
        .await?;
    Ok(())
}

/// Atomically accepts one pending challenge and creates its non-expiring lobby.
///
/// Repeating or racing acceptance cannot create a second lobby because the
/// challenge status transition and lobby insert commit in one transaction.
///
/// # Errors
///
/// Returns [`SocialStoreError`] for missing, consumed, unauthorized, malformed,
/// duplicate, or database records.
pub async fn accept_challenge_into_lobby(
    db: &dyn Database,
    id: ChallengeId,
    actor: AccountId,
    lobby_id: LobbyId,
) -> Result<Participants, SocialStoreError> {
    let tx = db.begin_transaction().await?;
    let rows = tx
        .select("challenges")
        .where_eq("challenge_id", id.value().to_string())
        .execute(&*tx)
        .await?;
    let row = exactly_one(&rows)?;
    if text(row, "status")? != "pending" {
        return Err(SocialStoreError::AlreadyUsed);
    }
    let participants = Participants {
        player_one: account(row, "from_account_id")?,
        player_two: account(row, "to_account_id")?,
    };
    if actor != participants.player_two {
        return Err(SocialStoreError::Unauthorized);
    }

    tx.insert("waiting_lobbies")
        .value("lobby_id", lobby_id.value().to_string())
        .value("player_one_id", participants.player_one.value().to_string())
        .value("player_two_id", participants.player_two.value().to_string())
        .value("status", "waiting")
        .value("match_id", DatabaseValue::Null)
        .execute(&*tx)
        .await?;
    let updated = tx
        .update("challenges")
        .value("status", format!("accepted:{}", lobby_id.value()))
        .where_eq("challenge_id", id.value().to_string())
        .where_eq("status", "pending")
        .execute(&*tx)
        .await?;
    if updated.len() != 1 {
        return Err(SocialStoreError::AlreadyUsed);
    }
    tx.commit().await?;
    Ok(participants)
}

/// Generates an opaque invitation and persists only its hash, returning the raw
/// token exactly once to the invitation-link boundary.
///
/// # Errors
///
/// Returns [`SocialStoreError`] when secure generation, validation, or
/// persistence fails.
pub async fn generate_invitation(
    db: &dyn Database,
    id: InvitationId,
    creator: AccountId,
    expires_at: u64,
    now: u64,
) -> Result<InvitationToken, SocialStoreError> {
    let token = InvitationToken::generate()?;
    create_invitation(db, id, creator, token.hash(), expires_at, now).await?;
    Ok(token)
}

/// Redeems a raw invitation token by hashing it at the transport boundary.
///
/// # Errors
///
/// Returns [`SocialStoreError`] for malformed, expired, revoked, consumed, or
/// unauthorized invitations and database failures.
pub async fn redeem_invitation_token_into_lobby(
    db: &dyn Database,
    token: &str,
    redeemer: AccountId,
    lobby_id: LobbyId,
    now: u64,
) -> Result<Participants, SocialStoreError> {
    let token = InvitationToken::parse(token)?;
    redeem_invitation_into_lobby(db, token.hash(), redeemer, lobby_id, now).await
}

/// Creates an expiring invitation while persisting only its token hash.
///
/// # Errors
///
/// Returns [`SocialStoreError`] for invalid expiry, duplicate records, overflow,
/// or database failures.
pub async fn create_invitation(
    db: &dyn Database,
    id: InvitationId,
    creator: AccountId,
    token_hash: InvitationTokenHash,
    expires_at: u64,
    now: u64,
) -> Result<(), SocialStoreError> {
    if expires_at <= now {
        return Err(SocialStoreError::InvalidExpiration);
    }
    db.insert("invitations")
        .value("invitation_id", id.value().to_string())
        .value("creator_id", creator.value().to_string())
        .value("token_hash", encode_hash(token_hash))
        .value("expires_at_ms", to_i64(expires_at)?)
        .value("revoked", 0_i64)
        .value("redeemed_lobby_id", DatabaseValue::Null)
        .execute(db)
        .await?;
    Ok(())
}

/// Revokes one unused invitation owned by the actor.
///
/// # Errors
///
/// Returns [`SocialStoreError`] for missing, used, unauthorized, or database
/// records.
pub async fn revoke_invitation(
    db: &dyn Database,
    id: InvitationId,
    actor: AccountId,
) -> Result<(), SocialStoreError> {
    let tx = db.begin_transaction().await?;
    let rows = tx
        .select("invitations")
        .where_eq("invitation_id", id.value().to_string())
        .execute(&*tx)
        .await?;
    let row = exactly_one(&rows)?;
    if account(row, "creator_id")? != actor {
        return Err(SocialStoreError::Unauthorized);
    }
    if !is_null(row, "redeemed_lobby_id")? {
        return Err(SocialStoreError::AlreadyUsed);
    }
    let updated = tx
        .update("invitations")
        .value("revoked", 1_i64)
        .where_eq("invitation_id", id.value().to_string())
        .where_eq("revoked", 0_i64)
        .where_eq("redeemed_lobby_id", DatabaseValue::Null)
        .execute(&*tx)
        .await?;
    if updated.len() != 1 {
        return Err(SocialStoreError::Revoked);
    }
    tx.commit().await?;
    Ok(())
}

/// Atomically redeems an invitation hash once and creates its non-expiring lobby.
///
/// # Errors
///
/// Returns [`SocialStoreError`] for missing, expired, revoked, consumed,
/// self-redeemed, malformed, duplicate, overflow, or database records.
pub async fn redeem_invitation_into_lobby(
    db: &dyn Database,
    token_hash: InvitationTokenHash,
    redeemer: AccountId,
    lobby_id: LobbyId,
    now: u64,
) -> Result<Participants, SocialStoreError> {
    let tx = db.begin_transaction().await?;
    let rows = tx
        .select("invitations")
        .where_eq("token_hash", encode_hash(token_hash))
        .execute(&*tx)
        .await?;
    let row = exactly_one(&rows)?;
    let creator = account(row, "creator_id")?;
    if creator == redeemer {
        return Err(SocialStoreError::SelfInvitation);
    }
    if integer(row, "revoked")? != 0 {
        return Err(SocialStoreError::Revoked);
    }
    if integer(row, "expires_at_ms")? <= to_i64(now)? {
        tx.delete("invitations")
            .where_eq("invitation_id", text(row, "invitation_id")?)
            .where_eq("redeemed_lobby_id", DatabaseValue::Null)
            .execute(&*tx)
            .await?;
        tx.commit().await?;
        return Err(SocialStoreError::Expired);
    }
    if !is_null(row, "redeemed_lobby_id")? {
        return Err(SocialStoreError::AlreadyUsed);
    }
    let invitation_id = text(row, "invitation_id")?;
    let participants = Participants {
        player_one: creator,
        player_two: redeemer,
    };

    tx.insert("waiting_lobbies")
        .value("lobby_id", lobby_id.value().to_string())
        .value("player_one_id", creator.value().to_string())
        .value("player_two_id", redeemer.value().to_string())
        .value("status", "waiting")
        .value("match_id", DatabaseValue::Null)
        .execute(&*tx)
        .await?;
    let updated = tx
        .update("invitations")
        .value("redeemed_lobby_id", lobby_id.value().to_string())
        .where_eq("invitation_id", invitation_id)
        .where_eq("revoked", 0_i64)
        .where_eq("redeemed_lobby_id", DatabaseValue::Null)
        .execute(&*tx)
        .await?;
    if updated.len() != 1 {
        return Err(SocialStoreError::AlreadyUsed);
    }
    tx.commit().await?;
    Ok(participants)
}

/// Durable social/lobby transaction failure.
#[derive(Debug, Error)]
pub enum SocialStoreError {
    /// Record does not exist.
    #[error("social record not found")]
    NotFound,
    /// Actor is not authorized for this transition.
    #[error("social operation is not authorized")]
    Unauthorized,
    /// Challenge targets its creator.
    #[error("cannot challenge self")]
    SelfChallenge,
    /// Same directed pending challenge already exists.
    #[error("pending challenge already exists")]
    DuplicateChallenge,
    /// Invitation creator attempted redemption.
    #[error("cannot redeem own invitation")]
    SelfInvitation,
    /// Invitation expiry is not in the future.
    #[error("invitation expiration is invalid")]
    InvalidExpiration,
    /// Invitation expired.
    #[error("invitation expired")]
    Expired,
    /// Invitation was revoked.
    #[error("invitation revoked")]
    Revoked,
    /// Social record was already consumed.
    #[error("social record already used")]
    AlreadyUsed,
    /// Stored social record is malformed or duplicated.
    #[error("stored social record is malformed")]
    Malformed,
    /// Token generation or canonical parsing failed.
    #[error(transparent)]
    Token(#[from] TokenError),
    /// Numeric value exceeds the portable database representation.
    #[error("social record value is out of range")]
    Overflow,
    /// Database operation failed without exposing backend details to clients.
    #[error("social storage failed")]
    Database(#[from] switchy_database::DatabaseError),
}

fn exactly_one(rows: &[switchy_database::Row]) -> Result<&switchy_database::Row, SocialStoreError> {
    match rows {
        [] => Err(SocialStoreError::NotFound),
        [row] => Ok(row),
        _ => Err(SocialStoreError::Malformed),
    }
}

fn text(row: &switchy_database::Row, column: &str) -> Result<String, SocialStoreError> {
    row.get(column)
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .ok_or(SocialStoreError::Malformed)
}

fn account(row: &switchy_database::Row, column: &str) -> Result<AccountId, SocialStoreError> {
    let value = text(row, column)?
        .parse::<u128>()
        .map_err(|_| SocialStoreError::Malformed)?;
    Ok(AccountId::new(value))
}

fn integer(row: &switchy_database::Row, column: &str) -> Result<i64, SocialStoreError> {
    row.get(column)
        .and_then(|value| value.as_i64())
        .ok_or(SocialStoreError::Malformed)
}

fn is_null(row: &switchy_database::Row, column: &str) -> Result<bool, SocialStoreError> {
    row.get(column)
        .map(|value| value.is_null())
        .ok_or(SocialStoreError::Malformed)
}

fn encode_hash(hash: InvitationTokenHash) -> String {
    let mut output = String::with_capacity(64);
    for byte in hash.bytes() {
        use std::fmt::Write as _;
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn to_i64(value: u64) -> Result<i64, SocialStoreError> {
    i64::try_from(value).map_err(|_| SocialStoreError::Overflow)
}

#[cfg(test)]
mod tests {
    use futures_lite::future::block_on;

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

    #[test]
    fn pending_challenges_are_filtered_and_stably_ordered() {
        block_on(async {
            let db = database().await;
            create_challenge(
                &*db,
                ChallengeId::new(20),
                AccountId::new(1),
                AccountId::new(3),
            )
            .await
            .unwrap();
            assert!(matches!(
                create_challenge(
                    &*db,
                    ChallengeId::new(21),
                    AccountId::new(1),
                    AccountId::new(3),
                )
                .await,
                Err(SocialStoreError::DuplicateChallenge)
            ));
            create_challenge(
                &*db,
                ChallengeId::new(10),
                AccountId::new(2),
                AccountId::new(3),
            )
            .await
            .unwrap();
            create_challenge(
                &*db,
                ChallengeId::new(5),
                AccountId::new(1),
                AccountId::new(4),
            )
            .await
            .unwrap();
            assert_eq!(
                pending_challenges_for(&*db, AccountId::new(3))
                    .await
                    .unwrap(),
                vec![
                    (ChallengeId::new(10), AccountId::new(2)),
                    (ChallengeId::new(20), AccountId::new(1)),
                ]
            );
        });
    }

    #[test]
    fn challenge_acceptance_creates_exactly_one_lobby_transactionally() {
        block_on(async {
            let db = database().await;
            create_challenge(
                &*db,
                ChallengeId::new(1),
                AccountId::new(1),
                AccountId::new(2),
            )
            .await
            .unwrap();
            assert_eq!(
                accept_challenge_into_lobby(
                    &*db,
                    ChallengeId::new(1),
                    AccountId::new(2),
                    LobbyId::new(10)
                )
                .await
                .unwrap(),
                Participants {
                    player_one: AccountId::new(1),
                    player_two: AccountId::new(2)
                }
            );
            assert!(matches!(
                accept_challenge_into_lobby(
                    &*db,
                    ChallengeId::new(1),
                    AccountId::new(2),
                    LobbyId::new(11)
                )
                .await,
                Err(SocialStoreError::AlreadyUsed)
            ));
            assert_eq!(
                db.select("waiting_lobbies")
                    .execute(&*db)
                    .await
                    .unwrap()
                    .len(),
                1
            );
        });
    }

    #[test]
    fn invitation_redemption_is_hash_only_single_use_and_transactional() {
        block_on(async {
            let db = database().await;
            let token = generate_invitation(&*db, InvitationId::new(1), AccountId::new(1), 100, 0)
                .await
                .unwrap();
            let hash = token.hash();
            assert_eq!(
                redeem_invitation_token_into_lobby(
                    &*db,
                    token.expose(),
                    AccountId::new(2),
                    LobbyId::new(20),
                    50,
                )
                .await
                .unwrap(),
                Participants {
                    player_one: AccountId::new(1),
                    player_two: AccountId::new(2)
                }
            );
            assert!(matches!(
                redeem_invitation_into_lobby(&*db, hash, AccountId::new(3), LobbyId::new(21), 51)
                    .await,
                Err(SocialStoreError::AlreadyUsed)
            ));
            let row = db
                .select("invitations")
                .execute(&*db)
                .await
                .unwrap()
                .remove(0);
            assert_eq!(text(&row, "token_hash").unwrap(), encode_hash(hash));
            assert_eq!(
                db.select("waiting_lobbies")
                    .execute(&*db)
                    .await
                    .unwrap()
                    .len(),
                1
            );
        });
    }

    #[test]
    fn revoked_and_expired_invitations_create_no_lobby() {
        block_on(async {
            let db = database().await;
            let revoked = InvitationTokenHash::new([8; 32]);
            create_invitation(
                &*db,
                InvitationId::new(2),
                AccountId::new(1),
                revoked,
                100,
                0,
            )
            .await
            .unwrap();
            revoke_invitation(&*db, InvitationId::new(2), AccountId::new(1))
                .await
                .unwrap();
            assert!(matches!(
                redeem_invitation_into_lobby(
                    &*db,
                    revoked,
                    AccountId::new(2),
                    LobbyId::new(30),
                    50
                )
                .await,
                Err(SocialStoreError::Revoked)
            ));

            let expired = InvitationTokenHash::new([9; 32]);
            create_invitation(
                &*db,
                InvitationId::new(3),
                AccountId::new(1),
                expired,
                100,
                0,
            )
            .await
            .unwrap();
            assert!(matches!(
                redeem_invitation_into_lobby(
                    &*db,
                    expired,
                    AccountId::new(2),
                    LobbyId::new(31),
                    100
                )
                .await,
                Err(SocialStoreError::Expired)
            ));
            assert!(
                db.select("invitations")
                    .where_eq("invitation_id", "3")
                    .execute(&*db)
                    .await
                    .unwrap()
                    .is_empty()
            );
            assert!(
                db.select("waiting_lobbies")
                    .execute(&*db)
                    .await
                    .unwrap()
                    .is_empty()
            );
        });
    }
}
