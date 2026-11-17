use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, LazyLock, mpsc};
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc as async_mpsc;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AsyncApp, Context, ElementId, Entity, Hsla, InteractiveElement, IntoElement, ObjectFit,
    ParentElement, RenderOnce, SharedString, StatefulInteractiveElement, Styled, StyledImage, Subscription, Window,
    black, div, img, px, relative, transparent_black, white,
};
use rise_canon::RiseLimits;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::ns_cache::NSCache;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_engine::rise_upload::rise_url_session::URLRequest;
use rise_media::audio::audio_file_metadata::{AudioFileMetadata, AudioFileRange, AudioFileRangeFetcher};
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::lifecycle::use_task;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::progress_view::{ControlSize, progress_view};

use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::{RemoteMediaDiskCache, RemoteMediaDownloadProgress};
use crate::core::media::rise_media_transfer_hub::{RiseMediaCacheIdentity, RiseMediaTransferKey};
use crate::core::media::rise_progressive_data_fetcher::{RiseProgressiveDataFetcher, RiseProgressiveDataFetcherLane};
use crate::core::ui::clever_image_disk_cache::{
    CleverImage, CleverImageAsyncWorkLimiter, CleverImageBitmapDecoder, CleverImageBitmapLoader, CleverImageLoadResult,
    CleverImageSharedLoadRegistry, CleverImageWorkLane,
};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::ui::rise_media_transfer_chip::{RiseMediaTransferChip, RiseMediaTransferChipViewMetrics};
use crate::modules::chat::components::chat::async_render::rise_message_layout_measurement::is_message_layout_measurement;
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouterModifiers;
use crate::modules::chat::components::chat::bubble::chat_audio_track_adapter::ChatAudioTrackAdapter;
use crate::modules::chat::components::chat::bubble::rise_exact_upload_progress_ring::RiseExactUploadProgressRing;
use crate::modules::chat::shared::chat_audio_queue_continuation::ChatAudioQueueContinuationOrder;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageMediaItem, RemoteMediaURL,
};
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::music::shared::utils::music::featured_profile_track_store::FeaturedProfileTrackStore;
use crate::modules::music::shared::utils::music::music_cover_letterbox::MusicCoverLetterbox;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::shared::utils::music::music_player_state::{MusicPlayOptions, MusicPlayerState};

const MAXIMUM_ARTWORK_PIXEL_DIMENSION: i64 = 132;
const METADATA_REFRESH_DELAY: Duration = Duration::from_millis(180);

pub type MessageAudioViewAction = Rc<dyn Fn(&mut Window, &mut App)>;

fn scoped_id(parent: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(parent.clone()), SharedString::new_static(name))
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct MessageAudioArtworkLoaderRequest {
    artwork_source: Option<String>,
    embedded_sources: Vec<String>,
    account_user_id: Option<String>,
    account_epoch: Option<String>,
}

impl MessageAudioArtworkLoaderRequest {
    fn cache_key(&self) -> String {
        let account_user_id = self.account_user_id.as_deref().unwrap_or_default();
        let account_epoch = self.account_epoch.as_deref().unwrap_or_default();
        let artwork_source = self.artwork_source.as_deref().unwrap_or_default();
        [account_user_id, account_epoch, artwork_source]
            .into_iter()
            .chain(self.embedded_sources.iter().map(String::as_str))
            .map(|part| format!("{}:{part}", part.len()))
            .collect()
    }

    fn is_current(&self) -> bool {
        SaiAccountCacheScope::is_current_account_epoch(self.account_user_id.as_deref(), self.account_epoch.as_deref())
    }
}

static CACHE: LazyLock<NSCache<String, CleverImage>> = LazyLock::new(|| {
    let cache = NSCache::new();
    cache.set_count_limit(usize::try_from(RiseLimits::client_value("message_audio_artwork_cache_entries")).unwrap_or(0));
    cache.set_total_cost_limit(usize::try_from(RiseLimits::client_value("message_audio_artwork_cache_bytes")).unwrap_or(0));
    cache
});
static LIMITER: LazyLock<CleverImageAsyncWorkLimiter> = LazyLock::new(|| {
    CleverImageAsyncWorkLimiter::new(RiseLimits::client_value("message_audio_artwork_load_concurrency"))
});
static LOADS: LazyLock<CleverImageSharedLoadRegistry<MessageAudioArtworkLoaderRequest, CleverImageLoadResult>> =
    LazyLock::new(CleverImageSharedLoadRegistry::new);

