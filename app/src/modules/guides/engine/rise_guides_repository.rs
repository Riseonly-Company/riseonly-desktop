use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use gpui::App;
use rise_canon::{RiseGuideCanon, RiseGuideTrigger, rise_request};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::{character_prefix, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_store::rise_store_models::{
    RiseStoreMutation, RiseStoreNamespace, RiseStoreVersionPrecondition, RiseStoreWriteBatch,
    RiseStoredValue, RiseStoredValueWritePolicy,
};
use rise_engine::{CancellationError, RiseEngine};
use rise_i18n::language_store::LanguageStore;
use rise_theme::RiseThemePlatform;
use serde::{Deserialize, Serialize};
use tokio::task::AbortHandle;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::guides::engine::core::rise_guides_rpc::RiseGuidesRPC;
use crate::modules::guides::engine::rise_guides_domain::RiseGuidesDomainRegistry;
use crate::modules::guides::engine::rise_guides_models::{
    RiseGuide, RiseGuideAck, RiseGuideAutoStartPolicy, RiseGuideReceipt, RiseGuidesCacheSource,
};
use crate::modules::guides::engine::rise_guides_presentation::{
    RiseGuidesPresentation, RiseGuidesSnapshot,
};
use crate::modules::notify::engine::core::rise_release_notes_seen_store::RiseReleaseNotesDeviceContext;

/// the running refresh, done resolves when it ends without cancelling it abort cancels it
/// and generation is its identity
struct RiseGuidesRepositoryInFlight {
    generation: u64,
    abort: AbortHandle,
    done: Shared<BoxFuture<'static, ()>>,
}

struct RiseGuidesRepositoryState {
    snapshot: RiseGuidesSnapshot,
    catalogue: Vec<RiseGuide>,
    receipts: HashMap<i64, RiseGuideReceipt>,
    auto_start: bool,
    auto_start_writes: u64,
    request_generation: u64,
    last_refreshed_at_ms: i64,
    is_shutdown: bool,
    in_flight_refresh: Option<RiseGuidesRepositoryInFlight>,
}

enum RiseGuidesRepositoryLoadStep {
    Skip,
    Join(Shared<BoxFuture<'static, ()>>),
    Start(u64, Shared<BoxFuture<'static, ()>>),
}

struct RiseGuidesRepositoryRequestContext {
    language: String,
    is_current_session: bool,
}

/// owns the guides this account may be offered
/// cache first, memory then encrypted store then one hidden refresh inside the freshness window
/// a network failure never clears a valid catalogue, no loader or error state here
pub struct RiseGuidesRepository {
    account_id: String,
    account_epoch: String,
    session_id: Uuid,
    rpc: RiseGuidesRPC,
    resource: RiseGuidesResourceRepository,
    app_version: String,
    state: RiseIsolated<RiseGuidesRepositoryState>,
}

fn guide_platform() -> String {
    RiseThemePlatform::THIS_DEVICE.raw_value().to_owned()
}

impl RiseGuidesRepository {
    const REFRESH_INTERVAL_MS: i64 = RiseGuideCanon::TIMINGS_CATALOGUE_FRESHNESS_MS;
    const MAXIMUM_OFFERED: i64 = RiseGuideCanon::LIMITS_MAX_ACTIVE_GUIDES_PER_ACCOUNT;
    const MAXIMUM_REACHED_STEP: i64 = RiseGuideCanon::LIMITS_MAX_STEPS_PER_GUIDE;

    pub fn new(engine: Arc<RiseEngine>, account_id: String, account_epoch: String, session_id: Uuid) -> Self {
        let rpc = RiseGuidesRPC::new(engine.clone());
        let app_version = RiseReleaseNotesDeviceContext::app_version();
        let resource = RiseGuidesResourceRepository::new(engine, account_id.clone());
        Self {
            account_id,
            account_epoch,
            session_id,
            rpc,
            resource,
            app_version,
            state: RiseIsolated::new(RiseGuidesRepositoryState {
                snapshot: RiseGuidesSnapshot::empty(),
                catalogue: Vec::new(),
                receipts: HashMap::new(),
                auto_start: true,
                auto_start_writes: 0,
                request_generation: 0,
                last_refreshed_at_ms: 0,
                is_shutdown: false,
                in_flight_refresh: None,
            }),
        }
    }

    /// reads what is already known with no network, called during activation so the first
    /// snapshot costs no round trip
    pub async fn activate(&self) -> RiseGuidesSnapshot {
        if self.state.with(|s| s.is_shutdown) {
            return self.current();
        }
        let Ok(cached) = self.resource.cached().await else {
            return self.current();
        };
        let app_version = self.app_version.as_str();
        self.state.with(|s| {
            if s.is_shutdown {
                return s.snapshot.clone();
            }
            if cached.guides.is_empty() && cached.receipts.is_empty() && cached.auto_start {
                return s.snapshot.clone();
            }
            s.catalogue = cached.guides;
            let mut table: HashMap<i64, RiseGuideReceipt> = HashMap::new();
            for receipt in cached.receipts {
                let folded = match table.get(&receipt.guide_id) {
                    Some(known) => known.ratcheted(&receipt),
                    None => receipt,
                };
                table.insert(folded.guide_id, folded);
            }
            s.receipts = table;
            s.auto_start = cached.auto_start;
            let offered = Self::offerable(s, app_version);
            let everything = Self::sorted_catalogue(s, app_version);
            let receipts = s.receipts.clone();
            let auto_start = s.auto_start;
            let source = cached.source;
            let _ = Self::apply(s, |next| {
                next.guides = offered;
                next.catalogue = everything;
                next.receipts = receipts;
                next.auto_start = auto_start;
                next.cache_source = source;
                next.is_refreshing = false;
            });
            s.snapshot.clone()
        })
    }

    pub async fn load(self: &Arc<Self>, force: bool) {
        if self.state.with(|s| s.is_shutdown) || self.account_id.is_empty() {
            return;
        }

        let now = Self::now_milliseconds();
        let weak = Arc::downgrade(self);
        let (step, published) = self.state.with(|s| {
            if !force && s.last_refreshed_at_ms > 0 && now - s.last_refreshed_at_ms < Self::REFRESH_INTERVAL_MS {
                return (RiseGuidesRepositoryLoadStep::Skip, None);
            }
            if let Some(in_flight) = s.in_flight_refresh.as_ref()
                && !force
            {
                return (RiseGuidesRepositoryLoadStep::Join(in_flight.done.clone()), None);
            }
            if let Some(in_flight) = s.in_flight_refresh.take() {
                in_flight.abort.abort();
            }

            s.request_generation = s.request_generation.wrapping_add(1);
            let generation = s.request_generation;
            let published = Self::apply(s, |next| next.is_refreshing = true).then(|| s.snapshot.clone());

            let handle = RiseAccountRuntime::current().spawn(async move {
                if let Some(this) = weak.upgrade() {
                    this.perform_refresh(generation).await;
                }
            });
            let abort = handle.abort_handle();
            let done: Shared<BoxFuture<'static, ()>> = async move {
                let _ = handle.await;
            }
            .boxed()
            .shared();
            s.in_flight_refresh = Some(RiseGuidesRepositoryInFlight {
                generation,
                abort,
                done: done.clone(),
            });
            (RiseGuidesRepositoryLoadStep::Start(generation, done), published)
        });
        self.post(published);

        match step {
            RiseGuidesRepositoryLoadStep::Skip => {}
            RiseGuidesRepositoryLoadStep::Join(done) => done.await,
            RiseGuidesRepositoryLoadStep::Start(generation, done) => {
                done.await;
                self.state.with(|s| {
                    if s.in_flight_refresh.as_ref().is_some_and(|in_flight| in_flight.generation == generation) {
                        s.in_flight_refresh = None;
                    }
                });
            }
        }
    }

    async fn perform_refresh(&self, generation: u64) {
        let context = self.current_context().await;
        let writes_at_start = self.state.with(|s| {
            if s.is_shutdown || generation != s.request_generation || !context.is_current_session {
                return None;
            }
            Some(s.auto_start_writes)
        });
        let Some(writes_at_start) = writes_at_start else {
            return;
        };

        let request = rise_request::notify::GetPendingGuides {
            app_version: self.app_version.clone(),
            platform: guide_platform(),
            include_finished: Some(true),
            locale: Some(context.language),
        };
        let response = match self.rpc.get_pending_guides(request).await {
            Ok(response) => response,
            Err(_) => {
                let published = self.state.with(|s| {
                    if s.is_shutdown || generation != s.request_generation {
                        return None;
                    }
                    Self::apply(s, |next| next.is_refreshing = false).then(|| s.snapshot.clone())
                });
                self.post(published);
                return;
            }
        };

        let guides: Vec<RiseGuide> = response.guides.iter().filter_map(RiseGuide::from_dto).collect();
        let app_version = self.app_version.as_str();
        let published = self.state.with(|s| {
            if s.is_shutdown || generation != s.request_generation {
                return None;
            }
            s.last_refreshed_at_ms = Self::now_milliseconds();
            Self::prune_receipts(s, &guides);
            s.catalogue = guides;
            for dto in &response.receipts {
                let Some(receipt) = RiseGuideReceipt::from_dto(dto, s.last_refreshed_at_ms) else {
                    continue;
                };
                Self::record(s, receipt);
            }
            if writes_at_start == s.auto_start_writes {
                s.auto_start = response.auto_start;
            }
            let offered = Self::offerable(s, app_version);
            let everything = Self::sorted_catalogue(s, app_version);
            let receipts = s.receipts.clone();
            let auto_start = s.auto_start;
            Some(
                Self::apply(s, |next| {
                    next.guides = offered;
                    next.catalogue = everything;
                    next.receipts = receipts;
                    next.auto_start = auto_start;
                    next.cache_source = RiseGuidesCacheSource::Network;
                    next.is_refreshing = false;
                })
                .then(|| s.snapshot.clone()),
            )
        });
        let Some(published) = published else {
            return;
        };
        self.post(published);

        self.persist().await;
    }

    /// includes a guide this account has already finished
    pub async fn guide(&self, id: i64) -> Option<RiseGuide> {
        self.lookup(id, "").await
    }

    pub async fn guide_by_key(&self, key: &str) -> Option<RiseGuide> {
        self.lookup(0, key).await
    }

    /// answers its caller and publishes nothing so it carries no request generation
    /// two concurrent lookups arent each other stale completion
    /// a guide fetched by name must not land in the offered list
    async fn lookup(&self, id: i64, key: &str) -> Option<RiseGuide> {
        if self.state.with(|s| s.is_shutdown) {
            return None;
        }
        let key = character_prefix(
            trimming_whitespaces_and_newlines(key),
            RiseGuideCanon::LIMITS_KEY_MAX_CHARS.max(0) as usize,
        )
        .to_owned();
        if id <= 0 && key.is_empty() {
            return None;
        }

        let known = self.state.with(|s| {
            s.catalogue
                .iter()
                .find(|guide| if id > 0 { guide.id == id } else { guide.key == key })
                .cloned()
        });
        if let Some(known) = known {
            return Some(known);
        }

        let context = self.current_context().await;
        if self.state.with(|s| s.is_shutdown) || !context.is_current_session {
            return None;
        }

        let response = self
            .rpc
            .get_guide(rise_request::notify::GetGuide {
                app_version: self.app_version.clone(),
                platform: guide_platform(),
                guide_id: Some(id.max(0)),
                key: Some(key),
                locale: Some(context.language),
            })
            .await
            .ok()?;
        if self.state.with(|s| s.is_shutdown) {
            return None;
        }
        if !response.found {
            return None;
        }
        let dto = response.guide.as_ref()?;
        RiseGuide::from_dto(dto)
    }

    /// local receipt first so the guide stops being offered on the same turn, then the ack which
    /// stops it on the other devices
    /// a shown started without the reader asking counts one automatic start here too
    pub async fn acknowledge(
        &self,
        guide_id: i64,
        guide_revision: i64,
        ack: RiseGuideAck,
        reached_step: i64,
        origin: RiseGuideTrigger,
    ) {
        if guide_id <= 0 {
            return;
        }
        let revision = guide_revision.max(1);
        let step = reached_step.max(0).min(Self::MAXIMUM_REACHED_STEP);
        let now = Self::now_milliseconds();
        let counts_auto_start = ack == RiseGuideAck::Shown && origin != RiseGuideTrigger::Manual;
        let app_version = self.app_version.as_str();
        let published = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            let previous_starts = s.receipts.get(&guide_id).map_or(0, |receipt| receipt.auto_starts);
            Self::record(
                s,
                RiseGuideReceipt::new(
                    guide_id,
                    revision,
                    ack.outcome(),
                    step,
                    if counts_auto_start { previous_starts + 1 } else { 0 },
                    if counts_auto_start { now } else { 0 },
                    now,
                ),
            );
            let offered = Self::offerable(s, app_version);
            let receipts = s.receipts.clone();
            Some(
                Self::apply(s, |next| {
                    next.guides = offered;
                    next.receipts = receipts;
                })
                .then(|| s.snapshot.clone()),
            )
        });
        let Some(published) = published else {
            return;
        };
        self.post(published);
        self.persist().await;
        if self.state.with(|s| s.is_shutdown) {
            return;
        }

        let context = self.current_context().await;
        if !context.is_current_session {
            return;
        }

        let _ = self
            .rpc
            .ack_guide(rise_request::notify::AckGuide {
                app_version: self.app_version.clone(),
                failure: Some(ack.failure().map(|failure| failure.raw_value()).unwrap_or_default().to_owned()),
                guide_id,
                guide_revision: Some(revision),
                locale: Some(context.language),
                origin: Some(origin.raw_value().to_owned()),
                outcome: ack.outcome().raw_value().to_owned(),
                platform: guide_platform(),
                reached_step: Some(step),
            })
            .await;
    }

    pub async fn set_auto_start(&self, enabled: bool) -> bool {
        let started = self.state.with(|s| {
            if s.is_shutdown {
                return Err(s.auto_start);
            }
            let previous = s.auto_start;
            s.auto_start_writes = s.auto_start_writes.wrapping_add(1);
            let write = s.auto_start_writes;
            s.auto_start = enabled;
            let published = Self::apply(s, |next| next.auto_start = enabled).then(|| s.snapshot.clone());
            Ok((previous, write, published))
        });
        let (previous, write, published) = match started {
            Ok(started) => started,
            Err(current) => return current,
        };
        self.post(published);
        self.persist().await;
        if self.state.with(|s| s.is_shutdown) {
            return self.auto_start();
        }

        let context = self.current_context().await;
        if !context.is_current_session || self.state.with(|s| s.is_shutdown) {
            return self.auto_start();
        }

        let settled = match self
            .rpc
            .set_guide_auto_start(rise_request::notify::SetGuideAutoStart::new(enabled))
            .await
        {
            Ok(response) => response.auto_start,
            Err(_) => previous,
        };
        let changed = self.state.with(|s| {
            if s.is_shutdown || write != s.auto_start_writes || settled == s.auto_start {
                return None;
            }
            s.auto_start = settled;
            Some(Self::apply(s, |next| next.auto_start = settled).then(|| s.snapshot.clone()))
        });
        if let Some(published) = changed {
            self.post(published);
            self.persist().await;
        }
        self.auto_start()
    }

    pub async fn shutdown(&self) {
        let stopped = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            s.is_shutdown = true;
            if let Some(in_flight) = s.in_flight_refresh.take() {
                in_flight.abort.abort();
            }
            s.catalogue.clear();
            s.receipts.clear();
            true
        });
        if !stopped {
            return;
        }
        self.resource.shutdown().await;
    }

    fn current(&self) -> RiseGuidesSnapshot {
        self.state.with(|s| s.snapshot.clone())
    }

    fn auto_start(&self) -> bool {
        self.state.with(|s| s.auto_start)
    }

    /// every guide this build can run, ordered as the runtime breaks a tie priority first then
    /// the older guide
    /// version gate stays since a guide whose anchors this build lacks isnt replayable either
    fn sorted_catalogue(s: &RiseGuidesRepositoryState, app_version: &str) -> Vec<RiseGuide> {
        let mut sorted: Vec<RiseGuide> =
            s.catalogue.iter().filter(|guide| guide.is_supported(app_version)).cloned().collect();
        sorted.sort_by(Self::runtime_order);
        sorted
    }

    /// guides that may start on their own as of now
    /// the auto start centre asks the policy again at start time with the receipts published
    /// beside this list since cooldown is a question of time
    fn offerable(s: &RiseGuidesRepositoryState, app_version: &str) -> Vec<RiseGuide> {
        let now = Self::now_milliseconds();
        let mut filtered: Vec<RiseGuide> = s
            .catalogue
            .iter()
            .filter(|guide| {
                guide.is_supported(app_version)
                    && RiseGuideAutoStartPolicy::allows_auto_start(
                        s.receipts.get(&guide.id),
                        guide.revision,
                        guide.reteach,
                        now,
                    )
            })
            .cloned()
            .collect();
        filtered.sort_by(Self::runtime_order);
        filtered.truncate(Self::MAXIMUM_OFFERED.max(0) as usize);
        filtered
    }

    fn runtime_order(lhs: &RiseGuide, rhs: &RiseGuide) -> std::cmp::Ordering {
        if lhs.priority == rhs.priority { lhs.id.cmp(&rhs.id) } else { rhs.priority.cmp(&lhs.priority) }
    }

    fn record(s: &mut RiseGuidesRepositoryState, receipt: RiseGuideReceipt) {
        let folded = match s.receipts.get(&receipt.guide_id) {
            Some(known) => known.ratcheted(&receipt),
            None => receipt,
        };
        s.receipts.insert(folded.guide_id, folded);
        let maximum = Self::MAXIMUM_OFFERED.max(0) as usize;
        if s.receipts.len() <= maximum {
            return;
        }
        let mut newest: Vec<RiseGuideReceipt> = s.receipts.drain().map(|(_, receipt)| receipt).collect();
        newest.sort_by(|lhs, rhs| {
            (rhs.recorded_at_ms, rhs.guide_id).cmp(&(lhs.recorded_at_ms, lhs.guide_id))
        });
        newest.truncate(maximum);
        s.receipts = newest.into_iter().map(|receipt| (receipt.guide_id, receipt)).collect();
    }

    fn prune_receipts(s: &mut RiseGuidesRepositoryState, guides: &[RiseGuide]) {
        let offered: HashSet<i64> = guides.iter().map(|guide| guide.id).collect();
        s.receipts.retain(|guide_id, _| offered.contains(guide_id));
    }

    /// cold launch reads this file before any network work, an in memory only receipt would
    /// re-offer a finished guide whenever the ack hadnt landed
    async fn persist(&self) {
        let (guides, written, auto_start) = self.state.with(|s| {
            let mut written: Vec<RiseGuideReceipt> = s.receipts.values().cloned().collect();
            written.sort_by_key(|receipt| receipt.guide_id);
            (s.catalogue.clone(), written, s.auto_start)
        });
        let _ = self.resource.store(guides, written, auto_start).await;
    }

    /// derives the revision and reports whether anything moved, the revision changes only
    /// together with the content it describes
    fn apply(s: &mut RiseGuidesRepositoryState, mutate: impl FnOnce(&mut RiseGuidesSnapshot)) -> bool {
        let mut next = s.snapshot.clone();
        mutate(&mut next);
        next.revision = s.snapshot.revision;
        if next == s.snapshot {
            return false;
        }
        next.revision = s.snapshot.revision.wrapping_add(1);
        s.snapshot = next;
        true
    }

    /// main thread half of publish, the isolated half is apply run in the same turn as the
    /// decision so state and presentation cant disagree
    fn post(&self, next: Option<RiseGuidesSnapshot>) {
        let Some(next) = next else {
            return;
        };
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        hop.post(move |cx: &mut App| {
            RiseGuidesPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish(next, &account_id, &account_epoch, session_id, cx);
            });
        });
    }

    /// language and session check in one main thread hop, both are read on every network turn
    /// the registry is part of the check since an account can be current again after a re-login
    /// while this repository belongs to the old session
    async fn current_context(&self) -> RiseGuidesRepositoryRequestContext {
        let not_current = RiseGuidesRepositoryRequestContext {
            language: String::new(),
            is_current_session: false,
        };
        let Some(hop) = RiseEngineMainHop::installed() else {
            return not_current;
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        let answer = hop
            .run(move |cx| {
                async move {
                    Ok::<_, anyhow::Error>(cx.update(|cx| {
                        let is_current_session = SaiAccountCacheScope::is_current_account_epoch(
                            Some(&account_id),
                            Some(&account_epoch),
                        ) && RiseGuidesDomainRegistry::shared(cx).read(cx).matches(
                            &account_id,
                            &account_epoch,
                            session_id,
                        );
                        (LanguageStore::shared(cx).code().to_owned(), is_current_session)
                    }))
                }
                .boxed_local()
            })
            .await;
        match answer {
            Ok((language, is_current_session)) => RiseGuidesRepositoryRequestContext {
                language,
                is_current_session,
            },
            Err(_) => not_current,
        }
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiseGuidesResourceSnapshot {
    pub guides: Vec<RiseGuide>,
    pub receipts: Vec<RiseGuideReceipt>,
    pub auto_start: bool,
    pub revision: i64,
    pub source: RiseGuidesCacheSource,
}

struct RiseGuidesResourceRepositoryMemoryEntry {
    data: Arc<Vec<u8>>,
    revision: i64,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RiseGuidesResourceRepositoryEnvelope {
    schema_version: i64,
    guides: Vec<RiseGuide>,
    receipts: Vec<RiseGuideReceipt>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auto_start: Option<bool>,
}

struct RiseGuidesResourceRepositoryState {
    memory_entry: Option<RiseGuidesResourceRepositoryMemoryEntry>,
    version_clock: i64,
    is_shutdown: bool,
}

static NAMESPACE: LazyLock<RiseStoreNamespace> =
    LazyLock::new(|| RiseStoreNamespace::new("guides.resources.v1"));
const GUIDES_COMPONENT: &str = "guides";

/// bounded memory accelerator over the encrypted store, one account guide catalogue and the
/// receipts this device wrote
/// eviction is safe, a miss falls through to the store instead of saying no guides
pub struct RiseGuidesResourceRepository {
    engine: Arc<RiseEngine>,
    resource_key: String,
    maximum_memory_bytes: usize,
    state: RiseIsolated<RiseGuidesResourceRepositoryState>,
}

impl RiseGuidesResourceRepository {
    const CURRENT_SCHEMA_VERSION: i64 = 2;
    pub const DEFAULT_MAXIMUM_MEMORY_BYTES: usize = 256 * 1_024;

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
            resource_key: Self::make_key(&[account_id.as_str(), GUIDES_COMPONENT]),
            maximum_memory_bytes,
            state: RiseIsolated::new(RiseGuidesResourceRepositoryState {
                memory_entry: None,
                version_clock: 0,
                is_shutdown: false,
            }),
        }
    }

    pub async fn cached(&self) -> anyhow::Result<RiseGuidesResourceSnapshot> {
        let memory = self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            Ok(s.memory_entry.as_ref().map(|entry| (entry.data.clone(), entry.revision)))
        })?;

        if let Some((data, revision)) = memory {
            let envelope = Self::decode(data).await?;
            self.ensure_running()?;
            return Ok(RiseGuidesResourceSnapshot {
                guides: envelope.guides,
                receipts: envelope.receipts,
                auto_start: envelope.auto_start.unwrap_or(true),
                revision,
                source: RiseGuidesCacheSource::Memory,
            });
        }

        let stored = self.engine.value(NAMESPACE.clone(), self.resource_key.clone()).await?;
        self.ensure_running()?;
        let Some(stored) = stored else {
            return Ok(RiseGuidesResourceSnapshot {
                guides: Vec::new(),
                receipts: Vec::new(),
                auto_start: true,
                revision: 0,
                source: RiseGuidesCacheSource::Miss,
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
        Ok(RiseGuidesResourceSnapshot {
            guides: envelope.guides,
            receipts: envelope.receipts,
            auto_start: envelope.auto_start.unwrap_or(true),
            revision: stored.version,
            source: RiseGuidesCacheSource::PersistentStore,
        })
    }

    pub async fn store(
        &self,
        guides: Vec<RiseGuide>,
        receipts: Vec<RiseGuideReceipt>,
        auto_start: bool,
    ) -> anyhow::Result<i64> {
        let version = self.state.with(|s| {
            if s.is_shutdown {
                return Err(CancellationError);
            }
            Ok(Self::next_version(s))
        })?;
        let data = Arc::new(Self::encode(guides, receipts, auto_start).await?);
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
        if self.state.with(|s| s.is_shutdown) { Err(CancellationError) } else { Ok(()) }
    }

    fn next_version(state: &mut RiseGuidesResourceRepositoryState) -> i64 {
        state.version_clock = (state.version_clock + 1).max(Self::now_milliseconds());
        state.version_clock
    }

    fn install_memory(
        state: &mut RiseGuidesResourceRepositoryState,
        maximum_memory_bytes: usize,
        data: Arc<Vec<u8>>,
        revision: i64,
    ) {
        let newest = state.memory_entry.as_ref().map_or(0, |entry| entry.revision);
        if data.len() > maximum_memory_bytes || revision < newest {
            return;
        }
        state.memory_entry = Some(RiseGuidesResourceRepositoryMemoryEntry { data, revision });
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

    /// coding runs off the caller task since this sits on the activation path and must not block
    /// the first frame
    async fn encode(
        guides: Vec<RiseGuide>,
        receipts: Vec<RiseGuideReceipt>,
        auto_start: bool,
    ) -> anyhow::Result<Vec<u8>> {
        let envelope = RiseGuidesResourceRepositoryEnvelope {
            schema_version: Self::CURRENT_SCHEMA_VERSION,
            guides,
            receipts,
            auto_start: Some(auto_start),
        };
        RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<Vec<u8>> {
                let mut value = serde_json::to_value(&envelope)?;
                value.sort_all_objects();
                Ok(serde_json::to_vec(&value)?)
            })
            .await?
    }

    async fn decode(data: Arc<Vec<u8>>) -> anyhow::Result<RiseGuidesResourceRepositoryEnvelope> {
        RiseAccountRuntime::current()
            .spawn_blocking(move || -> anyhow::Result<RiseGuidesResourceRepositoryEnvelope> {
                let envelope: RiseGuidesResourceRepositoryEnvelope = serde_json::from_slice(&data)?;
                if envelope.schema_version != Self::CURRENT_SCHEMA_VERSION {
                    return Ok(RiseGuidesResourceRepositoryEnvelope {
                        schema_version: Self::CURRENT_SCHEMA_VERSION,
                        guides: Vec::new(),
                        receipts: Vec::new(),
                        auto_start: None,
                    });
                }
                Ok(envelope)
            })
            .await?
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
