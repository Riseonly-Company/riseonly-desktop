use rusqlite::{Connection, OptionalExtension, params};

use super::rise_store_cipher::{ValueCipher, cipher_context};
use super::rise_store_models::{
    CacheSource, LoadState, LocalId, Namespace, PositionKey, StoredEntity, ViewId, ViewRow,
    ViewState,
};
use super::rise_store_writes::WriteError;

pub struct StoredValue {
    pub plaintext: Vec<u8>,
    pub version: i64,
    pub source: CacheSource,
}

pub fn read_value(
    connection: &Connection,
    account_id: &str,
    cipher: &dyn ValueCipher,
    namespace: &Namespace,
    key: &str,
) -> Result<Option<StoredValue>, WriteError> {
    let row: Option<(Vec<u8>, i64)> = connection
        .query_row(
            "SELECT value, version FROM key_values WHERE namespace = ?1 AND key = ?2",
            params![namespace.as_str(), key],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;

    let Some((sealed, version)) = row else {
        return Ok(None);
    };

    let context = cipher_context(account_id, "key_values", &[namespace.as_str(), key]);
    Ok(Some(StoredValue {
        plaintext: cipher.open(&sealed, &context)?,
        version,
        source: CacheSource::PersistentStore,
    }))
}

pub fn read_entity(
    connection: &Connection,
    account_id: &str,
    cipher: &dyn ValueCipher,
    namespace: &Namespace,
    local_id: &LocalId,
) -> Result<Option<StoredEntity>, WriteError> {
    let row: Option<(Vec<u8>, i64)> = connection
        .query_row(
            "SELECT payload, version FROM entities WHERE namespace = ?1 AND local_id = ?2",
            params![namespace.as_str(), local_id.as_str()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;

    let Some((sealed, version)) = row else {
        return Ok(None);
    };

    let context = cipher_context(
        account_id,
        "entities",
        &[namespace.as_str(), local_id.as_str()],
    );
    Ok(Some(StoredEntity {
        local_id: local_id.clone(),
        payload: cipher.open(&sealed, &context)?,
        version,
    }))
}

pub fn resolve_alias(
    connection: &Connection,
    namespace: &Namespace,
    kind: &str,
    value: &str,
) -> Result<Option<LocalId>, WriteError> {
    let found: Option<String> = connection
        .query_row(
            "SELECT local_id FROM entity_aliases
             WHERE namespace = ?1 AND alias_kind = ?2 AND alias_value = ?3",
            params![namespace.as_str(), kind, value],
            |row| row.get(0),
        )
        .optional()?;

    Ok(found.map(LocalId::new))
}

/// Reads a window of a materialized view in position order.
///
/// Ordering is `position_key, local_id` — the same order the index is declared
/// in — so two rows sharing a position still come back in a stable sequence
/// rather than whatever the page cache happened to hold.
pub fn read_view_window(
    connection: &Connection,
    account_id: &str,
    cipher: &dyn ValueCipher,
    namespace: &Namespace,
    view_id: &ViewId,
    limit: usize,
    after: Option<&PositionKey>,
) -> Result<Vec<ViewRow>, WriteError> {
    let mut statement = connection.prepare(
        "SELECT v.local_id, v.position_key, v.version, e.payload
         FROM entity_views v
         JOIN entities e ON e.namespace = v.namespace AND e.local_id = v.local_id
         WHERE v.namespace = ?1 AND v.view_id = ?2
           AND (?3 IS NULL OR v.position_key > ?3)
         ORDER BY v.position_key, v.local_id
         LIMIT ?4",
    )?;

    let rows = statement.query_map(
        params![
            namespace.as_str(),
            view_id.as_str(),
            after.map(PositionKey::as_bytes),
            limit as i64
        ],
        |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Vec<u8>>(3)?,
            ))
        },
    )?;

    let mut out = Vec::with_capacity(limit.min(64));
    for row in rows {
        let (local_id, position, version, sealed) = row?;
        let context = cipher_context(account_id, "entities", &[namespace.as_str(), &local_id]);
        out.push(ViewRow {
            local_id: LocalId::new(local_id),
            position: PositionKey::new(position).expect("schema forbids an empty position key"),
            payload: cipher.open(&sealed, &context)?,
            version,
        });
    }

    Ok(out)
}

pub fn view_row_count(
    connection: &Connection,
    namespace: &Namespace,
    view_id: &ViewId,
) -> Result<i64, WriteError> {
    let count: Option<i64> = connection
        .query_row(
            "SELECT row_count FROM entity_view_counts WHERE namespace = ?1 AND view_id = ?2",
            params![namespace.as_str(), view_id.as_str()],
            |row| row.get(0),
        )
        .optional()?;

    Ok(count.unwrap_or(0))
}

