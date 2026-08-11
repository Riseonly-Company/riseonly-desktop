use rusqlite::{Connection, params};

use super::rise_store_cipher::{ValueCipher, cipher_context};
use super::rise_store_models::{LocalId, Namespace, PositionKey, ViewId};

/// One atomic unit of change: an entity and every projection derived from it must
/// be collected here so they land in a single transaction, never one at a time.
#[derive(Default)]
pub struct WriteBatch {
    operations: Vec<Operation>,
}

enum Operation {
    PutValue {
        namespace: Namespace,
        key: String,
        plaintext: Vec<u8>,
    },
    RemoveValue {
        namespace: Namespace,
        key: String,
    },
    PutEntity {
        namespace: Namespace,
        local_id: LocalId,
        payload: Vec<u8>,
    },
    RemoveEntity {
        namespace: Namespace,
        local_id: LocalId,
    },
    PutAlias {
        namespace: Namespace,
        kind: String,
        value: String,
        local_id: LocalId,
    },
    PutViewRow {
        namespace: Namespace,
        view_id: ViewId,
        local_id: LocalId,
        position: PositionKey,
    },
    RemoveViewRow {
        namespace: Namespace,
        view_id: ViewId,
        local_id: LocalId,
    },
    BumpViewRevision {
        view_key: String,
    },
}

impl WriteBatch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.operations.is_empty()
    }

    pub fn len(&self) -> usize {
        self.operations.len()
    }

    pub fn put_value(
        &mut self,
        namespace: &Namespace,
        key: impl Into<String>,
        plaintext: impl Into<Vec<u8>>,
    ) -> &mut Self {
        self.operations.push(Operation::PutValue {
            namespace: namespace.clone(),
            key: key.into(),
            plaintext: plaintext.into(),
        });
        self
    }

    pub fn remove_value(&mut self, namespace: &Namespace, key: impl Into<String>) -> &mut Self {
        self.operations.push(Operation::RemoveValue {
            namespace: namespace.clone(),
            key: key.into(),
        });
        self
    }

    pub fn put_entity(
        &mut self,
        namespace: &Namespace,
        local_id: &LocalId,
        payload: impl Into<Vec<u8>>,
    ) -> &mut Self {
        self.operations.push(Operation::PutEntity {
            namespace: namespace.clone(),
            local_id: local_id.clone(),
            payload: payload.into(),
        });
        self
    }

    pub fn remove_entity(&mut self, namespace: &Namespace, local_id: &LocalId) -> &mut Self {
        self.operations.push(Operation::RemoveEntity {
            namespace: namespace.clone(),
            local_id: local_id.clone(),
        });
        self
    }

    pub fn put_alias(
        &mut self,
        namespace: &Namespace,
        kind: impl Into<String>,
        value: impl Into<String>,
        local_id: &LocalId,
    ) -> &mut Self {
        self.operations.push(Operation::PutAlias {
            namespace: namespace.clone(),
            kind: kind.into(),
            value: value.into(),
            local_id: local_id.clone(),
        });
        self
    }

    pub fn put_view_row(
        &mut self,
        namespace: &Namespace,
        view_id: &ViewId,
        local_id: &LocalId,
        position: PositionKey,
    ) -> &mut Self {
        self.operations.push(Operation::PutViewRow {
            namespace: namespace.clone(),
            view_id: view_id.clone(),
            local_id: local_id.clone(),
            position,
        });
        self
    }

    pub fn remove_view_row(
        &mut self,
        namespace: &Namespace,
        view_id: &ViewId,
        local_id: &LocalId,
    ) -> &mut Self {
        self.operations.push(Operation::RemoveViewRow {
            namespace: namespace.clone(),
            view_id: view_id.clone(),
            local_id: local_id.clone(),
        });
        self
    }

    pub fn bump_view_revision(&mut self, view_key: impl Into<String>) -> &mut Self {
        self.operations.push(Operation::BumpViewRevision {
            view_key: view_key.into(),
        });
        self
    }

    /// Applies every operation inside one transaction; a failure anywhere rolls
    /// the whole batch back.
    pub fn commit(
        self,
        connection: &mut Connection,
        account_id: &str,
        cipher: &dyn ValueCipher,
        now_ms: i64,
    ) -> Result<usize, WriteError> {
        if self.operations.is_empty() {
            return Ok(0);
        }

        let applied = self.operations.len();
        let transaction = connection.transaction()?;

        for operation in &self.operations {
            apply(&transaction, account_id, cipher, now_ms, operation)?;
        }

        transaction.commit()?;
        Ok(applied)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("cipher: {0}")]
    Cipher(#[from] super::rise_store_cipher::CipherError),
}

