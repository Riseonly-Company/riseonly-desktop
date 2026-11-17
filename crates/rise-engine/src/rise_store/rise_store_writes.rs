use std::collections::HashSet;

use rise_canon::rise_store::{
    CipherContextKind, Decision, StatementId, VersionPrecondition, WritePolicy, resolve,
};

use crate::rise_store::rise_sq_lite::RiseSQLiteConnection;
use crate::rise_store::rise_store::RiseStore;
use crate::rise_store::rise_store_entities::{RiseEntityAliasKind, RiseEntityViewRetention};
use crate::rise_store::rise_store_models::{
    RiseCollectionState, RiseDurableOperation, RiseDurableOperationState,
    RiseDurableTerminalResolution, RiseOrderedItem, RiseOrderedItemWritePolicy, RiseStoreError,
    RiseStoreMutation, RiseStoreNamespace, RiseStoreVersionPrecondition, RiseStoredValue,
    RiseStoredValueWritePolicy,
};

const TRANSITION_CLAIMED_DURABLE_OPERATION_SQL: &str = "UPDATE durable_operations SET state = ?, attempt_count = ?, next_attempt_ms = ?, last_error_code = ?, updated_at_ms = ?, claim_token = NULL WHERE request_id = ? AND state = 'in_flight' AND claim_token = ?";
const REMOVE_CLAIMED_DURABLE_OPERATION_SQL: &str = "DELETE FROM durable_operations WHERE request_id = ? AND state = 'in_flight' AND claim_token = ?";
const REMOVE_UNCLAIMED_DURABLE_OPERATION_SQL: &str = "DELETE FROM durable_operations WHERE request_id = ? AND state IN ('queued', 'retry_waiting', 'terminal_failure') AND claim_token IS NULL";
const RESOLVE_TERMINAL_REMOVE_SQL: &str =
    "DELETE FROM durable_operations WHERE request_id = ? AND state = 'terminal_failure'";
const RESOLVE_TERMINAL_REQUEUE_SQL: &str = "UPDATE durable_operations SET state = 'queued', attempt_count = CASE WHEN ? THEN 0 ELSE attempt_count END, next_attempt_ms = ?, last_error_code = NULL, updated_at_ms = ?, claim_token = NULL WHERE request_id = ? AND state = 'terminal_failure'";

impl RiseStore {
    pub(crate) fn apply_pruned_entities(
        &self,
        namespace: &RiseStoreNamespace,
        local_ids: &[String],
        database: &RiseSQLiteConnection,
    ) -> Result<bool, RiseStoreError> {
        let mut did_prune = false;
        for local_id in local_ids {
            let mut statement = database
                .statement(StatementId::RemoveEntity.sql(), "entity.prune_unreferenced")?;
            statement.bind(namespace.raw_value(), 1)?;
            statement.bind(local_id.as_str(), 2)?;
            let _ = statement.step()?;
            if database.changed_rows() > 0 {
                did_prune = true;
                self.invalidate_entity_read_cache(namespace, local_id);
            }
            statement.release();
        }
        Ok(did_prune)
    }

