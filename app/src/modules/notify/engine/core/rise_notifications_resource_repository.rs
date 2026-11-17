use std::collections::HashSet;
use std::sync::{Arc, LazyLock};

use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_store::rise_store_models::{
    RiseStoreMutation, RiseStoreNamespace, RiseStoreVersionPrecondition, RiseStoreWriteBatch,
    RiseStoredValue, RiseStoredValueWritePolicy,
};
use rise_engine::{CancellationError, RiseEngine};
use serde::{Deserialize, Serialize};

use crate::modules::notify::engine::core::rise_announcements_resource_repository::RiseAnnouncementsCacheSource;
use crate::modules::notify::stores::notify_actions::notify_types::NotificationItem;

static NAMESPACE: LazyLock<RiseStoreNamespace> =
    LazyLock::new(|| RiseStoreNamespace::new("notify.resources.v1"));
const CURRENT_SCHEMA_VERSION: i64 = 1;
const FEED_COMPONENT: &str = "feed";

#[derive(Clone, Debug)]
pub struct RiseNotificationsResourceSnapshot {
    pub items: Vec<NotificationItem>,
    pub total_unread: i64,
    pub revision: i64,
    pub source: RiseAnnouncementsCacheSource,
}

struct RiseNotificationsResourceRepositoryMemoryEntry {
    data: Arc<Vec<u8>>,
    revision: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseNotificationsResourceRepositoryEnvelope {
    schema_version: i64,
    total_unread: i64,
    items: Vec<NotificationItem>,
}

struct RiseNotificationsResourceRepositoryState {
    memory_entry: Option<RiseNotificationsResourceRepositoryMemoryEntry>,
    version_clock: i64,
    is_shutdown: bool,
}

/// head of the notification feed in the encrypted store
/// so the first frame after a cold launch is the feed the reader last saw
/// keyed by account only
pub struct RiseNotificationsResourceRepository {
    engine: Arc<RiseEngine>,
    resource_key: String,
    maximum_memory_bytes: usize,
    state: RiseIsolated<RiseNotificationsResourceRepositoryState>,
}

impl RiseNotificationsResourceRepository {
    pub const DEFAULT_MAXIMUM_MEMORY_BYTES: usize = 512 * 1_024;

    pub fn new(engine: Arc<RiseEngine>, account_id: String) -> Self {
        Self::with_maximum_memory_bytes(engine, account_id, Self::DEFAULT_MAXIMUM_MEMORY_BYTES)
    }

    pub fn with_maximum_memory_bytes(
        engine: Arc<RiseEngine>,
        account_id: String,
        maximum_memory_bytes: usize,
    ) -> Self {
        assert!(maximum_memory_bytes > 0);
        Self {
            engine,
            resource_key: Self::make_key(&[account_id.as_str(), FEED_COMPONENT]),
            maximum_memory_bytes,
            state: RiseIsolated::new(RiseNotificationsResourceRepositoryState {
                memory_entry: None,
                version_clock: 0,
                is_shutdown: false,
            }),
        }
    }

    pub async fn cached(&self) -> anyhow::Result<RiseNotificationsResourceSnapshot> {
        let memory = self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            Ok(s.memory_entry
                .as_ref()
                .map(|entry| (entry.data.clone(), entry.revision)))
        })?;

        if let Some((data, revision)) = memory {
            let envelope = Self::decode(data).await?;
            self.ensure_running()?;
            return Ok(RiseNotificationsResourceSnapshot {
                items: envelope.items,
                total_unread: envelope.total_unread,
                revision,
                source: RiseAnnouncementsCacheSource::Memory,
            });
        }

        let stored = self
            .engine
            .value(NAMESPACE.clone(), self.resource_key.clone())
            .await?;
        self.ensure_running()?;
        let Some(stored) = stored else {
            return Ok(RiseNotificationsResourceSnapshot {
                items: Vec::new(),
                total_unread: 0,
                revision: 0,
                source: RiseAnnouncementsCacheSource::Miss,
            });
        };

