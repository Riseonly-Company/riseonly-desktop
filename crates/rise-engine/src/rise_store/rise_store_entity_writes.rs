use rise_canon::rise_store::{CipherContextKind, StatementId};

use crate::rise_store::rise_sq_lite::RiseSQLiteConnection;
use crate::rise_store::rise_store::RiseStore;
use crate::rise_store::rise_store_entities::{
    RiseEntityAlias, RiseEntityViewMembership, RiseEntityViewState, RiseEntityWritePolicy,
    RiseStoreEntity,
};
use crate::rise_store::rise_store_models::{RiseStoreError, RiseStoreNamespace};

impl RiseStore {
    pub(crate) fn apply_entity(
        &self,
        namespace: &RiseStoreNamespace,
        entity: &RiseStoreEntity,
        policy: &RiseEntityWritePolicy,
        database: &RiseSQLiteConnection,
    ) -> Result<bool, RiseStoreError> {
        let existing = self.read_entity_local_id(namespace, &entity.local_id, None, database)?;
        if let RiseEntityWritePolicy::CompareAndSwap { expected } = policy {
            let matches = existing
                .as_ref()
                .is_some_and(|existing| existing == expected && entity.version > existing.version);
            if !matches {
                return Ok(false);
            }
        }
        if let Some(existing) = &existing {
            match policy {
                RiseEntityWritePolicy::RequireIdenticalReplay => {
                    if entity.version < existing.version {
                        return Err(RiseStoreError::EntityVersionConflict {
                            namespace: namespace.clone(),
                            local_id: entity.local_id.clone(),
                        });
                    }
                    if entity.version == existing.version {
                        if entity != existing {
                            return Err(RiseStoreError::EntityVersionConflict {
                                namespace: namespace.clone(),
                                local_id: entity.local_id.clone(),
                            });
                        }
                        return Ok(false);
                    }
                }
                RiseEntityWritePolicy::ServerAuthoritative => {
                    if entity.version < existing.version || entity == existing {
                        return Ok(false);
                    }
                }
                RiseEntityWritePolicy::IgnoreIfNotNewer => {
                    if entity.version <= existing.version {
                        return Ok(false);
                    }
                }
                RiseEntityWritePolicy::CompareAndSwap { .. } => {}
            }
        }

        let sealed = self.configuration.cipher.seal(
            &entity.data,
            &self.entity_cipher_context(namespace, &entity.local_id, entity.version),
        )?;
        let mut statement = database.statement(
            StatementId::UpsertEntity.sql(),
            "entity.write",
        )?;
        statement.bind(namespace.raw_value(), 1)?;
        statement.bind(entity.local_id.as_str(), 2)?;
        statement.bind(sealed.as_slice(), 3)?;
        statement.bind(entity.version, 4)?;
        statement.bind(entity.updated_at_milliseconds, 5)?;
        statement.step()?;
        if database.changed_rows() != 1 {
            return Err(RiseStoreError::EntityVersionConflict {
                namespace: namespace.clone(),
                local_id: entity.local_id.clone(),
            });
        }
        if let RiseEntityWritePolicy::CompareAndSwap { .. } = policy {
            let mut memberships = database.statement(
                "UPDATE entity_views SET version = ?
                WHERE namespace = ? AND local_id = ? AND version < ?",
                "entity_view.advance_patch_version",
            )?;
            memberships.bind(entity.version, 1)?;
            memberships.bind(namespace.raw_value(), 2)?;
            memberships.bind(entity.local_id.as_str(), 3)?;
            memberships.bind(entity.version, 4)?;
            memberships.step()?;
        }
        self.invalidate_entity_read_cache(namespace, &entity.local_id);
        Ok(true)
    }

    pub(crate) fn apply_entity_alias(
        &self,
        namespace: &RiseStoreNamespace,
        alias: &RiseEntityAlias,
        local_id: &str,
        database: &RiseSQLiteConnection,
    ) -> Result<bool, RiseStoreError> {
        let mut statement = database.statement(
            StatementId::SetEntityAlias.sql(),
            "entity_alias.write",
        )?;
        statement.bind(namespace.raw_value(), 1)?;
        statement.bind(alias.kind.raw_value(), 2)?;
        statement.bind(alias.value.as_str(), 3)?;
        statement.bind(local_id, 4)?;
        statement.step()?;
        Ok(database.changed_rows() == 1)
    }

