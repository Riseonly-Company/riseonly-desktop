use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::channel::oneshot;
use gpui::{App, AsyncApp, Context, Entity, Task, WeakEntity};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_uuid::Uuid;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::websocket::sai_web_socket_configuration::SaiWebSocketConfiguration;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::lib::sai::core::websocket::sai_ws_deferred_outbound_coordinator::{
    ACK_DEBT_PERSIST_FILENAME, SaiWsDeferredDisk, SaiWsDeferredDiskProtection,
    SaiWsDeferredPersistedRecord,
};
use crate::core::lib::sai::modules::sai_debugger::sai_ws_deferred_debug_store::{
    SaiWsDeferredDebugDiskRow, SaiWsDeferredDebugOutcome, SaiWsDeferredDebugStore,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsRequestOptions, SaiWsRequestOptionsInit,
};
use crate::core::lib::sai::sai::Sai;

/// true in debug or debug tools builds, the desktop has no debug tools build flag
fn debug_tools() -> bool {
    cfg!(debug_assertions)
}

/// error for an ack that didnt arrive in time
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, thiserror::Error)]
#[error("WebSocket request timed out waiting for a response.")]
pub struct SaiWsAckTimeoutError;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SaiWsAckDebtPersistedRecord {
    wire_request_id_millis: u64,
    envelope: SaiWsDeferredPersistedRecord,
}

static IS_FEATURE_ENABLED: AtomicBool = AtomicBool::new(false);

/// response debt coordinator, the main actor singleton is an entity here
pub struct SaiWsResponseDebtCoordinator {
    records: Vec<SaiWsAckDebtPersistedRecord>,
    timeout_tasks: HashMap<u64, Task<()>>,
    disk_loaded: bool,
    disk_epoch: u64,
    disk_load_waiters: Vec<oneshot::Sender<()>>,
}

impl SaiWsResponseDebtCoordinator {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    pub fn is_feature_enabled() -> bool {
        IS_FEATURE_ENABLED.load(Ordering::Relaxed)
    }

    pub fn set_is_feature_enabled(enabled: bool) {
        IS_FEATURE_ENABLED.store(enabled, Ordering::Relaxed);
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let this = Self {
            records: Vec::new(),
            timeout_tasks: HashMap::new(),
            disk_loaded: false,
            disk_epoch: 0,
            disk_load_waiters: Vec::new(),
        };
        let epoch = this.disk_epoch;
        let weak = cx.weak_entity();
        SaiWsDeferredDisk::load::<SaiWsAckDebtPersistedRecord>(
            ACK_DEBT_PERSIST_FILENAME,
            move |rows, cx| {
                let _ = weak.update(cx, |this, cx| this.merge_loaded_disk(rows, epoch, cx));
            },
            cx,
        );
        this
    }

    pub fn reset(&mut self) {
        self.timeout_tasks.clear();
        self.records.clear();
        self.disk_epoch = self.disk_epoch.wrapping_add(1);
    }

    pub fn purge_for_account_transition(&mut self, cx: &mut Context<Self>) {
        self.reset();
        Self::write_disk(Vec::new(), cx);
        if debug_tools() {
            self.debug_publish_snapshot(cx);
        }
    }

    pub fn test_record_count(&self) -> usize {
        self.records.len()
    }

    pub fn test_timer_count(&self) -> usize {
        self.timeout_tasks.len()
    }

    pub fn test_disk_record_count(&self) -> usize {
        SaiWsDeferredDisk::drain_and_read::<SaiWsAckDebtPersistedRecord>(ACK_DEBT_PERSIST_FILENAME)
            .len()
    }

    pub fn test_contains_record(&self, wire_id: u64) -> bool {
        self.records
            .iter()
            .any(|record| record.wire_request_id_millis == wire_id)
    }

