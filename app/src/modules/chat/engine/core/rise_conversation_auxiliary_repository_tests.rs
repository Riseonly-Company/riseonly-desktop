use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use rise_engine::RiseJSONValue;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use tokio::sync::watch;

use crate::modules::chat::engine::core::rise_conversation_auxiliary_repository::{
    RiseConversationAuxiliaryRepository, RiseConversationAuxiliaryRepositoryError,
    RiseConversationCompatibilityRequestCoordinator, RiseConversationCompatibilityRequestCoordinatorOutput,
    RiseConversationCompatibilityRequestCoordinatorResolution,
    RiseConversationCompatibilityRequestCoordinatorResource,
    RiseConversationCompatibilityRequestCoordinatorSignature, RiseConversationResolutionSource,
};
use crate::modules::chat::engine::core::rise_conversation_resource_repository::{
    RiseConversationResourceKeyPersistence, RiseConversationResourceReadSource,
};
use crate::modules::chat::engine::core::rise_conversation_rpc::{
    RiseConversationEndpoint, RiseConversationEndpointKind,
};

struct RiseCompatibilityRequestGate {
    is_open: watch::Sender<bool>,
}

impl RiseCompatibilityRequestGate {
    fn new() -> Arc<Self> {
        Arc::new(Self { is_open: watch::Sender::new(false) })
    }

    async fn wait(&self) {
        let mut receiver = self.is_open.subscribe();
        let _ = receiver.wait_for(|is_open| *is_open).await;
    }

    fn release(&self) {
        self.is_open.send_replace(true);
    }
}

#[derive(Default)]
struct RiseCompatibilityRequestProbe {
    starts: AtomicUsize,
}

impl RiseCompatibilityRequestProbe {
    fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn record_start(&self) {
        self.starts.fetch_add(1, Ordering::SeqCst);
    }

    fn count(&self) -> usize {
        self.starts.load(Ordering::SeqCst)
    }
}

fn coordinator() -> Arc<RiseConversationCompatibilityRequestCoordinator> {
    Arc::new(RiseConversationCompatibilityRequestCoordinator::new())
}

fn resource(account_id: &str) -> RiseConversationCompatibilityRequestCoordinatorResource {
    RiseConversationCompatibilityRequestCoordinatorResource {
        account_id: account_id.to_owned(),
        logical_key: "chat|search".to_owned(),
    }
}

fn signature(body_digest: &str) -> RiseConversationCompatibilityRequestCoordinatorSignature {
    RiseConversationCompatibilityRequestCoordinatorSignature {
        body_digest: body_digest.to_owned(),
        prefer_cached_value: false,
        persistence: RiseConversationResourceKeyPersistence::MemoryOnly,
        allow_persistent_creation: true,
    }
}

