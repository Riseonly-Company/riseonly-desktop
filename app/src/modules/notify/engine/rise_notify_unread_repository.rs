use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use rise_canon::RiseTimingCanon;
use rise_canon::rise_key::notify::UNREAD_COUNT;
use rise_canon::rise_request::notify::GetUnreadNotificationsCount;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::RiseEngine;
use rise_engine::rise_store::rise_store_models::{
    RiseStoreMutation, RiseStoreNamespace, RiseStoreVersionPrecondition, RiseStoreWriteBatch,
    RiseStoredValue, RiseStoredValueWritePolicy,
};

use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::notify::engine::core::rise_notify_rpc::RiseNotifyRPC;
use crate::modules::notify::engine::rise_notify_unread_presentation::RiseNotifyUnreadPresentation;

static NAMESPACE: LazyLock<RiseStoreNamespace> =
    LazyLock::new(|| RiseStoreNamespace::new("notify.resources.v1"));
static REFRESH_INTERVAL_MS: LazyLock<i64> =
    LazyLock::new(|| RiseTimingCanon::freshness_ms("notification_unread_count"));

struct RiseNotifyUnreadRepositoryState {
    unread_count: i64,
    version_clock: i64,
    request_generation: u64,
    last_refreshed_at_ms: i64,
    is_shutdown: bool,
}

pub struct RiseNotifyUnreadRepository {
    engine: Arc<RiseEngine>,
    account_id: String,
    account_epoch: String,
    session_id: Uuid,
    rpc: RiseNotifyRPC,
    resource_key: String,
    state: RiseIsolated<RiseNotifyUnreadRepositoryState>,
}

impl RiseNotifyUnreadRepository {
    pub fn new(
        engine: Arc<RiseEngine>,
        account_id: String,
        account_epoch: String,
        session_id: Uuid,
    ) -> Self {
        let resource_key = [account_id.as_str(), UNREAD_COUNT]
            .iter()
            .map(|component| format!("{}:{}", component.len(), component))
            .collect::<Vec<_>>()
            .join("|");
        Self {
            rpc: RiseNotifyRPC::new(engine.clone()),
            engine,
            account_id,
            account_epoch,
            session_id,
            resource_key,
            state: RiseIsolated::new(RiseNotifyUnreadRepositoryState {
                unread_count: 0,
                version_clock: 0,
                request_generation: 0,
                last_refreshed_at_ms: 0,
                is_shutdown: false,
            }),
        }
    }

    /// cache only read so the first published number on activation costs no round trip
    pub async fn activate(&self) -> i64 {
        if let Some(unread_count) =
            self.state.with(|state| state.is_shutdown.then_some(state.unread_count))
        {
            return unread_count;
        }
        let stored = self
            .engine
            .value(NAMESPACE.clone(), self.resource_key.clone())
            .await
            .ok()
            .flatten();
        self.state.with(|state| {
            if let Some(stored) = stored
                && !state.is_shutdown
            {
                state.version_clock = state.version_clock.max(stored.version);
                state.unread_count = Self::decode(&stored.data);
            }
            state.unread_count
        })
    }

    pub async fn load(&self, force: bool) {
        if self.account_id.is_empty() {
            return;
        }
        let now = Self::now_milliseconds();
        let generation = self.state.with(|state| {
            if state.is_shutdown {
                return None;
            }
            if !force
                && state.last_refreshed_at_ms > 0
                && now - state.last_refreshed_at_ms < *REFRESH_INTERVAL_MS
            {
                return None;
            }
            state.request_generation = state.request_generation.wrapping_add(1);
            Some(state.request_generation)
        });
        let Some(generation) = generation else {
            return;
        };
        let is_current_account = SaiAccountCacheScope::is_current_account_epoch(
            Some(&self.account_id),
            Some(&self.account_epoch),
        );
        if !is_current_account || !self.is_current_request(generation) {
            return;
        }

        let Ok(response) =
            self.rpc.get_unread_notifications_count(GetUnreadNotificationsCount {}).await
        else {
            return;
        };
        let accepted = self.state.with(|state| {
            if state.is_shutdown || generation != state.request_generation {
                return false;
            }
            state.last_refreshed_at_ms = Self::now_milliseconds();
            true
        });
        if !accepted {
            return;
        }
        self.install(response.unread_count).await;
    }

    /// server total from the realtime frame or the lists total_unread, newer than anything local
    pub async fn apply_authoritative(&self, count: i64) {
        let accepted = self.state.with(|state| {
            if state.is_shutdown {
                return false;
            }
            state.last_refreshed_at_ms = Self::now_milliseconds();
            true
        });
        if !accepted {
            return;
        }
        self.install(count).await;
    }

    /// leaving the notifications screen marks everything read
    /// local first so the badge clears on the same turn as the navigation
    pub async fn clear(&self) {
        self.install(0).await;
    }

    /// one row toggled on the list
    /// the list total follows on its next page, this just keeps the two numbers from disagreeing meanwhile
    pub async fn adjust(&self, by: i64) {
        self.install_with(|current| current + by).await;
    }

    pub async fn shutdown(&self) {
        self.state.with(|state| state.is_shutdown = true);
    }

    fn is_current_request(&self, generation: u64) -> bool {
        self.state.with(|state| !state.is_shutdown && generation == state.request_generation)
    }

    async fn install(&self, count: i64) {
        self.install_with(|_| count).await;
    }

    async fn install_with(&self, count: impl FnOnce(i64) -> i64) {
        let installed = self.state.with(|state| {
            if state.is_shutdown {
                return None;
            }
            state.request_generation = state.request_generation.wrapping_add(1);
            let next = count(state.unread_count).max(0);
            let changed = next != state.unread_count;
            state.unread_count = next;
            if !changed {
                return Some((next, None));
            }
            state.version_clock = (state.version_clock + 1).max(Self::now_milliseconds());
            Some((next, Some(state.version_clock)))
        });
        let Some((next, version)) = installed else {
            return;
        };
        self.publish(next);
        let Some(version) = version else {
            return;
        };
        let _ = self
            .engine
            .write(RiseStoreWriteBatch::new(
                vec![RiseStoreMutation::SetValue {
                    namespace: NAMESPACE.clone(),
                    key: self.resource_key.clone(),
                    value: RiseStoredValue::new(
                        Self::encode(next),
                        version,
                        Self::now_milliseconds(),
                    ),
                    precondition: RiseStoreVersionPrecondition::Any,
                    policy: RiseStoredValueWritePolicy::ServerAuthoritative,
                }],
                HashSet::new(),
            ))
            .await;
    }

    fn publish(&self, next: i64) {
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        let Some(main) = RiseEngineMainHop::installed() else {
            return;
        };
        main.post(move |cx| {
            RiseNotifyUnreadPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish(next, &account_id, &account_epoch, session_id, cx);
            });
        });
    }

    fn encode(count: i64) -> Vec<u8> {
        count.max(0).to_string().into_bytes()
    }

    fn decode(data: &[u8]) -> i64 {
        String::from_utf8_lossy(data).parse::<i64>().unwrap_or(0).max(0)
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}