    pub(crate) fn apply(
        &self,
        mutation: &RiseStoreMutation,
        database: &RiseSQLiteConnection,
    ) -> Result<bool, RiseStoreError> {
        match mutation {
            RiseStoreMutation::SetValue {
                namespace,
                key,
                value,
                precondition,
                policy,
            } => self.apply_stored_value(namespace, key, value, precondition, policy, database),

            RiseStoreMutation::RemoveValue {
                namespace,
                key,
                precondition,
            } => {
                self.check(precondition, namespace, key, database)?;
                let mut statement =
                    database.statement(StatementId::RemoveValue.sql(), "value.remove")?;
                statement.bind(namespace.raw_value(), 1)?;
                statement.bind(key.as_str(), 2)?;
                let _ = statement.step()?;
                Ok(database.changed_rows() == 1)
            }

            RiseStoreMutation::UpsertOrderedItem {
                namespace,
                collection,
                item,
                policy,
            } => self.apply_ordered_item(namespace, collection, item, policy, database),

            RiseStoreMutation::RemoveOrderedItem {
                namespace,
                collection,
                item_id,
            } => {
                let mut statement = database
                    .statement(StatementId::RemoveOrderedItem.sql(), "ordered_item.remove")?;
                statement.bind(namespace.raw_value(), 1)?;
                statement.bind(collection.as_str(), 2)?;
                statement.bind(item_id.as_str(), 3)?;
                let _ = statement.step()?;
                Ok(database.changed_rows() == 1)
            }

            RiseStoreMutation::SetCollectionState {
                namespace,
                collection,
                state,
            } => self.apply_collection_state(namespace, collection, state, database),

            RiseStoreMutation::UpsertEntity {
                namespace,
                entity,
                policy,
            } => self.apply_entity(namespace, entity, policy, database),

            RiseStoreMutation::RemoveEntity {
                namespace,
                local_id,
            } => {
                let mut statement =
                    database.statement(StatementId::RemoveEntity.sql(), "entity.remove")?;
                statement.bind(namespace.raw_value(), 1)?;
                statement.bind(local_id.as_str(), 2)?;
                let _ = statement.step()?;
                let did_remove = database.changed_rows() == 1;
                if did_remove {
                    self.invalidate_entity_read_cache(namespace, local_id);
                }
                Ok(did_remove)
            }

            RiseStoreMutation::SetEntityAlias {
                namespace,
                alias,
                local_id,
            } => self.apply_entity_alias(namespace, alias, local_id, database),

            RiseStoreMutation::RemoveEntityAlias { namespace, alias } => {
                let mut statement = database
                    .statement(StatementId::RemoveEntityAlias.sql(), "entity_alias.remove")?;
                statement.bind(namespace.raw_value(), 1)?;
                statement.bind(alias.kind.raw_value(), 2)?;
                statement.bind(alias.value.as_str(), 3)?;
                let _ = statement.step()?;
                Ok(database.changed_rows() == 1)
            }

            RiseStoreMutation::UpsertEntityViewMembership {
                namespace,
                view,
                membership,
            } => self.apply_entity_view_membership(namespace, view, membership, database),

            RiseStoreMutation::RemoveEntityViewMembership {
                namespace,
                view,
                local_id,
                if_version_at_most: maximum_version,
            } => {
                let mut statement = database.statement(
                    StatementId::RemoveEntityViewMembership.sql(),
                    "entity_view.remove_membership",
                )?;
                statement.bind(namespace.raw_value(), 1)?;
                statement.bind(view.as_str(), 2)?;
                statement.bind(local_id.as_str(), 3)?;
                if let Some(maximum_version) = maximum_version {
                    statement.bind(*maximum_version, 4)?;
                    statement.bind(*maximum_version, 5)?;
                } else {
                    statement.bind_null(4)?;
                    statement.bind_null(5)?;
                }
                let _ = statement.step()?;
                Ok(database.changed_rows() == 1)
            }

            RiseStoreMutation::RemoveAllEntityViewMemberships {
                namespace,
                view,
                if_version_at_most: maximum_version,
            } => {
                let mut statement = database.statement(
                    StatementId::RemoveAllEntityViewMemberships.sql(),
                    "entity_view.remove_all_memberships",
                )?;
                statement.bind(namespace.raw_value(), 1)?;
                statement.bind(view.as_str(), 2)?;
                if let Some(maximum_version) = maximum_version {
                    statement.bind(*maximum_version, 3)?;
                    statement.bind(*maximum_version, 4)?;
                } else {
                    statement.bind_null(3)?;
                    statement.bind_null(4)?;
                }
                let _ = statement.step()?;
                Ok(database.changed_rows() > 0)
            }

            RiseStoreMutation::TrimEntityView {
                namespace,
                view,
                keeping,
                if_version_at_most: maximum_version,
            } => {
                let trim = match keeping {
                    RiseEntityViewRetention::First(_) => StatementId::TrimEntityViewFirst,
                    RiseEntityViewRetention::Latest(_) => StatementId::TrimEntityViewLatest,
                };
                let mut statement = database.statement(trim.sql(), "entity_view.trim")?;
                statement.bind(namespace.raw_value(), 1)?;
                statement.bind(view.as_str(), 2)?;
                if let Some(maximum_version) = maximum_version {
                    statement.bind(*maximum_version, 3)?;
                    statement.bind(*maximum_version, 4)?;
                } else {
                    statement.bind_null(3)?;
                    statement.bind_null(4)?;
                }
                statement.bind(namespace.raw_value(), 5)?;
                statement.bind(view.as_str(), 6)?;
                statement.bind(keeping.limit(), 7)?;
                statement.bind(namespace.raw_value(), 8)?;
                let retained = RiseEntityAliasKind::retained();
                statement.bind(retained.raw_value(), 9)?;
                let _ = statement.step()?;
                Ok(database.changed_rows() > 0)
            }

            RiseStoreMutation::PruneUnreferencedEntities { namespace, limit } => {
                let mut references = HashSet::new();
                let local_ids = self.select_pruned_entity_references(
                    namespace,
                    *limit,
                    &mut references,
                    database,
                )?;
                self.apply_pruned_entities(namespace, &local_ids, database)
            }

            RiseStoreMutation::MergeEntities {
                namespace,
                survivor_local_id,
                duplicate_local_id,
            } => self.apply_entity_merge(
                namespace,
                survivor_local_id,
                duplicate_local_id,
                database,
            ),

            RiseStoreMutation::SetEntityViewState {
                namespace,
                view,
                state,
            } => self.apply_entity_view_state(namespace, view, state, database),

            RiseStoreMutation::PutDurableOperation(operation) => {
                self.put_durable_operation(operation, database)?;
                Ok(true)
            }

            RiseStoreMutation::TransitionClaimedDurableOperation {
                request_id,
                claim_token,
                state,
                attempt_count,
                next_attempt_at_milliseconds,
                last_error_code,
                updated_at_milliseconds,
            } => {
                if *attempt_count < 0 {
                    return Err(RiseStoreError::DurableOperationStatePreconditionFailed(
                        *request_id,
                    ));
                }
                let request_text = request_id.raw_value().to_string();
                let mut statement = database.statement(
                    TRANSITION_CLAIMED_DURABLE_OPERATION_SQL,
                    "durable.transition_claimed",
                )?;
                statement.bind(state.raw_value(), 1)?;
                statement.bind(*attempt_count, 2)?;
                statement.bind(*next_attempt_at_milliseconds, 3)?;
                if let Some(last_error_code) = last_error_code {
                    statement.bind(last_error_code.as_str(), 4)?;
                } else {
                    statement.bind_null(4)?;
                }
                statement.bind(*updated_at_milliseconds, 5)?;
                statement.bind(request_text.as_str(), 6)?;
                statement.bind(claim_token.as_str(), 7)?;
                let _ = statement.step()?;
                if database.changed_rows() != 1 {
                    return Err(RiseStoreError::DurableOperationStatePreconditionFailed(
                        *request_id,
                    ));
                }
                Ok(true)
            }

            RiseStoreMutation::RemoveClaimedDurableOperation {
                request_id,
                claim_token,
            } => {
                let request_text = request_id.raw_value().to_string();
                let mut statement = database
                    .statement(REMOVE_CLAIMED_DURABLE_OPERATION_SQL, "durable.remove_claimed")?;
                statement.bind(request_text.as_str(), 1)?;
                statement.bind(claim_token.as_str(), 2)?;
                let _ = statement.step()?;
                if database.changed_rows() != 1 {
                    return Err(RiseStoreError::DurableOperationStatePreconditionFailed(
                        *request_id,
                    ));
                }
                Ok(true)
            }

            RiseStoreMutation::RemoveUnclaimedDurableOperation { request_id } => {
                let request_text = request_id.raw_value().to_string();
                let mut statement = database.statement(
                    REMOVE_UNCLAIMED_DURABLE_OPERATION_SQL,
                    "durable.remove_unclaimed",
                )?;
                statement.bind(request_text.as_str(), 1)?;
                let _ = statement.step()?;
                if database.changed_rows() != 1 {
                    return Err(RiseStoreError::DurableOperationStatePreconditionFailed(
                        *request_id,
                    ));
                }
                Ok(true)
            }

            RiseStoreMutation::ResolveTerminalOperation {
                request_id,
                resolution,
                updated_at_milliseconds,
            } => {
                let request_text = request_id.raw_value().to_string();
                match resolution {
                    RiseDurableTerminalResolution::Remove => {
                        let mut statement = database.statement(
                            RESOLVE_TERMINAL_REMOVE_SQL,
                            "durable.resolve_terminal.remove",
                        )?;
                        statement.bind(request_text.as_str(), 1)?;
                        let _ = statement.step()?;
                    }
                    RiseDurableTerminalResolution::Requeue {
                        next_attempt_at_milliseconds,
                        reset_attempt_count,
                    } => {
                        let mut statement = database.statement(
                            RESOLVE_TERMINAL_REQUEUE_SQL,
                            "durable.resolve_terminal.requeue",
                        )?;
                        statement.bind(*reset_attempt_count, 1)?;
                        statement.bind(*next_attempt_at_milliseconds, 2)?;
                        statement.bind(*updated_at_milliseconds, 3)?;
                        statement.bind(request_text.as_str(), 4)?;
                        let _ = statement.step()?;
                    }
                }
                if database.changed_rows() != 1 {
                    return Err(RiseStoreError::DurableOperationStatePreconditionFailed(
                        *request_id,
                    ));
                }
                Ok(true)
            }
        }
    }

