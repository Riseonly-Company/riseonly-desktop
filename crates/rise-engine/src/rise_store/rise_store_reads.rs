use ::rise_store::{CipherContextKind, IdentifierKind};

use super::rise_sq_lite::{RiseSQLiteConnection, RiseSQLiteStatement};
use super::rise_store::RiseStore;
use super::rise_store_concurrency::RiseStoreReadBudget;
use super::rise_store_models::{
    RiseCollectionAnchor, RiseCollectionState, RiseDurableOperation, RiseDurableOperationState,
    RiseDurableOrderingKey, RiseOrderedItem, RiseStoreError, RiseStoreNamespace, RiseStoredValue,
};
use crate::rise_core::rise_identifiers::RiseRequestID;

fn corrupted(label: &str) -> RiseStoreError {
    RiseStoreError::CorruptedValue(label.to_owned())
}

fn identifier_text(text: Option<String>, kind: IdentifierKind) -> Option<String> {
    text.filter(|text| !text.is_empty() && text.len() <= kind.maximum_bytes())
}

/// columns of one queue row that must be valid before the payload is opened
struct DurableRow {
    request_text: String,
    request_id: RiseRequestID,
    ordering_key: RiseDurableOrderingKey,
    kind: String,
    sealed: Vec<u8>,
    state: RiseDurableOperationState,
    attempt_count: i64,
}

fn durable_row(statement: &RiseSQLiteStatement) -> Option<DurableRow> {
    let request_text = statement.text(0)?;
    let request_raw = request_text.parse::<u64>().ok().filter(|raw| *raw > 0)?;
    let request_id = RiseRequestID::new(request_raw)?;
    let ordering_text = identifier_text(statement.text(1), IdentifierKind::OrderingKey)?;
    let kind = identifier_text(statement.text(2), IdentifierKind::OperationKind)?;
    let sealed = statement.data(3)?;
    let state = statement
        .text(4)
        .and_then(|text| RiseDurableOperationState::from_raw(&text))?;
    let attempt_count = statement.int64(5);
    if attempt_count < 0 {
        return None;
    }
    Some(DurableRow {
        request_text,
        request_id,
        ordering_key: RiseDurableOrderingKey::new(ordering_text),
        kind,
        sealed,
        state,
        attempt_count,
    })
}

fn validate_stored_blob_size(
    stored_bytes: i64,
    plaintext_limit: i64,
) -> Result<(), RiseStoreError> {
    let maximum_cipher_overhead: i64 = 128;
    match plaintext_limit.checked_add(maximum_cipher_overhead) {
        Some(maximum_stored_bytes) if stored_bytes <= maximum_stored_bytes => Ok(()),
        _ => Err(corrupted("oversized_encrypted_value")),
    }
}

impl RiseStore {
    pub(crate) fn read_value(
        &self,
        namespace: &RiseStoreNamespace,
        key: &str,
        budget: Option<&mut RiseStoreReadBudget>,
        database: &RiseSQLiteConnection,
    ) -> Result<Option<RiseStoredValue>, RiseStoreError> {
        let mut statement = database.statement(
            "SELECT value, version, updated_at_ms FROM key_values WHERE namespace = ? AND key = ?",
            "value.read",
        )?;
        statement.bind(namespace.raw_value(), 1)?;
        statement.bind(key, 2)?;
        if !statement.step()? {
            return Ok(None);
        }
        let stored_bytes = statement.byte_count(0);
        validate_stored_blob_size(stored_bytes, self.configuration.limits.maximum_value_bytes)?;
        let Some(sealed) = statement.data(0) else {
            return Err(corrupted("key_value"));
        };
        let context = self.cipher_context(
            CipherContextKind::KeyValue.domain(),
            &[namespace.raw_value(), key],
        );
        let data = self.configuration.cipher.open(&sealed, &context)?;
        if let Some(budget) = budget {
            budget.consume(data.len())?;
        }
        Ok(Some(RiseStoredValue::new(
            data,
            statement.int64(1),
            statement.int64(2),
        )))
    }