/// one range the blocking tag reader asks the async side for
struct MessageAudioArtworkRangeRequest {
    offset: u64,
    length: u64,
    reply: mpsc::Sender<io::Result<AudioFileRange>>,
}

struct MessageAudioArtworkRangeFetcher {
    requests: async_mpsc::UnboundedSender<MessageAudioArtworkRangeRequest>,
}

impl AudioFileRangeFetcher for MessageAudioArtworkRangeFetcher {
    fn fetch(&self, offset: u64, length: u64) -> io::Result<AudioFileRange> {
        let (reply, answer) = mpsc::channel();
        self.requests
            .unbounded_send(MessageAudioArtworkRangeRequest { offset, length, reply })
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        answer.recv().map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?
    }
}

enum MessageAudioArtworkLoader {}

impl MessageAudioArtworkLoader {
    fn cached(request: &MessageAudioArtworkLoaderRequest) -> Option<CleverImage> {
        if !request.is_current() {
            return None;
        }
        CACHE.object(&request.cache_key())
    }

    async fn load(request: MessageAudioArtworkLoaderRequest, cx: &AsyncApp) -> Option<CleverImage> {
        if !request.is_current() {
            return None;
        }
        if let Some(image) = Self::cached(&request) {
            return Some(image);
        }
        let shared_request = request.clone();
        let result = LOADS
            .value(
                request.clone(),
                async move |cx: &mut AsyncApp| {
                    let Some(_permit) = LIMITER.acquire(CleverImageWorkLane::Visible).await else {
                        return CleverImageLoadResult::new(None);
                    };
                    if !shared_request.is_current() {
                        return CleverImageLoadResult::new(None);
                    }
                    let image = Self::prepare(&shared_request, cx).await;
                    if !shared_request.is_current() {
                        return CleverImageLoadResult::new(None);
                    }
                    if let Some(image) = &image {
                        let cost = image.as_bytes(0).map_or(0, <[u8]>::len);
                        CACHE.set_object_cost(image.clone(), shared_request.cache_key(), cost);
                    }
                    CleverImageLoadResult::new(image)
                },
                cx,
            )
            .await;
        if request.is_current() { result.image } else { None }
    }

    async fn prepare(request: &MessageAudioArtworkLoaderRequest, cx: &AsyncApp) -> Option<CleverImage> {
        if let Some(source) = request.artwork_source.as_deref() {
            let image = match Self::local_url(source) {
                Some(path) => {
                    cx.background_executor()
                        .spawn(async move {
                            let data = std::fs::read(path).ok()?;
                            CleverImageBitmapDecoder::decode(&data, Some(MAXIMUM_ARTWORK_PIXEL_DIMENSION))
                        })
                        .await
                }
                None => {
                    CleverImageBitmapLoader::load(
                        source,
                        true,
                        true,
                        request.account_user_id.clone(),
                        request.account_epoch.clone(),
                        Some(MAXIMUM_ARTWORK_PIXEL_DIMENSION),
                        CleverImageCachePolicy::Standard,
                        CleverImageWorkLane::Visible,
                        cx,
                    )
                    .await
                }
            };
            if !request.is_current() {
                return None;
            }
            if let Some(image) = image {
                return Some(cx.background_executor().spawn(async move { MusicCoverLetterbox::trimmed(&image) }).await);
            }
        }
        for source in &request.embedded_sources {
            if !request.is_current() {
                return None;
            }
            let image = Self::embedded_artwork(source, request, cx).await;
            if image.is_some() {
                return image;
            }
        }
        None
    }

    async fn embedded_artwork(
        source: &str,
        request: &MessageAudioArtworkLoaderRequest,
        cx: &AsyncApp,
    ) -> Option<CleverImage> {
        let file_source = source.to_owned();
        let account_user_id = request.account_user_id.clone();
        let account_epoch = request.account_epoch.clone();
        let answered = cx
            .background_executor()
            .spawn(async move {
                Self::file_artwork(&file_source, account_user_id.as_deref(), account_epoch.as_deref())
            })
            .await;
        match answered {
            Ok(image) => image,
            Err(remote) => Self::remote_embedded_artwork(remote, request, cx).await,
        }
    }

