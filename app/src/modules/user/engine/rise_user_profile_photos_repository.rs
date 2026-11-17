use rise_engine::desktop_runtime::rise_runtime_timer::RiseRuntimeTimer;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Weak};
use std::time::Duration;

use futures::FutureExt;
use futures::future::{BoxFuture, Either, Shared};
use gpui::AsyncApp;
use rise_canon::RiseTimingCanon;
use rise_canon::RiseIdentityCanon;
use rise_canon::rise_upload;
use rise_core::RiseIsolated;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_url_session::{URLError, URLErrorCode};
use rise_engine::{CancellationError, RiseEngine, RiseWireError};
use rise_i18n::tr;
use rise_limits::service_limit::MAX_PROFILE_PHOTOS_PER_USER;
use tokio::task::AbortHandle;

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::rise_engine_bridge::rise_engine_main_hop::RiseEngineMainHop;
use crate::modules::user::engine::core::rise_user_profile_photo_wire_models::RiseProfilePhoto;
use crate::modules::user::engine::core::rise_user_profile_photos_resource_repository::{
    RiseUserProfilePhotosCacheSource, RiseUserProfilePhotosResourcePayload, RiseUserProfilePhotosResourceRepository,
};
use crate::modules::user::engine::core::rise_user_rpc::RiseUserRPC;
use crate::modules::user::engine::rise_user_profile_photo_upload::{
    RiseProfilePhotoDraft, RiseProfilePhotoUploadResult, RiseUserProfilePhotoUpload,
};
use crate::modules::user::engine::rise_user_profile_photos_presentation::{
    RiseProfilePhotoAlbum, RiseUserProfilePhotosLoadState, RiseUserProfilePhotosPresentation,
    RiseUserProfilePhotosSnapshot,
};
use crate::modules::user::stores::user_profile_photos::user_profile_photos_interactions::user_profile_photos_interactions::UserProfilePhotosInteractionsStore;

const OPTIMISTIC_TEMPORARY_PREFIX: &str = RiseIdentityCanon::OPTIMISTIC_TEMPORARY_PREFIX;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RiseUserProfilePhotosError {
    #[error("rejected({0})")]
    Rejected(String),
    #[error("unknownAlbum")]
    UnknownAlbum,
}

impl RiseUserProfilePhotosError {
    pub fn error_description(&self) -> Option<String> {
        match self {
            Self::Rejected(message) => (!message.is_empty()).then(|| message.clone()),
            Self::UnknownAlbum => None,
        }
    }
}

#[derive(Clone)]
struct RiseUserProfilePhotosRepositoryPendingAddition {
    temp_id: String,
    draft: RiseProfilePhotoDraft,
}

/// a running album refresh, done waits without cancelling and abort cancels
struct RiseUserProfilePhotosRepositoryRefresh {
    abort: AbortHandle,
    done: Shared<BoxFuture<'static, ()>>,
}

/// mutable state of the repository, one closure is one synchronous stretch
struct RiseUserProfilePhotosRepositoryState {
    snapshot: RiseUserProfilePhotosSnapshot,
    is_shutdown: bool,
    generation_clock: u64,
    generations: HashMap<String, u64>,
    mutation_generations: HashMap<String, u64>,
    refreshed_at_ms: HashMap<String, i64>,
    in_flight: HashMap<String, RiseUserProfilePhotosRepositoryRefresh>,
    pending_rows: HashMap<String, RiseProfilePhoto>,
    pending_order: Vec<String>,
    addition_queue: VecDeque<RiseUserProfilePhotosRepositoryPendingAddition>,
    addition_task: Option<AbortHandle>,
}

/// what one stretch of load decided
enum RiseUserProfilePhotosRepositoryLoadStep {
    Done,
    Join(Shared<BoxFuture<'static, ()>>),
    Run(u64, Shared<BoxFuture<'static, ()>>),
}

/// what one attempt of perform_addition ended in when it didnt fail
enum RiseUserProfilePhotosRepositoryAdditionStep {
    Finished,
    Retry,
}

/// what the optimistic stretch of a promote or delete decided
enum RiseUserProfilePhotosRepositoryMutationStep {
    Settled(String),
    Send { album: RiseProfilePhotoAlbum, generation: u64 },
}

struct RiseUserProfilePhotosRepositoryWaitingNotice<'a> {
    repository: &'a RiseUserProfilePhotosRepository,
    user_id: &'a str,
    announced: bool,
}

impl Drop for RiseUserProfilePhotosRepositoryWaitingNotice<'_> {
    fn drop(&mut self) {
        if self.announced {
            self.repository.clear_waiting_notice(self.user_id);
        }
    }
}

/// owns the bounded set of albums this session opened
/// not part of a profile read, loads on first open
/// cache first, failure keeps the album
pub struct RiseUserProfilePhotosRepository {
    engine: Arc<RiseEngine>,
    account_id: String,
    account_epoch: String,
    session_id: Uuid,
    resource: RiseUserProfilePhotosResourceRepository,
    rpc: RiseUserRPC,
    this: Weak<RiseUserProfilePhotosRepository>,
    state: RiseIsolated<RiseUserProfilePhotosRepositoryState>,
}

