use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::{Arc, LazyLock};

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use gpui::{App, AsyncApp};
use rise_canon::{RiseTimingCanon, RiseUploadContextFormats, rise_request};
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_chunked_upload::{
    RiseChunkedUploadProgressHandler, RiseChunkedUploadRequest, RiseChunkedUploadResult,
    RiseChunkedUploadStep, RiseChunkedUploadTransferProgress,
};
use rise_engine::{CancellationError, RiseEngine};
use rise_i18n::tr;
use rise_platform::file_manager::FileManager;
use tokio::task::AbortHandle;

use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::chat::engine::core::rise_conversation_upload_support::RiseConversationUploadSupport;
use crate::modules::media_picker::api::media_compression_configuration::MediaCompressionConfiguration;
use crate::modules::media_picker::export::outgoing_media_compressor::OutgoingMediaCompressor;
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;
use crate::modules::music::engine::core::rise_music_resource_repository::RiseMusicResourceRepository;
use crate::modules::music::engine::core::rise_music_rpc::RiseMusicRPC;
use crate::modules::music::engine::rise_music_domain::RiseMusicDomainRegistry;
use crate::modules::music::engine::rise_music_presentation::RiseMusicPresentation;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;

/// cover picked for a local track, still on this device
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RiseMusicLocalCover {
    pub url: Url,
    pub filename: String,
    pub content_type: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RiseMusicRepositorySnapshot {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    pub track_overrides: HashMap<String, RiseStoredMusicTrack>,
    pub updating_track_ids: HashSet<String>,
    pub upload_progress_by_track_id: HashMap<String, f64>,
    pub error_by_track_id: HashMap<String, String>,
    pub sound: RiseMusicSoundSnapshot,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RiseMusicSoundSnapshot {
    pub track_id: String,
    pub track: Option<RiseStoredMusicTrack>,
    pub is_loading: bool,
    pub failed: bool,
    pub unavailable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RiseMusicEditError {
    InvalidTrack,
    StaleAccount,
    InvalidResponse,
    AlreadyUpdating,
}

impl RiseMusicEditError {
    pub fn error_description(&self) -> Option<String> {
        Some(match self {
            Self::InvalidTrack => tr("music_local_upload_invalid_response"),
            Self::StaleAccount => tr("music_message_import_account_changed"),
            Self::InvalidResponse => tr("music_local_upload_invalid_response"),
            Self::AlreadyUpdating => tr("music_uploading"),
        })
    }
}

impl fmt::Display for RiseMusicEditError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.error_description().unwrap_or_default())
    }
}

impl std::error::Error for RiseMusicEditError {}

/// running sound track read
/// done resolves when it ends without cancelling it, abort cancels it
struct RiseMusicSoundTask {
    abort: AbortHandle,
    done: Shared<BoxFuture<'static, ()>>,
}

struct RiseMusicRepositoryState {
    sound: RiseMusicSoundSnapshot,
    sound_task: Option<RiseMusicSoundTask>,
    sound_generation: u64,
    sound_loaded_at: Option<Date>,
    track_overrides: HashMap<String, RiseStoredMusicTrack>,
    track_order: Vec<String>,
    last_revalidation_at: Option<Date>,
    updating_track_ids: HashSet<String>,
    upload_progress_by_track_id: HashMap<String, f64>,
    error_by_track_id: HashMap<String, String>,
    is_revalidating: bool,
    is_shutdown: bool,
}

