use rusqlite::Connection;
use thiserror::Error;

use super::rise_store_cipher::{CipherError, ValueCipher, cipher_context};

pub const SCHEMA_VERSION: i64 = 4;

const KEY_VERIFIER_PLAINTEXT: &[u8] = b"rise.engine.store.key-verifier.v1";
const KEY_VERIFIER_ROW: &str = "key_verifier";

#[derive(Debug, Error)]
pub enum StoreOpenError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("the database key does not open this store")]
    WrongKey,
    #[error("store was written by schema version {found}, this build speaks {expected}")]
    SchemaMismatch { found: i64, expected: i64 },
    #[error("cipher: {0}")]
    Cipher(#[from] CipherError),
}

// Greenfield: the reference migrated v1 through v4, we create v4 directly.
// The table shapes are copied from the reference so a database stays readable
// by either implementation.
const SCHEMA: &str = r#"
CREATE TABLE metadata (
    key TEXT PRIMARY KEY NOT NULL,
    value TEXT NOT NULL
) WITHOUT ROWID;
INSERT INTO metadata(key, value) VALUES ('global_revision', '0');
INSERT INTO metadata(key, value) VALUES ('next_durable_operation_sequence', '1');

CREATE TABLE crypto_metadata (
    key TEXT PRIMARY KEY NOT NULL,
    value BLOB NOT NULL
) WITHOUT ROWID;

CREATE TABLE key_values (
    namespace TEXT NOT NULL,
    key TEXT NOT NULL,
    value BLOB NOT NULL,
    version INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(namespace, key)
) WITHOUT ROWID;

CREATE TABLE entities (
    namespace TEXT NOT NULL,
    local_id TEXT NOT NULL,
    payload BLOB NOT NULL,
    version INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(namespace, local_id)
) WITHOUT ROWID;

CREATE TABLE entity_aliases (
    namespace TEXT NOT NULL,
    alias_kind TEXT NOT NULL,
    alias_value TEXT NOT NULL,
    local_id TEXT NOT NULL,
    PRIMARY KEY(namespace, alias_kind, alias_value),
    FOREIGN KEY(namespace, local_id)
        REFERENCES entities(namespace, local_id) ON DELETE CASCADE
) WITHOUT ROWID;
CREATE INDEX entity_aliases_entity
    ON entity_aliases(namespace, local_id, alias_kind, alias_value);

CREATE TABLE entity_views (
    namespace TEXT NOT NULL,
    view_id TEXT NOT NULL,
    local_id TEXT NOT NULL,
    position_key BLOB NOT NULL CHECK(length(position_key) > 0),
    version INTEGER NOT NULL,
    PRIMARY KEY(namespace, view_id, local_id),
    FOREIGN KEY(namespace, local_id)
        REFERENCES entities(namespace, local_id) ON DELETE CASCADE
) WITHOUT ROWID;
CREATE INDEX entity_views_position
    ON entity_views(namespace, view_id, position_key, local_id);
CREATE INDEX entity_views_entity
    ON entity_views(namespace, local_id, view_id);

CREATE TABLE entity_view_counts (
    namespace TEXT NOT NULL,
    view_id TEXT NOT NULL,
    row_count INTEGER NOT NULL CHECK(row_count >= 0),
    PRIMARY KEY(namespace, view_id)
) WITHOUT ROWID;
CREATE TRIGGER entity_view_count_insert
AFTER INSERT ON entity_views
BEGIN
    INSERT INTO entity_view_counts(namespace, view_id, row_count)
    VALUES (NEW.namespace, NEW.view_id, 1)
    ON CONFLICT(namespace, view_id) DO UPDATE SET row_count = row_count + 1;
END;
CREATE TRIGGER entity_view_count_delete
AFTER DELETE ON entity_views
BEGIN
    UPDATE entity_view_counts
    SET row_count = row_count - 1
    WHERE namespace = OLD.namespace AND view_id = OLD.view_id;
    DELETE FROM entity_view_counts
    WHERE namespace = OLD.namespace AND view_id = OLD.view_id AND row_count = 0;
END;