impl RiseUserProfilePhotosRepository {
    pub const MAXIMUM_PHOTOS: i64 = MAX_PROFILE_PHOTOS_PER_USER as i64;

    const MAXIMUM_ALBUMS: usize = 8;

    const OWN_FRESHNESS: &'static str = "user_profile_own";
    const FOREIGN_FRESHNESS: &'static str = "user_profile_foreign";
    const READINESS_POLL: Duration = Duration::from_millis(300);

    pub fn new(engine: Arc<RiseEngine>, account_id: String, account_epoch: String, session_id: Uuid) -> Arc<Self> {
        let rpc = RiseUserRPC::new(engine.clone());
        let resource = RiseUserProfilePhotosResourceRepository::new(engine.clone(), &account_id);
        Arc::new_cyclic(|this| Self {
            engine,
            account_id,
            account_epoch,
            session_id,
            resource,
            rpc,
            this: this.clone(),
            state: RiseIsolated::new(RiseUserProfilePhotosRepositoryState {
                snapshot: RiseUserProfilePhotosSnapshot::EMPTY,
                is_shutdown: false,
                generation_clock: 0,
                generations: HashMap::new(),
                mutation_generations: HashMap::new(),
                refreshed_at_ms: HashMap::new(),
                in_flight: HashMap::new(),
                pending_rows: HashMap::new(),
                pending_order: Vec::new(),
                addition_queue: VecDeque::new(),
                addition_task: None,
            }),
        })
    }

    fn own_refresh_interval_ms() -> i64 {
        RiseTimingCanon::freshness_ms(Self::OWN_FRESHNESS)
    }

    fn foreign_refresh_interval_ms() -> i64 {
        RiseTimingCanon::freshness_ms(Self::FOREIGN_FRESHNESS)
    }

    pub fn current(&self) -> RiseUserProfilePhotosSnapshot {
        self.state.with(|s| s.snapshot.clone())
    }

    pub fn album(&self, user_id: &str) -> Option<RiseProfilePhotoAlbum> {
        self.state.with(|s| s.snapshot.album(user_id).cloned())
    }

    pub async fn seed_from_cache(&self, user_id: &str) -> Option<RiseProfilePhotoAlbum> {
        let existing = self.state.with(|s| {
            if s.is_shutdown || user_id.is_empty() {
                return Err(());
            }
            Ok(s.snapshot.album(user_id).filter(|album| !album.photos.is_empty()).cloned())
        });
        match existing {
            Err(()) => return None,
            Ok(Some(existing)) => return Some(existing),
            Ok(None) => {}
        }
        let cached = self.resource.cached(user_id).await.ok()?;
        self.state.with(|s| {
            if s.is_shutdown || cached.source == RiseUserProfilePhotosCacheSource::Miss {
                return None;
            }
            let mut album =
                s.snapshot.album(user_id).cloned().unwrap_or_else(|| RiseProfilePhotoAlbum::empty(user_id));
            album.photos = Self::ordered(cached.payload.photos.clone());
            album.total = cached.payload.total.max(album.photos.len() as i64);
            album.has_more = cached.payload.has_more;
            album.next_cursor = cached.payload.next_cursor.clone();
            album.is_visible = cached.payload.visible;
            album.load_state = RiseUserProfilePhotosLoadState::Loaded;
            self.upsert(s, album.clone());
            Some(album)
        })
    }

    pub async fn load(&self, user_id: &str, is_own: bool, force: bool) {
        let step = self.state.with(|s| {
            if s.is_shutdown || user_id.is_empty() {
                return RiseUserProfilePhotosRepositoryLoadStep::Done;
            }

            let now = Self::now_milliseconds();
            let window = if is_own { Self::own_refresh_interval_ms() } else { Self::foreign_refresh_interval_ms() };
            let existing = s.snapshot.album(user_id).cloned();
            if !force
                && existing.as_ref().map(|album| album.load_state) == Some(RiseUserProfilePhotosLoadState::Loaded)
                && now - s.refreshed_at_ms.get(user_id).copied().unwrap_or(0) < window
            {
                if let Some(mut album) = existing
                    && (album.error.is_some() || album.mutation_error.is_some())
                {
                    album.error = None;
                    album.mutation_error = None;
                    self.upsert(s, album);
                }
                return RiseUserProfilePhotosRepositoryLoadStep::Done;
            }
            if let Some(task) = s.in_flight.get(user_id) {
                if !force {
                    return RiseUserProfilePhotosRepositoryLoadStep::Join(task.done.clone());
                }
                task.abort.abort();
            }

            s.generation_clock = s.generation_clock.wrapping_add(1);
            let generation = s.generation_clock;
            s.generations.insert(user_id.to_owned(), generation);

            let mut pending =
                s.snapshot.album(user_id).cloned().unwrap_or_else(|| RiseProfilePhotoAlbum::empty(user_id));
            pending.load_state = if pending.photos.is_empty() {
                RiseUserProfilePhotosLoadState::Loading
            } else {
                RiseUserProfilePhotosLoadState::Loaded
            };
            pending.is_refreshing = true;
            pending.error = None;
            pending.mutation_error = None;
            self.upsert(s, pending);

            let this = self.this.clone();
            let refresh_user_id = user_id.to_owned();
            let handle = RiseAccountRuntime::current().spawn(async move {
                if let Some(this) = this.upgrade() {
                    this.perform_refresh(&refresh_user_id, generation).await;
                }
            });
            let abort = handle.abort_handle();
            let done = async move {
                let _ = handle.await;
            }
            .boxed()
            .shared();
            s.in_flight.insert(
                user_id.to_owned(),
                RiseUserProfilePhotosRepositoryRefresh { abort, done: done.clone() },
            );
            RiseUserProfilePhotosRepositoryLoadStep::Run(generation, done)
        });

        match step {
            RiseUserProfilePhotosRepositoryLoadStep::Done => {}
            RiseUserProfilePhotosRepositoryLoadStep::Join(done) => done.await,
            RiseUserProfilePhotosRepositoryLoadStep::Run(generation, done) => {
                done.await;
                self.state.with(|s| {
                    if s.generations.get(user_id) == Some(&generation) {
                        s.in_flight.remove(user_id);
                    }
                });
            }
        }
    }