    pub(crate) fn read_ordered_item(
        &self,
        namespace: &RiseStoreNamespace,
        collection: &str,
        item_id: &str,
        budget: Option<&mut RiseStoreReadBudget>,
        database: &RiseSQLiteConnection,
    ) -> Result<Option<RiseOrderedItem>, RiseStoreError> {
        let mut statement = database.statement(
            "SELECT item_id, server_id, sort_key, value, version, updated_at_ms \
             FROM ordered_items \
             WHERE namespace = ? AND collection_key = ? AND item_id = ?",
            "ordered_item.read",
        )?;
        statement.bind(namespace.raw_value(), 1)?;
        statement.bind(collection, 2)?;
        statement.bind(item_id, 3)?;
        if !statement.step()? {
            return Ok(None);
        }
        self.decode_ordered_item(&statement, namespace, collection, budget)
            .map(Some)
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn read_ordered_items(
        &self,
        namespace: &RiseStoreNamespace,
        collection: &str,
        limit: i64,
        ascending: bool,
        anchor: Option<&RiseCollectionAnchor>,
        mut budget: Option<&mut RiseStoreReadBudget>,
        database: &RiseSQLiteConnection,
    ) -> Result<Vec<RiseOrderedItem>, RiseStoreError> {
        let direction = if ascending { "ASC" } else { "DESC" };
        let anchor_predicate = match anchor {
            None => "",
            Some(RiseCollectionAnchor::Before(_)) => {
                "AND (sort_key < ? OR (sort_key = ? AND item_id < ?))"
            }
            Some(RiseCollectionAnchor::After(_)) => {
                "AND (sort_key > ? OR (sort_key = ? AND item_id > ?))"
            }
        };
        let sql = format!(
            "SELECT item_id, server_id, sort_key, value, version, updated_at_ms \
             FROM ordered_items \
             WHERE namespace = ? AND collection_key = ? \
             {anchor_predicate} \
             ORDER BY sort_key {direction}, item_id {direction} \
             LIMIT ?"
        );
        let mut statement = database.statement(&sql, "ordered_items.read")?;
        statement.bind(namespace.raw_value(), 1)?;
        statement.bind(collection, 2)?;
        let limit_index: i32 = match anchor {
            None => 3,
            Some(RiseCollectionAnchor::Before(position))
            | Some(RiseCollectionAnchor::After(position)) => {
                statement.bind(position.sort_key, 3)?;
                statement.bind(position.sort_key, 4)?;
                statement.bind(position.item_id.as_str(), 5)?;
                6
            }
        };
        statement.bind(limit, limit_index)?;

        let capacity = usize::try_from(limit.clamp(0, 256)).unwrap_or(0);
        let mut result: Vec<RiseOrderedItem> = Vec::with_capacity(capacity);
        while statement.step()? {
            result.push(self.decode_ordered_item(
                &statement,
                namespace,
                collection,
                budget.as_deref_mut(),
            )?);
        }
        Ok(result)
    }

    pub(crate) fn decode_ordered_item(
        &self,
        statement: &RiseSQLiteStatement,
        namespace: &RiseStoreNamespace,
        collection: &str,
        budget: Option<&mut RiseStoreReadBudget>,
    ) -> Result<RiseOrderedItem, RiseStoreError> {
        let stored_bytes = statement.byte_count(3);
        validate_stored_blob_size(stored_bytes, self.configuration.limits.maximum_value_bytes)?;
        let Some(item_id) = identifier_text(statement.text(0), IdentifierKind::ItemId) else {
            return Err(corrupted("ordered_item"));
        };
        let Some(sealed) = statement.data(3) else {
            return Err(corrupted("ordered_item"));
        };
        let context = self.cipher_context(
            CipherContextKind::OrderedItem.domain(),
            &[namespace.raw_value(), collection, item_id.as_str()],
        );
        let data = self.configuration.cipher.open(&sealed, &context)?;
        if let Some(budget) = budget {
            budget.consume(data.len())?;
        }
        Ok(RiseOrderedItem::new(
            item_id,
            statement.text(1),
            statement.int64(2),
            data,
            statement.int64(4),
            statement.int64(5),
        ))
    }

    pub(crate) fn read_collection_state(
        &self,
        namespace: &RiseStoreNamespace,
        collection: &str,
        budget: Option<&mut RiseStoreReadBudget>,
        database: &RiseSQLiteConnection,
    ) -> Result<Option<RiseCollectionState>, RiseStoreError> {
        let mut statement = database.statement(
            "SELECT revision, before_cursor, after_cursor, has_more_before, has_more_after \
             FROM collection_state \
             WHERE namespace = ? AND collection_key = ?",
            "collection_state.read",
        )?;
        statement.bind(namespace.raw_value(), 1)?;
        statement.bind(collection, 2)?;
        if !statement.step()? {
            return Ok(None);
        }

        let before_bytes = statement.byte_count(1);
        let after_bytes = statement.byte_count(2);
        let Some(stored_bytes) = before_bytes.checked_add(after_bytes) else {
            return Err(corrupted("collection_cursor_size"));
        };
        validate_stored_blob_size(stored_bytes, self.configuration.limits.maximum_value_bytes)?;

        let before = match statement.data(1) {
            Some(sealed) => {
                let context = self.cipher_context(
                    CipherContextKind::CollectionCursorBefore.domain(),
                    &[namespace.raw_value(), collection],
                );
                Some(self.configuration.cipher.open(&sealed, &context)?)
            }
            None => None,
        };
        let after = match statement.data(2) {
            Some(sealed) => {
                let context = self.cipher_context(
                    CipherContextKind::CollectionCursorAfter.domain(),
                    &[namespace.raw_value(), collection],
                );
                Some(self.configuration.cipher.open(&sealed, &context)?)
            }
            None => None,
        };
        let Some(plaintext_bytes) = before
            .as_ref()
            .map_or(0, Vec::len)
            .checked_add(after.as_ref().map_or(0, Vec::len))
        else {
            return Err(corrupted("collection_cursor_size"));
        };
        if let Some(budget) = budget {
            budget.consume(plaintext_bytes)?;
        }
        Ok(Some(RiseCollectionState::new(
            statement.int64(0),
            before,
            after,
            statement.bool(3),
            statement.bool(4),
        )))
    }

    pub(crate) fn read_operation(
        &self,
        request_id: RiseRequestID,
        budget: Option<&mut RiseStoreReadBudget>,
        database: &RiseSQLiteConnection,
    ) -> Result<Option<RiseDurableOperation>, RiseStoreError> {
        let mut statement = database.statement(
            "SELECT request_id, ordering_key, kind, payload, state, attempt_count, \
                    next_attempt_ms, last_error_code, created_at_ms, updated_at_ms \
             FROM durable_operations WHERE request_id = ?",
            "durable.read",
        )?;
        statement.bind(request_id.raw_value().to_string().as_str(), 1)?;
        Ok(self
            .read_operations(&mut statement, budget)?
            .into_iter()
            .next())
    }

    pub(crate) fn read_operations(
        &self,
        statement: &mut RiseSQLiteStatement,
        mut budget: Option<&mut RiseStoreReadBudget>,
    ) -> Result<Vec<RiseDurableOperation>, RiseStoreError> {
        let mut result: Vec<RiseDurableOperation> = Vec::new();
        while statement.step()? {
            let stored_bytes = statement.byte_count(3);
            validate_stored_blob_size(
                stored_bytes,
                self.configuration.limits.maximum_durable_payload_bytes,
            )?;
            let Some(row) = durable_row(statement) else {
                return Err(corrupted("durable_operation"));
            };
            let context = self.cipher_context(
                CipherContextKind::DurableOperation.domain(),
                &[row.request_text.as_str()],
            );
            let payload = self.configuration.cipher.open(&row.sealed, &context)?;
            if let Some(budget) = budget.as_deref_mut() {
                budget.consume(payload.len())?;
            }
            result.push(RiseDurableOperation {
                request_id: row.request_id,
                kind: row.kind,
                payload,
                ordering_key: row.ordering_key,
                state: row.state,
                attempt_count: row.attempt_count,
                next_attempt_at_milliseconds: statement.int64(6),
                last_error_code: statement.text(7),
                created_at_milliseconds: statement.int64(8),
                updated_at_milliseconds: statement.int64(9),
            });
        }
        Ok(result)
    }
}
