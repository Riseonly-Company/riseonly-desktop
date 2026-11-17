use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::http_client::http::{HeaderName, HeaderValue, Method, Request, Uri};
use gpui::{App, AsyncApp, Context, WeakEntity};
use parking_lot::Mutex;
use rise_canon::{RiseEvent, RiseLimits};
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_core::foundation::foundation_file_url::is_local_absolute_path;
use rise_core::foundation::foundation_text::{trimming_characters, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::{Url, UrlComponents};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::CancellationError;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use crate::core::engine_bridge::network_transport_config::NetworkClientSlot;
use rise_engine::rise_upload::rise_url_session::{URLError, URLErrorCode, URLRequest};
use rise_events::name;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_platform::file_manager::FileManager;
use rise_platform::paths::temporary_directory;
use serde_json::Value;

use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::utilities::sai_payload_path::SaiJSONCodec;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::sai::core::websocket::sai_web_socket_configuration::SaiWebSocketConfiguration;
use crate::core::lib::sai::modules::sai_fetch::sai_fetch_http_configuration::{
    SaiFetchHttpConfiguration, SaiFetchRequestInterceptor,
};
use crate::core::lib::sai::modules::sai_ws::rise_event_payload::RiseEventPayload;
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticLocalFile;
use crate::core::media::remote_media_disk_cache::{
    RemoteMediaDiskCache, RemoteMediaDownloadRetryPolicy, RemoteMediaHttpResponse, RemoteMediaTransferError,
};
use crate::core::media::rise_media_transfer_hub::RiseMediaCacheIdentity;
use crate::core::ui::clever_image_disk_cache::CleverImageDiskCache;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierIcon, NotifierSendOptions, NotifierType};
use crate::modules::auth::services::auth_token_refresh::AuthTokenRefresh;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::music::shared::utils::music::music_models::MusicLocalTrackTransferPhase;
use crate::modules::music::shared::utils::music::music_playback_disk_cache::MusicPlaybackDiskCache;
use crate::modules::music::shared::utils::music::music_track_file_upload::MusicTrackFileUpload;
use crate::modules::music::shared::utils::music::music_track_upload_notifier::MusicTrackUploadNotifier;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_message_import_types::{
    MusicMessageMP3CreateOutcome, MusicMessageMP3ImportError, MusicMessageMP3ImportJob, MusicMessageMP3ImportRequest,
    MusicMessageMP3ImportSource, MusicMessageMP3PendingCompletion,
};
use crate::modules::music::stores::music_actions::music_wire_types::RiseStoredMusicTrack;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;

const MESSAGE_MP3_DEBUG_TOOLS: bool = cfg!(any(debug_assertions, feature = "sai_debug_tools"));

/// created track callback and success callback are separate closures
/// the id crosses here so the mp3 hits the cache before the temp file is swept
#[derive(Default)]
struct MusicMessageMP3CreatedTrackRef {
    track_id: Option<String>,
}

pub enum MusicMessageMP3AuthPolicy {}

impl MusicMessageMP3AuthPolicy {
    const SENSITIVE_HEADERS: [&'static str; 4] = ["Authorization", "Cookie", "Proxy-Authorization", "X-API-Key"];

    pub fn is_trusted_origin(candidate: Option<&Url>, base_url: Option<&Url>) -> bool {
        let (Some(candidate), Some(base_url)) = (candidate, base_url) else {
            return false;
        };
        let (Some(candidate_scheme), Some(base_scheme)) = (
            candidate.scheme().map(str::to_lowercase),
            base_url.scheme().map(str::to_lowercase),
        ) else {
            return false;
        };
        let (Some(candidate_host), Some(base_host)) = (
            candidate.host().map(|host| host.to_lowercase()),
            base_url.host().map(|host| host.to_lowercase()),
        ) else {
            return false;
        };
        if candidate_scheme != base_scheme
            || candidate_host != base_host
            || Self::effective_port(candidate) != Self::effective_port(base_url)
        {
            return false;
        }

        if candidate_scheme == "https" {
            return true;
        }
        MESSAGE_MP3_DEBUG_TOOLS && candidate_scheme == "http"
    }

    pub fn is_allowed_remote_url(candidate: Option<&Url>, base_url: Option<&Url>) -> bool {
        let Some(scheme) = candidate.and_then(|candidate| candidate.scheme()).map(str::to_lowercase) else {
            return false;
        };
        if scheme == "https" {
            return true;
        }
        MESSAGE_MP3_DEBUG_TOOLS && scheme == "http" && Self::is_trusted_origin(candidate, base_url)
    }

    pub fn sensitive_headers(from_request: &URLRequest) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        for name in Self::SENSITIVE_HEADERS {
            if let Some(value) = from_request.value(name).filter(|value| !value.is_empty()) {
                headers.insert(name.to_owned(), value.to_owned());
            }
        }
        headers
    }

    pub fn strip_sensitive_headers(from_request: &mut URLRequest) {
        let mut stripped = URLRequest::new(from_request.url.clone());
        stripped.http_method = from_request.http_method.clone();
        stripped.http_body = from_request.http_body.clone();
        stripped.timeout_interval = from_request.timeout_interval;
        for (name, value) in from_request.all_http_header_fields() {
            let is_sensitive = Self::SENSITIVE_HEADERS
                .iter()
                .any(|sensitive| sensitive.eq_ignore_ascii_case(name));
            if !is_sensitive {
                stripped.set_value(value, name);
            }
        }
        *from_request = stripped;
    }

    fn effective_port(url: &Url) -> Option<i64> {
        if let Some(port) = url.port() {
            return Some(port);
        }
        match url.scheme().map(str::to_lowercase).as_deref() {
            Some("https") => Some(443),
            Some("http") => Some(80),
            _ => None,
        }
    }
}

pub enum MusicMessageMP3ProgressPolicy {}

impl MusicMessageMP3ProgressPolicy {
    pub const MINIMUM_BYTE_DELTA: i64 = 32 * 1_024;
    pub const MAXIMUM_SILENCE: TimeInterval = 0.1;

    pub fn should_publish(
        previous_bytes: i64,
        current_bytes: i64,
        total_bytes: i64,
        previous_time: TimeInterval,
        current_time: TimeInterval,
    ) -> bool {
        (current_bytes >= total_bytes && total_bytes > 0)
            || current_bytes - previous_bytes >= Self::MINIMUM_BYTE_DELTA
            || current_time - previous_time >= Self::MAXIMUM_SILENCE
    }
}

static MESSAGE_MP3_DOWNLOAD_SESSION: NetworkClientSlot = NetworkClientSlot::without_redirects();

struct MusicMessageMP3DownloadFile {
    path: PathBuf,
    is_owned: bool,
}

impl MusicMessageMP3DownloadFile {
    fn hand_over(mut self) -> PathBuf {
        self.is_owned = false;
        self.path.clone()
    }
}

impl Drop for MusicMessageMP3DownloadFile {
    fn drop(&mut self) {
        if self.is_owned {
            FileManager.remove_item(&self.path).ok();
        }
    }
}

struct MusicMessageMP3DownloadCancel(tokio::task::AbortHandle);

impl Drop for MusicMessageMP3DownloadCancel {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub struct MusicMessageMP3LimitedDownloader {
    max_bytes: i64,
    destination: PathBuf,
    trusted_base_url: Option<Url>,
    sensitive_headers: HashMap<String, String>,
    initially_trusted: bool,
    on_progress: Option<Arc<dyn Fn(i64, i64) + Send + Sync>>,
    lock: Mutex<(i64, TimeInterval)>,
}

impl MusicMessageMP3LimitedDownloader {
    const MAXIMUM_REDIRECTS: usize = 10;