    async fn perform_refresh(&self, user_id: &str, generation: u64) {
        let result = self
            .rpc
            .get_profile_photos(rise_canon::rise_request::user::GetProfilePhotos {
                limit: Some(Self::MAXIMUM_PHOTOS),
                relative_id: None,
                user_id: user_id.to_owned(),
            })
            .await;
        let response = match result {
            Ok(response) => response,
            Err(error) => {
                self.state.with(|s| {
                    if !Self::accepts_read(s, user_id, generation) {
                        return;
                    }
                    if Self::is_transport_unavailable(&error) {
                        let mut album = s
                            .snapshot
                            .album(user_id)
                            .cloned()
                            .unwrap_or_else(|| RiseProfilePhotoAlbum::empty(user_id));
                        album.is_refreshing = false;
                        self.upsert(s, album);
                        return;
                    }
                    self.finish_with_load_error(s, user_id, &CanonErrors::load_failure_text(&error));
                });
                return;
            }
        };

        let photos = Self::ordered(response.photos.iter().map(RiseProfilePhoto::from_canon).collect());
        let accepted = self.state.with(|s| {
            if !Self::accepts_read(s, user_id, generation) {
                return false;
            }
            s.refreshed_at_ms.insert(user_id.to_owned(), Self::now_milliseconds());
            true
        });
        if !accepted {
            return;
        }
        let _ = self
            .resource
            .store(
                &RiseUserProfilePhotosResourcePayload {
                    photos: photos.clone(),
                    total: response.total.max(photos.len() as i64),
                    has_more: response.has_more,
                    next_cursor: response.relative_id.clone(),
                    visible: response.visible,
                },
                user_id,
            )
            .await;

        self.state.with(|s| {
            if !Self::accepts_read(s, user_id, generation) {
                return;
            }
            let mut album =
                s.snapshot.album(user_id).cloned().unwrap_or_else(|| RiseProfilePhotoAlbum::empty(user_id));
            album.photos = if response.visible { self.with_pending_rows(s, photos.clone(), user_id) } else { Vec::new() };
            album.total = if response.visible {
                response.total.max(photos.len() as i64) + self.pending_count(s, user_id)
            } else {
                0
            };
            album.has_more = response.visible && response.has_more;
            album.next_cursor = if response.visible { response.relative_id.clone() } else { None };
            album.is_visible = response.visible;
            album.load_state = RiseUserProfilePhotosLoadState::Loaded;
            album.is_refreshing = false;
            album.mutating_ids.clear();
            album.error = None;
            self.upsert(s, album);
        });
    }

    fn accepts_read(s: &RiseUserProfilePhotosRepositoryState, user_id: &str, generation: u64) -> bool {
        !s.is_shutdown && s.generations.get(user_id) == Some(&generation)
    }

