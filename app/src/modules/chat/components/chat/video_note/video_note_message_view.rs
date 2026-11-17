use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, AsyncWindowContext, ElementId, Entity, FontWeight, IntoElement, ObjectFit, Pixels, RenderOnce, SharedString,
    Size, Subscription, Window, black, canvas, div, img, prelude::*, px, size, transparent_black, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_theme::AppTheme;
use rise_ui::icon_ui::IconUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::font::{Font, FontDesign, SwiftUiFont};
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, use_task};
use rise_ui::swiftui::motion::{MotionSpec, ease_out, linear, spring};
use rise_ui::swiftui::shapes::{Shape, centred_stroke};
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;

use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::remote_media_disk_cache::RemoteMediaDownloadRetryPolicy;
use crate::core::media_viewer::media_viewer_image_cache::{MediaViewerBitmapLoader, MediaViewerImageMemoryCache};
use crate::core::ui::clever_image_disk_cache::CleverImage;
use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi, ContentMode};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::async_render::rise_message_layout_measurement::is_message_layout_measurement;
use crate::modules::chat::components::chat::bubble::bubble_tap_router::BubbleTapRouterModifiers;
use crate::modules::chat::components::chat::bubble::rise_exact_upload_progress_ring::RiseExactUploadProgressRing;
use crate::modules::chat::components::chat::bubble::rise_native_message_image_grid::RiseNativeMediaThumbnailPipeline;
use crate::modules::chat::components::chat::video_note::video_note_floating_pip::paint_progress_trim;
use crate::modules::chat::components::chat::video_note::video_note_player_layer_view::VideoNotePlayerLayerView;
use crate::modules::chat::components::chat::video_note::video_note_player_store::VideoNotePlayerStore;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageMediaItem};
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;

const RESIZE_SPRING_RESPONSE: f64 = 0.34;
const RESIZE_SPRING_DAMPING_FRACTION: f64 = 0.86;
const COUNTER_FADE_DURATION: f64 = 0.12;
const COUNTER_FADE_DELAY: f64 = 0.22;
const PROGRESS_MOTION_DURATION: f64 = 0.05;
const PROGRESS_LINE_WIDTH: f32 = 3.0;
const REST_LINE_WIDTH: f32 = 2.0;
const OVERLAY_BUTTON_SIZE: f32 = 54.0;

/// video note message view, playing size and its member share a name which one rust type cant hold
/// so sizes live here and new hands out the element
pub enum VideoNoteMessageView {}

impl VideoNoteMessageView {
    pub const BUBBLE_SIZE: f32 = 212.0;
    pub const PLAYING_MAX_SIZE: f32 = 300.0;

    /// resize animation, the list own row spring
    pub fn resize_animation() -> MotionSpec {
        spring(RESIZE_SPRING_RESPONSE, RESIZE_SPRING_DAMPING_FRACTION)
    }

    pub fn new(
        id: impl Into<ElementId>,
        item: MessageMediaItem,
        message: GetMessageMessage,
        chat_id: String,
        sender_name: String,
        expanded_size: f32,
    ) -> VideoNoteMessageBody {
        VideoNoteMessageBody {
            id: id.into(),
            item,
            message,
            chat_id,
            sender_name,
            expanded_size,
            playing_size: None,
            on_cancel_upload: Rc::new(|_: &mut Window, _: &mut App| {}),
        }
    }

    pub fn expanded_size(viewport_width: f32, _reserves_avatar_space: bool) -> f32 {
        if viewport_width > 320.0 { 240.0 } else { Self::BUBBLE_SIZE }
    }

    /// a playing note grows in the feed up to the bubble width
    pub fn playing_size(viewport_width: f32, chat: &ChatInfo, is_own: bool) -> f32 {
        let rest = Self::expanded_size(viewport_width, MessageSizes::reserves_avatar_space(chat, is_own));
        let available = MessageSizes::available_bubble_width(viewport_width, chat, is_own);
        rest.max(Self::PLAYING_MAX_SIZE.min(available))
    }
}