    pub fn new(
        request: &URLRequest,
        max_bytes: i64,
        file_extension: &str,
        trusted_base_url: Option<Url>,
        on_progress: Option<Arc<dyn Fn(i64, i64) + Send + Sync>>,
    ) -> Self {
        let initially_trusted =
            MusicMessageMP3AuthPolicy::is_trusted_origin(Some(&request.url), trusted_base_url.as_ref());
        let ext = trimming_characters(file_extension, |character| !character.is_alphanumeric());
        let destination = temporary_directory().join(format!(
            "message-import-{}.{}",
            Uuid::new_v4().uuid_string(),
            if ext.is_empty() { "bin" } else { ext }
        ));
        Self {
            max_bytes,
            destination,
            trusted_base_url,
            sensitive_headers: MusicMessageMP3AuthPolicy::sensitive_headers(request),
            initially_trusted,
            on_progress,
            lock: Mutex::new((0, RiseClock::now().time_interval_since_reference_date())),
        }
    }

    pub async fn download(self: Arc<Self>, request: URLRequest) -> anyhow::Result<(PathBuf, RemoteMediaHttpResponse)> {
        let downloader = self.clone();
        let handle = RiseAccountRuntime::current().spawn(async move { downloader.transfer(request).await });
        let _cancel = MusicMessageMP3DownloadCancel(handle.abort_handle());
        match handle.await {
            Ok(result) => result,
            Err(_) => Err(CancellationError.into()),
        }
    }

    async fn transfer(&self, request: URLRequest) -> anyhow::Result<(PathBuf, RemoteMediaHttpResponse)> {
        let Some(session) = MESSAGE_MP3_DOWNLOAD_SESSION.current() else {
            return Err(URLError::new(URLErrorCode::SecureConnectionFailed).into());
        };
        let session = &session;
        let timeout_interval = request.timeout_interval;
        let mut current = request;
        let mut redirect_count = 0usize;
        let (mut response, url) = loop {
            let method = reqwest::Method::from_bytes(current.http_method.as_bytes())
                .map_err(|_| URLError::new(URLErrorCode::BadURL))?;
            let mut builder = session.request(method, current.url.absolute_string());
            for (name, value) in current.all_http_header_fields() {
                builder = builder.header(name.as_str(), value.as_str());
            }
            if let Some(body) = &current.http_body {
                builder = builder.body(body.clone());
            }
            let response = tokio::time::timeout(timeout_interval, builder.send())
                .await
                .map_err(|_| URLError::new(URLErrorCode::TimedOut))?
                .map_err(|error| RemoteMediaTransferError::url_error(&error))?;
            if !response.status().is_redirection() {
                break (response, current.url);
            }
            if redirect_count >= Self::MAXIMUM_REDIRECTS {
                return Err(URLError::new(URLErrorCode::Unknown).into());
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let Some(destination) = location
                .and_then(|location| Url::from_string_relative_to(&location, Some(&current.url)))
                .and_then(|resolved| Url::from_string(resolved.absolute_string()))
            else {
                break (response, current.url);
            };
            let mut proposed = current.clone();
            proposed.url = destination;
            if response.status().as_u16() == 303 {
                proposed.http_method = "GET".to_owned();
                proposed.http_body = None;
            }
            match self.url_session_will_perform_http_redirection(proposed) {
                Some(next) => {
                    current = next;
                    redirect_count += 1;
                }
                None => break (response, current.url),
            }
        };

        let head = RemoteMediaHttpResponse::from_reqwest(&response, url);
        let expected = head.expected_content_length();
        let total_bytes = if expected > 0 { expected } else { 0 };
        let owned = MusicMessageMP3DownloadFile {
            path: self.destination.clone(),
            is_owned: true,
        };
        let mut file = std::fs::File::create(&owned.path)?;
        let mut total_bytes_written: i64 = 0;
        loop {
            match tokio::time::timeout(timeout_interval, response.chunk()).await {
                Ok(Ok(Some(bytes))) => {
                    file.write_all(&bytes)?;
                    total_bytes_written =
                        total_bytes_written.saturating_add(i64::try_from(bytes.len()).unwrap_or(i64::MAX));
                    self.url_session_did_write_data(total_bytes_written, total_bytes)?;
                }
                Ok(Ok(None)) => break,
                Ok(Err(error)) => return Err(RemoteMediaTransferError::url_error(&error).into()),
                Err(_) => return Err(URLError::new(URLErrorCode::TimedOut).into()),
            }
        }
        file.flush()?;
        drop(file);
        self.url_session_did_finish_downloading(&owned.path)?;
        Ok((owned.hand_over(), head))
    }

    fn url_session_did_write_data(&self, total_bytes_written: i64, total_bytes_expected_to_write: i64) -> anyhow::Result<()> {
        let total_bytes = total_bytes_expected_to_write.max(0);
        let now = RiseClock::now().time_interval_since_reference_date();
        let should_publish = {
            let mut lock = self.lock.lock();
            let should_publish =
                MusicMessageMP3ProgressPolicy::should_publish(lock.0, total_bytes_written, total_bytes, lock.1, now);
            if should_publish {
                *lock = (total_bytes_written, now);
            }
            should_publish
        };
        if should_publish && let Some(on_progress) = &self.on_progress {
            on_progress(total_bytes_written, total_bytes);
        }

        if total_bytes_written > self.max_bytes
            || (total_bytes_expected_to_write > 0 && total_bytes_expected_to_write > self.max_bytes)
        {
            return Err(MusicMessageMP3ImportError::FileTooLarge.into());
        }
        Ok(())
    }

    fn url_session_did_finish_downloading(&self, location: &Path) -> anyhow::Result<()> {
        let size = FileManager.attributes_of_item(location).map(|attributes| attributes.size).unwrap_or(0);
        if size <= 0 {
            return Err(MusicMessageMP3ImportError::InvalidSource.into());
        }
        if size > self.max_bytes {
            return Err(MusicMessageMP3ImportError::FileTooLarge.into());
        }
        Ok(())
    }

    fn url_session_will_perform_http_redirection(&self, new_request: URLRequest) -> Option<URLRequest> {
        let mut redirected = new_request;
        if !MusicMessageMP3AuthPolicy::is_allowed_remote_url(Some(&redirected.url), self.trusted_base_url.as_ref()) {
            return None;
        }
        let remains_trusted = self.initially_trusted
            && MusicMessageMP3AuthPolicy::is_trusted_origin(Some(&redirected.url), self.trusted_base_url.as_ref());
        if remains_trusted {
            for (name, value) in &self.sensitive_headers {
                if redirected.value(name).is_none() {
                    redirected.set_value(value, name);
                }
            }
        } else {
            MusicMessageMP3AuthPolicy::strip_sensitive_headers(&mut redirected);
        }
        Some(redirected)
    }
}

enum MusicMessageMP3ImportFileIO {}

impl MusicMessageMP3ImportFileIO {
    fn file_exists(url: &Path) -> bool {
        FileManager.file_exists(url)
    }

    fn copy_and_validate_mp3(from_source: &Path, max_bytes: i64) -> anyhow::Result<PathBuf> {
        let destination = temporary_directory().join(format!("message-track-{}.mp3", Uuid::new_v4().uuid_string()));
        let copied = Self::validate_mp3(from_source, max_bytes)
            .and_then(|()| FileManager.copy_item(from_source, &destination).map_err(anyhow::Error::from))
            .and_then(|()| Self::validate_mp3(&destination, max_bytes));
        match copied {
            Ok(()) => Ok(destination),
            Err(error) => {
                FileManager.remove_item(&destination).ok();
                Err(error)
            }
        }
    }

    fn validate_mp3(url: &Path, max_bytes: i64) -> anyhow::Result<()> {
        let size = Self::file_size(url);
        if size <= 0 {
            return Err(MusicMessageMP3ImportError::InvalidSource.into());
        }
        if size > max_bytes {
            return Err(MusicMessageMP3ImportError::FileTooLarge.into());
        }
        let Ok(handle) = std::fs::File::open(url) else {
            return Err(MusicMessageMP3ImportError::InvalidSource.into());
        };
        let mut head = Vec::with_capacity(8_192);
        handle.take(8_192).read_to_end(&mut head).ok();
        if !Self::is_likely_mp3(&head) {
            return Err(MusicMessageMP3ImportError::Mp3Required.into());
        }
        Ok(())
    }