pub fn read_view_state(
    connection: &Connection,
    namespace: &Namespace,
    view_id: &ViewId,
) -> Result<ViewState, WriteError> {
    let row = connection
        .query_row(
            "SELECT revision, before_cursor, after_cursor, has_more_before, has_more_after,
                    load_state, last_error_code
             FROM entity_view_state WHERE namespace = ?1 AND view_id = ?2",
            params![namespace.as_str(), view_id.as_str()],
            |row| {
                Ok(ViewState {
                    revision: row.get(0)?,
                    before_cursor: row.get(1)?,
                    after_cursor: row.get(2)?,
                    has_more_before: row.get::<_, i64>(3)? != 0,
                    has_more_after: row.get::<_, i64>(4)? != 0,
                    load_state: LoadState::parse(&row.get::<_, String>(5)?).unwrap_or_default(),
                    last_error_code: row.get(6)?,
                })
            },
        )
        .optional()?;

    Ok(row.unwrap_or_default())
}

pub fn write_view_state(
    connection: &Connection,
    namespace: &Namespace,
    view_id: &ViewId,
    state: &ViewState,
    now_ms: i64,
) -> Result<(), WriteError> {
    connection.execute(
        "INSERT INTO entity_view_state(namespace, view_id, revision, before_cursor, after_cursor,
             has_more_before, has_more_after, load_state, last_error_code, updated_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
         ON CONFLICT(namespace, view_id) DO UPDATE SET
             revision = excluded.revision,
             before_cursor = excluded.before_cursor,
             after_cursor = excluded.after_cursor,
             has_more_before = excluded.has_more_before,
             has_more_after = excluded.has_more_after,
             load_state = excluded.load_state,
             last_error_code = excluded.last_error_code,
             updated_at_ms = excluded.updated_at_ms",
        params![
            namespace.as_str(),
            view_id.as_str(),
            state.revision,
            state.before_cursor,
            state.after_cursor,
            state.has_more_before as i64,
            state.has_more_after as i64,
            state.load_state.as_str(),
            state.last_error_code,
            now_ms
        ],
    )?;

    Ok(())
}