struct VideoNoteMessageViewState {
    was_this_playing: bool,
    display_size: Animated<f32>,
    progress: Animated<f32>,
    rest_ring_opacity: Animated<f32>,
    progress_ring_opacity: Animated<f32>,
    counter_opacity: Animated<f32>,
}

#[derive(IntoElement)]
pub struct VideoNoteMessageBody {
    id: ElementId,
    item: MessageMediaItem,
    message: GetMessageMessage,
    chat_id: String,
    sender_name: String,
    expanded_size: f32,
    playing_size: Option<f32>,
    on_cancel_upload: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl VideoNoteMessageBody {
    pub fn playing_size(mut self, playing_size: Option<f32>) -> Self {
        self.playing_size = playing_size;
        self
    }

    pub fn on_cancel_upload(mut self, on_cancel_upload: Rc<dyn Fn(&mut Window, &mut App)>) -> Self {
        self.on_cancel_upload = on_cancel_upload;
        self
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    fn center_badge(symbol: &str, icon_size: f32, is_filled: bool, offset_x: f32) -> impl IntoElement + use<> {
        div().absolute().inset_0().flex().items_center().justify_center().child(
            div()
                .size(px(OVERLAY_BUTTON_SIZE))
                .rounded_full()
                .bg(black().opacity(0.4))
                .flex()
                .items_center()
                .justify_center()
                .child(div().relative().left(px(offset_x)).children(IconUi::sized(
                    symbol,
                    px(icon_size),
                    white(),
                    is_filled,
                ))),
        )
    }

    fn duration_badge(seconds: f64) -> impl IntoElement {
        div()
            .absolute()
            .left(px(10.0))
            .bottom(px(10.0))
            .swiftui_font(Font::system(11.0, FontWeight::NORMAL, FontDesign::Default).monospaced_digit())
            .text_color(white())
            .px(px(7.0))
            .py(px(2.0))
            .rounded_full()
            .bg(black().opacity(0.30))
            .child(RiseFormat::duration_time_interval(seconds))
    }

    fn tap_action(&self) -> impl Fn(&mut Window, &mut App) + 'static + use<> {
        let is_transferring = self.item.is_uploading || self.item.is_compressing;
        let on_cancel_upload = self.on_cancel_upload.clone();
        let url = self.item.url.clone();
        let duration = self.item.duration.unwrap_or(0.0);
        let thumbnail_url = self.item.thumbnail_url.clone();
        let message_id = self.message.id.clone();
        let sender_id = self.message.sender_id.clone();
        let chat_id = self.chat_id.clone();
        let sender_name = self.sender_name.clone();
        move |window, cx| {
            if is_transferring {
                on_cancel_upload(window, cx);
                return;
            }
            let store = VideoNotePlayerStore::shared(cx);
            let is_this_playing = {
                let store = store.read(cx);
                store.is_active() && store.message_id() == message_id
            };
            if is_this_playing {
                store.update(cx, |store, cx| store.pause_or_resume(cx));
                return;
            }
            store.update(cx, |store, cx| {
                store.play(&url, &message_id, &chat_id, &sender_name, duration, thumbnail_url.as_deref(), cx);
            });
            let my_id = AuthServicesStore::shared(cx)
                .read(cx)
                .profile()
                .map(|profile| profile.id.clone())
                .unwrap_or_default();
            if sender_id != my_id {
                MessageInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.mark_media_played(&chat_id, &message_id, cx));
            }
        }
    }

