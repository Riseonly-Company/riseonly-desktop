use rise_canon::rise_store::{CipherContextKind, IdentifierKind, StatementId};

use crate::rise_store::rise_sq_lite::{RiseSQLiteConnection, RiseSQLiteStatement};
use crate::rise_store::rise_store::RiseStore;
use crate::rise_store::rise_store_concurrency::RiseStoreReadBudget;
use crate::rise_store::rise_store_entities::{
    RiseEntityAlias, RiseEntityViewItem, RiseEntityViewKey, RiseEntityViewLoadState,
    RiseEntityViewPosition, RiseEntityViewState, RiseEntityWindow, RiseEntityWindowQuery,
    RiseEntityWindowResult, RisePositionKey, RiseStoreEntity,
};
use crate::rise_store::rise_store_models::{RiseStoreError, RiseStoreNamespace};

const ENCRYPTED_ENTITY_OVERHEAD_BYTES: i64 = 128;

const MAXIMUM_SLICE_RESERVATION: usize = 320;

enum EntityViewPredicate {
    Before,
    After,
}

impl RiseStore {
    pub(crate) fn read_entity_view_position(
        &self,
        namespace: &RiseStoreNamespace,
        view: &str,
        local_id: &str,
        database: &RiseSQLiteConnection,
    ) -> Result<Option<RiseEntityViewPosition>, RiseStoreError> {
        let mut statement = database.statement(
            "SELECT position_key FROM entity_views
            WHERE namespace = ? AND view_id = ? AND local_id = ?",
            "entity_view.read_position",
        )?;
        statement.bind(namespace.raw_value(), 1)?;
        statement.bind(view, 2)?;
        statement.bind(local_id, 3)?;
        if !statement.step()? {
            return Ok(None);
        }
        let Some(raw_position) = statement.data(0).filter(|raw| {
            !raw.is_empty()
                && raw.len() as i64 <= self.configuration.limits.maximum_position_key_bytes
        }) else {
            return Err(RiseStoreError::CorruptedValue(
                "entity_view_position".to_string(),
            ));
        };
        Ok(Some(RiseEntityViewPosition::new(
            RisePositionKey::new(raw_position),
            local_id.to_string(),
        )))
    }

    pub(crate) fn read_entity_local_id(
        &self,
        namespace: &RiseStoreNamespace,
        local_id: &str,
        budget: Option<&mut RiseStoreReadBudget>,
        database: &RiseSQLiteConnection,
    ) -> Result<Option<RiseStoreEntity>, RiseStoreError> {
        let mut statement = database.statement(
            "SELECT local_id, payload, version, updated_at_ms
            FROM entities WHERE namespace = ? AND local_id = ?",
            "entity.read",
        )?;
        statement.bind(namespace.raw_value(), 1)?;
        statement.bind(local_id, 2)?;
        if !statement.step()? {
            return Ok(None);
        }
        self.decode_entity(&statement, namespace, 0, 1, 2, 3, budget)
            .map(Some)
    }

    pub(crate) fn read_entity_alias(
        &self,
        namespace: &RiseStoreNamespace,
        alias: &RiseEntityAlias,
        budget: Option<&mut RiseStoreReadBudget>,
        database: &RiseSQLiteConnection,
    ) -> Result<Option<RiseStoreEntity>, RiseStoreError> {
        let mut statement = database.statement(
            "SELECT entity.local_id, entity.payload, entity.version, entity.updated_at_ms
            FROM entity_aliases AS alias
            JOIN entities AS entity
              ON entity.namespace = alias.namespace
             AND entity.local_id = alias.local_id
            WHERE alias.namespace = ?
              AND alias.alias_kind = ?
              AND alias.alias_value = ?",
            "entity.read_alias",
        )?;
        statement.bind(namespace.raw_value(), 1)?;
        statement.bind(alias.kind.raw_value(), 2)?;
        statement.bind(alias.value.as_str(), 3)?;
        if !statement.step()? {
            return Ok(None);
        }
        self.decode_entity(&statement, namespace, 0, 1, 2, 3, budget)
            .map(Some)
    }