    fn data(from_url: &Path, max_bytes: i64) -> Option<Vec<u8>> {
        let size = Self::file_size(from_url);
        if size <= 0 || size > max_bytes {
            return None;
        }
        std::fs::read(from_url).ok()
    }

    fn remove_files(urls: &HashSet<PathBuf>) {
        for url in urls {
            FileManager.remove_item(url).ok();
        }
    }

    fn file_size(url: &Path) -> i64 {
        FileManager.attributes_of_item(url).map(|attributes| attributes.size).unwrap_or(0)
    }

    fn is_likely_mp3(data: &[u8]) -> bool {
        if data.len() < 2 {
            return false;
        }
        if data.len() >= 3 && data[0] == 0x49 && data[1] == 0x44 && data[2] == 0x33 {
            return true;
        }
        data.windows(2).any(|pair| pair[0] == 0xFF && pair[1] & 0xE0 == 0xE0)
    }
}

struct MusicMessageMP3CleanupURLs(HashSet<PathBuf>);

impl Drop for MusicMessageMP3CleanupURLs {
    fn drop(&mut self) {
        MusicServicesStore::schedule_file_cleanup(std::mem::take(&mut self.0));
    }
}

impl MusicServicesStore {
    fn message_mp3_max_bytes() -> i64 {
        100 * 1024 * 1024
    }

    fn message_cover_max_bytes() -> i64 {
        RiseLimits::value("music_cover")
    }

