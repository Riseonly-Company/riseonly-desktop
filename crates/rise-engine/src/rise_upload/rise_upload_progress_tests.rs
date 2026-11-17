use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use futures::future::BoxFuture;

use super::rise_chunked_upload::{RiseChunkedUploadClient, RiseChunkedUploadRequest};
use super::rise_upload_progress::{
    RiseUploadCancellationHub, RiseUploadKey, RiseUploadPhase, RiseUploadProgress,
    RiseUploadProgressDelta, RiseUploadProgressHub, RiseUploadRealtimeProgress,
    RiseUploadRealtimeProgressHub,
};
use crate::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use crate::rise_wire::rise_json_value::RiseJSONValue;

const MEBIBYTE: i64 = 1_024 * 1_024;

fn hub_with(capacity: usize, subscriber_capacity: usize) -> RiseUploadProgressHub {
    RiseUploadProgressHub::new(capacity, subscriber_capacity, Duration::ZERO)
}

fn zero_cadence_hub() -> RiseUploadProgressHub {
    hub_with(1_024, 1_024)
}

fn progress(key: &RiseUploadKey, fraction: f64, phase: RiseUploadPhase) -> RiseUploadProgress {
    RiseUploadProgress::new(key.clone(), fraction, 0, 0, phase)
}

#[test]
fn slab_policy_is_bounded_and_only_retries_acknowledged_offsets() {
    assert_eq!(RiseChunkedUploadClient::resolved_slab_size_value(None), 5 * MEBIBYTE);
    assert_eq!(
        RiseChunkedUploadClient::resolved_slab_size_value(Some(&RiseJSONValue::Integer(1))),
        64 * 1_024
    );
    assert_eq!(
        RiseChunkedUploadClient::resolved_slab_size_value(Some(&RiseJSONValue::Integer(
            64 * MEBIBYTE
        ))),
        8 * MEBIBYTE
    );
    assert!(RiseChunkedUploadClient::should_retry_slab(true, 0, Some(503), None));
    assert!(!RiseChunkedUploadClient::should_retry_slab(false, 0, Some(503), None));
    assert!(!RiseChunkedUploadClient::should_retry_slab(true, 2, Some(503), None));
}

#[test]
fn video_upload_hints_are_optional_and_remain_bound_to_the_request() {
    let file_url = PathBuf::from("/tmp/video.mp4");
    let legacy = RiseChunkedUploadRequest {
        file_url: file_url.clone(),
        filename: "video.mp4".to_string(),
        content_type: "video/mp4".to_string(),
        file_context: "chat".to_string(),
        resource_id: None,
        video_width: None,
        video_height: None,
        video_duration: None,
        video_thumbnail_file_url: None,
        awaits_network: false,
    };
    assert_eq!(legacy.video_width, None);
    assert_eq!(legacy.video_height, None);
    assert_eq!(legacy.video_duration, None);
    assert_eq!(legacy.video_thumbnail_file_url, None);

    let thumbnail_url = PathBuf::from("/tmp/poster.jpg");
    let hinted = RiseChunkedUploadRequest {
        file_url,
        filename: "video.mp4".to_string(),
        content_type: "video/mp4".to_string(),
        file_context: "chat".to_string(),
        resource_id: None,
        video_width: Some(720),
        video_height: Some(1_280),
        video_duration: Some(4),
        video_thumbnail_file_url: Some(thumbnail_url.clone()),
        awaits_network: false,
    };
    assert_eq!(hinted.video_width, Some(720));
    assert_eq!(hinted.video_height, Some(1_280));
    assert_eq!(hinted.video_duration, Some(4));
    assert_eq!(hinted.video_thumbnail_file_url, Some(thumbnail_url));
}