    pub(crate) fn read_entity_window(
        &self,
        query: &RiseEntityWindowQuery,
        mut budget: Option<&mut RiseStoreReadBudget>,
        database: &RiseSQLiteConnection,
    ) -> Result<RiseEntityWindowResult, RiseStoreError> {
        let mut anchor_index: Option<usize> = None;
        let mut resolved_anchor: Option<RiseEntityViewPosition> = None;
        let items: Vec<RiseEntityViewItem> = match &query.window {
            RiseEntityWindow::First { limit } => self.read_entity_view_slice(
                &query.key,
                None,
                None,
                *limit,
                false,
                false,
                budget.as_deref_mut(),
                database,
            )?,
            RiseEntityWindow::Latest { limit } => self.read_entity_view_slice(
                &query.key,
                None,
                None,
                *limit,
                true,
                true,
                budget.as_deref_mut(),
                database,
            )?,
            RiseEntityWindow::Before { anchor, limit } => self.read_entity_view_slice(
                &query.key,
                Some(EntityViewPredicate::Before),
                Some(anchor),
                *limit,
                true,
                true,
                budget.as_deref_mut(),
                database,
            )?,
            RiseEntityWindow::After { anchor, limit } => self.read_entity_view_slice(
                &query.key,
                Some(EntityViewPredicate::After),
                Some(anchor),
                *limit,
                false,
                false,
                budget.as_deref_mut(),
                database,
            )?,
            RiseEntityWindow::Around {
                anchor,
                before_limit,
                after_limit,
            } => {
                let mut around = self.read_entity_view_slice(
                    &query.key,
                    Some(EntityViewPredicate::Before),
                    Some(anchor),
                    *before_limit,
                    true,
                    true,
                    budget.as_deref_mut(),
                    database,
                )?;
                if let Some(exact) = self.read_entity_view_exact(
                    &query.key,
                    anchor,
                    budget.as_deref_mut(),
                    database,
                )? {
                    anchor_index = Some(around.len());
                    resolved_anchor = Some(RiseEntityViewPosition::new(
                        exact.position_key.clone(),
                        exact.entity.local_id.clone(),
                    ));
                    around.push(exact);
                }
                around.extend(self.read_entity_view_slice(
                    &query.key,
                    Some(EntityViewPredicate::After),
                    Some(anchor),
                    *after_limit,
                    false,
                    false,
                    budget.as_deref_mut(),
                    database,
                )?);
                around
            }
        };

        let state = self.read_entity_view_state(
            &query.key.namespace,
            &query.key.view,
            budget.as_deref_mut(),
            database,
        )?;
        let total_count = self.read_entity_view_count(&query.key, database)?;
        let fallback_anchor: Option<&RiseEntityViewPosition> = match &query.window {
            RiseEntityWindow::First { .. } | RiseEntityWindow::Latest { .. } => None,
            RiseEntityWindow::Before { anchor, .. }
            | RiseEntityWindow::After { anchor, .. }
            | RiseEntityWindow::Around { anchor, .. } => Some(anchor),
        };
        let lower_boundary = items
            .first()
            .map(|item| {
                RiseEntityViewPosition::new(
                    item.position_key.clone(),
                    item.entity.local_id.clone(),
                )
            })
            .or_else(|| fallback_anchor.cloned());
        let upper_boundary = items
            .last()
            .map(|item| {
                RiseEntityViewPosition::new(
                    item.position_key.clone(),
                    item.entity.local_id.clone(),
                )
            })
            .or_else(|| fallback_anchor.cloned());
        let has_cached_before = match &lower_boundary {
            Some(position) => self.entity_view_has_item(&query.key, position, true, database)?,
            None => false,
        };
        let has_cached_after = match &upper_boundary {
            Some(position) => self.entity_view_has_item(&query.key, position, false, database)?,
            None => false,
        };
        Ok(RiseEntityWindowResult {
            items,
            state,
            total_count,
            has_cached_before,
            has_cached_after,
            anchor_index,
            resolved_anchor,
        })
    }