    pub(crate) fn apply_entity_view_membership(
        &self,
        namespace: &RiseStoreNamespace,
        view: &str,
        membership: &RiseEntityViewMembership,
        database: &RiseSQLiteConnection,
    ) -> Result<bool, RiseStoreError> {
        let mut existing = database.statement(
            StatementId::ReadEntityViewMembership.sql(),
            "entity_view.read_membership_version",
        )?;
        existing.bind(view, 1)?;
        existing.bind(namespace.raw_value(), 2)?;
        existing.bind(membership.local_id.as_str(), 3)?;
        if !existing.step()? {
            return Err(RiseStoreError::EntityReferenceNotFound {
                namespace: namespace.clone(),
                local_id: membership.local_id.clone(),
            });
        }
        let canonical_version = existing.int64(0);
        if membership.version < canonical_version {
            return Ok(false);
        }
        if !existing.is_null(1) {
            let existing_position = match existing.data(1) {
                Some(position) if !existing.is_null(2) => position,
                _ => {
                    return Err(RiseStoreError::CorruptedValue(
                        "entity_view_membership".to_string(),
                    ));
                }
            };
            let existing_version = existing.int64(2);
            if membership.version < existing_version {
                return Ok(false);
            }
            if membership.version == existing_version {
                if membership.position_key.raw_value() != existing_position.as_slice() {
                    return Err(RiseStoreError::EntityViewMembershipVersionConflict {
                        namespace: namespace.clone(),
                        view: view.to_string(),
                        local_id: membership.local_id.clone(),
                    });
                }
                return Ok(false);
            }
        }
        existing.release();

        let mut statement = database.statement(
            StatementId::UpsertEntityViewMembership.sql(),
            "entity_view.write_membership",
        )?;
        statement.bind(namespace.raw_value(), 1)?;
        statement.bind(view, 2)?;
        statement.bind(membership.local_id.as_str(), 3)?;
        statement.bind(membership.position_key.raw_value(), 4)?;
        statement.bind(membership.version, 5)?;
        statement.step()?;
        if database.changed_rows() != 1 {
            return Err(RiseStoreError::EntityViewMembershipVersionConflict {
                namespace: namespace.clone(),
                view: view.to_string(),
                local_id: membership.local_id.clone(),
            });
        }
        Ok(true)
    }

    pub(crate) fn apply_entity_view_state(
        &self,
        namespace: &RiseStoreNamespace,
        view: &str,
        state: &RiseEntityViewState,
        database: &RiseSQLiteConnection,
    ) -> Result<bool, RiseStoreError> {
        if let Some(existing) = self.read_entity_view_state(namespace, view, None, database)? {
            if state.revision < existing.revision {
                return Err(RiseStoreError::EntityViewRevisionConflict {
                    namespace: namespace.clone(),
                    view: view.to_string(),
                });
            }
            if state.revision == existing.revision {
                if *state != existing {
                    return Err(RiseStoreError::EntityViewRevisionConflict {
                        namespace: namespace.clone(),
                        view: view.to_string(),
                    });
                }
                return Ok(false);
            }
        }

        let mut statement = database.statement(
            StatementId::SetEntityViewState.sql(),
            "entity_view.write_state",
        )?;
        statement.bind(namespace.raw_value(), 1)?;
        statement.bind(view, 2)?;
        statement.bind(state.revision, 3)?;
        if let Some(cursor) = &state.before_cursor {
            let sealed = self.configuration.cipher.seal(
                cursor,
                &self.entity_view_cursor_context(
                    CipherContextKind::EntityViewCursorBefore.domain(),
                    namespace,
                    view,
                    state.revision,
                ),
            )?;
            statement.bind(sealed.as_slice(), 4)?;
        } else {
            statement.bind_null(4)?;
        }
        if let Some(cursor) = &state.after_cursor {
            let sealed = self.configuration.cipher.seal(
                cursor,
                &self.entity_view_cursor_context(
                    CipherContextKind::EntityViewCursorAfter.domain(),
                    namespace,
                    view,
                    state.revision,
                ),
            )?;
            statement.bind(sealed.as_slice(), 5)?;
        } else {
            statement.bind_null(5)?;
        }
        statement.bind(state.has_more_before, 6)?;
        statement.bind(state.has_more_after, 7)?;
        statement.bind(state.load_state.raw_value(), 8)?;
        if let Some(error_code) = &state.last_error_code {
            statement.bind(error_code.as_str(), 9)?;
        } else {
            statement.bind_null(9)?;
        }
        statement.bind(state.updated_at_milliseconds, 10)?;
        statement.step()?;
        if database.changed_rows() != 1 {
            return Err(RiseStoreError::EntityViewRevisionConflict {
                namespace: namespace.clone(),
                view: view.to_string(),
            });
        }
        Ok(true)
    }