#[tokio::test]
async fn progress_is_monotonic_within_phase_and_resets_between_phases() {
    let hub = zero_cadence_hub();
    let key = RiseUploadKey::new("message-1", 0);

    hub.publish(RiseUploadProgress::new(key.clone(), 1.0, 1_000, 1_000, RiseUploadPhase::Preparing))
        .await;
    hub.publish(RiseUploadProgress::new(key.clone(), 0.2, 100, 500, RiseUploadPhase::Uploading))
        .await;
    hub.publish(RiseUploadProgress::new(key.clone(), 0.1, 50, 500, RiseUploadPhase::Uploading))
        .await;

    let value = hub.snapshot().await.remove(&key);
    assert_eq!(value.as_ref().map(|value| value.phase().clone()), Some(RiseUploadPhase::Uploading));
    assert_eq!(value.as_ref().map(RiseUploadProgress::fraction), Some(0.2));
    assert_eq!(value.as_ref().map(RiseUploadProgress::completed_bytes), Some(100));
    assert_eq!(value.as_ref().map(RiseUploadProgress::total_bytes), Some(500));
}

#[tokio::test]
async fn server_processing_cannot_regress_back_to_transfer_progress() {
    let hub = zero_cadence_hub();
    let key = RiseUploadKey::new("message-processing", 0);

    hub.publish(progress(&key, 0.99, RiseUploadPhase::Uploading)).await;
    hub.publish(progress(&key, 0.15, RiseUploadPhase::Processing)).await;
    hub.publish(progress(&key, 1.0, RiseUploadPhase::Uploading)).await;

    let value = hub.snapshot().await.remove(&key);
    assert_eq!(value.as_ref().map(|value| value.phase().clone()), Some(RiseUploadPhase::Processing));
    assert_eq!(value.as_ref().map(RiseUploadProgress::fraction), Some(0.15));
}

#[tokio::test]
async fn realtime_progress_hub_bridges_an_early_server_event_and_unbinds() {
    let hub = RiseUploadRealtimeProgressHub::default();
    let recorder: Arc<Mutex<Vec<RiseUploadRealtimeProgress>>> = Arc::new(Mutex::new(Vec::new()));
    let early = RiseUploadRealtimeProgress::new("upload-1", 0.25, Some("processing".to_string()), None);
    hub.publish(early.clone()).await;
    let sink = Arc::clone(&recorder);
    hub.bind(
        early.upload_id(),
        Arc::new(move |value: RiseUploadRealtimeProgress| -> BoxFuture<'static, ()> {
            let sink = Arc::clone(&sink);
            Box::pin(async move {
                sink.lock().unwrap_or_else(PoisonError::into_inner).push(value);
            })
        }),
    )
    .await;
    hub.publish(RiseUploadRealtimeProgress::new(early.upload_id(), 0.7, None, None))
        .await;
    hub.unbind(early.upload_id()).await;
    hub.publish(RiseUploadRealtimeProgress::new(early.upload_id(), 0.9, None, None))
        .await;

    let values = recorder.lock().unwrap_or_else(PoisonError::into_inner).clone();
    assert_eq!(
        values,
        vec![
            early.clone(),
            RiseUploadRealtimeProgress::new(early.upload_id(), 0.7, None, None),
        ]
    );
}

#[tokio::test]
async fn terminal_progress_rejects_late_callbacks_until_a_new_session_begins() {
    let hub = zero_cadence_hub();
    let key = RiseUploadKey::new("message-2", 0);
    hub.publish(progress(&key, 1.0, RiseUploadPhase::Completed)).await;
    hub.publish(progress(&key, 0.4, RiseUploadPhase::Uploading)).await;
    assert_eq!(
        hub.snapshot().await.get(&key).map(|value| value.phase().clone()),
        Some(RiseUploadPhase::Completed)
    );

    hub.remove_owner_id(&key.owner_id).await;
    hub.publish(progress(&key, 0.7, RiseUploadPhase::Uploading)).await;
    assert!(!hub.snapshot().await.contains_key(&key));

    hub.begin(&key.owner_id).await;
    hub.publish(progress(&key, 0.1, RiseUploadPhase::Uploading)).await;
    assert_eq!(
        hub.snapshot().await.get(&key).map(RiseUploadProgress::fraction),
        Some(0.1)
    );
}

