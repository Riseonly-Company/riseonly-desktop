use std::collections::{HashMap, HashSet, VecDeque};
use std::error::Error;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::LazyLock;
use std::thread::ThreadId;

use futures::FutureExt;
use futures::channel::oneshot;
use futures::future::LocalBoxFuture;
use gpui::{App, AsyncApp, Context, Entity, Global, WeakEntity};
use parking_lot::{Mutex, RwLock};
use rise_canon::rise_wire_methods::{post, user};
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_platform::AppPaths;
use rise_platform::file_manager::FileManager;
use rise_platform::url_resource_values::{URLResourceValues, set_resource_values};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::core::desktop_platform::rise_app_quit_drain::{QuitDrain, QuitLease};
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SaiWebSocketManager;
use crate::core::lib::sai::modules::sai_debugger::sai_ws_deferred_debug_store::{
    SaiWsDeferredDebugDiskRow, SaiWsDeferredDebugFifoRow, SaiWsDeferredDebugOutcome,
    SaiWsDeferredDebugStore,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws::{SaiWsOnError, SaiWsOnSuccess};
use crate::core::lib::sai::modules::sai_ws::sai_ws_coalescing::{
    SaiWsCoalescingOptions, SaiWsCoalescingRegistry, SaiWsCoalescingStrategy,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_envelope::SaiWsEnvelopeBuilder;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_coordinator::{
    SaiWsOptimisticCoordinator, SaiWsPreparedOptimisticSend,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::{
    SaiWsCachePreferredLayer, SaiWsDiskCachePolicy, SaiWsPendingCacheBinding, SaiWsRequestOptions,
    SaiWsRequestOptionsInit, SaiWsSendError,
};
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::core::lib::sai::sai::Sai;

/// error as a deferred send hands it on
/// one failure resumes several waiters and rolls the optimistic session back
/// so its shared and not moved
pub type SaiWsFailure = Rc<anyhow::Error>;

fn failure(error: impl Error + Send + Sync + 'static) -> SaiWsFailure {
    Rc::new(anyhow::Error::new(error))
}

/// the waiter owns its error and one still shared is rebuilt
/// stays downcastable for the manager retry decisions
fn owned_error(error: SaiWsFailure) -> anyhow::Error {
    Rc::try_unwrap(error).unwrap_or_else(|shared| match shared.downcast_ref::<SaiWsSendError>() {
        Some(send_error) => anyhow::Error::new(send_error.clone()),
        None => anyhow::anyhow!("{shared:#}"),
    })
}

fn std_error(error: &anyhow::Error) -> &(dyn Error + 'static) {
    <anyhow::Error as AsRef<dyn Error>>::as_ref(error)
}

/// true in debug or debug tools builds, the desktop has no debug tools build flag
fn debug_tools() -> bool {
    cfg!(debug_assertions)
}

#[derive(Default)]
struct SaiWsDeferredOutboundAppHooksSlot {
    on_cold_persisted_replay_finished: Option<Rc<dyn Fn(&mut App)>>,
}

impl Global for SaiWsDeferredOutboundAppHooksSlot {}

/// deferred outbound app hooks, the static closure lives in a main thread global
pub enum SaiWsDeferredOutboundAppHooks {}

impl SaiWsDeferredOutboundAppHooks {
    pub fn set_on_cold_persisted_replay_finished(
        hook: Option<Rc<dyn Fn(&mut App)>>,
        cx: &mut App,
    ) {
        cx.default_global::<SaiWsDeferredOutboundAppHooksSlot>()
            .on_cold_persisted_replay_finished = hook;
    }

    fn on_cold_persisted_replay_finished(cx: &mut App) -> Option<Rc<dyn Fn(&mut App)>> {
        cx.try_global::<SaiWsDeferredOutboundAppHooksSlot>()
            .and_then(|slot| slot.on_cold_persisted_replay_finished.clone())
    }
}

const DEFERRED_DIRECTORY_NAME: &str = "SaiWsDeferred";
const PROTECTED_FILENAMES: [&str; 2] = [OUTBOUND_PERSIST_FILENAME, ACK_DEBT_PERSIST_FILENAME];
pub(crate) const ACK_DEBT_PERSIST_FILENAME: &str = "ack_debt_v1.json";
const OUTBOUND_PERSIST_FILENAME: &str = "outbound_queue_v1.json";

const VOLATILE_USER_METHODS: [&str; 15] = [
    user::GET_CONTACT_SYNC_SETTINGS,
    user::UPDATE_CONTACT_SYNC_SETTINGS,
    user::SYNC_CONTACTS,
    user::GET_SAVED_CONTACT,
    user::LIST_SAVED_CONTACTS,
    user::UPSERT_SAVED_CONTACT,
    user::DELETE_SAVED_CONTACT,
    user::SET_SAVED_CONTACT_PHOTO,
    user::SUGGEST_CONTACT_PHOTO,
    user::LIST_CONTACT_PHOTO_SUGGESTIONS,
    user::RESOLVE_CONTACT_PHOTO_SUGGESTION,
    user::BLOCK_USER,
    user::UNBLOCK_USER,
    user::LIST_CONTACT_JOIN_EVENTS,
    user::MARK_CONTACT_JOIN_EVENTS_READ,
];

static DEFERRED_BASE_DIRECTORY: RwLock<Option<PathBuf>> = RwLock::new(None);

/// shared at rest policy for the retry and ack debt queue
/// requests marked exclude_from_debugger never enter this directory
pub enum SaiWsDeferredDiskProtection {}

impl SaiWsDeferredDiskProtection {
    pub fn install(paths: &AppPaths) {
        Self::install_in(paths.data_dir());
    }

    pub fn install_in(directory: &Path) {
        *DEFERRED_BASE_DIRECTORY.write() = Some(directory.to_path_buf());
    }

    /// before install the queue falls back to the temp directory
    pub fn directory() -> PathBuf {
        let base = DEFERRED_BASE_DIRECTORY
            .read()
            .clone()
            .unwrap_or_else(std::env::temp_dir);
        let directory = base.join(DEFERRED_DIRECTORY_NAME);
        let _ = FileManager.create_directory(&directory, true);
        let _ = set_resource_values(&directory, &Self::backup_exclusion());
        for filename in PROTECTED_FILENAMES {
            let file_url = directory.join(filename);
            if FileManager.file_exists(&file_url) {
                Self::protect_file(&file_url);
            }
        }
        directory
    }

    pub fn protect_file(url: &Path) {
        let _ = set_resource_values(url, &Self::backup_exclusion());
    }

    fn backup_exclusion() -> URLResourceValues {
        URLResourceValues {
            is_excluded_from_backup: Some(true),
        }
    }

    pub fn protected_filenames() -> [&'static str; 2] {
        PROTECTED_FILENAMES
    }

    pub fn must_remain_memory_only(service: &str, method: &str) -> bool {
        service == user::SERVICE && VOLATILE_USER_METHODS.contains(&method)
    }
}

type DiskJob = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct DiskLane {
    jobs: VecDeque<DiskJob>,
    draining: bool,
}

static DISK_LANES: LazyLock<Mutex<HashMap<ThreadId, DiskLane>>> = LazyLock::new(Mutex::default);
static DISK_IO: Mutex<()> = Mutex::new(());

/// one serial lane for the queue and ack debt files
/// json and file io run off the main thread and writes land in call order
pub enum SaiWsDeferredDisk {}

impl SaiWsDeferredDisk {
    fn url(filename: &str) -> PathBuf {
        SaiWsDeferredDiskProtection::directory().join(filename)
    }

    fn submit(job: DiskJob, cx: &App) {
        let owner = std::thread::current().id();
        let start_drainer = {
            let mut lanes = DISK_LANES.lock();
            let lane = lanes.entry(owner).or_default();
            lane.jobs.push_back(job);
            !std::mem::replace(&mut lane.draining, true)
        };
        if start_drainer {
            cx.background_executor()
                .spawn(async move { Self::run_lane(owner) })
                .detach();
        }
    }

    fn run_lane(owner: ThreadId) {
        loop {
            let _io = DISK_IO.lock();
            let job = {
                let mut lanes = DISK_LANES.lock();
                let lane = lanes.entry(owner).or_default();
                match lane.jobs.pop_front() {
                    Some(job) => job,
                    None => {
                        lane.draining = false;
                        return;
                    }
                }
            };
            job();
        }
    }

    pub fn write<Row: Serialize + Send + 'static>(
        rows: Vec<Row>,
        filename: &'static str,
        cx: &App,
    ) {
        Self::submit(
            Box::new(move || {
                let Ok(data) = serde_json::to_vec(&rows) else { return };
                let url = Self::url(filename);
                let _ = Self::write_atomically(&url, &data);
                SaiWsDeferredDiskProtection::protect_file(&url);
            }),
            cx,
        );
    }

    fn write_atomically(url: &Path, data: &[u8]) -> std::io::Result<()> {
        let mut staging = url.as_os_str().to_owned();
        staging.push(".tmp");
        let staging = PathBuf::from(staging);
        std::fs::write(&staging, data)?;
        std::fs::rename(&staging, url)
    }

    /// queued at call time so the read sees the file before any later write
    /// rows reach deliver on the main thread
    pub fn load<Row: DeserializeOwned + Send + 'static>(
        filename: &'static str,
        deliver: impl FnOnce(Vec<Row>, &mut App) + 'static,
        cx: &mut App,
    ) {
        let (sender, receiver) = oneshot::channel::<Vec<Row>>();
        Self::submit(
            Box::new(move || {
                let _ = sender.send(Self::read_now(filename));
            }),
            cx,
        );
        cx.spawn(async move |cx| {
            let rows = receiver.await.unwrap_or_default();
            cx.update(|cx| deliver(rows, cx));
        })
        .detach();
    }

    /// runs the queued writes then reads, tests only
    pub fn drain_and_read<Row: DeserializeOwned>(filename: &str) -> Vec<Row> {
        let owner = std::thread::current().id();
        loop {
            let _io = DISK_IO.lock();
            let job = DISK_LANES.lock().get_mut(&owner).and_then(|lane| lane.jobs.pop_front());
            match job {
                Some(job) => job(),
                None => return Self::read_now(filename),
            }
        }
    }

    fn read_now<Row: DeserializeOwned>(filename: &str) -> Vec<Row> {
        let Ok(data) = std::fs::read(Self::url(filename)) else { return Vec::new() };
        serde_json::from_slice::<Vec<Row>>(&data).unwrap_or_default()
    }
}

/// the desktop own disk format, keyed like the ios json
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaiWsDeferredPersistedRecord {
    pub id: String,
    #[serde(default)]
    pub account_user_id: Option<String>,
    #[serde(default)]
    pub account_epoch: Option<String>,
    pub service: String,
    pub method: String,
    #[serde(rename = "bodyJSON")]
    pub body_json: String,
    #[serde(default)]
    pub cache_key: Option<String>,
    pub reload_if_cached: bool,
    pub allow_concurrent_while_pending: bool,
    #[serde(rename = "reflectPendingInUI")]
    pub reflect_pending_in_ui: bool,
    pub send_immediately: bool,
    pub store_response_in_cache: bool,
    pub persist_cache_to_disk: bool,
    #[serde(default)]
    pub disk_cache_policy: Option<String>,
    pub reload_when_only_disk_cached: bool,
    pub preferred_cache_layer: String,
    #[serde(default)]
    pub path_to_array: Option<String>,
    #[serde(default)]
    pub path_to_id: Option<String>,
    #[serde(default)]
    pub coalescing_key: Option<String>,
    #[serde(default)]
    pub coalescing_strategy: Option<String>,
    #[serde(default)]
    pub deep_compare: Option<bool>,
    #[serde(default)]
    pub deep_compare_body_key: Option<String>,
    #[serde(default)]
    pub track_until_acknowledged: Option<bool>,
    #[serde(default)]
    pub offline_ack_timeout_seconds: Option<TimeInterval>,
}

impl SaiWsDeferredPersistedRecord {
    pub fn make(
        id: Uuid,
        data_value: &Value,
        options: &SaiWsRequestOptions,
    ) -> Result<Self, SaiWsSendError> {
        if !(data_value.is_object() || data_value.is_array()) {
            return Err(SaiWsSendError::EncodingFailed(
                "Deferred WS body is not JSON-serializable.".to_owned(),
            ));
        }
        let body_json = serde_json::to_string(data_value).map_err(|_| {
            SaiWsSendError::EncodingFailed("Deferred WS body UTF-8 encode failed.".to_owned())
        })?;
        Ok(Self {
            id: id.uuid_string(),
            account_user_id: options.account_user_id.clone(),
            account_epoch: options.account_epoch.clone(),
            service: options.service.clone(),
            method: options.method.clone(),
            body_json,
            cache_key: options.cache_key.clone(),
            reload_if_cached: options.reload_if_cached,
            allow_concurrent_while_pending: options.allow_concurrent_while_pending,
            reflect_pending_in_ui: options.reflect_pending_in_ui,
            send_immediately: options.send_immediately,
            store_response_in_cache: options.store_response_in_cache,
            persist_cache_to_disk: options.persist_cache_to_disk(),
            disk_cache_policy: Some(options.disk_cache_policy.raw_value().to_owned()),
            reload_when_only_disk_cached: options.reload_when_only_disk_cached,
            preferred_cache_layer: options.preferred_cache_layer.raw_value().to_owned(),
            path_to_array: options.path_to_array.clone(),
            path_to_id: options.path_to_id.clone(),
            coalescing_key: options.coalescing.as_ref().map(|coalescing| coalescing.key.clone()),
            coalescing_strategy: options
                .coalescing
                .as_ref()
                .map(|coalescing| coalescing.strategy.raw_value().to_owned()),
            deep_compare: Some(options.deep_compare),
            deep_compare_body_key: options.deep_compare_body_key.clone(),
            track_until_acknowledged: Some(options.track_until_acknowledged),
            offline_ack_timeout_seconds: options.offline_ack_timeout_seconds,
        })
    }

    pub fn decoded_body(&self) -> Option<Map<String, Value>> {
        match serde_json::from_str::<Value>(&self.body_json).ok()? {
            Value::Object(map) => Some(map),
            _ => None,
        }
    }

    pub fn to_request_options(&self) -> SaiWsRequestOptions {
        let layer = SaiWsCachePreferredLayer::from_raw(&self.preferred_cache_layer)
            .unwrap_or(SaiWsCachePreferredLayer::MemoryFirst);
        let disk_policy = self
            .disk_cache_policy
            .as_deref()
            .and_then(SaiWsDiskCachePolicy::from_raw);
        let coalescing = self
            .coalescing_key
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|key| !key.is_empty())
            .map(|key| {
                let strategy = self
                    .coalescing_strategy
                    .as_deref()
                    .and_then(SaiWsCoalescingStrategy::from_raw)
                    .unwrap_or(SaiWsCoalescingStrategy::ToggleParity);
                SaiWsCoalescingOptions { key: key.to_owned(), strategy }
            });
        let mut options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: self.service.clone(),
            method: self.method.clone(),
            cache_key: self.cache_key.clone(),
            reload_if_cached: self.reload_if_cached,
            allow_concurrent_while_pending: self.allow_concurrent_while_pending,
            reflect_pending_in_ui: self.reflect_pending_in_ui,
            send_immediately: self.send_immediately,
            store_response_in_cache: self.store_response_in_cache,
            persist_cache_to_disk: self.persist_cache_to_disk,
            disk_cache_policy: disk_policy,
            reload_when_only_disk_cached: self.reload_when_only_disk_cached,
            preferred_cache_layer: layer,
            path_to_array: self.path_to_array.clone(),
            path_to_id: self.path_to_id.clone(),
            optimistic_update: None,
            sai_slice: None,
            track_until_acknowledged: self.track_until_acknowledged.unwrap_or(false),
            offline_ack_timeout_seconds: self.offline_ack_timeout_seconds,
            coalescing,
            deep_compare: self.deep_compare.unwrap_or(false),
            deep_compare_body_key: self.deep_compare_body_key.clone(),
            ..SaiWsRequestOptionsInit::default()
        });
        options.account_user_id = self.account_user_id.clone();
        options.account_epoch = self.account_epoch.clone();
        options
    }
}