    /// fetches the next page and appends, the viewer pager is index anchored so a prepend would teleport it
    pub async fn load_more(&self, user_id: &str) {
        let start = self.state.with(|s| {
            if s.is_shutdown {
                return None;
            }
            let existing = s.snapshot.album(user_id)?;
            if !existing.has_more || s.in_flight.contains_key(user_id) {
                return None;
            }
            let cursor = existing.next_cursor.clone()?;
            s.generation_clock = s.generation_clock.wrapping_add(1);
            let generation = s.generation_clock;
            s.generations.insert(user_id.to_owned(), generation);
            Some((cursor, generation))
        });
        let Some((cursor, generation)) = start else {
            return;
        };

        let Ok(response) = self
            .rpc
            .get_profile_photos(rise_canon::rise_request::user::GetProfilePhotos {
                limit: Some(Self::MAXIMUM_PHOTOS),
                relative_id: Some(cursor),
                user_id: user_id.to_owned(),
            })
            .await
        else {
            return;
        };

        let payload = self.state.with(|s| {
            if !Self::accepts_read(s, user_id, generation) {
                return None;
            }
            let mut album =
                s.snapshot.album(user_id).cloned().unwrap_or_else(|| RiseProfilePhotoAlbum::empty(user_id));
            let mut server: Vec<RiseProfilePhoto> = album.photos.iter().filter(|photo| !photo.is_pending).cloned().collect();
            let known: std::collections::HashSet<String> = server.iter().map(|photo| photo.id.clone()).collect();
            server.extend(
                Self::ordered(response.photos.iter().map(RiseProfilePhoto::from_canon).collect())
                    .into_iter()
                    .filter(|photo| !known.contains(&photo.id)),
            );
            album.photos = self.with_pending_rows(s, server, user_id);
            album.total = response.total.max(album.photos.len() as i64);
            album.has_more = response.has_more;
            album.next_cursor = response.relative_id.clone();
            let payload = RiseUserProfilePhotosResourcePayload {
                photos: album.photos.clone(),
                total: album.total,
                has_more: album.has_more,
                next_cursor: album.next_cursor.clone(),
                visible: album.is_visible,
            };
            self.upsert(s, album);
            Some(payload)
        });
        if let Some(payload) = payload {
            let _ = self.resource.store(&payload, user_id).await;
        }
    }

    pub fn add_photos(&self, drafts: Vec<RiseProfilePhotoDraft>) {
        self.state.with(|s| {
            if s.is_shutdown || drafts.is_empty() {
                return;
            }
            let user_id = self.account_id.clone();

            let now = Self::now_milliseconds();
            for (index, draft) in drafts.iter().enumerate() {
                let temp_id = format!("{OPTIMISTIC_TEMPORARY_PREFIX}{}", Uuid::new_v4().uuid_string());
                s.pending_rows.insert(
                    temp_id.clone(),
                    RiseProfilePhoto::new(temp_id.clone(), draft.local_url.path(), now + index as i64, false, true),
                );
                s.pending_order.push(temp_id.clone());
                s.addition_queue
                    .push_back(RiseUserProfilePhotosRepositoryPendingAddition { temp_id, draft: draft.clone() });
            }

            let mut album =
                s.snapshot.album(&user_id).cloned().unwrap_or_else(|| RiseProfilePhotoAlbum::empty(&user_id));
            let server: Vec<RiseProfilePhoto> = album.photos.iter().filter(|photo| !photo.is_pending).cloned().collect();
            album.photos = self.with_pending_rows(s, server, &user_id);
            album.total = (album.total + drafts.len() as i64).max(album.photos.len() as i64);
            album.mutation_error = None;
            self.upsert(s, album);

            self.start_additions_if_needed(s);
        });
    }

    fn start_additions_if_needed(&self, s: &mut RiseUserProfilePhotosRepositoryState) {
        if s.addition_task.is_some() || s.is_shutdown {
            return;
        }
        let this = self.this.clone();
        let handle = RiseAccountRuntime::current().spawn(async move {
            if let Some(this) = this.upgrade() {
                this.run_additions().await;
            }
        });
        s.addition_task = Some(handle.abort_handle());
    }

    async fn run_additions(&self) {
        loop {
            let next = self.state.with(|s| {
                if s.is_shutdown {
                    s.addition_task = None;
                    return None;
                }
                let next = s.addition_queue.pop_front();
                if next.is_none() {
                    s.addition_task = None;
                }
                next
            });
            let Some(next) = next else {
                return;
            };
            self.perform_addition(next).await;
        }
    }

    async fn perform_addition(&self, pending: RiseUserProfilePhotosRepositoryPendingAddition) {
        let user_id = self.account_id.clone();
        let mut uploaded: Option<RiseProfilePhotoUploadResult> = None;
        let mut attempt: u64 = 0;
        loop {
            let error = match self.attempt_addition(&pending, &user_id, &mut uploaded).await {
                Ok(RiseUserProfilePhotosRepositoryAdditionStep::Finished) => return,
                Ok(RiseUserProfilePhotosRepositoryAdditionStep::Retry) => continue,
                Err(error) => error,
            };
            if self.is_shutdown() {
                return;
            }
            if CanonErrors::is_connection_failure(&error)
                && self.engine.is_ready_for_calls(false).await.ok() != Some(true)
            {
                continue;
            }
            attempt += 1;
            if Self::is_transient_addition_failure(&error) && attempt < rise_upload::RETRY_COMPLETE_MAXIMUM_ATTEMPTS {
                continue;
            }
            self.fail_addition(&user_id, &pending.temp_id, Self::addition_failure_message(&error));
            return;
        }
    }