CREATE TABLE entity_view_state (
    namespace TEXT NOT NULL,
    view_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    before_cursor BLOB,
    after_cursor BLOB,
    has_more_before INTEGER NOT NULL CHECK(has_more_before IN (0, 1)),
    has_more_after INTEGER NOT NULL CHECK(has_more_after IN (0, 1)),
    load_state TEXT NOT NULL CHECK(load_state IN ('idle','loading','loaded','failed')),
    last_error_code TEXT,
    updated_at_ms INTEGER NOT NULL,
    PRIMARY KEY(namespace, view_id)
) WITHOUT ROWID;

CREATE TABLE view_revisions (
    view_key TEXT PRIMARY KEY NOT NULL,
    revision INTEGER NOT NULL
) WITHOUT ROWID;

CREATE TABLE durable_operations (
    request_id TEXT PRIMARY KEY NOT NULL,
    enqueue_sequence INTEGER NOT NULL UNIQUE,
    ordering_key TEXT NOT NULL,
    kind TEXT NOT NULL,
    payload BLOB NOT NULL,
    state TEXT NOT NULL CHECK(
        state IN ('queued','in_flight','retry_waiting','terminal_failure')
    ),
    attempt_count INTEGER NOT NULL CHECK(attempt_count >= 0),
    next_attempt_ms INTEGER NOT NULL,
    last_error_code TEXT,
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL,
    claim_token TEXT,
    CHECK(
        (state = 'in_flight' AND claim_token IS NOT NULL AND length(claim_token) > 0)
        OR (state != 'in_flight' AND claim_token IS NULL)
    )
) WITHOUT ROWID;
CREATE INDEX durable_operations_replay
    ON durable_operations(state, next_attempt_ms, enqueue_sequence);
CREATE INDEX durable_operations_ordering_lane
    ON durable_operations(ordering_key, enqueue_sequence);
CREATE INDEX durable_operations_terminal_lane
    ON durable_operations(ordering_key, kind, enqueue_sequence)
    WHERE state = 'terminal_failure';

CREATE TABLE durable_operation_usage (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    row_count INTEGER NOT NULL,
    payload_bytes INTEGER NOT NULL
);
INSERT INTO durable_operation_usage(singleton, row_count, payload_bytes) VALUES (1, 0, 0);
CREATE TRIGGER durable_operation_usage_insert
AFTER INSERT ON durable_operations
BEGIN
    UPDATE durable_operation_usage
    SET row_count = row_count + 1,
        payload_bytes = payload_bytes + length(NEW.payload)
    WHERE singleton = 1;
END;
CREATE TRIGGER durable_operation_usage_delete
AFTER DELETE ON durable_operations
BEGIN
    UPDATE durable_operation_usage
    SET row_count = row_count - 1,
        payload_bytes = payload_bytes - length(OLD.payload)
    WHERE singleton = 1;
END;
"#;

pub fn open_connection(path: &std::path::Path) -> Result<Connection, StoreOpenError> {
    let connection = Connection::open(path)?;
    configure(&connection)?;
    Ok(connection)
}

pub fn open_in_memory() -> Result<Connection, StoreOpenError> {
    let connection = Connection::open_in_memory()?;
    configure(&connection)?;
    Ok(connection)
}

fn configure(connection: &Connection) -> Result<(), StoreOpenError> {
    connection.pragma_update(None, "journal_mode", "WAL")?;
    connection.pragma_update(None, "synchronous", "NORMAL")?;
    connection.pragma_update(None, "foreign_keys", true)?;
    connection.busy_timeout(std::time::Duration::from_secs(5))?;
    Ok(())
}

pub fn user_version(connection: &Connection) -> Result<i64, StoreOpenError> {
    Ok(connection.query_row("PRAGMA user_version", [], |row| row.get(0))?)
}

/// Creates the schema on a fresh database, or verifies an existing one.
///
/// The key verifier is what turns "wrong key" into a clean error at open time
/// instead of a confusing decryption failure on the first read of real data.
pub fn prepare(
    connection: &Connection,
    account_id: &str,
    cipher: &dyn ValueCipher,
) -> Result<(), StoreOpenError> {
    let version = user_version(connection)?;

    if version == 0 {
        connection.execute_batch(SCHEMA)?;
        connection.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        write_key_verifier(connection, account_id, cipher)?;
        return Ok(());
    }

    if version != SCHEMA_VERSION {
        return Err(StoreOpenError::SchemaMismatch {
            found: version,
            expected: SCHEMA_VERSION,
        });
    }

    verify_key(connection, account_id, cipher)
}