    fn notify_message_mp3_import_failed(content: String, cx: &mut App) {
        Notifier::send(
            tr("music_message_import_failed"),
            NotifierSendOptions {
                content: Some(content),
                r#type: NotifierType::Error,
                icon: NotifierIcon::System("exclamationmark.triangle.fill".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn hinted_duration_ms(source: &MusicMessageMP3ImportSource) -> i64 {
        (source.duration_seconds.unwrap_or(0.0).max(0.0) * 1_000.0) as i64
    }

    pub fn enqueue_message_mp3_import(&mut self, request: MusicMessageMP3ImportRequest, cx: &mut Context<Self>) {
        let title = trimming_whitespaces_and_newlines(&request.title).to_owned();
        let account = SaiAccountCacheScope::active_user_id().zip(SaiAccountCacheScope::active_epoch());
        let is_valid = request.destination.can_add_tracks
            && !request.request_id.is_empty()
            && !request.temp_id.is_empty()
            && !request.destination.id.is_empty()
            && !title.is_empty();
        let Some((user_id, epoch)) = account.filter(|_| is_valid) else {
            Self::schedule_managed_replacement_cover_cleanup(request.replacement_cover_file.as_ref());
            Self::notify_message_mp3_import_failed(tr("music_message_import_invalid_source"), cx);
            return;
        };

        self.prune_message_mp3_completion_state();
        if self.message_mp3_pending_completions.contains_key(&request.request_id)
            || self
                .message_mp3_import_queue
                .iter()
                .any(|job| job.request.request_id == request.request_id)
        {
            return;
        }

        let optimistic_audio_url = Self::local_file_url(request.source.local_preview_url.as_deref().unwrap_or(""))
            .or_else(|| Url::from_string(&request.source.remote_url));
        let Some(optimistic_audio_url) = optimistic_audio_url else {
            Self::schedule_managed_replacement_cover_cleanup(request.replacement_cover_file.as_ref());
            return;
        };
        let initial_cover_url = request
            .replacement_cover_file
            .as_ref()
            .map(|file| file.url.absolute_string().to_owned())
            .or_else(|| request.source.thumbnail_url.clone())
            .unwrap_or_default();
        let initial_artist = trimming_whitespaces_and_newlines(&request.artist_name);
        let artist_name = if initial_artist.is_empty() {
            tr("music_unknown_artist")
        } else {
            initial_artist.to_owned()
        };
        let draft = self.make_local_track_draft(
            &request.temp_id,
            &title,
            request.description.as_deref(),
            &artist_name,
            Self::hinted_duration_ms(&request.source),
            &user_id,
            &optimistic_audio_url,
            &initial_cover_url,
            Some(request.source.chat_id.as_str()),
            Some(request.source.message_id.as_str()),
            Some(request.source.remote_url.as_str()),
            Some(MusicLocalTrackTransferPhase::Queued),
            0,
            request.source.file_size.unwrap_or(0).max(0),
            None,
        );
        self.insert_local_track_optimistic(&request.destination.id, draft, Some(&request.destination), cx);

        self.message_mp3_import_queue.push(MusicMessageMP3ImportJob {
            request,
            account_user_id: user_id,
            account_epoch: epoch,
        });
        self.start_message_mp3_import_queue_if_needed(cx);
    }

    pub fn cancel_pending_local_track(&mut self, playlist_id: &str, temp_id: &str, cx: &mut Context<Self>) {
        let pid = trimming_whitespaces_and_newlines(playlist_id);
        let tid = trimming_whitespaces_and_newlines(temp_id);
        if pid.is_empty() || tid.is_empty() {
            return;
        }

        MusicTrackUploadNotifier::finish(tid, cx);
        if self.message_mp3_active_import_temp_ids.contains(tid) {
            self.message_mp3_cancelled_temp_ids.insert(tid.to_owned());
        }
        let cancelled_covers: HashSet<PathBuf> = self
            .message_mp3_import_queue
            .iter()
            .filter(|job| job.request.temp_id == tid)
            .filter_map(|job| job.request.replacement_cover_file.as_ref())
            .filter(|file| Self::is_managed_temporary_file(&file.url))
            .filter_map(|file| file.url.to_file_path())
            .collect();
        self.message_mp3_import_queue.retain(|job| job.request.temp_id != tid);
        Self::schedule_file_cleanup(cancelled_covers);
        self.message_mp3_pending_completions
            .retain(|_, pending| pending.temp_id != tid);

        self.message_mp3_failed_jobs.remove(tid);
        self.remove_local_track_optimistic(pid, tid, cx);
    }

    pub fn can_retry_message_mp3_import(&self, temp_id: &str) -> bool {
        self.message_mp3_failed_jobs
            .get(temp_id)
            .is_some_and(Self::is_current_account)
    }

    pub fn retry_message_mp3_import(&mut self, temp_id: &str, cx: &mut Context<Self>) {
        let Some(job) = self.message_mp3_failed_jobs.remove(temp_id) else {
            return;
        };
        if !Self::is_current_account(&job) {
            return;
        }
        self.message_mp3_pending_completions.remove(&job.request.request_id);
        let cover = job.request.replacement_cover_file.clone().filter(|file| {
            file.url
                .to_file_path()
                .is_some_and(|path| MusicMessageMP3ImportFileIO::file_exists(&path))
        });
        self.enqueue_message_mp3_import(
            MusicMessageMP3ImportRequest::new_with_ids(
                job.request.request_id.clone(),
                job.request.temp_id.clone(),
                job.request.source.clone(),
                job.request.destination.clone(),
                &job.request.title,
                job.request.description.as_deref(),
                &job.request.artist_name,
                cover,
            ),
            cx,
        );
    }

    pub fn is_pending_local_track(&self, playlist_id: &str, track_id: &str, cx: &mut App) -> bool {
        self.message_mp3_pending_playlist(playlist_id, cx)
            .and_then(|playlist| playlist.tracks)
            .is_some_and(|tracks| {
                tracks
                    .iter()
                    .any(|track| track.id.as_deref() == Some(track_id) && track.transfer_phase.is_some())
            })
    }

    pub fn reset_message_mp3_import_state(&mut self, cx: &mut Context<Self>) {
        MusicTrackUploadNotifier::finish_all(cx);
        self.message_mp3_import_generation = self.message_mp3_import_generation.wrapping_add(1);
        self.message_mp3_import_task = None;
        let queued_managed_covers: HashSet<PathBuf> = self
            .message_mp3_import_queue
            .iter()
            .filter_map(|job| job.request.replacement_cover_file.as_ref())
            .filter(|file| Self::is_managed_temporary_file(&file.url))
            .filter_map(|file| file.url.to_file_path())
            .collect();
        Self::schedule_file_cleanup(queued_managed_covers);
        self.message_mp3_import_queue.clear();
        self.message_mp3_pending_completions.clear();
        self.message_mp3_completed_request_ids.clear();
        self.message_mp3_active_import_temp_ids.clear();
        self.message_mp3_cancelled_temp_ids.clear();
        self.message_mp3_uploaded_temp_ids.clear();
        self.message_mp3_progress_persistence.clear();
        self.message_mp3_failed_jobs.clear();
        MusicActionsStore::slices(cx)
            .message_mp3_create_local_track_sai
            .reset_to_idle();
    }

    pub fn local_track_create_completed_event_handler(&mut self, payload: Option<&Value>, cx: &mut Context<Self>) {
        let Some(RiseEvent::LocalTrackCreateCompleted(event)) =
            RiseEvent::decode_payload(name::LOCAL_TRACK_CREATE_COMPLETED, payload)
        else {
            return;
        };
        if event.request_id.is_empty() || event.playlist_id.is_empty() {
            return;
        }
        let Some(track) = SaiJSONCodec::decode::<RiseStoredMusicTrack>(event.track.as_ref()) else {
            return;
        };
        let request_id = event.request_id;
        let playlist_id = event.playlist_id;
        if self.message_mp3_completed_request_ids.contains(&request_id) {
            return;
        }
        let Some(pending) = self
            .message_mp3_pending_completions
            .get(&request_id)
            .filter(|pending| pending.playlist_id == playlist_id)
            .cloned()
        else {
            return;
        };

        self.replace_local_track_optimistic(&playlist_id, &pending.temp_id, track, cx);
        self.mark_message_mp3_import_succeeded(&request_id, Some(pending.playlist_title.as_str()), cx);
    }

    fn start_message_mp3_import_queue_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.message_mp3_import_task.is_some() || self.message_mp3_import_queue.is_empty() {
            return;
        }
        let generation = self.message_mp3_import_generation;
        self.message_mp3_import_task = Some(cx.spawn(async move |this, cx| {
            Self::drain_message_mp3_import_queue(this, generation, cx).await;
        }));
    }

    async fn drain_message_mp3_import_queue(this: WeakEntity<Self>, generation: u64, cx: &mut AsyncApp) {
        loop {
            let next = this.update(cx, |service, _| {
                (generation == service.message_mp3_import_generation && !service.message_mp3_import_queue.is_empty())
                    .then(|| service.message_mp3_import_queue.remove(0))
            });
            let Ok(Some(job)) = next else {
                break;
            };
            Self::process_message_mp3_import(this.clone(), job, generation, cx).await;
        }

        this.update(cx, |service, cx| {
            if generation != service.message_mp3_import_generation {
                return;
            }
            if let Some(task) = service.message_mp3_import_task.take() {
                task.detach();
            }
            if !service.message_mp3_import_queue.is_empty() {
                service.start_message_mp3_import_queue_if_needed(cx);
            }
        })
        .ok();
    }

    async fn process_message_mp3_import(
        this: WeakEntity<Self>,
        job: MusicMessageMP3ImportJob,
        generation: u64,
        cx: &mut AsyncApp,
    ) {
        if !Self::is_current_account(&job) {
            return;
        }

        let started = this.update(cx, |service, cx| {
            service.message_mp3_active_import_temp_ids.insert(job.request.temp_id.clone());
            MusicTrackUploadNotifier::begin(&job.request.temp_id, cx);
            service.patch_message_mp3_pending_track(
                &job.request.destination.id,
                &job.request.temp_id,
                MusicLocalTrackTransferPhase::Downloading,
                0,
                job.request.source.file_size.unwrap_or(0).max(0),
                None,
                cx,
            );
        });
        if started.is_err() {
            return;
        }

        let mut cleanup_urls = MusicMessageMP3CleanupURLs(HashSet::new());
        if let Some(replacement) = job.request.replacement_cover_file.as_ref()
            && Self::is_managed_temporary_file(&replacement.url)
            && let Some(path) = replacement.url.to_file_path()
        {
            cleanup_urls.0.insert(path);
        }

        let processed = Self::process_message_mp3_import_steps(&this, &job, generation, &mut cleanup_urls.0, cx).await;
        if let Err(error) = processed {
            let is_cancelled = this
                .read_with(&*cx, |service, _| service.is_import_cancelled(&job))
                .unwrap_or(true);
            if Self::is_current_account(&job) && !is_cancelled {
                let message = error.to_string();
                this.update(cx, |service, cx| {
                    service.mark_message_mp3_import_failed(
                        &job,
                        &message,
                        job.request.source.file_size.unwrap_or(0).max(0),
                        cx,
                    );
                })
                .ok();
                cx.update(|cx| Self::notify_message_mp3_import_failed(message, cx));
            }
        }

        this.update(cx, |service, cx| {
            service.message_mp3_active_import_temp_ids.remove(&job.request.temp_id);
            service.message_mp3_cancelled_temp_ids.remove(&job.request.temp_id);
            MusicTrackUploadNotifier::finish(&job.request.temp_id, cx);
        })
        .ok();
        drop(cleanup_urls);
    }

    async fn process_message_mp3_import_steps(
        this: &WeakEntity<Self>,
        job: &MusicMessageMP3ImportJob,
        generation: u64,
        cleanup_urls: &mut HashSet<PathBuf>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<()> {
        let audio_url = Self::materialize_message_mp3(this, job, generation, cx).await?;
        cleanup_urls.insert(audio_url.clone());
        if !Self::is_current_account(job) {
            return Ok(());
        }

        let audio_byte_count = Self::file_size_off_main(&audio_url).await;
        this.update(cx, |service, cx| {
            service.patch_message_mp3_pending_track(
                &job.request.destination.id,
                &job.request.temp_id,
                MusicLocalTrackTransferPhase::Preparing,
                audio_byte_count,
                audio_byte_count,
                None,
                cx,
            );
        })?;

        let metadata = MusicTrackFileUpload::imported_metadata(audio_url.clone(), job.request.source.file_name.clone()).await;
        if !Self::is_current_account(job) {
            return Ok(());
        }

        let hinted_duration = Self::hinted_duration_ms(&job.request.source);
        let duration_ms = if metadata.duration_ms >= 500 {
            metadata.duration_ms
        } else {
            hinted_duration
        };
        if duration_ms < 500 {
            return Err(MusicMessageMP3ImportError::InvalidDuration.into());
        }

        let artist_name = Self::resolved_artist_name(&job.request.artist_name, metadata.artist.as_deref());
        let description = job
            .request
            .description
            .as_deref()
            .map(|description| trimming_whitespaces_and_newlines(description).to_owned())
            .filter(|description| !description.is_empty());
        let mut embedded_cover_file = None;
        if job.request.replacement_cover_file.is_none()
            && let Some(artwork) = metadata.artwork_data
        {
            embedded_cover_file = Self::make_artwork_file_off_main(artwork).await;
        }

        let cover_file = if let Some(replacement) = job.request.replacement_cover_file.clone() {
            Some(replacement)
        } else if let Some(embedded) = embedded_cover_file {
            if let Some(path) = embedded.url.to_file_path() {
                cleanup_urls.insert(path);
            }
            Some(embedded)
        } else {
            let cover_file = Self::message_cover_file(this, &job.request.source, job, cleanup_urls, cx)
                .await
                .ok()
                .flatten();
            if let Some(path) = cover_file.as_ref().and_then(|file| file.url.to_file_path()) {
                cleanup_urls.insert(path);
            }
            cover_file
        };

        let still_current = this.read_with(&*cx, |service, _| {
            generation == service.message_mp3_import_generation && !service.is_import_cancelled(job)
        })?;
        if !Self::is_current_account(job) || !still_current {
            return Ok(());
        }

        let upload_total_bytes = audio_byte_count + cover_file.as_ref().and_then(|file| file.byte_count).unwrap_or(0).max(0);
        let Some(audio_file_url) = Url::from_file_path(&audio_url) else {
            return Err(MusicMessageMP3ImportError::InvalidSource.into());
        };
        let title = trimming_whitespaces_and_newlines(&job.request.title).to_owned();
        let cover_url = cover_file
            .as_ref()
            .map(|file| file.url.absolute_string().to_owned())
            .or_else(|| job.request.source.thumbnail_url.clone())
            .unwrap_or_default();
        let upload_draft = this.update(cx, |service, cx| {
            let upload_draft = service.make_local_track_draft(
                &job.request.temp_id,
                &title,
                description.as_deref(),
                &artist_name,
                duration_ms,
                &job.account_user_id,
                &audio_file_url,
                &cover_url,
                Some(job.request.source.chat_id.as_str()),
                Some(job.request.source.message_id.as_str()),
                Some(job.request.source.remote_url.as_str()),
                Some(MusicLocalTrackTransferPhase::Uploading),
                0,
                upload_total_bytes,
                None,
            );
            service.insert_local_track_optimistic(
                &job.request.destination.id,
                upload_draft.clone(),
                Some(&job.request.destination),
                cx,
            );
            service.patch_message_mp3_pending_track(
                &job.request.destination.id,
                &job.request.temp_id,
                MusicLocalTrackTransferPhase::Uploading,
                0,
                upload_total_bytes,
                None,
                cx,
            );
            service.message_mp3_pending_completions.insert(
                job.request.request_id.clone(),
                MusicMessageMP3PendingCompletion {
                    playlist_id: job.request.destination.id.clone(),
                    playlist_title: job.request.destination.title.clone(),
                    temp_id: job.request.temp_id.clone(),
                    created_at: RiseClock::now(),
                },
            );
            upload_draft
        })?;

        let created_track = Rc::new(RefCell::new(MusicMessageMP3CreatedTrackRef::default()));
        let outcome = Self::create_message_mp3_track(
            this,
            job,
            duration_ms,
            &artist_name,
            description.as_deref(),
            &audio_file_url,
            audio_byte_count,
            cover_file,
            upload_draft,
            created_track.clone(),
            cx,
        )
        .await;
        if !Self::is_current_account(job) {
            return Ok(());
        }

        match outcome {
            MusicMessageMP3CreateOutcome::Success => {
                let server_track_id = created_track
                    .borrow()
                    .track_id
                    .as_deref()
                    .map(|track_id| trimming_whitespaces_and_newlines(track_id).to_owned())
                    .filter(|track_id| !track_id.is_empty());
                if let Some(server_track_id) = server_track_id {
                    let account_user_id = job.account_user_id.clone();
                    let source_url = audio_url.clone();
                    RiseAccountRuntime::current()
                        .spawn_blocking(move || {
                            MusicPlaybackDiskCache::adopt_external_file(
                                &server_track_id,
                                &source_url,
                                Some("mp3"),
                                Some(account_user_id.as_str()),
                            );
                        })
                        .await
                        .ok();
                }
                this.update(cx, |service, cx| {
                    service.mark_message_mp3_import_succeeded(
                        &job.request.request_id,
                        Some(job.request.destination.title.as_str()),
                        cx,
                    );
                })?;
            }
            MusicMessageMP3CreateOutcome::Failure(message) => {
                let should_report = this.update(cx, |service, cx| {
                    let should_report = !service.message_mp3_completed_request_ids.contains(&job.request.request_id)
                        && !service.is_import_cancelled(job);
                    if should_report {
                        service.mark_message_mp3_import_failed(job, &message, upload_total_bytes, cx);
                    }
                    should_report
                })?;
                if should_report {
                    cx.update(|cx| Self::notify_message_mp3_import_failed(message, cx));
                }
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn create_message_mp3_track(
        this: &WeakEntity<Self>,
        job: &MusicMessageMP3ImportJob,
        duration_ms: i64,
        artist_name: &str,
        description: Option<&str>,
        audio_url: &Url,
        audio_byte_count: i64,
        cover_file: Option<SaiWsOptimisticLocalFile>,
        optimistic_track: RiseStoredMusicTrack,
        created_track: Rc<RefCell<MusicMessageMP3CreatedTrackRef>>,
        cx: &mut AsyncApp,
    ) -> MusicMessageMP3CreateOutcome {
        let (continuation, mut stream) = mpsc::unbounded::<MusicMessageMP3CreateOutcome>();
        let local_audio_file = SaiWsOptimisticLocalFile {
            file_context_override: Some("music_audio".to_owned()),
            byte_count: Some(audio_byte_count),
            ..SaiWsOptimisticLocalFile::new(
                audio_url.clone(),
                job.request
                    .source
                    .file_name
                    .clone()
                    .unwrap_or_else(|| audio_url.last_path_component()),
                "audio/mpeg".to_owned(),
            )
        };
        let title = trimming_whitespaces_and_newlines(&job.request.title).to_owned();
        let success_continuation = continuation.clone();
        let on_success: Box<dyn FnOnce(&mut App)> = Box::new(move |_cx: &mut App| {
            success_continuation
                .unbounded_send(MusicMessageMP3CreateOutcome::Success)
                .ok();
        });
        let on_created: Box<dyn FnOnce(RiseStoredMusicTrack, &mut App)> =
            Box::new(move |track: RiseStoredMusicTrack, _cx: &mut App| {
                created_track.borrow_mut().track_id = track.id;
            });
        let error_continuation = continuation.clone();
        let on_error: Box<dyn FnOnce(String, &mut App)> = Box::new(move |message: String, _cx: &mut App| {
            error_continuation
                .unbounded_send(MusicMessageMP3CreateOutcome::Failure(message))
                .ok();
        });
        cx.update(|cx| {
            MusicActionsStore::shared(cx).update(cx, |actions, cx| {
                let request_slice = actions.message_mp3_create_local_track_sai.clone();
                actions.create_local_track_in_playlist_action(
                    &job.request.request_id,
                    &job.request.temp_id,
                    &job.request.destination.id,
                    &title,
                    description,
                    artist_name,
                    duration_ms,
                    local_audio_file,
                    cover_file,
                    Some(optimistic_track),
                    Some(job.request.destination.clone()),
                    false,
                    Some(request_slice),
                    Some(on_success),
                    Some(on_created),
                    Some(on_error),
                    cx,
                );
            });
        });

        let watchdog_service = this.clone();
        let playlist_id = job.request.destination.id.clone();
        let temp_id = job.request.temp_id.clone();
        let watchdog_continuation = continuation;
        let watchdog = cx.spawn(async move |cx| {
            let stalled =
                Self::await_stalled_message_mp3_transfer(&watchdog_service, &playlist_id, &temp_id, cx).await;
            if stalled {
                watchdog_continuation
                    .unbounded_send(MusicMessageMP3CreateOutcome::Failure(tr("music_local_upload_interrupted")))
                    .ok();
            }
        });

        let outcome = stream.next().await;
        drop(watchdog);
        this.update(cx, |service, _| {
            service.message_mp3_uploaded_temp_ids.remove(&job.request.temp_id);
        })
        .ok();
        outcome.unwrap_or_else(|| MusicMessageMP3CreateOutcome::Failure(tr("music_message_import_account_changed")))
    }

    async fn await_stalled_message_mp3_transfer(
        this: &WeakEntity<Self>,
        playlist_id: &str,
        temp_id: &str,
        cx: &mut AsyncApp,
    ) -> bool {
        fn mark(service: &MusicServicesStore, playlist_id: &str, temp_id: &str, cx: &mut App) -> String {
            let track = service
                .message_mp3_pending_playlist(playlist_id, cx)
                .and_then(|playlist| playlist.tracks)
                .and_then(|tracks| tracks.into_iter().find(|track| track.id.as_deref() == Some(temp_id)));
            format!(
                "{}:{}",
                track
                    .as_ref()
                    .and_then(|track| track.transfer_phase.clone())
                    .unwrap_or_default(),
                track.as_ref().and_then(|track| track.transferred_bytes).unwrap_or(0)
            )
        }

        let idle_limit = Duration::from_secs_f64(SaiWebSocketConfiguration::offline_ack_default_timeout_seconds().max(0.0));
        let Ok(mut last_mark) = this.update(cx, |service, cx| mark(service, playlist_id, temp_id, cx)) else {
            return false;
        };
        let mut idle_since = RiseClock::monotonic();
        loop {
            cx.background_executor().timer(Duration::from_secs(2)).await;
            let Ok((current, is_connected, is_uploaded)) = this.update(cx, |service, cx| {
                (
                    mark(service, playlist_id, temp_id, cx),
                    SaiWs::manager(cx).read(cx).is_connected(),
                    service.message_mp3_uploaded_temp_ids.contains(temp_id),
                )
            }) else {
                return false;
            };
            if current != last_mark || !is_connected || !is_uploaded {
                last_mark = current;
                idle_since = RiseClock::monotonic();
            } else if RiseClock::monotonic().saturating_sub(idle_since) >= idle_limit {
                return true;
            }
        }
    }

    async fn materialize_message_mp3(
        this: &WeakEntity<Self>,
        job: &MusicMessageMP3ImportJob,
        generation: u64,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<PathBuf> {
        if job
            .request
            .source
            .file_size
            .is_some_and(|hinted_size| hinted_size > Self::message_mp3_max_bytes())
        {
            return Err(MusicMessageMP3ImportError::FileTooLarge.into());
        }

        let playback_track_id = format!(
            "chat-audio:{}:{}",
            job.request.source.message_id, job.request.source.media_id
        );
        if let Some(cached) = MusicPlaybackDiskCache::find_cached_url(&playback_track_id, Some(job.account_user_id.as_str())) {
            match Self::copy_and_validate_local_mp3(cached, job).await {
                Ok(copied) => return Ok(copied),
                Err(error) if Self::is_import_error(&error, &MusicMessageMP3ImportError::Mp3Required) => {
                    MusicPlaybackDiskCache::clear_track(&playback_track_id, Some(job.account_user_id.as_str()));
                }
                Err(error) => return Err(error),
            }
        }

        for raw in [job.request.source.local_preview_url.as_deref()] {
            let Some(raw) = raw.map(trimming_whitespaces_and_newlines).filter(|raw| !raw.is_empty()) else {
                continue;
            };
            let Some(local_path) = Self::local_file_url(raw).and_then(|url| url.to_file_path()) else {
                continue;
            };
            if !Self::file_exists_off_main(&local_path).await {
                continue;
            }
            let ext = local_path
                .extension()
                .and_then(|ext| ext.to_str())
                .map(str::to_lowercase)
                .unwrap_or_default();
            if !(ext.is_empty() || ext == "mp3") {
                continue;
            }
            match Self::copy_and_validate_local_mp3(local_path, job).await {
                Ok(copied) => return Ok(copied),
                Err(error) if Self::is_import_error(&error, &MusicMessageMP3ImportError::Mp3Required) => continue,
                Err(error) => return Err(error),
            }
        }

        let Some(remote) = cx.update(|cx| Self::resolved_message_remote_url(&job.request.source.remote_url, cx)) else {
            return Err(MusicMessageMP3ImportError::InvalidSource.into());
        };
        let cache_key = RiseMediaCacheIdentity::key(&remote);
        let lookup_remote = remote.clone();
        let user_id = job.account_user_id.clone();
        let epoch = job.account_epoch.clone();
        let cached = RiseAccountRuntime::current()
            .spawn_blocking(move || {
                RemoteMediaDiskCache::shared().existing_local_url(
                    &lookup_remote,
                    Some(cache_key.as_str()),
                    Some(user_id.as_str()),
                    Some(epoch.as_str()),
                )
            })
            .await
            .ok()
            .flatten();
        if let Some(cached) = cached {
            let lease = RemoteMediaDiskCache::lease(Some(cached.as_path()));
            let copied = Self::copy_and_validate_local_mp3(cached, job).await;
            drop(lease);
            match copied {
                Ok(copied) => return Ok(copied),
                Err(error)
                    if Self::is_import_error(&error, &MusicMessageMP3ImportError::Mp3Required)
                        || Self::is_import_error(&error, &MusicMessageMP3ImportError::InvalidSource) => {}
                Err(error) => return Err(error),
            }
        }
        let downloaded = Self::download_message_import_file(
            this,
            &remote,
            Self::message_mp3_max_bytes(),
            "mp3",
            job,
            true,
            Some(generation),
            0,
            cx,
        )
        .await?;
        match Self::validate_mp3_off_main(&downloaded).await {
            Ok(()) => Ok(downloaded),
            Err(error) => {
                Self::remove_files_off_main(HashSet::from([downloaded])).await;
                Err(error)
            }
        }
    }

    fn is_import_error(error: &anyhow::Error, expected: &MusicMessageMP3ImportError) -> bool {
        error.downcast_ref::<MusicMessageMP3ImportError>() == Some(expected)
    }

    async fn copy_and_validate_local_mp3(source: PathBuf, job: &MusicMessageMP3ImportJob) -> anyhow::Result<PathBuf> {
        if !Self::is_current_account(job) {
            return Err(MusicMessageMP3ImportError::AccountChanged.into());
        }
        let max_bytes = Self::message_mp3_max_bytes();
        let copied = RiseAccountRuntime::current()
            .spawn_blocking(move || MusicMessageMP3ImportFileIO::copy_and_validate_mp3(&source, max_bytes))
            .await
            .map_err(|_| CancellationError)??;
        if !Self::is_current_account(job) {
            Self::remove_files_off_main(HashSet::from([copied])).await;
            return Err(MusicMessageMP3ImportError::AccountChanged.into());
        }
        Ok(copied)
    }

    async fn message_cover_file(
        this: &WeakEntity<Self>,
        source: &MusicMessageMP3ImportSource,
        job: &MusicMessageMP3ImportJob,
        cleanup_urls: &mut HashSet<PathBuf>,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<Option<SaiWsOptimisticLocalFile>> {
        let Some(raw) = source
            .thumbnail_url
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|raw| !raw.is_empty())
        else {
            return Ok(None);
        };

        let local_path = Self::local_file_url(raw).and_then(|url| url.to_file_path());
        let local_exists = match local_path.as_deref() {
            Some(local_path) => Self::file_exists_off_main(local_path).await,
            None => false,
        };
        let mut persistent_cache_key = None;
        let file_url = if let Some(local_path) = local_path.filter(|_| local_exists) {
            local_path
        } else {
            let Some(remote) = cx.update(|cx| Self::resolved_message_remote_url(raw, cx)) else {
                return Ok(None);
            };
            let cache_key = CleverImageDiskCache::normalized_cache_key(remote.absolute_string());
            let lookup_key = cache_key.clone();
            let user_id = job.account_user_id.clone();
            let cached_data = RiseAccountRuntime::current()
                .spawn_blocking(move || CleverImageDiskCache::image_data(&lookup_key, Some(user_id.as_str()), None))
                .await
                .ok()
                .flatten();
            if let Some(cached_data) = cached_data
                && i64::try_from(cached_data.len()).unwrap_or(i64::MAX) <= Self::message_cover_max_bytes()
                && let Some(artwork) = Self::make_artwork_file_off_main(cached_data).await
            {
                return Ok(Some(artwork));
            }
            let path_extension = remote.path_extension();
            let file_url = Self::download_message_import_file(
                this,
                &remote,
                Self::message_cover_max_bytes(),
                if path_extension.is_empty() { "jpg" } else { &path_extension },
                job,
                false,
                None,
                0,
                cx,
            )
            .await?;
            persistent_cache_key = Some(cache_key);
            cleanup_urls.insert(file_url.clone());
            file_url
        };

        let Some(data) = Self::file_data_off_main(&file_url, Self::message_cover_max_bytes()).await else {
            return Ok(None);
        };
        let Some(artwork) = Self::make_artwork_file_off_main(data.clone()).await else {
            return Ok(None);
        };
        if let Some(persistent_cache_key) = persistent_cache_key
            && Self::is_current_account(job)
        {
            let user_id = job.account_user_id.clone();
            let epoch = job.account_epoch.clone();
            RiseAccountRuntime::current()
                .spawn_blocking(move || {
                    if !SaiAccountCacheScope::is_current_account_epoch(Some(user_id.as_str()), Some(epoch.as_str())) {
                        return;
                    }
                    CleverImageDiskCache::store_data(&data, &persistent_cache_key, Some(user_id.as_str()), None);
                })
                .await
                .ok();
        }
        Ok(Some(artwork))
    }

    #[allow(clippy::too_many_arguments)]
    async fn download_message_import_file(
        this: &WeakEntity<Self>,
        remote: &Url,
        max_bytes: i64,
        file_extension: &str,
        job: &MusicMessageMP3ImportJob,
        reports_track_progress: bool,
        generation: Option<u64>,
        retry_attempt: i64,
        cx: &mut AsyncApp,
    ) -> anyhow::Result<PathBuf> {
        let mut retry_attempt = retry_attempt;
        loop {
            if !Self::is_current_account(job) {
                return Err(MusicMessageMP3ImportError::AccountChanged.into());
            }
            let (is_cancelled, current_generation) = this.read_with(&*cx, |service, _| {
                (service.is_import_cancelled(job), service.message_mp3_import_generation)
            })?;
            if is_cancelled || generation.is_some_and(|generation| generation != current_generation) {
                return Err(CancellationError.into());
            }

            let mut request = URLRequest::new(remote.clone());
            request.timeout_interval = Duration::from_secs(120);
            request.http_method = "GET".to_owned();
            request.set_value("*/*", "Accept");

            let (base_url, request_interceptors) = cx.update(|cx| {
                let configuration = SaiFetchHttpConfiguration::shared(cx);
                let configuration = configuration.read(cx);
                (configuration.base_url.clone(), configuration.request_interceptors.clone())
            });
            if !MusicMessageMP3AuthPolicy::is_allowed_remote_url(Some(remote), base_url.as_ref()) {
                return Err(MusicMessageMP3ImportError::InvalidSource.into());
            }
            let is_first_party = MusicMessageMP3AuthPolicy::is_trusted_origin(Some(remote), base_url.as_ref());
            if is_first_party {
                cx.update(AuthTokenRefresh::ensure_valid_access_token).await?;
                if !Self::is_current_account(job) {
                    return Err(MusicMessageMP3ImportError::AccountChanged.into());
                }
                request = Self::intercepted_message_import_request(request, &request_interceptors, cx).await?;
            }
            if !MusicMessageMP3AuthPolicy::is_trusted_origin(Some(&request.url), base_url.as_ref()) {
                MusicMessageMP3AuthPolicy::strip_sensitive_headers(&mut request);
            }

            let (progress_sender, mut progress_receiver) = mpsc::unbounded::<(i64, i64)>();
            let progress_handler: Option<Arc<dyn Fn(i64, i64) + Send + Sync>> = if reports_track_progress {
                Some(Arc::new(move |completed_bytes: i64, total_bytes: i64| {
                    progress_sender.unbounded_send((completed_bytes, total_bytes)).ok();
                }) as Arc<dyn Fn(i64, i64) + Send + Sync>)
            } else {
                None
            };
            let progress_service = this.clone();
            let progress_job = job.clone();
            let _progress_task = cx.spawn(async move |cx| {
                while let Some((completed_bytes, total_bytes)) = progress_receiver.next().await {
                    if !Self::is_current_account(&progress_job) {
                        continue;
                    }
                    progress_service
                        .update(cx, |service, cx| {
                            if generation != Some(service.message_mp3_import_generation) {
                                return;
                            }
                            let total = if total_bytes > 0 {
                                total_bytes
                            } else {
                                progress_job.request.source.file_size.unwrap_or(0).max(0)
                            };
                            service.patch_message_mp3_pending_track(
                                &progress_job.request.destination.id,
                                &progress_job.request.temp_id,
                                MusicLocalTrackTransferPhase::Downloading,
                                completed_bytes,
                                total,
                                None,
                                cx,
                            );
                            MusicTrackUploadNotifier::download(&progress_job.request.temp_id, completed_bytes, total, cx);
                        })
                        .ok();
                }
            });

            let downloader = Arc::new(MusicMessageMP3LimitedDownloader::new(
                &request,
                max_bytes,
                file_extension,
                base_url,
                progress_handler,
            ));
            let (temporary, response) = match downloader.download(request).await {
                Ok(downloaded) => downloaded,
                Err(error) => {
                    let retry_error = error.downcast_ref::<URLError>().copied().map(RemoteMediaTransferError::Url);
                    let should_retry = retry_error
                        .as_ref()
                        .is_some_and(|retry_error| RemoteMediaDownloadRetryPolicy::should_retry(retry_error, retry_attempt));
                    if !should_retry {
                        return Err(error);
                    }
                    let next_attempt = retry_attempt + 1;
                    cx.background_executor()
                        .timer(RemoteMediaDownloadRetryPolicy::delay(next_attempt))
                        .await;
                    retry_attempt = next_attempt;
                    continue;
                }
            };
            if !Self::is_current_account(job) {
                Self::remove_files_off_main(HashSet::from([temporary])).await;
                return Err(MusicMessageMP3ImportError::AccountChanged.into());
            }
            if !(200..=299).contains(&response.status_code) {
                Self::remove_files_off_main(HashSet::from([temporary])).await;
                let status = response.status_code;
                return Err(MusicMessageMP3ImportError::DownloadFailed(if status > 0 {
                    tr_args("music_message_import_http_error", &[FormatArg::Int(status)])
                } else {
                    tr("music_message_import_download_failed")
                })
                .into());
            }
            if response.expected_content_length() > max_bytes {
                Self::remove_files_off_main(HashSet::from([temporary])).await;
                return Err(MusicMessageMP3ImportError::FileTooLarge.into());
            }
            let content_type = response.value("Content-Type").map(str::to_lowercase).unwrap_or_default();
            if content_type.contains("text/html") || content_type.contains("application/xml") {
                Self::remove_files_off_main(HashSet::from([temporary])).await;
                return Err(MusicMessageMP3ImportError::DownloadFailed(tr("music_message_import_download_failed")).into());
            }

            return Ok(temporary);
        }
    }

    async fn intercepted_message_import_request(
        request: URLRequest,
        interceptors: &[SaiFetchRequestInterceptor],
        cx: &mut AsyncApp,
    ) -> anyhow::Result<URLRequest> {
        let mut intercepted = Request::new(request.http_body.clone().unwrap_or_default());
        *intercepted.method_mut() = Method::from_bytes(request.http_method.as_bytes())?;
        *intercepted.uri_mut() = request.url.absolute_string().parse::<Uri>()?;
        for (name, value) in request.all_http_header_fields() {
            intercepted
                .headers_mut()
                .insert(HeaderName::from_bytes(name.as_bytes())?, HeaderValue::from_str(value)?);
        }
        for interceptor in interceptors {
            let pending = cx.update(|cx| interceptor(intercepted, cx));
            intercepted = pending.await?;
        }
        let (parts, body) = intercepted.into_parts();
        let uri = parts.uri.to_string();
        let url = Url::from_string(&uri).ok_or(MusicMessageMP3ImportError::InvalidSource)?;
        let mut out = URLRequest::new(url);
        out.http_method = parts.method.as_str().to_owned();
        out.http_body = (!body.is_empty()).then_some(body);
        out.timeout_interval = request.timeout_interval;
        for (name, value) in &parts.headers {
            if let Ok(value) = value.to_str() {
                out.set_value(value, name.as_str());
            }
        }
        Ok(out)
    }

    fn resolved_message_remote_url(raw: &str, cx: &mut App) -> Option<Url> {
        let trimmed = RemoteMediaURL::optional_normalized(Some(raw))
            .unwrap_or_else(|| trimming_whitespaces_and_newlines(raw).to_owned());
        if trimmed.is_empty() {
            return None;
        }
        if let Some(url) = Url::from_string(&trimmed).filter(|url| url.scheme().is_some()) {
            return Some(url);
        }
        if !trimmed.starts_with('/') {
            return None;
        }
        let base = SaiFetchHttpConfiguration::shared(cx).read(cx).base_url.clone()?;
        let mut components = UrlComponents::from_url(&base, false)?;
        components.set_path(&trimmed);
        components.set_query(None);
        components.set_fragment(None);
        components.url()
    }

    fn resolved_artist_name(entered: &str, embedded: Option<&str>) -> String {
        let clean = trimming_whitespaces_and_newlines(entered);
        let unknowns: HashSet<String> = HashSet::from([
            String::new(),
            tr("chat_audio_unknown_artist"),
            tr("music_unknown_artist"),
        ]);
        if unknowns.contains(clean)
            && let Some(embedded) = embedded
                .map(trimming_whitespaces_and_newlines)
                .filter(|embedded| !embedded.is_empty())
        {
            return embedded.to_owned();
        }
        if clean.is_empty() {
            tr("music_unknown_artist")
        } else {
            clean.to_owned()
        }
    }

    fn mark_message_mp3_import_succeeded(&mut self, request_id: &str, playlist_title: Option<&str>, cx: &mut Context<Self>) {
        self.message_mp3_pending_completions.remove(request_id);
        if !self.message_mp3_completed_request_ids.insert(request_id.to_owned()) {
            return;
        }
        let title = playlist_title.map(trimming_whitespaces_and_newlines).unwrap_or("");
        Notifier::send(
            tr("music_message_import_success_title"),
            NotifierSendOptions {
                content: Some(if title.is_empty() {
                    tr("music_message_import_success_generic")
                } else {
                    tr_args("music_message_import_success", &[FormatArg::Text(title.into())])
                }),
                r#type: NotifierType::Success,
                icon: NotifierIcon::System("music.note.list".to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
        self.prune_message_mp3_completion_state();
    }

    fn is_import_cancelled(&self, job: &MusicMessageMP3ImportJob) -> bool {
        self.message_mp3_cancelled_temp_ids.contains(&job.request.temp_id)
    }

    fn mark_message_mp3_import_failed(
        &mut self,
        job: &MusicMessageMP3ImportJob,
        message: &str,
        total_bytes: i64,
        cx: &mut Context<Self>,
    ) {
        if self.is_import_cancelled(job) {
            return;
        }
        self.message_mp3_failed_jobs.insert(job.request.temp_id.clone(), job.clone());
        let remote = Self::resolved_message_remote_url(&job.request.source.remote_url, cx)
            .or_else(|| Url::from_string(&job.request.source.remote_url));
        if let Some(remote) = remote {
            let artist_name = trimming_whitespaces_and_newlines(&job.request.artist_name);
            let artist_name = if artist_name.is_empty() {
                tr("music_unknown_artist")
            } else {
                artist_name.to_owned()
            };
            let fallback = self.make_local_track_draft(
                &job.request.temp_id,
                trimming_whitespaces_and_newlines(&job.request.title),
                job.request.description.as_deref(),
                &artist_name,
                Self::hinted_duration_ms(&job.request.source),
                &job.account_user_id,
                &remote,
                job.request.source.thumbnail_url.as_deref().unwrap_or(""),
                Some(job.request.source.chat_id.as_str()),
                Some(job.request.source.message_id.as_str()),
                Some(job.request.source.remote_url.as_str()),
                Some(MusicLocalTrackTransferPhase::Failed),
                0,
                total_bytes.max(0),
                Some(message),
            );
            self.insert_local_track_optimistic(&job.request.destination.id, fallback, Some(&job.request.destination), cx);
        }
        self.patch_message_mp3_pending_track(
            &job.request.destination.id,
            &job.request.temp_id,
            MusicLocalTrackTransferPhase::Failed,
            0,
            total_bytes.max(0),
            Some(message),
            cx,
        );
    }

    fn prune_message_mp3_completion_state(&mut self) {
        let cutoff = RiseClock::now().adding_time_interval(-15.0 * 60.0);
        self.message_mp3_pending_completions
            .retain(|_, pending| pending.created_at >= cutoff);
        if self.message_mp3_completed_request_ids.len() > 200 {
            self.message_mp3_completed_request_ids.clear();
        }
    }

    fn is_current_account(job: &MusicMessageMP3ImportJob) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(Some(job.account_user_id.as_str()), Some(job.account_epoch.as_str()))
    }

    fn local_file_url(raw: &str) -> Option<Url> {
        if let Some(url) = Url::from_string(raw).filter(Url::is_file_url) {
            return Some(url);
        }
        if is_local_absolute_path(raw) {
            return Url::from_file_path(Path::new(raw));
        }
        None
    }

    async fn file_exists_off_main(url: &Path) -> bool {
        let url = url.to_path_buf();
        RiseAccountRuntime::current()
            .spawn_blocking(move || MusicMessageMP3ImportFileIO::file_exists(&url))
            .await
            .unwrap_or(false)
    }

    async fn validate_mp3_off_main(url: &Path) -> anyhow::Result<()> {
        let url = url.to_path_buf();
        let max_bytes = Self::message_mp3_max_bytes();
        RiseAccountRuntime::current()
            .spawn_blocking(move || MusicMessageMP3ImportFileIO::validate_mp3(&url, max_bytes))
            .await
            .map_err(|_| CancellationError)?
    }

    async fn file_data_off_main(from_url: &Path, max_bytes: i64) -> Option<Vec<u8>> {
        let url = from_url.to_path_buf();
        RiseAccountRuntime::current()
            .spawn_blocking(move || MusicMessageMP3ImportFileIO::data(&url, max_bytes))
            .await
            .ok()
            .flatten()
    }

    async fn file_size_off_main(url: &Path) -> i64 {
        let url = url.to_path_buf();
        RiseAccountRuntime::current()
            .spawn_blocking(move || MusicMessageMP3ImportFileIO::file_size(&url))
            .await
            .unwrap_or(0)
    }

    async fn make_artwork_file_off_main(from_data: Vec<u8>) -> Option<SaiWsOptimisticLocalFile> {
        RiseAccountRuntime::current()
            .spawn_blocking(move || MusicTrackFileUpload::local_artwork_file(&from_data))
            .await
            .ok()
            .flatten()
    }

    async fn remove_files_off_main(urls: HashSet<PathBuf>) {
        RiseAccountRuntime::current()
            .spawn_blocking(move || MusicMessageMP3ImportFileIO::remove_files(&urls))
            .await
            .ok();
    }

    fn schedule_managed_replacement_cover_cleanup(file: Option<&SaiWsOptimisticLocalFile>) {
        let Some(url) = file.map(|file| &file.url) else {
            return;
        };
        if !Self::is_managed_temporary_file(url) {
            return;
        }
        if let Some(path) = url.to_file_path() {
            Self::schedule_file_cleanup(HashSet::from([path]));
        }
    }

    fn schedule_file_cleanup(urls: HashSet<PathBuf>) {
        if urls.is_empty() {
            return;
        }
        drop(
            RiseAccountRuntime::current().spawn_blocking(move || MusicMessageMP3ImportFileIO::remove_files(&urls)),
        );
    }

    fn is_managed_temporary_file(url: &Url) -> bool {
        if !url.is_file_url() {
            return false;
        }
        let Some(file_path) = url.to_file_path() else {
            return false;
        };
        let file_path = std::fs::canonicalize(&file_path).unwrap_or(file_path);
        let temp_directory = temporary_directory();
        let temp_path = std::fs::canonicalize(&temp_directory).unwrap_or(temp_directory);
        file_path != temp_path && file_path.starts_with(&temp_path)
    }
}
