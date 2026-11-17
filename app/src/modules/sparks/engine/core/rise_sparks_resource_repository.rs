use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_store::rise_store_models::{
    RiseStoreMutation, RiseStoreNamespace, RiseStoreVersionPrecondition, RiseStoreWriteBatch,
    RiseStoredValue, RiseStoredValueWritePolicy,
};
use rise_engine::{CancellationError, RiseEngine};
use rise_canon::rise_ledger::HistoryFilter as RiseLedgerHistoryFilter;
use serde::{Deserialize, Serialize};

use crate::modules::sparks::engine::rise_sparks_engine_models::{
    RiseSparksBalanceState, RiseSparksEarningsDTO, RiseSparksHistoryPage, RiseSparksLimits,
};

/// earnings is optional so an envelope stored before it existed still decodes
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseSparksResourceRepositoryEnvelope {
    schema_version: i64,
    balance: RiseSparksBalanceState,
    history: BTreeMap<String, RiseSparksHistoryPage>,
    #[serde(default)]
    earnings: Option<RiseSparksEarningsDTO>,
}

pub struct RiseSparksResourceRepositoryCached {
    pub balance: RiseSparksBalanceState,
    pub history: HashMap<RiseLedgerHistoryFilter, RiseSparksHistoryPage>,
    pub earnings: Option<RiseSparksEarningsDTO>,
}

struct RiseSparksResourceRepositoryState {
    version_clock: i64,
    is_shutdown: bool,
}

static NAMESPACE: LazyLock<RiseStoreNamespace> =
    LazyLock::new(|| RiseStoreNamespace::new("sparks.resources.v1"));

pub struct RiseSparksResourceRepository {
    engine: Arc<RiseEngine>,
    key: String,
    state: RiseIsolated<RiseSparksResourceRepositoryState>,
}

impl RiseSparksResourceRepository {
    const SCHEMA_VERSION: i64 = 1;

    pub fn new(engine: Arc<RiseEngine>, account_id: &str) -> Self {
        Self {
            engine,
            key: Self::make_key(&[account_id, "wallet"]),
            state: RiseIsolated::new(RiseSparksResourceRepositoryState {
                version_clock: 0,
                is_shutdown: false,
            }),
        }
    }

    pub async fn cached(&self) -> anyhow::Result<Option<RiseSparksResourceRepositoryCached>> {
        self.ensure_running()?;
        let Some(value) = self.engine.value(NAMESPACE.clone(), self.key.clone()).await? else {
            return Ok(None);
        };
        self.state.with(|s| s.version_clock = s.version_clock.max(value.version));
        let data = value.data;
        let envelope = RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<RiseSparksResourceRepositoryEnvelope> {
                Ok(serde_json::from_slice(&data)?)
            })
            .await??;
        if envelope.schema_version != Self::SCHEMA_VERSION {
            return Ok(None);
        }
        let mut history = HashMap::new();
        for (raw, page) in envelope.history {
            let Some(filter) = RiseLedgerHistoryFilter::from_slug(&raw) else {
                continue;
            };
            history.insert(filter, page);
        }
        Ok(Some(RiseSparksResourceRepositoryCached {
            balance: envelope.balance,
            history,
            earnings: envelope.earnings,
        }))
    }

    pub async fn store(
        &self,
        balance: RiseSparksBalanceState,
        history: HashMap<RiseLedgerHistoryFilter, RiseSparksHistoryPage>,
        earnings: Option<RiseSparksEarningsDTO>,
    ) -> anyhow::Result<()> {
        self.ensure_running()?;
        let limit = usize::try_from(RiseSparksLimits::history_page_size()).unwrap_or(0);
        let mut heads = BTreeMap::new();
        for (filter, page) in history {
            let has_more = page.has_more || page.items.len() > limit;
            heads.insert(
                filter.slug().to_owned(),
                RiseSparksHistoryPage {
                    items: page.items.into_iter().take(limit).collect(),
                    has_more,
                },
            );
        }
        let envelope = RiseSparksResourceRepositoryEnvelope {
            schema_version: Self::SCHEMA_VERSION,
            balance,
            history: heads,
            earnings,
        };
        let data = RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
                let mut value = serde_json::to_value(&envelope)?;
                value.sort_all_objects();
                Ok(serde_json::to_vec(&value)?)
            })
            .await??;
        let now = Self::now_milliseconds();
        let version = self.state.with(|s| {
            s.version_clock = (s.version_clock + 1).max(now);
            s.version_clock
        });
        self.engine
            .write(RiseStoreWriteBatch::new(
                vec![RiseStoreMutation::SetValue {
                    namespace: NAMESPACE.clone(),
                    key: self.key.clone(),
                    value: RiseStoredValue::new(data, version, now),
                    precondition: RiseStoreVersionPrecondition::Any,
                    policy: RiseStoredValueWritePolicy::ServerAuthoritative,
                }],
                HashSet::new(),
            ))
            .await?;
        Ok(())
    }

    pub async fn shutdown(&self) {
        self.state.with(|s| s.is_shutdown = true);
    }

    fn ensure_running(&self) -> Result<(), CancellationError> {
        if self.state.with(|s| s.is_shutdown) { Err(CancellationError) } else { Ok(()) }
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