    fn file_artwork(
        source: &str,
        account_user_id: Option<&str>,
        account_epoch: Option<&str>,
    ) -> Result<Option<CleverImage>, Url> {
        let path = match Self::local_url(source) {
            Some(path) => path,
            None => {
                let Some(remote) = Url::from_string(source) else {
                    return Ok(None);
                };
                let cache_key = RiseMediaCacheIdentity::key(&remote);
                let existing = RemoteMediaDiskCache::shared().existing_local_url(
                    &remote,
                    Some(cache_key.as_str()),
                    account_user_id,
                    account_epoch,
                );
                match existing {
                    Some(path) => path,
                    None => return Err(remote),
                }
            }
        };
        let Ok(data) = std::fs::read(&path) else {
            return Ok(None);
        };
        let file_extension = path.extension().and_then(|extension| extension.to_str());
        Ok(Self::artwork_image(AudioFileMetadata::read(data, file_extension)))
    }

    async fn remote_embedded_artwork(
        remote: Url,
        request: &MessageAudioArtworkLoaderRequest,
        cx: &AsyncApp,
    ) -> Option<CleverImage> {
        let bearer = RemoteMediaDiskCache::bearer_token(&remote, None, cx).await;
        if !request.is_current() {
            return None;
        }
        let (requests, mut pending) = async_mpsc::unbounded::<MessageAudioArtworkRangeRequest>();
        let reader = RiseAccountRuntime::current().spawn_blocking(move || {
            let fetcher = MessageAudioArtworkRangeFetcher { requests };
            Self::artwork_image(AudioFileMetadata::read_ranges(fetcher, None))
        });
        let mut cx = cx.clone();
        while let Some(range) = pending.next().await {
            if !request.is_current() {
                return None;
            }
            let fetched = Self::fetch_range(&remote, bearer.as_deref(), &range, request, &mut cx).await;
            range.reply.send(fetched).ok();
        }
        reader.await.ok().flatten()
    }

    async fn fetch_range(
        remote: &Url,
        bearer: Option<&str>,
        range: &MessageAudioArtworkRangeRequest,
        request: &MessageAudioArtworkLoaderRequest,
        cx: &mut AsyncApp,
    ) -> io::Result<AudioFileRange> {
        let mut url_request = URLRequest::new(remote.clone());
        url_request.set_value("identity", reqwest::header::ACCEPT_ENCODING.as_str());
        url_request.set_value(
            &format!("bytes={}-{}", range.offset, range.offset + range.length.saturating_sub(1)),
            reqwest::header::RANGE.as_str(),
        );
        if let Some(bearer) = bearer {
            url_request.set_value(&format!("Bearer {bearer}"), reqwest::header::AUTHORIZATION.as_str());
        }
        let (bytes, response) = RiseProgressiveDataFetcher::data(
            url_request,
            i64::try_from(AudioFileMetadata::MAXIMUM_RANGE_READ_BYTES).unwrap_or(i64::MAX),
            RiseProgressiveDataFetcherLane::PrivateEphemeral,
            request.account_user_id.clone(),
            request.account_epoch.clone(),
            RiseProgressiveDataFetcher::DEFAULT_PRIORITY,
            Arc::new(|_: RemoteMediaDownloadProgress| {}),
            cx,
        )
        .await
        .map_err(|error| io::Error::other(error.into_anyhow().to_string()))?;
        match response.status_code {
            206 => {
                let total_length = response
                    .content_range_total()
                    .and_then(|total| u64::try_from(total).ok())
                    .ok_or_else(|| io::Error::from(io::ErrorKind::InvalidData))?;
                Ok(AudioFileRange {
                    offset: range.offset,
                    bytes,
                    total_length,
                })
            }
            200 => {
                let total_length = bytes.len() as u64;
                Ok(AudioFileRange {
                    offset: 0,
                    bytes,
                    total_length,
                })
            }
            _ => Err(io::Error::from(io::ErrorKind::InvalidData)),
        }
    }