struct FifoEntry {
    id: Uuid,
    enqueued_at: Date,
    data_value: Value,
    options: SaiWsRequestOptions,
    on_success: Option<SaiWsOnSuccess>,
    on_error: Option<SaiWsOnError>,
    wire_resume: Option<oneshot::Sender<Result<u64, SaiWsFailure>>>,
    prepared_optimistic_send: Option<SaiWsPreparedOptimisticSend>,
    single_handshake: bool,
    _quit_lease: Option<QuitLease>,
}

impl FifoEntry {
    fn resume(&mut self, result: Result<u64, SaiWsFailure>) {
        if let Some(resume) = self.wire_resume.take() {
            let _ = resume.send(result);
        }
    }
}

/// what one flush step sends while the entry itself stays queued
struct FifoHead {
    id: Uuid,
    enqueued_at: Date,
    data_value: Value,
    options: SaiWsRequestOptions,
    on_success: Option<SaiWsOnSuccess>,
    on_error: Option<SaiWsOnError>,
    prepared_optimistic_send: Option<SaiWsPreparedOptimisticSend>,
}

/// deferred outbound coordinator, the main actor singleton is an entity here
pub struct SaiWsDeferredOutboundCoordinator {
    fifo: Vec<FifoEntry>,
    disk_records: Vec<SaiWsDeferredPersistedRecord>,
    flush_generation: u64,
    disk_loaded: bool,
    disk_epoch: u64,
    disk_load_waiters: Vec<oneshot::Sender<()>>,
}

