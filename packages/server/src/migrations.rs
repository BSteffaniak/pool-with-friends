//! Ordered Switchy schema for canonical server records.

use switchy_database::{
    Database,
    schema::{Column, DataType, alter_table, create_index, create_table, drop_index, drop_table},
};
use switchy_schema::{
    discovery::code::{CodeMigration, CodeMigrationSource},
    runner::MigrationRunner,
};

/// Returns the ordered versioned PWMTF schema source.
#[allow(clippy::too_many_lines)]
#[must_use]
pub fn migrations() -> CodeMigrationSource<'static> {
    migrations_for(true)
}

#[allow(clippy::too_many_lines)]
fn migrations_for(include_profiles: bool) -> CodeMigrationSource<'static> {
    let mut source = CodeMigrationSource::new();
    source.add_migration(table(
        "001_external_identities",
        "external_identities",
        vec![
            text("identity_id"),
            text("issuer"),
            text("subject"),
            text("account_id"),
        ],
        "identity_id",
    ));
    source.add_migration(index(
        "002_external_identity_unique",
        "idx_external_identity_issuer_subject",
        "external_identities",
        vec!["issuer", "subject"],
    ));
    source.add_migration(table(
        "003_sessions",
        "sessions",
        vec![
            text("session_hash"),
            text("account_id"),
            bigint("expires_at_ms"),
            bigint("last_used_at_ms"),
        ],
        "session_hash",
    ));
    source.add_migration(table(
        "004_waiting_lobbies",
        "waiting_lobbies",
        vec![
            text("lobby_id"),
            text("player_one_id"),
            text("player_two_id"),
            text("status"),
            nullable_text("match_id"),
        ],
        "lobby_id",
    ));
    source.add_migration(table(
        "005_challenges",
        "challenges",
        vec![
            text("challenge_id"),
            text("from_account_id"),
            text("to_account_id"),
            text("status"),
        ],
        "challenge_id",
    ));
    source.add_migration(table(
        "006_invitations",
        "invitations",
        vec![
            text("invitation_id"),
            text("creator_id"),
            text("token_hash"),
            bigint("expires_at_ms"),
            bigint("revoked"),
            nullable_text("redeemed_lobby_id"),
        ],
        "invitation_id",
    ));
    source.add_migration(index(
        "007_invitation_hash_unique",
        "idx_invitations_token_hash",
        "invitations",
        vec!["token_hash"],
    ));
    source.add_migration(table(
        "008_matches",
        "matches",
        vec![
            text("match_id"),
            text("player_one_id"),
            text("player_two_id"),
            bigint("canonical_revision"),
            text("canonical_snapshot"),
            bigint("canonical_checksum"),
            nullable_bigint("deadline_revision"),
            nullable_bigint("deadline_player"),
            nullable_bigint("deadline_at_ms"),
            nullable_text("previous_match_id"),
        ],
        "match_id",
    ));
    source.add_migration(table(
        "009_accepted_commands",
        "accepted_commands",
        vec![
            text("accepted_command_id"),
            text("match_id"),
            bigint("revision"),
            text("command_id"),
            text("actor_id"),
            text("frame"),
            text("result_payload"),
            text("snapshot"),
            bigint("checksum"),
        ],
        "accepted_command_id",
    ));
    source.add_migration(index(
        "010_command_revision_unique",
        "idx_accepted_commands_match_revision",
        "accepted_commands",
        vec!["match_id", "revision"],
    ));
    source.add_migration(index(
        "011_command_id_unique",
        "idx_accepted_commands_match_command",
        "accepted_commands",
        vec!["match_id", "command_id"],
    ));
    if include_profiles {
        source.add_migration(table(
            "012_account_profiles",
            "account_profiles",
            vec![text("account_id"), text("handle")],
            "account_id",
        ));
        source.add_migration(index(
            "013_profile_handle_unique",
            "idx_account_profiles_handle",
            "account_profiles",
            vec!["handle"],
        ));
        source.add_migration(add_text_column(
            "014_command_deadline_revision",
            "accepted_commands",
            "deadline_revision",
        ));
        source.add_migration(add_text_column(
            "015_command_deadline_player",
            "accepted_commands",
            "deadline_player",
        ));
        source.add_migration(add_text_column(
            "016_command_deadline_at",
            "accepted_commands",
            "deadline_at_ms",
        ));
        source.add_migration(table(
            "017_oidc_attempts",
            "oidc_attempts",
            vec![
                text("attempt_id"),
                text("state_hash"),
                text("browser_binding_hash"),
                text("nonce"),
                text("pkce_verifier"),
                bigint("expires_at_ms"),
                text("status"),
            ],
            "attempt_id",
        ));
        source.add_migration(index(
            "018_oidc_state_unique",
            "idx_oidc_attempts_state",
            "oidc_attempts",
            vec!["state_hash"],
        ));
        source.add_migration(table(
            "019_match_summaries",
            "match_summaries",
            vec![
                text("match_id"),
                text("winner_account_id"),
                text("completion_reason"),
                bigint("canonical_revision"),
                text("canonical_checksum"),
            ],
            "match_id",
        ));
        source.add_migration(table(
            "020_rematch_offers",
            "rematch_offers",
            vec![
                text("previous_match_id"),
                text("offered_by_account_id"),
                nullable_text("accepted_match_id"),
            ],
            "previous_match_id",
        ));
    }
    source
}

