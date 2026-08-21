//! Durable, browser-bound, exactly-once OIDC authorization attempts.

use crate::{GoogleOidcError, OidcAttempt};
use sha2::{Digest as _, Sha256};
use switchy_database::{Database, query::FilterableQuery as _};
use thiserror::Error;

const ATTEMPT_ID_BYTES: usize = 16;

/// New OIDC attempt containing secrets returned only to its initiating browser.
///
/// Debug output deliberately omits all values.
#[derive(Clone, Eq, PartialEq)]
pub struct NewOidcAttempt {
    attempt_id: String,
    attempt: OidcAttempt,
    browser_binding: String,
}

impl NewOidcAttempt {
    /// Creates an attempt from already validated transport material.
    ///
    /// # Errors
    ///
    /// Returns [`GoogleOidcError::Callback`] for malformed validation material.
    #[doc(hidden)]
    pub fn from_parts(
        attempt_id: String,
        attempt: OidcAttempt,
        browser_binding: String,
    ) -> Result<Self, GoogleOidcError> {
        if attempt_id.is_empty() || browser_binding.is_empty() {
            return Err(GoogleOidcError::Callback);
        }
        Ok(Self {
            attempt_id,
            attempt,
            browser_binding,
        })
    }

    /// Returns the public opaque attempt identifier.
    #[must_use]
    pub fn attempt_id(&self) -> &str {
        &self.attempt_id
    }

    /// Returns OIDC state for the provider authorization request.
    #[must_use]
    pub fn state(&self) -> &str {
        self.attempt.state()
    }

    /// Returns the browser-binding secret for a secure callback cookie.
    #[must_use]
    pub fn browser_binding(&self) -> &str {
        &self.browser_binding
    }

    /// Returns the complete in-memory OIDC attempt for authorization URL creation.
    #[must_use]
    pub const fn attempt(&self) -> &OidcAttempt {
        &self.attempt
    }
}

impl std::fmt::Debug for NewOidcAttempt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("NewOidcAttempt([REDACTED])")
    }
}

/// Claimed exactly-once callback validation material.
#[derive(Clone, Eq, PartialEq)]
pub struct ClaimedOidcAttempt {
    attempt: OidcAttempt,
}

impl ClaimedOidcAttempt {
    /// Returns claimed server-side validation material.
    #[must_use]
    pub const fn attempt(&self) -> &OidcAttempt {
        &self.attempt
    }
}

impl std::fmt::Debug for ClaimedOidcAttempt {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ClaimedOidcAttempt([REDACTED])")
    }
}

/// Generates and durably stores a short-lived browser-bound OIDC attempt.
///
/// Only hashes of callback state and browser binding are persisted. Nonce and
/// PKCE verifier are encrypted-channel server validation material, not tokens.
///
/// # Errors
///
/// Returns [`OidcAttemptStoreError`] for invalid expiry, unavailable secure
/// randomness, duplicate records, overflow, or database failures.
pub async fn create_oidc_attempt(
    db: &dyn Database,
    now: u64,
    expires_at: u64,
) -> Result<NewOidcAttempt, OidcAttemptStoreError> {
    if expires_at <= now {
        return Err(OidcAttemptStoreError::InvalidExpiration);
    }
    let attempt = OidcAttempt::generate()?;
    let browser_binding = random_hex(32)?;
    let attempt_id = random_hex(ATTEMPT_ID_BYTES)?;
    db.insert("oidc_attempts")
        .value("attempt_id", attempt_id.clone())
        .value("state_hash", hash(attempt.state()))
        .value("browser_binding_hash", hash(&browser_binding))
        .value("nonce", attempt.nonce())
        .value("pkce_verifier", attempt.pkce_verifier())
        .value("expires_at_ms", to_i64(expires_at)?)
        .value("status", "pending")
        .execute(db)
        .await?;
    Ok(NewOidcAttempt {
        attempt_id,
        attempt,
        browser_binding,
    })
}

/// Atomically claims one callback attempt after matching state and browser binding.
///
/// # Errors
///
/// Returns [`OidcAttemptStoreError`] for missing, expired, mismatched, consumed,
/// malformed, or database records.
pub async fn claim_oidc_attempt(
    db: &dyn Database,
    attempt_id: &str,
    state: &str,
    browser_binding: &str,
    now: u64,
) -> Result<ClaimedOidcAttempt, OidcAttemptStoreError> {
    let tx = db.begin_transaction().await?;
    let rows = tx
        .select("oidc_attempts")
        .where_eq("attempt_id", attempt_id)
        .execute(&*tx)
        .await?;
    let row = match rows.as_slice() {
        [] => return Err(OidcAttemptStoreError::NotFound),
        [row] => row,
        _ => return Err(OidcAttemptStoreError::Malformed),
    };
    if text(row, "status")? != "pending" {
        return Err(OidcAttemptStoreError::AlreadyUsed);
    }
    if integer(row, "expires_at_ms")? <= to_i64(now)? {
        return Err(OidcAttemptStoreError::Expired);
    }
    if !constant_time_eq(&text(row, "state_hash")?, &hash(state))
        || !constant_time_eq(&text(row, "browser_binding_hash")?, &hash(browser_binding))
    {
        return Err(OidcAttemptStoreError::Mismatch);
    }
    let attempt = OidcAttempt::claimed(state, &text(row, "nonce")?, &text(row, "pkce_verifier")?)?;
    let updated = tx
        .update("oidc_attempts")
        .value("status", "consumed")
        .where_eq("attempt_id", attempt_id)
        .where_eq("status", "pending")
        .execute(&*tx)
        .await?;
    if updated.len() != 1 {
        return Err(OidcAttemptStoreError::AlreadyUsed);
    }
    tx.commit().await?;
    Ok(ClaimedOidcAttempt { attempt })
}