    fn live_body(self, window: &mut Window, cx: &mut App) -> impl IntoElement + use<> {
        let theme: AppTheme = rise_ui::theme(cx).clone();
        let host = window.current_view();
        let observed_id = self.message.id.clone();
        let observer_key = ElementId::NamedChild(
            Arc::new(self.id.clone()),
            SharedString::from(format!("store-observer-{}", self.message.id)),
        );
        let _store_observer = window.use_keyed_state(observer_key, cx, move |_, cx| -> Subscription {
            let store = VideoNotePlayerStore::shared(cx);
            let was_this = Cell::new(false);
            App::observe(cx, &store, move |store, cx: &mut App| {
                let is_this = {
                    let store = store.read(cx);
                    store.is_active() && store.message_id() == observed_id
                };
                let was_this_playing = was_this.replace(is_this);
                if is_this || was_this_playing {
                    cx.notify(host);
                }
            })
        });

        let store = VideoNotePlayerStore::shared(cx);
        let (is_this_playing, player, display_epoch, progress, elapsed_seconds) = {
            let store = store.read(cx);
            (
                store.is_active() && store.message_id() == self.message.id,
                store.player(),
                store.display_epoch(),
                store.progress(),
                store.elapsed_seconds(),
            )
        };
        let target_size =
            if is_this_playing { self.playing_size.unwrap_or(self.expanded_size) } else { self.expanded_size };
        let is_transferring = self.item.is_uploading || self.item.is_compressing;

        let playing_flag = if is_this_playing { 1.0 } else { 0.0 };
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, _| VideoNoteMessageViewState {
            was_this_playing: is_this_playing,
            display_size: Animated::new(target_size),
            progress: Animated::new(if is_this_playing { progress } else { 0.0 }),
            rest_ring_opacity: Animated::new(1.0 - playing_flag),
            progress_ring_opacity: Animated::new(playing_flag),
            counter_opacity: Animated::new(playing_flag),
        });
        let now = RiseClock::monotonic();
        let (display_size, shown_progress, rest_ring_opacity, progress_ring_opacity, counter_opacity) =
            state.update(cx, |state, cx| {
                let changed = state.was_this_playing != is_this_playing;
                state.was_this_playing = is_this_playing;
                let resize = changed.then(VideoNoteMessageView::resize_animation);
                state.display_size.set_in(target_size, resize, now, cx);
                state.rest_ring_opacity.set_in(1.0 - playing_flag, resize, now, cx);
                if is_this_playing {
                    let step = resize.unwrap_or_else(|| linear(PROGRESS_MOTION_DURATION));
                    state.progress.set_in(progress, step, now, cx);
                    state.progress_ring_opacity.set_in(1.0, resize, now, cx);
                    if changed {
                        state.counter_opacity.jump(0.0);
                        let fade = ease_out(COUNTER_FADE_DURATION).delay(COUNTER_FADE_DELAY);
                        state.counter_opacity.set_in(1.0, fade, now, cx);
                    }
                } else {
                    state.progress.jump(0.0);
                    state.progress_ring_opacity.jump(0.0);
                    state.counter_opacity.jump(0.0);
                }
                (
                    state.display_size.frame_value(now, window),
                    state.progress.frame_value(now, window),
                    state.rest_ring_opacity.frame_value(now, window),
                    state.progress_ring_opacity.frame_value(now, window),
                    state.counter_opacity.frame_value(now, window),
                )
            });

        let accent = theme.primary._100;
        let source = self.item.display_source();
        let video_source = self.item.url.clone();
        let thumbnail_id = self.child_id("thumbnail");
        let tap_id = self.child_id("tap");
        let label_id = self.child_id("label");
        let ring_id = self.child_id("upload-ring");
        let player_disappear_id = self.child_id("player-disappear");
        let tap = self.tap_action();

        let mut stack = div()
            .id(label_id)
            .tooltip_label(tr("msg_preview_video_note"))
            .relative()
            .size(px(display_size))
            .cursor_pointer();

        stack = stack.child(circle_clip().child(VideoNoteThumbnail::new(
            thumbnail_id,
            source,
            video_source,
            self.expanded_size,
        )));
        if is_this_playing && !is_transferring {
            stack = stack.child(
                circle_clip().child(VideoNotePlayerLayerView::new(player, display_epoch)).on_disappear(
                    player_disappear_id,
                    {
                        let store = store.clone();
                        move |_, cx| store.update(cx, |store, cx| store.bump_display(cx))
                    },
                ),
            );
        }

