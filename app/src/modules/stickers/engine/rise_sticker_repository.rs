use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::FutureExt as _;
use futures::channel::oneshot;
use futures::future::{BoxFuture, Shared};
use gpui::{App, AsyncApp};
use rise_canon::rise_key;
use rise_canon::rise_upload::{self, ErrorCode as RiseUploadErrorCode};
use rise_canon::{rise_request, rise_response};
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_chunked_upload::{
    RiseChunkedUploadClient, RiseChunkedUploadProgressHandler, RiseChunkedUploadRequest, RiseChunkedUploadResult,
    RiseChunkedUploadStep, RiseChunkedUploadTransferProgress,
};
use rise_engine::{CancellationError, RiseEngine, RiseJSONValue};
use rise_errors::{ErrorCode as RiseErrorCode, ErrorParam as RiseErrorParam};
use rise_i18n::tr;
use rise_platform::file_manager::FileManager;
use serde::{Deserialize, Serialize};
use tokio::task::JoinHandle;

use super::rise_sticker_models::{
    RiseStickerLoadState, RiseStickerMutationID, RiseStickerMutationState, RiseStickerPackContent, RiseStickerPage,
    RiseStickerPerformancePolicy, RiseStickerPickerTab, RiseStickerPresentationSnapshot, RiseStickerRepositoryError,
    RiseStickerResourceID, RiseStickerUploadPresentation, StickerMediaUploadDraft,
};
use super::rise_sticker_presentation::RiseStickerPresentation;
use crate::core::lib::canon_errors::{CanonErrors, RiseFailure};
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::core::ui::lottie_sticker::lottie_sticker_loader::LottieStickerLoader;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::core::rise_conversation_debug_trace::RiseConversationDebugTrace;
use crate::modules::chat::engine::core::rise_conversation_resource_repository::{
    RiseConversationResourceKey, RiseConversationResourceReadSource, RiseConversationResourceRepository,
};
use crate::modules::chat::engine::core::rise_conversation_rpc::{RiseConversationEndpoints, RiseConversationRPC};
use crate::modules::chat::engine::core::rise_conversation_upload_support::RiseConversationUploadSupport;
use crate::modules::chat::engine::messages::rise_message_repository::RiseMessageRepository;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{
    GifItem, PackKind, StickerAssetType, StickerAssetTypeInfer, StickerItem, StickerPack,
};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_wire_types::{
    AddGifFavouriteGatewayDTO, GetCustomEmojiByIdsGatewayDTO, GetGifFavouritesGatewayDTO, GetMyPacksGatewayDTO,
    GetPacksGatewayDTO, GetSavedPacksGatewayDTO, GetStickersByPackIdGatewayDTO, GetStickersByPackLinkGatewayDTO,
    GifWireDTO, GifWireDTOExt, PackWireDTO, PackWireDTOExt, SearchGifsGatewayDTO, StickerWireDTOExt,
};

const MAXIMUM_MEMORY_ENTRIES: usize = 96;
const MAXIMUM_MEMORY_BYTES: usize = 24 * 1_024 * 1_024;
const PICKER_REFRESH_DELAY_MS: u64 = 180;
const MY_PACKS_NEXT_PAGE_ATTEMPTS: usize = 8;
const MY_PACKS_NEXT_PAGE_RETRY_MS: u64 = 60;
const LOCAL_PACK_PREFIX: &str = "local-pack-";
const LOCAL_STICKER_PREFIX: &str = "local-sticker-";
const COMPLETE_PROGRESS_FRACTION: f64 = 0.999;

#[derive(Clone, Serialize, Deserialize)]
struct RiseStickerPackManifest {
    ids: Vec<String>,
}

#[derive(Clone, Copy)]
struct RiseStickerResourceLease {
    generation: u64,
    forced_ticket: Option<u64>,
}

#[derive(Clone)]
struct RiseStickerPendingCreateAttempt {
    pack_id: String,
    draft: StickerMediaUploadDraft,
    emojis: Vec<String>,
    requested_asset_type: Option<String>,
    requested_codec: Option<String>,
    upload_result: RiseChunkedUploadResult,
    create_request: rise_request::stickers::CreateSticker,
    remote_thumbnail_url: Option<String>,
    completion_asset_type: Option<String>,
    canonical_asset_type: StickerAssetType,
    completion_duration_ms: Option<i64>,
    legacy_duration_ms: Option<i64>,
    completion_codec: Option<String>,
}

impl RiseStickerPendingCreateAttempt {
    fn matches(
        &self,
        pack_id: &str,
        draft: &StickerMediaUploadDraft,
        emojis: &[String],
        asset_type: Option<StickerAssetType>,
        codec: Option<&str>,
    ) -> bool {
        self.pack_id == pack_id
            && self.draft == *draft
            && self.emojis.iter().collect::<HashSet<_>>() == emojis.iter().collect::<HashSet<_>>()
            && self.requested_asset_type.as_deref() == asset_type.map(|value| value.wire())
            && self.requested_codec.as_deref() == codec
    }
}

struct RiseStickerMutationRollbackSnapshot {
    pack_id: String,
    kind: PackKind,
    pack_content: Option<RiseStickerPackContent>,
    linked_contents: HashMap<String, RiseStickerPackContent>,
    favourites: Vec<StickerItem>,
    recents: HashMap<PackKind, Vec<StickerItem>>,
    default_page: Option<RiseStickerPage<StickerPack>>,
    saved_page: Option<RiseStickerPage<StickerPack>>,
    my_page: Option<RiseStickerPage<StickerPack>>,
}

struct RiseStickerHydrationTask {
    handle: JoinHandle<()>,
    finished: Shared<oneshot::Receiver<()>>,
}

/// one step of begin_coalesced, the in flight check and what follows must stay one isolated section
enum RiseStickerCoalescedStep {
    Finished(Option<RiseStickerResourceLease>),
    Wait(oneshot::Receiver<()>, u64),
}

struct RiseStickerDefer<Action: FnOnce()> {
    action: Option<Action>,
}

impl<Action: FnOnce()> RiseStickerDefer<Action> {
    fn new(action: Action) -> Self {
        Self { action: Some(action) }
    }

    fn disarm(mut self) {
        self.action = None;
    }
}

impl<Action: FnOnce()> Drop for RiseStickerDefer<Action> {
    fn drop(&mut self) {
        if let Some(action) = self.action.take() {
            action();
        }
    }
}

fn own_pack_count(packs: &[StickerPack]) -> i64 {
    packs.iter().filter(|pack| pack.is_creator).count() as i64
}

fn trimmed(value: &str) -> String {
    trimming_whitespaces_and_newlines(value).to_owned()
}

fn now_ms() -> i64 {
    (Date::now().time_interval_since_1970() * 1_000.0).round() as i64
}

fn limit_count(limit: i64) -> usize {
    usize::try_from(limit).unwrap_or(0)
}

fn cancelled() -> anyhow::Error {
    anyhow::Error::new(CancellationError)
}

fn is_cancellation(error: &anyhow::Error) -> bool {
    error.downcast_ref::<CancellationError>().is_some()
}

fn rejected(detail: &str) -> anyhow::Error {
    anyhow::Error::new(RiseStickerRepositoryError::Rejected(detail.to_owned()))
}

fn page_items<Value: Clone + PartialEq>(page: Option<&RiseStickerPage<Value>>) -> &[Value] {
    match page {
        Some(page) => page.items.as_slice(),
        None => &[],
    }
}

fn file_context(kind: PackKind) -> String {
    let context = if kind == PackKind::Emoji { rise_key::events::CUSTOM_EMOJI } else { rise_key::stickers::STICKER };
    context.to_owned()
}

fn video_duration_seconds(duration_ms: Option<i64>) -> Option<i64> {
    duration_ms.map(|value| ((value + 999) / 1_000).max(1))
}

fn normalized_remote_url(raw: &str) -> String {
    RemoteMediaURL::optional_normalized(Some(raw)).unwrap_or_else(|| trimmed(raw))
}

struct RiseStickerRepositoryState {
    state: RiseStickerPresentationSnapshot,
    upload_client: Option<Arc<RiseChunkedUploadClient>>,
    account_epoch: Option<String>,
    lifecycle_generation: u64,
    in_flight: HashSet<RiseStickerResourceID>,
    generations: HashMap<RiseStickerResourceID, u64>,
    forced_reload_tickets: HashMap<RiseStickerResourceID, u64>,
    completed_forced_reload_tickets: HashMap<RiseStickerResourceID, u64>,
    resource_idle_waiters: HashMap<RiseStickerResourceID, Vec<oneshot::Sender<()>>>,
    mutation_generations: HashMap<RiseStickerMutationID, u64>,
    pending_sticker_create_attempts: HashMap<String, RiseStickerPendingCreateAttempt>,
    refreshed_at: HashMap<RiseStickerResourceID, Date>,
    persisted_pack_ids: Vec<String>,
    search_order: Vec<String>,
    last_gif_search_query: Option<String>,
    hydration_task: Option<RiseStickerHydrationTask>,
    picker_refresh_task: Option<JoinHandle<()>>,
    picker_generation: u64,
    picker_visible: bool,
    active_picker_tab: RiseStickerPickerTab,
    cached_presentation_tabs: HashSet<RiseStickerPickerTab>,
    cold_persistent_tabs: HashSet<RiseStickerPickerTab>,
    is_active: bool,
    is_shutdown: bool,
    custom_emoji_order: Vec<String>,
    custom_emoji_misses: Vec<String>,
    custom_emoji_in_flight: HashSet<String>,
}

/// sticker repository, every stretch between awaits is one state.with section here
pub struct RiseStickerRepository {
    pub account_id: String,
    this: Weak<RiseStickerRepository>,
    resources: RiseConversationResourceRepository,
    rpc: RiseConversationRPC,
    session_id: Uuid,
    state: RiseIsolated<RiseStickerRepositoryState>,
}

impl RiseStickerRepository {
    pub fn new(engine: Arc<RiseEngine>, account_id: String) -> Arc<Self> {
        Arc::new_cyclic(|this| Self {
            resources: RiseConversationResourceRepository::with_memory_limits(
                engine.clone(),
                MAXIMUM_MEMORY_ENTRIES,
                MAXIMUM_MEMORY_BYTES,
            ),
            rpc: RiseConversationRPC::new(engine),
            session_id: Uuid::new_v4(),
            state: RiseIsolated::new(RiseStickerRepositoryState::new(account_id.clone())),
            account_id,
            this: this.clone(),
        })
    }