/// Runs all ordered PWMTF migrations.
///
/// # Errors
///
/// Returns a Switchy schema error when discovery or execution fails.
pub async fn migrate(db: &dyn Database) -> switchy_schema::Result<()> {
    migrate_source(db, migrations()).await
}

async fn migrate_source(
    db: &dyn Database,
    source: CodeMigrationSource<'static>,
) -> switchy_schema::Result<()> {
    MigrationRunner::new(Box::new(source))
        .with_table_name("__pwmtf_migrations")
        .run(db)
        .await
}

fn add_text_column(id: &str, table: &'static str, name: &'static str) -> CodeMigration<'static> {
    CodeMigration::new(
        id.to_owned(),
        Box::new(alter_table(table).add_column(name.to_owned(), DataType::Text, true, None)),
        Some(Box::new(alter_table(table).drop_column(name.to_owned()))),
    )
}

fn table(
    id: &str,
    name: &'static str,
    columns: Vec<Column>,
    primary_key: &'static str,
) -> CodeMigration<'static> {
    let mut statement = create_table(name);
    for column in columns {
        statement = statement.column(column);
    }
    CodeMigration::new(
        id.to_owned(),
        Box::new(statement.primary_key(primary_key)),
        Some(Box::new(drop_table(name).if_exists(true))),
    )
}

fn index(
    id: &str,
    name: &'static str,
    table: &'static str,
    columns: Vec<&'static str>,
) -> CodeMigration<'static> {
    CodeMigration::new(
        id.to_owned(),
        Box::new(
            create_index(name)
                .table(table)
                .columns(columns)
                .unique(true),
        ),
        Some(Box::new(drop_index(name, table).if_exists())),
    )
}

fn text(name: &str) -> Column {
    column(name, DataType::Text, false)
}
fn nullable_text(name: &str) -> Column {
    column(name, DataType::Text, true)
}
fn bigint(name: &str) -> Column {
    column(name, DataType::BigInt, false)
}
fn nullable_bigint(name: &str) -> Column {
    column(name, DataType::BigInt, true)
}
fn column(name: &str, data_type: DataType, nullable: bool) -> Column {
    Column {
        name: name.to_owned(),
        nullable,
        auto_increment: false,
        data_type,
        default: None,
    }
}

#[cfg(test)]
mod tests {
    use futures_lite::future::block_on;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };
    use switchy_schema::migration::MigrationSource as _;

    use super::*;

    #[test]
    fn migrations_execute_on_in_memory_turso() {
        block_on(async {
            let db = switchy_database_connection::builder()
                .turso()
                .with_in_memory()
                .build()
                .await
                .expect("in-memory Turso opens");
            migrate(&*db).await.expect("schema migrates");
            migrate(&*db).await.expect("schema migration is idempotent");
        });
    }

    #[test]
    fn file_backed_database_upgrades_from_previous_schema() {
        block_on(async {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock is after epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!(
                "pwmtf-migration-upgrade-{}-{unique}.db",
                std::process::id()
            ));
            let db = switchy_database_connection::builder()
                .turso()
                .with_path(&path)
                .build()
                .await
                .expect("file-backed Turso opens");

            migrate_source(&*db, migrations_for(false))
                .await
                .expect("previous schema migrates");
            assert!(
                !db.list_tables()
                    .await
                    .unwrap()
                    .contains(&"account_profiles".to_owned())
            );
            migrate(&*db).await.expect("current schema upgrades");
            assert!(
                db.list_tables()
                    .await
                    .unwrap()
                    .contains(&"account_profiles".to_owned())
            );
            migrate(&*db).await.expect("upgraded schema is idempotent");
            db.close().await.expect("database closes");
            fs::remove_file(&path).expect("database file is removed");
        });
    }

    #[test]
    fn migration_ids_are_ordered_and_unique() {
        let source = migrations();
        let migrations = block_on(source.migrations()).expect("migrations are discoverable");
        let ids = migrations
            .iter()
            .map(|migration| migration.id())
            .collect::<Vec<_>>();
        assert_eq!(ids.len(), 20);
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]));
    }
}