    pub(crate) fn read_entity_view_state(
        &self,
        namespace: &RiseStoreNamespace,
        view: &str,
        budget: Option<&mut RiseStoreReadBudget>,
        database: &RiseSQLiteConnection,
    ) -> Result<Option<RiseEntityViewState>, RiseStoreError> {
        let mut statement = database.statement(
            "SELECT revision, before_cursor, after_cursor, has_more_before,
                   has_more_after, load_state, last_error_code, updated_at_ms
            FROM entity_view_state
            WHERE namespace = ? AND view_id = ?",
            "entity_view.read_state",
        )?;
        statement.bind(namespace.raw_value(), 1)?;
        statement.bind(view, 2)?;
        if !statement.step()? {
            return Ok(None);
        }

        let before_bytes = statement.byte_count(1);
        let after_bytes = statement.byte_count(2);
        let Some(stored_bytes) = before_bytes.checked_add(after_bytes) else {
            return Err(RiseStoreError::CorruptedValue(
                "entity_view_cursor_size".to_string(),
            ));
        };
        self.validate_entity_stored_blob_size(
            stored_bytes,
            self.configuration.limits.maximum_value_bytes,
        )?;
        let revision = statement.int64(0);
        let before = self.open_entity_view_cursor(
            statement.data(1),
            CipherContextKind::EntityViewCursorBefore.domain(),
            namespace,
            view,
            revision,
        )?;
        let after = self.open_entity_view_cursor(
            statement.data(2),
            CipherContextKind::EntityViewCursorAfter.domain(),
            namespace,
            view,
            revision,
        )?;
        let Some(load_state) = statement
            .text(5)
            .and_then(|text| RiseEntityViewLoadState::from_raw(&text))
        else {
            return Err(RiseStoreError::CorruptedValue(
                "entity_view_state".to_string(),
            ));
        };
        let before_count = before.as_deref().map_or(0, <[u8]>::len);
        let after_count = after.as_deref().map_or(0, <[u8]>::len);
        let Some(cursor_bytes) = before_count.checked_add(after_count) else {
            return Err(RiseStoreError::CorruptedValue(
                "entity_view_cursor_size".to_string(),
            ));
        };
        if let Some(budget) = budget {
            budget.consume(cursor_bytes)?;
        }
        Ok(Some(RiseEntityViewState {
            revision,
            before_cursor: before,
            after_cursor: after,
            has_more_before: statement.bool(3),
            has_more_after: statement.bool(4),
            load_state,
            last_error_code: statement.text(6),
            updated_at_milliseconds: statement.int64(7),
        }))
    }