/// Deletes expired and consumed OIDC attempt records.
///
/// # Errors
///
/// Returns [`OidcAttemptStoreError`] for malformed records, overflow, or
/// database failures.
pub async fn cleanup_oidc_attempts(
    db: &dyn Database,
    now: u64,
) -> Result<(), OidcAttemptStoreError> {
    let now = to_i64(now)?;
    let rows = db.select("oidc_attempts").execute(db).await?;
    for row in rows {
        if integer(&row, "expires_at_ms")? <= now || text(&row, "status")? == "consumed" {
            db.delete("oidc_attempts")
                .where_eq("attempt_id", text(&row, "attempt_id")?)
                .execute(db)
                .await?;
        }
    }
    Ok(())
}

/// Durable OIDC-attempt failure without secret-bearing details.
#[derive(Debug, Error)]
pub enum OidcAttemptStoreError {
    /// Expiry is not in the future.
    #[error("OIDC attempt expiration is invalid")]
    InvalidExpiration,
    /// Attempt does not exist.
    #[error("OIDC attempt not found")]
    NotFound,
    /// Attempt expired.
    #[error("OIDC attempt expired")]
    Expired,
    /// State or browser binding did not match.
    #[error("OIDC callback binding is invalid")]
    Mismatch,
    /// Attempt was already consumed.
    #[error("OIDC attempt already used")]
    AlreadyUsed,
    /// Stored record is malformed or duplicated.
    #[error("stored OIDC attempt is malformed")]
    Malformed,
    /// Numeric value exceeds portable schema bounds.
    #[error("OIDC attempt value is out of range")]
    Overflow,
    /// OIDC secret generation or parsing failed.
    #[error(transparent)]
    Oidc(#[from] GoogleOidcError),
    /// Database operation failed.
    #[error("OIDC attempt storage failed")]
    Database(#[from] switchy_database::DatabaseError),
}

fn random_hex(size: usize) -> Result<String, OidcAttemptStoreError> {
    let mut bytes = vec![0_u8; size];
    getrandom::fill(&mut bytes).map_err(|_| GoogleOidcError::Randomness)?;
    Ok(hex(&bytes))
}

fn hash(value: &str) -> String {
    hex(&Sha256::digest(value.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(output, "{byte:02x}").expect("writing to String cannot fail");
    }
    output
}

fn constant_time_eq(left: &str, right: &str) -> bool {
    left.len() == right.len()
        && left
            .bytes()
            .zip(right.bytes())
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
}

fn text(row: &switchy_database::Row, column: &str) -> Result<String, OidcAttemptStoreError> {
    row.get(column)
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .ok_or(OidcAttemptStoreError::Malformed)
}

fn integer(row: &switchy_database::Row, column: &str) -> Result<i64, OidcAttemptStoreError> {
    row.get(column)
        .and_then(|value| value.as_i64())
        .ok_or(OidcAttemptStoreError::Malformed)
}

fn to_i64(value: u64) -> Result<i64, OidcAttemptStoreError> {
    i64::try_from(value).map_err(|_| OidcAttemptStoreError::Overflow)
}

#[cfg(test)]
mod tests {
    use futures_lite::future::block_on;

    use super::*;

    #[test]
    fn attempts_are_hash_only_browser_bound_and_exactly_once() {
        block_on(async {
            let db = switchy_database_connection::builder()
                .turso()
                .with_in_memory()
                .build()
                .await
                .unwrap();
            crate::migrate(&*db).await.unwrap();
            let created = create_oidc_attempt(&*db, 0, 100).await.unwrap();
            let row = db
                .select("oidc_attempts")
                .execute(&*db)
                .await
                .unwrap()
                .remove(0);
            assert_ne!(text(&row, "state_hash").unwrap(), created.state());
            assert_ne!(
                text(&row, "browser_binding_hash").unwrap(),
                created.browser_binding()
            );
            assert!(matches!(
                claim_oidc_attempt(
                    &*db,
                    created.attempt_id(),
                    created.state(),
                    "wrong-binding",
                    50
                )
                .await,
                Err(OidcAttemptStoreError::Mismatch)
            ));
            let claimed = claim_oidc_attempt(
                &*db,
                created.attempt_id(),
                created.state(),
                created.browser_binding(),
                50,
            )
            .await
            .unwrap();
            assert_eq!(claimed.attempt(), created.attempt());
            assert!(matches!(
                claim_oidc_attempt(
                    &*db,
                    created.attempt_id(),
                    created.state(),
                    created.browser_binding(),
                    50
                )
                .await,
                Err(OidcAttemptStoreError::AlreadyUsed)
            ));
            cleanup_oidc_attempts(&*db, 50).await.unwrap();
            assert!(
                db.select("oidc_attempts")
                    .execute(&*db)
                    .await
                    .unwrap()
                    .is_empty()
            );
        });
    }
}