#[tokio::test]
async fn progress_storage_is_bounded_per_slot() {
    let hub = hub_with(2, 1_024);
    for index in 0..3 {
        let key = RiseUploadKey::new(format!("message-{index}"), 0);
        hub.publish(progress(&key, 0.0, RiseUploadPhase::Preparing)).await;
    }
    let snapshot = hub.snapshot().await;
    assert_eq!(snapshot.len(), 2);
    assert!(!snapshot.contains_key(&RiseUploadKey::new("message-0", 0)));
}

#[tokio::test]
async fn legacy_snapshot_stream_still_replays_and_publishes() {
    let hub = zero_cadence_hub();
    let key = RiseUploadKey::new("legacy-owner", 0);
    let mut iterator = hub.updates().await;

    assert_eq!(iterator.next().await, Some(HashMap::new()));

    let published = progress(&key, 0.4, RiseUploadPhase::Uploading);
    hub.publish(published.clone()).await;

    assert_eq!(iterator.next().await, Some(HashMap::from([(key, published)])));

    hub.shutdown().await;
    assert_eq!(iterator.next().await, None);
}

#[tokio::test]
async fn scoped_delta_streams_replay_current_values_without_cross_owner_noise() {
    let hub = zero_cadence_hub();
    let first_key = RiseUploadKey::new("owner-a", 0);
    let second_key = RiseUploadKey::new("owner-a", 1);
    let other_key = RiseUploadKey::new("owner-b", 0);
    let first = progress(&first_key, 0.1, RiseUploadPhase::Preparing);
    let second = progress(&second_key, 0.2, RiseUploadPhase::Uploading);
    hub.publish(first.clone()).await;
    hub.publish(second.clone()).await;
    hub.publish(progress(&other_key, 0.3, RiseUploadPhase::Uploading)).await;

    let mut key_iterator = hub.updates_for(first_key.clone()).await;
    let mut owner_iterator = hub.deltas(&first_key.owner_id).await;

    let key_replay = key_iterator.next().await;
    assert!(key_replay.is_some());
    assert_eq!(key_replay.flatten(), Some(first.clone()));
    assert_eq!(
        owner_iterator.next().await,
        Some(RiseUploadProgressDelta::new(
            HashMap::from([(first_key.clone(), first), (second_key, second)]),
            HashSet::new(),
        ))
    );

    hub.publish(progress(&other_key, 0.7, RiseUploadPhase::Uploading)).await;
    let updated = progress(&first_key, 0.8, RiseUploadPhase::Uploading);
    hub.publish(updated.clone()).await;

    let key_update = key_iterator.next().await;
    assert!(key_update.is_some());
    assert_eq!(key_update.flatten(), Some(updated.clone()));
    assert_eq!(
        owner_iterator.next().await,
        Some(RiseUploadProgressDelta::new(
            HashMap::from([(first_key, updated)]),
            HashSet::new(),
        ))
    );
}

#[tokio::test]
async fn scoped_delta_stream_handles_terminal_remove_begin_and_shutdown() {
    let hub = zero_cadence_hub();
    let key = RiseUploadKey::new("reused-owner", 0);
    let mut iterator = hub.updates_for(key.clone()).await;

    let initial = iterator.next().await;
    assert!(initial.is_some());
    assert_eq!(initial.flatten(), None);

    let uploading = progress(&key, 0.5, RiseUploadPhase::Uploading);
    hub.publish(uploading.clone()).await;
    let upload_update = iterator.next().await;
    assert!(upload_update.is_some());
    assert_eq!(upload_update.flatten(), Some(uploading));

    let completed = progress(&key, 1.0, RiseUploadPhase::Completed);
    hub.publish(completed.clone()).await;
    let completion_update = iterator.next().await;
    assert!(completion_update.is_some());
    assert_eq!(completion_update.flatten(), Some(completed.clone()));

    hub.publish(progress(&key, 0.7, RiseUploadPhase::Uploading)).await;
    assert_eq!(hub.snapshot().await.get(&key), Some(&completed));

    hub.begin(&key.owner_id).await;
    let begin_removal = iterator.next().await;
    assert!(begin_removal.is_some());
    assert_eq!(begin_removal.flatten(), None);

    let restarted = progress(&key, 0.1, RiseUploadPhase::Preparing);
    hub.publish(restarted.clone()).await;
    let restart_update = iterator.next().await;
    assert!(restart_update.is_some());
    assert_eq!(restart_update.flatten(), Some(restarted));

    hub.remove(key.clone()).await;
    let explicit_removal = iterator.next().await;
    assert!(explicit_removal.is_some());
    assert_eq!(explicit_removal.flatten(), None);

    hub.shutdown().await;
    assert_eq!(iterator.next().await, None);
    let mut after_shutdown_iterator = hub.updates_for(key).await;
    assert_eq!(after_shutdown_iterator.next().await, None);
}

