use std::collections::HashSet;

use ::rise_store::{Limit, VersionPrecondition, WritePolicy};

use crate::rise_store::rise_store_models::{
    RiseDurableOperationState, RiseDurableOrderingKey, RiseStoreCollectionKey,
    RiseStoreCollectionQuery, RiseStoreLimits, RiseStoreNamespace, RiseStoreSnapshotRequest,
    RiseStoreValueKey, RiseStoreVersionPrecondition, RiseStoreViewKey,
    RiseStoredValueWritePolicy,
};

fn chat() -> RiseStoreNamespace {
    RiseStoreNamespace::new("chat")
}

#[test]
fn view_keys_keep_the_ios_length_prefixed_spelling() {
    assert_eq!(
        RiseStoreViewKey::value(&chat(), "settings").raw_value(),
        "5:value|4:chat|8:settings"
    );
    assert_eq!(
        RiseStoreViewKey::collection(&chat(), "inbox").raw_value(),
        "10:collection|4:chat|5:inbox"
    );
    assert_eq!(
        RiseStoreViewKey::entity(&chat(), "m1").raw_value(),
        "6:entity|4:chat|2:m1"
    );
    assert_eq!(
        RiseStoreViewKey::entity_view(&chat(), "feed").raw_value(),
        "11:entity-view|4:chat|4:feed"
    );
    assert_eq!(
        RiseStoreViewKey::durable_operations().raw_value(),
        "durable-operations"
    );
}

#[test]
fn view_key_components_count_utf8_bytes() {
    let key = RiseStoreViewKey::value(&chat(), "\u{0434}\u{0430}");
    assert_eq!(key.raw_value(), "5:value|4:chat|4:\u{0434}\u{0430}");
}

#[test]
fn production_limits_are_the_canon_limits() {
    let limits = RiseStoreLimits::production();
    assert_eq!(
        limits.maximum_pending_operations as u64,
        Limit::MaximumPendingOperations.value()
    );
    assert_eq!(
        limits.maximum_entity_window_items as u64,
        Limit::MaximumEntityWindowItems.value()
    );
    assert_eq!(
        limits.maximum_durable_payload_bytes as u64,
        Limit::MaximumDurablePayloadBytes.value()
    );
    assert_eq!(limits.maximum_observers as u64, Limit::MaximumObservers.value());
    assert_eq!(RiseStoreLimits::default(), limits);
}

#[test]
fn custom_limits_keep_the_other_production_values() {
    let limits = RiseStoreLimits {
        maximum_batch_views: 2,
        ..RiseStoreLimits::PRODUCTION
    }
    .validated();
    assert_eq!(limits.maximum_batch_views, 2);
    assert_eq!(
        limits.maximum_batch_mutations,
        RiseStoreLimits::PRODUCTION.maximum_batch_mutations
    );
}

#[test]
#[should_panic]
fn a_window_below_the_floor_is_refused() {
    let _ = RiseStoreLimits {
        maximum_entity_window_items: 319,
        ..RiseStoreLimits::PRODUCTION
    }
    .validated();
}

#[test]
fn durable_states_keep_their_raw_values() {
    let raw: Vec<&str> = RiseDurableOperationState::ALL_CASES
        .into_iter()
        .map(RiseDurableOperationState::raw_value)
        .collect();
    assert_eq!(raw, ["queued", "in_flight", "retry_waiting", "terminal_failure"]);
    for state in RiseDurableOperationState::ALL_CASES {
        assert_eq!(
            RiseDurableOperationState::from_raw(state.raw_value()),
            Some(state)
        );
    }
    assert_eq!(RiseDurableOperationState::from_raw("inFlight"), None);
}

#[test]
fn policies_and_preconditions_are_the_canon_types() {
    let policy: RiseStoredValueWritePolicy = WritePolicy::RequireIdenticalReplay;
    assert_eq!(policy.wire(), "require_identical_replay");
    let precondition: RiseStoreVersionPrecondition = VersionPrecondition::Equals(7);
    assert!(precondition.is_satisfied(Some(7)));
    assert!(!RiseStoreVersionPrecondition::Missing.is_satisfied(Some(1)));
}

#[test]
fn identifiers_from_disk_are_refused_instead_of_trapping() {
    assert!(RiseStoreNamespace::try_from(String::new()).is_err());
    assert!(RiseStoreNamespace::try_from("x".repeat(1_025)).is_err());
    assert!(RiseDurableOrderingKey::try_from("x".repeat(257)).is_err());
    assert_eq!(RiseDurableOrderingKey::global().raw_value(), "global");
}

#[test]
fn a_snapshot_request_names_every_view_it_reads() {
    let mut values = HashSet::new();
    values.insert(RiseStoreValueKey::new(chat(), "settings"));
    let mut collections = HashSet::new();
    collections.insert(RiseStoreCollectionQuery::new(chat(), "inbox", 20, false, None));
    let mut collection_states = HashSet::new();
    collection_states.insert(RiseStoreCollectionKey::new(chat(), "inbox"));
    let request = RiseStoreSnapshotRequest {
        values,
        collections,
        collection_states,
        ..RiseStoreSnapshotRequest::default()
    };
    let keys = request.required_view_keys();
    assert_eq!(keys.len(), 2);
    assert!(keys.contains(&RiseStoreViewKey::value(&chat(), "settings")));
    assert!(keys.contains(&RiseStoreViewKey::collection(&chat(), "inbox")));
}