    fn artwork_image(metadata: AudioFileMetadata) -> Option<CleverImage> {
        let artwork = metadata.artwork?;
        let image = CleverImageBitmapDecoder::decode(&artwork, Some(MAXIMUM_ARTWORK_PIXEL_DIMENSION))?;
        Some(MusicCoverLetterbox::trimmed(&image))
    }

    fn local_url(source: &str) -> Option<PathBuf> {
        if source.starts_with("file://") {
            return Url::from_string(source).and_then(|url| url.to_file_path());
        }
        if source.starts_with('/') || Path::new(source).is_absolute() {
            return Some(PathBuf::from(source));
        }
        None
    }
}

#[derive(Default)]
struct MessageAudioViewState {
    artwork_image: Option<CleverImage>,
    loaded_artwork_identity: Option<MessageAudioArtworkLoaderRequest>,
}

#[derive(IntoElement)]
pub struct MessageAudioView {
    id: ElementId,
    item: MessageMediaItem,
    upload_owner_local_id: String,
    caption: String,
    is_own: bool,
    is_playing: bool,
    is_loading: bool,
    progress: f64,
    on_toggle_play: MessageAudioViewAction,
    on_cancel_upload: MessageAudioViewAction,
}

impl MessageAudioView {
    pub fn new(id: impl Into<ElementId>, item: MessageMediaItem) -> Self {
        Self {
            id: id.into(),
            item,
            upload_owner_local_id: String::new(),
            caption: String::new(),
            is_own: false,
            is_playing: false,
            is_loading: false,
            progress: 0.0,
            on_toggle_play: Rc::new(|_: &mut Window, _: &mut App| {}),
            on_cancel_upload: Rc::new(|_: &mut Window, _: &mut App| {}),
        }
    }

    pub fn upload_owner_local_id(mut self, upload_owner_local_id: impl Into<String>) -> Self {
        self.upload_owner_local_id = upload_owner_local_id.into();
        self
    }

    pub fn caption(mut self, caption: impl Into<String>) -> Self {
        self.caption = caption.into();
        self
    }

    pub fn is_own(mut self, is_own: bool) -> Self {
        self.is_own = is_own;
        self
    }

    pub fn is_playing(mut self, is_playing: bool) -> Self {
        self.is_playing = is_playing;
        self
    }

    pub fn is_loading(mut self, is_loading: bool) -> Self {
        self.is_loading = is_loading;
        self
    }

    pub fn progress(mut self, progress: f64) -> Self {
        self.progress = progress;
        self
    }

    pub fn on_toggle_play(mut self, on_toggle_play: MessageAudioViewAction) -> Self {
        self.on_toggle_play = on_toggle_play;
        self
    }

    pub fn on_cancel_upload(mut self, on_cancel_upload: MessageAudioViewAction) -> Self {
        self.on_cancel_upload = on_cancel_upload;
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        scoped_id(&self.id, name)
    }

    fn subtitle_color(&self, theme: &AppTheme) -> Hsla {
        if self.is_own { theme.content_on(theme.primary._100).opacity(0.78) } else { theme.text.secondary }
    }

    fn is_transferring(&self) -> bool {
        self.item.is_uploading || self.item.is_compressing
    }

    fn transfer_key(&self) -> Option<RiseMediaTransferKey> {
        if self.is_transferring() || self.item.resolved_local_preview_url().is_some() {
            return None;
        }
        let normalized = RemoteMediaURL::optional_normalized(Some(self.item.url.as_str()))?;
        let remote = Url::from_string(&normalized)?;
        let scheme = remote.scheme().map(str::to_lowercase);
        if !matches!(scheme.as_deref(), Some("https" | "http")) {
            return None;
        }
        Some(RiseMediaTransferKey::for_active_account(RiseMediaCacheIdentity::key(&remote)))
    }

    fn artwork_source(&self) -> Option<String> {
        let raw = trimming_whitespaces_and_newlines(self.item.thumbnail_url.as_deref()?);
        if raw.is_empty() {
            return None;
        }
        RemoteMediaURL::optional_normalized(Some(raw)).or_else(|| Some(raw.to_owned()))
    }