    pub(crate) fn apply_stored_value(
        &self,
        namespace: &RiseStoreNamespace,
        key: &str,
        value: &RiseStoredValue,
        precondition: &RiseStoreVersionPrecondition,
        policy: &RiseStoredValueWritePolicy,
        database: &RiseSQLiteConnection,
    ) -> Result<bool, RiseStoreError> {
        self.check(precondition, namespace, key, database)?;
        if let Some(existing) = self.read_value(namespace, key, None, database)? {
            let identical = value.version == existing.version && value == &existing;
            match resolve(
                Some(existing.version),
                value.version,
                identical,
                stored_value_write_policy(policy),
            ) {
                Decision::Write => {}
                Decision::Skip => return Ok(false),
                Decision::Conflict => {
                    return Err(RiseStoreError::StoredValueVersionConflict {
                        namespace: namespace.clone(),
                        key: key.to_owned(),
                    });
                }
            }
        }

        let sealed = self.configuration.cipher.seal(
            &value.data,
            &self.cipher_context(
                CipherContextKind::KeyValue.domain(),
                &[namespace.raw_value(), key],
            ),
        )?;
        let mut statement = database.statement(StatementId::SetValue.sql(), "value.write")?;
        statement.bind(namespace.raw_value(), 1)?;
        statement.bind(key, 2)?;
        statement.bind(sealed.as_slice(), 3)?;
        statement.bind(value.version, 4)?;
        statement.bind(value.updated_at_milliseconds, 5)?;
        let _ = statement.step()?;
        if database.changed_rows() != 1 {
            return Err(RiseStoreError::StoredValueVersionConflict {
                namespace: namespace.clone(),
                key: key.to_owned(),
            });
        }
        Ok(true)
    }