impl SaiWsDeferredOutboundCoordinator {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(cx: &mut Context<Self>) -> Self {
        let this = Self {
            fifo: Vec::new(),
            disk_records: Vec::new(),
            flush_generation: 0,
            disk_loaded: false,
            disk_epoch: 0,
            disk_load_waiters: Vec::new(),
        };
        let epoch = this.disk_epoch;
        let weak = cx.weak_entity();
        SaiWsDeferredDisk::load::<SaiWsDeferredPersistedRecord>(
            OUTBOUND_PERSIST_FILENAME,
            move |rows, cx| {
                let _ = weak.update(cx, |this, cx| this.merge_loaded_disk(rows, epoch, cx));
            },
            cx,
        );
        this
    }

    pub fn reset(&mut self) {
        self.fifo.clear();
        self.disk_records.clear();
        self.flush_generation = 0;
        self.disk_epoch = self.disk_epoch.wrapping_add(1);
    }

    pub fn purge_for_account_transition(&mut self, cx: &mut Context<Self>) {
        self.flush_generation = self.flush_generation.wrapping_add(1);
        self.disk_epoch = self.disk_epoch.wrapping_add(1);
        self.fail_all_pending(anyhow::Error::new(SaiWsSendError::NotConnected), cx);
        self.disk_records.clear();
        Self::write_disk(Vec::new(), cx);
    }