    fn artwork_load_identity(&self) -> MessageAudioArtworkLoaderRequest {
        let embedded_sources = [self.item.local_preview_url.as_deref(), Some(self.item.url.as_str())]
            .into_iter()
            .flatten()
            .filter_map(|raw| {
                let source = trimming_whitespaces_and_newlines(raw);
                if source.is_empty() {
                    return None;
                }
                if MessageAudioArtworkLoader::local_url(source).is_some() {
                    return Some(source.to_owned());
                }
                RemoteMediaURL::optional_normalized(Some(source))
            })
            .collect();
        MessageAudioArtworkLoaderRequest {
            artwork_source: self.artwork_source(),
            embedded_sources,
            account_user_id: SaiAccountCacheScope::active_user_id(),
            account_epoch: SaiAccountCacheScope::active_epoch(),
        }
    }

    fn audio_control(
        &self,
        theme: &AppTheme,
        artwork: Option<CleverImage>,
        is_layout_measurement: bool,
    ) -> gpui::Div {
        let control = div().relative().flex_none().flex().items_center().justify_center().frame_size(44.0, 44.0);
        let glyph_color = if artwork.is_none() {
            theme.content_on(theme.primary._100)
        } else {
            white()
        };
        let has_artwork = artwork.is_some();
        let control = match artwork {
            Some(artwork) => control
                .child(img(artwork).absolute().inset_0().size_full().rounded_full().object_fit(ObjectFit::Cover))
                .child(div().absolute().inset_0().rounded_full().bg(black().opacity(0.35))),
            None => control.child(div().absolute().inset_0().rounded_full().bg(if self.is_own {
                theme.content_on(theme.primary._100).opacity(0.20)
            } else {
                theme.primary._100
            })),
        };
        control.children(self.control_glyph(glyph_color, has_artwork, is_layout_measurement))
    }

    fn control_glyph(&self, glyph_color: Hsla, has_artwork: bool, is_layout_measurement: bool) -> Vec<AnyElement> {
        if self.is_transferring() {
            let ring = RiseExactUploadProgressRing::new(
                self.child_id("upload-ring"),
                self.upload_owner_local_id.clone(),
                self.item.clone(),
                is_layout_measurement,
            )
            .progress_color(glyph_color)
            .track_color(transparent_black())
            .line_width(2.5);
            return vec![
                div().absolute().inset_0().child(ring).into_any_element(),
                div().relative().children(IconUi::sized("xmark", px(12.0), glyph_color, false)).into_any_element(),
            ];
        }
        if self.is_loading {
            let Some(key) = self.transfer_key() else {
                return vec![
                    progress_view()
                        .tint(glyph_color)
                        .control_size(ControlSize::Small)
                        .into_any_element(),
                ];
            };
            let chip = RiseMediaTransferChip::new(self.child_id("transfer-ring"), Some(key))
                .declared_byte_count(self.item.effective_total_bytes())
                .metrics(RiseMediaTransferChipViewMetrics::ring(44.0, 2.5, Some(glyph_color), Some(transparent_black())))
                .shows_byte_counts(false)
                .shows_idle_affordance(false)
                .is_interactive(false)
                .hides_small_transfers(false)
                .is_layout_measurement(is_layout_measurement);
            return vec![
                div().absolute().inset_0().child(chip).into_any_element(),
                div().relative().children(IconUi::sized("arrow.down", px(11.0), glyph_color, false)).into_any_element(),
            ];
        }
        let symbol = if self.is_playing { "pause.fill" } else { "play.fill" };
        let glyph = div()
            .relative()
            .left(px(if self.is_playing { 0.0 } else { 1.0 }))
            .flex()
            .children(IconUi::sized(symbol, px(14.0), glyph_color, true));
        if has_artwork {
            return vec![
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(px(32.0))
                    .rounded_full()
                    .bg(black().opacity(0.68))
                    .border_1()
                    .border_color(white().opacity(0.22))
                    .child(glyph)
                    .into_any_element(),
            ];
        }
        vec![glyph.into_any_element()]
    }

    fn progress_track(&self, theme: &AppTheme, value: f64) -> gpui::Div {
        let accent = if self.is_own { theme.content_on(theme.primary._100) } else { theme.primary._100 };
        let track = if self.is_own { theme.content_on(theme.primary._100).opacity(0.22) } else { theme.text.secondary.opacity(0.25) };
        div().relative().w_full().h(px(2.5)).rounded_full().bg(track).child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .bottom_0()
                .w(relative(value.clamp(0.0, 1.0) as f32))
                .rounded_full()
                .bg(accent),
        )
    }
}

