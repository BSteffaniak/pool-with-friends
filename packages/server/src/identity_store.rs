//! Switchy query adapter for verified external identity links.

use crate::{AccountId, GoogleIdentity, IdentityError};
use switchy_database::{Database, query::FilterableQuery as _};
use thiserror::Error;

/// Links a verified Google identity to one stable account.
///
/// Repeating the same link is idempotent. An identity already linked to a
/// different account fails closed.
///
/// # Errors
///
/// Returns [`IdentityStoreError`] for conflicting or malformed records and
/// database failures.
pub async fn link_google_identity(
    db: &dyn Database,
    identity: &GoogleIdentity,
    account: AccountId,
) -> Result<(), IdentityStoreError> {
    let tx = db.begin_transaction().await?;
    if let Some(existing) = account_for_google_identity(&*tx, identity).await? {
        if existing == account {
            return Ok(());
        }
        return Err(IdentityStoreError::Conflict);
    }

    let insert = tx
        .insert("external_identities")
        .value("identity_id", identity_key(identity))
        .value("issuer", identity.issuer())
        .value("subject", identity.subject())
        .value("account_id", account.value().to_string())
        .execute(&*tx)
        .await;
    if let Err(error) = insert {
        match account_for_google_identity(&*tx, identity).await? {
            Some(existing) if existing == account => return Ok(()),
            Some(_) => return Err(IdentityStoreError::Conflict),
            None => return Err(IdentityStoreError::Database(error)),
        }
    }
    tx.commit().await?;
    Ok(())
}

/// Resolves a verified Google identity to its stable account.
///
/// # Errors
///
/// Returns [`IdentityStoreError`] for malformed stored values or database
/// failures.
pub async fn account_for_google_identity(
    db: &dyn Database,
    identity: &GoogleIdentity,
) -> Result<Option<AccountId>, IdentityStoreError> {
    let rows = db
        .select("external_identities")
        .where_eq("issuer", identity.issuer())
        .where_eq("subject", identity.subject())
        .execute(db)
        .await?;
    let Some(row) = rows.first() else {
        return Ok(None);
    };
    if rows.len() != 1 {
        return Err(IdentityStoreError::Malformed);
    }
    let value = row
        .get("account_id")
        .and_then(|value| value.as_str().map(ToOwned::to_owned))
        .ok_or(IdentityStoreError::Malformed)?;
    let account = value
        .parse::<u128>()
        .map_err(|_| IdentityStoreError::Malformed)?;
    Ok(Some(AccountId::new(account)))
}

/// Durable external-identity storage failure.
#[derive(Debug, Error)]
pub enum IdentityStoreError {
    /// Verified identity is already linked to a different account.
    #[error("external identity conflict")]
    Conflict,
    /// Stored identity record is malformed or duplicated.
    #[error("stored external identity is malformed")]
    Malformed,
    /// Database operation failed without exposing backend details to clients.
    #[error("external identity storage failed")]
    Database(#[from] switchy_database::DatabaseError),
    /// Verified claims failed canonical validation.
    #[error(transparent)]
    Identity(#[from] IdentityError),
}

fn identity_key(identity: &GoogleIdentity) -> String {
    format!(
        "google:{}:{}:{}{}",
        identity.issuer().len(),
        identity.subject().len(),
        identity.issuer(),
        identity.subject()
    )
}

#[cfg(test)]
mod tests {
    use futures_lite::future::block_on;

    use super::*;

    #[test]
    fn concurrent_identity_links_have_one_stable_owner() {
        block_on(async {
            let db: std::sync::Arc<dyn Database> = switchy_database_connection::builder()
                .turso()
                .with_in_memory()
                .build()
                .await
                .expect("in-memory Turso opens")
                .into();
            crate::migrate(&*db).await.expect("schema migrates");
            let identity =
                GoogleIdentity::verified("https://accounts.google.com", "raced-subject").unwrap();
            let first_identity = identity.clone();
            let second_identity = identity.clone();
            let first_db = std::sync::Arc::clone(&db);
            let second_db = std::sync::Arc::clone(&db);
            let (first, second) = futures_lite::future::zip(
                async move {
                    link_google_identity(&*first_db, &first_identity, AccountId::new(1)).await
                },
                async move {
                    link_google_identity(&*second_db, &second_identity, AccountId::new(2)).await
                },
            )
            .await;
            assert!(first.is_ok() ^ second.is_ok());
            let owner = account_for_google_identity(&*db, &identity)
                .await
                .unwrap()
                .unwrap();
            assert!(owner == AccountId::new(1) || owner == AccountId::new(2));
        });
    }

    #[test]
    fn switchy_google_identity_is_stable_idempotent_and_conflict_safe() {
        block_on(async {
            let db = switchy_database_connection::builder()
                .turso()
                .with_in_memory()
                .build()
                .await
                .expect("in-memory Turso opens");
            crate::migrate(&*db).await.expect("schema migrates");
            let identity =
                GoogleIdentity::verified("https://accounts.google.com", "subject-1").unwrap();

            link_google_identity(&*db, &identity, AccountId::new(42))
                .await
                .unwrap();
            link_google_identity(&*db, &identity, AccountId::new(42))
                .await
                .unwrap();
            assert_eq!(
                account_for_google_identity(&*db, &identity).await.unwrap(),
                Some(AccountId::new(42))
            );
            assert!(matches!(
                link_google_identity(&*db, &identity, AccountId::new(7)).await,
                Err(IdentityStoreError::Conflict)
            ));
        });
    }
}