        if is_transferring {
            stack = stack.child(
                div().absolute().inset_0().child(
                    RiseExactUploadProgressRing::new(ring_id, self.message.id.clone(), self.item.clone(), false)
                        .progress_color(accent)
                        .track_color(transparent_black())
                        .line_width(PROGRESS_LINE_WIDTH),
                ),
            );
        } else {
            stack = stack
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .opacity(rest_ring_opacity)
                        .child(centred_stroke(accent.opacity(0.4), REST_LINE_WIDTH, Shape::Circle)),
                )
                .child(
                    canvas(
                        |_, _, _| (),
                        move |bounds, (), window, _| {
                            if progress_ring_opacity <= 0.0 {
                                return;
                            }
                            x767_mark("note stack", bounds, window);
                            let color = accent.opacity(progress_ring_opacity);
                            paint_progress_trim(bounds, shown_progress, PROGRESS_LINE_WIDTH, color, window);
                        },
                    )
                    .absolute()
                    .inset_0(),
                );
        }

        if is_transferring {
            stack = stack.child(Self::center_badge("xmark", 18.0, false, 0.0));
        } else if !is_this_playing {
            stack = stack.child(Self::center_badge("play.fill", 20.0, true, 2.0));
        } else {
            stack = stack.child(div().absolute().inset_0().opacity(counter_opacity).child(Self::duration_badge(elapsed_seconds)));
        }

        stack.bubble_tap_target(tap_id, tap)
    }
}

/// diagnostic mark for the playing note geometry, logged only when the bounds change
pub(crate) fn x767_mark(label: &str, bounds: gpui::Bounds<Pixels>, window: &Window) {
    thread_local! {
        static LAST: std::cell::RefCell<std::collections::HashMap<String, String>> = Default::default();
    }
    let line = format!("bounds={:?} mask={:?} viewport={:?}", bounds, window.content_mask().bounds, window.viewport_size());
    let changed = LAST.with(|last| last.borrow_mut().insert(label.to_owned(), line.clone()).as_deref() != Some(line.as_str()));
    if changed {
        tracing::warn!(target: "riseonly::X767", "{label} {line}");
    }
}

/// circle clip over the whole area of the note
fn circle_clip() -> gpui::Div {
    div().absolute().inset_0().rounded_full().overflow_hidden()
}

impl RenderOnce for VideoNoteMessageBody {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if is_message_layout_measurement(cx) {
            return div().w(px(self.expanded_size)).h(px(self.expanded_size)).into_any_element();
        }
        self.live_body(window, cx).into_any_element()
    }
}

#[derive(Default)]
struct VideoNoteThumbnailState {
    photo_library_image: Option<CleverImage>,
    loaded_photo_library_source: String,
    poster_image: Option<CleverImage>,
}

#[derive(IntoElement)]
struct VideoNoteThumbnail {
    id: ElementId,
    source: String,
    video_source: String,
    size: f32,
}

impl VideoNoteThumbnail {
    fn new(id: ElementId, source: String, video_source: String, size: f32) -> Self {
        Self { id, source, video_source, size }
    }

    fn child_id(&self, name: &'static str) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static(name))
    }

    /// memory only lookup, cheap enough for the first render pass
    fn cached_poster(video_source: &str, target: Size<Pixels>, screen_scale: f32) -> Option<CleverImage> {
        let video = trimming_whitespaces_and_newlines(video_source);
        if video.is_empty() {
            return None;
        }
        let user_id = SaiAccountCacheScope::active_user_id();
        let epoch = SaiAccountCacheScope::active_epoch();
        RiseNativeMediaThumbnailPipeline::cached_thumbnail(
            "",
            video,
            target,
            screen_scale,
            1.0,
            false,
            false,
            user_id.as_deref(),
            epoch.as_deref(),
        )
        .map(|thumbnail| thumbnail.image)
    }

    fn set_poster(state: &Entity<VideoNoteThumbnailState>, poster: Option<CleverImage>, cx: &mut AsyncWindowContext) {
        let _ = cx.update(|_, cx| {
            state.update(cx, |state, cx| {
                state.poster_image = poster;
                cx.notify();
            });
        });
    }