impl RenderOnce for MessageAudioView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = ThemeStore::shared(cx).read(cx).current_theme().clone();
        let is_layout_measurement = is_message_layout_measurement(cx);
        let (title, artist) = ChatAudioTrackAdapter::title_and_artist(&self.item, &self.caption);
        let duration = self.item.duration.map(RiseFormat::duration_time_interval);
        let description = match &duration {
            Some(duration) => format!("{artist} \u{b7} {duration}"),
            None => artist.clone(),
        };

        let state = window.use_keyed_state(self.child_id("artwork-state"), cx, |_, _| MessageAudioViewState::default());
        let artwork_request = self.artwork_load_identity();
        let artwork = if is_layout_measurement {
            None
        } else {
            let current = state.read(cx);
            current
                .artwork_image
                .clone()
                .filter(|_| current.loaded_artwork_identity.as_ref() == Some(&artwork_request))
                .or_else(|| MessageAudioArtworkLoader::cached(&artwork_request))
        };

        let task_request = artwork_request.clone();
        let task_state = state.clone();
        use_task(self.child_id("artwork-task"), artwork_request, window, cx, async move |cx| {
            if is_layout_measurement || !task_request.is_current() {
                let _ = cx.update(|_, cx| {
                    task_state.update(cx, |state, cx| {
                        state.artwork_image = None;
                        state.loaded_artwork_identity = None;
                        cx.notify();
                    })
                });
                return;
            }
            let cached = MessageAudioArtworkLoader::cached(&task_request);
            let is_cached = cached.is_some();
            let identity = task_request.clone();
            let _ = cx.update(|_, cx| {
                task_state.update(cx, |state, cx| {
                    state.artwork_image = cached;
                    state.loaded_artwork_identity = Some(identity);
                    cx.notify();
                })
            });
            if is_cached {
                return;
            }
            let loaded = MessageAudioArtworkLoader::load(task_request.clone(), cx).await;
            if !task_request.is_current() {
                return;
            }
            let _ = cx.update(|_, cx| {
                task_state.update(cx, |state, cx| {
                    if state.loaded_artwork_identity.as_ref() == Some(&task_request) {
                        state.artwork_image = loaded;
                        cx.notify();
                    }
                })
            });
        });

        let subtitle_color = self.subtitle_color(&theme);
        let is_transferring = self.is_transferring();
        let progress = if is_transferring {
            self.item.normalized_transfer_progress()
        } else if self.is_loading {
            0.0
        } else {
            self.progress
        };
        let inline_chip = self.transfer_key().filter(|_| self.is_loading).map(|key| {
            RiseMediaTransferChip::new(self.child_id("transfer-inline"), Some(key))
                .declared_byte_count(self.item.effective_total_bytes())
                .metrics(RiseMediaTransferChipViewMetrics::inline_label(13.0, subtitle_color))
                .shows_idle_affordance(true)
                .shows_size_when_cached(false)
                .is_interactive(false)
                .is_layout_measurement(is_layout_measurement)
        });
        let on_toggle_play = self.on_toggle_play.clone();
        let on_cancel_upload = self.on_cancel_upload.clone();

        vstack(8.0)
            .align(HorizontalAlignment::Leading)
            .w_full()
            .min_w_0()
            .cursor_pointer()
            .child(
                hstack(8.0)
                    .w_full()
                    .min_h(px(44.0))
                    .child(self.audio_control(&theme, artwork, is_layout_measurement))
                    .child(
                        vstack(2.0)
                            .align(HorizontalAlignment::Leading)
                            .flex_1()
                            .min_w_0()
                            .child(
                                MainText::new(title.clone())
                                    .px(16.0)
                                    .color(if self.is_own { theme.content_on(theme.primary._100) } else { theme.text.primary })
                                    .number_of_lines(1),
                            )
                            .child(
                                div()
                                    .relative()
                                    .w_full()
                                    .min_w_0()
                                    .child(
                                        hstack(0.0)
                                            .w_full()
                                            .min_w_0()
                                            .opacity(if self.is_loading { 0.0 } else { 1.0 })
                                            .child(
                                                MainText::new(artist)
                                                    .px(13.0)
                                                    .color(subtitle_color)
                                                    .number_of_lines(1)
                                                    .min_w_0(),
                                            )
                                            .when_some(duration, |row, duration| {
                                                row.child(
                                                    MainText::new(format!(" \u{b7} {duration}"))
                                                        .px(13.0)
                                                        .color(subtitle_color)
                                                        .monospaced_digits(true)
                                                        .number_of_lines(1)
                                                        .flex_none(),
                                                )
                                            }),
                                    )
                                    .when_some(inline_chip, |stack, chip| {
                                        stack.child(
                                            div()
                                                .absolute()
                                                .left_0()
                                                .top_0()
                                                .bottom_0()
                                                .flex()
                                                .items_center()
                                                .child(chip),
                                        )
                                    }),
                            )
                            .child(self.progress_track(&theme, progress).mt(px(3.0))),
                    ),
            )
            .when(!self.caption.is_empty(), |content| {
                content.child(
                    MainText::new(self.caption.clone())
                        .px(MessageSizes::text_font_size())
                        .color(if self.is_own { theme.content_on(theme.primary._100).opacity(0.95) } else { theme.text.primary }),
                )
            })
            .id(self.child_id("root"))
            .aria_label(format!("{title}, {description}"))
            .bubble_tap_target(self.id.clone(), move |window: &mut Window, cx: &mut App| {
                if is_transferring {
                    on_cancel_upload(window, cx);
                } else {
                    on_toggle_play(window, cx);
                }
            })
    }
}