    async fn attempt_addition(
        &self,
        pending: &RiseUserProfilePhotosRepositoryPendingAddition,
        user_id: &str,
        uploaded: &mut Option<RiseProfilePhotoUploadResult>,
    ) -> anyhow::Result<RiseUserProfilePhotosRepositoryAdditionStep> {
        self.wait_until_ready_for_calls(user_id).await?;
        let file = match uploaded.clone() {
            Some(file) => file,
            None => {
                let Some(sent) = self.upload_while_connected(pending.draft.clone(), user_id.to_owned()).await? else {
                    if self.is_shutdown() {
                        return Ok(RiseUserProfilePhotosRepositoryAdditionStep::Finished);
                    }
                    return Ok(RiseUserProfilePhotosRepositoryAdditionStep::Retry);
                };
                *uploaded = Some(sent.clone());
                sent
            }
        };
        if self.is_shutdown() {
            return Ok(RiseUserProfilePhotosRepositoryAdditionStep::Finished);
        }

        let response = self
            .rpc
            .add_profile_photo(rise_canon::rise_request::user::AddProfilePhoto {
                asset_id: file.asset_id.clone(),
                file_url: file.file_url.clone(),
            })
            .await?;
        if self.is_shutdown() {
            return Ok(RiseUserProfilePhotosRepositoryAdditionStep::Finished);
        }
        let Some(photo) = response.photo.as_ref().map(RiseProfilePhoto::from_canon).filter(|photo| !photo.id.is_empty())
        else {
            return Err(RiseUserProfilePhotosError::Rejected(response.error.clone()).into());
        };

        self.settle_addition(user_id, &pending.temp_id, photo, response.total);
        self.persist(user_id).await;
        RiseUserProfilePhotoUpload::discard_staged_file(&pending.draft.local_url);
        self.mirror_account_avatar(&response.logo).await;
        Ok(RiseUserProfilePhotosRepositoryAdditionStep::Finished)
    }

    async fn wait_until_ready_for_calls(&self, user_id: &str) -> anyhow::Result<()> {
        let mut notice = RiseUserProfilePhotosRepositoryWaitingNotice { repository: self, user_id, announced: false };
        while !self.engine.is_ready_for_calls(false).await? {
            if self.is_shutdown() {
                return Err(CancellationError.into());
            }
            if !notice.announced {
                notice.announced = true;
                self.set_mutation_error(Some(Self::waiting_for_network_text()), user_id);
            }
            RiseRuntimeTimer::sleep(Self::READINESS_POLL).await;
        }
        Ok(())
    }

    async fn upload_while_connected(
        &self,
        draft: RiseProfilePhotoDraft,
        account_id: String,
    ) -> anyhow::Result<Option<RiseProfilePhotoUploadResult>> {
        let engine = self.engine.clone();
        let upload = async move {
            let Some(hop) = RiseEngineMainHop::installed() else {
                return Err::<Option<RiseProfilePhotoUploadResult>, anyhow::Error>(CancellationError.into());
            };
            hop.run(move |mut cx: AsyncApp| {
                async move { RiseUserProfilePhotoUpload::upload(&draft, &account_id, &mut cx).await }.boxed_local()
            })
            .await
            .map(Some)
        }
        .boxed();
        let watch = async move {
            while engine.is_ready_for_calls(false).await? {
                RiseRuntimeTimer::sleep(Self::READINESS_POLL).await;
            }
            Ok::<Option<RiseProfilePhotoUploadResult>, anyhow::Error>(None)
        }
        .boxed();
        match futures::future::select(upload, watch).await {
            Either::Left((first, _)) | Either::Right((first, _)) => first,
        }
    }

    fn waiting_for_network_text() -> String {
        tr("chats_status_waiting_for_network")
    }

    fn set_mutation_error(&self, message: Option<String>, user_id: &str) {
        self.state.with(|s| {
            let mut album =
                s.snapshot.album(user_id).cloned().unwrap_or_else(|| RiseProfilePhotoAlbum::empty(user_id));
            if album.mutation_error == message {
                return;
            }
            album.mutation_error = message;
            self.upsert(s, album);
        });
    }

    fn clear_waiting_notice(&self, user_id: &str) {
        let waiting = Self::waiting_for_network_text();
        let shows_waiting = self.state.with(|s| {
            s.snapshot.album(user_id).and_then(|album| album.mutation_error.as_deref()) == Some(waiting.as_str())
        });
        if !shows_waiting {
            return;
        }
        self.set_mutation_error(None, user_id);
    }

    fn is_transient_addition_failure(error: &anyhow::Error) -> bool {
        if error.is::<CancellationError>() {
            return true;
        }
        if let Some(wire) = error.downcast_ref::<RiseWireError>() {
            return matches!(
                wire,
                RiseWireError::NoSocketSession
                    | RiseWireError::NotConnected
                    | RiseWireError::Suspended
                    | RiseWireError::Timeout(_)
                    | RiseWireError::Disconnected(_)
            );
        }
        if let Some(url) = error.downcast_ref::<URLError>() {
            return matches!(
                url.code,
                URLErrorCode::TimedOut
                    | URLErrorCode::NetworkConnectionLost
                    | URLErrorCode::NotConnectedToInternet
                    | URLErrorCode::CannotConnectToHost
                    | URLErrorCode::CannotFindHost
                    | URLErrorCode::DnsLookupFailed
                    | URLErrorCode::DataNotAllowed
                    | URLErrorCode::Cancelled
            );
        }
        false
    }