        let data = Arc::new(stored.data);
        let envelope = Self::decode(data.clone()).await?;
        self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            s.version_clock = s.version_clock.max(stored.version);
            Self::install_memory(s, self.maximum_memory_bytes, data, stored.version);
            Ok(())
        })?;
        Ok(RiseNotificationsResourceSnapshot {
            items: envelope.items,
            total_unread: envelope.total_unread,
            revision: stored.version,
            source: RiseAnnouncementsCacheSource::PersistentStore,
        })
    }

    pub async fn store(&self, items: &[NotificationItem], total_unread: i64) -> anyhow::Result<i64> {
        let version = self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            Ok(Self::next_version(s))
        })?;
        let data = Arc::new(Self::encode(items.to_vec(), total_unread).await?);
        self.ensure_running()?;

        self.engine
            .write(Self::set_value_batch(&self.resource_key, data.as_ref().clone(), version))
            .await?;
        self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            Self::install_memory(s, self.maximum_memory_bytes, data, version);
            Ok(())
        })?;
        Ok(version)
    }

    pub async fn shutdown(&self) {
        self.state.with(|s| {
            s.is_shutdown = true;
            s.memory_entry = None;
        });
    }

    fn ensure_running(&self) -> Result<(), CancellationError> {
        if self.state.with(|s| s.is_shutdown) {
            Err(CancellationError)
        } else {
            Ok(())
        }
    }

    fn next_version(state: &mut RiseNotificationsResourceRepositoryState) -> i64 {
        state.version_clock = (state.version_clock + 1).max(Self::now_milliseconds());
        state.version_clock
    }

    fn install_memory(
        state: &mut RiseNotificationsResourceRepositoryState,
        maximum_memory_bytes: usize,
        data: Arc<Vec<u8>>,
        revision: i64,
    ) {
        let newest = state.memory_entry.as_ref().map_or(0, |entry| entry.revision);
        if data.len() > maximum_memory_bytes || revision < newest {
            return;
        }
        state.memory_entry = Some(RiseNotificationsResourceRepositoryMemoryEntry { data, revision });
    }

    fn set_value_batch(key: &str, data: Vec<u8>, version: i64) -> RiseStoreWriteBatch {
        RiseStoreWriteBatch::new(
            vec![RiseStoreMutation::SetValue {
                namespace: NAMESPACE.clone(),
                key: key.to_owned(),
                value: RiseStoredValue::new(data, version, Self::now_milliseconds()),
                precondition: RiseStoreVersionPrecondition::Any,
                policy: RiseStoredValueWritePolicy::ServerAuthoritative,
            }],
            HashSet::new(),
        )
    }

    /// coding runs off the callers task
    /// the read sits on the activation path and must not delay the first frame
    async fn encode(items: Vec<NotificationItem>, total_unread: i64) -> anyhow::Result<Vec<u8>> {
        let envelope = RiseNotificationsResourceRepositoryEnvelope {
            schema_version: CURRENT_SCHEMA_VERSION,
            total_unread,
            items,
        };
        let encoded = RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
                let mut value = serde_json::to_value(&envelope)?;
                value.sort_all_objects();
                Ok(serde_json::to_vec(&value)?)
            })
            .await??;
        Ok(encoded)
    }

    async fn decode(data: Arc<Vec<u8>>) -> anyhow::Result<RiseNotificationsResourceRepositoryEnvelope> {
        let decoded = RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<RiseNotificationsResourceRepositoryEnvelope> {
                let envelope: RiseNotificationsResourceRepositoryEnvelope =
                    serde_json::from_slice(&data)?;
                if envelope.schema_version != CURRENT_SCHEMA_VERSION {
                    return Ok(RiseNotificationsResourceRepositoryEnvelope {
                        schema_version: CURRENT_SCHEMA_VERSION,
                        total_unread: 0,
                        items: Vec::new(),
                    });
                }
                Ok(envelope)
            })
            .await??;
        Ok(decoded)
    }

    fn make_key(components: &[&str]) -> String {
        components
            .iter()
            .map(|component| format!("{}:{}", component.len(), component))
            .collect::<Vec<_>>()
            .join("|")
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}