async fn eventually<Predicate, Checked>(mut predicate: Predicate) -> bool
where
    Predicate: FnMut() -> Checked,
    Checked: Future<Output = bool>,
{
    for _ in 0..500 {
        if predicate().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    false
}

/// runs the read through the gate, records the start and waits for it
fn gated_read(
    coordinator: &Arc<RiseConversationCompatibilityRequestCoordinator>,
    resource: &RiseConversationCompatibilityRequestCoordinatorResource,
    signature: &RiseConversationCompatibilityRequestCoordinatorSignature,
    probe: &Arc<RiseCompatibilityRequestProbe>,
    gate: &Arc<RiseCompatibilityRequestGate>,
    value: RiseJSONValue,
) -> tokio::task::JoinHandle<anyhow::Result<RiseConversationCompatibilityRequestCoordinatorOutput>> {
    let coordinator = coordinator.clone();
    let resource = resource.clone();
    let signature = signature.clone();
    let probe = probe.clone();
    let gate = gate.clone();
    RiseAccountRuntime::current().spawn(async move {
        coordinator
            .perform_read(resource, signature, move || async move {
                probe.record_start();
                gate.wait().await;
                Ok(value)
            })
            .await
    })
}

fn expect_stale(result: anyhow::Result<RiseConversationCompatibilityRequestCoordinatorOutput>) {
    let error = result.err().expect("the superseded read must not publish");
    assert_eq!(
        error.downcast_ref::<RiseConversationAuxiliaryRepositoryError>(),
        Some(&RiseConversationAuxiliaryRepositoryError::StaleCompatibilityResult)
    );
}

#[tokio::test]
async fn identical_reads_share_one_in_flight_operation() {
    let coordinator = coordinator();
    let resource = resource("account-a");
    let signature = signature("same-body");
    let gate = RiseCompatibilityRequestGate::new();
    let probe = RiseCompatibilityRequestProbe::new();

    let first = gated_read(&coordinator, &resource, &signature, &probe, &gate, RiseJSONValue::Integer(42));
    assert!(eventually(|| async { probe.count() == 1 }).await);

    let second = gated_read(&coordinator, &resource, &signature, &probe, &gate, RiseJSONValue::Integer(99));
    assert!(
        eventually(|| async { coordinator.in_flight_waiter_count(&resource, &signature).await == 2 })
            .await
    );
    assert_eq!(probe.count(), 1);

    gate.release();
    let first_output = first.await.expect("first task").expect("first read");
    let second_output = second.await.expect("second task").expect("second read");

    assert_eq!(first_output.value, RiseJSONValue::Integer(42));
    assert_eq!(second_output.value, RiseJSONValue::Integer(42));
    assert_eq!(first_output.receipt, second_output.receipt);
    assert!(!first_output.joined_existing_flight);
    assert!(second_output.joined_existing_flight);
    assert_eq!(first_output.resolution_source, RiseConversationResolutionSource::Network);
    assert_eq!(second_output.resolution_source, RiseConversationResolutionSource::Network);
    assert!(coordinator.is_current(&first_output.receipt).await);
}

#[tokio::test]
async fn resolved_read_carries_exact_cache_source_through_single_flight() {
    let coordinator = coordinator();
    let output = coordinator
        .perform_resolved_read(resource("account-a"), signature("cached-body"), || async {
            Ok(RiseConversationCompatibilityRequestCoordinatorResolution {
                value: RiseJSONValue::String("cached".to_owned()),
                cache_lookup_source: Some(RiseConversationResourceReadSource::PersistentStore),
                source: RiseConversationResolutionSource::PersistentStore,
            })
        })
        .await
        .expect("resolved read");

    assert_eq!(output.value, RiseJSONValue::String("cached".to_owned()));
    assert_eq!(output.cache_lookup_source, Some(RiseConversationResourceReadSource::PersistentStore));
    assert_eq!(output.resolution_source, RiseConversationResolutionSource::PersistentStore);
    assert!(!output.joined_existing_flight);
}

#[tokio::test]
async fn newer_request_for_same_resource_suppresses_older_result() {
    let coordinator = coordinator();
    let resource = resource("account-a");
    let old_signature = signature("old-body");
    let new_signature = signature("new-body");
    let gate = RiseCompatibilityRequestGate::new();
    let probe = RiseCompatibilityRequestProbe::new();

    let old = gated_read(
        &coordinator,
        &resource,
        &old_signature,
        &probe,
        &gate,
        RiseJSONValue::String("old".to_owned()),
    );
    assert!(eventually(|| async { probe.count() == 1 }).await);

    let newest = coordinator
        .perform_read(resource.clone(), new_signature, || async {
            Ok(RiseJSONValue::String("new".to_owned()))
        })
        .await
        .expect("newest read");
    assert_eq!(newest.value, RiseJSONValue::String("new".to_owned()));
    assert!(coordinator.is_current(&newest.receipt).await);

    gate.release();
    expect_stale(old.await.expect("old task"));
}

#[tokio::test]
async fn reissued_signature_does_not_join_its_superseded_flight() {
    let coordinator = coordinator();
    let resource = resource("account-a");
    let first_signature = signature("first-body");
    let intervening_signature = signature("intervening-body");
    let stale_gate = RiseCompatibilityRequestGate::new();
    let probe = RiseCompatibilityRequestProbe::new();

    let stale = gated_read(
        &coordinator,
        &resource,
        &first_signature,
        &probe,
        &stale_gate,
        RiseJSONValue::String("stale".to_owned()),
    );
    assert!(eventually(|| async { probe.count() == 1 }).await);

    coordinator
        .perform_read(resource.clone(), intervening_signature, || async {
            Ok(RiseJSONValue::String("intervening".to_owned()))
        })
        .await
        .expect("intervening read");

    let reissue_probe = probe.clone();
    let reissued = coordinator
        .perform_read(resource.clone(), first_signature, move || async move {
            reissue_probe.record_start();
            Ok(RiseJSONValue::String("reissued".to_owned()))
        })
        .await
        .expect("reissued read");
    assert_eq!(reissued.value, RiseJSONValue::String("reissued".to_owned()));
    assert_eq!(probe.count(), 2);
    assert!(coordinator.is_current(&reissued.receipt).await);

    stale_gate.release();
    expect_stale(stale.await.expect("stale task"));
}

#[tokio::test]
async fn account_identity_prevents_cross_account_coalescing() {
    let coordinator = coordinator();
    let first_resource = resource("account-a");
    let second_resource = resource("account-b");
    let signature = signature("same-body");
    let gate = RiseCompatibilityRequestGate::new();
    let probe = RiseCompatibilityRequestProbe::new();

    let first =
        gated_read(&coordinator, &first_resource, &signature, &probe, &gate, RiseJSONValue::Integer(1));
    let second =
        gated_read(&coordinator, &second_resource, &signature, &probe, &gate, RiseJSONValue::Integer(2));

    assert!(eventually(|| async { probe.count() == 2 }).await);
    assert_eq!(coordinator.in_flight_waiter_count(&first_resource, &signature).await, 1);
    assert_eq!(coordinator.in_flight_waiter_count(&second_resource, &signature).await, 1);

    gate.release();
    let first_output = first.await.expect("first task").expect("first read");
    let second_output = second.await.expect("second task").expect("second read");
    assert_eq!(first_output.value, RiseJSONValue::Integer(1));
    assert_eq!(second_output.value, RiseJSONValue::Integer(2));
}

#[test]
fn only_reads_use_compatibility_coordination() {
    let uses = RiseConversationAuxiliaryRepository::uses_compatibility_read_coordinator;
    assert!(uses(&RiseConversationEndpoint::new("chat", "read", RiseConversationEndpointKind::Read)));
    assert!(!uses(&RiseConversationEndpoint::new("chat", "mutation", RiseConversationEndpointKind::Mutation)));
    assert!(!uses(&RiseConversationEndpoint::new(
        "chat",
        "request-id",
        RiseConversationEndpointKind::RequestIDMutation
    )));
    assert!(!uses(&RiseConversationEndpoint::new(
        "chat",
        "idempotent",
        RiseConversationEndpointKind::IdempotentMutation { field: "request_id".to_owned() }
    )));
}