    pub(crate) fn apply_ordered_item(
        &self,
        namespace: &RiseStoreNamespace,
        collection: &str,
        item: &RiseOrderedItem,
        policy: &RiseOrderedItemWritePolicy,
        database: &RiseSQLiteConnection,
    ) -> Result<bool, RiseStoreError> {
        if let Some(existing) =
            self.read_ordered_item(namespace, collection, &item.item_id, None, database)?
        {
            let identical = item.version == existing.version
                && match policy {
                    RiseOrderedItemWritePolicy::RequireIdenticalReplay => {
                        item.item_id == existing.item_id
                            && item.server_id == existing.server_id
                            && item.sort_key == existing.sort_key
                            && item.data == existing.data
                    }
                    RiseOrderedItemWritePolicy::ServerAuthoritative
                    | RiseOrderedItemWritePolicy::IgnoreIfNotNewer => item == &existing,
                };
            match resolve(
                Some(existing.version),
                item.version,
                identical,
                ordered_item_write_policy(policy),
            ) {
                Decision::Write => {}
                Decision::Skip => return Ok(false),
                Decision::Conflict => {
                    return Err(RiseStoreError::OrderedItemVersionConflict {
                        namespace: namespace.clone(),
                        collection: collection.to_owned(),
                        item_id: item.item_id.clone(),
                    });
                }
            }
        }

        let sealed = self.configuration.cipher.seal(
            &item.data,
            &self.cipher_context(
                CipherContextKind::OrderedItem.domain(),
                &[namespace.raw_value(), collection, item.item_id.as_str()],
            ),
        )?;
        let mut statement =
            database.statement(StatementId::UpsertOrderedItem.sql(), "ordered_item.write")?;
        statement.bind(namespace.raw_value(), 1)?;
        statement.bind(collection, 2)?;
        statement.bind(item.item_id.as_str(), 3)?;
        if let Some(server_id) = &item.server_id {
            statement.bind(server_id.as_str(), 4)?;
        } else {
            statement.bind_null(4)?;
        }
        statement.bind(item.sort_key, 5)?;
        statement.bind(sealed.as_slice(), 6)?;
        statement.bind(item.version, 7)?;
        statement.bind(item.updated_at_milliseconds, 8)?;
        let _ = statement.step()?;
        if database.changed_rows() != 1 {
            return Err(RiseStoreError::OrderedItemVersionConflict {
                namespace: namespace.clone(),
                collection: collection.to_owned(),
                item_id: item.item_id.clone(),
            });
        }
        Ok(true)
    }