/// what a row shows of the shared player
/// a row whose track isnt current reads nothing else so it stays equal through ticks
#[derive(Clone, Copy, PartialEq)]
struct AudioMessagePlaybackRowPlayback {
    is_playing: bool,
    is_loading: bool,
    progress: f64,
}

impl AudioMessagePlaybackRowPlayback {
    fn of(player: &MusicPlayerState, track: &TrackModel) -> Self {
        if ChatAudioTrackAdapter::is_same_audio(player.current_track.as_ref(), track) {
            Self { is_playing: player.is_playing, is_loading: player.is_preparing_playback, progress: player.progress }
        } else {
            Self { is_playing: false, is_loading: false, progress: 0.0 }
        }
    }
}

struct AudioMessagePlaybackRowObservation {
    track: TrackModel,
    playback: AudioMessagePlaybackRowPlayback,
    _subscription: Subscription,
    _position_subscription: Subscription,
}

impl AudioMessagePlaybackRowObservation {
    fn new(track: TrackModel, player: &Entity<MusicPlayerState>, cx: &mut Context<Self>) -> Self {
        let playback = AudioMessagePlaybackRowPlayback::of(player.read(cx), &track);
        let publication = player.read(cx).position_publication().clone();
        let positioned = player.clone();
        Self {
            track,
            playback,
            _subscription: cx.observe(player, |this, player, cx| {
                let playback = AudioMessagePlaybackRowPlayback::of(player.read(cx), &this.track);
                this.set_playback(playback, cx);
            }),
            _position_subscription: cx.observe(&publication, move |this, _, cx| {
                let playback = AudioMessagePlaybackRowPlayback::of(positioned.read(cx), &this.track);
                this.set_playback(playback, cx);
            }),
        }
    }

    fn set_playback(&mut self, playback: AudioMessagePlaybackRowPlayback, cx: &mut Context<Self>) {
        if playback != self.playback {
            self.playback = playback;
            cx.notify();
        }
    }
}

#[derive(IntoElement)]
pub struct AudioMessagePlaybackRow {
    id: ElementId,
    message_id: String,
    cache_message_id: Option<String>,
    chat_id: String,
    item: MessageMediaItem,
    chat_messages: Arc<Vec<Arc<GetMessageMessage>>>,
    caption: String,
    is_own: bool,
    on_cancel_upload: MessageAudioViewAction,
}