pub fn view_revision(connection: &Connection, view_key: &str) -> Result<i64, WriteError> {
    let revision: Option<i64> = connection
        .query_row(
            "SELECT revision FROM view_revisions WHERE view_key = ?1",
            params![view_key],
            |row| row.get(0),
        )
        .optional()?;

    Ok(revision.unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rise_store::rise_sqlite::{open_in_memory, prepare};
    use crate::rise_store::rise_store_cipher::{AesGcmCipher, PlaintextCipher};
    use crate::rise_store::rise_store_writes::WriteBatch;

    fn ns() -> Namespace {
        Namespace::new("conversation.messages.v1")
    }

    fn view() -> ViewId {
        ViewId::new("message.timeline:chat1")
    }

    fn seeded() -> Connection {
        let connection = open_in_memory().unwrap();
        prepare(&connection, "u1", &PlaintextCipher).unwrap();
        let mut connection = connection;

        let mut batch = WriteBatch::new();
        for (index, id) in ["m1", "m2", "m3"].iter().enumerate() {
            let local = LocalId::new(*id);
            batch
                .put_entity(&ns(), &local, format!("payload-{id}").into_bytes())
                .put_view_row(
                    &ns(),
                    &view(),
                    &local,
                    PositionKey::from_ordinal(index as i64),
                );
        }
        batch.put_alias(&ns(), "server", "s-1", &LocalId::new("m1"));
        batch
            .commit(&mut connection, "u1", &PlaintextCipher, 0)
            .unwrap();

        connection
    }

    #[test]
    fn a_missing_value_reads_as_none_rather_than_an_error() {
        let connection = seeded();
        let found = read_value(&connection, "u1", &PlaintextCipher, &ns(), "absent").unwrap();
        assert!(found.is_none());
    }

    #[test]
    fn a_value_round_trips_through_the_cipher() {
        let connection = open_in_memory().unwrap();
        let cipher = AesGcmCipher::new(&[5u8; 32]).unwrap();
        prepare(&connection, "u1", &cipher).unwrap();
        let mut connection = connection;

        let mut batch = WriteBatch::new();
        batch.put_value(&ns(), "token", b"secret".to_vec());
        batch.commit(&mut connection, "u1", &cipher, 0).unwrap();

        let found = read_value(&connection, "u1", &cipher, &ns(), "token")
            .unwrap()
            .unwrap();
        assert_eq!(found.plaintext, b"secret");
        assert_eq!(found.version, 1);
        assert_eq!(found.source, CacheSource::PersistentStore);
    }

    #[test]
    fn another_accounts_context_cannot_open_a_stored_value() {
        let connection = open_in_memory().unwrap();
        let cipher = AesGcmCipher::new(&[5u8; 32]).unwrap();
        prepare(&connection, "u1", &cipher).unwrap();
        let mut connection = connection;

        let mut batch = WriteBatch::new();
        batch.put_value(&ns(), "token", b"secret".to_vec());
        batch.commit(&mut connection, "u1", &cipher, 0).unwrap();

        assert!(
            read_value(&connection, "u2", &cipher, &ns(), "token").is_err(),
            "a value must not open under a different account"
        );
    }

    #[test]
    fn an_entity_reads_back_with_its_payload_and_version() {
        let connection = seeded();
        let found = read_entity(
            &connection,
            "u1",
            &PlaintextCipher,
            &ns(),
            &LocalId::new("m2"),
        )
        .unwrap()
        .unwrap();

        assert_eq!(found.payload, b"payload-m2");
        assert_eq!(found.version, 1);
    }

    #[test]
    fn an_alias_resolves_to_its_entity() {
        let connection = seeded();
        assert_eq!(
            resolve_alias(&connection, &ns(), "server", "s-1").unwrap(),
            Some(LocalId::new("m1"))
        );
        assert_eq!(
            resolve_alias(&connection, &ns(), "server", "missing").unwrap(),
            None
        );
    }

    #[test]
    fn a_view_window_comes_back_in_position_order() {
        let connection = seeded();
        let rows = read_view_window(
            &connection,
            "u1",
            &PlaintextCipher,
            &ns(),
            &view(),
            10,
            None,
        )
        .unwrap();

        let ids: Vec<&str> = rows.iter().map(|r| r.local_id.as_str()).collect();
        assert_eq!(ids, vec!["m1", "m2", "m3"]);
        assert_eq!(rows[0].payload, b"payload-m1");
    }

    #[test]
    fn a_window_is_bounded_and_resumes_after_a_cursor() {
        let connection = seeded();

        let first =
            read_view_window(&connection, "u1", &PlaintextCipher, &ns(), &view(), 2, None).unwrap();
        assert_eq!(first.len(), 2);

        let next = read_view_window(
            &connection,
            "u1",
            &PlaintextCipher,
            &ns(),
            &view(),
            2,
            Some(&first[1].position),
        )
        .unwrap();

        assert_eq!(next.len(), 1);
        assert_eq!(next[0].local_id.as_str(), "m3");
    }

    #[test]
    fn the_row_count_is_maintained_without_scanning() {
        let connection = seeded();
        assert_eq!(view_row_count(&connection, &ns(), &view()).unwrap(), 3);
        assert_eq!(
            view_row_count(&connection, &ns(), &ViewId::new("empty")).unwrap(),
            0
        );
    }

    #[test]
    fn a_view_with_no_stored_state_reads_as_the_default() {
        let connection = seeded();
        let state = read_view_state(&connection, &ns(), &view()).unwrap();
        assert_eq!(state, ViewState::default());
    }

    #[test]
    fn view_state_round_trips_including_cursors_and_load_state() {
        let connection = seeded();
        let written = ViewState {
            revision: 7,
            before_cursor: Some(b"before".to_vec()),
            after_cursor: None,
            has_more_before: true,
            has_more_after: false,
            load_state: LoadState::Failed,
            last_error_code: Some("network".into()),
        };

        write_view_state(&connection, &ns(), &view(), &written, 1_000).unwrap();
        assert_eq!(
            read_view_state(&connection, &ns(), &view()).unwrap(),
            written
        );
    }

    #[test]
    fn a_revision_starts_at_zero_and_advances_only_when_bumped() {
        let connection = seeded();
        let mut connection = connection;
        assert_eq!(view_revision(&connection, "timeline").unwrap(), 0);

        let mut batch = WriteBatch::new();
        batch.bump_view_revision("timeline");
        batch
            .commit(&mut connection, "u1", &PlaintextCipher, 0)
            .unwrap();

        assert_eq!(view_revision(&connection, "timeline").unwrap(), 1);
    }

    #[test]
    fn a_deleted_entity_disappears_from_its_view_window() {
        let mut connection = seeded();

        let mut batch = WriteBatch::new();
        batch.remove_entity(&ns(), &LocalId::new("m2"));
        batch
            .commit(&mut connection, "u1", &PlaintextCipher, 0)
            .unwrap();

        let rows = read_view_window(
            &connection,
            "u1",
            &PlaintextCipher,
            &ns(),
            &view(),
            10,
            None,
        )
        .unwrap();
        let ids: Vec<&str> = rows.iter().map(|r| r.local_id.as_str()).collect();

        assert_eq!(ids, vec!["m1", "m3"]);
        assert_eq!(view_row_count(&connection, &ns(), &view()).unwrap(), 2);
    }
}
