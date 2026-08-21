//! Switchy query adapters for hash-only sessions.

use crate::{AccountId, Session, SessionToken, SessionTokenHash, TokenError};
use switchy_database::{Database, query::FilterableQuery as _};
use thiserror::Error;

/// Inserts a hash-only session record.
///
/// # Errors
///
/// Returns [`SessionStoreError`] for timestamp/account conversion overflow or
/// database failures.
pub async fn insert_session(
    db: &dyn Database,
    token_hash: SessionTokenHash,
    session: Session,
) -> Result<(), SessionStoreError> {
    db.insert("sessions")
        .value("session_hash", encode_hash(token_hash))
        .value("account_id", session.account.value().to_string())
        .value("expires_at_ms", to_i64(session.expires_at)?)
        .value("last_used_at_ms", to_i64(session.last_used_at)?)
        .execute(db)
        .await?;
    Ok(())
}

/// Generates and inserts a hash-only session, returning its raw token exactly
/// once to the authenticated transport boundary.
///
/// # Errors
///
/// Returns [`SessionStoreError`] when secure generation or persistence fails.
pub async fn create_session(
    db: &dyn Database,
    session: Session,
) -> Result<SessionToken, SessionStoreError> {
    let token = SessionToken::generate()?;
    insert_session(db, token.hash(), session).await?;
    Ok(token)
}

/// Resolves an unexpired raw session token through its hash.
///
/// # Errors
///
/// Returns [`SessionStoreError`] for malformed tokens, stored values, or
/// database failures.
pub async fn resolve_token(
    db: &dyn Database,
    token: &str,
    now: u64,
) -> Result<Option<AccountId>, SessionStoreError> {
    let token = SessionToken::parse(token)?;
    resolve_session(db, token.hash(), now).await
}

/// Revokes a raw session token through its hash.
///
/// # Errors
///
/// Returns [`SessionStoreError`] for malformed tokens or database failures.
pub async fn revoke_token(db: &dyn Database, token: &str) -> Result<(), SessionStoreError> {
    let token = SessionToken::parse(token)?;
    revoke_session(db, token.hash()).await
}

/// Resolves an unexpired session hash from Switchy storage.
///
/// # Errors
///
/// Returns [`SessionStoreError`] for malformed stored values or database failures.
pub async fn resolve_session(
    db: &dyn Database,
    token_hash: SessionTokenHash,
    now: u64,
) -> Result<Option<AccountId>, SessionStoreError> {
    let rows = db
        .select("sessions")
        .where_eq("session_hash", encode_hash(token_hash))
        .execute(db)
        .await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    let expires_at = row
        .get("expires_at_ms")
        .and_then(|value| value.as_i64())
        .ok_or(SessionStoreError::Malformed)?;
    if expires_at <= to_i64(now)? {
        return Ok(None);
    }
    let account = row
        .get("account_id")
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .ok_or(SessionStoreError::Malformed)?
        .parse::<u128>()
        .map_err(|_| SessionStoreError::Malformed)?;
    Ok(Some(AccountId::new(account)))
}

/// Revokes a session by deleting only its hash-indexed record.
///
/// # Errors
///
/// Returns [`SessionStoreError::Database`] on query failure.
pub async fn revoke_session(
    db: &dyn Database,
    token_hash: SessionTokenHash,
) -> Result<(), SessionStoreError> {
    db.delete("sessions")
        .where_eq("session_hash", encode_hash(token_hash))
        .execute(db)
        .await?;
    Ok(())
}

/// Session query adapter failure.
#[derive(Debug, Error)]
pub enum SessionStoreError {
    /// Switchy query failed.
    #[error(transparent)]
    Database(#[from] switchy_database::DatabaseError),
    /// Token generation or canonical parsing failed.
    #[error(transparent)]
    Token(#[from] TokenError),
    /// Numeric value cannot be represented by the portable schema.
    #[error("session value exceeds portable schema bounds")]
    Overflow,
    /// Stored record is malformed.
    #[error("stored session is malformed")]
    Malformed,
}

fn encode_hash(hash: SessionTokenHash) -> String {
    let mut output = String::with_capacity(64);
    for byte in hash.bytes() {
        use std::fmt::Write as _;
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn to_i64(value: u64) -> Result<i64, SessionStoreError> {
    i64::try_from(value).map_err(|_| SessionStoreError::Overflow)
}

#[cfg(test)]
mod tests {
    use futures_lite::future::block_on;

    use super::*;

    #[test]
    fn generated_session_persists_only_hash_and_resolves_raw_token() {
        block_on(async {
            let db = switchy_database_connection::builder()
                .turso()
                .with_in_memory()
                .build()
                .await
                .expect("in-memory Turso opens");
            crate::migrate(&*db).await.expect("schema migrates");
            let token = create_session(
                &*db,
                Session {
                    account: AccountId::new(9),
                    expires_at: 100,
                    last_used_at: 0,
                },
            )
            .await
            .unwrap();
            let stored = db
                .select("sessions")
                .execute(&*db)
                .await
                .unwrap()
                .remove(0)
                .get("session_hash")
                .and_then(|value| value.as_str().map(ToOwned::to_owned))
                .unwrap();
            assert_ne!(stored, token.expose());
            assert_eq!(
                resolve_token(&*db, token.expose(), 50).await.unwrap(),
                Some(AccountId::new(9))
            );
            revoke_token(&*db, token.expose()).await.unwrap();
            assert_eq!(resolve_token(&*db, token.expose(), 50).await.unwrap(), None);
        });
    }

    #[test]
    fn switchy_session_round_trip_and_revocation() {
        block_on(async {
            let db = switchy_database_connection::builder()
                .turso()
                .with_in_memory()
                .build()
                .await
                .expect("in-memory Turso opens");
            crate::migrate(&*db).await.expect("schema migrates");
            let hash = SessionTokenHash::new([5; 32]);
            insert_session(
                &*db,
                hash,
                Session {
                    account: AccountId::new(42),
                    expires_at: 100,
                    last_used_at: 0,
                },
            )
            .await
            .expect("session inserts");
            assert_eq!(
                resolve_session(&*db, hash, 50).await.unwrap(),
                Some(AccountId::new(42))
            );
            assert_eq!(resolve_session(&*db, hash, 100).await.unwrap(), None);
            revoke_session(&*db, hash).await.unwrap();
            assert_eq!(resolve_session(&*db, hash, 50).await.unwrap(), None);
        });
    }
}