    fn settle_addition(&self, user_id: &str, temp_id: &str, photo: RiseProfilePhoto, server_total: i64) {
        self.state.with(|s| {
            Self::forget_pending(s, temp_id);

            let mut album =
                s.snapshot.album(user_id).cloned().unwrap_or_else(|| RiseProfilePhotoAlbum::empty(user_id));
            let mut server: Vec<RiseProfilePhoto> = album.photos.iter().filter(|row| !row.is_pending).cloned().collect();
            let photo_id = photo.id.clone();
            match server.iter().position(|row| row.id == photo.id) {
                Some(index) => server[index] = photo,
                None => server.push(photo),
            }
            server = Self::reflagging_main(Self::ordered(server), Some(&photo_id));

            album.photos = self.with_pending_rows(s, server, user_id);
            album.total = (server_total + self.pending_count(s, user_id)).max(album.photos.len() as i64);
            self.upsert(s, album);
        });
    }

    fn fail_addition(&self, user_id: &str, temp_id: &str, message: Option<String>) {
        self.state.with(|s| {
            Self::forget_pending(s, temp_id);

            let mut album =
                s.snapshot.album(user_id).cloned().unwrap_or_else(|| RiseProfilePhotoAlbum::empty(user_id));
            let server: Vec<RiseProfilePhoto> = album.photos.iter().filter(|row| !row.is_pending).cloned().collect();
            album.photos = self.with_pending_rows(s, server, user_id);
            album.total = (album.photos.len() as i64).max(0);
            album.mutation_error = message;
            self.upsert(s, album);
        });
    }

    fn forget_pending(s: &mut RiseUserProfilePhotosRepositoryState, temp_id: &str) {
        s.pending_rows.remove(temp_id);
        s.pending_order.retain(|id| id != temp_id);
    }

    fn pending_count(&self, s: &RiseUserProfilePhotosRepositoryState, user_id: &str) -> i64 {
        if user_id == self.account_id { s.pending_order.len() as i64 } else { 0 }
    }

    /// server rows with whatever this device is still uploading on top
    /// pending rows are newest first, newest carries the main flag
    fn with_pending_rows(
        &self,
        s: &RiseUserProfilePhotosRepositoryState,
        server_photos: Vec<RiseProfilePhoto>,
        user_id: &str,
    ) -> Vec<RiseProfilePhoto> {
        if user_id != self.account_id || s.pending_order.is_empty() {
            return server_photos;
        }
        let pending: Vec<RiseProfilePhoto> =
            s.pending_order.iter().filter_map(|id| s.pending_rows.get(id).cloned()).collect();
        let Some(leading_id) = pending.last().map(|row| row.id.clone()) else {
            return server_photos;
        };
        let mut rows: Vec<RiseProfilePhoto> = pending.into_iter().rev().collect();
        rows.extend(server_photos);
        Self::reflagging_main(rows, Some(&leading_id))
    }