    pub fn has_pending_outbound(&self) -> bool {
        !self.fifo.is_empty()
    }

    pub fn test_fifo_count(&self) -> usize {
        self.fifo.len()
    }

    pub fn test_disk_record_count(&self) -> usize {
        self.disk_records.len()
    }

    pub fn test_persisted_disk_record_count(&self) -> usize {
        SaiWsDeferredDisk::drain_and_read::<SaiWsDeferredPersistedRecord>(OUTBOUND_PERSIST_FILENAME)
            .len()
    }

    pub fn test_fifo_snapshot(&self) -> Vec<String> {
        self.fifo
            .iter()
            .map(|entry| format!("{}.{}", entry.options.service, entry.options.method))
            .collect()
    }

    pub fn prepared_optimistic_wire_ids(&self) -> HashSet<u64> {
        self.fifo
            .iter()
            .filter_map(|entry| entry.prepared_optimistic_send.as_ref())
            .map(|prepared| prepared.wire_request_id)
            .collect()
    }

    pub fn test_enqueue_without_continuation(&mut self, service: &str, method: &str) {
        let id = Uuid::new_v4();
        let options = SaiWsRequestOptions::new(SaiWsRequestOptionsInit {
            service: service.to_owned(),
            method: method.to_owned(),
            ..SaiWsRequestOptionsInit::default()
        });
        let body = Value::Object(Map::new());
        if let Ok(persisted) = SaiWsDeferredPersistedRecord::make(id, &body, &options) {
            self.disk_records.push(persisted);
        }
        self.fifo.push(FifoEntry {
            id,
            enqueued_at: RiseClock::now(),
            data_value: body,
            options,
            on_success: None,
            on_error: None,
            wire_resume: None,
            prepared_optimistic_send: None,
            single_handshake: false,
            _quit_lease: None,
        });
    }

    pub fn test_enqueue_prepared_optimistic_without_continuation(
        &mut self,
        data_value: Value,
        options: SaiWsRequestOptions,
        cx: &mut Context<Self>,
    ) -> Result<Option<u64>, SaiWsFailure> {
        let prepared = Self::prepare_optimistic_send_if_needed(&data_value, &options, cx)?;
        let wire_request_id = prepared.as_ref().map(|prepared| prepared.wire_request_id);
        self.fifo.push(FifoEntry {
            id: Uuid::new_v4(),
            enqueued_at: RiseClock::now(),
            data_value,
            options,
            on_success: None,
            on_error: None,
            wire_resume: None,
            prepared_optimistic_send: prepared,
            single_handshake: false,
            _quit_lease: None,
        });
        Ok(wire_request_id)
    }

    pub fn test_persist_records_to_disk(&self, cx: &App) {
        Self::write_disk(self.disk_records.clone(), cx);
    }

    pub fn test_drain_fifo_in_order(&mut self, cx: &mut Context<Self>) -> Vec<String> {
        let mut drained = Vec::new();
        while !self.fifo.is_empty() {
            let head = self.fifo.remove(0);
            drained.push(format!("{}.{}", head.options.service, head.options.method));
            self.remove_disk_record(head.id, cx);
        }
        drained
    }