    pub(crate) fn apply_collection_state(
        &self,
        namespace: &RiseStoreNamespace,
        collection: &str,
        state: &RiseCollectionState,
        database: &RiseSQLiteConnection,
    ) -> Result<bool, RiseStoreError> {
        if let Some(existing) = self.read_collection_state(namespace, collection, None, database)? {
            let identical = state.revision == existing.revision && state == &existing;
            match resolve(
                Some(existing.revision),
                state.revision,
                identical,
                WritePolicy::RequireIdenticalReplay,
            ) {
                Decision::Write => {}
                Decision::Skip => return Ok(false),
                Decision::Conflict => {
                    return Err(RiseStoreError::CollectionRevisionConflict {
                        namespace: namespace.clone(),
                        collection: collection.to_owned(),
                    });
                }
            }
        }

        let mut statement = database
            .statement(StatementId::SetCollectionState.sql(), "collection_state.write")?;
        statement.bind(namespace.raw_value(), 1)?;
        statement.bind(collection, 2)?;
        statement.bind(state.revision, 3)?;
        if let Some(cursor) = &state.before_cursor {
            let sealed = self.configuration.cipher.seal(
                cursor,
                &self.cipher_context(
                    CipherContextKind::CollectionCursorBefore.domain(),
                    &[namespace.raw_value(), collection],
                ),
            )?;
            statement.bind(sealed.as_slice(), 4)?;
        } else {
            statement.bind_null(4)?;
        }
        if let Some(cursor) = &state.after_cursor {
            let sealed = self.configuration.cipher.seal(
                cursor,
                &self.cipher_context(
                    CipherContextKind::CollectionCursorAfter.domain(),
                    &[namespace.raw_value(), collection],
                ),
            )?;
            statement.bind(sealed.as_slice(), 5)?;
        } else {
            statement.bind_null(5)?;
        }
        statement.bind(state.has_more_before, 6)?;
        statement.bind(state.has_more_after, 7)?;
        let _ = statement.step()?;
        if database.changed_rows() != 1 {
            return Err(RiseStoreError::CollectionRevisionConflict {
                namespace: namespace.clone(),
                collection: collection.to_owned(),
            });
        }
        Ok(true)
    }

    pub(crate) fn put_durable_operation(
        &self,
        operation: &RiseDurableOperation,
        database: &RiseSQLiteConnection,
    ) -> Result<(), RiseStoreError> {
        if operation.state != RiseDurableOperationState::Queued
            || operation.attempt_count != 0
            || operation.last_error_code.is_some()
        {
            return Err(RiseStoreError::DurableOperationStatePreconditionFailed(
                operation.request_id,
            ));
        }
        let request_text = operation.request_id.raw_value().to_string();
        let mut duplicate =
            database.statement(StatementId::ReadDurableDuplicate.sql(), "durable.duplicate")?;
        duplicate.bind(request_text.as_str(), 1)?;
        let already_exists = duplicate.step()?;
        if already_exists {
            return Err(RiseStoreError::DuplicateOperation(operation.request_id));
        }

        let mut usage = database.statement(StatementId::ReadDurableUsage.sql(), "durable.usage")?;
        if !usage.step()? {
            return Err(RiseStoreError::CorruptedValue(
                "durable_operation_usage".to_owned(),
            ));
        }
        let row_count = usage.int64(0);
        let payload_bytes = usage.int64(1);
        let limits = &self.configuration.limits;
        let maximum_operations =
            i64::try_from(limits.maximum_durable_operations).unwrap_or(i64::MAX);
        let maximum_bytes = i64::try_from(limits.maximum_durable_bytes).unwrap_or(i64::MAX);
        if row_count >= maximum_operations {
            return Err(RiseStoreError::DurableOperationCountLimitReached(
                limits.maximum_durable_operations,
            ));
        }

        let sealed_payload = self.configuration.cipher.seal(
            &operation.payload,
            &self.cipher_context(
                CipherContextKind::DurableOperation.domain(),
                &[request_text.as_str()],
            ),
        )?;
        let fits = i64::try_from(sealed_payload.len())
            .ok()
            .and_then(|sealed_bytes| payload_bytes.checked_add(sealed_bytes))
            .is_some_and(|total| total <= maximum_bytes);
        if !fits {
            return Err(RiseStoreError::DurableOperationByteLimitReached(
                limits.maximum_durable_bytes,
            ));
        }

        let sequence = self.allocate_durable_operation_sequence(database)?;
        let mut statement =
            database.statement(StatementId::PutDurableOperation.sql(), "durable.put")?;
        statement.bind(request_text.as_str(), 1)?;
        statement.bind(sequence, 2)?;
        statement.bind(operation.ordering_key.raw_value(), 3)?;
        statement.bind(operation.kind.as_str(), 4)?;
        statement.bind(sealed_payload.as_slice(), 5)?;
        statement.bind(operation.state.raw_value(), 6)?;
        statement.bind(operation.attempt_count, 7)?;
        statement.bind(operation.next_attempt_at_milliseconds, 8)?;
        if let Some(error) = &operation.last_error_code {
            statement.bind(error.as_str(), 9)?;
        } else {
            statement.bind_null(9)?;
        }
        statement.bind(operation.created_at_milliseconds, 10)?;
        statement.bind(operation.updated_at_milliseconds, 11)?;
        let _ = statement.step()?;
        Ok(())
    }