fn apply(
    tx: &rusqlite::Transaction<'_>,
    account_id: &str,
    cipher: &dyn ValueCipher,
    now_ms: i64,
    operation: &Operation,
) -> Result<(), WriteError> {
    match operation {
        Operation::PutValue {
            namespace,
            key,
            plaintext,
        } => {
            let context = cipher_context(account_id, "key_values", &[namespace.as_str(), key]);
            let sealed = cipher.seal(plaintext, &context)?;
            tx.execute(
                "INSERT INTO key_values(namespace, key, value, version, updated_at_ms)
                 VALUES (?1, ?2, ?3, 1, ?4)
                 ON CONFLICT(namespace, key) DO UPDATE SET
                     value = excluded.value,
                     version = key_values.version + 1,
                     updated_at_ms = excluded.updated_at_ms",
                params![namespace.as_str(), key, sealed, now_ms],
            )?;
        }

        Operation::RemoveValue { namespace, key } => {
            tx.execute(
                "DELETE FROM key_values WHERE namespace = ?1 AND key = ?2",
                params![namespace.as_str(), key],
            )?;
        }

        Operation::PutEntity {
            namespace,
            local_id,
            payload,
        } => {
            let context = cipher_context(
                account_id,
                "entities",
                &[namespace.as_str(), local_id.as_str()],
            );
            let sealed = cipher.seal(payload, &context)?;
            tx.execute(
                "INSERT INTO entities(namespace, local_id, payload, version, updated_at_ms)
                 VALUES (?1, ?2, ?3, 1, ?4)
                 ON CONFLICT(namespace, local_id) DO UPDATE SET
                     payload = excluded.payload,
                     version = entities.version + 1,
                     updated_at_ms = excluded.updated_at_ms",
                params![namespace.as_str(), local_id.as_str(), sealed, now_ms],
            )?;
        }

        Operation::RemoveEntity {
            namespace,
            local_id,
        } => {
            tx.execute(
                "DELETE FROM entities WHERE namespace = ?1 AND local_id = ?2",
                params![namespace.as_str(), local_id.as_str()],
            )?;
        }

        Operation::PutAlias {
            namespace,
            kind,
            value,
            local_id,
        } => {
            tx.execute(
                "INSERT INTO entity_aliases(namespace, alias_kind, alias_value, local_id)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(namespace, alias_kind, alias_value) DO UPDATE SET
                     local_id = excluded.local_id",
                params![namespace.as_str(), kind, value, local_id.as_str()],
            )?;
        }

        Operation::PutViewRow {
            namespace,
            view_id,
            local_id,
            position,
        } => {
            tx.execute(
                "INSERT INTO entity_views(namespace, view_id, local_id, position_key, version)
                 VALUES (?1, ?2, ?3, ?4, 1)
                 ON CONFLICT(namespace, view_id, local_id) DO UPDATE SET
                     position_key = excluded.position_key,
                     version = entity_views.version + 1",
                params![
                    namespace.as_str(),
                    view_id.as_str(),
                    local_id.as_str(),
                    position.as_bytes()
                ],
            )?;
        }

        Operation::RemoveViewRow {
            namespace,
            view_id,
            local_id,
        } => {
            tx.execute(
                "DELETE FROM entity_views
                 WHERE namespace = ?1 AND view_id = ?2 AND local_id = ?3",
                params![namespace.as_str(), view_id.as_str(), local_id.as_str()],
            )?;
        }

        Operation::BumpViewRevision { view_key } => {
            tx.execute(
                "INSERT INTO view_revisions(view_key, revision) VALUES (?1, 1)
                 ON CONFLICT(view_key) DO UPDATE SET revision = view_revisions.revision + 1",
                params![view_key],
            )?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rise_store::rise_sqlite::{open_in_memory, prepare};
    use crate::rise_store::rise_store_cipher::PlaintextCipher;

    fn store() -> Connection {
        let connection = open_in_memory().unwrap();
        prepare(&connection, "u1", &PlaintextCipher).unwrap();
        connection
    }

    fn ns() -> Namespace {
        Namespace::new("conversation.chats.v1")
    }

    fn commit(batch: WriteBatch, connection: &mut Connection) -> usize {
        batch
            .commit(connection, "u1", &PlaintextCipher, 1_000)
            .unwrap()
    }

    #[test]
    fn an_empty_batch_touches_nothing() {
        let mut connection = store();
        assert_eq!(commit(WriteBatch::new(), &mut connection), 0);
    }

    #[test]
    fn a_value_is_written_and_versioned_on_each_write() {
        let mut connection = store();

        let mut batch = WriteBatch::new();
        batch.put_value(&ns(), "settings", b"first".to_vec());
        commit(batch, &mut connection);

        let mut batch = WriteBatch::new();
        batch.put_value(&ns(), "settings", b"second".to_vec());
        commit(batch, &mut connection);

        let (value, version): (Vec<u8>, i64) = connection
            .query_row(
                "SELECT value, version FROM key_values WHERE key = 'settings'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();

        assert_eq!(value, b"second");
        assert_eq!(version, 2, "an overwrite must advance the version");
    }

    #[test]
    fn an_entity_and_its_projections_land_in_one_transaction() {
        let mut connection = store();
        let entity = LocalId::new("m1");
        let view = ViewId::new("message.timeline:chat1");

        let mut batch = WriteBatch::new();
        batch
            .put_entity(&ns(), &entity, b"payload".to_vec())
            .put_alias(&ns(), "server", "server-1", &entity)
            .put_view_row(&ns(), &view, &entity, PositionKey::from_ordinal(10))
            .bump_view_revision("message.timeline:chat1");
        assert_eq!(commit(batch, &mut connection), 4);

        let entities: i64 = connection
            .query_row("SELECT count(*) FROM entities", [], |r| r.get(0))
            .unwrap();
        let aliases: i64 = connection
            .query_row("SELECT count(*) FROM entity_aliases", [], |r| r.get(0))
            .unwrap();
        let rows: i64 = connection
            .query_row("SELECT count(*) FROM entity_views", [], |r| r.get(0))
            .unwrap();
        let revision: i64 = connection
            .query_row("SELECT revision FROM view_revisions", [], |r| r.get(0))
            .unwrap();

        assert_eq!((entities, aliases, rows, revision), (1, 1, 1, 1));
    }

    #[test]
    fn a_failing_operation_rolls_the_whole_batch_back() {
        let mut connection = store();
        let known = LocalId::new("m1");
        let unknown = LocalId::new("ghost");

        let mut batch = WriteBatch::new();
        batch
            .put_entity(&ns(), &known, b"payload".to_vec())
            .put_view_row(
                &ns(),
                &ViewId::new("v1"),
                &unknown,
                PositionKey::from_ordinal(1),
            );

        assert!(
            batch
                .commit(&mut connection, "u1", &PlaintextCipher, 1_000)
                .is_err(),
            "a view row for a missing entity violates the foreign key"
        );

        let entities: i64 = connection
            .query_row("SELECT count(*) FROM entities", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            entities, 0,
            "the successful half of a failed batch must not survive"
        );
    }

    #[test]
    fn removing_an_entity_takes_its_view_rows_with_it() {
        let mut connection = store();
        let entity = LocalId::new("m1");
        let view = ViewId::new("v1");

        let mut batch = WriteBatch::new();
        batch
            .put_entity(&ns(), &entity, b"payload".to_vec())
            .put_view_row(&ns(), &view, &entity, PositionKey::from_ordinal(1));
        commit(batch, &mut connection);

        let mut batch = WriteBatch::new();
        batch.remove_entity(&ns(), &entity);
        commit(batch, &mut connection);

        let rows: i64 = connection
            .query_row("SELECT count(*) FROM entity_views", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0);
    }

    #[test]
    fn an_alias_can_be_repointed_when_a_temp_id_becomes_real() {
        let mut connection = store();
        let temp = LocalId::new("temp-1");
        let real = LocalId::new("m1");

        let mut batch = WriteBatch::new();
        batch
            .put_entity(&ns(), &temp, b"optimistic".to_vec())
            .put_entity(&ns(), &real, b"authoritative".to_vec())
            .put_alias(&ns(), "server", "server-1", &temp);
        commit(batch, &mut connection);

        let mut batch = WriteBatch::new();
        batch.put_alias(&ns(), "server", "server-1", &real);
        commit(batch, &mut connection);

        let target: String = connection
            .query_row(
                "SELECT local_id FROM entity_aliases WHERE alias_value = 'server-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(target, "m1");
    }

    #[test]
    fn a_view_row_moves_rather_than_duplicating_when_its_position_changes() {
        let mut connection = store();
        let entity = LocalId::new("m1");
        let view = ViewId::new("v1");

        let mut batch = WriteBatch::new();
        batch
            .put_entity(&ns(), &entity, b"payload".to_vec())
            .put_view_row(&ns(), &view, &entity, PositionKey::from_ordinal(1));
        commit(batch, &mut connection);

        let mut batch = WriteBatch::new();
        batch.put_view_row(&ns(), &view, &entity, PositionKey::from_ordinal(99));
        commit(batch, &mut connection);

        let (rows, version): (i64, i64) = connection
            .query_row(
                "SELECT count(*), max(version) FROM entity_views",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(rows, 1);
        assert_eq!(version, 2);
    }

    #[test]
    fn values_are_sealed_on_disk() {
        use crate::rise_store::rise_store_cipher::AesGcmCipher;

        let connection = open_in_memory().unwrap();
        let cipher = AesGcmCipher::new(&[3u8; 32]).unwrap();
        prepare(&connection, "u1", &cipher).unwrap();
        let mut connection = connection;

        let mut batch = WriteBatch::new();
        batch.put_value(&ns(), "token", b"super-secret".to_vec());
        batch.commit(&mut connection, "u1", &cipher, 0).unwrap();

        let stored: Vec<u8> = connection
            .query_row("SELECT value FROM key_values", [], |row| row.get(0))
            .unwrap();

        assert_ne!(stored, b"super-secret");
        assert!(
            !stored
                .windows(b"super-secret".len())
                .any(|w| w == b"super-secret"),
            "plaintext must not appear anywhere in the stored bytes"
        );
    }
}