    async fn on_main<R: Send + 'static>(work: impl FnOnce(&mut App) -> R + Send + 'static) -> Option<R> {
        let hop = RiseEngineMainHop::installed()?;
        hop.run(move |cx: AsyncApp| async move { Ok(cx.update(work)) }.boxed_local()).await.ok()
    }

    pub async fn activate(&self, account_epoch: &str) {
        let expected_generation = self.state.with(|s| {
            if s.is_shutdown || s.is_active {
                return None;
            }
            s.account_epoch = Some(account_epoch.to_owned());
            s.lifecycle_generation = s.lifecycle_generation.wrapping_add(1);
            s.is_active = true;
            Some(s.lifecycle_generation)
        });
        let Some(expected_generation) = expected_generation else { return };
        let additional_validation = self.upload_validation_step(account_epoch.to_owned(), expected_generation);
        let account_user_id = self.account_id.clone();
        let epoch = account_epoch.to_owned();
        let client = Self::on_main(move |cx| {
            RiseConversationUploadSupport::make_client(
                Some(account_user_id),
                Some(epoch),
                Some(additional_validation),
                cx,
            )
        })
        .await;
        self.state.with(|s| s.upload_client = client.map(Arc::new));
        let account_id = self.account_id.clone();
        Self::on_main(move |cx| {
            RiseStickerPresentation::shared(cx).update(cx, |presentation, cx| presentation.activate(&account_id, cx));
        })
        .await;
        let (finished_sender, finished) = oneshot::channel::<()>();
        let this = self.this.clone();
        let handle = RiseAccountRuntime::current().spawn(async move {
            let _finished_sender = finished_sender;
            if let Some(repository) = this.upgrade() {
                repository.hydrate_from_store().await;
            }
        });
        self.state.with(|s| {
            s.hydration_task = Some(RiseStickerHydrationTask { handle, finished: finished.shared() });
        });
    }

    fn upload_validation_step(&self, account_epoch: String, lifecycle_generation: u64) -> RiseChunkedUploadStep {
        let this = self.this.clone();
        let session_id = self.session_id;
        Arc::new(move || -> BoxFuture<'static, anyhow::Result<()>> {
            let repository = this.upgrade();
            let account_epoch = account_epoch.clone();
            async move {
                let Some(repository) = repository else { return Err(cancelled()) };
                repository.validate_upload_context(&account_epoch, session_id, lifecycle_generation)
            }
            .boxed()
        })
    }

    pub async fn shutdown(&self) {
        let proceeded = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            s.is_shutdown = true;
            s.is_active = false;
            s.lifecycle_generation = s.lifecycle_generation.wrapping_add(1);
            s.account_epoch = None;
            s.upload_client = None;
            s.picker_visible = false;
            s.picker_generation = s.picker_generation.wrapping_add(1);
            if let Some(task) = s.picker_refresh_task.take() {
                task.abort();
            }
            if let Some(task) = s.hydration_task.take() {
                task.handle.abort();
            }
            s.cached_presentation_tabs.clear();
            s.cold_persistent_tabs.clear();
            for generation in s.generations.values_mut() {
                *generation = generation.wrapping_add(1);
            }
            for generation in s.mutation_generations.values_mut() {
                *generation = generation.wrapping_add(1);
            }
            s.pending_sticker_create_attempts.clear();
            s.in_flight.clear();
            s.custom_emoji_order.clear();
            s.custom_emoji_misses.clear();
            s.custom_emoji_in_flight.clear();
            s.resume_all_resource_idle_waiters();
            true
        });
        if !proceeded {
            return;
        }
        self.resources.invalidate_requests().await;
        self.resources.clear_memory().await;
        let account_id = self.account_id.clone();
        Self::on_main(move |cx| {
            RiseStickerPresentation::shared(cx).update(cx, |presentation, cx| presentation.deactivate(&account_id, cx));
        })
        .await;
    }

    pub async fn picker_did_open(&self, tab: RiseStickerPickerTab) {
        let generation = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            s.picker_visible = true;
            s.active_picker_tab = tab;
            s.picker_generation = s.picker_generation.wrapping_add(1);
            Some(s.picker_generation)
        });
        let Some(generation) = generation else { return };
        self.picker_did_activate(tab, generation).await;
    }

    pub async fn picker_did_select(&self, tab: RiseStickerPickerTab) {
        let generation = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            s.active_picker_tab = tab;
            s.picker_generation = s.picker_generation.wrapping_add(1);
            Some(s.picker_generation)
        });
        let Some(generation) = generation else { return };
        self.picker_did_activate(tab, generation).await;
    }

    async fn picker_did_activate(&self, tab: RiseStickerPickerTab, generation: u64) {
        self.wait_for_hydration().await;
        if !self.state.with(|s| s.is_picker_current(tab, generation)) {
            return;
        }
        self.trace_picker_cache_hit(tab).await;
        self.schedule_picker_refresh(generation, false);
    }

    pub async fn picker_did_close(&self) {
        self.state.with(|s| {
            s.picker_visible = false;
            s.picker_generation = s.picker_generation.wrapping_add(1);
            if let Some(task) = s.picker_refresh_task.take() {
                task.abort();
            }
        });
    }

    pub async fn refresh_visible_picker_after_reconnect(&self) {
        let generation = self.state.with(|s| {
            if !s.picker_visible {
                return None;
            }
            s.picker_generation = s.picker_generation.wrapping_add(1);
            Some(s.picker_generation)
        });
        if let Some(generation) = generation {
            self.schedule_picker_refresh(generation, true);
        }
    }

    pub async fn load_default_packs(&self, kind: PackKind, relative_id: Option<String>, force: bool) {
        self.wait_for_hydration().await;
        let resource = RiseStickerResourceID::DefaultPacks(kind);
        let Some(lease) = self.begin_coalesced(&resource, force, relative_id.is_some()).await else { return };
        let generation = lease.generation;
        let _end = RiseStickerDefer::new(|| self.end(&resource, lease));
        self.set_loading(&resource).await;
        let result: anyhow::Result<()> = async {
            let response: GetPacksGatewayDTO = self
                .rpc
                .call(
                    RiseConversationEndpoints::get_default_sticker_packs(),
                    rise_request::stickers::GetDefaultPacks {
                        limit: Some(RiseStickerPerformancePolicy::packs_page_limit()),
                        pack_kind: Some(kind.wire().to_owned()),
                        page: None,
                        relative_id: relative_id.clone(),
                    },
                )
                .await?;
            let wire = self.state.with(|s| {
                if !s.accepts(&resource, generation) {
                    return None;
                }
                let (presentation, wire) = Self::merged_pack_page(
                    s.state.default_packs.get(&kind),
                    &response.packs,
                    relative_id.as_deref(),
                    response.relative_id.clone(),
                    Some(response.has_more),
                );
                s.state.default_packs.insert(kind, presentation);
                s.cached_presentation_tabs.insert(RiseStickerRepositoryState::picker_tab(kind));
                s.mark_loaded(&resource);
                Some(wire)
            });
            let Some(wire) = wire else { return Ok(()) };
            self.resources.store(&wire, &Self::default_packs_key(kind)).await?;
            if self.state.with(|s| s.accepts(&resource, generation)) {
                self.publish().await;
            }
            anyhow::Ok(())
        }
        .await;
        self.finish_load(result, &resource).await;
    }

    pub async fn load_saved_packs(&self, kind: PackKind, relative_id: Option<String>, force: bool) {
        self.wait_for_hydration().await;
        let resource = RiseStickerResourceID::SavedPacks(kind);
        let Some(lease) = self.begin_coalesced(&resource, force, relative_id.is_some()).await else { return };
        let generation = lease.generation;
        let _end = RiseStickerDefer::new(|| self.end(&resource, lease));
        self.set_loading(&resource).await;
        let result: anyhow::Result<()> = async {
            let response: GetSavedPacksGatewayDTO = self
                .rpc
                .call(
                    RiseConversationEndpoints::get_saved_sticker_packs(),
                    rise_request::stickers::GetSavedPacks {
                        limit: Some(RiseStickerPerformancePolicy::packs_page_limit()),
                        pack_kind: Some(kind.wire().to_owned()),
                        relative_id: relative_id.clone(),
                    },
                )
                .await?;
            let wire = self.state.with(|s| {
                if !s.accepts(&resource, generation) {
                    return None;
                }
                let (presentation, wire) = Self::merged_saved_pack_page(
                    s.state.saved_packs.get(&kind),
                    &response.packs,
                    relative_id.as_deref(),
                    response.relative_id.clone(),
                    Some(response.has_more),
                );
                s.state.saved_packs.insert(kind, presentation);
                s.cached_presentation_tabs.insert(RiseStickerRepositoryState::picker_tab(kind));
                s.mark_loaded(&resource);
                Some(wire)
            });
            let Some(wire) = wire else { return Ok(()) };
            self.resources.store(&wire, &Self::saved_packs_key(kind)).await?;
            if self.state.with(|s| s.accepts(&resource, generation)) {
                self.publish().await;
            }
            anyhow::Ok(())
        }
        .await;
        self.finish_load(result, &resource).await;
    }

    pub async fn load_my_packs(&self, kind: PackKind, relative_id: Option<String>, force: bool) {
        self.wait_for_hydration().await;
        let resource = RiseStickerResourceID::MyPacks(kind);
        let lease = match relative_id.as_deref() {
            Some(cursor) => self.begin_my_packs_next_page(&resource, kind, cursor).await,
            None => self.begin_coalesced(&resource, force, false).await,
        };
        let Some(lease) = lease else { return };
        let generation = lease.generation;
        let _end = RiseStickerDefer::new(|| self.end(&resource, lease));
        self.set_loading(&resource).await;
        let result: anyhow::Result<()> = async {
            let response: GetMyPacksGatewayDTO = self
                .rpc
                .call(
                    RiseConversationEndpoints::get_my_sticker_packs(),
                    rise_request::stickers::GetMyPacks {
                        limit: Some(RiseStickerPerformancePolicy::packs_page_limit()),
                        pack_kind: Some(kind.wire().to_owned()),
                        page: None,
                        relative_id: relative_id.clone(),
                    },
                )
                .await?;
            let wire = self.state.with(|s| {
                if !s.accepts(&resource, generation) {
                    return None;
                }
                let (presentation, wire) = Self::merged_my_pack_page(
                    s.state.my_packs.get(&kind),
                    &response.packs,
                    kind,
                    relative_id.as_deref(),
                    response.relative_id.clone(),
                    Some(response.has_more),
                    Some(response.total),
                );
                s.state.my_packs.insert(kind, presentation);
                s.mark_loaded(&resource);
                Some(wire)
            });
            let Some(wire) = wire else { return Ok(()) };
            self.resources.store(&wire, &Self::my_packs_key(kind)).await?;
            if self.state.with(|s| s.accepts(&resource, generation)) {
                self.publish().await;
            }
            anyhow::Ok(())
        }
        .await;
        self.finish_load(result, &resource).await;
    }

    async fn begin_my_packs_next_page(
        &self,
        resource: &RiseStickerResourceID,
        kind: PackKind,
        cursor: &str,
    ) -> Option<RiseStickerResourceLease> {
        for _ in 0..MY_PACKS_NEXT_PAGE_ATTEMPTS {
            self.wait_until_resource_is_idle(resource).await;
            let current =
                self.state.with(|s| s.state.my_packs.get(&kind).and_then(|page| page.next_relative_id.clone()));
            if current.as_deref() != Some(cursor) {
                return None;
            }
            if let Some(lease) = self.begin_coalesced(resource, false, true).await {
                return Some(lease);
            }
            if self.state.with(|s| s.is_shutdown) {
                return None;
            }
            if !self.state.with(|s| s.in_flight.contains(resource)) {
                tokio::time::sleep(Duration::from_millis(MY_PACKS_NEXT_PAGE_RETRY_MS)).await;
            }
        }
        None
    }

    async fn finish_load(&self, result: anyhow::Result<()>, resource: &RiseStickerResourceID) {
        if let Err(error) = result
            && !is_cancellation(&error)
        {
            self.fail(resource).await;
        }
    }

    pub async fn load_pack(&self, pack_id: &str, force: bool) {
        let id = trimmed(pack_id);
        if id.is_empty() {
            return;
        }
        self.wait_for_hydration().await;
        let resource = RiseStickerResourceID::Pack(id.clone());
        let missing_tab = self
            .state
            .with(|s| (!s.state.packs.contains_key(&id)).then(|| s.picker_tab_for_pack_id(&id)));
        if let Some(tab) = missing_tab
            && self.hydrate_pack(&id, tab).await
        {
            self.publish().await;
        }
        let Some(lease) = self.begin_coalesced(&resource, force, false).await else { return };
        let generation = lease.generation;
        let _end = RiseStickerDefer::new(|| self.end(&resource, lease));
        let cancelled_read = RiseStickerDefer::new(|| {
            self.state.with(|s| {
                s.refreshed_at.remove(&resource);
            });
        });
        self.set_loading(&resource).await;
        let result: anyhow::Result<()> = async {
            let response: GetStickersByPackIdGatewayDTO = self
                .rpc
                .call(
                    RiseConversationEndpoints::get_stickers_by_pack_id(),
                    rise_request::stickers::GetStickersByPackId { pack_id: Some(id.clone()) },
                )
                .await?;
            if !self.state.with(|s| s.accepts(&resource, generation)) {
                return Ok(());
            }
            let bounded = GetStickersByPackIdGatewayDTO {
                error: response.error,
                pack: response.pack,
                stickers: response
                    .stickers
                    .into_iter()
                    .take(limit_count(RiseStickerPerformancePolicy::stickers_per_pack_limit()))
                    .collect(),
                success: response.success,
            };
            self.install_authoritative_pack(&bounded, &id).await?;
            let accepted = self.state.with(|s| {
                if let Some(tab) = s.picker_tab_for_pack_id(&id) {
                    s.cached_presentation_tabs.insert(tab);
                }
                s.mark_loaded(&resource);
                s.accepts(&resource, generation)
            });
            if accepted {
                self.publish().await;
            }
            anyhow::Ok(())
        }
        .await;
        cancelled_read.disarm();
        if let Err(error) = result {
            self.state.with(|s| {
                s.refreshed_at.remove(&resource);
            });
            if !is_cancellation(&error) {
                self.fail(&resource).await;
            }
        }
    }

    pub async fn load_pack_link(&self, raw_link: &str, force: bool) -> bool {
        let link = Self::normalized_search(raw_link);
        if link.is_empty() {
            return false;
        }
        self.wait_for_hydration().await;
        let resource = RiseStickerResourceID::PackLink(link.clone());
        if !self.state.with(|s| s.state.linked_packs.contains_key(&link)) && self.hydrate_linked_pack(&link).await {
            self.publish().await;
        }
        let Some(lease) = self.begin_coalesced(&resource, force, false).await else {
            let known = self.state.with(|s| s.state.linked_packs.get(&link).is_some_and(|content| content.pack.is_some()));
            tracing::warn!(target: "riseonly::X1022", link = %link, known, "pack link coalesced without lease");
            return known;
        };
        let generation = lease.generation;
        let _end = RiseStickerDefer::new(|| self.end(&resource, lease));
        self.set_loading(&resource).await;
        let result: anyhow::Result<bool> = async {
            let response: GetStickersByPackLinkGatewayDTO = self
                .rpc
                .call(
                    RiseConversationEndpoints::get_stickers_by_pack_link(),
                    rise_request::stickers::GetStickersByPackLink { pack_link: Some(link.clone()) },
                )
                .await?;
            if !self.state.with(|s| s.accepts(&resource, generation)) {
                tracing::warn!(target: "riseonly::X1022", link = %link, "pack link reply superseded");
                return Ok(false);
            }
            tracing::warn!(
                target: "riseonly::X1022",
                link = %link,
                has_pack = response.pack.is_some(),
                stickers = response.stickers.len(),
                success = ?response.success,
                error = ?response.error,
                "pack link reply"
            );
            let bounded = GetStickersByPackLinkGatewayDTO {
                error: response.error,
                pack: response.pack,
                stickers: response
                    .stickers
                    .into_iter()
                    .take(limit_count(RiseStickerPerformancePolicy::stickers_per_pack_limit()))
                    .collect(),
                success: response.success,
            };
            let mut content = Self::pack_content_link(&bounded);
            let pack_id = content.pack.as_ref().map(|pack| pack.id.clone()).filter(|id| !id.is_empty());
            if let Some(id) = pack_id {
                self.state.with(|s| {
                    s.preserve_uploading_stickers(&mut content, &id);
                    s.state.linked_packs.insert(link.clone(), content.clone());
                    s.state.packs.insert(id.clone(), content.clone());
                });
                self.resources.store(&bounded, &Self::pack_link_key(&link)).await?;
                if !self.state.with(|s| s.accepts(&resource, generation)) {
                    return Ok(false);
                }
                let by_id = GetStickersByPackIdGatewayDTO {
                    error: bounded.error.clone(),
                    pack: bounded.pack.clone(),
                    stickers: bounded.stickers.clone(),
                    success: bounded.success,
                };
                self.resources.store(&by_id, &Self::pack_key(&id)).await?;
                if !self.state.with(|s| s.accepts(&resource, generation)) {
                    return Ok(false);
                }
                self.record_persisted_pack(&id).await?;
            } else {
                self.state.with(|s| {
                    s.state.linked_packs.remove(&link);
                });
                self.resources.remove(&Self::pack_link_key(&link)).await?;
            }
            let accepted = self.state.with(|s| {
                if !s.accepts(&resource, generation) {
                    return false;
                }
                s.mark_loaded(&resource);
                true
            });
            if !accepted {
                return Ok(false);
            }
            self.publish().await;
            anyhow::Ok(content.pack.is_some())
        }
        .await;
        match result {
            Ok(found) => found,
            Err(error) => {
                tracing::warn!(target: "riseonly::X1022", link = %link, error = %error, "pack link read failed");
                if !is_cancellation(&error) {
                    self.fail(&resource).await;
                }
                false
            }
        }
    }

    pub async fn load_favourites(&self, force: bool) {
        self.wait_for_hydration().await;
        let resource = RiseStickerResourceID::Favourites;
        let Some(generation) = self.state.with(|s| s.begin(&resource, force)) else { return };
        let _end = RiseStickerDefer::new(|| self.end_in_flight(&resource));
        self.set_loading(&resource).await;
        let favourites_limit = self.current_favourites_limit().await;
        let result: anyhow::Result<()> = async {
            let response: rise_response::stickers::GetUserFavourites = self
                .rpc
                .call(
                    RiseConversationEndpoints::get_sticker_favourites(),
                    rise_request::stickers::GetUserFavourites { limit: Some(favourites_limit) },
                )
                .await?;
            let wire = self.state.with(|s| {
                if !s.accepts(&resource, generation) {
                    return None;
                }
                let wire = rise_response::stickers::GetUserFavourites {
                    stickers: response.stickers.iter().take(limit_count(favourites_limit)).cloned().collect(),
                    ..Default::default()
                };
                s.state.favourites = wire.stickers.iter().map(|sticker| sticker.to_sticker()).collect();
                s.cached_presentation_tabs.insert(RiseStickerPickerTab::Stickers);
                s.mark_loaded(&resource);
                Some(wire)
            });
            let Some(wire) = wire else { return Ok(()) };
            self.resources.store(&wire, &Self::favourites_key()).await?;
            self.publish().await;
            anyhow::Ok(())
        }
        .await;
        self.finish_load(result, &resource).await;
    }

    pub async fn load_recents(&self, kind: PackKind, force: bool) {
        self.wait_for_hydration().await;
        let resource = RiseStickerResourceID::Recents(kind);
        let Some(generation) = self.state.with(|s| s.begin(&resource, force)) else { return };
        let _end = RiseStickerDefer::new(|| self.end_in_flight(&resource));
        self.set_loading(&resource).await;
        let result: anyhow::Result<()> = async {
            let recents_limit = RiseStickerPerformancePolicy::recents_limit();
            let response: rise_response::stickers::GetUserRecent = self
                .rpc
                .call(
                    RiseConversationEndpoints::get_sticker_recents(),
                    rise_request::stickers::GetUserRecent {
                        kind: Some(kind.wire().to_owned()),
                        limit: Some(recents_limit),
                    },
                )
                .await?;
            let wire = self.state.with(|s| {
                if !s.accepts(&resource, generation) {
                    return None;
                }
                let wire = rise_response::stickers::GetUserRecent {
                    stickers: response.stickers.iter().take(limit_count(recents_limit)).cloned().collect(),
                    ..Default::default()
                };
                s.state.recents.insert(kind, wire.stickers.iter().map(|sticker| sticker.to_sticker()).collect());
                s.cached_presentation_tabs.insert(RiseStickerRepositoryState::picker_tab(kind));
                s.mark_loaded(&resource);
                Some(wire)
            });
            let Some(wire) = wire else { return Ok(()) };
            self.resources.store(&wire, &Self::recents_key(kind)).await?;
            self.publish().await;
            anyhow::Ok(())
        }
        .await;
        self.finish_load(result, &resource).await;
    }

    pub async fn load_gif_favourites(&self, relative_id: Option<String>, force: bool) {
        self.wait_for_hydration().await;
        let resource = RiseStickerResourceID::GifFavourites;
        let Some(generation) = self.state.with(|s| s.begin(&resource, force || relative_id.is_some())) else {
            return;
        };
        let _end = RiseStickerDefer::new(|| self.end_in_flight(&resource));
        self.set_loading(&resource).await;
        let result: anyhow::Result<()> = async {
            let response: GetGifFavouritesGatewayDTO = self
                .rpc
                .call(
                    RiseConversationEndpoints::get_gif_favourites(),
                    rise_request::stickers::GetGifFavourites {
                        limit: Some(RiseStickerPerformancePolicy::gif_page_limit()),
                        relative_id: relative_id.clone(),
                    },
                )
                .await?;
            let wire = self.state.with(|s| {
                if !s.accepts(&resource, generation) || !s.state.pending_gif_ids.is_empty() {
                    s.refreshed_at.remove(&resource);
                    return None;
                }
                let (presentation, wire) = Self::merged_gif_page(
                    &s.state.gif_favourites,
                    &response.gifs,
                    relative_id.as_deref(),
                    response.relative_id.clone(),
                    Some(response.has_more),
                    RiseStickerPerformancePolicy::gif_limit(),
                );
                s.state.gif_favourites = presentation;
                s.cached_presentation_tabs.insert(RiseStickerPickerTab::Gifs);
                s.mark_loaded(&resource);
                Some(wire)
            });
            let Some(wire) = wire else { return Ok(()) };
            self.resources.store(&wire, &Self::gif_favourites_key()).await?;
            self.publish().await;
            anyhow::Ok(())
        }
        .await;
        self.finish_load(result, &resource).await;
    }

    pub async fn search_gifs(&self, raw_query: &str, relative_id: Option<String>, force: bool) {
        let query = Self::normalized_search(raw_query);
        self.wait_for_hydration().await;
        let resource = RiseStickerResourceID::GifSearch(query.clone());
        let generation = self.state.with(|s| {
            if relative_id.is_none() {
                s.last_gif_search_query = Some(query.clone());
            }
            let retrying_failure = s.state.load_states.get(&resource) == Some(&RiseStickerLoadState::Failed);
            s.begin(&resource, force || retrying_failure || relative_id.is_some())
        });
        let Some(generation) = generation else { return };
        let _end = RiseStickerDefer::new(|| self.end_in_flight(&resource));
        self.set_loading(&resource).await;
        let result: anyhow::Result<()> = async {
            let response: SearchGifsGatewayDTO = self
                .rpc
                .call(
                    RiseConversationEndpoints::search_gifs(),
                    rise_request::stickers::SearchGifs {
                        limit: Some(RiseStickerPerformancePolicy::gif_page_limit()),
                        pos: relative_id.clone(),
                        query: Some(query.clone()),
                    },
                )
                .await?;
            let accepted = self.state.with(|s| {
                if !s.accepts(&resource, generation) {
                    return false;
                }
                let page = Self::merged_search_page(
                    s.state.gif_searches.get(&query),
                    &response.gifs,
                    relative_id.as_deref(),
                    response.relative_id.clone(),
                    Some(response.has_more),
                );
                s.state.gif_searches.insert(query.clone(), page);
                s.touch_search(&query);
                s.mark_loaded(&resource);
                true
            });
            if accepted {
                self.publish().await;
            }
            anyhow::Ok(())
        }
        .await;
        self.finish_load(result, &resource).await;
    }

    async fn current_favourites_limit(&self) -> i64 {
        let account_id = self.account_id.clone();
        let is_premium = Self::on_main(move |cx| {
            let profile = AuthServicesStore::try_read(cx)?.profile()?;
            (profile.id == account_id).then_some(profile.more.is_premium)
        })
        .await
        .flatten();
        RiseStickerPerformancePolicy::favourites_limit(is_premium.unwrap_or(true))
    }

    pub async fn add_favourite(&self, sticker: StickerItem) {
        if sticker.id.is_empty() {
            return;
        }
        let favourites_limit = self.current_favourites_limit().await;
        let old = self.state.with(|s| {
            let old = s.state.favourites.clone();
            s.state.pending_favourite_ids.insert(sticker.id.clone());
            let mut values = vec![sticker.clone()];
            values.extend(old.iter().cloned());
            s.state.favourites = Self::deduplicated(values, |item| item.id.as_str(), favourites_limit);
            old
        });
        self.publish().await;
        let rollback = RiseStickerDefer::new(|| {
            self.state.with(|s| {
                s.state.favourites = old.clone();
                s.state.pending_favourite_ids.remove(&sticker.id);
            });
            self.publish_detached();
        });
        let result: anyhow::Result<()> = async {
            let _: rise_response::stickers::AddStickerFavourite = self
                .rpc
                .call(
                    RiseConversationEndpoints::add_sticker_favourite(),
                    rise_request::stickers::AddStickerFavourite { sticker_id: Some(sticker.id.clone()) },
                )
                .await?;
            self.persist_favourites().await
        }
        .await;
        rollback.disarm();
        self.state.with(|s| {
            if result.is_err() {
                s.state.favourites = old.clone();
            }
            s.state.pending_favourite_ids.remove(&sticker.id);
        });
        self.publish().await;
    }

    pub async fn remove_favourite(&self, sticker_id: &str) {
        if sticker_id.is_empty() {
            return;
        }
        let old = self.state.with(|s| {
            let old = s.state.favourites.clone();
            s.state.pending_favourite_ids.insert(sticker_id.to_owned());
            s.state.favourites.retain(|item| item.id != sticker_id);
            old
        });
        self.publish().await;
        let rollback = RiseStickerDefer::new(|| {
            self.state.with(|s| {
                s.state.favourites = old.clone();
                s.state.pending_favourite_ids.remove(sticker_id);
            });
            self.publish_detached();
        });
        let result: anyhow::Result<()> = async {
            let _: rise_response::stickers::RemoveStickerFavourite = self
                .rpc
                .call(
                    RiseConversationEndpoints::remove_sticker_favourite(),
                    rise_request::stickers::RemoveStickerFavourite { sticker_id: Some(sticker_id.to_owned()) },
                )
                .await?;
            self.persist_favourites().await
        }
        .await;
        rollback.disarm();
        self.state.with(|s| {
            if result.is_err() {
                s.state.favourites = old.clone();
            }
            s.state.pending_favourite_ids.remove(sticker_id);
        });
        self.publish().await;
    }

    pub async fn write_recent(&self, sticker_id: &str, kind: PackKind) {
        if sticker_id.is_empty() {
            return;
        }
        let found = self.state.with(|s| {
            let item = s.sticker(sticker_id)?;
            let mut values = vec![item];
            values.extend(s.state.recents.get(&kind).into_iter().flatten().cloned());
            s.state.recents.insert(
                kind,
                Self::deduplicated(values, |item| item.id.as_str(), RiseStickerPerformancePolicy::recents_limit()),
            );
            Some(())
        });
        if found.is_some() {
            self.publish().await;
            let _ = self.persist_recents(kind).await;
        }
        let _: anyhow::Result<rise_response::stickers::WriteRecent> = self
            .rpc
            .call(
                RiseConversationEndpoints::write_sticker_recent(),
                rise_request::stickers::WriteRecent {
                    kind: Some(kind.wire().to_owned()),
                    sticker_id: Some(sticker_id.to_owned()),
                },
            )
            .await;
    }

    pub async fn clear_recents(&self, kind: PackKind) {
        self.wait_for_hydration().await;
        let resource = RiseStickerResourceID::Recents(kind);
        let (previous, clear_generation) = self.state.with(|s| {
            let previous = s.state.recents.get(&kind).cloned().unwrap_or_default();
            let generation = s.generations.entry(resource.clone()).or_default();
            *generation = generation.wrapping_add(1);
            let clear_generation = *generation;
            s.state.recents.insert(kind, Vec::new());
            s.mark_loaded(&resource);
            s.cached_presentation_tabs.insert(RiseStickerRepositoryState::picker_tab(kind));
            (previous, clear_generation)
        });
        self.publish().await;
        let _ = self.persist_recents(kind).await;

        let result: anyhow::Result<rise_response::stickers::ClearRecent> = self
            .rpc
            .call(
                RiseConversationEndpoints::clear_sticker_recent(),
                rise_request::stickers::ClearRecent { kind: Some(kind.wire().to_owned()) },
            )
            .await;
        let Err(error) = result else { return };
        if is_cancellation(&error) {
            return;
        }
        let restored = self.state.with(|s| {
            if s.generations.get(&resource) != Some(&clear_generation)
                || !s.state.recents.get(&kind).is_some_and(Vec::is_empty)
            {
                return false;
            }
            s.state.recents.insert(kind, previous);
            s.refreshed_at.remove(&resource);
            true
        });
        if restored {
            self.publish().await;
            let _ = self.persist_recents(kind).await;
        }
    }

    pub async fn add_gif_favourite(&self, gif: GifItem) {
        if gif.id.is_empty() {
            return;
        }
        self.wait_for_hydration().await;
        let old = self.state.with(|s| {
            if s.state.pending_gif_ids.contains(&gif.id) {
                return None;
            }
            s.bump_generation(&RiseStickerResourceID::GifFavourites);
            let old = s.state.gif_favourites.clone();
            s.state.pending_gif_ids.insert(gif.id.clone());
            let mut values = vec![gif.clone()];
            values.extend(old.items.iter().cloned());
            s.state.gif_favourites.items =
                Self::deduplicated(values, |item| item.id.as_str(), RiseStickerPerformancePolicy::gif_limit());
            Some(old)
        });
        let Some(old) = old else { return };
        self.publish().await;
        let rollback = RiseStickerDefer::new(|| {
            self.state.with(|s| {
                s.state.gif_favourites = old.clone();
                s.state.pending_gif_ids.remove(&gif.id);
            });
            self.publish_detached();
        });
        let result: anyhow::Result<()> = async {
            let response: AddGifFavouriteGatewayDTO = self
                .rpc
                .call(
                    RiseConversationEndpoints::add_gif_favourite(),
                    rise_request::stickers::AddGifFavourite {
                        gif: Some(rise_request::stickers::add_gif_favourite::Gif {
                            duration_ms: Some(gif.duration_ms),
                            file_url: Some(gif.file_url.clone()),
                            height: Some(gif.height),
                            preview_url: Some(gif.preview_url.clone()),
                            source: Some(gif.source.clone()),
                            source_id: Some(gif.source_id.clone()),
                            width: Some(gif.width),
                            ..Default::default()
                        }),
                    },
                )
                .await?;
            if let Some(real) = response.gif.map(|value| value.to_gif()).filter(|real| !real.id.is_empty()) {
                self.state.with(|s| {
                    s.state.gif_favourites.items.retain(|item| item.id != gif.id && item.id != real.id);
                    s.state.gif_favourites.items.insert(0, real);
                });
            }
            self.persist_gif_favourites().await
        }
        .await;
        rollback.disarm();
        self.state.with(|s| {
            if result.is_err() {
                s.state.gif_favourites = old.clone();
            }
            s.state.pending_gif_ids.remove(&gif.id);
        });
        self.publish().await;
    }

    pub async fn remove_gif_favourite(&self, gif_id: &str) {
        if gif_id.is_empty() {
            return;
        }
        self.wait_for_hydration().await;
        let old = self.state.with(|s| {
            if s.state.pending_gif_ids.contains(gif_id) {
                return None;
            }
            s.bump_generation(&RiseStickerResourceID::GifFavourites);
            let old = s.state.gif_favourites.clone();
            s.state.pending_gif_ids.insert(gif_id.to_owned());
            s.state.gif_favourites.items.retain(|item| item.id != gif_id);
            Some(old)
        });
        let Some(old) = old else { return };
        self.publish().await;
        let rollback = RiseStickerDefer::new(|| {
            self.state.with(|s| {
                s.state.gif_favourites = old.clone();
                s.state.pending_gif_ids.remove(gif_id);
            });
            self.publish_detached();
        });
        let result: anyhow::Result<()> = async {
            let _: rise_response::stickers::RemoveGifFavourite = self
                .rpc
                .call(
                    RiseConversationEndpoints::remove_gif_favourite(),
                    rise_request::stickers::RemoveGifFavourite { gif_id: Some(gif_id.to_owned()) },
                )
                .await?;
            self.persist_gif_favourites().await
        }
        .await;
        rollback.disarm();
        self.state.with(|s| {
            if result.is_err() {
                s.state.gif_favourites = old.clone();
            }
            s.state.pending_gif_ids.remove(gif_id);
        });
        self.publish().await;
    }

    pub async fn set_pack_saved(&self, pack_id: &str, kind: PackKind, pack: Option<StickerPack>, is_saved: bool) {
        if pack_id.is_empty() {
            return;
        }
        let (old, initially_fenced_link_loads) = self.state.with(|s| {
            let old = s.pack_mutation_rollback(pack_id, kind);
            let fenced = s.fence_pack_mutation_loads(pack_id, kind);
            s.apply_saved(pack_id, kind, pack.as_ref(), is_saved);
            (old, fenced)
        });
        self.publish().await;
        let rollback = RiseStickerDefer::new(|| {
            self.state.with(|s| s.restore_pack_mutation_rollback(&old));
            self.publish_detached();
        });
        let result: anyhow::Result<()> = async {
            if is_saved {
                let _: rise_response::stickers::SavePack = self
                    .rpc
                    .call(
                        RiseConversationEndpoints::save_sticker_pack(),
                        rise_request::stickers::SavePack { pack_id: Some(pack_id.to_owned()) },
                    )
                    .await?;
            } else {
                let _: rise_response::stickers::RemoveSavedPack = self
                    .rpc
                    .call(
                        RiseConversationEndpoints::remove_saved_sticker_pack(),
                        rise_request::stickers::RemoveSavedPack { pack_id: Some(pack_id.to_owned()) },
                    )
                    .await?;
            }
            anyhow::Ok(())
        }
        .await;
        rollback.disarm();
        if result.is_err() {
            self.state.with(|s| s.restore_pack_mutation_rollback(&old));
            self.publish().await;
            self.wait_until_resources_are_idle(&initially_fenced_link_loads).await;
            self.reconcile_pack_mutation_authoritatively(pack_id, kind).await;
            return;
        }

        let fenced_link_loads = self.state.with(|s| {
            let fenced = s.fence_pack_mutation_loads(pack_id, kind);
            s.apply_saved(pack_id, kind, pack.as_ref(), is_saved);
            fenced
        });
        self.publish().await;
        self.wait_until_resources_are_idle(&fenced_link_loads).await;
        self.reconcile_pack_mutation_authoritatively(pack_id, kind).await;
        let _ = self.persist_pack_lists(kind).await;
        let _ = self.persist_my_packs(kind).await;
        let _ = self.persist_pack(pack_id).await;
        let _ = self.persist_linked_packs(pack_id).await;
    }

    pub async fn delete_pack(&self, pack_id: &str, kind: PackKind) {
        if pack_id.is_empty() {
            return;
        }
        let (old, initially_fenced_link_loads) = self.state.with(|s| {
            let old = s.pack_mutation_rollback(pack_id, kind);
            let fenced = s.fence_pack_mutation_loads(pack_id, kind);
            s.apply_deleted_pack(pack_id, kind);
            (old, fenced)
        });
        let mut linked_keys: HashSet<String> = old.linked_contents.keys().cloned().collect();
        self.publish().await;
        let rollback = RiseStickerDefer::new(|| {
            self.state.with(|s| s.restore_pack_mutation_rollback(&old));
            self.publish_detached();
        });
        let result: anyhow::Result<rise_response::stickers::DeletePack> = self
            .rpc
            .call(
                RiseConversationEndpoints::delete_sticker_pack(),
                rise_request::stickers::DeletePack { pack_id: Some(pack_id.to_owned()) },
            )
            .await;
        rollback.disarm();
        if result.is_err() {
            self.state.with(|s| s.restore_pack_mutation_rollback(&old));
            self.publish().await;
            self.wait_until_resources_are_idle(&initially_fenced_link_loads).await;
            self.reconcile_pack_mutation_authoritatively(pack_id, kind).await;
            return;
        }

        let fenced_link_loads = self.state.with(|s| {
            let fenced = s.fence_pack_mutation_loads(pack_id, kind);
            linked_keys.extend(s.linked_keys(pack_id));
            s.apply_deleted_pack(pack_id, kind);
            fenced
        });
        self.publish().await;
        self.wait_until_resources_are_idle(&fenced_link_loads).await;
        self.reconcile_pack_mutation_authoritatively(pack_id, kind).await;
        self.state.with(|s| linked_keys.extend(s.linked_keys(pack_id)));
        let _ = self.persist_pack_lists(kind).await;
        let _ = self.persist_my_packs(kind).await;
        let _ = self.remove_persisted_pack(pack_id).await;
        let links: Vec<String> = linked_keys.into_iter().collect();
        let _ = self.remove_persisted_linked_packs(&links).await;
    }

    pub async fn rename_pack(&self, pack_id: &str, kind: PackKind, raw_title: &str) -> anyhow::Result<()> {
        self.wait_for_hydration().await;
        let title = trimmed(raw_title);
        if pack_id.is_empty() || title.is_empty() {
            return Err(anyhow::Error::new(RiseStickerRepositoryError::InvalidPackTitle));
        }
        let _: rise_response::stickers::EditPack = self
            .rpc
            .call(
                RiseConversationEndpoints::edit_sticker_pack(),
                rise_request::stickers::EditPack {
                    pack_id: Some(pack_id.to_owned()),
                    title: Some(title),
                    ..Default::default()
                },
            )
            .await?;
        self.reconcile_pack_mutation_authoritatively(pack_id, kind).await;
        let _ = self.persist_pack_lists(kind).await;
        let _ = self.persist_my_packs(kind).await;
        let _ = self.persist_pack(pack_id).await;
        let _ = self.persist_linked_packs(pack_id).await;
        Ok(())
    }

    pub async fn create_pack(&self, raw_title: &str, kind: PackKind, request_id: &str) -> anyhow::Result<StickerPack> {
        self.wait_for_hydration().await;
        let title = trimmed(raw_title);
        if title.is_empty() {
            return Err(anyhow::Error::new(RiseStickerRepositoryError::InvalidPackTitle));
        }
        if trimming_whitespaces_and_newlines(request_id).is_empty() {
            return Err(rejected("Sticker pack request ID is missing"));
        }

        let mutation = RiseStickerMutationID::CreatePack(kind);
        let local_id = format!("{LOCAL_PACK_PREFIX}{request_id}");
        let (mutation_generation, expected_lifecycle_generation) = self.state.with(|s| {
            let current_total = s.state.my_packs.get(&kind).map_or(0, |page| {
                page.total.unwrap_or(0).max(own_pack_count(&page.items))
            });
            if current_total >= RiseStickerPerformancePolicy::created_packs_per_kind_limit() {
                return Err(anyhow::Error::new(RiseStickerRepositoryError::PackCreationLimitReached(kind)));
            }
            let generation = s.begin_mutation(&mutation)?;
            let now = now_ms();
            let optimistic = StickerPack::new(
                local_id.clone(),
                title.clone(),
                String::new(),
                false,
                true,
                now,
                now,
                false,
                true,
                kind,
                String::new(),
                StickerAssetType::StaticImage,
                false,
                0,
                None,
            );
            s.insert_optimistic_pack(&optimistic, kind);
            s.state.packs.insert(
                local_id.clone(),
                RiseStickerPackContent { pack: Some(optimistic), stickers: Vec::new() },
            );
            s.state.mutation_states.insert(mutation.clone(), RiseStickerMutationState::Submitting);
            Ok((generation, s.lifecycle_generation))
        })?;
        self.publish().await;

        let rollback = RiseStickerDefer::new(|| {
            self.state.with(|s| {
                s.rollback_optimistic_pack(&local_id, kind);
                s.fail_mutation(&mutation, mutation_generation, &cancelled());
            });
            self.publish_detached();
        });
        let result: anyhow::Result<StickerPack> = async {
            let response: rise_response::stickers::CreatePack = self
                .rpc
                .call(
                    RiseConversationEndpoints::create_sticker_pack(),
                    rise_request::stickers::CreatePack {
                        pack_kind: Some(kind.wire().to_owned()),
                        request_id: Some(request_id.to_owned()),
                        title: Some(title.clone()),
                        ..Default::default()
                    },
                )
                .await?;
            if !self.state.with(|s| s.accepts_mutation(&mutation, mutation_generation, expected_lifecycle_generation)) {
                return Err(cancelled());
            }
            let Some(wire) = response else {
                return Err(rejected("Sticker pack response is missing the pack"));
            };
            let decoded = wire.to_pack();
            let decoded_title = trimmed(&decoded.title);
            let created = StickerPack::new(
                decoded.id,
                if decoded_title.is_empty() { title.clone() } else { decoded_title },
                decoded.link,
                decoded.is_admin,
                true,
                wire.created_at_ms,
                wire.updated_at_ms,
                decoded.is_default,
                true,
                kind,
                decoded.preview_url,
                decoded.preview_asset_type,
                decoded.is_premium,
                decoded.unique_saves,
                None,
            );
            if created.id.is_empty() {
                return Err(rejected("Sticker pack response is missing its ID"));
            }

            self.state.with(|s| {
                s.reconcile_created_pack(&created, &local_id, kind);
                s.finish_mutation(&mutation);
            });
            self.publish().await;
            let _ = self.persist_pack_lists(kind).await;
            let _ = self.persist_my_packs(kind).await;
            let _ = self.persist_pack(&created.id).await;
            let _ = self.record_persisted_pack(&created.id).await;
            anyhow::Ok(created)
        }
        .await;
        rollback.disarm();
        match result {
            Ok(created) => Ok(created),
            Err(error) => {
                self.state.with(|s| {
                    s.rollback_optimistic_pack(&local_id, kind);
                    s.fail_mutation(&mutation, mutation_generation, &error);
                });
                self.publish().await;
                Err(error)
            }
        }
    }

    /// the pack the mutation targets, loaded first if the snapshot doesnt know it yet
    async fn resolved_pack_loading(&self, pack_id: &str, requires_content: bool) -> Option<StickerPack> {
        let known = self.state.with(|s| {
            let pack = s.resolved_pack(pack_id);
            if requires_content && !s.state.packs.contains_key(pack_id) {
                return None;
            }
            pack
        });
        if known.is_some() {
            return known;
        }
        self.load_pack(pack_id, false).await;
        self.state.with(|s| s.resolved_pack(pack_id))
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn stage_sticker_preparation(
        &self,
        raw_pack_id: &str,
        raw_request_id: &str,
        raw_preview_url: &str,
        raw_thumbnail_url: Option<String>,
        width: Option<i64>,
        height: Option<i64>,
        duration_ms: Option<i64>,
        emojis: Vec<String>,
        asset_type: StickerAssetType,
        mime_type: Option<String>,
    ) -> anyhow::Result<()> {
        self.wait_for_hydration().await;
        let pack_id = trimmed(raw_pack_id);
        let request_id = trimmed(raw_request_id);
        let preview_url = trimmed(raw_preview_url);
        if pack_id.is_empty() || request_id.is_empty() {
            return Err(anyhow::Error::new(RiseStickerRepositoryError::InvalidMediaDraft));
        }

        let local_id = format!("{LOCAL_STICKER_PREFIX}{request_id}");
        if self.state.with(|s| s.state.sticker_uploads.contains_key(&local_id)) {
            return Ok(());
        }

        let Some(target_pack) = self.resolved_pack_loading(&pack_id, false).await else {
            return Err(rejected("Sticker pack must be loaded before adding media"));
        };
        if !target_pack.can_add_stickers() {
            return Err(rejected("Only a pack owner, administrator or co-author with the add right can add stickers"));
        }

        self.state.with(|s| {
            let mut content = s.state.packs.get(&pack_id).cloned().unwrap_or_else(|| RiseStickerPackContent {
                pack: Some(target_pack),
                stickers: Vec::new(),
            });
            if content.stickers.len() as i64 >= RiseStickerPerformancePolicy::stickers_per_pack_limit() {
                return Err(anyhow::Error::new(Self::pack_full_failure()));
            }
            let position = content.stickers.iter().map(|sticker| sticker.position).max().unwrap_or(-1) + 1;
            content.stickers.push(StickerItem::new(
                local_id.clone(),
                pack_id.clone(),
                preview_url,
                raw_thumbnail_url,
                width.unwrap_or(0).max(0),
                height.unwrap_or(0).max(0),
                duration_ms,
                position,
                now_ms(),
                emojis,
                asset_type,
                mime_type,
                None,
            ));
            s.state.packs.insert(pack_id.clone(), content);
            s.state.sticker_uploads.insert(
                local_id.clone(),
                RiseStickerUploadPresentation::new(
                    request_id,
                    pack_id.clone(),
                    local_id.clone(),
                    RiseStickerMutationState::Preparing,
                    0.0,
                    0,
                    0,
                ),
            );
            Ok(())
        })?;
        self.publish().await;
        Ok(())
    }

    pub async fn update_sticker_preparation_progress(
        &self,
        raw_request_id: &str,
        raw_pack_id: &str,
        raw_sticker_id: Option<&str>,
        fraction: f64,
        total_bytes: i64,
    ) {
        let request_id = trimmed(raw_request_id);
        let pack_id = trimmed(raw_pack_id);
        let local_id = raw_sticker_id
            .map(trimmed)
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| format!("{LOCAL_STICKER_PREFIX}{request_id}"));
        let changed = self.state.with(|s| {
            let Some(upload) = s.state.sticker_uploads.get_mut(&local_id) else { return false };
            if upload.request_id != request_id
                || upload.pack_id != pack_id
                || upload.phase != RiseStickerMutationState::Preparing
            {
                return false;
            }
            let bounded = fraction.clamp(0.0, 1.0);
            let bounded_total = total_bytes.max(0);
            if bounded - upload.fraction < rise_upload::PROGRESS_EPSILON_FRACTION
                && bounded < 1.0
                && bounded_total <= upload.total_bytes
            {
                return false;
            }
            upload.fraction = upload.fraction.max(bounded);
            upload.total_bytes = upload.total_bytes.max(bounded_total);
            upload.completed_bytes = if upload.total_bytes > 0 {
                upload.total_bytes.min((upload.total_bytes as f64 * upload.fraction).round() as i64)
            } else {
                0
            };
            true
        });
        if changed {
            self.publish().await;
        }
    }

    pub async fn stage_sticker_edit_preparation(
        &self,
        raw_pack_id: &str,
        raw_sticker_id: &str,
        raw_request_id: &str,
    ) -> anyhow::Result<()> {
        self.wait_for_hydration().await;
        let pack_id = trimmed(raw_pack_id);
        let sticker_id = trimmed(raw_sticker_id);
        let request_id = trimmed(raw_request_id);
        if pack_id.is_empty() || sticker_id.is_empty() || request_id.is_empty() {
            return Err(anyhow::Error::new(RiseStickerRepositoryError::InvalidMediaDraft));
        }
        if self.state.with(|s| s.state.sticker_uploads.contains_key(&sticker_id)) {
            return Ok(());
        }

        let target_pack = self.resolved_pack_loading(&pack_id, false).await;
        self.state.with(|s| {
            let allowed = target_pack.as_ref().is_some_and(StickerPack::can_edit_stickers)
                && s.contains_sticker(&pack_id, &sticker_id);
            if !allowed {
                return Err(rejected("Only a pack owner or administrator can edit stickers"));
            }
            s.state.sticker_uploads.insert(
                sticker_id.clone(),
                RiseStickerUploadPresentation::new(
                    request_id,
                    pack_id.clone(),
                    sticker_id.clone(),
                    RiseStickerMutationState::Preparing,
                    0.0,
                    0,
                    0,
                ),
            );
            Ok(())
        })?;
        self.publish().await;
        Ok(())
    }

    async fn prepare_upload_authentication(&self, account_epoch: &str) -> anyhow::Result<()> {
        let Some(hop) = RiseEngineMainHop::installed() else { return Err(cancelled()) };
        let account_user_id = self.account_id.clone();
        let account_epoch = account_epoch.to_owned();
        hop.run(move |mut cx: AsyncApp| {
            async move {
                RiseConversationUploadSupport::prepare_authentication(
                    Some(account_user_id),
                    Some(account_epoch),
                    &mut cx,
                )
                .await
            }
            .boxed_local()
        })
        .await
    }

    fn upload_progress_handler(
        &self,
        mutation: &RiseStickerMutationID,
        generation: u64,
        local_sticker_id: &str,
    ) -> RiseChunkedUploadProgressHandler {
        let this = self.this.clone();
        let mutation = mutation.clone();
        let local_sticker_id = local_sticker_id.to_owned();
        Arc::new(move |progress: RiseChunkedUploadTransferProgress| {
            let Some(repository) = this.upgrade() else { return };
            let mutation = mutation.clone();
            let local_sticker_id = local_sticker_id.clone();
            RiseAccountRuntime::current().spawn(async move {
                repository.update_mutation_progress(progress, &mutation, generation, &local_sticker_id).await;
            });
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn add_sticker(
        &self,
        raw_pack_id: &str,
        draft: StickerMediaUploadDraft,
        emojis: Vec<String>,
        asset_type: Option<StickerAssetType>,
        codec: Option<String>,
        raw_request_id: &str,
    ) -> anyhow::Result<StickerItem> {
        self.wait_for_hydration().await;
        let pack_id = trimmed(raw_pack_id);
        let request_id = trimmed(raw_request_id);
        let filename = trimmed(&draft.filename);
        let mime_type = trimmed(&draft.mime_type);
        let retained_attempt = self.state.with(|s| s.pending_sticker_create_attempts.get(&request_id).cloned());
        let draft_exists = retained_attempt.is_some()
            || draft.local_url.to_file_path().is_some_and(|path| FileManager.file_exists(&path));
        if pack_id.is_empty()
            || request_id.is_empty()
            || !draft.local_url.is_file_url()
            || !draft_exists
            || filename.is_empty()
            || mime_type.is_empty()
        {
            return Err(anyhow::Error::new(RiseStickerRepositoryError::InvalidMediaDraft));
        }
        if let Some(retained_attempt) = &retained_attempt
            && !retained_attempt.matches(&pack_id, &draft, &emojis, asset_type, codec.as_deref())
        {
            return Err(rejected("Sticker retry does not match its original upload"));
        }
        let Some(target_pack) = self.resolved_pack_loading(&pack_id, false).await else {
            return Err(rejected("Sticker pack must be loaded before adding media"));
        };
        if !target_pack.can_add_stickers() {
            return Err(rejected("Only a pack owner, administrator or co-author with the add right can add stickers"));
        }
        let target_kind = target_pack.pack_kind;

        let mutation = RiseStickerMutationID::AddSticker { pack_id: pack_id.clone() };
        let local_id = format!("{LOCAL_STICKER_PREFIX}{request_id}");
        let resolved_asset_type = asset_type.unwrap_or_else(|| StickerAssetType::infer(None, &filename));
        let draft_bytes = Self::local_file_size(&draft.local_url);
        let (mutation_generation, expected_lifecycle_generation, optimistic_content) = self.state.with(|s| {
            let generation = s.begin_mutation(&mutation)?;
            let content = s.state.packs.get(&pack_id).cloned().unwrap_or_else(|| RiseStickerPackContent {
                pack: Some(target_pack),
                stickers: Vec::new(),
            });
            let staged_index = content.stickers.iter().position(|sticker| sticker.id == local_id);
            let counted = content.stickers.len() as i64 - i64::from(staged_index.is_some());
            if counted >= RiseStickerPerformancePolicy::stickers_per_pack_limit() {
                s.finish_mutation(&mutation);
                return Err(anyhow::Error::new(Self::pack_full_failure()));
            }
            let next_position = staged_index.map_or_else(
                || content.stickers.iter().map(|sticker| sticker.position).max().unwrap_or(-1) + 1,
                |index| content.stickers[index].position,
            );
            let retained = retained_attempt.as_ref();
            let optimistic = StickerItem::new(
                local_id.clone(),
                pack_id.clone(),
                retained
                    .map(|attempt| attempt.upload_result.file_url.clone())
                    .unwrap_or_else(|| draft.local_url.absolute_string().to_owned()),
                retained.and_then(|attempt| attempt.remote_thumbnail_url.clone()).or_else(|| {
                    draft.thumbnail_url.as_ref().map(|url| url.absolute_string().to_owned())
                }),
                retained.and_then(|attempt| attempt.create_request.width).or(draft.width).unwrap_or(0).max(0),
                retained.and_then(|attempt| attempt.create_request.height).or(draft.height).unwrap_or(0).max(0),
                retained.and_then(|attempt| attempt.create_request.duration_ms).or(draft.duration_ms),
                next_position,
                now_ms(),
                emojis.clone(),
                retained.map_or(resolved_asset_type, |attempt| attempt.canonical_asset_type),
                retained
                    .and_then(|attempt| attempt.create_request.mime_type.clone())
                    .or_else(|| Some(mime_type.clone())),
                retained.and_then(|attempt| attempt.create_request.codec.clone()).or_else(|| codec.clone()),
            );
            let mut optimistic_content = content;
            match staged_index {
                Some(index) => optimistic_content.stickers[index] = optimistic,
                None => optimistic_content.stickers.push(optimistic),
            }
            s.state.packs.insert(pack_id.clone(), optimistic_content.clone());
            let has_retained = retained.is_some();
            s.state.sticker_uploads.insert(
                local_id.clone(),
                RiseStickerUploadPresentation::new(
                    request_id.clone(),
                    pack_id.clone(),
                    local_id.clone(),
                    if has_retained { RiseStickerMutationState::Submitting } else { RiseStickerMutationState::Preparing },
                    if has_retained { 1.0 } else { 0.0 },
                    if has_retained { draft_bytes } else { 0 },
                    draft_bytes,
                ),
            );
            Ok((generation, s.lifecycle_generation, optimistic_content))
        })?;
        self.publish().await;

        let rollback = RiseStickerDefer::new(|| {
            self.state.with(|s| {
                s.rollback_optimistic_sticker(&local_id, &pack_id);
                s.fail_mutation(&mutation, mutation_generation, &cancelled());
            });
            self.publish_detached();
        });
        let accepts = || {
            self.state.with(|s| s.accepts_mutation(&mutation, mutation_generation, expected_lifecycle_generation))
        };
        let result: anyhow::Result<StickerItem> = async {
            let Some(account_epoch) = self.state.with(|s| s.account_epoch.clone()) else {
                return Err(cancelled());
            };
            self.prepare_upload_authentication(&account_epoch).await?;
            self.validate_upload_context(&account_epoch, self.session_id, expected_lifecycle_generation)?;

            let attempt = if let Some(retained_attempt) = retained_attempt.clone() {
                if !accepts() {
                    return Err(cancelled());
                }
                retained_attempt
            } else {
                let upload_client = self.state.with(|s| {
                    let client = s.upload_client.clone()?;
                    s.state.mutation_states.insert(mutation.clone(), RiseStickerMutationState::Uploading);
                    s.update_sticker_upload_phase(RiseStickerMutationState::Uploading, &local_id);
                    Some(client)
                });
                let Some(upload_client) = upload_client else { return Err(cancelled()) };
                self.publish().await;

                let Some(file_path) = draft.local_url.to_file_path() else {
                    return Err(anyhow::Error::new(RiseStickerRepositoryError::InvalidMediaDraft));
                };
                let mut upload_request = RiseChunkedUploadRequest::new(
                    file_path,
                    filename.clone(),
                    mime_type.clone(),
                    file_context(target_kind),
                );
                upload_request.video_width = draft.width;
                upload_request.video_height = draft.height;
                upload_request.video_duration = video_duration_seconds(draft.duration_ms);
                let upload_result = upload_client
                    .upload(
                        upload_request,
                        None,
                        None,
                        Some(self.upload_progress_handler(&mutation, mutation_generation, &local_id)),
                    )
                    .await?;
                if !accepts() {
                    return Err(cancelled());
                }

                let remote_thumbnail_url = upload_result
                    .thumbnail_url
                    .clone()
                    .or_else(|| Self::remote_url_string(draft.thumbnail_url.as_ref()));
                let completion_asset_type =
                    Self::metadata_string(&upload_result.metadata, &[rise_key::stickers::ASSET_TYPE]);
                let canonical_asset_type = completion_asset_type.as_deref().map_or(resolved_asset_type, |value| {
                    StickerAssetType::infer(Some(value), &upload_result.file_url)
                });
                if canonical_asset_type == StickerAssetType::Lottie {
                    LottieStickerLoader::shared()
                        .cache_uploaded_sticker(&draft.local_url, &upload_result.file_url)
                        .await;
                    if !accepts() {
                        return Err(cancelled());
                    }
                }
                let completion_duration_ms =
                    Self::metadata_int(&upload_result.metadata, &[rise_key::stickers::DURATION_MS]);
                let legacy_duration_ms = upload_result.duration.map(|value| value * 1_000).or(draft.duration_ms);
                let canonical_duration_ms = completion_duration_ms.or(legacy_duration_ms);
                let completion_codec = Self::metadata_string(&upload_result.metadata, &[rise_key::stickers::CODEC]);
                let canonical_codec = completion_codec.clone().or_else(|| codec.clone());
                let create_request = rise_request::stickers::CreateSticker {
                    asset_id: Some(upload_result.asset_id.clone()),
                    asset_type: Some(canonical_asset_type.wire().to_owned()),
                    codec: canonical_codec,
                    duration_ms: canonical_duration_ms,
                    emojis: Some(emojis.clone()),
                    file_url: Some(upload_result.file_url.clone()),
                    height: Some(upload_result.height.or(draft.height).unwrap_or(0).max(0)),
                    mime_type: Some(if upload_result.mime_type.is_empty() {
                        mime_type.clone()
                    } else {
                        upload_result.mime_type.clone()
                    }),
                    pack_id: Some(pack_id.clone()),
                    request_id: Some(request_id.clone()),
                    thumbnail_url: remote_thumbnail_url.clone(),
                    width: Some(upload_result.width.or(draft.width).unwrap_or(0).max(0)),
                };
                let attempt = RiseStickerPendingCreateAttempt {
                    pack_id: pack_id.clone(),
                    draft: draft.clone(),
                    emojis: emojis.clone(),
                    requested_asset_type: asset_type.map(|value| value.wire().to_owned()),
                    requested_codec: codec.clone(),
                    upload_result,
                    create_request,
                    remote_thumbnail_url,
                    completion_asset_type,
                    canonical_asset_type,
                    completion_duration_ms,
                    legacy_duration_ms,
                    completion_codec,
                };
                self.state.with(|s| {
                    s.pending_sticker_create_attempts.insert(request_id.clone(), attempt.clone());
                });
                attempt
            };

            self.state.with(|s| {
                s.state.mutation_states.insert(mutation.clone(), RiseStickerMutationState::Submitting);
                s.state.mutation_progress.insert(mutation.clone(), 1.0);
                s.update_sticker_upload_phase(RiseStickerMutationState::Submitting, &local_id);
            });
            self.publish().await;

            let response: anyhow::Result<rise_response::stickers::CreateSticker> = self
                .rpc
                .call(RiseConversationEndpoints::create_sticker(), attempt.create_request.clone())
                .await;
            let response = match response {
                Ok(response) => response,
                Err(error) if is_cancellation(&error) => return Err(cancelled()),
                Err(error) => {
                    if let Some(recovered) =
                        self.recover_sticker_after_ambiguous_create(&pack_id, &attempt.upload_result.file_url).await
                    {
                        if !accepts() {
                            return Err(cancelled());
                        }
                        return Ok(self
                            .finish_created_sticker(
                                recovered,
                                &local_id,
                                &pack_id,
                                &optimistic_content,
                                &mutation,
                                &request_id,
                            )
                            .await);
                    }
                    return Err(error);
                }
            };
            if !accepts() {
                return Err(cancelled());
            }
            let Some(wire) = response else {
                return Err(rejected("Sticker response is missing the sticker"));
            };
            let created_url = trimmed(&wire.file_url);
            let created_file_url =
                if created_url.is_empty() { attempt.upload_result.file_url.clone() } else { created_url };
            let resolved_type = if attempt.completion_asset_type.is_some() || wire.asset_type.is_empty() {
                attempt.canonical_asset_type
            } else {
                StickerAssetType::infer(Some(&wire.asset_type), &created_file_url)
            };
            let created = StickerItem::new(
                wire.id.clone(),
                pack_id.clone(),
                created_file_url,
                wire.thumbnail_url.clone().or_else(|| attempt.remote_thumbnail_url.clone()),
                wire.width.max(0),
                wire.height.max(0),
                attempt.completion_duration_ms.or(wire.duration_ms).or(attempt.legacy_duration_ms),
                wire.position,
                wire.created_at_ms,
                wire.associated_emojis.clone(),
                resolved_type,
                wire.mime_type.clone().or_else(|| {
                    Some(if attempt.upload_result.mime_type.is_empty() {
                        mime_type.clone()
                    } else {
                        attempt.upload_result.mime_type.clone()
                    })
                }),
                attempt.completion_codec.clone().or_else(|| wire.codec.clone()).or_else(|| codec.clone()),
            );
            if created.id.is_empty() || created.file_url.is_empty() {
                return Err(rejected("Sticker response is incomplete"));
            }

            anyhow::Ok(
                self.finish_created_sticker(created, &local_id, &pack_id, &optimistic_content, &mutation, &request_id)
                    .await,
            )
        }
        .await;
        rollback.disarm();
        match result {
            Ok(created) => Ok(created),
            Err(error) => {
                self.state.with(|s| {
                    s.rollback_optimistic_sticker(&local_id, &pack_id);
                    s.fail_mutation(&mutation, mutation_generation, &error);
                });
                self.publish().await;
                Err(error)
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn edit_sticker(
        &self,
        raw_pack_id: &str,
        raw_sticker_id: &str,
        draft: Option<StickerMediaUploadDraft>,
        emojis: Vec<String>,
        requested_asset_type: Option<StickerAssetType>,
        requested_codec: Option<String>,
        raw_request_id: &str,
    ) -> anyhow::Result<StickerItem> {
        self.wait_for_hydration().await;
        let pack_id = trimmed(raw_pack_id);
        let sticker_id = trimmed(raw_sticker_id);
        let request_id = trimmed(raw_request_id);
        if pack_id.is_empty() || sticker_id.is_empty() || request_id.is_empty() || emojis.is_empty() {
            return Err(anyhow::Error::new(RiseStickerRepositoryError::InvalidMediaDraft));
        }

        let target_pack = self.resolved_pack_loading(&pack_id, true).await;
        let original = self.state.with(|s| s.sticker_in_pack(&pack_id, &sticker_id));
        let (Some(target_pack), Some(original)) = (target_pack, original) else {
            return Err(rejected("Only a pack owner or administrator can edit stickers"));
        };
        if !target_pack.can_edit_stickers() {
            return Err(rejected("Only a pack owner or administrator can edit stickers"));
        }

        if let Some(draft) = &draft {
            let exists = draft.local_url.to_file_path().is_some_and(|path| FileManager.file_exists(&path));
            if !draft.local_url.is_file_url()
                || !exists
                || trimmed(&draft.filename).is_empty()
                || trimmed(&draft.mime_type).is_empty()
            {
                return Err(anyhow::Error::new(RiseStickerRepositoryError::InvalidMediaDraft));
            }
        }

        let mutation = RiseStickerMutationID::EditSticker { sticker_id: sticker_id.clone() };
        let draft_bytes = draft.as_ref().map_or(0, |draft| Self::local_file_size(&draft.local_url));
        let (mutation_generation, expected_lifecycle_generation) = self.state.with(|s| {
            let generation = s.begin_mutation(&mutation)?;
            s.state.sticker_uploads.entry(sticker_id.clone()).or_insert_with(|| {
                RiseStickerUploadPresentation::new(
                    request_id.clone(),
                    pack_id.clone(),
                    sticker_id.clone(),
                    RiseStickerMutationState::Preparing,
                    0.0,
                    0,
                    draft_bytes,
                )
            });
            anyhow::Ok((generation, s.lifecycle_generation))
        })?;
        self.publish().await;

        let rollback = RiseStickerDefer::new(|| {
            self.state.with(|s| {
                s.state.sticker_uploads.remove(&sticker_id);
                s.fail_mutation(&mutation, mutation_generation, &cancelled());
            });
            self.publish_detached();
        });
        let accepts = || {
            self.state.with(|s| s.accepts_mutation(&mutation, mutation_generation, expected_lifecycle_generation))
        };
        let result: anyhow::Result<StickerItem> = async {
            let mut file_url: Option<String> = None;
            let mut asset_id: Option<String> = None;
            let mut width = Some(original.width);
            let mut height = Some(original.height);
            let mut asset_type = Some(original.asset_type);
            let mut duration_ms = original.duration_ms;
            let mut thumbnail_url = original.thumbnail_url.clone();
            let mut mime_type = original.mime_type.clone();
            let mut codec = original.codec.clone();

            if let Some(draft) = &draft {
                let Some(account_epoch) = self.state.with(|s| s.account_epoch.clone()) else {
                    return Err(cancelled());
                };
                self.prepare_upload_authentication(&account_epoch).await?;
                self.validate_upload_context(&account_epoch, self.session_id, expected_lifecycle_generation)?;
                let upload_client = self.state.with(|s| {
                    let client = s.upload_client.clone()?;
                    s.state.mutation_states.insert(mutation.clone(), RiseStickerMutationState::Uploading);
                    s.update_sticker_upload_phase(RiseStickerMutationState::Uploading, &sticker_id);
                    Some(client)
                });
                let Some(upload_client) = upload_client else { return Err(cancelled()) };
                self.publish().await;

                let resolved_asset_type =
                    requested_asset_type.unwrap_or_else(|| StickerAssetType::infer(None, &draft.filename));
                let Some(file_path) = draft.local_url.to_file_path() else {
                    return Err(anyhow::Error::new(RiseStickerRepositoryError::InvalidMediaDraft));
                };
                let mut upload_request = RiseChunkedUploadRequest::new(
                    file_path,
                    draft.filename.clone(),
                    draft.mime_type.clone(),
                    file_context(target_pack.pack_kind),
                );
                upload_request.video_width = draft.width;
                upload_request.video_height = draft.height;
                upload_request.video_duration = video_duration_seconds(draft.duration_ms);
                let upload_result = upload_client
                    .upload(
                        upload_request,
                        None,
                        None,
                        Some(self.upload_progress_handler(&mutation, mutation_generation, &sticker_id)),
                    )
                    .await?;
                if !accepts() {
                    return Err(cancelled());
                }

                let completion_asset_type =
                    Self::metadata_string(&upload_result.metadata, &[rise_key::stickers::ASSET_TYPE]);
                file_url = Some(upload_result.file_url.clone());
                asset_id = Some(upload_result.asset_id.clone());
                width = Some(upload_result.width.or(draft.width).unwrap_or(original.width).max(0));
                height = Some(upload_result.height.or(draft.height).unwrap_or(original.height).max(0));
                let uploaded_asset_type = completion_asset_type.as_deref().map_or(resolved_asset_type, |value| {
                    StickerAssetType::infer(Some(value), &upload_result.file_url)
                });
                asset_type = Some(uploaded_asset_type);
                if uploaded_asset_type == StickerAssetType::Lottie {
                    LottieStickerLoader::shared()
                        .cache_uploaded_sticker(&draft.local_url, &upload_result.file_url)
                        .await;
                    if !accepts() {
                        return Err(cancelled());
                    }
                }
                duration_ms = Self::metadata_int(&upload_result.metadata, &[rise_key::stickers::DURATION_MS])
                    .or_else(|| upload_result.duration.map(|value| value * 1_000))
                    .or(draft.duration_ms);
                thumbnail_url = upload_result
                    .thumbnail_url
                    .clone()
                    .or_else(|| Self::remote_url_string(draft.thumbnail_url.as_ref()));
                mime_type = Some(if upload_result.mime_type.is_empty() {
                    draft.mime_type.clone()
                } else {
                    upload_result.mime_type.clone()
                });
                codec = Self::metadata_string(&upload_result.metadata, &[rise_key::stickers::CODEC])
                    .or_else(|| requested_codec.clone());
            }

            self.state.with(|s| {
                s.state.mutation_states.insert(mutation.clone(), RiseStickerMutationState::Submitting);
                s.state.mutation_progress.insert(mutation.clone(), 1.0);
                s.update_sticker_upload_phase(RiseStickerMutationState::Submitting, &sticker_id);
            });
            self.publish().await;

            let request = rise_request::stickers::EditSticker {
                asset_id,
                asset_type: asset_type.map(|value| value.wire().to_owned()),
                codec: codec.clone(),
                duration_ms,
                emojis: Some(emojis.clone()),
                file_url: file_url.clone(),
                height,
                mime_type: mime_type.clone(),
                premium_effect_url: None,
                request_id: Some(request_id.clone()),
                sticker_id: Some(sticker_id.clone()),
                thumbnail_url: thumbnail_url.clone(),
                width,
            };
            let response: anyhow::Result<rise_response::stickers::CreateSticker> =
                self.rpc.call(RiseConversationEndpoints::edit_sticker(), request).await;
            let response = match response {
                Ok(response) => response,
                Err(error) if is_cancellation(&error) => return Err(cancelled()),
                Err(error) => {
                    if let Some(recovered) = self
                        .recover_sticker_after_ambiguous_edit(&pack_id, &sticker_id, file_url.as_deref(), &emojis)
                        .await
                    {
                        if !accepts() {
                            return Err(cancelled());
                        }
                        return Ok(self.finish_edited_sticker(recovered, &pack_id, &mutation).await);
                    }
                    return Err(error);
                }
            };
            if !accepts() {
                return Err(cancelled());
            }
            let Some(wire) = response else {
                return Err(rejected("Sticker response is missing the edited sticker"));
            };
            let response_url = trimmed(&wire.file_url);
            let edited_file_url = if response_url.is_empty() {
                file_url.clone().unwrap_or_else(|| original.file_url.clone())
            } else {
                response_url
            };
            let resolved_type = if wire.asset_type.is_empty() {
                asset_type.unwrap_or(original.asset_type)
            } else {
                StickerAssetType::infer(Some(&wire.asset_type), &edited_file_url)
            };
            let edited = StickerItem::new(
                wire.id.clone(),
                wire.pack_id.clone(),
                edited_file_url,
                wire.thumbnail_url.clone().or(thumbnail_url),
                wire.width.max(0),
                wire.height.max(0),
                wire.duration_ms.or(duration_ms),
                wire.position,
                wire.created_at_ms,
                wire.associated_emojis.clone(),
                resolved_type,
                wire.mime_type.clone().or(mime_type),
                wire.codec.clone().or(codec),
            );
            if edited.id != sticker_id || edited.file_url.is_empty() {
                return Err(rejected("Edited sticker response is incomplete"));
            }
            anyhow::Ok(self.finish_edited_sticker(edited, &pack_id, &mutation).await)
        }
        .await;
        rollback.disarm();
        match result {
            Ok(edited) => Ok(edited),
            Err(error) => {
                self.state.with(|s| {
                    s.state.sticker_uploads.remove(&sticker_id);
                    s.fail_mutation(&mutation, mutation_generation, &error);
                });
                self.publish().await;
                Err(error)
            }
        }
    }

    pub async fn delete_sticker(&self, raw_pack_id: &str, raw_sticker_id: &str) -> anyhow::Result<()> {
        self.wait_for_hydration().await;
        let pack_id = trimmed(raw_pack_id);
        let sticker_id = trimmed(raw_sticker_id);
        if pack_id.is_empty() || sticker_id.is_empty() {
            return Ok(());
        }
        let pack = self.resolved_pack_loading(&pack_id, true).await;
        let allowed = pack.as_ref().is_some_and(StickerPack::can_delete_stickers)
            && self.state.with(|s| s.contains_sticker(&pack_id, &sticker_id));
        let (true, Some(pack)) = (allowed, pack) else {
            return Err(rejected("Only a pack owner or administrator can delete stickers"));
        };

        let mutation = RiseStickerMutationID::DeleteSticker { sticker_id: sticker_id.clone() };
        let (generation, lifecycle, rollback_snapshot, fenced_link_loads) = self.state.with(|s| {
            let generation = s.begin_mutation(&mutation)?;
            let lifecycle = s.lifecycle_generation;
            let rollback_snapshot = s.mutation_rollback_snapshot(&pack_id, pack.pack_kind);
            let fenced = s.fence_pack_mutation_loads(&pack_id, pack.pack_kind);
            s.remove_sticker_everywhere(&sticker_id, &pack_id);
            s.synchronize_pack_preview(&pack_id);
            s.state.mutation_states.insert(mutation.clone(), RiseStickerMutationState::Submitting);
            anyhow::Ok((generation, lifecycle, rollback_snapshot, fenced))
        })?;
        self.publish().await;

        let rollback = RiseStickerDefer::new(|| {
            self.state.with(|s| {
                s.restore_mutation_rollback_snapshot(&rollback_snapshot);
                s.fail_mutation(&mutation, generation, &cancelled());
            });
            self.publish_detached();
        });
        let result: anyhow::Result<()> = async {
            let _: rise_response::stickers::DeleteSticker = self
                .rpc
                .call(
                    RiseConversationEndpoints::delete_sticker(),
                    rise_request::stickers::DeleteSticker { sticker_id: Some(sticker_id.clone()) },
                )
                .await?;
            let accepted = self.state.with(|s| {
                if !s.accepts_mutation(&mutation, generation, lifecycle) {
                    return false;
                }
                s.finish_mutation(&mutation);
                true
            });
            if !accepted {
                return Err(cancelled());
            }
            self.publish().await;
            self.wait_until_resources_are_idle(&fenced_link_loads).await;
            self.persist_sticker_mutation(&pack_id, pack.pack_kind).await;
            anyhow::Ok(())
        }
        .await;
        rollback.disarm();
        if let Err(error) = result {
            self.state.with(|s| {
                s.restore_mutation_rollback_snapshot(&rollback_snapshot);
                s.fail_mutation(&mutation, generation, &error);
            });
            self.publish().await;
            return Err(error);
        }
        Ok(())
    }

    pub async fn reorder_stickers(&self, raw_pack_id: &str, raw_sticker_ids: &[String]) -> anyhow::Result<()> {
        self.wait_for_hydration().await;
        let pack_id = trimmed(raw_pack_id);
        let sticker_ids: Vec<String> = raw_sticker_ids.iter().map(|value| trimmed(value)).collect();
        if pack_id.is_empty() {
            return Ok(());
        }
        let pack = self.resolved_pack_loading(&pack_id, true).await;
        let current = self.state.with(|s| s.state.packs.get(&pack_id).map(|content| content.stickers.clone()));
        let (Some(pack), Some(current)) = (pack, current) else {
            return Err(rejected("Only a pack owner or administrator can reorder stickers"));
        };
        if !pack.can_edit_stickers() {
            return Err(rejected("Only a pack owner or administrator can reorder stickers"));
        }
        let current_ids: Vec<&String> = current.iter().map(|sticker| &sticker.id).collect();
        if sticker_ids.iter().collect::<HashSet<_>>() != current_ids.iter().copied().collect::<HashSet<_>>()
            || sticker_ids.len() != current.len()
        {
            return Err(rejected("Sticker order no longer matches the pack"));
        }
        if sticker_ids.iter().collect::<Vec<_>>() == current_ids {
            return Ok(());
        }

        let mutation = RiseStickerMutationID::ReorderStickers { pack_id: pack_id.clone() };
        let (generation, lifecycle, rollback_snapshot, fenced_link_loads) = self.state.with(|s| {
            let generation = s.begin_mutation(&mutation)?;
            let lifecycle = s.lifecycle_generation;
            let rollback_snapshot = s.mutation_rollback_snapshot(&pack_id, pack.pack_kind);
            let fenced = s.fence_pack_mutation_loads(&pack_id, pack.pack_kind);
            s.apply_sticker_order(&sticker_ids, &pack_id);
            s.synchronize_pack_preview(&pack_id);
            s.state.mutation_states.insert(mutation.clone(), RiseStickerMutationState::Submitting);
            anyhow::Ok((generation, lifecycle, rollback_snapshot, fenced))
        })?;
        self.publish().await;

        let rollback = RiseStickerDefer::new(|| {
            self.state.with(|s| {
                s.restore_mutation_rollback_snapshot(&rollback_snapshot);
                s.fail_mutation(&mutation, generation, &cancelled());
            });
            self.publish_detached();
        });
        let result: anyhow::Result<()> = async {
            let _: rise_response::stickers::ReorderStickers = self
                .rpc
                .call(
                    RiseConversationEndpoints::reorder_stickers(),
                    rise_request::stickers::ReorderStickers {
                        pack_id: Some(pack_id.clone()),
                        sticker_ids: Some(sticker_ids.clone()),
                    },
                )
                .await?;
            let accepted = self.state.with(|s| {
                if !s.accepts_mutation(&mutation, generation, lifecycle) {
                    return false;
                }
                s.finish_mutation(&mutation);
                true
            });
            if !accepted {
                return Err(cancelled());
            }
            self.publish().await;
            self.wait_until_resources_are_idle(&fenced_link_loads).await;
            self.persist_sticker_mutation(&pack_id, pack.pack_kind).await;
            anyhow::Ok(())
        }
        .await;
        rollback.disarm();
        if let Err(error) = result {
            self.state.with(|s| {
                s.restore_mutation_rollback_snapshot(&rollback_snapshot);
                s.fail_mutation(&mutation, generation, &error);
            });
            self.publish().await;
            return Err(error);
        }
        Ok(())
    }

    pub async fn reorder_saved_packs(&self, kind: PackKind, raw_pack_ids: &[String]) -> anyhow::Result<()> {
        self.wait_for_hydration().await;
        let prepared = self.state.with(|s| {
            let old_page = s.state.saved_packs.get(&kind)?.clone();
            let mut reorderable: HashMap<String, StickerPack> = HashMap::new();
            for pack in old_page.items.iter().filter(|pack| !pack.is_default) {
                reorderable.entry(pack.id.clone()).or_insert_with(|| pack.clone());
            }
            let mut seen: HashSet<String> = HashSet::new();
            let pack_ids: Vec<String> = raw_pack_ids
                .iter()
                .map(|value| trimmed(value))
                .filter(|id| reorderable.contains_key(id) && seen.insert(id.clone()))
                .collect();
            if pack_ids.is_empty() {
                return None;
            }
            let current_order: Vec<&String> = old_page
                .items
                .iter()
                .filter(|pack| !pack.is_default)
                .map(|pack| &pack.id)
                .take(pack_ids.len())
                .collect();
            if pack_ids.iter().collect::<Vec<_>>() == current_order {
                return None;
            }
            let mut page = old_page.clone();
            page.items = pack_ids
                .iter()
                .filter_map(|id| reorderable.get(id).cloned())
                .chain(old_page.items.iter().filter(|pack| !seen.contains(&pack.id)).cloned())
                .collect();
            s.state.saved_packs.insert(kind, page.clone());
            Some((old_page, page, pack_ids))
        });
        let Some((old_page, page, pack_ids)) = prepared else { return Ok(()) };
        self.publish().await;

        let restore = || {
            self.state.with(|s| {
                if s.state.saved_packs.get(&kind) != Some(&page) {
                    return false;
                }
                s.state.saved_packs.insert(kind, old_page.clone());
                true
            })
        };
        let rollback = RiseStickerDefer::new(|| {
            if restore() {
                self.publish_detached();
            }
        });
        let result: anyhow::Result<rise_response::stickers::ReorderSavedPacks> = self
            .rpc
            .call(
                RiseConversationEndpoints::reorder_saved_packs(),
                rise_request::stickers::ReorderSavedPacks { pack_ids: Some(pack_ids) },
            )
            .await;
        rollback.disarm();
        if let Err(error) = result {
            if restore() {
                self.publish().await;
            }
            return Err(error);
        }
        let _ = self.persist_pack_lists(kind).await;
        Ok(())
    }

    pub async fn discard_sticker_upload_attempt(&self, raw_request_id: &str) {
        let request_id = trimmed(raw_request_id);
        if request_id.is_empty() {
            return;
        }
        self.state.with(|s| {
            s.pending_sticker_create_attempts.remove(&request_id);
        });
    }

    pub async fn cancel_sticker_upload(&self, raw_request_id: &str, raw_pack_id: &str) {
        let request_id = trimmed(raw_request_id);
        let pack_id = trimmed(raw_pack_id);
        if request_id.is_empty() || pack_id.is_empty() {
            return;
        }
        let found = self.state.with(|s| {
            let sticker_id = s
                .state
                .sticker_uploads
                .iter()
                .find(|(_, upload)| upload.request_id == request_id && upload.pack_id == pack_id)
                .map(|(sticker_id, _)| sticker_id.clone());
            s.pending_sticker_create_attempts.remove(&request_id);
            let sticker_id = sticker_id?;
            let is_create = sticker_id.starts_with(LOCAL_STICKER_PREFIX);
            let mutation = if is_create {
                RiseStickerMutationID::AddSticker { pack_id: pack_id.clone() }
            } else {
                RiseStickerMutationID::EditSticker { sticker_id: sticker_id.clone() }
            };
            let generation = s.mutation_generations.entry(mutation.clone()).or_default();
            *generation = generation.wrapping_add(1);
            if is_create {
                s.rollback_optimistic_sticker(&sticker_id, &pack_id);
            } else {
                s.state.sticker_uploads.remove(&sticker_id);
            }
            s.finish_mutation(&mutation);
            Some(())
        });
        if found.is_some() {
            self.publish().await;
        }
    }

    pub async fn clear_mutation_error(&self, mutation: &RiseStickerMutationID) {
        let cleared = self.state.with(|s| {
            if s.state.mutation_states.get(mutation) != Some(&RiseStickerMutationState::Failed) {
                return false;
            }
            s.finish_mutation(mutation);
            true
        });
        if cleared {
            self.publish().await;
        }
    }

    fn schedule_picker_refresh(&self, generation: u64, force_refresh: bool) {
        let tab = self.state.with(|s| {
            if let Some(task) = s.picker_refresh_task.take() {
                task.abort();
            }
            s.active_picker_tab
        });
        let this = self.this.clone();
        let handle = RiseAccountRuntime::current().spawn(async move {
            tokio::time::sleep(Duration::from_millis(PICKER_REFRESH_DELAY_MS)).await;
            if let Some(repository) = this.upgrade() {
                repository.refresh_picker_tab(tab, generation, force_refresh).await;
            }
        });
        self.state.with(|s| {
            if let Some(task) = s.picker_refresh_task.replace(handle) {
                task.abort();
            }
        });
    }

    async fn refresh_picker_tab(&self, tab: RiseStickerPickerTab, generation: u64, force_refresh: bool) {
        if !self.state.with(|s| s.is_picker_current(tab, generation)) {
            return;
        }
        match tab {
            RiseStickerPickerTab::Stickers => {
                futures::join!(
                    self.load_favourites(force_refresh),
                    self.load_recents(PackKind::Sticker, force_refresh),
                    self.load_saved_packs(PackKind::Sticker, None, force_refresh),
                    self.load_default_packs(PackKind::Sticker, None, force_refresh),
                );
                if self.state.with(|s| s.picker_visible && s.picker_generation == generation) {
                    self.prefetch_initial_packs(PackKind::Sticker, force_refresh).await;
                }
            }
            RiseStickerPickerTab::Emoji => {
                futures::join!(
                    self.load_recents(PackKind::Emoji, force_refresh),
                    self.load_saved_packs(PackKind::Emoji, None, force_refresh),
                    self.load_default_packs(PackKind::Emoji, None, force_refresh),
                );
                if self.state.with(|s| s.picker_visible && s.picker_generation == generation) {
                    self.prefetch_initial_packs(PackKind::Emoji, force_refresh).await;
                }
            }
            RiseStickerPickerTab::Gifs => {
                self.load_gif_favourites(None, force_refresh).await;
                if force_refresh {
                    let failed_query = self.state.with(|s| {
                        s.last_gif_search_query.clone().filter(|query| {
                            s.state.load_states.get(&RiseStickerResourceID::GifSearch(query.clone()))
                                == Some(&RiseStickerLoadState::Failed)
                        })
                    });
                    if let Some(query) = failed_query {
                        self.search_gifs(&query, None, true).await;
                    }
                }
            }
        }
    }

    /// a pack whose load failed offline used up its refresh interval, so a reconnect refresh forces it
    /// or its section keeps the skeleton after the server is back
    async fn prefetch_initial_packs(&self, kind: PackKind, retrying_failures: bool) {
        let (ids, forced) = self.state.with(|s| {
            let saved = page_items(s.state.saved_packs.get(&kind));
            let defaults = page_items(s.state.default_packs.get(&kind));
            let mut ids = RiseStickerPerformancePolicy::initial_pack_ids(saved, defaults);
            let is_failed = |id: &str| {
                s.state.load_states.get(&RiseStickerResourceID::Pack(id.to_owned()))
                    == Some(&RiseStickerLoadState::Failed)
            };
            if retrying_failures {
                let mut seen: HashSet<String> = ids.iter().cloned().collect();
                for pack in saved.iter().chain(defaults.iter()) {
                    if is_failed(&pack.id) && seen.insert(pack.id.clone()) {
                        ids.push(pack.id.clone());
                    }
                }
            }
            let forced: HashSet<String> = if retrying_failures {
                ids.iter().filter(|id| !s.state.packs.contains_key(*id) || is_failed(id)).cloned().collect()
            } else {
                HashSet::new()
            };
            (ids, forced)
        });
        futures::future::join_all(ids.iter().map(|id| self.load_pack(id, forced.contains(id)))).await;
    }

    async fn trace_picker_cache_hit(&self, tab: RiseStickerPickerTab) {
        let source = self.state.with(|s| {
            let has_recents = |kind: PackKind| s.state.recents.get(&kind).is_some_and(|items| !items.is_empty());
            let has_saved = |kind: PackKind| !page_items(s.state.saved_packs.get(&kind)).is_empty();
            let has_defaults = |kind: PackKind| !page_items(s.state.default_packs.get(&kind)).is_empty();
            let has_cached_presentation = match tab {
                RiseStickerPickerTab::Stickers => {
                    !s.state.favourites.is_empty()
                        || has_recents(PackKind::Sticker)
                        || has_saved(PackKind::Sticker)
                        || has_defaults(PackKind::Sticker)
                }
                RiseStickerPickerTab::Emoji => {
                    has_recents(PackKind::Emoji) || has_saved(PackKind::Emoji) || has_defaults(PackKind::Emoji)
                }
                RiseStickerPickerTab::Gifs => !s.state.gif_favourites.items.is_empty(),
            };
            if !has_cached_presentation && !s.cached_presentation_tabs.contains(&tab) {
                return None;
            }
            Some(if s.cold_persistent_tabs.remove(&tab) {
                RiseConversationResourceReadSource::PersistentStore
            } else {
                RiseConversationResourceReadSource::Memory
            })
        });
        let Some(source) = source else { return };
        RiseConversationDebugTrace::record_resource_cache_hit(
            "RiseStickerRepository",
            &format!("picker.{}", tab.wire()),
            source,
        )
        .await;
    }

    async fn hydrate_from_store(&self) {
        if self.state.with(|s| s.is_shutdown) {
            return;
        }
        let result: anyhow::Result<()> = async {
            for kind in [PackKind::Sticker, PackKind::Emoji] {
                let tab = RiseStickerRepositoryState::picker_tab(kind);
                let defaults = self.resources.cached::<GetPacksGatewayDTO>(&Self::default_packs_key(kind)).await?;
                if let Some(value) = &defaults.value {
                    self.state.with(|s| {
                        s.state.default_packs.insert(kind, Self::pack_page(value));
                        s.note_cache_hydration(defaults.read_source, tab);
                    });
                }
                let saved = self.resources.cached::<GetSavedPacksGatewayDTO>(&Self::saved_packs_key(kind)).await?;
                if let Some(value) = &saved.value {
                    self.state.with(|s| {
                        s.state.saved_packs.insert(kind, Self::saved_pack_page(value));
                        s.note_cache_hydration(saved.read_source, tab);
                    });
                }
                let mine = self.resources.cached::<GetMyPacksGatewayDTO>(&Self::my_packs_key(kind)).await?;
                if let Some(value) = &mine.value {
                    self.state.with(|s| {
                        s.state.my_packs.insert(kind, Self::my_pack_page(value, kind));
                    });
                }
                let recents =
                    self.resources.cached::<rise_response::stickers::GetUserRecent>(&Self::recents_key(kind)).await?;
                if let Some(value) = &recents.value {
                    self.state.with(|s| {
                        s.state.recents.insert(kind, value.stickers.iter().map(|sticker| sticker.to_sticker()).collect());
                        s.note_cache_hydration(recents.read_source, tab);
                    });
                }
            }
            let favourites =
                self.resources.cached::<rise_response::stickers::GetUserFavourites>(&Self::favourites_key()).await?;
            if let Some(value) = &favourites.value {
                self.state.with(|s| {
                    s.state.favourites = value.stickers.iter().map(|sticker| sticker.to_sticker()).collect();
                    s.note_cache_hydration(favourites.read_source, RiseStickerPickerTab::Stickers);
                });
            }
            let gif_favourites = self.resources.cached::<GetGifFavouritesGatewayDTO>(&Self::gif_favourites_key()).await?;
            if let Some(value) = &gif_favourites.value {
                self.state.with(|s| {
                    s.state.gif_favourites = Self::gif_page(value);
                    s.note_cache_hydration(gif_favourites.read_source, RiseStickerPickerTab::Gifs);
                });
            }
            let manifest = self.resources.cached::<RiseStickerPackManifest>(&Self::pack_manifest_key()).await?;
            if let Some(manifest) = manifest.value {
                self.state.with(|s| {
                    s.persisted_pack_ids = manifest
                        .ids
                        .into_iter()
                        .take(limit_count(RiseStickerPerformancePolicy::persisted_pack_limit()))
                        .collect();
                });
            }
            for kind in [PackKind::Sticker, PackKind::Emoji] {
                let tab = RiseStickerRepositoryState::picker_tab(kind);
                let initial_ids = self.state.with(|s| {
                    RiseStickerPerformancePolicy::initial_pack_ids(
                        page_items(s.state.saved_packs.get(&kind)),
                        page_items(s.state.default_packs.get(&kind)),
                    )
                });
                for id in initial_ids {
                    self.hydrate_pack(&id, Some(tab)).await;
                }
            }
            anyhow::Ok(())
        }
        .await;
        if let Err(error) = &result
            && is_cancellation(error)
        {
            return;
        }
        self.publish().await;
    }

    async fn hydrate_pack(&self, id: &str, tab: Option<RiseStickerPickerTab>) -> bool {
        if self.state.with(|s| s.state.packs.contains_key(id)) {
            return false;
        }
        let Ok(snapshot) = self.resources.cached::<GetStickersByPackIdGatewayDTO>(&Self::pack_key(id)).await else {
            return false;
        };
        let Some(value) = &snapshot.value else { return false };
        self.state.with(|s| {
            s.state.packs.insert(id.to_owned(), Self::pack_content(value));
            if let Some(tab) = tab {
                s.note_cache_hydration(snapshot.read_source, tab);
            }
        });
        true
    }

    async fn hydrate_linked_pack(&self, link: &str) -> bool {
        if self.state.with(|s| s.state.linked_packs.contains_key(link)) {
            return false;
        }
        let Ok(snapshot) = self.resources.cached::<GetStickersByPackLinkGatewayDTO>(&Self::pack_link_key(link)).await
        else {
            return false;
        };
        let Some(value) = &snapshot.value else { return false };
        let content = Self::pack_content_link(value);
        let Some(pack) = content.pack.clone().filter(|pack| !pack.id.is_empty()) else { return false };
        self.state.with(|s| {
            s.state.linked_packs.insert(link.to_owned(), content.clone());
            s.state.packs.entry(pack.id.clone()).or_insert(content);
            s.note_cache_hydration(snapshot.read_source, RiseStickerRepositoryState::picker_tab(pack.pack_kind));
        });
        true
    }

    /// used by visible glyphs, never starts catalogue reads or picker refreshes
    pub async fn hydrate_cached_emoji_resources(&self) {
        self.wait_for_hydration().await;
    }

    /// resolves ce ids this account has in no pack, like another users premium badge
    /// one batch per call, at most custom_emoji_batch_limit ids
    /// server skipped ids are remembered as misses
    pub async fn resolve_custom_emoji(&self, raw_ids: &[String]) {
        if !self.state.with(|s| s.is_active && !s.is_shutdown) {
            return;
        }
        self.wait_for_hydration().await;
        let (generation, wanted) = self.state.with(|s| {
            let mut seen = HashSet::new();
            let wanted: Vec<String> = raw_ids
                .iter()
                .map(|raw| trimmed(raw))
                .filter(|id| {
                    !id.is_empty()
                        && seen.insert(id.clone())
                        && !s.state.custom_emoji.contains_key(id)
                        && !s.custom_emoji_in_flight.contains(id)
                        && !s.custom_emoji_misses.contains(id)
                })
                .collect();
            s.custom_emoji_in_flight.extend(wanted.iter().cloned());
            (s.lifecycle_generation, wanted)
        });
        if wanted.is_empty() {
            return;
        }
        let _in_flight = RiseStickerDefer::new(|| {
            self.state.with(|s| {
                for id in &wanted {
                    s.custom_emoji_in_flight.remove(id);
                }
            });
        });

        let mut resolved: Vec<StickerItem> = Vec::new();
        let mut answered: HashSet<String> = HashSet::new();
        let batch = limit_count(RiseStickerPerformancePolicy::custom_emoji_batch_limit()).max(1);
        for chunk in wanted.chunks(batch) {
            let response: anyhow::Result<GetCustomEmojiByIdsGatewayDTO> = self
                .rpc
                .call(
                    RiseConversationEndpoints::get_custom_emoji_by_ids(),
                    rise_request::stickers::GetCustomEmojiByIds { ids: chunk.to_vec() },
                )
                .await;
            match response {
                Ok(response) => {
                    if self.state.with(|s| s.is_shutdown || s.lifecycle_generation != generation) {
                        return;
                    }
                    resolved.extend(
                        response
                            .stickers
                            .iter()
                            .map(|sticker| sticker.to_sticker())
                            .filter(|item| chunk.contains(&item.id)),
                    );
                    answered.extend(chunk.iter().cloned());
                }
                Err(error) if is_cancellation(&error) => return,
                Err(_) => break,
            }
        }

        let any_found = self.state.with(|s| {
            if s.is_shutdown || s.lifecycle_generation != generation {
                return false;
            }
            let limit = limit_count(RiseStickerPerformancePolicy::custom_emoji_cache_limit()).max(1);
            let mut found: HashSet<String> = HashSet::new();
            for item in resolved {
                if found.insert(item.id.clone()) && s.state.custom_emoji.insert(item.id.clone(), item.clone()).is_none()
                {
                    s.custom_emoji_order.push(item.id);
                }
            }
            if s.custom_emoji_order.len() > limit {
                let overflow = s.custom_emoji_order.len() - limit;
                for id in s.custom_emoji_order.drain(..overflow) {
                    s.state.custom_emoji.remove(&id);
                }
            }
            s.custom_emoji_misses
                .extend(wanted.iter().filter(|id| answered.contains(*id) && !found.contains(*id)).cloned());
            if s.custom_emoji_misses.len() > limit {
                let overflow = s.custom_emoji_misses.len() - limit;
                s.custom_emoji_misses.drain(..overflow);
            }
            !found.is_empty()
        });
        if any_found {
            self.publish().await;
        }
    }

    async fn wait_for_hydration(&self) {
        let finished = self.state.with(|s| s.hydration_task.as_ref().map(|task| task.finished.clone()));
        if let Some(finished) = finished {
            let _ = finished.await;
        }
    }

    /// force requests invalidate older responses at once,
    /// a request already running gets one coalesced follow up
    /// that covers every force request seen before it starts
    async fn begin_coalesced(
        &self,
        resource: &RiseStickerResourceID,
        force: bool,
        bypass_throttle: bool,
    ) -> Option<RiseStickerResourceLease> {
        let requested_ticket = self.state.with(|s| {
            if s.is_shutdown {
                return Err(());
            }
            if force {
                let ticket = s.forced_reload_tickets.entry(resource.clone()).or_default();
                *ticket = ticket.wrapping_add(1);
                let ticket = *ticket;
                s.bump_generation(resource);
                s.refreshed_at.remove(resource);
                return Ok(Some(ticket));
            }
            let requested = s.forced_reload_tickets.get(resource).copied().unwrap_or(0);
            let completed = s.completed_forced_reload_tickets.get(resource).copied().unwrap_or(0);
            if requested > completed { Err(()) } else { Ok(None) }
        });
        let requested_ticket = requested_ticket.ok()?;

        loop {
            let step = self.state.with(|s| s.coalesced_step(resource, requested_ticket, force, bypass_throttle));
            let (idle, ticket) = match step {
                RiseStickerCoalescedStep::Finished(lease) => return lease,
                RiseStickerCoalescedStep::Wait(idle, ticket) => (idle, ticket),
            };
            let abandoned = RiseStickerDefer::new(|| {
                self.state.with(|s| s.complete_forced_ticket(resource, ticket));
            });
            let _ = idle.await;
            if self.state.with(|s| s.is_shutdown) {
                return None;
            }
            abandoned.disarm();
        }
    }

    fn end(&self, resource: &RiseStickerResourceID, lease: RiseStickerResourceLease) {
        self.state.with(|s| {
            s.in_flight.remove(resource);
            if let Some(ticket) = lease.forced_ticket {
                s.complete_forced_ticket(resource, ticket);
            }
            s.resume_resource_idle_waiters(resource);
        });
    }

    fn end_in_flight(&self, resource: &RiseStickerResourceID) {
        self.state.with(|s| {
            s.in_flight.remove(resource);
        });
    }

    async fn wait_until_resource_is_idle(&self, resource: &RiseStickerResourceID) {
        let idle = self.state.with(|s| {
            s.in_flight.contains(resource).then(|| {
                let (sender, receiver) = oneshot::channel::<()>();
                s.resource_idle_waiters.entry(resource.clone()).or_default().push(sender);
                receiver
            })
        });
        if let Some(idle) = idle {
            let _ = idle.await;
        }
    }

    async fn set_loading(&self, resource: &RiseStickerResourceID) {
        self.state.with(|s| {
            s.state.load_states.insert(resource.clone(), RiseStickerLoadState::Loading);
        });
        if matches!(resource, RiseStickerResourceID::GifSearch(_) | RiseStickerResourceID::MyPacks(_)) {
            self.publish().await;
        }
    }

    async fn fail(&self, resource: &RiseStickerResourceID) {
        let proceeded = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            s.state.load_states.insert(resource.clone(), RiseStickerLoadState::Failed);
            true
        });
        if proceeded
            && matches!(
                resource,
                RiseStickerResourceID::GifSearch(_) | RiseStickerResourceID::MyPacks(_) | RiseStickerResourceID::SavedPacks(_)
            )
        {
            self.publish().await;
        }
    }

    async fn publish(&self) {
        let Some(hop) = RiseEngineMainHop::installed() else { return };
        let queued = self.state.with(|s| {
            let value = s.publishable_snapshot()?;
            let account_id = self.account_id.clone();
            Some(hop.run(move |cx: AsyncApp| {
                async move {
                    cx.update(|cx| Self::publish_on_main(&account_id, value, cx));
                    Ok(())
                }
                .boxed_local()
            }))
        });
        if let Some(queued) = queued {
            let _ = queued.await;
        }
    }

    fn publish_detached(&self) {
        let Some(hop) = RiseEngineMainHop::installed() else { return };
        self.state.with(|s| {
            let Some(value) = s.publishable_snapshot() else { return };
            let account_id = self.account_id.clone();
            hop.post(move |cx| Self::publish_on_main(&account_id, value, cx));
        });
    }

    fn publish_on_main(account_id: &str, value: RiseStickerPresentationSnapshot, cx: &mut App) {
        let presentation = RiseStickerPresentation::shared(cx);
        if presentation.read(cx).snapshot().account_id != account_id {
            return;
        }
        presentation.update(cx, |presentation, cx| presentation.publish(value, cx));
    }

    async fn install_authoritative_pack(
        &self,
        wire: &GetStickersByPackIdGatewayDTO,
        pack_id: &str,
    ) -> anyhow::Result<()> {
        let mut content = Self::pack_content(wire);
        let has_pack = content.pack.is_some();
        let linked = self.state.with(|s| {
            s.preserve_uploading_stickers(&mut content, pack_id);
            let linked = s.linked_keys(pack_id);
            for link in &linked {
                let resource = RiseStickerResourceID::PackLink(link.clone());
                s.bump_generation(&resource);
                if has_pack {
                    s.state.linked_packs.insert(link.clone(), content.clone());
                } else {
                    s.state.linked_packs.remove(link);
                }
                s.mark_loaded(&resource);
            }
            if has_pack {
                s.state.packs.insert(pack_id.to_owned(), content.clone());
            } else {
                s.state.packs.remove(pack_id);
            }
            linked
        });
        if !has_pack {
            self.remove_persisted_pack(pack_id).await?;
            self.remove_persisted_linked_packs(&linked).await?;
            return Ok(());
        }
        self.resources.store(wire, &Self::pack_key(pack_id)).await?;
        self.persist_linked_packs(pack_id).await?;
        self.record_persisted_pack(pack_id).await
    }

    async fn record_persisted_pack(&self, id: &str) -> anyhow::Result<()> {
        let (evicted, ids) = self.state.with(|s| {
            s.persisted_pack_ids.retain(|value| value != id);
            s.persisted_pack_ids.insert(0, id.to_owned());
            let limit = limit_count(RiseStickerPerformancePolicy::persisted_pack_limit());
            let evicted = if s.persisted_pack_ids.len() > limit {
                s.persisted_pack_ids.split_off(limit)
            } else {
                Vec::new()
            };
            (evicted, s.persisted_pack_ids.clone())
        });
        for old_id in &evicted {
            self.resources.remove(&Self::pack_key(old_id)).await?;
        }
        self.resources.store(&RiseStickerPackManifest { ids }, &Self::pack_manifest_key()).await?;
        Ok(())
    }

    async fn remove_persisted_pack(&self, id: &str) -> anyhow::Result<()> {
        self.resources.remove(&Self::pack_key(id)).await?;
        let ids = self.state.with(|s| {
            let old_count = s.persisted_pack_ids.len();
            s.persisted_pack_ids.retain(|value| value != id);
            (s.persisted_pack_ids.len() != old_count).then(|| s.persisted_pack_ids.clone())
        });
        let Some(ids) = ids else { return Ok(()) };
        self.resources.store(&RiseStickerPackManifest { ids }, &Self::pack_manifest_key()).await?;
        Ok(())
    }

    async fn remove_persisted_linked_packs(&self, links: &[String]) -> anyhow::Result<()> {
        for link in links {
            self.resources.remove(&Self::pack_link_key(link)).await?;
        }
        Ok(())
    }

    async fn persist_favourites(&self) -> anyhow::Result<()> {
        let wire = self.state.with(|s| rise_response::stickers::GetUserFavourites {
            stickers: s.state.favourites.iter().map(StickerItem::to_wire_dto).collect(),
            ..Default::default()
        });
        self.resources.store(&wire, &Self::favourites_key()).await?;
        Ok(())
    }

    async fn persist_recents(&self, kind: PackKind) -> anyhow::Result<()> {
        let wire = self.state.with(|s| rise_response::stickers::GetUserRecent {
            stickers: s.state.recents.get(&kind).into_iter().flatten().map(StickerItem::to_wire_dto).collect(),
            ..Default::default()
        });
        self.resources.store(&wire, &Self::recents_key(kind)).await?;
        Ok(())
    }

    async fn persist_gif_favourites(&self) -> anyhow::Result<()> {
        let wire = self.state.with(|s| GetGifFavouritesGatewayDTO {
            gifs: s.state.gif_favourites.items.iter().map(GifItem::to_wire_dto).collect(),
            relative_id: s.state.gif_favourites.next_relative_id.clone(),
            has_more: s.state.gif_favourites.has_more,
        });
        self.resources.store(&wire, &Self::gif_favourites_key()).await?;
        Ok(())
    }

    async fn persist_pack_lists(&self, kind: PackKind) -> anyhow::Result<()> {
        let (defaults, saved) = self.state.with(|s| {
            let defaults = s.state.default_packs.get(&kind).cloned().unwrap_or_default();
            let saved = s.state.saved_packs.get(&kind).cloned().unwrap_or_default();
            (
                GetPacksGatewayDTO {
                    packs: defaults.items.iter().map(StickerPack::to_wire_dto).collect(),
                    total: defaults.items.len() as i64,
                    relative_id: defaults.next_relative_id,
                    has_more: defaults.has_more,
                },
                GetSavedPacksGatewayDTO {
                    packs: saved.items.iter().map(StickerPack::to_wire_dto).collect(),
                    relative_id: saved.next_relative_id,
                    has_more: saved.has_more,
                },
            )
        });
        self.resources.store(&defaults, &Self::default_packs_key(kind)).await?;
        self.resources.store(&saved, &Self::saved_packs_key(kind)).await?;
        Ok(())
    }

    async fn persist_my_packs(&self, kind: PackKind) -> anyhow::Result<()> {
        let wire = self.state.with(|s| {
            let page = s.state.my_packs.get(&kind).cloned().unwrap_or_default();
            GetMyPacksGatewayDTO {
                packs: page.items.iter().map(StickerPack::to_wire_dto).collect(),
                total: page.total.unwrap_or(page.items.len() as i64),
                relative_id: page.next_relative_id,
                has_more: page.has_more,
            }
        });
        self.resources.store(&wire, &Self::my_packs_key(kind)).await?;
        Ok(())
    }

    async fn persist_pack(&self, pack_id: &str) -> anyhow::Result<()> {
        let wire = self.state.with(|s| {
            s.state.packs.get(pack_id).map(|content| GetStickersByPackIdGatewayDTO {
                error: String::new(),
                pack: content.pack.as_ref().map(StickerPack::to_wire_dto),
                stickers: content.stickers.iter().map(StickerItem::to_wire_dto).collect(),
                success: true,
            })
        });
        let Some(wire) = wire else { return Ok(()) };
        self.resources.store(&wire, &Self::pack_key(pack_id)).await?;
        Ok(())
    }

    async fn persist_linked_packs(&self, pack_id: &str) -> anyhow::Result<()> {
        let wires: Vec<(String, GetStickersByPackLinkGatewayDTO)> = self.state.with(|s| {
            s.state
                .linked_packs
                .iter()
                .filter(|(_, content)| content.pack.as_ref().is_some_and(|pack| pack.id == pack_id))
                .map(|(link, content)| {
                    (
                        link.clone(),
                        GetStickersByPackLinkGatewayDTO {
                            error: String::new(),
                            pack: content.pack.as_ref().map(StickerPack::to_wire_dto),
                            stickers: content.stickers.iter().map(StickerItem::to_wire_dto).collect(),
                            success: true,
                        },
                    )
                })
                .collect()
        });
        for (link, wire) in &wires {
            self.resources.store(wire, &Self::pack_link_key(link)).await?;
        }
        Ok(())
    }

    async fn wait_until_resources_are_idle(&self, resources: &HashSet<RiseStickerResourceID>) {
        for resource in resources {
            while self.state.with(|s| s.in_flight.contains(resource)) {
                self.wait_until_resource_is_idle(resource).await;
                if self.state.with(|s| s.is_shutdown) {
                    return;
                }
            }
        }
    }

    /// same refusal the server sends for a full pack so the screen shows canon text
    pub fn pack_full_failure() -> RiseFailure {
        let limit = RiseStickerPerformancePolicy::stickers_per_pack_limit();
        RiseFailure::new(
            Some(RiseErrorCode::StickerPackFull.wire().to_owned()),
            "This pack has reached the sticker limit".to_owned(),
            HashMap::from([(RiseErrorParam::Limit.wire().to_owned(), limit.to_string())]),
        )
    }

    /// upload errors are english for the log, screen gets canon text
    /// a file refused as a sticker gets the list of accepted formats
    pub fn failure_text(error: &anyhow::Error) -> String {
        if let Some(code) = RiseMessageRepository::upload_error_code(error) {
            return if code == RiseUploadErrorCode::BadRequest {
                tr("sticker_pack_media_unsupported")
            } else {
                tr(code.i18n_key())
            };
        }
        if let Some(text) =
            error.downcast_ref::<RiseStickerRepositoryError>().and_then(RiseStickerRepositoryError::error_description)
        {
            return text;
        }
        CanonErrors::refusal_text(error)
    }

    async fn update_mutation_progress(
        &self,
        progress: RiseChunkedUploadTransferProgress,
        mutation: &RiseStickerMutationID,
        generation: u64,
        local_sticker_id: &str,
    ) {
        let changed = self.state.with(|s| {
            if s.mutation_generations.get(mutation) != Some(&generation)
                || s.state.mutation_states.get(mutation) != Some(&RiseStickerMutationState::Uploading)
            {
                return false;
            }
            let Some(mut upload) = s.state.sticker_uploads.get(local_sticker_id).cloned() else { return false };

            let previous_fraction = s.state.mutation_progress.get(mutation).copied().unwrap_or(0.0).max(upload.fraction);
            let previous_completed_bytes = upload.completed_bytes;
            let previous_total_bytes = upload.total_bytes;
            let incoming_completed_bytes = progress.completed_bytes.max(0);
            let incoming_total_bytes = incoming_completed_bytes.max(progress.total_bytes);
            let next_total_bytes = previous_total_bytes.max(incoming_total_bytes);
            let mut next_completed_bytes = next_total_bytes.min(previous_completed_bytes.max(incoming_completed_bytes));
            let mut next_fraction = previous_fraction.max(progress.fraction.clamp(0.0, 1.0));
            if next_fraction >= COMPLETE_PROGRESS_FRACTION {
                next_fraction = 1.0;
                next_completed_bytes = next_total_bytes;
            }
            if next_fraction - previous_fraction < rise_upload::PROGRESS_EPSILON_FRACTION
                && next_completed_bytes <= previous_completed_bytes
                && next_total_bytes <= previous_total_bytes
                && !(next_fraction == 1.0 && previous_fraction < 1.0)
            {
                return false;
            }

            s.state.mutation_progress.insert(mutation.clone(), next_fraction);
            upload.phase = RiseStickerMutationState::Uploading;
            upload.fraction = next_fraction;
            upload.completed_bytes = next_completed_bytes;
            upload.total_bytes = next_total_bytes;
            s.state.sticker_uploads.insert(local_sticker_id.to_owned(), upload);
            true
        });
        if changed {
            self.publish().await;
        }
    }

    fn validate_upload_context(
        &self,
        account_epoch: &str,
        session_id: Uuid,
        lifecycle_generation: u64,
    ) -> anyhow::Result<()> {
        let valid = self.state.with(|s| {
            s.is_active
                && !s.is_shutdown
                && s.account_epoch.as_deref() == Some(account_epoch)
                && self.session_id == session_id
                && s.lifecycle_generation == lifecycle_generation
        });
        if valid { Ok(()) } else { Err(cancelled()) }
    }

    /// drains invalidated requests then refreshes every pack projection,
    /// so no pre mutation cache write can win
    async fn reconcile_pack_mutation_authoritatively(&self, pack_id: &str, kind: PackKind) {
        futures::join!(
            self.load_saved_packs(kind, None, true),
            self.load_my_packs(kind, None, true),
            self.load_default_packs(kind, None, true),
            self.load_pack(pack_id, true),
        );
    }

    async fn recover_sticker_after_ambiguous_create(
        &self,
        pack_id: &str,
        canonical_file_url: &str,
    ) -> Option<StickerItem> {
        let expected_url = normalized_remote_url(canonical_file_url);
        if expected_url.is_empty() {
            return None;
        }
        self.load_pack(pack_id, true).await;
        self.state.with(|s| {
            s.state.packs.get(pack_id)?.stickers.iter().find(|sticker| {
                !sticker.id.starts_with(LOCAL_STICKER_PREFIX) && normalized_remote_url(&sticker.file_url) == expected_url
            })
            .cloned()
        })
    }

    async fn finish_created_sticker(
        &self,
        sticker: StickerItem,
        local_id: &str,
        pack_id: &str,
        fallback: &RiseStickerPackContent,
        mutation: &RiseStickerMutationID,
        request_id: &str,
    ) -> StickerItem {
        let (fenced_loads, preview_kind) = self.state.with(|s| {
            let kind = s.resolved_pack(pack_id).map_or(PackKind::Sticker, |pack| pack.pack_kind);
            let mut fenced_loads = s.fence_pack_mutation_loads(pack_id, kind);
            let resource = RiseStickerResourceID::Pack(pack_id.to_owned());
            if s.in_flight.contains(&resource) {
                fenced_loads.insert(resource);
            }
            s.pending_sticker_create_attempts.remove(request_id);
            s.state.sticker_uploads.remove(local_id);
            Self::reconcile_created_sticker(&sticker, local_id, pack_id, fallback, &mut s.state);
            if let Some(content) = s.state.packs.get(pack_id).cloned() {
                for linked in s.state.linked_packs.values_mut() {
                    if linked.pack.as_ref().is_some_and(|pack| pack.id == pack_id) {
                        *linked = content.clone();
                    }
                }
            }
            let preview_kind = Self::reconcile_pack_preview(&sticker, pack_id, &mut s.state);
            s.finish_mutation(mutation);
            (fenced_loads, preview_kind)
        });
        self.publish().await;
        self.wait_until_resources_are_idle(&fenced_loads).await;
        if self.state.with(|s| s.is_shutdown || !s.is_active) {
            return sticker;
        }
        let _ = self.persist_pack(pack_id).await;
        let _ = self.persist_linked_packs(pack_id).await;
        if let Some(preview_kind) = preview_kind {
            let _ = self.persist_pack_lists(preview_kind).await;
            let _ = self.persist_my_packs(preview_kind).await;
        }
        let _ = self.record_persisted_pack(pack_id).await;
        sticker
    }

    async fn recover_sticker_after_ambiguous_edit(
        &self,
        pack_id: &str,
        sticker_id: &str,
        canonical_file_url: Option<&str>,
        emojis: &[String],
    ) -> Option<StickerItem> {
        self.load_pack(pack_id, true).await;
        let candidate = self.state.with(|s| s.sticker_in_pack(pack_id, sticker_id))?;
        if candidate.associated_emojis.iter().collect::<HashSet<_>>() != emojis.iter().collect::<HashSet<_>>() {
            return None;
        }
        let Some(canonical_file_url) = canonical_file_url else { return Some(candidate) };
        (normalized_remote_url(canonical_file_url) == normalized_remote_url(&candidate.file_url)).then_some(candidate)
    }

    async fn finish_edited_sticker(
        &self,
        sticker: StickerItem,
        pack_id: &str,
        mutation: &RiseStickerMutationID,
    ) -> StickerItem {
        let fenced_link_loads = self.state.with(|s| {
            let kind = s.resolved_pack(pack_id).map_or(PackKind::Sticker, |pack| pack.pack_kind);
            let fenced = s.fence_pack_mutation_loads(pack_id, kind);
            s.state.sticker_uploads.remove(&sticker.id);
            s.replace_sticker_everywhere(&sticker, pack_id);
            s.synchronize_pack_preview(pack_id);
            s.finish_mutation(mutation);
            fenced
        });
        self.publish().await;
        self.wait_until_resources_are_idle(&fenced_link_loads).await;
        if let Some(pack) = self.state.with(|s| s.resolved_pack(pack_id)) {
            self.persist_sticker_mutation(pack_id, pack.pack_kind).await;
        }
        sticker
    }

    async fn persist_sticker_mutation(&self, pack_id: &str, kind: PackKind) {
        let _ = self.persist_pack(pack_id).await;
        let _ = self.persist_linked_packs(pack_id).await;
        let _ = self.persist_favourites().await;
        let _ = self.persist_recents(PackKind::Sticker).await;
        let _ = self.persist_recents(PackKind::Emoji).await;
        let _ = self.persist_pack_lists(kind).await;
        let _ = self.persist_my_packs(kind).await;
        let _ = self.record_persisted_pack(pack_id).await;
    }

    fn reconcile_created_sticker(
        sticker: &StickerItem,
        local_id: &str,
        pack_id: &str,
        fallback: &RiseStickerPackContent,
        state: &mut RiseStickerPresentationSnapshot,
    ) {
        let mut content = state.packs.get(pack_id).cloned().unwrap_or_else(|| fallback.clone());
        let mut replaced = false;
        for item in &mut content.stickers {
            if item.id == local_id {
                replaced = true;
                *item = sticker.clone();
            }
        }
        if !replaced {
            content.stickers.push(sticker.clone());
        }
        content.stickers = Self::deduplicated(
            content.stickers,
            |item| item.id.as_str(),
            RiseStickerPerformancePolicy::stickers_per_pack_limit(),
        );
        state.packs.insert(pack_id.to_owned(), content);
    }

    fn update_pack_projections(
        state: &mut RiseStickerPresentationSnapshot,
        pack_id: &str,
        mut updated: impl FnMut(&StickerPack) -> StickerPack,
    ) {
        if let Some(content) = state.packs.get_mut(pack_id)
            && let Some(pack) = &content.pack
        {
            content.pack = Some(updated(pack));
        }
        for pages in [&mut state.my_packs, &mut state.saved_packs, &mut state.default_packs] {
            for kind in [PackKind::Sticker, PackKind::Emoji] {
                let Some(page) = pages.get_mut(&kind) else { continue };
                for pack in page.items.iter_mut().filter(|pack| pack.id == pack_id) {
                    *pack = updated(pack);
                }
            }
        }
        for content in state.linked_packs.values_mut() {
            if let Some(pack) = content.pack.as_ref().filter(|pack| pack.id == pack_id) {
                content.pack = Some(updated(pack));
            }
        }
    }

    fn reconcile_pack_preview(
        sticker: &StickerItem,
        pack_id: &str,
        state: &mut RiseStickerPresentationSnapshot,
    ) -> Option<PackKind> {
        let (preview_url, preview_asset_type) = Self::preview_of(sticker);
        if preview_url.is_empty() {
            return None;
        }
        let mut kind: Option<PackKind> = None;
        Self::update_pack_projections(state, pack_id, |pack| {
            kind = kind.or(Some(pack.pack_kind));
            if !trimming_whitespaces_and_newlines(&pack.preview_url).is_empty() {
                return pack.clone();
            }
            pack.with_preview(preview_url.clone(), preview_asset_type)
        });
        kind
    }

    fn preview_of(sticker: &StickerItem) -> (String, StickerAssetType) {
        if sticker.asset_type == StickerAssetType::Video
            && let Some(thumbnail) = sticker.thumbnail_url.as_deref().map(trimmed).filter(|value| !value.is_empty())
        {
            return (thumbnail, StickerAssetType::StaticImage);
        }
        (trimmed(&sticker.file_url), sticker.asset_type)
    }

    fn normalized_search(value: &str) -> String {
        trimmed(value)
    }

    fn pack_page(value: &GetPacksGatewayDTO) -> RiseStickerPage<StickerPack> {
        RiseStickerPage {
            items: value
                .packs
                .iter()
                .map(|pack| pack.to_pack())
                .take(limit_count(RiseStickerPerformancePolicy::packs_limit()))
                .collect(),
            next_relative_id: value.relative_id.clone(),
            has_more: value.has_more,
            total: Some(value.total),
        }
    }

    fn saved_pack_page(value: &GetSavedPacksGatewayDTO) -> RiseStickerPage<StickerPack> {
        RiseStickerPage {
            items: value
                .packs
                .iter()
                .map(|pack| pack.to_pack())
                .take(limit_count(RiseStickerPerformancePolicy::SAVED_PACK_SUMMARY_LIMIT))
                .collect(),
            next_relative_id: value.relative_id.clone(),
            has_more: value.has_more,
            total: Some(value.packs.len() as i64),
        }
    }

    fn my_pack_page(value: &GetMyPacksGatewayDTO, kind: PackKind) -> RiseStickerPage<StickerPack> {
        let values: Vec<StickerPack> = value.packs.iter().map(|pack| pack.to_pack().with_pack_kind(kind)).collect();
        let items = Self::sorted_created_packs(Self::deduplicated(
            values,
            |pack| pack.id.as_str(),
            RiseStickerPerformancePolicy::pack_summary_limit(),
        ));
        RiseStickerPage {
            items,
            next_relative_id: value.relative_id.clone(),
            has_more: value.has_more,
            total: Some(value.total),
        }
    }

    fn gif_page(value: &GetGifFavouritesGatewayDTO) -> RiseStickerPage<GifItem> {
        RiseStickerPage {
            items: value
                .gifs
                .iter()
                .map(|gif| gif.to_gif())
                .take(limit_count(RiseStickerPerformancePolicy::gif_limit()))
                .collect(),
            next_relative_id: value.relative_id.clone(),
            has_more: value.has_more,
            total: None,
        }
    }

    fn pack_content(value: &GetStickersByPackIdGatewayDTO) -> RiseStickerPackContent {
        RiseStickerPackContent {
            pack: value.pack.as_ref().map(|pack| pack.to_pack()),
            stickers: value
                .stickers
                .iter()
                .map(|sticker| sticker.to_sticker())
                .take(limit_count(RiseStickerPerformancePolicy::stickers_per_pack_limit()))
                .collect(),
        }
    }

    fn pack_content_link(value: &GetStickersByPackLinkGatewayDTO) -> RiseStickerPackContent {
        RiseStickerPackContent {
            pack: value.pack.as_ref().map(|pack| pack.to_pack()),
            stickers: value
                .stickers
                .iter()
                .map(|sticker| sticker.to_sticker())
                .take(limit_count(RiseStickerPerformancePolicy::stickers_per_pack_limit()))
                .collect(),
        }
    }

    fn merged_pack_page(
        current: Option<&RiseStickerPage<StickerPack>>,
        incoming: &[PackWireDTO],
        relative_id: Option<&str>,
        next: Option<String>,
        has_more: Option<bool>,
    ) -> (RiseStickerPage<StickerPack>, GetPacksGatewayDTO) {
        let mut values = if relative_id.is_none() { Vec::new() } else { page_items(current).to_vec() };
        values.extend(incoming.iter().map(|pack| pack.to_pack()));
        let items =
            Self::deduplicated(values, |pack| pack.id.as_str(), RiseStickerPerformancePolicy::packs_limit());
        let has_more = has_more.unwrap_or(false);
        let wire = GetPacksGatewayDTO {
            packs: items.iter().map(StickerPack::to_wire_dto).collect(),
            total: items.len() as i64,
            relative_id: next.clone(),
            has_more,
        };
        (RiseStickerPage { items, next_relative_id: next, has_more, total: None }, wire)
    }

    fn merged_saved_pack_page(
        current: Option<&RiseStickerPage<StickerPack>>,
        incoming: &[PackWireDTO],
        relative_id: Option<&str>,
        next: Option<String>,
        has_more: Option<bool>,
    ) -> (RiseStickerPage<StickerPack>, GetSavedPacksGatewayDTO) {
        let mut values = if relative_id.is_none() { Vec::new() } else { page_items(current).to_vec() };
        values.extend(incoming.iter().map(|pack| pack.to_pack()));
        let items = Self::deduplicated(
            values,
            |pack| pack.id.as_str(),
            RiseStickerPerformancePolicy::SAVED_PACK_SUMMARY_LIMIT,
        );
        let has_more = has_more.unwrap_or(false);
        let wire = GetSavedPacksGatewayDTO {
            packs: items.iter().map(StickerPack::to_wire_dto).collect(),
            relative_id: next.clone(),
            has_more,
        };
        let total = Some(items.len() as i64);
        (RiseStickerPage { items, next_relative_id: next, has_more, total }, wire)
    }

    fn merged_my_pack_page(
        current: Option<&RiseStickerPage<StickerPack>>,
        incoming: &[PackWireDTO],
        kind: PackKind,
        relative_id: Option<&str>,
        next: Option<String>,
        has_more: Option<bool>,
        total: Option<i64>,
    ) -> (RiseStickerPage<StickerPack>, GetMyPacksGatewayDTO) {
        let mut values = if relative_id.is_none() { Vec::new() } else { page_items(current).to_vec() };
        values.extend(incoming.iter().map(|pack| pack.to_pack().with_pack_kind(kind)));
        let items = Self::sorted_created_packs(Self::deduplicated(
            values,
            |pack| pack.id.as_str(),
            RiseStickerPerformancePolicy::pack_summary_limit(),
        ));
        let authoritative_total =
            if relative_id.is_none() { total } else { total.or_else(|| current.and_then(|page| page.total)) };
        let own_count = own_pack_count(&items);
        let resolved_total = authoritative_total.unwrap_or(own_count).max(own_count);
        let has_more = has_more.unwrap_or(false);
        let wire = GetMyPacksGatewayDTO {
            packs: items.iter().map(StickerPack::to_wire_dto).collect(),
            total: resolved_total,
            relative_id: next.clone(),
            has_more,
        };
        (RiseStickerPage { items, next_relative_id: next, has_more, total: Some(resolved_total) }, wire)
    }

    fn merged_gif_page(
        current: &RiseStickerPage<GifItem>,
        incoming: &[GifWireDTO],
        relative_id: Option<&str>,
        next: Option<String>,
        has_more: Option<bool>,
        limit: i64,
    ) -> (RiseStickerPage<GifItem>, GetGifFavouritesGatewayDTO) {
        let mut values = if relative_id.is_none() { Vec::new() } else { current.items.clone() };
        values.extend(incoming.iter().map(|gif| gif.to_gif()));
        let items = Self::deduplicated(values, |gif| gif.id.as_str(), limit);
        let has_more = has_more.unwrap_or(false);
        let wire = GetGifFavouritesGatewayDTO {
            gifs: items.iter().map(GifItem::to_wire_dto).collect(),
            relative_id: next.clone(),
            has_more,
        };
        (RiseStickerPage { items, next_relative_id: next, has_more, total: None }, wire)
    }

    fn merged_search_page(
        current: Option<&RiseStickerPage<GifItem>>,
        incoming: &[GifWireDTO],
        relative_id: Option<&str>,
        next: Option<String>,
        has_more: Option<bool>,
    ) -> RiseStickerPage<GifItem> {
        let mut values = if relative_id.is_none() { Vec::new() } else { page_items(current).to_vec() };
        values.extend(incoming.iter().map(|gif| gif.to_gif()));
        let limit = RiseStickerPerformancePolicy::search_items_limit();
        let items = Self::deduplicated(values, |gif| gif.id.as_str(), limit);
        let is_full = items.len() as i64 >= limit;
        RiseStickerPage {
            items,
            next_relative_id: if is_full { None } else { next },
            has_more: !is_full && has_more.unwrap_or(false),
            total: None,
        }
    }

    fn deduplicated<Value>(values: Vec<Value>, id: impl Fn(&Value) -> &str, limit: i64) -> Vec<Value> {
        let limit = limit_count(limit);
        let mut seen: HashSet<String> = HashSet::new();
        let mut result = Vec::with_capacity(values.len().min(limit));
        for value in values {
            let key = id(&value);
            if key.is_empty() || !seen.insert(key.to_owned()) {
                continue;
            }
            result.push(value);
            if result.len() == limit {
                break;
            }
        }
        result
    }

    fn sorted_created_packs(mut packs: Vec<StickerPack>) -> Vec<StickerPack> {
        packs.sort_by(|left, right| {
            right.created_at_ms.cmp(&left.created_at_ms).then_with(|| right.id.cmp(&left.id))
        });
        packs
    }

    fn decremented(value: Option<i64>) -> Option<i64> {
        value.map(|value| (value - 1).max(0))
    }

    fn local_file_size(url: &Url) -> i64 {
        if !url.is_file_url() {
            return 0;
        }
        let Some(path) = url.to_file_path() else { return 0 };
        std::fs::metadata(path).map_or(0, |metadata| i64::try_from(metadata.len()).unwrap_or(i64::MAX))
    }

    fn remote_url_string(url: Option<&Url>) -> Option<String> {
        let url = url?;
        let scheme = url.scheme()?.to_lowercase();
        (scheme == "https" || scheme == "http").then(|| url.absolute_string().to_owned())
    }

    fn metadata_string(
        metadata: &std::collections::BTreeMap<String, RiseJSONValue>,
        keys: &[&str],
    ) -> Option<String> {
        keys.iter().find_map(|key| {
            let value = trimmed(metadata.get(*key)?.string_value()?);
            (!value.is_empty()).then_some(value)
        })
    }

    fn metadata_int(metadata: &std::collections::BTreeMap<String, RiseJSONValue>, keys: &[&str]) -> Option<i64> {
        keys.iter().find_map(|key| {
            let value = match metadata.get(*key)? {
                RiseJSONValue::Integer(number) => Some(*number),
                RiseJSONValue::UnsignedInteger(number) => i64::try_from(*number).ok(),
                RiseJSONValue::Number(number) if number.is_finite() => {
                    let rounded = number.round();
                    (rounded >= i64::MIN as f64 && rounded < i64::MAX as f64).then_some(rounded as i64)
                }
                RiseJSONValue::String(number) => number.parse::<i64>().ok(),
                _ => None,
            };
            value.filter(|value| *value >= 0)
        })
    }

    fn default_packs_key(kind: PackKind) -> RiseConversationResourceKey {
        RiseConversationResourceKey::new(["stickers", "default-packs", kind.wire()])
    }

    fn saved_packs_key(kind: PackKind) -> RiseConversationResourceKey {
        RiseConversationResourceKey::new(["stickers", "saved-packs", kind.wire()])
    }

    fn my_packs_key(kind: PackKind) -> RiseConversationResourceKey {
        RiseConversationResourceKey::new(["stickers", "my-packs", kind.wire()])
    }

    fn pack_key(id: &str) -> RiseConversationResourceKey {
        RiseConversationResourceKey::new(["stickers", "pack", id])
    }

    fn pack_link_key(link: &str) -> RiseConversationResourceKey {
        RiseConversationResourceKey::new(["stickers", "pack-link", link])
    }

    fn recents_key(kind: PackKind) -> RiseConversationResourceKey {
        RiseConversationResourceKey::new(["stickers", "recents", kind.wire()])
    }

    fn favourites_key() -> RiseConversationResourceKey {
        RiseConversationResourceKey::new(["stickers", "favourites"])
    }

    fn gif_favourites_key() -> RiseConversationResourceKey {
        RiseConversationResourceKey::new(["stickers", "gif-favourites"])
    }

    fn pack_manifest_key() -> RiseConversationResourceKey {
        RiseConversationResourceKey::new(["stickers", "pack-manifest"])
    }
}

/// saved old pages and content so a failed save or delete can be rolled back
struct RiseStickerPackMutationRollback {
    pack_id: String,
    kind: PackKind,
    default_page: Option<RiseStickerPage<StickerPack>>,
    saved_page: Option<RiseStickerPage<StickerPack>>,
    my_page: Option<RiseStickerPage<StickerPack>>,
    content: Option<RiseStickerPackContent>,
    linked_contents: HashMap<String, RiseStickerPackContent>,
}

fn assign<Key: std::hash::Hash + Eq, Value>(map: &mut HashMap<Key, Value>, key: Key, value: Option<Value>) {
    match value {
        Some(value) => {
            map.insert(key, value);
        }
        None => {
            map.remove(&key);
        }
    }
}

impl RiseStickerRepositoryState {
    fn new(account_id: String) -> Self {
        Self {
            state: RiseStickerPresentationSnapshot::new(account_id),
            upload_client: None,
            account_epoch: None,
            lifecycle_generation: 0,
            in_flight: HashSet::default(),
            generations: HashMap::default(),
            forced_reload_tickets: HashMap::default(),
            completed_forced_reload_tickets: HashMap::default(),
            resource_idle_waiters: HashMap::default(),
            mutation_generations: HashMap::default(),
            pending_sticker_create_attempts: HashMap::default(),
            refreshed_at: HashMap::default(),
            persisted_pack_ids: Vec::new(),
            search_order: Vec::new(),
            last_gif_search_query: None,
            hydration_task: None,
            picker_refresh_task: None,
            picker_generation: 0,
            picker_visible: false,
            active_picker_tab: RiseStickerPickerTab::Stickers,
            cached_presentation_tabs: HashSet::default(),
            cold_persistent_tabs: HashSet::default(),
            is_active: false,
            is_shutdown: false,
            custom_emoji_order: Vec::new(),
            custom_emoji_misses: Vec::new(),
            custom_emoji_in_flight: HashSet::default(),
        }
    }

    fn is_picker_current(&self, tab: RiseStickerPickerTab, generation: u64) -> bool {
        self.picker_visible && self.picker_generation == generation && self.active_picker_tab == tab
    }

    fn note_cache_hydration(&mut self, source: RiseConversationResourceReadSource, tab: RiseStickerPickerTab) {
        self.cached_presentation_tabs.insert(tab);
        if source == RiseConversationResourceReadSource::PersistentStore {
            self.cold_persistent_tabs.insert(tab);
        }
    }

    fn picker_tab_for_pack_id(&self, id: &str) -> Option<RiseStickerPickerTab> {
        [PackKind::Sticker, PackKind::Emoji].into_iter().find_map(|kind| {
            let found = [&self.state.saved_packs, &self.state.default_packs, &self.state.my_packs]
                .into_iter()
                .any(|pages| page_items(pages.get(&kind)).iter().any(|pack| pack.id == id));
            found.then(|| Self::picker_tab(kind))
        })
    }

    fn picker_tab(kind: PackKind) -> RiseStickerPickerTab {
        if kind == PackKind::Sticker { RiseStickerPickerTab::Stickers } else { RiseStickerPickerTab::Emoji }
    }

    fn is_within_refresh_interval(&self, resource: &RiseStickerResourceID) -> bool {
        self.refreshed_at.get(resource).is_some_and(|date| {
            Date::now().time_interval_since(date) < RiseStickerPerformancePolicy::refresh_interval()
        })
    }

    fn coalesced_step(
        &mut self,
        resource: &RiseStickerResourceID,
        requested_ticket: Option<u64>,
        force: bool,
        bypass_throttle: bool,
    ) -> RiseStickerCoalescedStep {
        let completed = self.completed_forced_reload_tickets.get(resource).copied().unwrap_or(0);
        if self.in_flight.contains(resource) {
            let Some(ticket) = requested_ticket else { return RiseStickerCoalescedStep::Finished(None) };
            if completed >= ticket {
                return RiseStickerCoalescedStep::Finished(None);
            }
            let (sender, receiver) = oneshot::channel::<()>();
            self.resource_idle_waiters.entry(resource.clone()).or_default().push(sender);
            return RiseStickerCoalescedStep::Wait(receiver, ticket);
        }
        if requested_ticket.is_some_and(|ticket| completed >= ticket) {
            return RiseStickerCoalescedStep::Finished(None);
        }
        if !force && !bypass_throttle && self.is_within_refresh_interval(resource) {
            return RiseStickerCoalescedStep::Finished(None);
        }
        self.in_flight.insert(resource.clone());
        self.refreshed_at.insert(resource.clone(), Date::now());
        self.bump_generation(resource);
        RiseStickerCoalescedStep::Finished(Some(RiseStickerResourceLease {
            generation: self.generations.get(resource).copied().unwrap_or(0),
            forced_ticket: if force { self.forced_reload_tickets.get(resource).copied() } else { None },
        }))
    }

    fn complete_forced_ticket(&mut self, resource: &RiseStickerResourceID, ticket: u64) {
        let completed = self.completed_forced_reload_tickets.entry(resource.clone()).or_default();
        *completed = (*completed).max(ticket);
    }

    fn resume_resource_idle_waiters(&mut self, resource: &RiseStickerResourceID) {
        for waiter in self.resource_idle_waiters.remove(resource).unwrap_or_default() {
            let _ = waiter.send(());
        }
    }

    fn resume_all_resource_idle_waiters(&mut self) {
        for (_, waiters) in self.resource_idle_waiters.drain() {
            for waiter in waiters {
                let _ = waiter.send(());
            }
        }
    }

    /// returns the generation the request runs under, none when it shouldnt run
    fn begin(&mut self, resource: &RiseStickerResourceID, force: bool) -> Option<u64> {
        if self.is_shutdown || self.in_flight.contains(resource) {
            return None;
        }
        if !force && self.is_within_refresh_interval(resource) {
            return None;
        }
        self.in_flight.insert(resource.clone());
        self.refreshed_at.insert(resource.clone(), Date::now());
        Some(self.bump_generation(resource))
    }

    fn bump_generation(&mut self, resource: &RiseStickerResourceID) -> u64 {
        let generation = self.generations.entry(resource.clone()).or_default();
        *generation = generation.wrapping_add(1);
        *generation
    }

    fn accepts(&self, resource: &RiseStickerResourceID, generation: u64) -> bool {
        !self.is_shutdown && self.generations.get(resource) == Some(&generation)
    }

    fn mark_loaded(&mut self, resource: &RiseStickerResourceID) {
        self.state.load_states.insert(resource.clone(), RiseStickerLoadState::Loaded);
        self.refreshed_at.insert(resource.clone(), Date::now());
    }

    fn publishable_snapshot(&mut self) -> Option<RiseStickerPresentationSnapshot> {
        if !self.is_active || self.is_shutdown {
            return None;
        }
        self.state.revision = self.state.revision.wrapping_add(1);
        Some(self.state.clone())
    }

    fn preserve_uploading_stickers(&self, content: &mut RiseStickerPackContent, pack_id: &str) {
        if content.pack.is_none() {
            return;
        }
        let Some(current) = self.state.packs.get(pack_id) else { return };
        for sticker in current.stickers.iter().filter(|sticker| {
            sticker.id.starts_with(LOCAL_STICKER_PREFIX) && self.state.sticker_uploads.contains_key(&sticker.id)
        }) {
            match content.stickers.iter_mut().find(|item| item.id == sticker.id) {
                Some(item) => *item = sticker.clone(),
                None => content.stickers.push(sticker.clone()),
            }
        }
    }

    fn linked_keys(&self, pack_id: &str) -> Vec<String> {
        self.state
            .linked_packs
            .iter()
            .filter(|(_, content)| content.pack.as_ref().is_some_and(|pack| pack.id == pack_id))
            .map(|(link, _)| link.clone())
            .collect()
    }

    fn remove_linked_pack_projections(&mut self, pack_id: &str) {
        for link in self.linked_keys(pack_id) {
            let resource = RiseStickerResourceID::PackLink(link.clone());
            self.bump_generation(&resource);
            self.refreshed_at.remove(&resource);
            self.state.linked_packs.remove(&link);
        }
    }

    fn fence_pack_mutation_loads(&mut self, pack_id: &str, kind: PackKind) -> HashSet<RiseStickerResourceID> {
        let mut resources: HashSet<RiseStickerResourceID> = HashSet::from([
            RiseStickerResourceID::DefaultPacks(kind),
            RiseStickerResourceID::SavedPacks(kind),
            RiseStickerResourceID::MyPacks(kind),
            RiseStickerResourceID::Pack(pack_id.to_owned()),
        ]);
        let mut in_flight_link_resources = HashSet::new();
        resources.extend(self.linked_keys(pack_id).into_iter().map(RiseStickerResourceID::PackLink));
        for resource in &self.in_flight {
            if matches!(resource, RiseStickerResourceID::PackLink(_)) {
                resources.insert(resource.clone());
                in_flight_link_resources.insert(resource.clone());
            }
        }
        for resource in &resources {
            self.bump_generation(resource);
            self.refreshed_at.remove(resource);
        }
        in_flight_link_resources
    }

    fn pack_mutation_rollback(&self, pack_id: &str, kind: PackKind) -> RiseStickerPackMutationRollback {
        RiseStickerPackMutationRollback {
            pack_id: pack_id.to_owned(),
            kind,
            default_page: self.state.default_packs.get(&kind).cloned(),
            saved_page: self.state.saved_packs.get(&kind).cloned(),
            my_page: self.state.my_packs.get(&kind).cloned(),
            content: self.state.packs.get(pack_id).cloned(),
            linked_contents: self.linked_contents(pack_id),
        }
    }

    fn restore_pack_mutation_rollback(&mut self, rollback: &RiseStickerPackMutationRollback) {
        assign(&mut self.state.default_packs, rollback.kind, rollback.default_page.clone());
        assign(&mut self.state.saved_packs, rollback.kind, rollback.saved_page.clone());
        assign(&mut self.state.my_packs, rollback.kind, rollback.my_page.clone());
        assign(&mut self.state.packs, rollback.pack_id.clone(), rollback.content.clone());
        for (link, content) in &rollback.linked_contents {
            self.state.linked_packs.insert(link.clone(), content.clone());
        }
    }

    fn linked_contents(&self, pack_id: &str) -> HashMap<String, RiseStickerPackContent> {
        self.state
            .linked_packs
            .iter()
            .filter(|(_, content)| content.pack.as_ref().is_some_and(|pack| pack.id == pack_id))
            .map(|(link, content)| (link.clone(), content.clone()))
            .collect()
    }

    fn apply_deleted_pack(&mut self, pack_id: &str, kind: PackKind) {
        if let Some(page) = self.state.default_packs.get_mut(&kind) {
            page.items.retain(|pack| pack.id != pack_id);
        }
        if let Some(page) = self.state.saved_packs.get_mut(&kind)
            && page.items.iter().any(|pack| pack.id == pack_id)
        {
            page.items.retain(|pack| pack.id != pack_id);
            page.total = RiseStickerRepository::decremented(page.total);
        }
        if let Some(page) = self.state.my_packs.get_mut(&kind)
            && let Some(removed_is_creator) = page.items.iter().find(|pack| pack.id == pack_id).map(|pack| pack.is_creator)
        {
            page.items.retain(|pack| pack.id != pack_id);
            if removed_is_creator {
                page.total = RiseStickerRepository::decremented(page.total);
            }
        }
        self.state.packs.remove(pack_id);
        self.remove_linked_pack_projections(pack_id);
    }

    fn apply_saved(&mut self, pack_id: &str, kind: PackKind, pack: Option<&StickerPack>, is_saved: bool) {
        let was_saved = page_items(self.state.saved_packs.get(&kind)).iter().any(|item| item.id == pack_id);
        for pages in [&mut self.state.default_packs, &mut self.state.saved_packs, &mut self.state.my_packs] {
            if let Some(page) = pages.get_mut(&kind) {
                for item in page.items.iter_mut().filter(|item| item.id == pack_id) {
                    *item = item.with_saved(is_saved);
                }
            }
        }

        if let (true, Some(pack)) = (is_saved, pack) {
            let page = self.state.saved_packs.entry(kind).or_default();
            let mut values = vec![pack.with_saved(true)];
            values.append(&mut page.items);
            page.items = RiseStickerRepository::deduplicated(
                values,
                |item| item.id.as_str(),
                RiseStickerPerformancePolicy::SAVED_PACK_SUMMARY_LIMIT,
            );
            if !was_saved {
                page.total = Some((page.total.unwrap_or(0) + 1).max(page.items.len() as i64));
            }
        } else if !is_saved && let Some(page) = self.state.saved_packs.get_mut(&kind) {
            page.items.retain(|item| item.id != pack_id);
            if was_saved {
                page.total = RiseStickerRepository::decremented(page.total);
            }
        }

        if let Some(content) = self.state.packs.get_mut(pack_id)
            && let Some(embedded) = &content.pack
        {
            content.pack = Some(embedded.with_saved(is_saved));
        }
        for content in self.state.linked_packs.values_mut() {
            if let Some(embedded) = content.pack.as_ref().filter(|embedded| embedded.id == pack_id) {
                content.pack = Some(embedded.with_saved(is_saved));
            }
        }
    }

    fn sticker(&self, id: &str) -> Option<StickerItem> {
        self.state
            .favourites
            .iter()
            .chain(self.state.recents.values().flatten())
            .chain(self.state.packs.values().flat_map(|content| content.stickers.iter()))
            .find(|sticker| sticker.id == id)
            .cloned()
    }

    fn contains_sticker(&self, pack_id: &str, sticker_id: &str) -> bool {
        self.state.packs.get(pack_id).is_some_and(|content| content.stickers.iter().any(|item| item.id == sticker_id))
    }

    fn sticker_in_pack(&self, pack_id: &str, sticker_id: &str) -> Option<StickerItem> {
        self.state.packs.get(pack_id)?.stickers.iter().find(|item| item.id == sticker_id).cloned()
    }

    fn touch_search(&mut self, query: &str) {
        self.search_order.retain(|value| value != query);
        self.search_order.insert(0, query.to_owned());
        while self.search_order.len() > limit_count(RiseStickerPerformancePolicy::search_cache_limit()) {
            let Some(removed) = self.search_order.pop() else { break };
            let resource = RiseStickerResourceID::GifSearch(removed.clone());
            self.state.gif_searches.remove(&removed);
            self.state.load_states.remove(&resource);
            self.refreshed_at.remove(&resource);
        }
    }

    fn begin_mutation(&mut self, mutation: &RiseStickerMutationID) -> anyhow::Result<u64> {
        if !self.is_active || self.is_shutdown {
            return Err(cancelled());
        }
        match self.state.mutation_states.get(mutation) {
            Some(RiseStickerMutationState::Failed) | None => {}
            Some(_) => return Err(anyhow::Error::new(RiseStickerRepositoryError::MutationInProgress)),
        }
        let generation = self.mutation_generations.entry(mutation.clone()).or_default();
        *generation = generation.wrapping_add(1);
        let generation = *generation;
        self.state.mutation_states.insert(mutation.clone(), RiseStickerMutationState::Preparing);
        self.state.mutation_errors.remove(mutation);
        self.state.mutation_progress.insert(mutation.clone(), 0.0);
        Ok(generation)
    }

    fn accepts_mutation(&self, mutation: &RiseStickerMutationID, generation: u64, lifecycle_generation: u64) -> bool {
        self.is_active
            && !self.is_shutdown
            && self.lifecycle_generation == lifecycle_generation
            && self.mutation_generations.get(mutation) == Some(&generation)
    }

    fn finish_mutation(&mut self, mutation: &RiseStickerMutationID) {
        self.state.mutation_states.remove(mutation);
        self.state.mutation_errors.remove(mutation);
        self.state.mutation_progress.remove(mutation);
    }

    fn fail_mutation(&mut self, mutation: &RiseStickerMutationID, generation: u64, error: &anyhow::Error) {
        if self.mutation_generations.get(mutation) != Some(&generation) {
            return;
        }
        if is_cancellation(error) {
            self.finish_mutation(mutation);
        } else {
            self.state.mutation_states.insert(mutation.clone(), RiseStickerMutationState::Failed);
            self.state.mutation_errors.insert(mutation.clone(), RiseStickerRepository::failure_text(error));
            self.state.mutation_progress.remove(mutation);
        }
    }

    fn update_sticker_upload_phase(&mut self, phase: RiseStickerMutationState, local_sticker_id: &str) {
        let Some(upload) = self.state.sticker_uploads.get_mut(local_sticker_id) else { return };
        upload.phase = phase;
        if phase == RiseStickerMutationState::Submitting {
            upload.fraction = 1.0;
            upload.completed_bytes = upload.total_bytes;
        }
    }

    fn insert_optimistic_pack(&mut self, pack: &StickerPack, kind: PackKind) {
        let my_page = self.state.my_packs.entry(kind).or_default();
        let old_my_count = my_page.items.len();
        let old_own_count = own_pack_count(&my_page.items);
        let mut values = vec![pack.clone()];
        values.append(&mut my_page.items);
        my_page.items = RiseStickerRepository::sorted_created_packs(RiseStickerRepository::deduplicated(
            values,
            |item| item.id.as_str(),
            RiseStickerPerformancePolicy::pack_summary_limit(),
        ));
        if my_page.items.len() > old_my_count {
            my_page.total = Some((my_page.total.unwrap_or(old_own_count) + 1).max(own_pack_count(&my_page.items)));
        }

        let saved_page = self.state.saved_packs.entry(kind).or_default();
        let old_saved_count = saved_page.items.len();
        let mut values = vec![pack.clone()];
        values.append(&mut saved_page.items);
        saved_page.items = RiseStickerRepository::deduplicated(
            values,
            |item| item.id.as_str(),
            RiseStickerPerformancePolicy::SAVED_PACK_SUMMARY_LIMIT,
        );
        if saved_page.items.len() > old_saved_count {
            saved_page.total =
                Some((saved_page.total.unwrap_or(old_saved_count as i64) + 1).max(saved_page.items.len() as i64));
        }
    }

    fn rollback_optimistic_pack(&mut self, local_id: &str, kind: PackKind) {
        for page in [self.state.my_packs.get_mut(&kind), self.state.saved_packs.get_mut(&kind)].into_iter().flatten() {
            if page.items.iter().any(|item| item.id == local_id) {
                page.items.retain(|item| item.id != local_id);
                page.total = RiseStickerRepository::decremented(page.total);
            }
        }
        self.state.packs.remove(local_id);
    }

    fn reconcile_created_pack(&mut self, pack: &StickerPack, local_id: &str, kind: PackKind) {
        let reconciled = |page: &mut RiseStickerPage<StickerPack>, limit: i64, own_only: bool| {
            let already_represented = page.items.iter().any(|item| item.id == local_id || item.id == pack.id);
            let mut values: Vec<StickerPack> = std::mem::take(&mut page.items)
                .into_iter()
                .map(|item| if item.id == local_id { pack.clone() } else { item })
                .collect();
            values.push(pack.clone());
            page.items = RiseStickerRepository::deduplicated(values, |item| item.id.as_str(), limit);
            let total = page.total.unwrap_or(0) + i64::from(!already_represented);
            let floor = if own_only { own_pack_count(&page.items) } else { page.items.len() as i64 };
            page.total = Some(total.max(floor));
        };

        let my_page = self.state.my_packs.entry(kind).or_default();
        reconciled(&mut *my_page, RiseStickerPerformancePolicy::pack_summary_limit(), true);
        my_page.items = RiseStickerRepository::sorted_created_packs(std::mem::take(&mut my_page.items));
        reconciled(
            self.state.saved_packs.entry(kind).or_default(),
            RiseStickerPerformancePolicy::SAVED_PACK_SUMMARY_LIMIT,
            false,
        );

        let local_content = self.state.packs.remove(local_id);
        self.state.packs.insert(
            pack.id.clone(),
            RiseStickerPackContent {
                pack: Some(pack.clone()),
                stickers: local_content.map(|content| content.stickers).unwrap_or_default(),
            },
        );
    }

    fn rollback_optimistic_sticker(&mut self, local_id: &str, pack_id: &str) {
        self.state.sticker_uploads.remove(local_id);
        if let Some(content) = self.state.packs.get_mut(pack_id) {
            content.stickers.retain(|item| item.id != local_id);
        }
    }

    fn mutation_rollback_snapshot(&self, pack_id: &str, kind: PackKind) -> RiseStickerMutationRollbackSnapshot {
        RiseStickerMutationRollbackSnapshot {
            pack_id: pack_id.to_owned(),
            kind,
            pack_content: self.state.packs.get(pack_id).cloned(),
            linked_contents: self.linked_contents(pack_id),
            favourites: self.state.favourites.clone(),
            recents: self.state.recents.clone(),
            default_page: self.state.default_packs.get(&kind).cloned(),
            saved_page: self.state.saved_packs.get(&kind).cloned(),
            my_page: self.state.my_packs.get(&kind).cloned(),
        }
    }

    fn restore_mutation_rollback_snapshot(&mut self, rollback: &RiseStickerMutationRollbackSnapshot) {
        assign(&mut self.state.packs, rollback.pack_id.clone(), rollback.pack_content.clone());
        for link in self.linked_keys(&rollback.pack_id) {
            self.state.linked_packs.remove(&link);
        }
        for (link, content) in &rollback.linked_contents {
            self.state.linked_packs.insert(link.clone(), content.clone());
        }
        self.state.favourites = rollback.favourites.clone();
        self.state.recents = rollback.recents.clone();
        assign(&mut self.state.default_packs, rollback.kind, rollback.default_page.clone());
        assign(&mut self.state.saved_packs, rollback.kind, rollback.saved_page.clone());
        assign(&mut self.state.my_packs, rollback.kind, rollback.my_page.clone());
    }

    fn pack_contents_mut<'a>(
        &'a mut self,
        pack_id: &'a str,
    ) -> impl Iterator<Item = &'a mut RiseStickerPackContent> + 'a {
        let linked = self
            .state
            .linked_packs
            .values_mut()
            .filter(move |content| content.pack.as_ref().is_some_and(|pack| pack.id == pack_id));
        self.state.packs.get_mut(pack_id).into_iter().chain(linked)
    }

    fn replace_sticker_everywhere(&mut self, sticker: &StickerItem, pack_id: &str) {
        for content in self.pack_contents_mut(pack_id) {
            for item in content.stickers.iter_mut().filter(|item| item.id == sticker.id) {
                *item = sticker.clone();
            }
        }
        for item in self.state.favourites.iter_mut().chain(self.state.recents.values_mut().flatten()) {
            if item.id == sticker.id {
                *item = sticker.clone();
            }
        }
    }

    fn remove_sticker_everywhere(&mut self, sticker_id: &str, pack_id: &str) {
        self.state.sticker_uploads.remove(sticker_id);
        for content in self.pack_contents_mut(pack_id) {
            content.stickers.retain(|item| item.id != sticker_id);
        }
        self.state.favourites.retain(|item| item.id != sticker_id);
        for values in self.state.recents.values_mut() {
            values.retain(|item| item.id != sticker_id);
        }
    }

    fn apply_sticker_order(&mut self, sticker_ids: &[String], pack_id: &str) {
        let Some(current) = self.state.packs.get(pack_id).map(|content| content.stickers.clone()) else { return };
        let by_id: HashMap<&str, &StickerItem> = current.iter().map(|item| (item.id.as_str(), item)).collect();
        let ordered: Vec<StickerItem> = sticker_ids
            .iter()
            .enumerate()
            .filter_map(|(index, id)| Some(by_id.get(id.as_str())?.with_position(index as i64)))
            .collect();
        if ordered.len() != current.len() {
            return;
        }
        let positioned_by_id: HashMap<&str, &StickerItem> =
            ordered.iter().map(|item| (item.id.as_str(), item)).collect();
        for content in self.pack_contents_mut(pack_id) {
            content.stickers = ordered.clone();
        }
        for item in self.state.favourites.iter_mut().chain(self.state.recents.values_mut().flatten()) {
            if let Some(positioned) = positioned_by_id.get(item.id.as_str()) {
                *item = (*positioned).clone();
            }
        }
    }

    fn synchronize_pack_preview(&mut self, pack_id: &str) {
        let (preview_url, preview_asset_type) =
            match self.state.packs.get(pack_id).and_then(|content| content.stickers.first()) {
                Some(first) => RiseStickerRepository::preview_of(first),
                None => (String::new(), StickerAssetType::StaticImage),
            };
        RiseStickerRepository::update_pack_projections(&mut self.state, pack_id, |pack| {
            pack.with_preview(preview_url.clone(), preview_asset_type)
        });
    }

    fn resolved_pack(&self, pack_id: &str) -> Option<StickerPack> {
        if let Some(pack) = self.state.packs.get(pack_id).and_then(|content| content.pack.clone()) {
            return Some(pack);
        }
        [PackKind::Sticker, PackKind::Emoji].into_iter().find_map(|kind| {
            [&self.state.my_packs, &self.state.saved_packs, &self.state.default_packs]
                .into_iter()
                .find_map(|pages| page_items(pages.get(&kind)).iter().find(|pack| pack.id == pack_id).cloned())
        })
    }
}