#[tokio::test]
async fn owner_delta_removal_is_batched_for_every_current_slot() {
    let hub = zero_cadence_hub();
    let keys = [RiseUploadKey::new("owner", 0), RiseUploadKey::new("owner", 1)];
    for key in &keys {
        hub.publish(progress(key, 0.25, RiseUploadPhase::Uploading)).await;
    }

    let mut iterator = hub.deltas("owner").await;
    assert_eq!(iterator.next().await.map(|delta| delta.values.len()), Some(2));

    hub.remove_owner_id("owner").await;
    assert_eq!(
        iterator.next().await,
        Some(RiseUploadProgressDelta::new(HashMap::new(), HashSet::from(keys)))
    );
}

#[tokio::test]
async fn scoped_subscriber_bookkeeping_evicts_and_finishes_the_oldest_stream() {
    let hub = hub_with(1_024, 2);
    let first_key = RiseUploadKey::new("first", 0);
    let second_key = RiseUploadKey::new("second", 0);
    let third_key = RiseUploadKey::new("third", 0);

    let mut first_iterator = hub.updates_for(first_key).await;
    let _ = first_iterator.next().await;

    let mut second_iterator = hub.updates_for(second_key.clone()).await;
    let _ = second_iterator.next().await;

    let mut third_iterator = hub.updates_for(third_key.clone()).await;
    let _ = third_iterator.next().await;

    assert_eq!(first_iterator.next().await, None);

    let second = progress(&second_key, 0.2, RiseUploadPhase::Uploading);
    let third = progress(&third_key, 0.3, RiseUploadPhase::Uploading);
    hub.publish(second.clone()).await;
    let second_update = second_iterator.next().await;
    assert!(second_update.is_some());
    assert_eq!(second_update.flatten(), Some(second));
    hub.publish(third.clone()).await;
    let third_update = third_iterator.next().await;
    assert!(third_update.is_some());
    assert_eq!(third_update.flatten(), Some(third));
}

#[tokio::test]
async fn slot_cancellation_does_not_cancel_sibling_uploads_and_begin_resets_the_session() {
    let hub = RiseUploadCancellationHub::default();
    hub.begin("message").await;
    hub.cancel("message", Some(1)).await;

    assert!(hub.is_cancelled("message", Some(1)).await);
    assert!(!hub.is_cancelled("message", Some(0)).await);
    assert!(!hub.is_cancelled("message", None).await);
    assert_eq!(hub.cancelled_slot_indices("message").await, HashSet::from([1]));

    hub.begin("message").await;
    assert!(!hub.is_cancelled("message", Some(1)).await);
    assert!(hub.cancelled_slot_indices("message").await.is_empty());
}