    /// the sync part runs now
    /// the future resolves with the wire id once a connection sent it or with the error that settled it
    pub fn enqueue_await_connected_send(
        &mut self,
        manager: Entity<SaiWebSocketManager>,
        data_value: Value,
        options: SaiWsRequestOptions,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        single_handshake: bool,
        cx: &mut Context<Self>,
    ) -> LocalBoxFuture<'static, anyhow::Result<u64>> {
        if !options.allows_deferred_send {
            return Self::settled(Err(failure(SaiWsSendError::NotConnected)));
        }
        let prepared = match Self::prepare_optimistic_send_if_needed(&data_value, &options, cx) {
            Ok(prepared) => prepared,
            Err(error) => return Self::settled(Err(error)),
        };
        let queued = self.enqueue(
            &manager,
            data_value,
            options,
            on_success,
            on_error,
            prepared,
            single_handshake,
            cx,
        );
        Self::awaiting(queued)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn enqueue_prepared_await_connected_send(
        &mut self,
        manager: Entity<SaiWebSocketManager>,
        data_value: Value,
        options: SaiWsRequestOptions,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        prepared_optimistic_send: SaiWsPreparedOptimisticSend,
        cx: &mut Context<Self>,
    ) -> LocalBoxFuture<'static, anyhow::Result<u64>> {
        let queued = self.enqueue(
            &manager,
            data_value,
            options,
            on_success,
            on_error,
            Some(prepared_optimistic_send),
            false,
            cx,
        );
        Self::awaiting(queued)
    }

    fn settled(result: Result<u64, SaiWsFailure>) -> LocalBoxFuture<'static, anyhow::Result<u64>> {
        let result = result.map_err(owned_error);
        async move { result }.boxed_local()
    }

    fn awaiting(
        queued: Result<oneshot::Receiver<Result<u64, SaiWsFailure>>, SaiWsFailure>,
    ) -> LocalBoxFuture<'static, anyhow::Result<u64>> {
        async move {
            let receiver = queued.map_err(owned_error)?;
            receiver
                .await
                .unwrap_or_else(|_| Err(failure(SaiWsSendError::NotConnected)))
                .map_err(owned_error)
        }
        .boxed_local()
    }

    #[allow(clippy::too_many_arguments)]
    fn enqueue(
        &mut self,
        manager: &Entity<SaiWebSocketManager>,
        data_value: Value,
        options: SaiWsRequestOptions,
        on_success: Option<SaiWsOnSuccess>,
        on_error: Option<SaiWsOnError>,
        prepared_optimistic_send: Option<SaiWsPreparedOptimisticSend>,
        single_handshake: bool,
        cx: &mut Context<Self>,
    ) -> Result<oneshot::Receiver<Result<u64, SaiWsFailure>>, SaiWsFailure> {
        if !options.allows_deferred_send {
            return Err(failure(SaiWsSendError::NotConnected));
        }
        let id = Uuid::new_v4();
        if !options.exclude_from_debugger {
            let persisted =
                SaiWsDeferredPersistedRecord::make(id, &data_value, &options).map_err(failure)?;
            self.disk_records.push(persisted);
            self.persist(cx);
        }
        if prepared_optimistic_send.is_none()
            && options.reflect_pending_in_ui
            && let Some(slice) = options.sai_slice.clone()
        {
            slice.mark_pending();
        }
        let (resume, receiver) = oneshot::channel();
        let label = format!("{}.{}", options.service, options.method);
        let quit_lease = (!single_handshake).then(|| QuitDrain::lease("sai_ws.deferred_outbound", cx));
        self.fifo.push(FifoEntry {
            id,
            enqueued_at: RiseClock::now(),
            data_value,
            options,
            on_success,
            on_error,
            wire_resume: Some(resume),
            prepared_optimistic_send,
            single_handshake,
            _quit_lease: quit_lease,
        });
        Sai::log(
            "WS",
            move || format!("deferred outbound queued id={id} {label} (handshake in progress)"),
            SaiLogKind::Websocket,
            cx,
        );
        if debug_tools() {
            self.debug_notify_state(cx);
        }
        if manager.read(cx).is_connected() {
            self.schedule_flush(manager, "already connected race", cx);
        }
        Ok(receiver)
    }

    pub fn notify_connected(&mut self, manager: Entity<SaiWebSocketManager>, cx: &mut Context<Self>) {
        self.schedule_flush(&manager, "connected", cx);
    }

    fn schedule_flush(
        &mut self,
        manager: &Entity<SaiWebSocketManager>,
        reason: &'static str,
        cx: &mut Context<Self>,
    ) {
        self.flush_generation = self.flush_generation.wrapping_add(1);
        let generation = self.flush_generation;
        let manager = manager.downgrade();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Some(manager) = manager.upgrade() else { return };
            let current = this.update(cx, |this, cx| {
                if this.flush_generation != generation {
                    return false;
                }
                let (fifo, disk) = (this.fifo.len(), this.disk_records.len());
                Sai::log(
                    "WS",
                    move || format!("flush deferred outbound ({reason}) count={fifo} disk={disk}"),
                    SaiLogKind::Websocket,
                    cx,
                );
                true
            });
            if !matches!(current, Ok(true)) {
                return;
            }
            Self::flush_fifo(&this, &manager, cx).await;
            Self::flush_cold_disk_backlog(&this, &manager, cx).await;
            if debug_tools() {
                let _ = this.update(cx, |this, cx| this.debug_notify_state(cx));
            }
        })
        .detach();
    }

    async fn flush_fifo(
        this: &WeakEntity<Self>,
        manager: &Entity<SaiWebSocketManager>,
        cx: &mut AsyncApp,
    ) {
        loop {
            let head = this.update(cx, |this, cx| {
                if this.fifo.is_empty() || !manager.read(cx).is_connected() {
                    return None;
                }
                let head = &this.fifo[0];
                Some(FifoHead {
                    id: head.id,
                    enqueued_at: head.enqueued_at,
                    data_value: head.data_value.clone(),
                    options: head.options.clone(),
                    on_success: head.on_success.clone(),
                    on_error: head.on_error.clone(),
                    prepared_optimistic_send: head.prepared_optimistic_send.clone(),
                })
            });
            let Ok(Some(head)) = head else { return };
            let preserve_prepared = head.prepared_optimistic_send.is_some();
            let result = SaiWebSocketManager::send_service_call_payload_when_connected(
                manager.clone(),
                head.data_value.clone(),
                head.options.clone(),
                head.on_success.clone(),
                head.on_error.clone(),
                head.prepared_optimistic_send.clone(),
                preserve_prepared,
                cx.clone(),
            )
            .await
            .map_err(Rc::new);
            let proceed = this.update(cx, |this, cx| this.finish_fifo_head(head, result, cx));
            if !matches!(proceed, Ok(true)) {
                return;
            }
        }
    }

    fn finish_fifo_head(
        &mut self,
        head: FifoHead,
        result: Result<u64, SaiWsFailure>,
        cx: &mut Context<Self>,
    ) -> bool {
        let outcome = match &result {
            Ok(_) => SaiWsDeferredDebugOutcome::Sent,
            Err(error) => {
                if Self::is_retryable_deferred_error(error) {
                    let (id, message) = (head.id, error.to_string());
                    Sai::log(
                        "WS",
                        move || {
                            format!("deferred outbound retained id={id} after transient failure: {message}")
                        },
                        SaiLogKind::Websocket,
                        cx,
                    );
                    return false;
                }
                Self::rollback_prepared_optimistic(
                    head.prepared_optimistic_send.as_ref(),
                    &head.options,
                    error,
                    cx,
                );
                SaiWsDeferredDebugOutcome::SendFailed(error.to_string())
            }
        };
        if debug_tools() && !head.options.exclude_from_debugger {
            Self::debug_record_resolved(
                head.id,
                &head.options,
                Self::debug_body_json(&head.data_value),
                Some(head.enqueued_at),
                outcome,
                cx,
            );
        }
        let Some(live_index) = self.fifo.iter().position(|entry| entry.id == head.id) else {
            return true;
        };
        let mut entry = self.fifo.remove(live_index);
        self.remove_disk_record(head.id, cx);
        entry.resume(result);
        if debug_tools() {
            self.debug_notify_state(cx);
        }
        true
    }

    async fn flush_cold_disk_backlog(
        this: &WeakEntity<Self>,
        manager: &Entity<SaiWebSocketManager>,
        cx: &mut AsyncApp,
    ) {
        let Ok(waiter) = this.update(cx, |this, _| this.await_disk_load()) else { return };
        if let Some(waiter) = waiter {
            let _ = waiter.await;
        }
        let split = this.update(cx, |this, _| {
            if this.disk_records.is_empty() {
                return None;
            }
            let live_fifo_ids: HashSet<String> =
                this.fifo.iter().map(|entry| entry.id.uuid_string()).collect();
            let (kept, batch): (Vec<_>, Vec<_>) = this
                .disk_records
                .iter()
                .cloned()
                .partition(|record| live_fifo_ids.contains(&record.id));
            Some((batch, kept))
        });
        let Ok(Some((batch, mut kept))) = split else { return };
        let mut replayed_post_get_feed = false;
        for record in batch {
            if !cx.update(|cx| manager.read(cx).is_connected()) {
                kept.push(record);
                continue;
            }
            let Some(body) = record.decoded_body() else {
                let id = record.id.clone();
                cx.update(|cx| {
                    Sai::log(
                        "WS",
                        move || format!("drop persisted deferred (bad JSON) id={id}"),
                        SaiLogKind::Websocket,
                        cx,
                    );
                });
                continue;
            };
            let options = record.to_request_options();
            let result = SaiWebSocketManager::send_service_call_payload_when_connected(
                manager.clone(),
                Value::Object(body),
                options,
                None,
                None,
                None,
                false,
                cx.clone(),
            )
            .await;
            match result {
                Ok(_) => {
                    if debug_tools() {
                        cx.update(|cx| {
                            Self::debug_record_cold_replay(
                                &record,
                                SaiWsDeferredDebugOutcome::ColdReplaySent,
                                cx,
                            );
                        });
                    }
                    if record.service == post::SERVICE && record.method == post::GET_FEED {
                        replayed_post_get_feed = true;
                    }
                }
                Err(error) => {
                    let message = error.to_string();
                    cx.update(|cx| {
                        if debug_tools() {
                            Self::debug_record_cold_replay(
                                &record,
                                SaiWsDeferredDebugOutcome::ColdReplayFailed(message.clone()),
                                cx,
                            );
                        }
                        let id = record.id.clone();
                        Sai::log(
                            "WS",
                            move || format!("cold replay deferred failed id={id}: {message}"),
                            SaiLogKind::Websocket,
                            cx,
                        );
                    });
                    kept.push(record);
                }
            }
        }
        let _ = this.update(cx, |this, cx| {
            this.disk_records = kept;
            this.persist(cx);
        });
        if replayed_post_get_feed {
            cx.update(|cx| {
                if let Some(hook) = SaiWsDeferredOutboundAppHooks::on_cold_persisted_replay_finished(cx) {
                    hook(cx);
                }
            });
        }
        if debug_tools() {
            let _ = this.update(cx, |this, cx| this.debug_notify_state(cx));
        }
    }

    fn remove_disk_record(&mut self, id: Uuid, cx: &App) {
        let id = id.uuid_string();
        let before = self.disk_records.len();
        self.disk_records.retain(|record| record.id != id);
        if self.disk_records.len() != before {
            self.persist(cx);
        }
    }

    pub fn settle_single_handshake_entries(
        &mut self,
        mut serve_cached: impl FnMut(&SaiWsRequestOptions, Option<&SaiWsOnSuccess>, &mut App) -> bool,
        cx: &mut Context<Self>,
    ) {
        let error = failure(SaiWsSendError::NotConnected);
        let (entries, remaining): (Vec<_>, Vec<_>) = std::mem::take(&mut self.fifo)
            .into_iter()
            .partition(|entry| entry.single_handshake);
        self.fifo = remaining;
        if entries.is_empty() {
            return;
        }
        let settled_disk_ids: HashSet<String> =
            entries.iter().map(|entry| entry.id.uuid_string()).collect();
        let disk_before = self.disk_records.len();
        self.disk_records
            .retain(|record| !settled_disk_ids.contains(&record.id));
        if self.disk_records.len() != disk_before {
            self.persist(cx);
        }
        let settled_at = RiseClock::now();
        let count = entries.len();
        for mut entry in entries {
            if let Some(coalescing) = &entry.options.coalescing {
                SaiWsCoalescingRegistry::discard(coalescing, cx);
            }
            let served_from_cache = serve_cached(&entry.options, entry.on_success.as_ref(), &mut **cx);
            if debug_tools() && !entry.options.exclude_from_debugger {
                let reason = if served_from_cache {
                    "handshake failed, served from cache".to_owned()
                } else {
                    error.to_string()
                };
                Self::debug_record_resolved_at(
                    entry.id,
                    &entry.options,
                    Self::debug_body_json(&entry.data_value),
                    Some(entry.enqueued_at),
                    settled_at,
                    SaiWsDeferredDebugOutcome::Cancelled(reason),
                    cx,
                );
            }
            if served_from_cache {
                entry.resume(Ok(0));
                continue;
            }
            Self::rollback_prepared_optimistic(
                entry.prepared_optimistic_send.as_ref(),
                &entry.options,
                &error,
                cx,
            );
            if let Some(slice) = entry.options.sai_slice.clone() {
                slice.mark_rejected(&error);
            }
            entry.resume(Err(error.clone()));
        }
        Sai::log(
            "WS",
            move || format!("settled {count} single-handshake outbound after failed handshake"),
            SaiLogKind::Websocket,
            cx,
        );
        if debug_tools() {
            self.debug_notify_state(cx);
        }
    }

    pub fn fail_all_pending(&mut self, error: anyhow::Error, cx: &mut Context<Self>) {
        let error: SaiWsFailure = Rc::new(error);
        let entries = std::mem::take(&mut self.fifo);
        for entry in &entries {
            if let Some(coalescing) = &entry.options.coalescing {
                SaiWsCoalescingRegistry::discard(coalescing, cx);
            }
        }
        if debug_tools() {
            let message = error.to_string();
            let now = RiseClock::now();
            for entry in entries.iter().filter(|entry| !entry.options.exclude_from_debugger) {
                Self::debug_record_resolved_at(
                    entry.id,
                    &entry.options,
                    Self::debug_body_json(&entry.data_value),
                    Some(entry.enqueued_at),
                    now,
                    SaiWsDeferredDebugOutcome::Cancelled(message.clone()),
                    cx,
                );
            }
        }
        for mut entry in entries {
            Self::rollback_prepared_optimistic(
                entry.prepared_optimistic_send.as_ref(),
                &entry.options,
                &error,
                cx,
            );
            if let Some(slice) = entry.options.sai_slice.clone() {
                slice.mark_rejected(&error);
            }
            entry.resume(Err(error.clone()));
        }
        if !self.disk_records.is_empty() || !self.disk_loaded {
            self.disk_records.clear();
            self.disk_epoch = self.disk_epoch.wrapping_add(1);
            Self::write_disk(Vec::new(), cx);
        }
        if debug_tools() {
            self.debug_notify_state(cx);
        }
    }

    pub fn cancel_volatile_requests(
        &mut self,
        service: &str,
        method: &str,
        error: anyhow::Error,
        cx: &mut Context<Self>,
    ) {
        let error: SaiWsFailure = Rc::new(error);
        let (entries, remaining): (Vec<_>, Vec<_>) =
            std::mem::take(&mut self.fifo).into_iter().partition(|entry| {
                entry.options.exclude_from_debugger
                    && entry.options.service == service
                    && entry.options.method == method
            });
        self.fifo = remaining;
        if entries.is_empty() {
            return;
        }
        for mut entry in entries {
            Self::rollback_prepared_optimistic(
                entry.prepared_optimistic_send.as_ref(),
                &entry.options,
                &error,
                cx,
            );
            if let Some(slice) = entry.options.sai_slice.clone() {
                slice.mark_rejected(&error);
            }
            entry.resume(Err(error.clone()));
        }
        if debug_tools() {
            self.debug_notify_state(cx);
        }
    }

    fn merge_loaded_disk(
        &mut self,
        decoded: Vec<SaiWsDeferredPersistedRecord>,
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
                        &record.service,
                        &record.method,
                    )
                })
                .collect();
            let retained_count = retained.len();
            let known: HashSet<String> =
                self.disk_records.iter().map(|record| record.id.clone()).collect();
            let mut restored: Vec<_> = retained
                .into_iter()
                .filter(|record| !known.contains(&record.id))
                .collect();
            let restored_count = restored.len();
            restored.append(&mut self.disk_records);
            self.disk_records = restored;
            if retained_count != decoded_count || self.disk_records.len() != restored_count {
                self.persist(cx);
            }
        }
        for waiter in std::mem::take(&mut self.disk_load_waiters) {
            let _ = waiter.send(());
        }
        if debug_tools() {
            self.debug_notify_state(cx);
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
        Self::write_disk(self.disk_records.clone(), cx);
    }

    fn write_disk(records: Vec<SaiWsDeferredPersistedRecord>, cx: &App) {
        SaiWsDeferredDisk::write(records, OUTBOUND_PERSIST_FILENAME, cx);
    }

    pub fn prepare_optimistic_send_if_needed(
        data_value: &Value,
        options: &SaiWsRequestOptions,
        cx: &mut App,
    ) -> Result<Option<SaiWsPreparedOptimisticSend>, SaiWsFailure> {
        let Some(optimistic) = &options.optimistic_update else { return Ok(None) };
        if !optimistic.enabled {
            return Ok(None);
        }
        let wire_request_id = SaiWsEnvelopeBuilder::make_outbound_id();
        let outcome = SaiWsOptimisticCoordinator::apply_before_send_if_needed(
            wire_request_id,
            data_value,
            options,
            cx,
        )
        .map_err(failure)?;
        if !outcome.did_register_session {
            return Ok(None);
        }
        SaiWsResponseCache::note_active_slice(
            options.sai_slice.clone(),
            options.cache_key.as_deref(),
            options.take_path.as_deref(),
            cx,
        );
        if let Some(cache_key) = options.cache_key.as_deref()
            && let Some(root) = SaiWsResponseCache::current_payload(
                cache_key,
                options.preferred_cache_layer,
                options.disk_cache_policy,
                cx,
            )
            && let Some(slice) = options.sai_slice.clone()
        {
            let raw = if root.is_null() { None } else { Some(root) };
            slice.mark_fulfilled(raw.as_ref(), options.take_path.as_deref());
        }
        Ok(Some(SaiWsPreparedOptimisticSend { wire_request_id, outcome }))
    }

    fn rollback_prepared_optimistic(
        prepared: Option<&SaiWsPreparedOptimisticSend>,
        options: &SaiWsRequestOptions,
        error: &SaiWsFailure,
        cx: &mut App,
    ) {
        let Some(prepared) = prepared else { return };
        SaiWsOptimisticCoordinator::handle_failure(
            prepared.wire_request_id,
            std_error(error),
            options.sai_slice.clone(),
            Some(&SaiWsPendingCacheBinding::new(options)),
            cx,
        );
    }

    fn is_retryable_deferred_error(error: &SaiWsFailure) -> bool {
        if let Some(send_error) = error.downcast_ref::<SaiWsSendError>() {
            return match send_error {
                SaiWsSendError::NotConnected | SaiWsSendError::SendFailed(_) => true,
                SaiWsSendError::Maintenance
                | SaiWsSendError::EncodingFailed(_)
                | SaiWsSendError::GuideSandbox => false,
            };
        }
        error.downcast_ref::<std::io::Error>().is_some()
            || error
                .downcast_ref::<tokio_tungstenite::tungstenite::Error>()
                .is_some()
    }

    pub fn debug_body_json(data_value: &Value) -> String {
        if (data_value.is_object() || data_value.is_array())
            && let Ok(text) = serde_json::to_string_pretty(&Self::sorted_keys(data_value))
        {
            return text;
        }
        data_value.to_string()
    }

    /// copy of the value with object keys sorted, whatever map order it was built with
    fn sorted_keys(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<&String> = map.keys().collect();
                keys.sort();
                Value::Object(
                    keys.into_iter()
                        .map(|key| (key.clone(), Self::sorted_keys(&map[key])))
                        .collect(),
                )
            }
            Value::Array(items) => Value::Array(items.iter().map(Self::sorted_keys).collect()),
            other => other.clone(),
        }
    }

    fn debug_record_resolved(
        id: Uuid,
        options: &SaiWsRequestOptions,
        body_json: String,
        enqueued_at: Option<Date>,
        outcome: SaiWsDeferredDebugOutcome,
        cx: &mut App,
    ) {
        Self::debug_record_resolved_at(id, options, body_json, enqueued_at, RiseClock::now(), outcome, cx);
    }

    fn debug_record_resolved_at(
        id: Uuid,
        options: &SaiWsRequestOptions,
        body_json: String,
        enqueued_at: Option<Date>,
        processed_at: Date,
        outcome: SaiWsDeferredDebugOutcome,
        cx: &mut App,
    ) {
        let (service, method, cache_key) =
            (options.service.clone(), options.method.clone(), options.cache_key.clone());
        SaiWsDeferredDebugStore::shared(cx).update(cx, |store, cx| {
            store.record_resolved(
                id,
                service,
                method,
                body_json,
                cache_key,
                enqueued_at,
                processed_at,
                outcome,
                cx,
            );
        });
    }

    fn debug_record_cold_replay(
        record: &SaiWsDeferredPersistedRecord,
        outcome: SaiWsDeferredDebugOutcome,
        cx: &mut App,
    ) {
        let id = Uuid::from_uuid_string(&record.id).unwrap_or_else(Uuid::new_v4);
        let record = record.clone();
        SaiWsDeferredDebugStore::shared(cx).update(cx, |store, cx| {
            store.record_resolved(
                id,
                record.service,
                record.method,
                record.body_json,
                record.cache_key,
                None,
                RiseClock::now(),
                outcome,
                cx,
            );
        });
    }

    pub fn debug_notify_state(&self, cx: &mut App) {
        let fifo_oldest_first: Vec<SaiWsDeferredDebugFifoRow> = self
            .fifo
            .iter()
            .filter(|entry| !entry.options.exclude_from_debugger)
            .map(|entry| SaiWsDeferredDebugFifoRow {
                id: entry.id,
                enqueued_at: entry.enqueued_at,
                service: entry.options.service.clone(),
                method: entry.options.method.clone(),
                body_json: Self::debug_body_json(&entry.data_value),
                cache_key: entry.options.cache_key.clone(),
            })
            .collect();
        let fifo_ids: HashSet<String> = self.fifo.iter().map(|entry| entry.id.uuid_string()).collect();
        let disk_not_in_fifo: Vec<SaiWsDeferredDebugDiskRow> = self
            .disk_records
            .iter()
            .filter(|record| !fifo_ids.contains(&record.id))
            .map(|record| SaiWsDeferredDebugDiskRow {
                id: record.id.clone(),
                service: record.service.clone(),
                method: record.method.clone(),
                body_json: record.body_json.clone(),
                cache_key: record.cache_key.clone(),
            })
            .collect();
        SaiWsDeferredDebugStore::shared(cx).update(cx, |store, cx| {
            store.apply_snapshot(fifo_oldest_first, disk_not_in_fifo, cx);
        });
    }
}