    async fn mirror_account_avatar(&self, logo: &str) {
        let logo = trimming_whitespaces_and_newlines(logo).to_owned();
        if logo.is_empty() {
            return;
        }
        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        let _ = hop
            .run(move |cx: AsyncApp| {
                async move {
                    cx.update(|cx| {
                        UserProfilePhotosInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.settle_account_avatar(logo, cx));
                    });
                    Ok(())
                }
                .boxed_local()
            })
            .await;
    }

    fn addition_failure_message(error: &anyhow::Error) -> Option<String> {
        if error.is::<CancellationError>() {
            return None;
        }
        Some(CanonErrors::refusal_text(error))
    }

    pub async fn set_main(&self, user_id: &str, photo_id: &str) -> anyhow::Result<String> {
        let step = self.state.with(|s| -> anyhow::Result<(RiseUserProfilePhotosRepositoryMutationStep, Option<String>)> {
            if s.is_shutdown {
                return Err(CancellationError.into());
            }
            let Some(mut album) = s.snapshot.album(user_id).cloned() else {
                return Err(RiseUserProfilePhotosError::UnknownAlbum.into());
            };
            let Some(index) = album.photos.iter().position(|photo| photo.id == photo_id && !photo.is_pending) else {
                return Err(RiseUserProfilePhotosError::UnknownAlbum.into());
            };
            let previous_main_id = album.main_photo().map(|photo| photo.id.clone());
            if previous_main_id.as_deref() == Some(photo_id) {
                return Ok((RiseUserProfilePhotosRepositoryMutationStep::Settled(album.photos[index].file_url.clone()), None));
            }

            let generation = Self::bump_mutation_generation(s, user_id);

            album.photos = Self::reflagging_main(album.photos, Some(photo_id));
            album.mutating_ids.insert(photo_id.to_owned());
            album.mutation_error = None;
            self.upsert(s, album.clone());
            Ok((RiseUserProfilePhotosRepositoryMutationStep::Send { album, generation }, previous_main_id))
        })?;
        let (album, generation, previous_main_id) = match step {
            (RiseUserProfilePhotosRepositoryMutationStep::Settled(file_url), _) => return Ok(file_url),
            (RiseUserProfilePhotosRepositoryMutationStep::Send { album, generation }, previous_main_id) => {
                (album, generation, previous_main_id)
            }
        };
        self.persist(user_id).await;

        let result = self
            .rpc
            .set_main_profile_photo(rise_canon::rise_request::user::SetMainProfilePhoto { photo_id: photo_id.to_owned() })
            .await;
        match result {
            Ok(response) => {
                let current = self.state.with(|s| {
                    if !Self::accepts_mutation(s, user_id, generation) {
                        return false;
                    }
                    let mut settled = s.snapshot.album(user_id).cloned().unwrap_or_else(|| album.clone());
                    settled.photos = Self::reflagging_main(settled.photos, Some(photo_id));
                    settled.mutating_ids.remove(photo_id);
                    settled.mutation_error = None;
                    self.upsert(s, settled);
                    true
                });
                if current {
                    self.persist(user_id).await;
                }
                Ok(response.logo)
            }
            Err(error) => {
                let current = self.state.with(|s| {
                    if !Self::accepts_mutation(s, user_id, generation) {
                        return false;
                    }
                    let mut reverted = s.snapshot.album(user_id).cloned().unwrap_or_else(|| album.clone());
                    reverted.photos = Self::reflagging_main(reverted.photos, previous_main_id.as_deref());
                    reverted.mutating_ids.remove(photo_id);
                    reverted.mutation_error = Some(CanonErrors::refusal_text(&error));
                    self.upsert(s, reverted);
                    true
                });
                if current {
                    self.persist(user_id).await;
                }
                Err(error)
            }
        }
    }

    /// removes an album row, deleting the current photo promotes the next one and the reply names it
    pub async fn delete(&self, user_id: &str, photo_id: &str) -> anyhow::Result<String> {
        let step = self.state.with(|s| -> anyhow::Result<(RiseProfilePhotoAlbum, u64, Vec<RiseProfilePhoto>, i64)> {
            if s.is_shutdown {
                return Err(CancellationError.into());
            }
            let Some(mut album) = s.snapshot.album(user_id).cloned() else {
                return Err(RiseUserProfilePhotosError::UnknownAlbum.into());
            };
            let Some(index) = album.photos.iter().position(|photo| photo.id == photo_id && !photo.is_pending) else {
                return Err(RiseUserProfilePhotosError::UnknownAlbum.into());
            };
            let previous_photos = album.photos.clone();
            let previous_total = album.total;

            let generation = Self::bump_mutation_generation(s, user_id);

            let was_main = album.photos[index].is_main;
            album.photos.remove(index);
            if was_main && let Some(first_id) = album.photos.first().map(|photo| photo.id.clone()) {
                album.photos = Self::reflagging_main(album.photos, Some(&first_id));
            }
            album.total = (previous_total - 1).max(0);
            album.mutating_ids.insert(photo_id.to_owned());
            album.mutation_error = None;
            self.upsert(s, album.clone());
            Ok((album, generation, previous_photos, previous_total))
        })?;
        let (album, generation, previous_photos, previous_total) = step;
        self.persist(user_id).await;

        let result = self
            .rpc
            .delete_profile_photo(rise_canon::rise_request::user::DeleteProfilePhoto { photo_id: photo_id.to_owned() })
            .await;
        match result {
            Ok(response) => {
                let current = self.state.with(|s| {
                    if !Self::accepts_mutation(s, user_id, generation) {
                        return false;
                    }
                    let mut settled = s.snapshot.album(user_id).cloned().unwrap_or_else(|| album.clone());
                    settled.photos = Self::reflagging_main(settled.photos, response.main_photo_id.as_deref());
                    settled.total = response.total.max(settled.photos.len() as i64);
                    settled.mutating_ids.remove(photo_id);
                    settled.mutation_error = None;
                    self.upsert(s, settled);
                    true
                });
                if current {
                    self.persist(user_id).await;
                }
                Ok(response.logo)
            }
            Err(error) => {
                let current = self.state.with(|s| {
                    if !Self::accepts_mutation(s, user_id, generation) {
                        return false;
                    }
                    let mut reverted = s.snapshot.album(user_id).cloned().unwrap_or_else(|| album.clone());
                    reverted.photos = previous_photos.clone();
                    reverted.total = previous_total;
                    reverted.mutating_ids.remove(photo_id);
                    reverted.mutation_error = Some(CanonErrors::refusal_text(&error));
                    self.upsert(s, reverted);
                    true
                });
                if current {
                    self.persist(user_id).await;
                }
                Err(error)
            }
        }
    }

    fn bump_mutation_generation(s: &mut RiseUserProfilePhotosRepositoryState, user_id: &str) -> u64 {
        let slot = s.mutation_generations.entry(user_id.to_owned()).or_insert(0);
        *slot = slot.wrapping_add(1);
        *slot
    }

    fn accepts_mutation(s: &RiseUserProfilePhotosRepositoryState, user_id: &str, generation: u64) -> bool {
        !s.is_shutdown && s.mutation_generations.get(user_id) == Some(&generation)
    }

    /// avatar changed some other way, eg settings upload or another device
    /// so the album is a guess, drops its stamp so next open re reads
    pub fn invalidate(&self, user_id: &str) {
        self.state.with(|s| {
            s.refreshed_at_ms.insert(user_id.to_owned(), 0);
            s.generation_clock = s.generation_clock.wrapping_add(1);
            s.generations.insert(user_id.to_owned(), s.generation_clock);
            if let Some(task) = s.in_flight.remove(user_id) {
                task.abort.abort();
            }
        });
    }

    pub async fn shutdown(&self) {
        let proceed = self.state.with(|s| {
            if s.is_shutdown {
                return false;
            }
            s.is_shutdown = true;
            for (_, task) in s.in_flight.drain() {
                task.abort.abort();
            }
            if let Some(task) = s.addition_task.take() {
                task.abort();
            }
            s.addition_queue.clear();
            s.pending_rows.clear();
            s.pending_order.clear();
            s.generations.clear();
            s.mutation_generations.clear();
            s.refreshed_at_ms.clear();
            true
        });
        if proceed {
            self.resource.shutdown().await;
        }
    }

    fn finish_with_load_error(&self, s: &mut RiseUserProfilePhotosRepositoryState, user_id: &str, message: &str) {
        let mut album =
            s.snapshot.album(user_id).cloned().unwrap_or_else(|| RiseProfilePhotoAlbum::empty(user_id));
        album.is_refreshing = false;
        album.load_state = if album.photos.is_empty() {
            RiseUserProfilePhotosLoadState::Failed
        } else {
            RiseUserProfilePhotosLoadState::Loaded
        };
        album.error = Self::message(message);
        self.upsert(s, album);
    }

    async fn persist(&self, user_id: &str) {
        let payload = self.state.with(|s| {
            s.snapshot.album(user_id).map(|album| RiseUserProfilePhotosResourcePayload {
                photos: album.photos.iter().filter(|photo| !photo.is_pending).cloned().collect(),
                total: album.total,
                has_more: album.has_more,
                next_cursor: album.next_cursor.clone(),
                visible: album.is_visible,
            })
        });
        let Some(payload) = payload else {
            return;
        };
        let _ = self.resource.store(&payload, user_id).await;
    }

    fn upsert(&self, s: &mut RiseUserProfilePhotosRepositoryState, album: RiseProfilePhotoAlbum) {
        let mut albums = s.snapshot.albums.clone();
        match albums.iter().position(|existing| existing.user_id == album.user_id) {
            Some(index) => albums[index] = album,
            None => {
                albums.push(album);
                if albums.len() > Self::MAXIMUM_ALBUMS {
                    albums.drain(..albums.len() - Self::MAXIMUM_ALBUMS);
                }
            }
        }
        self.publish(s, RiseUserProfilePhotosSnapshot { albums });
    }

    fn publish(&self, s: &mut RiseUserProfilePhotosRepositoryState, next: RiseUserProfilePhotosSnapshot) {
        if next == s.snapshot {
            return;
        }
        s.snapshot = next.clone();

        let Some(hop) = RiseEngineMainHop::installed() else {
            return;
        };
        let account_id = self.account_id.clone();
        let account_epoch = self.account_epoch.clone();
        let session_id = self.session_id;
        hop.post(move |cx| {
            RiseUserProfilePhotosPresentation::shared(cx).update(cx, |presentation, cx| {
                presentation.publish(next, &account_id, &account_epoch, session_id, cx);
            });
        });
    }

    fn is_shutdown(&self) -> bool {
        self.state.with(|s| s.is_shutdown)
    }

    /// newest first,
    /// tie broken by id,
    /// same as server paging
    fn ordered(mut photos: Vec<RiseProfilePhoto>) -> Vec<RiseProfilePhoto> {
        photos.sort_by(|lhs, rhs| rhs.created_at_ms.cmp(&lhs.created_at_ms).then_with(|| rhs.id.cmp(&lhs.id)));
        photos
    }

    /// moves the main flag without touching order, none clears it as with a generated placeholder avatar
    fn reflagging_main(photos: Vec<RiseProfilePhoto>, photo_id: Option<&str>) -> Vec<RiseProfilePhoto> {
        photos
            .into_iter()
            .map(|mut photo| {
                photo.is_main = Some(photo.id.as_str()) == photo_id;
                photo
            })
            .collect()
    }

    fn is_transport_unavailable(error: &anyhow::Error) -> bool {
        matches!(
            error.downcast_ref::<RiseWireError>(),
            Some(RiseWireError::NoSocketSession | RiseWireError::NotConnected | RiseWireError::Suspended)
        )
    }

    fn message(raw: &str) -> Option<String> {
        let trimmed = trimming_whitespaces_and_newlines(raw);
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    }

    fn now_milliseconds() -> i64 {
        Date::now().ms()
    }
}