    #[allow(clippy::too_many_arguments)]
    fn read_entity_view_slice(
        &self,
        key: &RiseEntityViewKey,
        predicate: Option<EntityViewPredicate>,
        anchor: Option<&RiseEntityViewPosition>,
        limit: usize,
        descending: bool,
        reverse_result: bool,
        mut budget: Option<&mut RiseStoreReadBudget>,
        database: &RiseSQLiteConnection,
    ) -> Result<Vec<RiseEntityViewItem>, RiseStoreError> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let predicate_sql = match predicate {
            None => "",
            Some(EntityViewPredicate::Before) => {
                "AND (
                    membership.position_key < ?
                    OR (membership.position_key = ? AND membership.local_id < ?)
                )"
            }
            Some(EntityViewPredicate::After) => {
                "AND (
                    membership.position_key > ?
                    OR (membership.position_key = ? AND membership.local_id > ?)
                )"
            }
        };
        let direction = if descending { "DESC" } else { "ASC" };
        let sql = format!(
            "SELECT membership.position_key, entity.local_id, entity.payload,
                   entity.version, entity.updated_at_ms, membership.version
            FROM entity_views AS membership
            JOIN entities AS entity
              ON entity.namespace = membership.namespace
             AND entity.local_id = membership.local_id
            WHERE membership.namespace = ? AND membership.view_id = ?
            {predicate_sql}
            ORDER BY membership.position_key {direction}, membership.local_id {direction}
            LIMIT ?"
        );
        let mut statement = database.statement(&sql, "entity_view.read_slice")?;
        statement.bind(key.namespace.raw_value(), 1)?;
        statement.bind(key.view.as_str(), 2)?;
        let limit_index: i32 = if predicate_sql.is_empty() {
            3
        } else {
            let Some(anchor) = anchor else {
                return Err(RiseStoreError::CorruptedValue(
                    "entity_view_anchor".to_string(),
                ));
            };
            statement.bind(anchor.position_key.raw_value(), 3)?;
            statement.bind(anchor.position_key.raw_value(), 4)?;
            statement.bind(anchor.local_id.as_str(), 5)?;
            6
        };
        statement.bind(limit, limit_index)?;

        let mut result: Vec<RiseEntityViewItem> =
            Vec::with_capacity(limit.min(MAXIMUM_SLICE_RESERVATION));
        while statement.step()? {
            result.push(self.decode_entity_view_item(
                &statement,
                &key.namespace,
                budget.as_deref_mut(),
            )?);
        }
        if reverse_result {
            result.reverse();
        }
        Ok(result)
    }

    fn read_entity_view_exact(
        &self,
        key: &RiseEntityViewKey,
        position: &RiseEntityViewPosition,
        budget: Option<&mut RiseStoreReadBudget>,
        database: &RiseSQLiteConnection,
    ) -> Result<Option<RiseEntityViewItem>, RiseStoreError> {
        let mut statement = database.statement(
            "SELECT membership.position_key, entity.local_id, entity.payload,
                   entity.version, entity.updated_at_ms, membership.version
            FROM entity_views AS membership
            JOIN entities AS entity
              ON entity.namespace = membership.namespace
             AND entity.local_id = membership.local_id
            WHERE membership.namespace = ? AND membership.view_id = ?
              AND membership.position_key = ? AND membership.local_id = ?",
            "entity_view.read_exact",
        )?;
        statement.bind(key.namespace.raw_value(), 1)?;
        statement.bind(key.view.as_str(), 2)?;
        statement.bind(position.position_key.raw_value(), 3)?;
        statement.bind(position.local_id.as_str(), 4)?;
        if !statement.step()? {
            return Ok(None);
        }
        self.decode_entity_view_item(&statement, &key.namespace, budget)
            .map(Some)
    }

    fn decode_entity_view_item(
        &self,
        statement: &RiseSQLiteStatement<'_>,
        namespace: &RiseStoreNamespace,
        budget: Option<&mut RiseStoreReadBudget>,
    ) -> Result<RiseEntityViewItem, RiseStoreError> {
        let Some(raw_position) = statement.data(0).filter(|raw| {
            !raw.is_empty()
                && raw.len() as i64 <= self.configuration.limits.maximum_position_key_bytes
        }) else {
            return Err(RiseStoreError::CorruptedValue(
                "entity_view_position".to_string(),
            ));
        };
        let entity = self.decode_entity(statement, namespace, 1, 2, 3, 4, budget)?;
        Ok(RiseEntityViewItem {
            entity,
            position_key: RisePositionKey::new(raw_position),
            membership_version: statement.int64(5),
        })
    }

    fn read_entity_view_count(
        &self,
        key: &RiseEntityViewKey,
        database: &RiseSQLiteConnection,
    ) -> Result<usize, RiseStoreError> {
        let mut statement = database.statement(
            StatementId::ReadEntityViewCount.sql(),
            "entity_view.read_count",
        )?;
        statement.bind(key.namespace.raw_value(), 1)?;
        statement.bind(key.view.as_str(), 2)?;
        if !statement.step()? {
            return Ok(0);
        }
        usize::try_from(statement.int64(0))
            .map_err(|_| RiseStoreError::CorruptedValue("entity_view_count".to_string()))
    }

    fn entity_view_has_item(
        &self,
        key: &RiseEntityViewKey,
        relative_to: &RiseEntityViewPosition,
        before: bool,
        database: &RiseSQLiteConnection,
    ) -> Result<bool, RiseStoreError> {
        let comparator = if before { "<" } else { ">" };
        let sql = format!(
            "SELECT 1 FROM entity_views
            WHERE namespace = ? AND view_id = ?
              AND (
                  position_key {comparator} ?
                  OR (position_key = ? AND local_id {comparator} ?)
              )
            LIMIT 1"
        );
        let mut statement = database.statement(&sql, "entity_view.read_boundary")?;
        statement.bind(key.namespace.raw_value(), 1)?;
        statement.bind(key.view.as_str(), 2)?;
        statement.bind(relative_to.position_key.raw_value(), 3)?;
        statement.bind(relative_to.position_key.raw_value(), 4)?;
        statement.bind(relative_to.local_id.as_str(), 5)?;
        statement.step()
    }

    #[allow(clippy::too_many_arguments)]
    fn decode_entity(
        &self,
        statement: &RiseSQLiteStatement<'_>,
        namespace: &RiseStoreNamespace,
        local_id_column: i32,
        payload_column: i32,
        version_column: i32,
        updated_column: i32,
        budget: Option<&mut RiseStoreReadBudget>,
    ) -> Result<RiseStoreEntity, RiseStoreError> {
        let stored_bytes = statement.byte_count(payload_column);
        self.validate_entity_stored_blob_size(
            stored_bytes,
            self.configuration.limits.maximum_value_bytes,
        )?;
        let Some(local_id) = statement.text(local_id_column).filter(|local_id| {
            !local_id.is_empty() && local_id.len() <= IdentifierKind::LocalId.maximum_bytes()
        }) else {
            return Err(RiseStoreError::CorruptedValue("entity".to_string()));
        };
        let version = statement.int64(version_column);
        let data = match self.cached_entity_data(namespace, &local_id, version) {
            Some(cached) => cached,
            None => {
                let Some(sealed) = statement.data(payload_column) else {
                    return Err(RiseStoreError::CorruptedValue("entity".to_string()));
                };
                let opened = self.configuration.cipher.open(
                    &sealed,
                    &self.entity_cipher_context(namespace, &local_id, version),
                )?;
                self.cache_entity_data(opened.clone(), namespace, &local_id, version);
                opened
            }
        };
        if let Some(budget) = budget {
            budget.consume(data.len())?;
        }
        Ok(RiseStoreEntity::new(
            local_id,
            data,
            version,
            statement.int64(updated_column),
        ))
    }

    fn open_entity_view_cursor(
        &self,
        sealed: Option<Vec<u8>>,
        domain: &str,
        namespace: &RiseStoreNamespace,
        view: &str,
        revision: i64,
    ) -> Result<Option<Vec<u8>>, RiseStoreError> {
        let Some(sealed) = sealed else {
            return Ok(None);
        };
        self.configuration
            .cipher
            .open(
                &sealed,
                &self.entity_view_cursor_context(domain, namespace, view, revision),
            )
            .map(Some)
            .map_err(RiseStoreError::from)
    }

    fn validate_entity_stored_blob_size(
        &self,
        stored_bytes: i64,
        plaintext_limit: i64,
    ) -> Result<(), RiseStoreError> {
        match plaintext_limit.checked_add(ENCRYPTED_ENTITY_OVERHEAD_BYTES) {
            Some(maximum_stored_bytes) if stored_bytes <= maximum_stored_bytes => Ok(()),
            _ => Err(RiseStoreError::CorruptedValue(
                "oversized_encrypted_entity".to_string(),
            )),
        }
    }
}
