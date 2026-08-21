//! Switchy query adapter for stable public account handles.

use crate::{AccountId, Handle, SocialError};
use switchy_database::{Database, query::FilterableQuery as _};
use thiserror::Error;

/// Assigns one unique stable public handle to an account.
///
/// Repeating the same assignment is idempotent. Handles and accounts cannot be
/// reassigned through this boundary.
///
/// # Errors
///
/// Returns [`ProfileStoreError`] for conflicting or malformed records and
/// database failures.
pub async fn assign_handle(
    db: &dyn Database,
    account: AccountId,
    handle: &Handle,
) -> Result<(), ProfileStoreError> {
    let account_id = account.value().to_string();
    let account_rows = db
        .select("account_profiles")
        .where_eq("account_id", account_id.clone())
        .execute(db)
        .await?;
    if let Some(row) = exactly_one_or_none(&account_rows)? {
        let stored = text(row, "handle")?;
        return if stored == handle.as_str() {
            Ok(())
        } else {
            Err(ProfileStoreError::AccountConflict)
        };
    }

    if account_for_handle(db, handle).await?.is_some() {
        return Err(ProfileStoreError::HandleConflict);
    }

    db.insert("account_profiles")
        .value("account_id", account_id)
        .value("handle", handle.as_str())
        .execute(db)
        .await?;
    Ok(())
}

/// Resolves an exact handle to its stable account.
///
/// # Errors
///
/// Returns [`ProfileStoreError`] for malformed stored records or database
/// failures.
pub async fn account_for_handle(
    db: &dyn Database,
    handle: &Handle,
) -> Result<Option<AccountId>, ProfileStoreError> {
    let rows = db
        .select("account_profiles")
        .where_eq("handle", handle.as_str())
        .execute(db)
        .await?;
    let Some(row) = exactly_one_or_none(&rows)? else {
        return Ok(None);
    };
    let account = text(row, "account_id")?
        .parse::<u128>()
        .map_err(|_| ProfileStoreError::Malformed)?;
    Ok(Some(AccountId::new(account)))
}

/// Resolves an account's stable public handle.
///
/// # Errors
///
/// Returns [`ProfileStoreError`] for malformed stored records or database
/// failures.
pub async fn handle_for_account(
    db: &dyn Database,
    account: AccountId,
) -> Result<Option<Handle>, ProfileStoreError> {
    let rows = db
        .select("account_profiles")
        .where_eq("account_id", account.value().to_string())
        .execute(db)
        .await?;
    let Some(row) = exactly_one_or_none(&rows)? else {
        return Ok(None);
    };
    let handle = text(row, "handle")?;
    Ok(Some(Handle::new(&handle)?))
}

/// Durable account-profile storage failure.
#[derive(Debug, Error)]
pub enum ProfileStoreError {
    /// Account already owns a different stable handle.
    #[error("account already has a handle")]
    AccountConflict,
    /// Handle already belongs to another account.
    #[error("handle is unavailable")]
    HandleConflict,
    /// Stored account-profile record is malformed or duplicated.
    #[error("stored account profile is malformed")]
    Malformed,
    /// Database operation failed without exposing backend details to clients.
    #[error("account profile storage failed")]
    Database(#[from] switchy_database::DatabaseError),
    /// Handle failed canonical syntax validation.
    #[error(transparent)]
    Social(#[from] SocialError),
}

fn exactly_one_or_none(
    rows: &[switchy_database::Row],
) -> Result<Option<&switchy_database::Row>, ProfileStoreError> {
    match rows {
        [] => Ok(None),
        [row] => Ok(Some(row)),
        _ => Err(ProfileStoreError::Malformed),
    }
}

fn text(row: &switchy_database::Row, column: &str) -> Result<String, ProfileStoreError> {
    row.get(column)
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .ok_or(ProfileStoreError::Malformed)
}

#[cfg(test)]
mod tests {
    use futures_lite::future::block_on;

    use super::*;

    #[test]
    fn switchy_handles_are_unique_stable_and_exactly_resolved() {
        block_on(async {
            let db = switchy_database_connection::builder()
                .turso()
                .with_in_memory()
                .build()
                .await
                .expect("in-memory Turso opens");
            crate::migrate(&*db).await.expect("schema migrates");
            let alice = Handle::new("alice_1").unwrap();

            assign_handle(&*db, AccountId::new(1), &alice)
                .await
                .unwrap();
            assign_handle(&*db, AccountId::new(1), &alice)
                .await
                .unwrap();
            assert_eq!(
                account_for_handle(&*db, &alice).await.unwrap(),
                Some(AccountId::new(1))
            );
            assert_eq!(
                handle_for_account(&*db, AccountId::new(1)).await.unwrap(),
                Some(alice.clone())
            );
            assert!(matches!(
                assign_handle(&*db, AccountId::new(2), &alice).await,
                Err(ProfileStoreError::HandleConflict)
            ));
            assert!(matches!(
                assign_handle(&*db, AccountId::new(1), &Handle::new("alice_2").unwrap()).await,
                Err(ProfileStoreError::AccountConflict)
            ));
        });
    }
}
