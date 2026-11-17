use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

use futures::FutureExt;
use futures::future::{BoxFuture, Shared};
use gpui::{App, Context, Entity, Task};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_file_url::is_local_absolute_path;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::tr;
use rise_platform::AppPaths;
use rise_platform::file_manager::{CocoaError, CocoaErrorCode, DirectoryEnumerationOptions, FileManager};
use rise_platform::url_resource_values::{URLResourceValues, set_resource_values};
use serde::{Deserialize, Serialize};

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::lib::sai::sai::Sai;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierType,
};
use crate::modules::music::shared::utils::music::music_models::MusicLocalTrackTransferPhase;
use crate::modules::music::shared::utils::music::music_track_upload_notifier::MusicTrackUploadNotifier;
use crate::modules::music::shared::utils::music::music_playback_disk_cache::MusicPlaybackDiskCache;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MusicLocalTrackDraftRecord {
    version: i64,
    account_user_id: String,
    request_id: String,
    temp_id: String,
    playlist_id: String,
    title: String,
    description: Option<String>,
    artist_name: String,
    duration_ms: i64,
    audio_filename: String,
    audio_content_type: String,
    audio_byte_count: i64,
    cover_filename: Option<String>,
    cover_content_type: Option<String>,
    cover_byte_count: i64,
    created_at_ms: i64,
    phase: String,
    error: Option<String>,
}

impl MusicLocalTrackDraftRecord {
    const CURRENT_VERSION: i64 = 1;
}

#[derive(Clone, Debug)]
struct MusicPreparedLocalTrackDraft {
    record: MusicLocalTrackDraftRecord,
    audio_url: PathBuf,
    cover_url: Option<PathBuf>,
}

/// arguments of persist draft, owned so the copy can run off the main thread
struct MusicLocalTrackDraftInput {
    account_user_id: String,
    request_id: String,
    temp_id: String,
    playlist_id: String,
    title: String,
    description: Option<String>,
    artist_name: String,
    duration_ms: i64,
    audio_file: SaiWsOptimisticLocalFile,
    cover_file: Option<SaiWsOptimisticLocalFile>,
}

/// chain of record writes, each waits for the previous one
type MusicLocalTrackDraftPersistence = Shared<BoxFuture<'static, ()>>;

pub type MusicLocalTrackDraftCreationObserver = Box<dyn FnOnce(RiseStoredMusicTrack, &mut App)>;
pub type MusicLocalTrackDraftErrorHandler = Rc<dyn Fn(String, &mut App)>;

static DATA_ROOT: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    let paths = AppPaths::resolve(AppEnvironment::current().canon()).ok()?;
    paths.create_all().ok()?;
    Some(paths.data_dir().to_path_buf())
});

static NULL_DEVICE_URL: LazyLock<Url> =
    LazyLock::new(|| Url::from_file_path(Path::new("/dev/null")).expect("the null device path is a file URL"));

/// owns the only durable copy of an mp3 that hasnt reached the music service
/// kept out of the evictable cache so eviction cant make a row impossible to retry
pub struct MusicLocalTrackDraftStore {
    records_by_temp_id: HashMap<String, MusicLocalTrackDraftRecord>,
    creation_observers: HashMap<String, MusicLocalTrackDraftCreationObserver>,
    failure_observers: HashMap<String, MusicLocalTrackDraftErrorHandler>,
    upload_in_flight: HashSet<String>,
    queued_upload_temp_ids: Vec<String>,
    queue_drain_task: Option<Task<()>>,
    record_persistence_task: Option<MusicLocalTrackDraftPersistence>,
    pending_persistence_temp_ids: HashSet<String>,
    loaded_account_user_id: Option<String>,
    restore_generation: Uuid,
    legacy_adoption_in_flight: HashSet<String>,
    finished_records_awaiting_adoption: HashMap<String, MusicLocalTrackDraftRecord>,
}