enum RiseMusicSoundLoadStep {
    Skip,
    Join(Shared<BoxFuture<'static, ()>>),
    Start(u64, Shared<BoxFuture<'static, ()>>),
}

static REVALIDATION_WINDOW_SECONDS: LazyLock<TimeInterval> =
    LazyLock::new(|| RiseTimingCanon::freshness_seconds("music_track_overrides"));

const MAXIMUM_TRACK_OVERRIDES: usize = 32;

const SOUND_RESOURCE_SCOPE: &str = "active-sound-track";

pub struct RiseMusicRepository {
    pub account_id: String,
    pub account_epoch: String,
    pub session_id: Uuid,
    rpc: RiseMusicRPC,
    resource: RiseMusicResourceRepository,
    sound_resource: RiseMusicResourceRepository,
    state: RiseIsolated<RiseMusicRepositoryState>,
}

impl RiseMusicRepository {
    pub fn new(engine: Arc<RiseEngine>, account_id: String, account_epoch: String, session_id: Uuid) -> Self {
        let rpc = RiseMusicRPC::new(engine.clone());
        let resource = RiseMusicResourceRepository::new(engine.clone(), &account_id);
        let sound_resource = RiseMusicResourceRepository::with_scope(engine, &account_id, SOUND_RESOURCE_SCOPE);
        Self {
            account_id,
            account_epoch,
            session_id,
            rpc,
            resource,
            sound_resource,
            state: RiseIsolated::new(RiseMusicRepositoryState {
                sound: RiseMusicSoundSnapshot::default(),
                sound_task: None,
                sound_generation: 0,
                sound_loaded_at: None,
                track_overrides: HashMap::new(),
                track_order: Vec::new(),
                last_revalidation_at: None,
                updating_track_ids: HashSet::new(),
                upload_progress_by_track_id: HashMap::new(),
                error_by_track_id: HashMap::new(),
                is_revalidating: false,
                is_shutdown: false,
            }),
        }
    }

    pub async fn load_sound_track(self: &Arc<Self>, track_id: &str, force: bool) {
        if track_id.is_empty() || !self.is_current_session().await || self.state.with(|s| s.is_shutdown) {
            return;
        }
        let now = Date::now();
        let this = self.clone();
        let step = self.state.with(|s| {
            if s.sound.track_id == track_id {
                if let Some(sound_task) = s.sound_task.as_ref() {
                    return RiseMusicSoundLoadStep::Join(sound_task.done.clone());
                }
                if !force
                    && let Some(sound_loaded_at) = s.sound_loaded_at
                    && now.time_interval_since(&sound_loaded_at) < *REVALIDATION_WINDOW_SECONDS
                {
                    return RiseMusicSoundLoadStep::Skip;
                }
            }
            if let Some(sound_task) = s.sound_task.take() {
                sound_task.abort.abort();
            }
            s.sound_generation = s.sound_generation.wrapping_add(1);
            let generation = s.sound_generation;
            if s.sound.track_id != track_id {
                s.sound = RiseMusicSoundSnapshot {
                    track_id: track_id.to_owned(),
                    ..RiseMusicSoundSnapshot::default()
                };
                s.sound_loaded_at = None;
            }
            s.sound.is_loading = true;
            s.sound.failed = false;
            s.sound.unavailable = false;
            let requested = track_id.to_owned();
            let handle = RiseAccountRuntime::current().spawn(async move {
                this.refresh_sound_track(requested, generation).await;
            });
            let abort = handle.abort_handle();
            let done: Shared<BoxFuture<'static, ()>> = async move {
                let _ = handle.await;
            }
            .boxed()
            .shared();
            s.sound_task = Some(RiseMusicSoundTask {
                abort,
                done: done.clone(),
            });
            RiseMusicSoundLoadStep::Start(generation, done)
        });
        match step {
            RiseMusicSoundLoadStep::Skip => {}
            RiseMusicSoundLoadStep::Join(done) => done.await,
            RiseMusicSoundLoadStep::Start(generation, done) => {
                done.await;
                self.state.with(|s| {
                    if s.sound_generation == generation {
                        s.sound_task = None;
                    }
                });
            }
        }
    }

    async fn refresh_sound_track(&self, track_id: String, generation: u64) {
        if self.state.with(|s| s.sound.track.is_none()) {
            let cached = self.sound_resource.cached().await.ok();
            if !self.owns_sound_request(generation).await {
                return;
            }
            let track = cached.and_then(|tracks| {
                tracks.into_iter().find(|track| track.id.as_deref() == Some(track_id.as_str()))
            });
            self.state.with(|s| s.sound.track = track);
        }
        let _ = self.publish_snapshot().await;
        let fetched = match self.rpc.get_track(&track_id).await {
            Ok(fetched) => fetched,
            Err(_) => {
                self.fail_sound_request(generation).await;
                return;
            }
        };
        if !self.owns_sound_request(generation).await {
            return;
        }
        let Some(track) = fetched else {
            self.state.with(|s| {
                s.sound.track = None;
                s.sound.is_loading = false;
                s.sound.unavailable = true;
                s.sound_loaded_at = Some(Date::now());
            });
            let _ = self.publish_snapshot().await;
            return;
        };
        if track.id.as_deref() != Some(track_id.as_str()) {
            self.fail_sound_request(generation).await;
            return;
        }
        self.state.with(|s| {
            s.sound.track = Some(track.clone());
            s.sound.is_loading = false;
            s.sound_loaded_at = Some(Date::now());
        });
        let _ = self.publish_snapshot().await;
        if !self.owns_sound_request(generation).await {
            return;
        }
        let _ = self.sound_resource.store(vec![track]).await;
    }

    /// failure path of the sound track read
    async fn fail_sound_request(&self, generation: u64) {
        if !self.owns_sound_request(generation).await {
            return;
        }
        self.state.with(|s| {
            s.sound.is_loading = false;
            s.sound.failed = true;
        });
        let _ = self.publish_snapshot().await;
    }

    async fn owns_sound_request(&self, generation: u64) -> bool {
        let owns = |s: &mut RiseMusicRepositoryState| !s.is_shutdown && s.sound_generation == generation;
        if !self.state.with(owns) {
            return false;
        }
        let current = self.is_current_session().await;
        current && self.state.with(owns)
    }

    pub async fn activate(&self) -> RiseMusicRepositorySnapshot {
        if self.state.with(|s| s.is_shutdown) {
            return self.make_snapshot();
        }
        if let Ok(cached) = self.resource.cached().await {
            self.state.with(|s| {
                for track in cached {
                    Self::install(s, track);
                }
            });
        }
        self.make_snapshot()
    }

    pub async fn revalidate_cached_tracks(&self, now: Date) {
        let ids = self.state.with(|s| {
            if s.is_revalidating || s.track_order.is_empty() {
                return None;
            }
            if let Some(last_revalidation_at) = s.last_revalidation_at
                && now.time_interval_since(&last_revalidation_at) < *REVALIDATION_WINDOW_SECONDS
            {
                return None;
            }
            s.last_revalidation_at = Some(now);
            s.is_revalidating = true;
            let skipped = s.track_order.len().saturating_sub(MAXIMUM_TRACK_OVERRIDES);
            Some(s.track_order[skipped..].to_vec())
        });
        let Some(ids) = ids else {
            return;
        };
        let _revalidating = RiseMusicRevalidatingFlag { state: &self.state };
        for track_id in ids {
            if self.state.with(|s| s.is_shutdown) || !self.is_current_session().await {
                return;
            }
            let Ok(Some(track)) = self.rpc.get_track(&track_id).await else {
                continue;
            };
            self.state.with(|s| Self::install(s, track));
        }
        let _ = self.persist_committed().await;
        let _ = self.publish_snapshot().await;
    }

    pub async fn update_local_track(
        self: &Arc<Self>,
        source: RiseStoredMusicTrack,
        request_id: &str,
        title: &str,
        artist_name: &str,
        cover: Option<RiseMusicLocalCover>,
    ) -> anyhow::Result<RiseStoredMusicTrack> {
        if self.state.with(|s| s.is_shutdown) || !self.is_current_session().await {
            return Err(RiseMusicEditError::StaleAccount.into());
        }
        let track_id = match source.id.as_deref() {
            Some(track_id)
                if source.status.as_deref() == Some(RiseMusicTrackStatus::LocalLibrary.wire())
                    && source.library_owner_user_id.as_deref() == Some(self.account_id.as_str())
                    && !track_id.is_empty() =>
            {
                track_id.to_owned()
            }
            _ => return Err(RiseMusicEditError::InvalidTrack.into()),
        };
        let previous = self.state.with(|s| {
            if !s.updating_track_ids.insert(track_id.clone()) {
                return Err(RiseMusicEditError::AlreadyUpdating);
            }
            let previous = s.track_overrides.get(&track_id).cloned().unwrap_or_else(|| source.clone());
            let optimistic = previous.with_local_metadata(
                title,
                artist_name,
                cover.as_ref().map(|cover| cover.url.absolute_string().to_owned()),
            );
            Self::install(s, optimistic);
            s.upload_progress_by_track_id
                .insert(track_id.clone(), if cover.is_none() { 1.0 } else { 0.0 });
            s.error_by_track_id.remove(&track_id);
            Ok(previous)
        })?;
        let mut rollback = RiseMusicEditRollback {
            repository: self.as_ref(),
            track_id: track_id.clone(),
            previous: Some(previous),
        };
        let _ = self.publish_snapshot().await;

        let outcome = async {
            let uploaded_cover = self.upload_cover(cover.as_ref(), request_id, &track_id).await?;
            if self.state.with(|s| s.is_shutdown) || !self.is_current_session().await {
                return Err(RiseMusicEditError::StaleAccount.into());
            }
            let updated = self
                .rpc
                .update_local_track(rise_request::music::UpdateLocalTrack {
                    artist_name: Some(artist_name.to_owned()),
                    cover_asset_id: uploaded_cover.as_ref().map(|uploaded| uploaded.asset_id.clone()),
                    cover_url: uploaded_cover.as_ref().map(|uploaded| uploaded.file_url.clone()),
                    request_id: Some(request_id.to_owned()),
                    title: Some(title.to_owned()),
                    track_id: Some(track_id.clone()),
                })
                .await?;
            let Some(track) = updated.filter(|track| track.id.as_deref() == Some(track_id.as_str())) else {
                return Err(RiseMusicEditError::InvalidResponse.into());
            };
            self.state.with(|s| {
                Self::install(s, track.clone());
                s.updating_track_ids.remove(&track_id);
                s.upload_progress_by_track_id.remove(&track_id);
                s.error_by_track_id.remove(&track_id);
            });
            self.persist_committed().await?;
            let _ = self.publish_snapshot().await;
            anyhow::Ok(track)
        }
        .await;

        match outcome {
            Ok(track) => {
                rollback.previous = None;
                Ok(track)
            }
            Err(error) => {
                rollback.roll_back(error.to_string());
                let _ = self.publish_snapshot().await;
                Err(error)
            }
        }
    }

    pub async fn shutdown(&self) {
        self.state.with(|s| {
            s.is_shutdown = true;
            s.sound_generation = s.sound_generation.wrapping_add(1);
            if let Some(sound_task) = s.sound_task.take() {
                sound_task.abort.abort();
            }
            s.updating_track_ids.clear();
        });
        self.resource.shutdown().await;
        self.sound_resource.shutdown().await;
    }

    async fn upload_cover(
        self: &Arc<Self>,
        cover: Option<&RiseMusicLocalCover>,
        request_id: &str,
        track_id: &str,
    ) -> anyhow::Result<Option<RiseChunkedUploadResult>> {
        let Some(cover) = cover else {
            return Ok(None);
        };
        let Some(hop) = RiseEngineMainHop::installed() else {
            return Err(CancellationError.into());
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let additional_validation = self.additional_validation(hop.clone());
        let client = hop
            .run(move |cx: AsyncApp| {
                async move {
                    let mut cx = cx;
                    RiseConversationUploadSupport::prepare_authentication(
                        Some(account_id.clone()),
                        Some(account_epoch.clone()),
                        &mut cx,
                    )
                    .await?;
                    Ok::<_, anyhow::Error>(cx.update(|cx| {
                        RiseConversationUploadSupport::make_client(
                            Some(account_id),
                            Some(account_epoch),
                            Some(additional_validation),
                            cx,
                        )
                    }))
                }
                .boxed_local()
            })
            .await?;

        let context = RiseUploadContextFormats::MUSIC_COVER.context;
        let Some(cover_path) = cover.url.to_file_path() else {
            return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
        };
        let mut original = RiseOutgoingMediaFile::new(cover_path, cover.filename.clone(), cover.content_type.clone());
        original.file_context = Some(context.to_owned());
        let compressed = OutgoingMediaCompressor::compress_if_needed(
            original.clone(),
            None,
            MediaCompressionConfiguration::cover(),
            None,
            None,
        )
        .await?;
        let _compressed_copy = RiseMusicCompressedCoverCopy {
            url: (compressed.url.standardized() != cover.url.standardized()).then(|| compressed.url.clone()),
        };
        let prepared = RiseConversationUploadSupport::prepared_local_file(&original, &compressed);
        let Some(compressed_path) = compressed.url.to_file_path() else {
            return Err(std::io::Error::from(std::io::ErrorKind::NotFound).into());
        };
        let mut request = RiseChunkedUploadRequest::new(
            compressed_path,
            prepared.filename.clone(),
            compressed.content_type.clone(),
            context.to_owned(),
        );
        request.resource_id = Some(format!("{}:{}", self.account_id, request_id));
        let this = self.clone();
        let progress_track_id = track_id.to_owned();
        let on_progress: RiseChunkedUploadProgressHandler =
            Arc::new(move |progress: RiseChunkedUploadTransferProgress| {
                let this = this.clone();
                let track_id = progress_track_id.clone();
                drop(RiseAccountRuntime::current().spawn(async move {
                    this.update_progress(progress.fraction, &track_id).await;
                }));
            });
        let result = client.upload(request, None, None, Some(on_progress)).await?;
        Ok(Some(result))
    }

    /// extra gate for the upload
    /// account must still be current and this repositorys session still installed
    /// checked on the main thread before every chunk
    fn additional_validation(&self, hop: RiseEngineMainHop) -> RiseChunkedUploadStep {
        let expected_account_id = self.account_id.clone();
        let expected_account_epoch = self.account_epoch.clone();
        let expected_session_id = self.session_id;
        Arc::new(move || -> BoxFuture<'static, anyhow::Result<()>> {
            let expected_account_id = expected_account_id.clone();
            let expected_account_epoch = expected_account_epoch.clone();
            hop.run(move |cx: AsyncApp| {
                async move {
                    let is_current = cx.update(|cx| {
                        SaiAccountCacheScope::is_current_account_epoch(
                            Some(&expected_account_id),
                            Some(&expected_account_epoch),
                        ) && RiseMusicDomainRegistry::shared(cx).read(cx).matches(
                            &expected_account_id,
                            &expected_account_epoch,
                            expected_session_id,
                        )
                    });
                    if !is_current {
                        return Err(anyhow::Error::from(CancellationError));
                    }
                    Ok::<(), anyhow::Error>(())
                }
                .boxed_local()
            })
        })
    }

    async fn update_progress(&self, value: f64, track_id: &str) {
        let moved = self.state.with(|s| {
            if !s.updating_track_ids.contains(track_id) {
                return false;
            }
            let shown = s.upload_progress_by_track_id.get(track_id).copied().unwrap_or(0.0);
            if value - shown < 0.01 && value < 1.0 {
                return false;
            }
            s.upload_progress_by_track_id.insert(track_id.to_owned(), value);
            true
        });
        if moved {
            let _ = self.publish_snapshot().await;
        }
    }

    fn install(s: &mut RiseMusicRepositoryState, track: RiseStoredMusicTrack) {
        let Some(id) = track.id.clone().filter(|id| !id.is_empty()) else {
            return;
        };
        s.track_overrides.insert(id.clone(), track);
        s.track_order.retain(|existing| *existing != id);
        s.track_order.push(id);
        if s.track_order.len() > MAXIMUM_TRACK_OVERRIDES {
            let skipped = s.track_order.len() - MAXIMUM_TRACK_OVERRIDES;
            for removed in s.track_order.drain(..skipped) {
                s.track_overrides.remove(&removed);
            }
        }
    }

    async fn persist_committed(&self) -> anyhow::Result<()> {
        let tracks = self.state.with(|s| {
            s.track_order
                .iter()
                .filter_map(|id| s.track_overrides.get(id).cloned())
                .collect::<Vec<_>>()
        });
        self.resource.store(tracks).await
    }

    async fn publish_snapshot(&self) -> bool {
        let snapshot = self.make_snapshot();
        let Some(hop) = RiseEngineMainHop::installed() else {
            return false;
        };
        hop.run(move |cx: AsyncApp| {
            async move { Ok::<_, anyhow::Error>(cx.update(|cx| Self::publish_on_main(snapshot, cx))) }.boxed_local()
        })
        .await
        .unwrap_or(false)
    }

    fn post_snapshot(&self) {
        let snapshot = self.make_snapshot();
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        hop.post(move |cx: &mut App| {
            Self::publish_on_main(snapshot, cx);
        });
    }

    fn publish_on_main(snapshot: RiseMusicRepositorySnapshot, cx: &mut App) -> bool {
        if !RiseMusicDomainRegistry::shared(cx).read(cx).matches(
            &snapshot.account_id,
            &snapshot.account_epoch,
            snapshot.session_id,
        ) {
            return false;
        }
        RiseMusicPresentation::publish(snapshot, cx)
    }

    async fn is_current_session(&self) -> bool {
        let Some(hop) = RiseEngineMainHop::installed() else {
            return false;
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        hop.run(move |cx: AsyncApp| {
            async move {
                Ok::<_, anyhow::Error>(cx.update(|cx| {
                    SaiAccountCacheScope::is_current_account_epoch(Some(&account_id), Some(&account_epoch))
                        && RiseMusicDomainRegistry::shared(cx).read(cx).matches(
                            &account_id,
                            &account_epoch,
                            session_id,
                        )
                }))
            }
            .boxed_local()
        })
        .await
        .unwrap_or(false)
    }

    fn make_snapshot(&self) -> RiseMusicRepositorySnapshot {
        self.state.with(|s| RiseMusicRepositorySnapshot {
            account_id: self.account_id.clone(),
            account_epoch: self.account_epoch.clone(),
            session_id: self.session_id,
            track_overrides: s.track_overrides.clone(),
            updating_track_ids: s.updating_track_ids.clone(),
            upload_progress_by_track_id: s.upload_progress_by_track_id.clone(),
            error_by_track_id: s.error_by_track_id.clone(),
            sound: s.sound.clone(),
        })
    }
}

struct RiseMusicRevalidatingFlag<'a> {
    state: &'a RiseIsolated<RiseMusicRepositoryState>,
}

impl Drop for RiseMusicRevalidatingFlag<'_> {
    fn drop(&mut self) {
        self.state.with(|s| s.is_revalidating = false);
    }
}

struct RiseMusicEditRollback<'a> {
    repository: &'a RiseMusicRepository,
    track_id: String,
    previous: Option<RiseStoredMusicTrack>,
}

impl RiseMusicEditRollback<'_> {
    fn roll_back(&mut self, error_description: String) {
        let Some(previous) = self.previous.take() else {
            return;
        };
        self.repository.state.with(|s| {
            RiseMusicRepository::install(s, previous);
            s.updating_track_ids.remove(&self.track_id);
            s.upload_progress_by_track_id.remove(&self.track_id);
            s.error_by_track_id.insert(self.track_id.clone(), error_description);
        });
    }
}

impl Drop for RiseMusicEditRollback<'_> {
    fn drop(&mut self) {
        if self.previous.is_none() {
            return;
        }
        self.roll_back(CancellationError.to_string());
        self.repository.post_snapshot();
    }
}

struct RiseMusicCompressedCoverCopy {
    url: Option<Url>,
}

impl Drop for RiseMusicCompressedCoverCopy {
    fn drop(&mut self) {
        let Some(path) = self.url.take().and_then(|url| url.to_file_path()) else {
            return;
        };
        let _ = FileManager.remove_item(&path);
    }
}