    pub(crate) fn apply_entity_merge(
        &self,
        namespace: &RiseStoreNamespace,
        survivor_local_id: &str,
        duplicate_local_id: &str,
        database: &RiseSQLiteConnection,
    ) -> Result<bool, RiseStoreError> {
        let precondition_failed = || RiseStoreError::EntityMergePreconditionFailed {
            namespace: namespace.clone(),
            survivor_local_id: survivor_local_id.to_string(),
            duplicate_local_id: duplicate_local_id.to_string(),
        };
        if survivor_local_id == duplicate_local_id {
            return Err(precondition_failed());
        }
        let Some(survivor) =
            self.read_entity_local_id(namespace, survivor_local_id, None, database)?
        else {
            return Err(precondition_failed());
        };
        let Some(duplicate) =
            self.read_entity_local_id(namespace, duplicate_local_id, None, database)?
        else {
            return Err(precondition_failed());
        };

        if duplicate.version > survivor.version {
            self.apply_entity(
                namespace,
                &RiseStoreEntity::new(
                    survivor_local_id.to_string(),
                    duplicate.data,
                    duplicate.version,
                    duplicate.updated_at_milliseconds,
                ),
                &RiseEntityWritePolicy::ServerAuthoritative,
                database,
            )?;
        }

        let mut aliases = database.statement(
            "UPDATE entity_aliases SET local_id = ?
            WHERE namespace = ? AND local_id = ?",
            "entity.merge_aliases",
        )?;
        aliases.bind(survivor_local_id, 1)?;
        aliases.bind(namespace.raw_value(), 2)?;
        aliases.bind(duplicate_local_id, 3)?;
        aliases.step()?;
        aliases.release();

        let mut memberships = database.statement(
            "INSERT INTO entity_views(namespace, view_id, local_id, position_key, version)
            SELECT namespace, view_id, ?, position_key, version
            FROM entity_views
            WHERE namespace = ? AND local_id = ?
            ON CONFLICT(namespace, view_id, local_id) DO UPDATE SET
                position_key = excluded.position_key,
                version = excluded.version
            WHERE excluded.version > entity_views.version",
            "entity.merge_memberships",
        )?;
        memberships.bind(survivor_local_id, 1)?;
        memberships.bind(namespace.raw_value(), 2)?;
        memberships.bind(duplicate_local_id, 3)?;
        memberships.step()?;
        memberships.release();

        let mut remove_memberships = database.statement(
            "DELETE FROM entity_views WHERE namespace = ? AND local_id = ?",
            "entity.merge_remove_memberships",
        )?;
        remove_memberships.bind(namespace.raw_value(), 1)?;
        remove_memberships.bind(duplicate_local_id, 2)?;
        remove_memberships.step()?;
        remove_memberships.release();

        let mut remove_duplicate = database.statement(
            StatementId::RemoveEntity.sql(),
            "entity.merge_remove_duplicate",
        )?;
        remove_duplicate.bind(namespace.raw_value(), 1)?;
        remove_duplicate.bind(duplicate_local_id, 2)?;
        remove_duplicate.step()?;
        if database.changed_rows() != 1 {
            return Err(precondition_failed());
        }
        self.invalidate_entity_read_cache(namespace, survivor_local_id);
        self.invalidate_entity_read_cache(namespace, duplicate_local_id);
        Ok(true)
    }

    pub(crate) fn entity_cipher_context(
        &self,
        namespace: &RiseStoreNamespace,
        local_id: &str,
        version: i64,
    ) -> Vec<u8> {
        let version = version.to_string();
        self.cipher_context(
            CipherContextKind::Entity.domain(),
            &[namespace.raw_value(), local_id, version.as_str()],
        )
    }

    pub(crate) fn entity_view_cursor_context(
        &self,
        domain: &str,
        namespace: &RiseStoreNamespace,
        view: &str,
        revision: i64,
    ) -> Vec<u8> {
        let revision = revision.to_string();
        self.cipher_context(
            domain,
            &[namespace.raw_value(), view, revision.as_str()],
        )
    }
}