fn verifier_context(account_id: &str) -> Vec<u8> {
    cipher_context(account_id, "crypto", &[KEY_VERIFIER_ROW])
}

fn write_key_verifier(
    connection: &Connection,
    account_id: &str,
    cipher: &dyn ValueCipher,
) -> Result<(), StoreOpenError> {
    let sealed = cipher.seal(KEY_VERIFIER_PLAINTEXT, &verifier_context(account_id))?;
    connection.execute(
        "INSERT OR REPLACE INTO crypto_metadata(key, value) VALUES (?1, ?2)",
        rusqlite::params![KEY_VERIFIER_ROW, sealed],
    )?;
    Ok(())
}

fn verify_key(
    connection: &Connection,
    account_id: &str,
    cipher: &dyn ValueCipher,
) -> Result<(), StoreOpenError> {
    let sealed: Vec<u8> = connection
        .query_row(
            "SELECT value FROM crypto_metadata WHERE key = ?1",
            [KEY_VERIFIER_ROW],
            |row| row.get(0),
        )
        .map_err(|_| StoreOpenError::WrongKey)?;

    match cipher.open(&sealed, &verifier_context(account_id)) {
        Ok(plaintext) if plaintext == KEY_VERIFIER_PLAINTEXT => Ok(()),
        _ => Err(StoreOpenError::WrongKey),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rise_store::rise_store_cipher::{AesGcmCipher, PlaintextCipher};

    fn tables(connection: &Connection) -> Vec<String> {
        let mut statement = connection
            .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
            .unwrap();
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap();
        rows.map(Result::unwrap).collect()
    }

    #[test]
    fn a_fresh_database_is_created_at_the_current_schema_version() {
        let connection = open_in_memory().unwrap();
        prepare(&connection, "u1", &PlaintextCipher).unwrap();
        assert_eq!(user_version(&connection).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn every_expected_table_exists() {
        let connection = open_in_memory().unwrap();
        prepare(&connection, "u1", &PlaintextCipher).unwrap();
        let tables = tables(&connection);
        for expected in [
            "crypto_metadata",
            "durable_operation_usage",
            "durable_operations",
            "entities",
            "entity_aliases",
            "entity_view_counts",
            "entity_view_state",
            "entity_views",
            "key_values",
            "metadata",
            "view_revisions",
        ] {
            assert!(tables.contains(&expected.to_string()), "missing {expected}");
        }
    }

    #[test]
    fn preparing_an_existing_database_is_idempotent() {
        let connection = open_in_memory().unwrap();
        prepare(&connection, "u1", &PlaintextCipher).unwrap();
        prepare(&connection, "u1", &PlaintextCipher).unwrap();
        assert_eq!(user_version(&connection).unwrap(), SCHEMA_VERSION);
    }

    #[test]
    fn the_wrong_key_is_rejected_at_open_rather_than_on_first_read() {
        let connection = open_in_memory().unwrap();
        let right = AesGcmCipher::new(&[1u8; 32]).unwrap();
        prepare(&connection, "u1", &right).unwrap();

        let wrong = AesGcmCipher::new(&[2u8; 32]).unwrap();
        assert!(matches!(
            prepare(&connection, "u1", &wrong),
            Err(StoreOpenError::WrongKey)
        ));
    }

    #[test]
    fn the_right_key_reopens_the_store() {
        let connection = open_in_memory().unwrap();
        let cipher = AesGcmCipher::new(&[1u8; 32]).unwrap();
        prepare(&connection, "u1", &cipher).unwrap();
        assert!(prepare(&connection, "u1", &cipher).is_ok());
    }

    #[test]
    fn another_accounts_key_verifier_does_not_open_this_store() {
        let connection = open_in_memory().unwrap();
        let cipher = AesGcmCipher::new(&[1u8; 32]).unwrap();
        prepare(&connection, "u1", &cipher).unwrap();
        assert!(matches!(
            prepare(&connection, "u2", &cipher),
            Err(StoreOpenError::WrongKey)
        ));
    }

    #[test]
    fn a_future_schema_is_refused_instead_of_silently_used() {
        let connection = open_in_memory().unwrap();
        prepare(&connection, "u1", &PlaintextCipher).unwrap();
        connection
            .pragma_update(None, "user_version", SCHEMA_VERSION + 1)
            .unwrap();
        assert!(matches!(
            prepare(&connection, "u1", &PlaintextCipher),
            Err(StoreOpenError::SchemaMismatch { .. })
        ));
    }

    #[test]
    fn an_in_flight_durable_row_must_carry_a_claim_token() {
        let connection = open_in_memory().unwrap();
        prepare(&connection, "u1", &PlaintextCipher).unwrap();

        let insert = |state: &str, token: Option<&str>| {
            connection.execute(
                "INSERT INTO durable_operations(request_id, enqueue_sequence, ordering_key, kind,
                 payload, state, attempt_count, next_attempt_ms, created_at_ms, updated_at_ms,
                 claim_token) VALUES (?1, ?2, 'lane', 'k', x'00', ?3, 0, 0, 0, 0, ?4)",
                rusqlite::params![state, rand_seq(), state, token],
            )
        };
        assert!(
            insert("in_flight", None).is_err(),
            "in_flight without a claim token must be rejected by the schema"
        );
        assert!(
            insert("queued", Some("tok")).is_err(),
            "a claim token outside in_flight must be rejected by the schema"
        );
        assert!(insert("in_flight", Some("tok")).is_ok());
    }

    #[test]
    fn view_counts_are_maintained_by_trigger() {
        let connection = open_in_memory().unwrap();
        prepare(&connection, "u1", &PlaintextCipher).unwrap();
        connection
            .execute(
                "INSERT INTO entities(namespace, local_id, payload, version, updated_at_ms)
                 VALUES ('ns', 'e1', x'00', 1, 0)",
                [],
            )
            .unwrap();
        connection
            .execute(
                "INSERT INTO entity_views(namespace, view_id, local_id, position_key, version)
                 VALUES ('ns', 'v1', 'e1', x'01', 1)",
                [],
            )
            .unwrap();

        let count: i64 = connection
            .query_row(
                "SELECT row_count FROM entity_view_counts WHERE namespace='ns' AND view_id='v1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        connection
            .execute("DELETE FROM entity_views WHERE local_id='e1'", [])
            .unwrap();
        let remaining: i64 = connection
            .query_row("SELECT count(*) FROM entity_view_counts", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(remaining, 0, "an emptied view must not leave a zero row");
    }

    #[test]
    fn deleting_an_entity_cascades_to_its_views_and_aliases() {
        let connection = open_in_memory().unwrap();
        prepare(&connection, "u1", &PlaintextCipher).unwrap();
        connection
            .execute_batch(
                "INSERT INTO entities(namespace, local_id, payload, version, updated_at_ms)
                    VALUES ('ns','e1',x'00',1,0);
                 INSERT INTO entity_aliases(namespace, alias_kind, alias_value, local_id)
                    VALUES ('ns','server','s1','e1');
                 INSERT INTO entity_views(namespace, view_id, local_id, position_key, version)
                    VALUES ('ns','v1','e1',x'01',1);
                 DELETE FROM entities WHERE local_id='e1';",
            )
            .unwrap();

        let aliases: i64 = connection
            .query_row("SELECT count(*) FROM entity_aliases", [], |r| r.get(0))
            .unwrap();
        let views: i64 = connection
            .query_row("SELECT count(*) FROM entity_views", [], |r| r.get(0))
            .unwrap();
        assert_eq!((aliases, views), (0, 0));
    }

    fn rand_seq() -> i64 {
        use std::sync::atomic::{AtomicI64, Ordering};
        static NEXT: AtomicI64 = AtomicI64::new(1);
        NEXT.fetch_add(1, Ordering::Relaxed)
    }
}