    pub fn test_append_record(&mut self, wire_request_id_millis: u64, service: &str, method: &str) {
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: service.to_owned(),
            method: method.to_owned(),
            ..SaiWsRequestOptionsInit::default()
        });
        let Ok(envelope) = SaiWsDeferredPersistedRecord::make(
            Uuid::new_v4(),
            &json!({ "wire": wire_request_id_millis }),
            &options,
        ) else {
            return;
        };
        self.records.push(SaiWsAckDebtPersistedRecord { wire_request_id_millis, envelope });
    }

    pub fn test_persist_records_to_disk(&self, cx: &App) {
        Self::write_disk(self.records.clone(), cx);
    }

    pub fn prepare_for_socket_transport_drop(&mut self) {
        self.timeout_tasks = HashMap::new();
    }

    pub fn tracked_wire_ids(&self) -> HashSet<u64> {
        self.records
            .iter()
            .map(|record| record.wire_request_id_millis)
            .collect()
    }

    pub fn track_after_successful_wire_send_if_needed(
        &mut self,
        wire_request_id_millis: u64,
        data_value: &Value,
        options: &SaiWsRequestOptions,
        cx: &mut Context<Self>,
    ) {
        if !Self::is_feature_enabled() || !options.track_until_acknowledged {
            return;
        }
        if options.exclude_from_debugger || wire_request_id_millis == 0 {
            return;
        }
        match SaiWsDeferredPersistedRecord::make(Uuid::new_v4(), data_value, options) {
            Ok(envelope) => {
                self.records
                    .push(SaiWsAckDebtPersistedRecord { wire_request_id_millis, envelope });
                self.persist(cx);
                self.arm_timeout(wire_request_id_millis, options, cx);
                if debug_tools() {
                    self.debug_publish_snapshot(cx);
                }
            }
            Err(error) => {
                let message = error.to_string();
                Sai::log(
                    "WS",
                    move || format!("ack debt: persist skipped \u{2014} {message}"),
                    SaiLogKind::Websocket,
                    cx,
                );
            }
        }
    }

    pub fn on_wire_acknowledged(&mut self, wire_key: &str, cx: &mut Context<Self>) {
        let Ok(wire_id) = wire_key.parse::<u64>() else { return };
        self.timeout_tasks.remove(&wire_id);
        let Some(index) = self
            .records
            .iter()
            .position(|record| record.wire_request_id_millis == wire_id)
        else {
            return;
        };
        let removed = self.records[index].clone();
        self.records
            .retain(|record| record.wire_request_id_millis != wire_id);
        self.persist(cx);
        if debug_tools() {
            Self::debug_record_history(
                &removed.envelope,
                SaiWsDeferredDebugOutcome::AckDebtAcknowledged,
                false,
                cx,
            );
            self.debug_publish_snapshot(cx);
        }
    }

    pub fn schedule_flush_when_connected(
        &mut self,
        manager: Entity<SaiWebSocketManager>,
        cx: &mut Context<Self>,
    ) {
        let manager = manager.downgrade();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Some(manager) = manager.upgrade() else { return };
            Self::flush_when_connected(&this, &manager, cx).await;
        })
        .detach();
    }

    pub async fn flush_when_connected(
        this: &WeakEntity<Self>,
        manager: &Entity<SaiWebSocketManager>,
        cx: &mut AsyncApp,
    ) {
        if !Self::is_feature_enabled() {
            return;
        }
        let Ok(waiter) = this.update(cx, |this, _| this.await_disk_load()) else { return };
        if let Some(waiter) = waiter {
            let _ = waiter.await;
        }
        let Ok(batch) = this.update(cx, |this, _| this.records.clone()) else { return };
        if batch.is_empty() {
            return;
        }
        let mut kept = Vec::new();
        for record in batch {
            if !cx.update(|cx| manager.read(cx).is_connected()) {
                kept.push(record);
                continue;
            }
            let Some(body) = record.envelope.decoded_body() else {
                let wire = record.wire_request_id_millis;
                cx.update(|cx| {
                    Sai::log(
                        "WS",
                        move || format!("ack debt: drop bad JSON wire={wire}"),
                        SaiLogKind::Websocket,
                        cx,
                    );
                });
                continue;
            };
            let mut options = record.envelope.to_request_options();
            options.reload_if_cached = true;
            options.reload_when_only_disk_cached = true;
            let result = SaiWebSocketManager::resend_persisted_wire_frame(
                manager.clone(),
                record.wire_request_id_millis,
                Value::Object(body),
                options,
                cx.clone(),
            )
            .await;
            match result {
                Ok(()) => {
                    if debug_tools() {
                        cx.update(|cx| {
                            Self::debug_record_history(
                                &record.envelope,
                                SaiWsDeferredDebugOutcome::AckDebtColdReplaySent,
                                false,
                                cx,
                            );
                        });
                    }
                }
                Err(error) => {
                    let message = error.to_string();
                    let wire = record.wire_request_id_millis;
                    cx.update(|cx| {
                        if debug_tools() {
                            Self::debug_record_history(
                                &record.envelope,
                                SaiWsDeferredDebugOutcome::AckDebtColdReplayFailed(message.clone()),
                                false,
                                cx,
                            );
                        }
                        Sai::log(
                            "WS",
                            move || format!("ack debt cold replay failed wire={wire}: {message}"),
                            SaiLogKind::Websocket,
                            cx,
                        );
                    });
                    kept.push(record);
                }
            }
        }
        let _ = this.update(cx, |this, cx| {
            this.records = kept;
            this.persist(cx);
            if debug_tools() {
                this.debug_publish_snapshot(cx);
            }
        });
    }

    fn arm_timeout(&mut self, wire_id: u64, options: &SaiWsRequestOptions, cx: &mut Context<Self>) {
        self.timeout_tasks.remove(&wire_id);
        let seconds = options
            .offline_ack_timeout_seconds
            .unwrap_or_else(SaiWebSocketConfiguration::offline_ack_default_timeout_seconds);
        if seconds <= 0.0 {
            return;
        }
        let delay = Duration::try_from_secs_f64(seconds).unwrap_or(Duration::MAX);
        let timer = cx.background_executor().timer(delay);
        let task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            timer.await;
            Self::fire_timeout_if_needed(&this, wire_id, cx);
        });
        self.timeout_tasks.insert(wire_id, task);
    }

    fn fire_timeout_if_needed(this: &WeakEntity<Self>, wire_id: u64, cx: &mut AsyncApp) {
        let detached = this.update(cx, |this, _| {
            if let Some(task) = this.timeout_tasks.remove(&wire_id) {
                task.detach();
            }
        });
        if detached.is_err() {
            return;
        }
        let error = anyhow::Error::new(SaiWsAckTimeoutError);
        let failed = cx.update(|cx| {
            SaiWsPendingRegistry::shared(cx)
                .update(cx, |registry, cx| registry.fail_pending(wire_id, &error, cx))
        });
        if !failed {
            return;
        }
        let _ = this.update(cx, |this, cx| this.did_time_out(wire_id, cx));
    }

    fn did_time_out(&mut self, wire_id: u64, cx: &mut Context<Self>) {
        Sai::log(
            "WS",
            move || format!("ack debt: timeout id={wire_id} \u{2014} row kept for replay"),
            SaiLogKind::Websocket,
            cx,
        );
        if debug_tools() {
            if let Some(record) = self
                .records
                .iter()
                .find(|record| record.wire_request_id_millis == wire_id)
            {
                Self::debug_record_history(
                    &record.envelope,
                    SaiWsDeferredDebugOutcome::AckDebtTimedOut,
                    true,
                    cx,
                );
            }
            self.debug_publish_snapshot(cx);
        }
    }

    fn merge_loaded_disk(
        &mut self,
        decoded: Vec<SaiWsAckDebtPersistedRecord>,
        epoch: u64,
        cx: &mut Context<Self>,
    ) {
        self.disk_loaded = true;
        if epoch == self.disk_epoch {
            let decoded_count = decoded.len();
            let retained: Vec<_> = decoded
                .into_iter()
                .filter(|record| {
                    !SaiWsDeferredDiskProtection::must_remain_memory_only(
                        &record.envelope.service,
                        &record.envelope.method,
                    )
                })
                .collect();
            let retained_count = retained.len();
            let known: HashSet<u64> = self
                .records
                .iter()
                .map(|record| record.wire_request_id_millis)
                .collect();
            let mut restored: Vec<_> = retained
                .into_iter()
                .filter(|record| !known.contains(&record.wire_request_id_millis))
                .collect();
            let restored_count = restored.len();
            restored.append(&mut self.records);
            self.records = restored;
            if retained_count != decoded_count || self.records.len() != restored_count {
                self.persist(cx);
            }
        }
        for waiter in std::mem::take(&mut self.disk_load_waiters) {
            let _ = waiter.send(());
        }
        if debug_tools() {
            self.debug_publish_snapshot(cx);
        }
    }

    fn await_disk_load(&mut self) -> Option<oneshot::Receiver<()>> {
        if self.disk_loaded {
            return None;
        }
        let (waiter, receiver) = oneshot::channel();
        self.disk_load_waiters.push(waiter);
        Some(receiver)
    }

    /// writes before the first read would overwrite rows it hasnt seen yet
    fn persist(&self, cx: &App) {
        if !self.disk_loaded {
            return;
        }
        Self::write_disk(self.records.clone(), cx);
    }

    fn write_disk(rows: Vec<SaiWsAckDebtPersistedRecord>, cx: &App) {
        SaiWsDeferredDisk::write(rows, ACK_DEBT_PERSIST_FILENAME, cx);
    }

    /// records ack debt history
    /// only the timeout row has an enqueue time and its stamped with the same instant as processing
    fn debug_record_history(
        envelope: &SaiWsDeferredPersistedRecord,
        outcome: SaiWsDeferredDebugOutcome,
        stamps_enqueue: bool,
        cx: &mut App,
    ) {
        let envelope = envelope.clone();
        let now = RiseClock::now();
        let enqueued_at = stamps_enqueue.then_some(now);
        SaiWsDeferredDebugStore::shared(cx).update(cx, |store, cx| {
            store.record_ack_debt_history(
                envelope.service,
                envelope.method,
                envelope.body_json,
                envelope.cache_key,
                enqueued_at,
                now,
                outcome,
                cx,
            );
        });
    }

    pub fn debug_publish_snapshot(&self, cx: &mut App) {
        let cards: Vec<SaiWsDeferredDebugDiskRow> = self
            .records
            .iter()
            .map(|record| SaiWsDeferredDebugDiskRow {
                id: format!("w:{}", record.wire_request_id_millis),
                service: record.envelope.service.clone(),
                method: record.envelope.method.clone(),
                body_json: record.envelope.body_json.clone(),
                cache_key: record.envelope.cache_key.clone(),
            })
            .collect();
        SaiWsDeferredDebugStore::shared(cx).update(cx, |store, cx| {
            store.apply_ack_debt_snapshot(cards, cx);
        });
    }
}