impl MusicLocalTrackDraftStore {
    const ADOPTION_FALLBACK_MILLISECONDS: u64 = 5_000;
    const QUEUE_DRAIN_MILLISECONDS: u64 = 350;
    const AUDIO_FILE_NAME: &str = "audio.mp3";
    const RECORD_FILE_NAME: &str = "record.json";
    const COVER_STEM: &str = "cover";

    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            records_by_temp_id: HashMap::new(),
            creation_observers: HashMap::new(),
            failure_observers: HashMap::new(),
            upload_in_flight: HashSet::new(),
            queued_upload_temp_ids: Vec::new(),
            queue_drain_task: None,
            record_persistence_task: None,
            pending_persistence_temp_ids: HashSet::new(),
            loaded_account_user_id: None,
            restore_generation: Uuid::new_v4(),
            legacy_adoption_in_flight: HashSet::new(),
            finished_records_awaiting_adoption: HashMap::new(),
        }
    }

    pub fn observe_creation(&mut self, temp_id: &str, handler: MusicLocalTrackDraftCreationObserver) {
        self.creation_observers.insert(temp_id.to_owned(), handler);
    }

    pub fn clear_creation_observer(&mut self, temp_id: &str) {
        self.creation_observers.remove(temp_id);
        self.failure_observers.remove(temp_id);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn enqueue(
        &mut self,
        request_id: &str,
        temp_id: &str,
        playlist_id: &str,
        title: &str,
        description: Option<&str>,
        artist_name: &str,
        duration_ms: i64,
        local_audio_file: SaiWsOptimisticLocalFile,
        local_cover_file: Option<SaiWsOptimisticLocalFile>,
        on_accepted: Option<Box<dyn FnOnce(&mut App)>>,
        on_error: Option<MusicLocalTrackDraftErrorHandler>,
        cx: &mut Context<Self>,
    ) {
        let account_user_id = match SaiAccountCacheScope::active_user_id() {
            Some(user_id) if SaiAccountCacheScope::is_current_account(Some(&user_id)) => user_id,
            _ => {
                Self::report_error(on_error, tr("music_message_import_account_changed"), cx);
                return;
            }
        };

        let account_epoch = SaiAccountCacheScope::active_epoch();
        let normalized_request_id = request_id.to_lowercase();
        if self.creation_observers.contains_key(temp_id)
            && let Some(on_error) = on_error.as_ref()
        {
            self.failure_observers.insert(temp_id.to_owned(), on_error.clone());
        }
        self.pending_persistence_temp_ids.insert(temp_id.to_owned());
        let input = MusicLocalTrackDraftInput {
            account_user_id: account_user_id.clone(),
            request_id: normalized_request_id,
            temp_id: temp_id.to_owned(),
            playlist_id: playlist_id.to_owned(),
            title: title.to_owned(),
            description: description.map(str::to_owned),
            artist_name: artist_name.to_owned(),
            duration_ms,
            audio_file: local_audio_file,
            cover_file: local_cover_file,
        };
        let temp_id = temp_id.to_owned();
        cx.spawn(async move |this, cx| {
            let prepared = Self::detached(move || Self::persist_draft(&input)).await;
            let _ = this.update(cx, |this, cx| {
                let is_current =
                    SaiAccountCacheScope::is_current_account_epoch(Some(&account_user_id), account_epoch.as_deref());
                if is_current {
                    this.pending_persistence_temp_ids.remove(&temp_id);
                }
                match prepared {
                    Some(Ok(prepared)) => {
                        if !is_current {
                            Self::remove_draft_directory(prepared.record);
                            return;
                        }
                        this.records_by_temp_id
                            .insert(temp_id.clone(), prepared.record.clone());
                        let phase = if Self::is_connected(cx) {
                            MusicLocalTrackTransferPhase::Uploading
                        } else {
                            MusicLocalTrackTransferPhase::WaitingForConnection
                        };
                        this.install_visible_draft(&prepared.record, phase, None, cx);
                        if let Some(on_accepted) = on_accepted {
                            cx.defer(on_accepted);
                        }

                        if Self::is_connected(cx) {
                            this.start_upload(prepared.record, cx);
                        } else {
                            this.set_state(
                                &prepared.record,
                                MusicLocalTrackTransferPhase::WaitingForConnection,
                                None,
                                cx,
                            );
                        }
                    }
                    None => {
                        if !is_current {
                            return;
                        }
                        Self::report_error(on_error, tr("music_message_import_account_changed"), cx);
                    }
                    Some(Err(error)) => {
                        if !is_current {
                            return;
                        }
                        let description = error.to_string();
                        let logged = description.clone();
                        Sai::log(
                            "Music",
                            move || format!("local MP3 draft persistence failed: {logged}"),
                            SaiLogKind::Cache,
                            cx,
                        );
                        Self::report_error(on_error, description, cx);
                    }
                }
            });
        })
        .detach();
    }

    pub fn restore_drafts(&mut self, playlist_id: &str, cx: &mut Context<Self>) {
        let Some(account_user_id) = SaiAccountCacheScope::active_user_id() else {
            return;
        };
        if self.loaded_account_user_id.as_deref() == Some(account_user_id.as_str()) {
            let records: Vec<MusicLocalTrackDraftRecord> = self
                .records_by_temp_id
                .values()
                .filter(|record| record.playlist_id == playlist_id)
                .cloned()
                .collect();
            for record in records {
                self.restore_visible_draft(record, cx);
            }
            self.adopt_legacy_pending_drafts(playlist_id, &account_user_id, cx);
            return;
        }

        let generation = Uuid::new_v4();
        self.restore_generation = generation;
        let pending_persistence = self.record_persistence_task.clone();
        let playlist_id = playlist_id.to_owned();
        cx.spawn(async move |this, cx| {
            let loading_user_id = account_user_id.clone();
            let Some(records) =
                Self::after_persistence(pending_persistence, move || Self::load_records(&loading_user_id)).await
            else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                if this.restore_generation != generation
                    || !SaiAccountCacheScope::is_current_account(Some(&account_user_id))
                {
                    return;
                }

                this.loaded_account_user_id = Some(account_user_id.clone());
                let mut records_by_temp_id: HashMap<String, MusicLocalTrackDraftRecord> = HashMap::new();
                for record in &records {
                    let keeps_existing = records_by_temp_id
                        .get(&record.temp_id)
                        .is_some_and(|existing| record.created_at_ms < existing.created_at_ms);
                    if !keeps_existing {
                        records_by_temp_id.insert(record.temp_id.clone(), record.clone());
                    }
                }
                this.records_by_temp_id = records_by_temp_id;
                for record in records.into_iter().filter(|record| record.playlist_id == playlist_id) {
                    this.restore_visible_draft(record, cx);
                }
                this.adopt_legacy_pending_drafts(&playlist_id, &account_user_id, cx);
            });
        })
        .detach();
    }

    pub fn retry(&mut self, track_id: &str, cx: &mut Context<Self>) {
        let retried_id = track_id.to_owned();
        self.with_record(
            track_id,
            move |this, record, cx| {
                if this.upload_in_flight.contains(&retried_id) {
                    return;
                }
                if !Self::is_connected(cx) {
                    this.set_state(&record, MusicLocalTrackTransferPhase::WaitingForConnection, None, cx);
                    Notifier::send(
                        tr("music_local_upload_waiting_for_connection"),
                        NotifierSendOptions {
                            r#type: NotifierType::Info,
                            position: NotifierPosition::Bottom,
                            icon: NotifierIcon::System("wifi.slash".to_owned()),
                            ..NotifierSendOptions::default()
                        },
                        cx,
                    );
                    return;
                }
                this.start_upload(record, cx);
            },
            |cx| {
                Notifier::send(
                    tr("music_local_upload_device_file_missing"),
                    NotifierSendOptions {
                        r#type: NotifierType::Error,
                        position: NotifierPosition::Bottom,
                        ..NotifierSendOptions::default()
                    },
                    cx,
                );
            },
            cx,
        );
    }

    /// removed row must vanish even mid flight or with a record on another playlist
    /// its device local so refusing leaves an undeletable track
    pub fn remove(&mut self, track_id: &str, playlist_id: Option<&str>, cx: &mut Context<Self>) {
        MusicTrackUploadNotifier::finish(track_id, cx);
        Self::purge_visible_draft(track_id, playlist_id, cx);
        let removed_id = track_id.to_owned();
        let requested_playlist_id = playlist_id.map(str::to_owned);
        self.with_record(
            track_id,
            move |this, record, cx| {
                this.records_by_temp_id.remove(&removed_id);
                this.upload_in_flight.remove(&removed_id);
                this.queued_upload_temp_ids.retain(|temp_id| temp_id != &removed_id);
                if requested_playlist_id.as_deref() != Some(record.playlist_id.as_str()) {
                    Self::purge_visible_draft(&removed_id, Some(&record.playlist_id), cx);
                }
                Self::remove_draft_directory(record);
                this.drain_upload_queue(cx);
            },
            |_| {},
            cx,
        );
    }

    fn purge_visible_draft(temp_id: &str, playlist_id: Option<&str>, cx: &mut App) {
        let Some(playlist_id) = playlist_id.filter(|playlist_id| !playlist_id.is_empty()) else {
            return;
        };
        MusicServicesStore::shared(cx).update(cx, |services, cx| {
            services.cancel_pending_local_track(playlist_id, temp_id, cx);
        });
    }

    pub fn interrupt_for_account_change(&mut self) -> bool {
        let mut interrupted = !self.pending_persistence_temp_ids.is_empty();
        let records: Vec<MusicLocalTrackDraftRecord> = self.records_by_temp_id.values().cloned().collect();
        for record in records {
            let Some(phase) = MusicLocalTrackTransferPhase::from_raw(&record.phase) else {
                continue;
            };
            if phase == MusicLocalTrackTransferPhase::Failed {
                continue;
            }
            let interrupted_record = Self::updated_record(
                &record,
                MusicLocalTrackTransferPhase::Failed,
                Some(tr("music_message_import_account_changed")),
            );
            self.persist_record_asynchronously(interrupted_record);
            interrupted = true;
        }
        interrupted
    }

    pub fn reset_account_state(&mut self, cx: &mut Context<Self>) {
        MusicTrackUploadNotifier::finish_all(cx);
        self.restore_generation = Uuid::new_v4();
        self.pending_persistence_temp_ids.clear();
        self.records_by_temp_id.clear();
        self.creation_observers.clear();
        self.failure_observers.clear();
        self.upload_in_flight.clear();
        self.queued_upload_temp_ids.clear();
        self.queue_drain_task = None;
        self.loaded_account_user_id = None;
        self.legacy_adoption_in_flight.clear();
        let stranded: Vec<MusicLocalTrackDraftRecord> =
            self.finished_records_awaiting_adoption.drain().map(|(_, record)| record).collect();
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            for record in &stranded {
                Self::remove_draft_directory_now(record);
            }
        }));
    }

    fn adopt_uploaded_audio_into_playback_cache(&mut self, temp_id: &str, server_track_id: Option<&str>) {
        let Some(record) = self.finished_records_awaiting_adoption.remove(temp_id) else {
            return;
        };
        let track_id = server_track_id
            .map(trimming_whitespaces_and_newlines)
            .unwrap_or("")
            .to_owned();
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            if !track_id.is_empty()
                && let Some(prepared) = Self::prepared_draft(&record)
            {
                MusicPlaybackDiskCache::adopt_external_file(
                    &track_id,
                    &prepared.audio_url,
                    Some("mp3"),
                    Some(&record.account_user_id),
                );
            }
            Self::remove_draft_directory_now(&record);
        }));
    }

    fn release_stranded_adoption_record(&mut self, temp_id: &str) {
        let Some(record) = self.finished_records_awaiting_adoption.remove(temp_id) else {
            return;
        };
        Self::remove_draft_directory(record);
    }

    pub fn remove_persisted_account(&self, user_id: &str) {
        let normalized = trimming_whitespaces_and_newlines(user_id).to_owned();
        if normalized.is_empty() {
            return;
        }
        drop(RiseAccountRuntime::current().spawn_blocking(move || {
            let Ok(directory) = Self::account_directory(&normalized, false) else {
                return;
            };
            let _ = FileManager.remove_item(&directory);
        }));
    }

    fn start_upload(&mut self, source_record: MusicLocalTrackDraftRecord, cx: &mut Context<Self>) {
        let account_epoch = SaiAccountCacheScope::active_epoch();
        if !SaiAccountCacheScope::is_current_account_epoch(
            Some(&source_record.account_user_id),
            account_epoch.as_deref(),
        ) {
            return;
        }
        if self.upload_in_flight.contains(&source_record.temp_id) {
            return;
        }
        let Some(prepared) = Self::prepared_draft(&source_record) else {
            self.set_state(
                &source_record,
                MusicLocalTrackTransferPhase::Failed,
                Some(tr("music_local_upload_device_file_missing")),
                cx,
            );
            return;
        };
        if !Self::is_connected(cx) {
            self.set_state(&source_record, MusicLocalTrackTransferPhase::WaitingForConnection, None, cx);
            return;
        }
        if !self.upload_in_flight.is_empty() || Self::is_create_local_track_pending(cx) {
            if !self.queued_upload_temp_ids.contains(&source_record.temp_id) {
                self.queued_upload_temp_ids.push(source_record.temp_id.clone());
            }
            self.set_state(&source_record, MusicLocalTrackTransferPhase::Queued, None, cx);
            self.schedule_upload_queue_drain(cx);
            return;
        }
        let (Some(audio_url), cover_url) = (
            Url::from_file_path(&prepared.audio_url),
            prepared.cover_url.as_deref().and_then(Url::from_file_path),
        ) else {
            self.set_state(
                &source_record,
                MusicLocalTrackTransferPhase::Failed,
                Some(tr("music_local_upload_device_file_missing")),
                cx,
            );
            return;
        };

        self.upload_in_flight.insert(source_record.temp_id.clone());
        self.queued_upload_temp_ids
            .retain(|temp_id| temp_id != &source_record.temp_id);
        let record = Self::updated_record(&source_record, MusicLocalTrackTransferPhase::Uploading, None);
        self.records_by_temp_id
            .insert(record.temp_id.clone(), record.clone());
        self.persist_record_asynchronously(record.clone());
        self.install_visible_draft(&record, MusicLocalTrackTransferPhase::Uploading, None, cx);
        MusicTrackUploadNotifier::begin(&record.temp_id, cx);

        let audio_file = SaiWsOptimisticLocalFile {
            file_context_override: Some("music_audio".to_owned()),
            byte_count: Some(record.audio_byte_count),
            ..SaiWsOptimisticLocalFile::new(
                audio_url,
                record.audio_filename.clone(),
                record.audio_content_type.clone(),
            )
        };
        let cover_file = cover_url.map(|url| {
            let filename = record
                .cover_filename
                .clone()
                .unwrap_or_else(|| url.last_path_component());
            SaiWsOptimisticLocalFile {
                file_context_override: Some("music_cover".to_owned()),
                byte_count: Some(record.cover_byte_count),
                ..SaiWsOptimisticLocalFile::new(
                    url,
                    filename,
                    record
                        .cover_content_type
                        .clone()
                        .unwrap_or_else(|| "image/jpeg".to_owned()),
                )
            }
        });
        let optimistic_track = self.visible_draft(&record, MusicLocalTrackTransferPhase::Uploading, None, cx);

        let weak = cx.weak_entity();
        let success_owner = weak.clone();
        let success_record = record.clone();
        let success_epoch = account_epoch.clone();
        let on_success: Box<dyn FnOnce(&mut App)> = Box::new(move |cx: &mut App| {
            cx.defer(move |cx| {
                let record = success_record;
                let account_epoch = success_epoch;
                if !SaiAccountCacheScope::is_current_account_epoch(
                    Some(&record.account_user_id),
                    account_epoch.as_deref(),
                ) {
                    return;
                }
                let _ = success_owner.update(cx, |this, cx| {
                    this.upload_in_flight.remove(&record.temp_id);
                    MusicTrackUploadNotifier::succeed(
                        &record.temp_id,
                        tr("music_message_import_success_title"),
                        Some(tr("music_message_import_success_generic")),
                        cx,
                    );
                    if this.records_by_temp_id.remove(&record.temp_id).is_some() {
                        this.finished_records_awaiting_adoption
                            .insert(record.temp_id.clone(), record.clone());
                        let temp_id = record.temp_id.clone();
                        let user_id = record.account_user_id.clone();
                        let epoch = account_epoch.clone();
                        cx.spawn(async move |this, cx| {
                            cx.background_executor()
                                .timer(Duration::from_millis(Self::ADOPTION_FALLBACK_MILLISECONDS))
                                .await;
                            if !SaiAccountCacheScope::is_current_account_epoch(Some(&user_id), epoch.as_deref()) {
                                return;
                            }
                            let _ = this.update(cx, |this, _| this.release_stranded_adoption_record(&temp_id));
                        })
                        .detach();
                    }
                    this.drain_upload_queue(cx);
                });
            });
        });

        let created_owner = weak.clone();
        let created_record = record.clone();
        let created_epoch = account_epoch.clone();
        let on_created: Box<dyn FnOnce(RiseStoredMusicTrack, &mut App)> =
            Box::new(move |track: RiseStoredMusicTrack, cx: &mut App| {
                cx.defer(move |cx| {
                    if !SaiAccountCacheScope::is_current_account_epoch(
                        Some(&created_record.account_user_id),
                        created_epoch.as_deref(),
                    ) {
                        return;
                    }
                    let observer = created_owner
                        .update(cx, |this, _| {
                            this.failure_observers.remove(&created_record.temp_id);
                            this.creation_observers.remove(&created_record.temp_id)
                        })
                        .ok()
                        .flatten();
                    let server_track_id = track.id.clone();
                    if let Some(observer) = observer {
                        observer(track, cx);
                    }
                    let _ = created_owner.update(cx, |this, _| {
                        this.adopt_uploaded_audio_into_playback_cache(
                            &created_record.temp_id,
                            server_track_id.as_deref(),
                        );
                    });
                });
            });

        let error_owner = weak;
        let error_record = record.clone();
        let error_epoch = account_epoch;
        let on_error: Box<dyn FnOnce(String, &mut App)> = Box::new(move |message: String, cx: &mut App| {
            cx.defer(move |cx| {
                if !SaiAccountCacheScope::is_current_account_epoch(
                    Some(&error_record.account_user_id),
                    error_epoch.as_deref(),
                ) {
                    return;
                }
                let failure_observer = error_owner
                    .update(cx, |this, cx| {
                        this.upload_in_flight.remove(&error_record.temp_id);
                        let mut failure_observer = None;
                        if this.records_by_temp_id.contains_key(&error_record.temp_id) {
                            let phase = if Self::is_connected(cx) {
                                MusicLocalTrackTransferPhase::Failed
                            } else {
                                MusicLocalTrackTransferPhase::WaitingForConnection
                            };
                            let error = (phase == MusicLocalTrackTransferPhase::Failed).then(|| message.clone());
                            this.set_state(&error_record, phase, error, cx);
                            if phase == MusicLocalTrackTransferPhase::Failed {
                                MusicTrackUploadNotifier::fail(
                                    &error_record.temp_id,
                                    tr("music_message_import_failed"),
                                    Some(message.clone()),
                                    cx,
                                );
                                this.creation_observers.remove(&error_record.temp_id);
                                failure_observer = this.failure_observers.remove(&error_record.temp_id);
                            }
                        }
                        this.drain_upload_queue(cx);
                        failure_observer
                    })
                    .ok()
                    .flatten();
                if let Some(failure_observer) = failure_observer {
                    failure_observer(message, cx);
                }
            });
        });

        MusicActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.create_local_track_in_playlist_action(
                &record.request_id,
                &record.temp_id,
                &record.playlist_id,
                &record.title,
                record.description.as_deref(),
                &record.artist_name,
                record.duration_ms,
                audio_file,
                cover_file,
                Some(optimistic_track),
                None,
                false,
                None,
                Some(on_success),
                Some(on_created),
                Some(on_error),
                cx,
            );
        });
    }

    fn drain_upload_queue(&mut self, cx: &mut Context<Self>) {
        if !self.upload_in_flight.is_empty() || !Self::is_connected(cx) {
            return;
        }
        if Self::is_create_local_track_pending(cx) {
            self.schedule_upload_queue_drain(cx);
            return;
        }
        while !self.queued_upload_temp_ids.is_empty() {
            let temp_id = self.queued_upload_temp_ids.remove(0);
            let Some(record) = self.records_by_temp_id.get(&temp_id).cloned() else {
                continue;
            };
            self.start_upload(record, cx);
            return;
        }
    }

    fn schedule_upload_queue_drain(&mut self, cx: &mut Context<Self>) {
        if self.queue_drain_task.is_some() {
            return;
        }
        self.queue_drain_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(Self::QUEUE_DRAIN_MILLISECONDS))
                .await;
            let _ = this.update(cx, |this, cx| {
                if let Some(task) = this.queue_drain_task.take() {
                    task.detach();
                }
                this.drain_upload_queue(cx);
            });
        }));
    }

    fn restore_visible_draft(&mut self, record: MusicLocalTrackDraftRecord, cx: &mut Context<Self>) {
        if Self::prepared_draft(&record).is_none() {
            self.records_by_temp_id.remove(&record.temp_id);
            MusicServicesStore::shared(cx).update(cx, |services, cx| {
                services.remove_local_track_optimistic(&record.playlist_id, &record.temp_id, cx);
            });
            Self::remove_draft_directory(record);
            return;
        }

        if self.upload_in_flight.contains(&record.temp_id) {
            self.set_state(&record, MusicLocalTrackTransferPhase::Uploading, None, cx);
            return;
        }
        if self.queued_upload_temp_ids.contains(&record.temp_id) {
            self.set_state(&record, MusicLocalTrackTransferPhase::Queued, None, cx);
            return;
        }

        let saved_phase =
            MusicLocalTrackTransferPhase::from_raw(&record.phase).unwrap_or(MusicLocalTrackTransferPhase::Failed);
        if saved_phase == MusicLocalTrackTransferPhase::WaitingForConnection && Self::is_connected(cx) {
            self.start_upload(record, cx);
            return;
        }
        let (restored_phase, restored_error) = match saved_phase {
            MusicLocalTrackTransferPhase::Queued
            | MusicLocalTrackTransferPhase::Downloading
            | MusicLocalTrackTransferPhase::Preparing
            | MusicLocalTrackTransferPhase::Uploading => {
                if Self::is_connected(cx) {
                    (
                        MusicLocalTrackTransferPhase::Failed,
                        Some(tr("music_local_upload_interrupted")),
                    )
                } else {
                    (MusicLocalTrackTransferPhase::WaitingForConnection, None)
                }
            }
            MusicLocalTrackTransferPhase::WaitingForConnection | MusicLocalTrackTransferPhase::Failed => {
                (saved_phase, record.error.clone())
            }
        };
        self.set_state(&record, restored_phase, restored_error, cx);
    }

    fn install_visible_draft(
        &mut self,
        record: &MusicLocalTrackDraftRecord,
        phase: MusicLocalTrackTransferPhase,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let track = self.visible_draft(record, phase, error, cx);
        MusicServicesStore::shared(cx).update(cx, |services, cx| {
            services.insert_local_track_optimistic(&record.playlist_id, track, None, cx);
        });
    }

    fn visible_draft(
        &self,
        record: &MusicLocalTrackDraftRecord,
        phase: MusicLocalTrackTransferPhase,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) -> RiseStoredMusicTrack {
        let services = MusicServicesStore::shared(cx);
        let prepared = Self::prepared_draft(record);
        let audio_url = prepared
            .as_ref()
            .and_then(|prepared| Url::from_file_path(&prepared.audio_url));
        let (Some(prepared), Some(audio_url)) = (prepared, audio_url) else {
            let missing_error = tr("music_local_upload_device_file_missing");
            return services.read(cx).make_local_track_draft(
                &record.temp_id,
                &record.title,
                record.description.as_deref(),
                &record.artist_name,
                record.duration_ms,
                &record.account_user_id,
                &NULL_DEVICE_URL,
                "",
                None,
                None,
                None,
                Some(MusicLocalTrackTransferPhase::Failed),
                0,
                0,
                Some(&missing_error),
            );
        };
        let cover_url = prepared
            .cover_url
            .as_deref()
            .and_then(Url::from_file_path)
            .map(|url| url.absolute_string().to_owned())
            .unwrap_or_default();
        services.read(cx).make_local_track_draft(
            &record.temp_id,
            &record.title,
            record.description.as_deref(),
            &record.artist_name,
            record.duration_ms,
            &record.account_user_id,
            &audio_url,
            &cover_url,
            None,
            None,
            None,
            Some(phase),
            0,
            record.audio_byte_count + record.cover_byte_count,
            error.as_deref(),
        )
    }

    fn set_state(
        &mut self,
        source_record: &MusicLocalTrackDraftRecord,
        phase: MusicLocalTrackTransferPhase,
        error: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let record = Self::updated_record(source_record, phase, error.clone());
        self.records_by_temp_id
            .insert(record.temp_id.clone(), record.clone());
        self.persist_record_asynchronously(record.clone());
        self.install_visible_draft(&record, phase, error, cx);
    }

    fn updated_record(
        source: &MusicLocalTrackDraftRecord,
        phase: MusicLocalTrackTransferPhase,
        error: Option<String>,
    ) -> MusicLocalTrackDraftRecord {
        let mut record = source.clone();
        record.phase = phase.raw_value().to_owned();
        record.error = error;
        record
    }

    fn with_record(
        &mut self,
        track_id: &str,
        action: impl FnOnce(&mut Self, MusicLocalTrackDraftRecord, &mut Context<Self>) + 'static,
        on_missing: impl FnOnce(&mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        if let Some(record) = self.records_by_temp_id.get(track_id).cloned() {
            action(self, record, cx);
            return;
        }
        let Some(account_user_id) = SaiAccountCacheScope::active_user_id() else {
            return;
        };
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let pending_persistence = self.record_persistence_task.clone();
        let track_id = track_id.to_owned();
        cx.spawn(async move |this, cx| {
            let loading_user_id = account_user_id.clone();
            let Some(record) = Self::after_persistence(pending_persistence, move || {
                Self::load_records(&loading_user_id)
                    .into_iter()
                    .find(|record| record.temp_id == track_id)
            })
            .await
            else {
                return;
            };
            if !SaiAccountCacheScope::is_current_account_epoch(Some(&account_user_id), account_epoch.as_deref()) {
                return;
            }
            let Some(record) = record else {
                cx.update(on_missing);
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.records_by_temp_id
                    .insert(record.temp_id.clone(), record.clone());
                action(this, record, cx);
            });
        })
        .detach();
    }

    fn adopt_legacy_pending_drafts(&mut self, playlist_id: &str, account_user_id: &str, cx: &mut Context<Self>) {
        let services = MusicServicesStore::shared(cx);
        let tracks = services
            .update(cx, |services, cx| services.message_mp3_pending_playlist(playlist_id, cx))
            .and_then(|playlist| playlist.tracks)
            .unwrap_or_default();
        let candidates: Vec<RiseStoredMusicTrack> = tracks
            .into_iter()
            .filter(|track| {
                let Some(temp_id) = track.id.as_deref() else {
                    return false;
                };
                if temp_id.is_empty()
                    || track.source_chat_id.is_some()
                    || track.transfer_phase.is_none()
                    || self.records_by_temp_id.contains_key(temp_id)
                    || self.legacy_adoption_in_flight.contains(temp_id)
                {
                    return false;
                }
                Self::local_file_url(track.audio_url.as_deref()).is_some()
            })
            .collect();
        if candidates.is_empty() {
            return;
        }

        let candidate_ids: HashSet<String> = candidates.iter().filter_map(|track| track.id.clone()).collect();
        self.legacy_adoption_in_flight
            .extend(candidate_ids.iter().cloned());
        let connected = Self::is_connected(cx);
        let interrupted_error = tr("music_local_upload_interrupted");
        let playlist_id = playlist_id.to_owned();
        let account_user_id = account_user_id.to_owned();
        cx.spawn(async move |this, cx| {
            let adopting_user_id = account_user_id.clone();
            let adopted = Self::detached(move || {
                let mut known: HashSet<String> = Self::load_records(&adopting_user_id)
                    .into_iter()
                    .map(|record| record.temp_id)
                    .collect();
                candidates
                    .iter()
                    .filter_map(|track| {
                        let temp_id = track.id.as_deref()?;
                        if known.contains(temp_id) {
                            return None;
                        }
                        let record = Self::adopt_legacy_pending_draft(
                            track,
                            &playlist_id,
                            &adopting_user_id,
                            connected,
                            &interrupted_error,
                        )?;
                        known.insert(temp_id.to_owned());
                        Some(record)
                    })
                    .collect::<Vec<MusicLocalTrackDraftRecord>>()
            })
            .await
            .unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.legacy_adoption_in_flight
                    .retain(|temp_id| !candidate_ids.contains(temp_id));
                if !SaiAccountCacheScope::is_current_account(Some(&account_user_id)) {
                    return;
                }
                for record in adopted {
                    this.records_by_temp_id
                        .insert(record.temp_id.clone(), record.clone());
                    this.restore_visible_draft(record, cx);
                }
            });
        })
        .detach();
    }

    fn persist_record_asynchronously(&mut self, record: MusicLocalTrackDraftRecord) {
        let previous = self.record_persistence_task.take();
        let write = RiseAccountRuntime::current().spawn(async move {
            if let Some(previous) = previous {
                previous.await;
            }
            let _ = RiseAccountRuntime::current()
                .spawn_blocking(move || Self::write_record(&record))
                .await;
        });
        self.record_persistence_task = Some(
            async move {
                let _ = write.await;
            }
            .boxed()
            .shared(),
        );
    }

    fn persist_draft(input: &MusicLocalTrackDraftInput) -> anyhow::Result<MusicPreparedLocalTrackDraft> {
        let directory = Self::draft_directory(&input.account_user_id, &input.request_id, true)?;
        let prepared = Self::copy_draft_files(input, &directory);
        if prepared.is_err() {
            let _ = FileManager.remove_item(&directory);
        }
        prepared
    }

    fn copy_draft_files(
        input: &MusicLocalTrackDraftInput,
        directory: &Path,
    ) -> anyhow::Result<MusicPreparedLocalTrackDraft> {
        let audio_url = directory.join(Self::AUDIO_FILE_NAME);
        let audio_source = input
            .audio_file
            .url
            .to_file_path()
            .ok_or_else(|| CocoaError::new(CocoaErrorCode::FileNoSuchFile))?;
        Self::replace_copy(&audio_source, &audio_url)?;
        let audio_bytes = Self::file_size(&audio_url);
        if audio_bytes <= 0 {
            return Err(CocoaError::new(CocoaErrorCode::FileReadCorruptFile).into());
        }

        let mut cover_url: Option<PathBuf> = None;
        let mut cover_filename: Option<String> = None;
        let mut cover_content_type: Option<String> = None;
        let mut cover_bytes: i64 = 0;
        if let Some(cover_file) = input.cover_file.as_ref() {
            let extension = Self::safe_extension(&cover_file.url.path_extension(), "jpg");
            let target = directory.join(format!("{}.{extension}", Self::COVER_STEM));
            let cover_source = cover_file
                .url
                .to_file_path()
                .ok_or_else(|| CocoaError::new(CocoaErrorCode::FileNoSuchFile))?;
            Self::replace_copy(&cover_source, &target)?;
            cover_bytes = Self::file_size(&target);
            cover_url = Some(target);
            cover_filename = Some(cover_file.filename.clone());
            cover_content_type = Some(cover_file.content_type.clone());
        }

        let record = MusicLocalTrackDraftRecord {
            version: MusicLocalTrackDraftRecord::CURRENT_VERSION,
            account_user_id: input.account_user_id.clone(),
            request_id: input.request_id.clone(),
            temp_id: input.temp_id.clone(),
            playlist_id: input.playlist_id.clone(),
            title: input.title.clone(),
            description: input.description.clone(),
            artist_name: input.artist_name.clone(),
            duration_ms: input.duration_ms,
            audio_filename: input.audio_file.filename.clone(),
            audio_content_type: input.audio_file.content_type.clone(),
            audio_byte_count: audio_bytes,
            cover_filename,
            cover_content_type,
            cover_byte_count: cover_bytes,
            created_at_ms: Self::now_ms(),
            phase: MusicLocalTrackTransferPhase::WaitingForConnection
                .raw_value()
                .to_owned(),
            error: None,
        };
        Self::write_record(&record)?;
        Ok(MusicPreparedLocalTrackDraft {
            record,
            audio_url,
            cover_url,
        })
    }

    fn adopt_legacy_pending_draft(
        track: &RiseStoredMusicTrack,
        playlist_id: &str,
        account_user_id: &str,
        connected: bool,
        interrupted_error: &str,
    ) -> Option<MusicLocalTrackDraftRecord> {
        let temp_id = track.id.as_deref()?;
        let audio_path = Self::local_file_url(track.audio_url.as_deref())?;
        if !FileManager.file_exists(&audio_path) {
            return None;
        }
        let audio_url = Url::from_file_path(&audio_path)?;

        let cover_path = Self::local_file_url(track.cover_url.as_deref()).filter(|path| FileManager.file_exists(path));
        let audio_name = audio_url.last_path_component();
        let audio_file = SaiWsOptimisticLocalFile {
            file_context_override: Some("music_audio".to_owned()),
            byte_count: Some(Self::file_size(&audio_path)),
            ..SaiWsOptimisticLocalFile::new(
                audio_url,
                if audio_name.is_empty() {
                    "track.mp3".to_owned()
                } else {
                    audio_name
                },
                "audio/mpeg".to_owned(),
            )
        };
        let cover_file = cover_path.and_then(|path| {
            let url = Url::from_file_path(&path)?;
            let content_type = if url.path_extension().to_lowercase() == "png" {
                "image/png"
            } else {
                "image/jpeg"
            };
            Some(SaiWsOptimisticLocalFile {
                file_context_override: Some("music_cover".to_owned()),
                byte_count: Some(Self::file_size(&path)),
                ..SaiWsOptimisticLocalFile::new(url.clone(), url.last_path_component(), content_type.to_owned())
            })
        });

        let input = MusicLocalTrackDraftInput {
            account_user_id: account_user_id.to_owned(),
            request_id: Uuid::new_v4().uuid_string().to_lowercase(),
            temp_id: temp_id.to_owned(),
            playlist_id: playlist_id.to_owned(),
            title: track.title.clone().unwrap_or_else(|| "MP3".to_owned()),
            description: track.description.clone(),
            artist_name: track
                .artist
                .as_ref()
                .map(|artist| artist.stage_name.clone())
                .unwrap_or_else(|| "Unknown Artist".to_owned()),
            duration_ms: track.duration_ms.unwrap_or(500).max(500),
            audio_file,
            cover_file,
        };
        let prepared = Self::persist_draft(&input).ok()?;
        let mut record = prepared.record;
        record.phase = if connected {
            MusicLocalTrackTransferPhase::Failed.raw_value().to_owned()
        } else {
            MusicLocalTrackTransferPhase::WaitingForConnection
                .raw_value()
                .to_owned()
        };
        record.error = if connected {
            Some(
                track
                    .transfer_error
                    .clone()
                    .unwrap_or_else(|| interrupted_error.to_owned()),
            )
        } else {
            None
        };
        Self::write_record(&record).ok()?;
        Some(record)
    }

    fn prepared_draft(record: &MusicLocalTrackDraftRecord) -> Option<MusicPreparedLocalTrackDraft> {
        let directory = Self::draft_directory(&record.account_user_id, &record.request_id, false).ok()?;
        let audio_url = directory.join(Self::AUDIO_FILE_NAME);
        if !FileManager.file_exists(&audio_url) || Self::file_size(&audio_url) <= 0 {
            return None;
        }
        let cover_url = FileManager
            .contents_of_directory_at(&directory, DirectoryEnumerationOptions::SKIPS_HIDDEN_FILES)
            .ok()
            .and_then(|children| {
                children
                    .into_iter()
                    .find(|child| child.file_stem().is_some_and(|stem| stem == Self::COVER_STEM))
            });
        Some(MusicPreparedLocalTrackDraft {
            record: record.clone(),
            audio_url,
            cover_url,
        })
    }

    fn load_records(account_user_id: &str) -> Vec<MusicLocalTrackDraftRecord> {
        let Ok(directory) = Self::account_directory(account_user_id, false) else {
            return Vec::new();
        };
        let Ok(children) =
            FileManager.contents_of_directory_at(&directory, DirectoryEnumerationOptions::SKIPS_HIDDEN_FILES)
        else {
            return Vec::new();
        };

        let decoded: Vec<MusicLocalTrackDraftRecord> = children
            .iter()
            .filter_map(|child| {
                let data = std::fs::read(child.join(Self::RECORD_FILE_NAME)).ok()?;
                let record: MusicLocalTrackDraftRecord = serde_json::from_slice(&data).ok()?;
                (record.version == MusicLocalTrackDraftRecord::CURRENT_VERSION
                    && record.account_user_id == account_user_id)
                    .then_some(record)
            })
            .collect();
        Self::deduplicated_by_temp_id(decoded)
    }

    fn deduplicated_by_temp_id(records: Vec<MusicLocalTrackDraftRecord>) -> Vec<MusicLocalTrackDraftRecord> {
        let mut winner_by_temp_id: HashMap<String, MusicLocalTrackDraftRecord> = HashMap::new();
        let mut discarded: Vec<MusicLocalTrackDraftRecord> = Vec::new();
        for record in &records {
            let Some(current) = winner_by_temp_id.get(&record.temp_id) else {
                winner_by_temp_id.insert(record.temp_id.clone(), record.clone());
                continue;
            };
            if record.created_at_ms > current.created_at_ms {
                let replaced = current.clone();
                winner_by_temp_id.insert(record.temp_id.clone(), record.clone());
                discarded.push(replaced);
            } else {
                discarded.push(record.clone());
            }
        }
        if discarded.is_empty() {
            return records;
        }

        discarded.iter().for_each(Self::remove_draft_directory_now);
        let survivors: HashSet<String> = winner_by_temp_id
            .into_values()
            .map(|record| record.request_id)
            .collect();
        records
            .into_iter()
            .filter(|record| survivors.contains(&record.request_id))
            .collect()
    }

    fn write_record(record: &MusicLocalTrackDraftRecord) -> anyhow::Result<()> {
        let directory = Self::draft_directory(&record.account_user_id, &record.request_id, true)?;
        let data = serde_json::to_vec(record)?;
        Self::write_atomically(&directory.join(Self::RECORD_FILE_NAME), &data)?;
        Ok(())
    }

    fn write_atomically(url: &Path, data: &[u8]) -> std::io::Result<()> {
        let mut temporary = url.as_os_str().to_owned();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        std::fs::write(&temporary, data)?;
        std::fs::rename(&temporary, url).inspect_err(|_| {
            let _ = std::fs::remove_file(&temporary);
        })
    }

    fn remove_draft_directory(record: MusicLocalTrackDraftRecord) {
        drop(RiseAccountRuntime::current().spawn_blocking(move || Self::remove_draft_directory_now(&record)));
    }

    fn remove_draft_directory_now(record: &MusicLocalTrackDraftRecord) {
        let Ok(directory) = Self::draft_directory(&record.account_user_id, &record.request_id, false) else {
            return;
        };
        let _ = FileManager.remove_item(&directory);
    }

    fn replace_copy(source: &Path, target: &Path) -> Result<(), CocoaError> {
        if FileManager.file_exists(target) {
            FileManager.remove_item(target)?;
        }
        FileManager.copy_item(source, target)
    }

    fn file_size(url: &Path) -> i64 {
        FileManager
            .attributes_of_item(url)
            .map(|attributes| attributes.size)
            .unwrap_or(0)
    }

    fn local_file_url(raw_value: Option<&str>) -> Option<PathBuf> {
        let raw = trimming_whitespaces_and_newlines(raw_value.unwrap_or(""));
        if raw.is_empty() {
            return None;
        }
        if let Some(url) = Url::from_string(raw)
            && url.is_file_url()
        {
            return url.to_file_path();
        }
        if is_local_absolute_path(raw) {
            return Some(PathBuf::from(raw));
        }
        None
    }

    fn draft_directory(account_user_id: &str, request_id: &str, create: bool) -> Result<PathBuf, CocoaError> {
        let account = Self::account_directory(account_user_id, create)?;
        let directory = account.join(Self::safe_component(request_id));
        if create {
            FileManager.create_directory(&directory, true)?;
            let values = URLResourceValues {
                is_excluded_from_backup: Some(true),
            };
            let _ = set_resource_values(&directory, &values);
        }
        Ok(directory)
    }

    fn account_directory(account_user_id: &str, create: bool) -> Result<PathBuf, CocoaError> {
        let root = DATA_ROOT
            .as_ref()
            .ok_or_else(|| CocoaError::new(CocoaErrorCode::FileNoSuchFile))?;
        let directory = root
            .join("Riseonly")
            .join("MusicLocalTrackDrafts")
            .join(Self::safe_component(account_user_id));
        if create {
            FileManager.create_directory(&directory, true)?;
        }
        Ok(directory)
    }

    fn safe_component(value: &str) -> String {
        value
            .chars()
            .map(|character| {
                if character.is_alphanumeric() || character == '-' || character == '_' {
                    character
                } else {
                    '_'
                }
            })
            .take(128)
            .collect()
    }

    fn safe_extension(value: &str, fallback: &str) -> String {
        let filtered: String = value
            .to_lowercase()
            .chars()
            .filter(|character| character.is_alphanumeric())
            .collect();
        if filtered.is_empty() {
            fallback.to_owned()
        } else {
            filtered.chars().take(8).collect()
        }
    }

    fn now_ms() -> i64 {
        (RiseClock::now().time_interval_since_1970() * 1_000.0) as i64
    }

    fn is_connected(cx: &mut App) -> bool {
        SaiWs::manager(cx).read(cx).is_connected()
    }

    fn is_create_local_track_pending(cx: &mut App) -> bool {
        MusicActionsStore::slices(cx)
            .create_local_track_in_playlist_sai
            .is_pending()
    }

    fn report_error(on_error: Option<MusicLocalTrackDraftErrorHandler>, message: String, cx: &mut App) {
        let Some(on_error) = on_error else {
            return;
        };
        cx.defer(move |cx| on_error(message, cx));
    }

    /// blocking file work on the account runtime, none if a reset cancelled it
    async fn detached<R: Send + 'static>(work: impl FnOnce() -> R + Send + 'static) -> Option<R> {
        RiseAccountRuntime::current().spawn_blocking(work).await.ok()
    }

    /// reads wait for the record writes queued before them
    async fn after_persistence<R: Send + 'static>(
        pending: Option<MusicLocalTrackDraftPersistence>,
        work: impl FnOnce() -> R + Send + 'static,
    ) -> Option<R> {
        RiseAccountRuntime::current()
            .spawn(async move {
                if let Some(pending) = pending {
                    pending.await;
                }
                RiseAccountRuntime::current().spawn_blocking(work).await.ok()
            })
            .await
            .ok()
            .flatten()
    }
}