#[tokio::test]
async fn exact_cancellation_signal_aborts_owned_task_without_touching_sibling() {
    let hub = RiseUploadCancellationHub::default();
    hub.begin("message-signal").await;
    let first_signal = hub.signal("message-signal", Some(0)).await;
    let second_signal = hub.signal("message-signal", Some(1)).await;
    let runtime = RiseAccountRuntime::current();
    let first_operation = runtime.spawn(async { tokio::time::sleep(Duration::from_secs(30)).await });
    let second_operation = runtime.spawn(async { tokio::time::sleep(Duration::from_secs(30)).await });
    let first_abort = first_operation.abort_handle();
    let second_abort = second_operation.abort_handle();
    let first_cancelled = Arc::new(AtomicBool::new(false));
    let second_cancelled = Arc::new(AtomicBool::new(false));
    let first_registration = first_signal.on_cancel({
        let first_abort = first_abort.clone();
        let first_cancelled = Arc::clone(&first_cancelled);
        move || {
            first_cancelled.store(true, Ordering::SeqCst);
            first_abort.abort();
        }
    });
    let second_registration = second_signal.on_cancel({
        let second_abort = second_abort.clone();
        let second_cancelled = Arc::clone(&second_cancelled);
        move || {
            second_cancelled.store(true, Ordering::SeqCst);
            second_abort.abort();
        }
    });

    hub.cancel("message-signal", Some(1)).await;

    assert!(!first_signal.is_cancelled());
    assert!(!first_cancelled.load(Ordering::SeqCst));
    assert!(second_signal.is_cancelled());
    assert!(second_cancelled.load(Ordering::SeqCst));
    match second_operation.await {
        Ok(()) => panic!("The exact-slot operation was not cancelled"),
        Err(error) => assert!(error.is_cancelled(), "Unexpected cancellation error: {error}"),
    }

    first_registration.cancel();
    second_registration.cancel();
    first_abort.abort();
    second_abort.abort();
}

#[tokio::test]
async fn cancellation_signal_replays_and_session_reset_invalidates_old_work() {
    let hub = RiseUploadCancellationHub::default();
    hub.begin("message-replay").await;
    hub.cancel("message-replay", Some(2)).await;

    let late_signal = hub.signal("message-replay", Some(2)).await;
    assert!(late_signal.is_cancelled());

    hub.begin("message-replay").await;
    let fresh_signal = hub.signal("message-replay", Some(2)).await;
    assert!(!fresh_signal.is_cancelled());

    hub.finish("message-replay").await;
    assert!(fresh_signal.is_cancelled());
    let after_finish = hub.signal("message-replay", Some(2)).await;
    assert!(after_finish.is_cancelled());
}

#[tokio::test]
async fn exact_slot_seal_and_cancel_choose_one_atomic_winner() {
    let missing = RiseUploadCancellationHub::default();
    assert!(!missing.cancel("not-active", Some(1)).await);
    assert!(!missing.seal("not-active", 1).await);

    let cancel_first = RiseUploadCancellationHub::default();
    cancel_first.begin("cancel-first").await;
    assert!(cancel_first.cancel("cancel-first", Some(1)).await);
    assert!(!cancel_first.seal("cancel-first", 1).await);
    assert_eq!(
        cancel_first.cancelled_slot_indices("cancel-first").await,
        HashSet::from([1])
    );

    let seal_first = RiseUploadCancellationHub::default();
    seal_first.begin("seal-first").await;
    assert!(seal_first.seal("seal-first", 1).await);
    assert!(!seal_first.cancel("seal-first", Some(1)).await);
    assert!(seal_first.cancelled_slot_indices("seal-first").await.is_empty());

    seal_first.finish("seal-first").await;
    assert!(!seal_first.cancel("seal-first", Some(1)).await);
    assert!(!seal_first.seal("seal-first", 1).await);
}

#[tokio::test]
async fn whole_upload_cancellation_covers_every_slot_and_storage_is_bounded() {
    let hub = RiseUploadCancellationHub::new(2);
    hub.begin("old").await;
    hub.begin("middle").await;
    hub.begin("latest").await;
    hub.cancel("middle", None).await;

    assert!(!hub.is_cancelled("old", Some(0)).await);
    assert!(hub.signal("old", Some(0)).await.is_cancelled());
    assert!(!hub.seal("old", 0).await);
    assert!(hub.is_cancelled("middle", Some(0)).await);
    assert!(hub.is_cancelled("middle", Some(99)).await);

    hub.finish("middle").await;
    assert!(!hub.is_cancelled("middle", Some(0)).await);
}