    async fn load_poster(
        state: Entity<VideoNoteThumbnailState>,
        video_source: String,
        target: Size<Pixels>,
        screen_scale: f32,
        cx: &mut AsyncWindowContext,
    ) {
        Self::set_poster(&state, None, cx);
        let video = trimming_whitespaces_and_newlines(&video_source).to_owned();
        if video.is_empty() {
            return;
        }
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        if let Some(cached) = Self::cached_poster(&video, target, screen_scale) {
            Self::set_poster(&state, Some(cached), cx);
            return;
        }
        let maximum_retries = RemoteMediaDownloadRetryPolicy::maximum_retries();
        for attempt in 0..=maximum_retries {
            if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
                return;
            }
            let resolved = RiseNativeMediaThumbnailPipeline::thumbnail(
                "",
                &video,
                target,
                screen_scale,
                1.0,
                false,
                false,
                account_user_id.clone(),
                account_epoch.clone(),
                &mut *cx,
            )
            .await;
            if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref()) {
                return;
            }
            if let Some(resolved) = resolved {
                Self::set_poster(&state, Some(resolved.image), cx);
                return;
            }
            if attempt >= maximum_retries {
                return;
            }
            let retry = cx.background_executor().timer(RemoteMediaDownloadRetryPolicy::delay(attempt + 1));
            retry.await;
        }
    }

    async fn load_photo_library_image(
        state: Entity<VideoNoteThumbnailState>,
        source: String,
        cx: &mut AsyncWindowContext,
    ) {
        let user_id = SaiAccountCacheScope::active_user_id();
        let loaded = match MediaViewerImageMemoryCache::memory_only(&source, user_id.as_deref()) {
            Some(cached) => Some(cached),
            None => {
                MediaViewerBitmapLoader::load_into_memory(&source, false, false, CleverImageCachePolicy::MemoryOnly, cx)
                    .await
            }
        };
        let _ = cx.update(|_, cx| {
            state.update(cx, |state, cx| {
                state.photo_library_image = loaded;
                state.loaded_photo_library_source = source;
                cx.notify();
            });
        });
    }

    fn picture(image: Option<CleverImage>, theme: &AppTheme) -> gpui::AnyElement {
        match image {
            Some(image) => img(image).size_full().rounded_full().object_fit(ObjectFit::Cover).into_any_element(),
            None => div().size_full().rounded_full().bg(theme.bg._300).into_any_element(),
        }
    }
}

impl RenderOnce for VideoNoteThumbnail {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let normalized_source = trimming_whitespaces_and_newlines(&self.source).to_owned();
        let state = window.use_keyed_state(self.child_id("state"), cx, |_, _| VideoNoteThumbnailState::default());

        if normalized_source.to_lowercase().starts_with("ph://") {
            let user_id = SaiAccountCacheScope::active_user_id();
            let immediate = {
                let state = state.read(cx);
                if state.loaded_photo_library_source == normalized_source {
                    state.photo_library_image.clone()
                } else {
                    MediaViewerImageMemoryCache::memory_only(&normalized_source, user_id.as_deref())
                }
            };
            let task_source = normalized_source.clone();
            let task_state = state;
            use_task(self.child_id("photo-library-task"), normalized_source, window, cx, async move |cx| {
                Self::load_photo_library_image(task_state, task_source, cx).await;
            });
            return Self::picture(immediate, &theme);
        }

        if normalized_source.is_empty() || MessageMediaItem::has_video_file_extension(&normalized_source) {
            let target = size(px(self.size), px(self.size));
            let screen_scale = window.scale_factor();
            let poster = state
                .read(cx)
                .poster_image
                .clone()
                .or_else(|| Self::cached_poster(&self.video_source, target, screen_scale));
            let task_video_source = self.video_source.clone();
            let task_state = state;
            use_task(self.child_id("poster-task"), self.video_source.clone(), window, cx, async move |cx| {
                Self::load_poster(task_state, task_video_source, target, screen_scale, cx).await;
            });
            return Self::picture(poster, &theme);
        }

        CleverImageUi::new(normalized_source)
            .content_mode(ContentMode::Fill)
            .corner_radius(1.0e6)
            .persist_to_disk(true)
            .maximum_decode_pixel_dimension(((self.size * 3.0).ceil() as i64).max(1))
            .into_any_element()
    }
}