impl AudioMessagePlaybackRow {
    pub fn new(
        id: impl Into<ElementId>,
        message_id: impl Into<String>,
        chat_id: impl Into<String>,
        item: MessageMediaItem,
        chat_messages: Arc<Vec<Arc<GetMessageMessage>>>,
    ) -> Self {
        Self {
            id: id.into(),
            message_id: message_id.into(),
            cache_message_id: None,
            chat_id: chat_id.into(),
            item,
            chat_messages,
            caption: String::new(),
            is_own: false,
            on_cancel_upload: Rc::new(|_: &mut Window, _: &mut App| {}),
        }
    }

    pub fn cache_message_id(mut self, cache_message_id: impl Into<Option<String>>) -> Self {
        self.cache_message_id = cache_message_id.into();
        self
    }

    pub fn caption(mut self, caption: impl Into<String>) -> Self {
        self.caption = caption.into();
        self
    }

    pub fn is_own(mut self, is_own: bool) -> Self {
        self.is_own = is_own;
        self
    }

    pub fn on_cancel_upload(mut self, on_cancel_upload: MessageAudioViewAction) -> Self {
        self.on_cancel_upload = on_cancel_upload;
        self
    }

    fn track(&self) -> TrackModel {
        ChatAudioTrackAdapter::track(
            &self.message_id,
            self.cache_message_id.as_deref(),
            &self.chat_id,
            &self.item,
            &self.caption,
        )
    }

    fn toggle_play(
        track: &TrackModel,
        chat_id: &str,
        chat_messages: &[Arc<GetMessageMessage>],
        cx: &mut App,
    ) {
        let player = MusicPlayerState::shared(cx);
        if ChatAudioTrackAdapter::is_same_audio(player.read(cx).current_track.as_ref(), track) {
            player.update(cx, |player, cx| player.toggle_play_pause(cx));
            return;
        }
        let options = MusicPlayOptions {
            queue: ChatAudioTrackAdapter::tracks(chat_id, chat_messages),
            scope_id: Some(format!("chat:{chat_id}")),
            record_play_on_success: false,
            ..MusicPlayOptions::default()
        };
        player.update(cx, |player, cx| player.play(track.clone(), options, cx));
        MessageInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.continue_chat_audio_queue(chat_id, ChatAudioQueueContinuationOrder::Chronological, cx);
        });
    }
}

impl RenderOnce for AudioMessagePlaybackRow {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let track = self.track();
        let player = MusicPlayerState::shared(cx);
        let observed_track = track.clone();
        let observed_player = player.clone();
        let observation = window.use_keyed_state(scoped_id(&self.id, "player"), cx, move |_, cx| {
            AudioMessagePlaybackRowObservation::new(observed_track, &observed_player, cx)
        });
        let playback = observation.update(cx, |observation, cx| {
            observation.playback = AudioMessagePlaybackRowPlayback::of(player.read(cx), &track);
            observation.track = track.clone();
            observation.playback
        });

        let is_layout_measurement = is_message_layout_measurement(cx);
        let task_id = format!("{}|{}|{}|{}", track.id, track.title, track.artist, track.stable_audio_url());
        let refreshed_track = track.clone();
        use_task(scoped_id(&self.id, "metadata-task"), task_id, window, cx, async move |cx| {
            if is_layout_measurement {
                return;
            }
            cx.background_executor().timer(METADATA_REFRESH_DELAY).await;
            let _ = cx.update(|_, cx| {
                MusicPlayerState::shared(cx).update(cx, |player, cx| {
                    player.refresh_chat_track_metadata(refreshed_track.clone(), cx);
                });
                FeaturedProfileTrackStore::shared(cx).update(cx, |store, cx| {
                    store.refresh_if_matching(&refreshed_track, cx);
                });
            });
        });

        let chat_id = self.chat_id.clone();
        let chat_messages = self.chat_messages.clone();
        MessageAudioView::new(scoped_id(&self.id, "view"), self.item.clone())
            .upload_owner_local_id(self.message_id.clone())
            .caption(self.caption.clone())
            .is_own(self.is_own)
            .is_playing(playback.is_playing)
            .is_loading(playback.is_loading)
            .progress(playback.progress)
            .on_toggle_play(Rc::new(move |_: &mut Window, cx: &mut App| {
                Self::toggle_play(&track, &chat_id, &chat_messages, cx);
            }))
            .on_cancel_upload(self.on_cancel_upload.clone())
    }
}