    pub(crate) fn allocate_durable_operation_sequence(
        &self,
        database: &RiseSQLiteConnection,
    ) -> Result<i64, RiseStoreError> {
        let mut read = database
            .statement(StatementId::ReadDurableSequence.sql(), "durable.sequence.read")?;
        let stored = if read.step()? {
            read.text(0).and_then(|text| text.parse::<i64>().ok())
        } else {
            None
        };
        let Some(sequence) = stored.filter(|sequence| *sequence > 0 && *sequence < i64::MAX)
        else {
            return Err(RiseStoreError::CorruptedValue(
                "next_durable_operation_sequence".to_owned(),
            ));
        };
        let mut write = database
            .statement(StatementId::WriteDurableSequence.sql(), "durable.sequence.write")?;
        write.bind((sequence + 1).to_string().as_str(), 1)?;
        let _ = write.step()?;
        if database.changed_rows() != 1 {
            return Err(RiseStoreError::CorruptedValue(
                "next_durable_operation_sequence".to_owned(),
            ));
        }
        Ok(sequence)
    }

    pub(crate) fn check(
        &self,
        precondition: &RiseStoreVersionPrecondition,
        namespace: &RiseStoreNamespace,
        key: &str,
        database: &RiseSQLiteConnection,
    ) -> Result<(), RiseStoreError> {
        let precondition = version_precondition(precondition);
        if !precondition.needs_probe() {
            return Ok(());
        }
        let mut statement =
            database.statement(StatementId::ReadValueVersion.sql(), "value.precondition")?;
        statement.bind(namespace.raw_value(), 1)?;
        statement.bind(key, 2)?;
        let version = if statement.step()? {
            Some(statement.int64(0))
        } else {
            None
        };
        if !precondition.is_satisfied(version) {
            return Err(RiseStoreError::VersionPreconditionFailed {
                namespace: namespace.clone(),
                key: key.to_owned(),
            });
        }
        Ok(())
    }
}

fn version_precondition(precondition: &RiseStoreVersionPrecondition) -> VersionPrecondition {
    match precondition {
        RiseStoreVersionPrecondition::Any => VersionPrecondition::Any,
        RiseStoreVersionPrecondition::Missing => VersionPrecondition::Missing,
        RiseStoreVersionPrecondition::Equals(version) => VersionPrecondition::Equals(*version),
    }
}

fn stored_value_write_policy(policy: &RiseStoredValueWritePolicy) -> WritePolicy {
    match policy {
        RiseStoredValueWritePolicy::RequireIdenticalReplay => WritePolicy::RequireIdenticalReplay,
        RiseStoredValueWritePolicy::ServerAuthoritative => WritePolicy::ServerAuthoritative,
        RiseStoredValueWritePolicy::IgnoreIfNotNewer => WritePolicy::IgnoreIfNotNewer,
    }
}

fn ordered_item_write_policy(policy: &RiseOrderedItemWritePolicy) -> WritePolicy {
    match policy {
        RiseOrderedItemWritePolicy::RequireIdenticalReplay => WritePolicy::RequireIdenticalReplay,
        RiseOrderedItemWritePolicy::ServerAuthoritative => WritePolicy::ServerAuthoritative,
        RiseOrderedItemWritePolicy::IgnoreIfNotNewer => WritePolicy::IgnoreIfNotNewer,
    }
}
