use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, AnyView, App, AppContext, Bounds, ClickEvent, Context, DispatchPhase, ElementId, Entity, EntityId, FocusHandle,
    Focusable, Hitbox, HitboxBehavior, Hsla, ImageSource, InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent,
    MouseMoveEvent, ObjectFit, ParentElement, Pixels, Point, Render, ScrollDelta, ScrollWheelEvent, SharedString, Size,
    StatefulInteractiveElement, Styled, StyledImage, Subscription, Task, WeakEntity, Window, black, canvas, div, hsla, img, point,
    px, size, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription};
use rise_i18n::tr;
use rise_platform::wheel_gesture::{WHEEL_SETTLE_DELAY, WheelGestureTracker, WheelStep};
use rise_theme::alpha;
use rise_ui::hold_context_menu_ui::bubble_scroll_passthrough::BubbleScrollPassthrough;
use rise_ui::IconUi;
use rise_ui::image_ui::render_image_atlas::RenderImageAtlas;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::drag_gesture::{DragGesture, DragGestureModifiers, DragGestureValue};
use rise_ui::swiftui::environment::LayoutDirection;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::{ease_in_out, ease_out};
use rise_ui::swiftui::progress_view::progress_view;
use rise_ui::window_screen_frame_reader::{
    MediaCellScreenFrameRegistry, WindowScreenCoordinates, WindowScreenFrameReader, WindowScreenFrameReaderFrameView,
};

use crate::app::router::in_app_link_browser::InAppLinkBrowser;
use crate::app::router::navigation_store::NavigationStoreSettlingPush;
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::{SAI_WS_DID_CONNECT, SAI_WS_DID_DISCONNECT};
use crate::core::lib::sai::modules::sai_ws::sai_ws::SaiWs;
use crate::core::media_viewer::media_viewer_image_cache::MediaViewerImageMemoryCache;
use crate::core::media_viewer::media_viewer_presentation_state::MediaViewerPresentationState;
use crate::core::media_viewer::media_viewer_types::MediaViewerFrameKeys;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapLoader, CleverImageWorkLane};
use crate::core::ui::media_link_areas_overlay::MediaLinkZone;
use crate::core::ui::pager_back_swipe_yield::{LeadingPageReporting, PagerBackSwipeYield};
use crate::modules::post::components::download::post_download_policy::PostMediaFocus;
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_link_zones_view::{
    PostRowLinkZonesView, PostRowLinkZonesViewTarget,
};
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use rise_core::foundation::foundation_url::Url;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::post::components::collaborators::post_tag_bubbles_overlay::{PostMediaTagsState, PostTagBubblesLayer};
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::components::post_feed_media_prefetch::PostFeedMediaPrefetchPolicy;
use crate::modules::post::components::repost::repost_overlay::RepostStickerStyle;
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_media_viewer_lift_observer::{
    PostListMediaViewerLiftObservation, PostListMediaViewerLiftObserver,
};
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_playback_focus::PostListFocusFrameView;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostListFocusReporting, PostListHostedEnvironment, PostListPlaybackToken, PostListPlaybackTokenKind, PostRowActionRouting,
    PostRowAnchorRegistering, PostRowBodyKind, PostRowMainContext, PostRowMediaBoxLayout, PostRowNativeKind, PostRowPalette,
    PostRowRoute, PreparedPostRow, PreparedPostRowKey,
};
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_row_action_router::PostRowMediaPageSource;
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_native_row_view::{
    PostRowMediaDoubleTap, PostRowMediaHosting,
};
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_hosted_leaf::{
    PostRowAmbientMusicDriver, PostRowHostedLeafView, PostRowInlineVideoLeaf, PostRowLeafHitPolicy, PostRowRepostStickerLeaf,
    PostRowRiffCardLeaf,
};
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_media_overlays::{
    PostRowBackdropCache, PostRowBackdropRenderer, PostRowMediaCaches, PostRowMediaMetrics, PostRowMusicPlateView,
    PostRowPageIndicatorView, PostRowPlayGlyphView, PostRowSoundButton, PostRowTagsChipView,
};
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_riff_card_view::PostRowRiffCardView;
use crate::modules::post::shared::post_media_box::PostMediaBox;
use crate::modules::post::shared::post_media_slides::PostMediaSlide;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;

/// tap handler of a control, runs outside any update of the view that owns it
pub type PostRowMediaTap = Rc<dyn Fn(&mut App)>;

#[derive(Clone, Debug, PartialEq)]
pub struct PostRowMediaPageViewContent {
    pub poster_url: Option<String>,
    pub is_video: bool,
    pub natural_aspect: Option<f32>,
    pub layout_fills: bool,
    pub box_aspect: f32,
    pub box_size: Size<Pixels>,
    pub is_own: bool,
    pub account_user_id: Option<String>,
    pub account_epoch: Option<String>,
    pub memory_buckets: Vec<Option<i64>>,
    pub stores_viewer_image: bool,
    pub draws_placeholder: bool,
}

impl PostRowMediaPageViewContent {
    /// network load decodes at the first memory bucket so its bitmap lands where the next read looks
    fn load_pixel_dimension(&self) -> Option<i64> {
        self.memory_buckets.first().copied().flatten()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PostRowMediaPageViewSurface {
    Blank,
    Loading,
    Image,
    Failed,
}

/// one media page with placeholder, blur fill and photo or poster and and for video the play glyph
/// sound switch and hosted player leaf frames come from the box and nothing here measures
pub struct PostRowMediaPageView {
    pub index: usize,
    pub video_leaf: Entity<PostRowHostedLeafView<PostRowInlineVideoLeaf>>,
    reader_id: String,
    is_slide_lifted: bool,
    focus_frame_view: Rc<PostListFocusFrameView>,
    is_muted: bool,
    on_sound_tap: Option<PostRowMediaTap>,

    content: Option<PostRowMediaPageViewContent>,
    decoded_aspect: Option<f32>,
    is_current: bool,
    palette: Option<PostRowPalette>,
    installed_image: Option<CleverImage>,
    surface: PostRowMediaPageViewSurface,
    wants_image: bool,
    deferred_image: bool,
    image_generation: u64,
    image_task: Option<Task<()>>,
    backdrop_generation: u64,
    backdrop_task: Option<Task<()>>,
    backdrop_key: Option<String>,
    backdrop_image: Option<CleverImage>,
    backdrop_is_hidden: bool,
    shows_video_sound_button: bool,
    is_visible_in_viewport: bool,
    play_glyph_is_hidden: bool,
    sound_button_is_hidden: bool,
    _socket_observers: (RiseSubscription, RiseSubscription),
    _socket_task: Task<()>,
}

impl PostRowMediaPageView {
    const FAILURE_GLYPH_POINT_SIZE: f32 = 17.0;

    /// row_view is the mounted row entity the page draws in
    pub fn new(row_view: EntityId, cx: &mut Context<Self>) -> Self {
        let (signal, mut signals) = mpsc::unbounded::<bool>();
        let disconnect_signal = signal.clone();
        let disconnect = RiseAppEvents::default_center().observe(SAI_WS_DID_DISCONNECT, move |_| {
            let _ = disconnect_signal.unbounded_send(false);
        });
        let connect = RiseAppEvents::default_center().observe(SAI_WS_DID_CONNECT, move |_| {
            let _ = signal.unbounded_send(true);
        });
        let socket_task = cx.spawn(async move |this, cx| {
            while let Some(is_connected) = signals.next().await {
                let delivered = this.update(cx, |this, cx| {
                    if is_connected {
                        this.socket_did_connect(cx);
                    } else {
                        this.socket_did_disconnect(cx);
                    }
                });
                if delivered.is_err() {
                    break;
                }
            }
        });
        Self {
            index: 0,
            video_leaf: cx.new(PostRowHostedLeafView::new),
            reader_id: format!("post-row-media-page-{}", cx.entity_id().as_u64()),
            is_slide_lifted: false,
            focus_frame_view: Rc::new(PostListFocusFrameView::new(row_view)),
            is_muted: true,
            on_sound_tap: None,
            content: None,
            decoded_aspect: None,
            is_current: false,
            palette: None,
            installed_image: None,
            surface: PostRowMediaPageViewSurface::Blank,
            wants_image: true,
            deferred_image: false,
            image_generation: 0,
            image_task: None,
            backdrop_generation: 0,
            backdrop_task: None,
            backdrop_key: None,
            backdrop_image: None,
            backdrop_is_hidden: true,
            shows_video_sound_button: false,
            is_visible_in_viewport: true,
            play_glyph_is_hidden: true,
            sound_button_is_hidden: true,
            _socket_observers: (disconnect, connect),
            _socket_task: socket_task,
        }
    }

    pub fn content(&self) -> Option<&PostRowMediaPageViewContent> {
        self.content.as_ref()
    }

    pub fn decoded_aspect(&self) -> Option<f32> {
        self.decoded_aspect
    }

    pub fn is_current(&self) -> bool {
        self.is_current
    }

    pub fn sound_button_is_hidden(&self) -> bool {
        self.sound_button_is_hidden
    }

    pub fn focus_frame_view(&self) -> &Rc<PostListFocusFrameView> {
        &self.focus_frame_view
    }

    /// the slide record of register_slide, none until the page was drawn once
    pub fn anchor_view(&self) -> Option<WindowScreenFrameReaderFrameView> {
        MediaCellScreenFrameRegistry::attached_view(&self.reader_id)
    }

    pub fn set_slide_lifted(&mut self, is_lifted: bool, cx: &mut Context<Self>) {
        if self.is_slide_lifted != is_lifted {
            self.is_slide_lifted = is_lifted;
            cx.notify();
        }
    }

    pub fn set_on_sound_tap(&mut self, on_sound_tap: Option<PostRowMediaTap>) {
        self.on_sound_tap = on_sound_tap;
    }

    pub fn set_muted(&mut self, is_muted: bool, cx: &mut Context<Self>) {
        if self.is_muted != is_muted {
            self.is_muted = is_muted;
            cx.notify();
        }
    }

    /// what the viewer reads of a page as its source view
    pub fn media_page_source(&self) -> Option<PostRowMediaPageSource> {
        let screen_frame = WindowScreenCoordinates::screen_frame(&self.anchor_view()?)?;
        Some(PostRowMediaPageSource {
            index: self.index,
            decoded_aspect: self.decoded_aspect,
            resolved_fills: self.resolved_fills(),
            screen_frame,
        })
    }

    /// fill or fit, uses the prepared aspect if known or else the decoded picture
    pub fn resolved_fills(&self) -> bool {
        let Some(content) = &self.content else {
            return true;
        };
        if content.natural_aspect.is_some() {
            return content.layout_fills;
        }
        PostMediaBox::fills(self.decoded_aspect, content.box_aspect)
    }

    pub fn trimmed_source(raw: Option<&str>) -> Option<String> {
        let value = trimming_whitespaces_and_newlines(raw?);
        if value.is_empty() { None } else { Some(value.to_owned()) }
    }

    /// sound button at bottom trailing with 12 padding
    pub fn sound_button_frame(in_size: Size<Pixels>) -> Bounds<Pixels> {
        let tap = PostRowMediaMetrics::SOUND_BUTTON_TAP;
        let inset = PostRowMediaMetrics::OVERLAY_INSET;
        Bounds {
            origin: point(in_size.width - px(inset + tap), in_size.height - px(inset + tap)),
            size: size(px(tap), px(tap)),
        }
    }

    pub fn configure(&mut self, content: PostRowMediaPageViewContent, palette: PostRowPalette, cx: &mut Context<Self>) {
        let source_changed = match &self.content {
            None => true,
            Some(previous) => {
                previous.poster_url != content.poster_url
                    || previous.is_own != content.is_own
                    || previous.account_user_id != content.account_user_id
                    || previous.account_epoch != content.account_epoch
            }
        };
        self.content = Some(content);
        self.palette = Some(palette);
        if source_changed {
            self.cancel_image();
            self.begin_image(cx);
        } else {
            cx.notify();
        }
        self.update_backdrop(cx);
        self.refresh_video_chrome(cx);
    }

    pub fn set_current(&mut self, current: bool, shows_video_sound_button: bool, cx: &mut Context<Self>) {
        self.is_current = current;
        self.shows_video_sound_button = shows_video_sound_button;
        self.refresh_video_chrome(cx);
    }

    pub fn set_wants_image(&mut self, wants_image: bool, cx: &mut Context<Self>) {
        if self.wants_image == wants_image {
            return;
        }
        self.wants_image = wants_image;
        if wants_image && self.deferred_image && self.image_task.is_none() && self.installed_image.is_none() {
            self.begin_image(cx);
        }
    }

    pub fn set_visible_in_viewport(&mut self, is_visible: bool, cx: &mut Context<Self>) {
        if self.is_visible_in_viewport == is_visible {
            return;
        }
        self.is_visible_in_viewport = is_visible;
        if self.surface == PostRowMediaPageViewSurface::Loading {
            cx.notify();
        }
    }

    pub fn mount_video_leaf(
        &mut self,
        leaf: PostRowInlineVideoLeaf,
        shows_sound_button: bool,
        environment: &PostListHostedEnvironment,
        cx: &mut Context<Self>,
    ) {
        let rects = match (&self.content, shows_sound_button) {
            (Some(content), true) => vec![Self::sound_button_frame(content.box_size)],
            _ => Vec::new(),
        };
        self.video_leaf.update(cx, |view, cx| {
            view.hit_policy = if shows_sound_button { PostRowLeafHitPolicy::Rects } else { PostRowLeafHitPolicy::None };
            view.interactive_rects = rects;
            view.mount(leaf, environment, cx);
        });
        self.refresh_video_chrome(cx);
    }

    pub fn update_video_leaf(
        &mut self,
        leaf: PostRowInlineVideoLeaf,
        environment: &PostListHostedEnvironment,
        cx: &mut Context<Self>,
    ) {
        self.video_leaf.update(cx, |view, cx| view.update(leaf, environment, cx));
    }

    pub fn unmount_video_leaf(&mut self, cx: &mut Context<Self>) {
        if !self.video_leaf.read(cx).is_mounted() {
            return;
        }
        self.video_leaf.update(cx, |view, cx| {
            view.unmount(cx);
            view.hit_policy = PostRowLeafHitPolicy::None;
            view.interactive_rects = Vec::new();
        });
        self.refresh_video_chrome(cx);
    }

    pub fn prepare_for_reuse(&mut self, cx: &mut Context<Self>) {
        self.cancel_image();
        self.hide_backdrop(cx);
        self.unmount_video_leaf(cx);
        self.focus_frame_view.clear_painted();
        self.is_slide_lifted = false;
        self.content = None;
        self.decoded_aspect = None;
        self.installed_image = None;
        self.wants_image = true;
        self.deferred_image = false;
        self.is_current = false;
        self.shows_video_sound_button = false;
        self.set_surface(PostRowMediaPageViewSurface::Blank, cx);
        self.refresh_video_chrome(cx);
    }

    fn cancel_image(&mut self) {
        self.image_generation = self.image_generation.wrapping_add(1);
        self.image_task = None;
    }

    fn begin_image(&mut self, cx: &mut Context<Self>) {
        self.installed_image = None;
        self.decoded_aspect = None;
        self.deferred_image = false;
        let Some(content) = self.content.clone() else {
            self.set_surface(PostRowMediaPageViewSurface::Blank, cx);
            return;
        };
        let Some(url) = content.poster_url.clone() else {
            self.set_surface(PostRowMediaPageViewSurface::Blank, cx);
            return;
        };
        let policy = PostMediaCachePolicy::FEED_IMAGE_POLICY;
        for bucket in &content.memory_buckets {
            let image = CleverImageBitmapLoader::memory_image(
                &url,
                PostMediaCachePolicy::FEED_IMAGES_PERSIST_TO_DISK,
                content.account_user_id.as_deref(),
                content.account_epoch.as_deref(),
                *bucket,
                policy,
                None,
                false,
            );
            if let Some(image) = image {
                self.install(image, &url, cx);
                return;
            }
        }
        if !self.wants_image {
            self.deferred_image = true;
            self.set_surface(PostRowMediaPageViewSurface::Loading, cx);
            return;
        }
        let surface = if Self::link_is_down(cx) {
            PostRowMediaPageViewSurface::Failed
        } else {
            PostRowMediaPageViewSurface::Loading
        };
        self.set_surface(surface, cx);
        self.image_generation = self.image_generation.wrapping_add(1);
        let generation = self.image_generation;
        let persists_to_disk = PostMediaCachePolicy::FEED_IMAGES_PERSIST_TO_DISK;
        let user_id = content.account_user_id.clone();
        let epoch = content.account_epoch.clone();
        let maximum_pixel_dimension = content.load_pixel_dimension();
        self.image_task = Some(cx.spawn(async move |this, cx| {
            let image = CleverImageBitmapLoader::load(
                &url,
                persists_to_disk,
                true,
                user_id.clone(),
                epoch.clone(),
                maximum_pixel_dimension,
                policy,
                CleverImageWorkLane::Visible,
                cx,
            )
            .await;
            this.update(cx, |this, cx| {
                let is_same_source = this.content.as_ref().and_then(|content| content.poster_url.as_deref()) == Some(url.as_str());
                if this.image_generation != generation
                    || !is_same_source
                    || !SaiAccountCacheScope::is_current_account_epoch(user_id.as_deref(), epoch.as_deref())
                {
                    return;
                }
                this.image_task = None;
                if let Some(image) = image {
                    this.install(image, &url, cx);
                } else if this.installed_image.is_none() {
                    this.set_surface(PostRowMediaPageViewSurface::Failed, cx);
                }
            })
            .ok();
        }));
    }

    fn link_is_down(cx: &mut App) -> bool {
        let manager = SaiWs::manager(cx);
        let manager = manager.read(cx);
        !manager.is_connected() && manager.is_reconnecting_after_failure()
    }

    fn socket_did_disconnect(&mut self, cx: &mut Context<Self>) {
        if self.surface != PostRowMediaPageViewSurface::Loading || self.image_task.is_none() {
            return;
        }
        self.set_surface(PostRowMediaPageViewSurface::Failed, cx);
    }

    fn socket_did_connect(&mut self, cx: &mut Context<Self>) {
        if self.surface != PostRowMediaPageViewSurface::Failed || self.image_task.is_none() {
            return;
        }
        self.set_surface(PostRowMediaPageViewSurface::Loading, cx);
    }

    fn install(&mut self, image: CleverImage, url: &str, cx: &mut Context<Self>) {
        let pixel_size = image.size(0);
        self.decoded_aspect = if pixel_size.width.0 > 0 && pixel_size.height.0 > 0 {
            Some(pixel_size.width.0 as f32 / pixel_size.height.0 as f32)
        } else {
            None
        };
        self.installed_image = Some(image.clone());
        self.set_surface(PostRowMediaPageViewSurface::Image, cx);
        if let Some(content) = &self.content
            && content.stores_viewer_image
            && content.load_pixel_dimension().is_none()
        {
            MediaViewerImageMemoryCache::store(
                image,
                &RemoteMediaURL::display_source(url, None),
                content.account_user_id.as_deref(),
            );
        }
        self.update_backdrop(cx);
    }

    fn set_surface(&mut self, next: PostRowMediaPageViewSurface, cx: &mut Context<Self>) {
        self.surface = next;
        cx.notify();
    }

    fn surface_chrome(&self) -> (Option<Hsla>, bool, bool) {
        let draws_placeholder = self.content.as_ref().is_none_or(|content| content.draws_placeholder);
        let placeholder = self.palette.as_ref().map(|palette| palette.background300);
        let solid_black = Some(black());
        match self.surface {
            PostRowMediaPageViewSurface::Blank => (solid_black, false, false),
            PostRowMediaPageViewSurface::Loading => (
                if draws_placeholder { placeholder } else { solid_black },
                draws_placeholder && self.is_visible_in_viewport,
                false,
            ),
            PostRowMediaPageViewSurface::Image => (if draws_placeholder { None } else { solid_black }, false, false),
            PostRowMediaPageViewSurface::Failed => {
                (if draws_placeholder { placeholder } else { solid_black }, false, draws_placeholder)
            }
        }
    }

    fn update_backdrop(&mut self, cx: &mut Context<Self>) {
        let fills = self.resolved_fills();
        let Some(content) = self.content.clone() else {
            self.hide_backdrop(cx);
            return;
        };
        let box_is_measured = f32::from(content.box_size.width) > 1.0 && f32::from(content.box_size.height) > 1.0;
        let url = match (&content.poster_url, fills, box_is_measured) {
            (Some(url), false, true) => url.clone(),
            _ => {
                self.hide_backdrop(cx);
                return;
            }
        };
        self.backdrop_is_hidden = false;

        let key = PostRowBackdropCache::key(
            &url,
            content.account_user_id.as_deref(),
            content.account_epoch.as_deref(),
            Self::f32_size(content.box_size),
        );
        if self.backdrop_key.as_deref() == Some(key.as_str()) {
            return;
        }
        if let Some(cached) = PostRowBackdropCache::shared().image(&key) {
            self.cancel_backdrop();
            self.backdrop_key = Some(key);
            self.set_backdrop(Some(cached), cx);
            return;
        }
        let bucket = PostRowMediaMetrics::BLUR_DECODE_PIXEL_DIMENSION;
        let small = CleverImageBitmapLoader::memory_image(
            &url,
            PostMediaCachePolicy::FEED_IMAGES_PERSIST_TO_DISK,
            content.account_user_id.as_deref(),
            content.account_epoch.as_deref(),
            Some(bucket),
            PostMediaCachePolicy::FEED_IMAGE_POLICY,
            None,
            false,
        );
        let Some(source) = small.or_else(|| self.installed_image.clone()) else {
            return;
        };

        self.cancel_backdrop();
        self.backdrop_key = Some(key.clone());
        self.set_backdrop(None, cx);
        self.backdrop_generation = self.backdrop_generation.wrapping_add(1);
        let generation = self.backdrop_generation;
        let display_size = Self::f32_size(content.box_size);
        let user_id = content.account_user_id;
        let epoch = content.account_epoch;
        let render =
            cx.background_spawn(async move { PostRowBackdropRenderer::render(&source, display_size, bucket) });
        self.backdrop_task = Some(cx.spawn(async move |this, cx| {
            let rendered = render.await;
            this.update(cx, |this, cx| {
                if this.backdrop_generation != generation
                    || !SaiAccountCacheScope::is_current_account_epoch(user_id.as_deref(), epoch.as_deref())
                {
                    return;
                }
                let Some(rendered) = rendered else {
                    return;
                };
                this.backdrop_task = None;
                PostRowBackdropCache::shared().insert(rendered.clone(), key.as_str());
                this.set_backdrop(Some(rendered), cx);
            })
            .ok();
        }));
    }

    fn f32_size(size_in_pixels: Size<Pixels>) -> Size<f32> {
        size(f32::from(size_in_pixels.width), f32::from(size_in_pixels.height))
    }

    fn cancel_backdrop(&mut self) {
        self.backdrop_generation = self.backdrop_generation.wrapping_add(1);
        self.backdrop_task = None;
    }

    fn hide_backdrop(&mut self, cx: &mut Context<Self>) {
        self.cancel_backdrop();
        self.backdrop_key = None;
        self.backdrop_image = None;
        self.backdrop_is_hidden = true;
        cx.notify();
    }

    fn set_backdrop(&mut self, image: Option<CleverImage>, cx: &mut Context<Self>) {
        self.backdrop_image = image;
        cx.notify();
    }

    /// the leaf draws glyph and switch itself, the native ones stand in until it mounts
    fn refresh_video_chrome(&mut self, cx: &mut Context<Self>) {
        let is_video = self.content.as_ref().is_some_and(|content| content.is_video);
        let mounted = self.video_leaf.read(cx).is_mounted();
        self.play_glyph_is_hidden = !is_video || mounted;
        self.sound_button_is_hidden = !(is_video && self.is_current && self.shows_video_sound_button && !mounted);
        cx.notify();
    }
}

impl Render for PostRowMediaPageView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (mut placeholder, shows_spinner, shows_failure) = self.surface_chrome();
        let installed = self.installed_image.clone().filter(|image| RenderImageAtlas::is_ready(image, window, cx));
        if installed.is_none() && self.installed_image.is_some() {
            placeholder = placeholder.or(self.palette.as_ref().map(|palette| palette.background300));
        }
        let fills = self.resolved_fills();
        let box_size = self.content.as_ref().map(|content| content.box_size);
        let frame_view = self.focus_frame_view.clone();
        let failure_tint = alpha(
            self.palette.as_ref().map_or(hsla(0.0, 0.0, 0.5, 1.0), |palette| palette.text_secondary),
            PostRowMediaMetrics::FAILURE_GLYPH_ALPHA,
        );

        let mut page = div()
            .relative()
            .size_full()
            .overflow_hidden()
            .when(self.is_slide_lifted, |page| page.opacity(0.0))
            .child(WindowScreenFrameReader::new(self.reader_id.clone()))
            .child(frame_view.probe())
            .when_some(placeholder, |page, color| page.child(div().absolute().inset_0().bg(color)));

        if !self.backdrop_is_hidden {
            if let (Some(image), Some(box_size)) = (&self.backdrop_image, box_size)
                && RenderImageAtlas::is_ready(image, window, cx)
            {
                let overscan = PostRowMediaMetrics::BLUR_OVERSCAN;
                let width = f32::from(box_size.width);
                let height = f32::from(box_size.height);
                page = page.child(
                    img(ImageSource::Render(image.clone()))
                        .absolute()
                        .left(px(width * (1.0 - overscan) / 2.0))
                        .top(px(height * (1.0 - overscan) / 2.0))
                        .w(px(width * overscan))
                        .h(px(height * overscan))
                        .object_fit(ObjectFit::Cover),
                );
            }
            page = page.child(div().absolute().inset_0().bg(alpha(black(), PostRowMediaMetrics::BLUR_DIM)));
        }

        if let Some(image) = installed {
            page = page.child(
                img(ImageSource::Render(image))
                    .absolute()
                    .inset_0()
                    .size_full()
                    .object_fit(if fills { ObjectFit::Cover } else { ObjectFit::Contain }),
            );
        }

        if shows_spinner {
            page = page.child(
                div().absolute().inset_0().flex().items_center().justify_center().child(
                    div().child(progress_view().scale_effect(PostRowMediaMetrics::LOADING_INDICATOR_SCALE)),
                ),
            );
        }
        if shows_failure {
            page = page.child(
                div().absolute().inset_0().flex().items_center().justify_center().children(IconUi::sized(
                    "photo",
                    px(Self::FAILURE_GLYPH_POINT_SIZE),
                    failure_tint,
                    false,
                )),
            );
        }
        if !self.play_glyph_is_hidden {
            page = page.child(div().absolute().inset_0().child(PostRowPlayGlyphView::new()));
        }
        if !self.sound_button_is_hidden
            && let Some(box_size) = box_size
        {
            let frame = Self::sound_button_frame(box_size);
            let on_sound_tap = self.on_sound_tap.clone();
            page = page.child(
                div()
                    .absolute()
                    .left(frame.origin.x)
                    .top(frame.origin.y)
                    .w(frame.size.width)
                    .h(frame.size.height)
                    .child(
                        PostRowSoundButton::new(SharedString::from(format!("post-row-page-sound-{}", cx.entity_id())))
                            .set_muted(self.is_muted)
                            .on_tap(move |cx| {
                                if let Some(on_sound_tap) = &on_sound_tap {
                                    on_sound_tap(cx);
                                }
                            }),
                    ),
            );
        }
        page.child(self.video_leaf.clone())
    }
}

/// horizontal paging scroller of a multi page gallery
/// wheel, trackpad and drag
/// not a scroll host so the guide scroller and the list never see it and neither moves a page
pub struct PostRowMediaPagerScrollView {
    pub is_mirrored: bool,
    content_offset: Animated<f32>,
    page_count: usize,
    wheel: WheelGestureTracker,
    claims_swipe: Option<bool>,
    coasting: bool,
    swipe_base: f32,
    wheel_settle_task: Option<Task<()>>,
    deceleration_task: Option<Task<()>>,
}

impl LeadingPageReporting for PostRowMediaPagerScrollView {
    fn rests_at_leading_page(&self) -> bool {
        self.is_at_first_page_edge()
    }
}

impl PostRowMediaPagerScrollView {
    const DRAG_MINIMUM_DISTANCE: f32 = 10.0;
    const SETTLE_DURATION: f64 = 0.24;
    const PAGE_SWIPE_THRESHOLD: f32 = 0.2;

    pub fn new() -> Self {
        Self {
            is_mirrored: false,
            content_offset: Animated::new(0.0),
            page_count: 0,
            wheel: WheelGestureTracker::new(),
            claims_swipe: None,
            coasting: false,
            swipe_base: 0.0,
            wheel_settle_task: None,
            deceleration_task: None,
        }
    }

    pub fn is_at_first_page_edge(&self) -> bool {
        let offset = *self.content_offset.target();
        if self.is_mirrored { offset >= self.last_slot() - 0.01 } else { offset <= 0.01 }
    }

    pub fn gesture_recognizer_should_begin(&self, translation: Point<Pixels>) -> bool {
        let dx = f32::from(translation.x);
        let dy = f32::from(translation.y);
        if dx.abs() <= dy.abs() {
            return false;
        }
        let back_sign = if self.is_mirrored { -1.0 } else { 1.0 };
        !(self.is_at_first_page_edge() && dx * back_sign > 0.0)
    }

    pub fn is_tracking(&self) -> bool {
        self.claims_swipe == Some(true) || self.wheel.is_tracking()
    }

    pub fn is_decelerating(&self) -> bool {
        self.deceleration_task.is_some()
    }

    fn last_slot(&self) -> f32 {
        self.page_count.saturating_sub(1) as f32
    }

    fn slot(&self, index: usize) -> usize {
        if self.is_mirrored { self.page_count.saturating_sub(1).saturating_sub(index) } else { index }
    }

    /// continuous page index under the viewport
    fn position(&self, offset: f32) -> f32 {
        if self.is_mirrored { self.last_slot() - offset } else { offset }
    }

    fn mount_window(&self, offset: f32) -> (usize, usize) {
        let position = self.position(offset);
        let last = self.page_count.saturating_sub(1);
        let low = (position.floor() as i64 - 1).max(0) as usize;
        let high = ((position.ceil() as i64 + 1).max(0) as usize).min(last);
        (low.min(high), high)
    }
}

impl Default for PostRowMediaPagerScrollView {
    fn default() -> Self {
        Self::new()
    }
}

/// holds the pages or the riff card, the accessibility element of the media
/// keyboard focus is the desktop path, arrows step and Enter or Space open
#[derive(Clone, Default)]
pub struct PostRowMediaBoxView {
    pub on_accessibility_increment: Option<PostRowMediaTap>,
    pub on_accessibility_decrement: Option<PostRowMediaTap>,
    pub on_accessibility_activate: Option<Rc<dyn Fn(&mut App) -> bool>>,
    pub accessibility_label: Option<String>,
    pub accessibility_value: Option<String>,
}

impl PostRowMediaBoxView {
    pub fn accessibility_increment(&self, cx: &mut App) {
        if let Some(on_accessibility_increment) = &self.on_accessibility_increment {
            on_accessibility_increment(cx);
        }
    }

    pub fn accessibility_decrement(&self, cx: &mut App) {
        if let Some(on_accessibility_decrement) = &self.on_accessibility_decrement {
            on_accessibility_decrement(cx);
        }
    }

    pub fn accessibility_activate(&self, cx: &mut App) -> bool {
        self.on_accessibility_activate.as_ref().is_some_and(|on_accessibility_activate| on_accessibility_activate(cx))
    }
}

struct PostRowViewerLiftHubEntry {
    state: WeakEntity<MediaViewerPresentationState>,
    observer: Rc<PostListMediaViewerLiftObserver>,
}

thread_local! {
    static POST_ROW_VIEWER_LIFT_HUB_ENTRIES: RefCell<HashMap<EntityId, PostRowViewerLiftHubEntry>> =
        RefCell::new(HashMap::new());
}

/// one lift observer per presentation state, shared by every native gallery
/// the main context carries no observer and the app has one root viewer state
pub enum PostRowViewerLiftHub {}

impl PostRowViewerLiftHub {
    pub fn observer(state: &Entity<MediaViewerPresentationState>, cx: &mut App) -> Rc<PostListMediaViewerLiftObserver> {
        let id = state.entity_id();
        let existing = POST_ROW_VIEWER_LIFT_HUB_ENTRIES.with_borrow(|entries| {
            entries.get(&id).filter(|entry| entry.state.upgrade().is_some()).map(|entry| entry.observer.clone())
        });
        if let Some(observer) = existing {
            return observer;
        }
        let stale: Vec<PostRowViewerLiftHubEntry> = POST_ROW_VIEWER_LIFT_HUB_ENTRIES.with_borrow_mut(|entries| {
            let keys: Vec<EntityId> = entries
                .iter()
                .filter(|(key, entry)| entry.state.upgrade().is_none() || **key == id)
                .map(|(key, _)| *key)
                .collect();
            keys.into_iter().filter_map(|key| entries.remove(&key)).collect()
        });
        for entry in stale {
            entry.observer.invalidate();
        }
        let observer = PostListMediaViewerLiftObserver::new(state, cx);
        POST_ROW_VIEWER_LIFT_HUB_ENTRIES.with_borrow_mut(|entries| {
            entries.insert(id, PostRowViewerLiftHubEntry { state: state.downgrade(), observer: observer.clone() });
        });
        observer
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PostRowMediaViewMode {
    None,
    Gallery,
    Riff,
}

/// everything a page or overlay frame depends on
#[derive(Clone, Debug, PartialEq)]
struct PostRowMediaViewSignature {
    key: PreparedPostRowKey,
    route: PostRowRoute,
    box_size: Size<Pixels>,
    box_aspect: f32,
    naturals: Vec<Option<f32>>,
    fills: Vec<bool>,
    is_provisional: bool,
}

/// native media box of a gallery or riff row
/// host_view is this view, the row registers anchor_view as the media anchor
/// this view only registers post_image records
pub struct PostRowMediaView {
    pub on_double_tap: Option<PostRowMediaDoubleTap>,

    weak_self: WeakEntity<Self>,
    row_view: EntityId,
    row_frame_view: Option<Rc<PostListFocusFrameView>>,
    frame_view: Rc<PostListFocusFrameView>,
    focus_handle: FocusHandle,
    box_view: PostRowMediaBoxView,
    pager: PostRowMediaPagerScrollView,
    pages: Vec<Entity<PostRowMediaPageView>>,
    spare_pages: Vec<Entity<PostRowMediaPageView>>,
    riff_card: Option<Entity<PostRowRiffCardView>>,
    plate_is_hidden: bool,
    bubbles_leaf: Entity<PostRowHostedLeafView<PostTagBubblesLayer>>,
    bubbles_alpha: Animated<f32>,
    page_indicator_is_hidden: bool,
    is_bold_text_enabled: bool,
    sound_button_is_hidden: bool,
    is_muted: bool,
    repost_leaf: Entity<PostRowHostedLeafView<PostRowRepostStickerLeaf>>,
    chip_is_hidden: bool,
    journey_chip_is_hidden: bool,
    ambient: Entity<PostRowAmbientMusicDriver>,
    anchor_is_lifted: bool,

    configured_presentation_id: Option<String>,
    mode: PostRowMediaViewMode,
    prepared: Option<Arc<PreparedPostRow>>,
    layout: Option<PostRowMediaBoxLayout>,
    signature: Option<PostRowMediaViewSignature>,
    router: Option<Rc<dyn PostRowActionRouting>>,
    focus: Option<Rc<dyn PostListFocusReporting>>,
    anchors: Option<Rc<dyn PostRowAnchorRegistering>>,
    hosted: Option<PostListHostedEnvironment>,
    session: u64,
    tags_state: Entity<PostMediaTagsState>,
    _tags_subscription: Subscription,
    leaf_post: Option<Rc<GetPostFeedResponse>>,
    current_page: usize,
    pager_in_use: bool,
    is_mirrored: bool,
    laid_out_pager_width: f32,
    leaves_active: bool,
    leaf_content_version: Option<i64>,
    is_visible_in_viewport: bool,
    tracked_focus_token: Option<PostListPlaybackToken>,
    tracked_focus_view: Option<Rc<PostListFocusFrameView>>,
    hero_target_observation: Option<PostListMediaViewerLiftObservation>,
    passthrough_id: Option<Uuid>,
    window_frame: Option<Bounds<Pixels>>,
    repost_sticker_size: Size<Pixels>,
    bubbles_generation: u64,
    bubbles_observed_generation: Option<u64>,
    bubbles_unmount_task: Option<Task<()>>,
    registered_anchors_id: Option<usize>,
    registered_slides_post_id: Option<String>,
    registered_slide_count: usize,
    has_pending_slide_registration: bool,
    is_hovered: bool,
    drag_did_move: bool,
    arrow_press_position: Option<Point<Pixels>>,
    pending_page_tap: Option<Task<()>>,
}

impl PostRowMediaView {
    const SPARE_PAGE_LIMIT: usize = 4;
    const DOUBLE_TAP_WINDOW: Duration = Duration::from_millis(250);
    const ARROW_SIZE: f32 = 28.0;
    const ARROW_GLYPH_SIZE: f32 = 14.0;
    const ARROW_INSET: f32 = 10.0;

    /// row_view is the mounted row entity the box draws in
    pub fn new(row_view: EntityId, cx: &mut Context<Self>) -> Self {
        let weak_self = cx.entity().downgrade();
        let tags_state = cx.new(PostMediaTagsState::new);
        let tags_subscription = cx.observe(&tags_state, |this, _, cx| this.bubbles_visibility_did_change(cx));

        let increment_weak = weak_self.clone();
        let decrement_weak = weak_self.clone();
        let activate_weak = weak_self.clone();
        let box_view = PostRowMediaBoxView {
            on_accessibility_increment: Some(Rc::new(move |cx: &mut App| {
                increment_weak.update(cx, |this, cx| this.accessibility_step(1, cx)).ok();
            })),
            on_accessibility_decrement: Some(Rc::new(move |cx: &mut App| {
                decrement_weak.update(cx, |this, cx| this.accessibility_step(-1, cx)).ok();
            })),
            on_accessibility_activate: Some(Rc::new(move |cx: &mut App| Self::accessibility_open(&activate_weak, cx))),
            accessibility_label: None,
            accessibility_value: None,
        };

        let bubbles_leaf: Entity<PostRowHostedLeafView<PostTagBubblesLayer>> = cx.new(PostRowHostedLeafView::new);
        bubbles_leaf.update(cx, |leaf, _| leaf.hit_policy = PostRowLeafHitPolicy::Everywhere);
        let repost_leaf: Entity<PostRowHostedLeafView<PostRowRepostStickerLeaf>> = cx.new(PostRowHostedLeafView::new);
        repost_leaf.update(cx, |leaf, _| leaf.hit_policy = PostRowLeafHitPolicy::Rects);

        Self {
            on_double_tap: None,
            weak_self,
            row_view,
            row_frame_view: None,
            frame_view: Rc::new(PostListFocusFrameView::new(row_view)),
            focus_handle: cx.focus_handle().tab_stop(true),
            box_view,
            pager: PostRowMediaPagerScrollView::new(),
            pages: Vec::new(),
            spare_pages: Vec::new(),
            riff_card: None,
            plate_is_hidden: true,
            bubbles_leaf,
            bubbles_alpha: Animated::new(1.0),
            page_indicator_is_hidden: true,
            is_bold_text_enabled: false,
            sound_button_is_hidden: true,
            is_muted: true,
            repost_leaf,
            chip_is_hidden: true,
            journey_chip_is_hidden: true,
            ambient: cx.new(PostRowAmbientMusicDriver::new),
            anchor_is_lifted: false,
            configured_presentation_id: None,
            mode: PostRowMediaViewMode::None,
            prepared: None,
            layout: None,
            signature: None,
            router: None,
            focus: None,
            anchors: None,
            hosted: None,
            session: 0,
            tags_state,
            _tags_subscription: tags_subscription,
            leaf_post: None,
            current_page: 0,
            pager_in_use: false,
            is_mirrored: false,
            laid_out_pager_width: 0.0,
            leaves_active: false,
            leaf_content_version: None,
            is_visible_in_viewport: true,
            tracked_focus_token: None,
            tracked_focus_view: None,
            hero_target_observation: None,
            passthrough_id: None,
            window_frame: None,
            repost_sticker_size: Size::default(),
            bubbles_generation: 0,
            bubbles_observed_generation: None,
            bubbles_unmount_task: None,
            registered_anchors_id: None,
            registered_slides_post_id: None,
            registered_slide_count: 0,
            has_pending_slide_registration: false,
            is_hovered: false,
            drag_did_move: false,
            arrow_press_position: None,
            pending_page_tap: None,
        }
    }

    pub fn configured_presentation_id(&self) -> Option<&str> {
        self.configured_presentation_id.as_deref()
    }

    pub fn set_row_frame_view(&mut self, row_frame_view: Rc<PostListFocusFrameView>) {
        self.row_frame_view = Some(row_frame_view);
    }

    pub fn set_anchor_lifted(&mut self, is_lifted: bool, cx: &mut Context<Self>) {
        if self.anchor_is_lifted != is_lifted {
            self.anchor_is_lifted = is_lifted;
            cx.notify();
        }
    }

    /// the Media anchor, pages or card without the overlays that stay visible during a lift
    /// none until the box was drawn once
    pub fn anchor_view(&self) -> Option<WindowScreenFrameReaderFrameView> {
        MediaCellScreenFrameRegistry::attached_view(&self.box_reader_id()?)
    }

    fn box_reader_id(&self) -> Option<String> {
        Some(format!("post-row-media-box-{}", self.configured_presentation_id.as_deref()?))
    }

    fn element_name(&self, suffix: &str) -> SharedString {
        SharedString::from(format!("post-row-media-{}-{suffix}", self.configured_presentation_id.as_deref().unwrap_or("none")))
    }

    pub fn configure(
        &mut self,
        prepared: &Arc<PreparedPostRow>,
        layout: &PostRowMediaBoxLayout,
        context: &PostRowMainContext,
        anchors: Rc<dyn PostRowAnchorRegistering>,
        cx: &mut Context<Self>,
    ) {
        PostRowMediaCaches::register_purge_if_needed(cx);
        let next_mode = if prepared.route == PostRowRoute::Native(PostRowNativeKind::RiffCard) {
            PostRowMediaViewMode::Riff
        } else {
            PostRowMediaViewMode::Gallery
        };
        let is_new_subject =
            self.configured_presentation_id.as_deref() != Some(prepared.presentation_id.as_str()) || self.mode != next_mode;
        if is_new_subject {
            self.reset_subject(cx);
        }
        self.configured_presentation_id = Some(prepared.presentation_id.clone());
        self.mode = next_mode;
        self.prepared = Some(prepared.clone());
        self.layout = Some(layout.clone());
        self.router = Some(context.router.clone());
        self.focus = Some(context.focus.clone());
        self.hosted = Some(context.hosted.clone());
        self.session = context.session;

        let mirrored = LayoutDirection::current() == LayoutDirection::RightToLeft;
        if mirrored != self.is_mirrored {
            self.is_mirrored = mirrored;
            self.pager.is_mirrored = mirrored;
            self.laid_out_pager_width = 0.0;
        }

        let next = PostRowMediaViewSignature {
            key: prepared.key.clone(),
            route: prepared.route,
            box_size: layout.frame.size,
            box_aspect: layout.box_aspect,
            naturals: layout.slides.iter().map(|slide| slide.natural_aspect).collect(),
            fills: layout.slides.iter().map(|slide| slide.fills).collect(),
            is_provisional: layout.is_aspect_provisional,
        };
        if self.signature.as_ref() != Some(&next) {
            self.signature = Some(next);
            self.leaf_content_version = None;
            self.leaf_post = None;
            self.apply_content(prepared, layout, context, cx);
            self.update_back_swipe_arbitration(cx);
        }

        self.register_slides(anchors, &prepared.id, cx);
        self.apply_chrome(context, cx);
        if is_new_subject {
            self.sync_tag_slide(cx);
            self.update_back_swipe_arbitration(cx);
        }
        if is_new_subject || (self.hero_target_observation.is_none() && self.pages.len() > 1) {
            self.observe_hero_target(&context.hosted.media_viewer_presentation, cx);
        }
        self.update_accessibility();
        if self.leaves_active {
            self.sync_leaves(cx);
        }
        cx.notify();
    }

    pub fn prepare_for_row_reuse(&mut self, cx: &mut Context<Self>) {
        self.leaves_active = false;
        self.reset_subject(cx);
        self.ambient.update(cx, |ambient, cx| ambient.stop(cx));
        let anchors = self.anchors.clone();
        for page in std::mem::take(&mut self.pages) {
            if let Some(anchors) = &anchors {
                if let Some(view) = page.read(cx).anchor_view() {
                    anchors.unregister(&view, cx);
                }
            }
            if self.spare_pages.len() < Self::SPARE_PAGE_LIMIT {
                self.spare_pages.push(page);
            }
        }
        self.pager_in_use = false;
        self.pager.page_count = 0;
        if let Some(card) = &self.riff_card {
            card.update(cx, |card, cx| card.set_hidden(true, cx));
        }
        self.configured_presentation_id = None;
        self.mode = PostRowMediaViewMode::None;
        self.prepared = None;
        self.layout = None;
        self.router = None;
        self.focus = None;
        self.hosted = None;
        self.anchors = None;
        self.plate_is_hidden = true;
        self.anchor_is_lifted = false;
        self.chip_is_hidden = true;
        self.journey_chip_is_hidden = true;
        self.sound_button_is_hidden = true;
        self.page_indicator_is_hidden = true;
        self.frame_view.clear_painted();
        cx.notify();
    }

    pub fn set_hosted_leaves_active(&mut self, is_active: bool, cx: &mut Context<Self>) {
        if self.leaves_active == is_active {
            if is_active {
                self.sync_leaves(cx);
            }
            return;
        }
        self.leaves_active = is_active;
        if is_active {
            self.sync_leaves(cx);
        } else {
            self.deactivate_leaves(cx);
        }
        cx.notify();
    }

    pub fn set_animated_media_visible_in_viewport(&mut self, is_visible: bool, cx: &mut Context<Self>) {
        if self.is_visible_in_viewport == is_visible {
            return;
        }
        self.is_visible_in_viewport = is_visible;
        for page in self.pages.clone() {
            page.update(cx, |page, cx| page.set_visible_in_viewport(is_visible, cx));
        }
        if let Some(card) = self.riff_card.clone() {
            card.update(cx, |card, cx| card.set_visible_in_viewport(is_visible, cx));
        }
        cx.notify();
    }

    fn reset_subject(&mut self, cx: &mut Context<Self>) {
        self.deactivate_leaves(cx);
        self.ambient.update(cx, |ambient, cx| ambient.stop(cx));
        self.hero_target_observation = None;
        self.unregister_passthrough();
        for page in self.pages.clone() {
            page.update(cx, |page, cx| page.prepare_for_reuse(cx));
        }
        if let Some(card) = self.riff_card.clone() {
            card.update(cx, |card, cx| card.prepare_for_reuse(cx));
        }
        self.tags_state = cx.new(PostMediaTagsState::new);
        self._tags_subscription = cx.observe(&self.tags_state, |this, _, cx| this.bubbles_visibility_did_change(cx));
        self.current_page = 0;
        self.signature = None;
        self.leaf_content_version = None;
        self.leaf_post = None;
        self.laid_out_pager_width = 0.0;
        self.pager.claims_swipe = None;
        self.pager.coasting = false;
        self.pager.wheel_settle_task = None;
        self.pager.deceleration_task = None;
        self.pager.wheel.reset();
        self.pending_page_tap = None;
        self.registered_anchors_id = None;
        self.registered_slides_post_id = None;
        self.registered_slide_count = 0;
    }

    fn apply_content(
        &mut self,
        prepared: &Arc<PreparedPostRow>,
        layout: &PostRowMediaBoxLayout,
        context: &PostRowMainContext,
        cx: &mut Context<Self>,
    ) {
        let seed = &prepared.seed;
        match self.mode {
            PostRowMediaViewMode::Riff => {
                self.release_pages(0, cx);
                self.pager_in_use = false;
                let card = self.ensure_riff_card(cx);
                card.update(cx, |card, cx| {
                    card.set_hidden(false, cx);
                    card.configure(prepared, layout, context, cx);
                });
            }
            PostRowMediaViewMode::Gallery => {
                if let Some(card) = self.riff_card.clone() {
                    card.update(cx, |card, cx| {
                        card.prepare_for_reuse(cx);
                        card.set_hidden(true, cx);
                    });
                }
                let slides = &seed.media.slides;
                let uses_pager = slides.len() > 1;
                if uses_pager != self.pager_in_use {
                    self.pager_in_use = uses_pager;
                    self.laid_out_pager_width = 0.0;
                }
                self.release_pages(slides.len(), cx);
                while self.pages.len() < slides.len() {
                    let page = match self.spare_pages.pop() {
                        Some(page) => page,
                        None => self.make_page(cx),
                    };
                    self.pages.push(page);
                }
                self.current_page = self.current_page.min(slides.len().saturating_sub(1));
                self.pager.page_count = slides.len();
                let shows_video_sound = self.video_sound_button_allowed();
                let is_visible = self.is_visible_in_viewport;
                let current_page = self.current_page;
                for (index, page) in self.pages.clone().into_iter().enumerate() {
                    let slide_layout = layout.slides.get(index);
                    let content = PostRowMediaPageViewContent {
                        poster_url: PostRowMediaPageView::trimmed_source(slides[index].poster_url()),
                        is_video: slides[index].is_video(),
                        natural_aspect: slide_layout.and_then(|slide| slide.natural_aspect),
                        layout_fills: slide_layout.is_none_or(|slide| slide.fills),
                        box_aspect: layout.box_aspect,
                        box_size: layout.frame.size,
                        is_own: seed.is_own,
                        account_user_id: context.account_user_id.clone(),
                        account_epoch: context.account_epoch.clone(),
                        memory_buckets: vec![PostFeedMediaPrefetchPolicy::slide_pixel_dimension(slides[index].is_video())],
                        stores_viewer_image: true,
                        draws_placeholder: true,
                    };
                    let palette = context.palette.clone();
                    let wants_image = index.abs_diff(current_page) <= 1;
                    page.update(cx, |page, cx| {
                        page.index = index;
                        page.set_wants_image(wants_image, cx);
                        page.configure(content, palette, cx);
                        page.set_visible_in_viewport(is_visible, cx);
                        page.set_current(index == current_page, shows_video_sound, cx);
                    });
                }
                self.pager.is_mirrored = self.is_mirrored;
            }
            PostRowMediaViewMode::None => {}
        }

        if self.mode == PostRowMediaViewMode::Gallery && layout.page_indicator.is_some() {
            self.page_indicator_is_hidden = false;
            self.is_bold_text_enabled = context.spec.is_bold_text_enabled;
        } else {
            self.page_indicator_is_hidden = true;
        }
        self.sound_button_is_hidden = layout.sound_button.is_none();
        self.plate_is_hidden = layout.music_plate.is_none() || seed.track.is_none();
        self.journey_chip_is_hidden = prepared.post.map_journey_id.is_none();
        self.chip_is_hidden = !seed.media.has_tagged_users;
        if !seed.counts.is_reposted && self.repost_leaf.read(cx).is_mounted() {
            self.unmount_repost_leaf(cx);
        }
    }

    fn make_page(&mut self, cx: &mut Context<Self>) -> Entity<PostRowMediaPageView> {
        let row_view = self.row_view;
        let weak_self = self.weak_self.clone();
        cx.new(move |cx| {
            let mut page = PostRowMediaPageView::new(row_view, cx);
            page.set_on_sound_tap(Some(Rc::new(move |cx: &mut App| Self::toggle_feed_sound(&weak_self, cx))));
            page
        })
    }

    fn release_pages(&mut self, keeping: usize, cx: &mut Context<Self>) {
        let anchors = self.anchors.clone();
        while self.pages.len() > keeping {
            let Some(page) = self.pages.pop() else {
                break;
            };
            if let Some(anchors) = &anchors {
                if let Some(view) = page.read(cx).anchor_view() {
                    anchors.unregister(&view, cx);
                }
            }
            page.update(cx, |page, cx| page.prepare_for_reuse(cx));
            if self.spare_pages.len() < Self::SPARE_PAGE_LIMIT {
                self.spare_pages.push(page);
            }
        }
        if self.registered_slide_count > self.pages.len() {
            self.registered_slide_count = self.pages.len();
        }
        self.pager.page_count = self.pages.len();
    }

    fn ensure_riff_card(&mut self, cx: &mut Context<Self>) -> Entity<PostRowRiffCardView> {
        if let Some(card) = &self.riff_card {
            return card.clone();
        }
        let row_view = self.row_view;
        let open_weak = self.weak_self.clone();
        let sound_weak = self.weak_self.clone();
        let is_visible = self.is_visible_in_viewport;
        let card = cx.new(move |cx| {
            let mut card = PostRowRiffCardView::new(row_view, cx);
            card.on_open = Some(Rc::new(move |cx: &mut App| Self::open_riff(&open_weak, cx)));
            card.on_toggle_sound = Some(Rc::new(move |cx: &mut App| Self::toggle_feed_sound(&sound_weak, cx)));
            card.set_visible_in_viewport(is_visible, cx);
            card
        });
        self.riff_card = Some(card.clone());
        card
    }

    fn register_slides(&mut self, anchors: Rc<dyn PostRowAnchorRegistering>, post_id: &str, cx: &mut Context<Self>) {
        let count = if self.mode == PostRowMediaViewMode::Gallery { self.pages.len() } else { 0 };
        let id = Rc::as_ptr(&anchors) as *const () as usize;
        self.anchors = Some(anchors.clone());
        if self.registered_anchors_id == Some(id)
            && self.registered_slides_post_id.as_deref() == Some(post_id)
            && self.registered_slide_count == count
        {
            return;
        }
        self.registered_anchors_id = Some(id);
        self.registered_slides_post_id = Some(post_id.to_owned());
        self.registered_slide_count = count;
        let weak_self = self.weak_self.clone();
        anchors.set_on_slide_lift_change(Some(Rc::new(move |index: i64, is_hidden: bool, cx: &mut App| {
            let weak_self = weak_self.clone();
            cx.defer(move |cx| {
                weak_self.update(cx, |this, cx| this.set_slide_lifted(index, is_hidden, cx)).ok();
            });
        })));
        self.register_drawn_slides(cx);
    }

    /// registers slides of the drawn pages, safe to repeat
    /// and runs again after a frame for pages not drawn yet
    fn register_drawn_slides(&mut self, cx: &mut Context<Self>) {
        self.has_pending_slide_registration = false;
        let (Some(anchors), Some(post_id)) = (self.anchors.clone(), self.registered_slides_post_id.clone()) else {
            return;
        };
        if self.mode != PostRowMediaViewMode::Gallery {
            return;
        }
        for page in self.pages.clone() {
            let (view, index) = {
                let page = page.read(cx);
                (page.anchor_view(), page.index)
            };
            match view {
                Some(view) => anchors.register_slide(&view, &post_id, index as i64, cx),
                None => self.has_pending_slide_registration = true,
            }
        }
    }

    fn set_slide_lifted(&mut self, index: i64, is_hidden: bool, cx: &mut Context<Self>) {
        for page in &self.pages {
            page.update(cx, |page, cx| {
                if page.index as i64 == index {
                    page.set_slide_lifted(is_hidden, cx);
                }
            });
        }
    }

    fn apply_chrome(&mut self, context: &PostRowMainContext, cx: &mut Context<Self>) {
        self.set_sound_buttons_muted(context.feed_audio_muted, cx);
    }

    fn video_sound_button_allowed(&self) -> bool {
        self.prepared.as_ref().is_some_and(|prepared| prepared.seed.track.is_none() && !prepared.post.shows_upload_overlay())
    }

    fn current_slide_is_video(&self) -> bool {
        if self.mode != PostRowMediaViewMode::Gallery {
            return false;
        }
        self.prepared
            .as_ref()
            .and_then(|prepared| prepared.seed.media.slides.get(self.current_page))
            .is_some_and(|slide| slide.is_video())
    }

    /// syncs the tag slide, a photos image index or -1 for the video page
    fn sync_tag_slide(&mut self, cx: &mut Context<Self>) {
        if self.mode != PostRowMediaViewMode::Gallery {
            return;
        }
        let Some(slide) = self.prepared.as_ref().and_then(|prepared| prepared.seed.media.slides.get(self.current_page)) else {
            return;
        };
        let value = slide.image_index().unwrap_or(-1);
        self.tags_state.update(cx, |state, cx| state.set_slide(value, cx));
    }

    fn scroll_view_did_scroll(&mut self, cx: &mut Context<Self>) {
        if self.mode != PostRowMediaViewMode::Gallery || self.pages.len() <= 1 {
            return;
        }
        let offset = *self.pager.content_offset.target();
        let mut index = (offset.round().max(0.0) as usize).min(self.pages.len() - 1);
        if self.is_mirrored {
            index = self.pages.len() - 1 - index;
        }
        if index != self.current_page {
            self.page_did_change(index, cx);
        }
    }

    fn page_did_change(&mut self, index: usize, cx: &mut Context<Self>) {
        let previous = self.current_page;
        self.current_page = index;
        if let Some(prepared) = self.prepared.as_ref() {
            PostMediaFocus::set(&prepared.id, index, cx);
        }
        let shows_video_sound = self.video_sound_button_allowed();
        if let Some(page) = self.pages.get(previous).cloned() {
            page.update(cx, |page, cx| page.set_current(false, shows_video_sound, cx));
        }
        if let Some(page) = self.pages.get(index).cloned() {
            page.update(cx, |page, cx| page.set_current(true, shows_video_sound, cx));
        }
        for (position, page) in self.pages.clone().into_iter().enumerate() {
            page.update(cx, |page, cx| page.set_wants_image(position.abs_diff(index) <= 1, cx));
        }
        self.sync_tag_slide(cx);
        self.update_back_swipe_arbitration(cx);
        let is_video = self.current_slide_is_video();
        self.ambient.update(cx, |ambient, cx| ambient.set_suppressed(is_video, cx));
        if self.leaves_active {
            self.update_playback_focus(cx);
        }
        self.update_accessibility();
        cx.notify();
    }

    fn pager_did_settle(&mut self, cx: &mut Context<Self>) {
        if !self.leaves_active || self.mode != PostRowMediaViewMode::Gallery {
            return;
        }
        let (Some(prepared), Some(hosted)) = (self.prepared.clone(), self.hosted.clone()) else {
            return;
        };
        self.sync_video_leaves(&prepared, &hosted, false, cx);
    }

    fn scroll_view_did_end_decelerating(&mut self, cx: &mut Context<Self>) {
        self.pager_did_settle(cx);
    }

    fn scroll_to_page(&mut self, index: usize, animated: bool, cx: &mut Context<Self>) {
        if self.mode != PostRowMediaViewMode::Gallery || index >= self.pages.len() || index == self.current_page {
            return;
        }
        let width = self.layout.as_ref().map_or(0.0, |layout| f32::from(layout.frame.size.width));
        if !self.pager_in_use || width <= 1.0 {
            self.page_did_change(index, cx);
            self.laid_out_pager_width = 0.0;
            cx.notify();
            return;
        }
        let slot = self.pager.slot(index) as f32;
        if animated {
            self.settle_pager(slot, cx);
        } else {
            self.pager.content_offset.jump(slot);
            self.scroll_view_did_scroll(cx);
            self.pager_did_settle(cx);
            cx.notify();
        }
    }

    fn layout_pager(&mut self, width: f32) {
        if (self.laid_out_pager_width - width).abs() > 0.5 {
            self.laid_out_pager_width = width;
            self.pager.content_offset.jump(self.pager.slot(self.current_page) as f32);
        }
    }

    fn settle_pager(&mut self, target_slot: f32, cx: &mut Context<Self>) {
        if self.pager.page_count == 0 {
            return;
        }
        let target_slot = target_slot.round().clamp(0.0, self.pager.last_slot());
        self.pager.content_offset.set_in(
            target_slot,
            ease_out(PostRowMediaPagerScrollView::SETTLE_DURATION),
            RiseClock::monotonic(),
            cx,
        );
        self.scroll_view_did_scroll(cx);
        self.schedule_deceleration_end(cx);
        cx.notify();
    }

    fn schedule_deceleration_end(&mut self, cx: &mut Context<Self>) {
        self.pager.deceleration_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f64(PostRowMediaPagerScrollView::SETTLE_DURATION))
                .await;
            this.update(cx, |this, cx| {
                this.pager.deceleration_task = None;
                this.scroll_view_did_end_decelerating(cx);
            })
            .ok();
        }));
    }

    fn handle_scroll_wheel(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pager_in_use {
            return;
        }
        let event = Self::horizontal_from_shift(event);
        let feed = self.pager.wheel.feed(&event, window.line_height());
        let claimed = self.apply_pager_step(feed.step, cx);
        if claimed {
            cx.stop_propagation();
        }
        if let Some(token) = feed.settle {
            self.pager.wheel_settle_task = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(WHEEL_SETTLE_DELAY).await;
                this.update(cx, |this, cx| {
                    if let Some(step) = this.pager.wheel.settle(token) {
                        this.apply_pager_step(step, cx);
                    }
                })
                .ok();
            }));
        }
    }

    fn horizontal_from_shift(event: &ScrollWheelEvent) -> ScrollWheelEvent {
        let mut event = event.clone();
        if event.modifiers.shift {
            event.delta = match event.delta {
                ScrollDelta::Pixels(delta) if f32::from(delta.x) == 0.0 => ScrollDelta::Pixels(point(delta.y, delta.x)),
                ScrollDelta::Lines(delta) if delta.x == 0.0 => ScrollDelta::Lines(point(delta.y, delta.x)),
                other => other,
            };
        }
        event
    }

    /// returns true when the step belongs to this pager and not to whatever lies under it
    fn apply_pager_step(&mut self, step: WheelStep, cx: &mut Context<Self>) -> bool {
        let page_width = self.layout.as_ref().map_or(1.0, |layout| f32::from(layout.frame.size.width).max(1.0));
        match step {
            WheelStep::Ignored => self.pager.coasting,
            WheelStep::Moved(translation) => {
                self.pager.coasting = false;
                if translation == Point::default() {
                    return false;
                }
                let claims = match self.pager.claims_swipe {
                    Some(claims) => claims,
                    None => {
                        let claims = self.pager.gesture_recognizer_should_begin(translation);
                        self.pager.claims_swipe = Some(claims);
                        self.pager.swipe_base = *self.pager.content_offset.target();
                        claims
                    }
                };
                if claims {
                    let moved = f32::from(translation.x) / page_width;
                    let offset = (self.pager.swipe_base - moved).clamp(0.0, self.pager.last_slot());
                    self.pager.deceleration_task = None;
                    self.pager.content_offset.jump(offset);
                    self.scroll_view_did_scroll(cx);
                    cx.notify();
                }
                claims
            }
            WheelStep::Ended(translation) => {
                let claimed = self.pager.claims_swipe.take() == Some(true);
                if claimed {
                    self.pager.coasting = true;
                    let moved = -f32::from(translation.x) / page_width;
                    let threshold = PostRowMediaPagerScrollView::PAGE_SWIPE_THRESHOLD;
                    let base = self.pager.swipe_base;
                    let target = if moved > threshold {
                        base + 1.0
                    } else if moved < -threshold {
                        base - 1.0
                    } else {
                        base
                    };
                    self.settle_pager(target, cx);
                }
                claimed
            }
            WheelStep::Cancelled => {
                let claimed = self.pager.claims_swipe.take() == Some(true);
                if claimed {
                    let base = self.pager.swipe_base;
                    self.settle_pager(base, cx);
                }
                claimed
            }
        }
    }

    fn observe_hero_target(&mut self, state: &Entity<MediaViewerPresentationState>, cx: &mut Context<Self>) {
        self.hero_target_observation = None;
        if self.mode != PostRowMediaViewMode::Gallery || self.pages.len() <= 1 {
            return;
        }
        let weak_self = self.weak_self.clone();
        let observer = PostRowViewerLiftHub::observer(state, cx);
        self.hero_target_observation = Some(observer.observe_hero_target(move |hero_lift_id, cx| {
            weak_self.update(cx, |this, cx| this.follow_hero_target(hero_lift_id, cx)).ok();
        }));
    }

    fn follow_hero_target(&mut self, hero_lift_id: &str, cx: &mut Context<Self>) {
        if self.mode != PostRowMediaViewMode::Gallery || self.pages.len() <= 1 {
            return;
        }
        let Some(prepared) = self.prepared.clone() else {
            return;
        };
        let target = (0..self.pages.len()).find(|index| MediaViewerFrameKeys::post_image(&prepared.id, *index) == hero_lift_id);
        if let Some(index) = target
            && index != self.current_page
        {
            self.scroll_to_page(index, false, cx);
        }
    }

    /// off page 0 the box claims the back drag, on page 0
    /// the pager declines it so the full screen pop wins
    fn update_back_swipe_arbitration(&mut self, cx: &mut Context<Self>) {
        let off_first_page = self.mode == PostRowMediaViewMode::Gallery && self.pager_in_use && self.current_page > 0;
        if off_first_page && self.window_frame.is_some() {
            self.register_passthrough(cx);
        } else {
            self.unregister_passthrough();
        }
    }

    fn register_passthrough(&mut self, cx: &mut Context<Self>) {
        let Some(frame) = self.window_frame else {
            return;
        };
        if let Some(id) = self.passthrough_id {
            BubbleScrollPassthrough::with_shared(|passthrough| passthrough.update(id, frame));
        } else {
            let target = AnyView::from(cx.entity()).downgrade();
            self.passthrough_id = Some(BubbleScrollPassthrough::with_shared(|passthrough| passthrough.register(frame, Some(target))));
        }
    }

    fn unregister_passthrough(&mut self) {
        if let Some(id) = self.passthrough_id.take() {
            BubbleScrollPassthrough::with_shared(|passthrough| passthrough.unregister(id));
        }
    }

    fn window_frame_did_change(&mut self, frame: Bounds<Pixels>, cx: &mut Context<Self>) {
        self.window_frame = Some(frame);
        self.update_back_swipe_arbitration(cx);
    }

    fn resolved_leaf_post(&mut self, prepared: &PreparedPostRow) -> Rc<GetPostFeedResponse> {
        self.leaf_post.get_or_insert_with(|| Rc::new(prepared.post.clone())).clone()
    }

    fn sync_leaves(&mut self, cx: &mut Context<Self>) {
        if !self.leaves_active {
            return;
        }
        let (Some(prepared), Some(hosted)) = (self.prepared.clone(), self.hosted.clone()) else {
            return;
        };
        let version_changed = self.leaf_content_version != Some(prepared.content_version);
        if version_changed {
            self.leaf_post = None;
        }
        self.leaf_content_version = Some(prepared.content_version);
        self.update_playback_focus(cx);
        match self.mode {
            PostRowMediaViewMode::Gallery => self.sync_video_leaves(&prepared, &hosted, version_changed, cx),
            PostRowMediaViewMode::Riff => self.sync_riff_leaf(&prepared, &hosted, version_changed, cx),
            PostRowMediaViewMode::None => {}
        }
        self.sync_repost_leaf(&prepared, &hosted, version_changed, cx);
        if version_changed && self.bubbles_leaf.read(cx).is_mounted() {
            let post = self.resolved_leaf_post(&prepared);
            let layer = PostTagBubblesLayer::new(post, self.tags_state.clone());
            self.bubbles_leaf.update(cx, |leaf, cx| leaf.update(layer, &hosted, cx));
        }
        self.sync_ambient(&prepared, cx);
        cx.notify();
    }

    fn deactivate_leaves(&mut self, cx: &mut Context<Self>) {
        if let (Some(token), Some(focus)) = (&self.tracked_focus_token, &self.focus) {
            focus.end_focus_tracking(token, cx);
        }
        self.tracked_focus_token = None;
        self.tracked_focus_view = None;
        for page in self.pages.clone() {
            page.update(cx, |page, cx| page.unmount_video_leaf(cx));
        }
        if let Some(card) = self.riff_card.clone() {
            card.update(cx, |card, cx| card.unmount_leaf(cx));
        }
        self.unmount_repost_leaf(cx);
        self.hide_bubbles_now(cx);
        self.ambient.update(cx, |ambient, cx| ambient.set_active(false, cx));
        self.leaf_content_version = None;
    }

    /// one focus token, the current video page or the riff card
    fn update_playback_focus(&mut self, cx: &mut Context<Self>) {
        let desired = self.desired_playback_focus(cx);
        let is_same = match (&desired, &self.tracked_focus_token, &self.tracked_focus_view) {
            (None, None, _) => true,
            (Some((token, view)), Some(tracked), Some(tracked_view)) => token == tracked && Rc::ptr_eq(view, tracked_view),
            _ => false,
        };
        if is_same {
            return;
        }
        if let (Some(token), Some(focus)) = (&self.tracked_focus_token, &self.focus) {
            focus.end_focus_tracking(token, cx);
        }
        self.tracked_focus_token = None;
        self.tracked_focus_view = None;
        let (Some((token, view)), Some(focus)) = (desired, self.focus.clone()) else {
            return;
        };
        focus.begin_focus_tracking(&token, &view, cx);
        self.tracked_focus_token = Some(token);
        self.tracked_focus_view = Some(view);
    }

    fn desired_playback_focus(&self, cx: &App) -> Option<(PostListPlaybackToken, Rc<PostListFocusFrameView>)> {
        if !self.leaves_active {
            return None;
        }
        let prepared = self.prepared.as_ref()?;
        match self.mode {
            PostRowMediaViewMode::Riff => {
                let card = self.riff_card.as_ref()?;
                if prepared.post.resolved_video_url().is_none_or(|url| url.is_empty()) {
                    return None;
                }
                Some((
                    PostListPlaybackToken {
                        session: self.session,
                        presentation_id: prepared.presentation_id.clone(),
                        kind: PostListPlaybackTokenKind::RiffCard,
                    },
                    card.read(cx).focus_frame_view(cx),
                ))
            }
            PostRowMediaViewMode::Gallery => {
                let slide = prepared.seed.media.slides.get(self.current_page)?;
                if !slide.is_video() {
                    return None;
                }
                let page = self.pages.get(self.current_page)?;
                Some((
                    PostListPlaybackToken {
                        session: self.session,
                        presentation_id: prepared.presentation_id.clone(),
                        kind: PostListPlaybackTokenKind::VideoPage { slide_index: self.current_page },
                    },
                    page.read(cx).focus_frame_view().clone(),
                ))
            }
            PostRowMediaViewMode::None => None,
        }
    }

    /// the current video page mounts its player leaf once
    /// the pager is at rest, other pages drop theirs
    fn sync_video_leaves(
        &mut self,
        prepared: &Arc<PreparedPostRow>,
        environment: &PostListHostedEnvironment,
        force_update: bool,
        cx: &mut Context<Self>,
    ) {
        let is_settled = !self.pager.is_tracking() && !self.pager.is_decelerating();
        let shows_sound = self.video_sound_button_allowed();
        for page in self.pages.clone() {
            let index = page.read(cx).index;
            let mut leaf = None;
            if self.leaves_active
                && index == self.current_page
                && !prepared.post.shows_upload_overlay()
                && let Some(PostMediaSlide::Video { url, thumbnail, .. }) = prepared.seed.media.slides.get(index)
                && let Some(token) = self.tracked_focus_token.clone()
                && token.kind == (PostListPlaybackTokenKind::VideoPage { slide_index: index })
            {
                let fills = page.read(cx).resolved_fills();
                leaf = Some(PostRowInlineVideoLeaf::new(
                    SharedString::from(format!("post-row-inline-video-{}", token.raw_value())),
                    self.resolved_leaf_post(prepared),
                    url.clone(),
                    thumbnail.clone(),
                    fills,
                    token.raw_value(),
                    shows_sound,
                ));
            }
            let is_mounted = page.read(cx).video_leaf.read(cx).is_mounted();
            let leaves_active = self.leaves_active;
            page.update(cx, |page, cx| match leaf {
                Some(leaf) => {
                    if !is_mounted {
                        if is_settled {
                            page.mount_video_leaf(leaf, shows_sound, environment, cx);
                        }
                    } else if force_update {
                        page.update_video_leaf(leaf, environment, cx);
                    }
                }
                None => {
                    if is_mounted && (!leaves_active || is_settled) {
                        page.unmount_video_leaf(cx);
                    }
                }
            });
        }
    }

    fn sync_riff_leaf(
        &mut self,
        prepared: &Arc<PreparedPostRow>,
        environment: &PostListHostedEnvironment,
        force_update: bool,
        cx: &mut Context<Self>,
    ) {
        let Some(card) = self.riff_card.clone() else {
            return;
        };
        let token = self
            .tracked_focus_token
            .clone()
            .filter(|token| self.leaves_active && token.kind == PostListPlaybackTokenKind::RiffCard);
        let Some(token) = token else {
            card.update(cx, |card, cx| card.unmount_leaf(cx));
            return;
        };
        let weak_self = self.weak_self.clone();
        let leaf = PostRowRiffCardLeaf::new(
            SharedString::from(format!("post-row-riff-leaf-{}", token.raw_value())),
            self.resolved_leaf_post(prepared),
            token.raw_value(),
            card.read(cx).fills(cx),
            move |cx: &mut App| Self::open_riff(&weak_self, cx),
        );
        card.update(cx, |card, cx| {
            if card.is_leaf_mounted(cx) {
                if force_update {
                    card.update_leaf(leaf, environment, cx);
                }
            } else {
                card.mount_leaf(leaf, environment, cx);
            }
        });
    }

    fn sync_repost_leaf(
        &mut self,
        prepared: &Arc<PreparedPostRow>,
        environment: &PostListHostedEnvironment,
        force_update: bool,
        cx: &mut Context<Self>,
    ) {
        if !self.leaves_active || !prepared.seed.counts.is_reposted {
            self.unmount_repost_leaf(cx);
            return;
        }
        let weak_self = self.weak_self.clone();
        let leaf = PostRowRepostStickerLeaf::new(
            SharedString::from(format!("post-row-repost-leaf-{}", prepared.presentation_id)),
            self.resolved_leaf_post(prepared),
            move |size: Size<Pixels>, cx: &mut App| {
                weak_self.update(cx, |this, cx| this.repost_sticker_size_did_change(size, cx)).ok();
            },
        );
        self.repost_leaf.update(cx, |view, cx| {
            if view.is_mounted() {
                if force_update {
                    view.update(leaf, environment, cx);
                }
            } else {
                view.mount(leaf, environment, cx);
            }
        });
    }

    fn unmount_repost_leaf(&mut self, cx: &mut Context<Self>) {
        self.repost_sticker_size = Size::default();
        self.repost_leaf.update(cx, |view, cx| {
            view.unmount(cx);
            view.interactive_rects = Vec::new();
        });
    }

    fn repost_sticker_size_did_change(&mut self, size: Size<Pixels>, cx: &mut Context<Self>) {
        if size == self.repost_sticker_size {
            return;
        }
        self.repost_sticker_size = size;
        tracing::warn!(target: "riseonly::X574", "repost sticker size {:?}", size);
        self.update_repost_hit_rect(cx);
        cx.notify();
    }

    fn update_repost_hit_rect(&mut self, cx: &mut Context<Self>) {
        let sticker = self.repost_sticker_size;
        let is_measured = f32::from(sticker.width) > 0.0 && f32::from(sticker.height) > 0.0;
        let height = self.layout.as_ref().map(|layout| layout.corner_row_origin.y.max(px(0.0)));
        self.repost_leaf.update(cx, |view, _| {
            view.interactive_rects = match height.filter(|_| view.is_mounted() && is_measured) {
                Some(height) => vec![Bounds {
                    origin: point(px(0.0), (height - sticker.height).max(px(0.0))),
                    size: size(sticker.width, sticker.height.min(height)),
                }],
                None => Vec::new(),
            };
        });
    }

    fn sync_ambient(&mut self, prepared: &Arc<PreparedPostRow>, cx: &mut Context<Self>) {
        let seed = &prepared.seed;
        let focus = self.focus.clone().filter(|_| {
            self.mode == PostRowMediaViewMode::Gallery && seed.body == PostRowBodyKind::Standard && seed.has_ambient_music
        });
        let Some(focus) = focus else {
            self.ambient.update(cx, |ambient, cx| ambient.stop(cx));
            return;
        };
        let token = PostListPlaybackToken {
            session: self.session,
            presentation_id: seed.presentation_id.clone(),
            kind: PostListPlaybackTokenKind::AmbientMusic,
        };
        let frame_view = self.row_frame_view.clone().unwrap_or_else(|| self.frame_view.clone());
        let is_video = self.current_slide_is_video();
        let is_active = self.leaves_active;
        self.ambient.update(cx, |ambient, cx| {
            ambient.configure(&prepared.post, token, focus, frame_view, cx);
            ambient.set_suppressed(is_video, cx);
            ambient.set_active(is_active, cx);
        });
    }

    fn chip_tapped(this: &WeakEntity<Self>, cx: &mut App) {
        let Some(view) = this.upgrade() else {
            return;
        };
        let (prepared, router, tags_state) = {
            let view = view.read(cx);
            (view.prepared.clone(), view.router.clone(), view.tags_state.clone())
        };
        let Some(prepared) = prepared else {
            return;
        };
        if let Some(router) = &router {
            router.selection_haptic(cx);
        }
        let (slide, shows_tags) = {
            let state = tags_state.read(cx);
            (state.slide, state.shows_tags)
        };
        if prepared.seed.media.tagged_slide_indices.contains(&slide) {
            if shows_tags {
                tags_state.update(cx, |state, cx| state.toggle(cx));
                view.update(cx, |view, cx| view.schedule_bubbles_unmount(cx));
            } else {
                view.update(cx, |view, cx| view.show_bubbles(&prepared, cx));
                tags_state.update(cx, |state, cx| state.toggle(cx));
            }
        } else {
            view.update(cx, |view, cx| view.hide_bubbles_animated(cx));
            if let Some(router) = &router {
                router.open_tagged_people(&prepared.id, cx);
            }
        }
    }

    fn chip_long_pressed(this: &WeakEntity<Self>, cx: &mut App) {
        let Some(view) = this.upgrade() else {
            return;
        };
        let (prepared, router) = {
            let view = view.read(cx);
            (view.prepared.clone(), view.router.clone())
        };
        let Some(prepared) = prepared else {
            return;
        };
        view.update(cx, |view, cx| view.hide_bubbles_animated(cx));
        if let Some(router) = router {
            router.open_tagged_people(&prepared.id, cx);
        }
    }

    fn journey_chip_tapped(this: &WeakEntity<Self>, cx: &mut App) {
        let Some(view) = this.upgrade() else {
            return;
        };
        let (prepared, hosted) = {
            let view = view.read(cx);
            (view.prepared.clone(), view.hosted.clone())
        };
        let (Some(prepared), Some(hosted)) = (prepared, hosted) else {
            return;
        };
        if prepared.post.map_journey_id.is_none() {
            return;
        }
        let Ok(post_id) = prepared.post.id.parse::<i64>() else {
            return;
        };
        hosted.navigation_store.update(cx, |store, cx| {
            store.push(RootRoute::MapJourney { post_id }, NavigationStoreSettlingPush::Drop, cx);
        });
    }

    fn show_bubbles(&mut self, prepared: &Arc<PreparedPostRow>, cx: &mut Context<Self>) {
        let Some(hosted) = self.hosted.clone() else {
            return;
        };
        self.bubbles_generation = self.bubbles_generation.wrapping_add(1);
        self.bubbles_unmount_task = None;
        if self.bubbles_leaf.read(cx).is_mounted() {
            self.bubbles_alpha.jump(1.0);
        } else {
            let post = self.resolved_leaf_post(prepared);
            let layer = PostTagBubblesLayer::new(post, self.tags_state.clone());
            self.bubbles_leaf.update(cx, |leaf, cx| leaf.mount(layer, &hosted, cx));
            self.bubbles_alpha.jump(0.0);
            self.bubbles_alpha.set_in(1.0, ease_in_out(PostRowMediaMetrics::TAGS_FADE), RiseClock::monotonic(), cx);
        }
        self.bubbles_observed_generation = Some(self.bubbles_generation);
        cx.notify();
    }

    fn bubbles_visibility_did_change(&mut self, cx: &mut Context<Self>) {
        if self.bubbles_observed_generation != Some(self.bubbles_generation) || !self.bubbles_leaf.read(cx).is_mounted() {
            return;
        }
        if !self.tags_state.read(cx).shows_tags {
            self.schedule_bubbles_unmount(cx);
        }
    }

    fn hide_bubbles_animated(&mut self, cx: &mut Context<Self>) {
        let shows_tags = self.tags_state.read(cx).shows_tags;
        if !shows_tags && !self.bubbles_leaf.read(cx).is_mounted() {
            return;
        }
        self.tags_state.update(cx, |state, cx| state.hide(cx));
        self.schedule_bubbles_unmount(cx);
    }

    /// unmounts after the fade out unless the tags came back meanwhile
    fn schedule_bubbles_unmount(&mut self, cx: &mut Context<Self>) {
        self.bubbles_generation = self.bubbles_generation.wrapping_add(1);
        let generation = self.bubbles_generation;
        self.bubbles_unmount_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(PostRowMediaMetrics::TAGS_UNMOUNT_DELAY)).await;
            this.update(cx, |this, cx| {
                if generation != this.bubbles_generation || this.tags_state.read(cx).shows_tags {
                    return;
                }
                this.bubbles_leaf.update(cx, |leaf, cx| leaf.unmount(cx));
                this.bubbles_alpha.jump(1.0);
                cx.notify();
            })
            .ok();
        }));
    }

    fn hide_bubbles_now(&mut self, cx: &mut Context<Self>) {
        self.bubbles_generation = self.bubbles_generation.wrapping_add(1);
        self.bubbles_unmount_task = None;
        if self.tags_state.read(cx).shows_tags {
            self.tags_state.update(cx, |state, cx| state.hide(cx));
        }
        self.bubbles_leaf.update(cx, |leaf, cx| leaf.unmount(cx));
        self.bubbles_alpha.jump(1.0);
    }

    fn toggle_feed_sound(this: &WeakEntity<Self>, cx: &mut App) {
        let Some(view) = this.upgrade() else {
            return;
        };
        let router = view.read(cx).router.clone();
        if let Some(router) = router {
            router.toggle_feed_sound(cx);
        }
        let muted = ShortsServicesStore::shared(cx).read(cx).feed_audio_muted;
        view.update(cx, |view, cx| view.set_sound_buttons_muted(muted, cx));
    }

    fn set_sound_buttons_muted(&mut self, muted: bool, cx: &mut Context<Self>) {
        self.is_muted = muted;
        for page in self.pages.clone() {
            page.update(cx, |page, cx| page.set_muted(muted, cx));
        }
        if let Some(card) = self.riff_card.clone() {
            card.update(cx, |card, cx| card.set_muted(muted, cx));
        }
        cx.notify();
    }

    fn open_riff(this: &WeakEntity<Self>, cx: &mut App) {
        let Some(view) = this.upgrade() else {
            return;
        };
        let (prepared, router) = {
            let view = view.read(cx);
            (view.prepared.clone(), view.router.clone())
        };
        if let (Some(prepared), Some(router)) = (prepared, router) {
            router.open_riff(&prepared.id, cx);
        }
    }

    fn handle_box_click(this: &WeakEntity<Self>, event: &ClickEvent, cx: &mut App) {
        if event.is_keyboard() {
            return;
        }
        let Some(view) = this.upgrade() else {
            return;
        };
        let arrow_press = view.update(cx, |view, _| view.arrow_press_position.take());
        if let ClickEvent::Mouse(click) = event {
            if arrow_press == Some(click.down.position) {
                tracing::warn!(target: "riseonly::X602", "box click from arrow press skipped");
                return;
            }
            let arrow_delta = view.read(cx).arrow_delta_at(click.down.position);
            if let Some(delta) = arrow_delta {
                tracing::warn!(target: "riseonly::X564", "box click over arrow steps {delta}");
                view.update(cx, |view, cx| view.step_page(delta, true, cx));
                return;
            }
        }
        let position = event.position();
        let click_count = event.click_count();
        let (receives, on_double_tap) = {
            let view = view.read(cx);
            (view.mode == PostRowMediaViewMode::Gallery && !view.drag_did_move && view.gesture_recognizer(position, cx), view.on_double_tap.clone())
        };
        if !receives {
            return;
        }
        match (click_count, on_double_tap) {
            (2, Some(on_double_tap)) => {
                view.update(cx, |view, _| view.pending_page_tap = None);
                on_double_tap(cx);
            }
            (1, Some(_)) => {
                let weak_view = this.clone();
                view.update(cx, |view, cx| {
                    view.pending_page_tap = Some(cx.spawn(async move |this, cx| {
                        cx.background_executor().timer(Self::DOUBLE_TAP_WINDOW).await;
                        this.update(cx, |this, cx| {
                            this.pending_page_tap = None;
                            cx.defer(move |cx| Self::handle_page_tap(&weak_view, position, cx));
                        })
                        .ok();
                    }));
                });
            }
            (1, None) => Self::handle_page_tap(this, position, cx),
            _ => {}
        }
    }

    fn link_zones_target(&self, cx: &App) -> Option<PostRowLinkZonesViewTarget> {
        if self.mode != PostRowMediaViewMode::Gallery {
            return None;
        }
        let prepared = self.prepared.as_ref()?;
        if prepared.seed.media.link_areas.is_empty() {
            return None;
        }
        let page = self.pages.get(self.current_page)?.read(cx);
        let frame_size = self.layout.as_ref()?.frame.size;
        if frame_size.width <= px(0.0) || frame_size.height <= px(0.0) {
            return None;
        }
        let zones = MediaLinkZone::zones_post(&prepared.seed.media.link_areas, page.index as i64);
        if zones.is_empty() {
            return None;
        }
        Some(PostRowLinkZonesViewTarget {
            zones,
            frame: Bounds::new(Point::default(), frame_size),
            media_aspect: page.content().and_then(|content| content.natural_aspect).or(page.decoded_aspect()),
            media_fills: page.resolved_fills(),
        })
    }

    fn handle_page_tap(this: &WeakEntity<Self>, position: Point<Pixels>, cx: &mut App) {
        let Some(view) = this.upgrade() else {
            return;
        };
        let (prepared, router, plate, frame, tapped, source) = {
            let view = view.read(cx);
            (
                view.prepared.clone(),
                view.router.clone(),
                view.layout.as_ref().and_then(|layout| layout.music_plate.clone()).filter(|_| !view.plate_is_hidden),
                view.window_frame,
                view.current_page,
                view.pages.get(view.current_page).and_then(|page| page.read(cx).media_page_source()),
            )
        };
        let (Some(prepared), Some(router)) = (prepared, router) else {
            return;
        };
        if let (Some(plate), Some(frame), Some(track)) = (&plate, frame, &prepared.seed.track)
            && PostRowMusicPlateView::contains_plate(track, plate, position - frame.origin)
        {
            router.open_audio_page(&prepared.id, cx);
            return;
        }
        router.open_media_viewer(&prepared.id, tapped, source, cx);
    }

    fn gesture_recognizer(&self, position: Point<Pixels>, cx: &App) -> bool {
        let Some(frame) = self.window_frame else {
            return true;
        };
        let Some(layout) = &self.layout else {
            return true;
        };
        if self.bubbles_leaf.read(cx).is_mounted() {
            return false;
        }
        let local = position - frame.origin;
        if !self.sound_button_is_hidden && layout.sound_button.is_some_and(|button| button.contains(&local)) {
            return false;
        }
        if self.corner_row_rects(layout).iter().any(|rect| rect.contains(&local)) {
            return false;
        }
        let repost_local = point(local.x - layout.corner_row_origin.x, local.y);
        if self.repost_leaf.read(cx).takes_touch(repost_local) {
            return false;
        }
        if let Some(page) = self.pages.get(self.current_page) {
            let page = page.read(cx);
            let sound_frame = PostRowMediaPageView::sound_button_frame(layout.frame.size);
            let page_owns_sound = !page.sound_button_is_hidden() || page.video_leaf.read(cx).takes_touch(local);
            if page_owns_sound && sound_frame.contains(&local) {
                return false;
            }
        }
        true
    }

    /// chip frames of the corner row, sticker is bubble_width wide then the spacing
    fn corner_row_rects(&self, layout: &PostRowMediaBoxLayout) -> Vec<Bounds<Pixels>> {
        let origin = layout.corner_row_origin;
        let side = PostRowMediaMetrics::CHIP_SIZE;
        let spacing = PostRowMediaMetrics::CORNER_ROW_SPACING;
        let is_reposted = self.prepared.as_ref().is_some_and(|prepared| prepared.seed.counts.is_reposted);
        let chip_x = f32::from(origin.x) + if is_reposted { RepostStickerStyle::Feed.bubble_width() + spacing } else { 0.0 };
        let top = origin.y - px(side);
        let mut rects = Vec::with_capacity(2);
        if !self.journey_chip_is_hidden {
            rects.push(Bounds { origin: point(px(chip_x), top), size: size(px(side), px(side)) });
        }
        if !self.chip_is_hidden {
            let tags_x = chip_x + if self.journey_chip_is_hidden { 0.0 } else { side + spacing };
            rects.push(Bounds { origin: point(px(tags_x), top), size: size(px(side), px(side)) });
        }
        rects
    }

    fn update_accessibility(&mut self) {
        let Some(prepared) = self.prepared.clone() else {
            self.box_view.accessibility_label = None;
            self.box_view.accessibility_value = None;
            return;
        };
        match self.mode {
            PostRowMediaViewMode::Riff => {
                self.box_view.accessibility_label = Some(tr("chat_inline_type_video"));
                self.box_view.accessibility_value = None;
            }
            PostRowMediaViewMode::Gallery => {
                let slides = &prepared.seed.media.slides;
                let is_video = slides.get(self.current_page).is_some_and(|slide| slide.is_video());
                self.box_view.accessibility_label =
                    Some(tr(if is_video { "chat_inline_type_video" } else { "chat_inline_type_photo" }));
                self.box_view.accessibility_value =
                    (slides.len() > 1).then(|| format!("{}/{}", self.current_page + 1, slides.len()));
            }
            PostRowMediaViewMode::None => {
                self.box_view.accessibility_label = None;
                self.box_view.accessibility_value = None;
            }
        }
    }

    fn accessibility_step(&mut self, delta: i64, cx: &mut Context<Self>) {
        self.step_page(delta, false, cx);
    }

    fn step_page(&mut self, delta: i64, animated: bool, cx: &mut Context<Self>) {
        if self.mode != PostRowMediaViewMode::Gallery || self.pages.len() <= 1 {
            return;
        }
        let target = (self.current_page as i64 + delta).clamp(0, self.pages.len() as i64 - 1) as usize;
        if target != self.current_page {
            self.scroll_to_page(target, animated, cx);
        }
    }

    fn accessibility_open(this: &WeakEntity<Self>, cx: &mut App) -> bool {
        let Some(view) = this.upgrade() else {
            return false;
        };
        let (prepared, router, mode, tapped, page_count, source) = {
            let view = view.read(cx);
            (
                view.prepared.clone(),
                view.router.clone(),
                view.mode,
                view.current_page,
                view.pages.len(),
                view.pages.get(view.current_page).and_then(|page| page.read(cx).media_page_source()),
            )
        };
        let (Some(prepared), Some(router)) = (prepared, router) else {
            return false;
        };
        match mode {
            PostRowMediaViewMode::Riff => {
                router.open_riff(&prepared.id, cx);
                true
            }
            PostRowMediaViewMode::Gallery => {
                if tapped >= page_count {
                    return false;
                }
                router.open_media_viewer(&prepared.id, tapped, source, cx);
                true
            }
            PostRowMediaViewMode::None => false,
        }
    }

    fn handle_key_down(this: &WeakEntity<Self>, event: &KeyDownEvent, cx: &mut App) {
        if event.keystroke.modifiers.modified() {
            return;
        }
        let Some(view) = this.upgrade() else {
            return;
        };
        let (box_view, is_mirrored) = {
            let view = view.read(cx);
            (view.box_view.clone(), view.is_mirrored)
        };
        let (next_key, previous_key) = if is_mirrored { ("left", "right") } else { ("right", "left") };
        let key = event.keystroke.key.as_str();
        if key == next_key {
            box_view.accessibility_increment(cx);
        } else if key == previous_key {
            box_view.accessibility_decrement(cx);
        } else if key == "enter" || key == "space" {
            box_view.accessibility_activate(cx);
        } else {
            return;
        }
        cx.stop_propagation();
    }

    fn arrow_delta_at(&self, position: Point<Pixels>) -> Option<i64> {
        if !self.is_hovered || !self.pager_in_use {
            return None;
        }
        let frame = self.window_frame?;
        let local = position - frame.origin;
        let page_count = self.pages.len();
        let (leading_delta, trailing_delta) = if self.is_mirrored { (1, -1) } else { (-1, 1) };
        let shows_leading = if self.is_mirrored { self.current_page + 1 < page_count } else { self.current_page > 0 };
        let shows_trailing = if self.is_mirrored { self.current_page > 0 } else { self.current_page + 1 < page_count };
        let side = px(Self::ARROW_SIZE);
        let inset = px(Self::ARROW_INSET);
        let top = (frame.size.height - side) / 2.0;
        let leading = Bounds { origin: point(inset, top), size: size(side, side) };
        let trailing = Bounds { origin: point(frame.size.width - inset - side, top), size: size(side, side) };
        if shows_leading && leading.contains(&local) {
            return Some(leading_delta);
        }
        if shows_trailing && trailing.contains(&local) {
            return Some(trailing_delta);
        }
        None
    }

    fn arrow(&self, name: &str, glyph: &str, delta: i64) -> impl IntoElement + use<> {
        let weak_self = self.weak_self.clone();
        let press_weak = self.weak_self.clone();
        div()
            .on_mouse_down(MouseButton::Left, move |event: &MouseDownEvent, _, cx| {
                let position = event.position;
                press_weak.update(cx, |this, _| this.arrow_press_position = Some(position)).ok();
                cx.stop_propagation();
            })
            .child(
                plain(self.element_name(name))
                    .on_press(move |_, cx| {
                        weak_self.update(cx, |this, cx| this.step_page(delta, true, cx)).ok();
                    })
                    .child(
                        div()
                            .size(px(Self::ARROW_SIZE))
                            .flex()
                            .items_center()
                            .justify_center()
                            .children(IconUi::sized(glyph, px(Self::ARROW_GLYPH_SIZE), white(), false))
                            .liquid_chrome_circle(None),
                    ),
            )
    }

    fn pager_element(&mut self, frame_size: Size<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let width = f32::from(frame_size.width).max(1.0);
        self.layout_pager(width);
        let offset = self.pager.content_offset.frame_value(RiseClock::monotonic(), window);
        let (low, high) = self.pager.mount_window(offset);
        let mut pager = div().relative().size_full().overflow_hidden();
        for (index, page) in self.pages.iter().enumerate() {
            if index < low || index > high {
                page.read(cx).focus_frame_view().clear_painted();
                continue;
            }
            let left = (self.pager.slot(index) as f32 - offset) * width;
            let mut holder = div().absolute().top_0().left(px(left)).w(frame_size.width).h(frame_size.height);
            if index == 0 {
                holder = holder.child(div().absolute().inset_0().child(PagerBackSwipeYield::new(
                    self.element_name("back-swipe-yield"),
                    Some(self.current_page == 0),
                )));
            }
            pager = pager.child(holder.child(page.clone()));
        }
        pager.into_any_element()
    }
}

impl Focusable for PostRowMediaView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Drop for PostRowMediaView {
    fn drop(&mut self) {
        self.unregister_passthrough();
    }
}

impl Render for PostRowMediaView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.has_pending_slide_registration {
            let view = cx.weak_entity();
            window.on_next_frame(move |_, cx| {
                let _ = view.update(cx, |view, cx| view.register_drawn_slides(cx));
            });
        }
        let Some(layout) = self.layout.clone() else {
            return div().into_any_element();
        };
        let frame_size = layout.frame.size;
        let weak_self = self.weak_self.clone();

        let mut box_view = div()
            .id(self.element_name("box"))
            .relative()
            .w(frame_size.width)
            .h(frame_size.height)
            .overflow_hidden()
            .track_focus(&self.focus_handle)
            .when(self.anchor_is_lifted, |box_view| box_view.opacity(0.0));

        match self.mode {
            PostRowMediaViewMode::Gallery if self.pager_in_use => {
                box_view = box_view.child(self.pager_element(frame_size, window, cx));
            }
            PostRowMediaViewMode::Gallery => {
                for page in &self.pages {
                    box_view = box_view.child(div().absolute().inset_0().child(page.clone()));
                }
            }
            PostRowMediaViewMode::Riff => {
                if let Some(card) = &self.riff_card {
                    box_view = box_view.child(div().absolute().inset_0().child(card.clone()));
                }
            }
            PostRowMediaViewMode::None => {}
        }
        if let Some(reader_id) = self.box_reader_id() {
            box_view = box_view.child(WindowScreenFrameReader::new(reader_id));
        }
        if !self.page_indicator_is_hidden
            && let Some(frame) = layout.page_indicator
        {
            box_view = box_view.child(
                div()
                    .absolute()
                    .left(frame.origin.x)
                    .top(frame.origin.y)
                    .w(frame.size.width)
                    .h(frame.size.height)
                    .child(PostRowPageIndicatorView::new().configure(
                        self.pages.len() as i64,
                        self.current_page as i64,
                        self.is_bold_text_enabled,
                    )),
            );
        }

        let down_weak = weak_self.clone();
        let click_weak = weak_self.clone();
        let wheel_weak = weak_self.clone();
        let key_weak = weak_self.clone();
        let wheel_capture = self.pager_in_use.then(|| {
            canvas(
                |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                move |_, hitbox: Hitbox, window, _| {
                    window.on_mouse_event(move |event: &ScrollWheelEvent, phase, window, cx| {
                        if phase != DispatchPhase::Capture || !hitbox.should_handle_scroll(window) {
                            return;
                        }
                        wheel_weak.update(cx, |this, cx| this.handle_scroll_wheel(event, window, cx)).ok();
                    });
                },
            )
            .absolute()
            .inset_0()
        });
        let box_view = box_view
            .children(wheel_capture)
            .on_mouse_down(MouseButton::Left, move |_, _, cx| {
                down_weak.update(cx, |this, _| this.drag_did_move = false).ok();
            })
            .on_click(move |event: &ClickEvent, _, cx| Self::handle_box_click(&click_weak, event, cx))
            .on_key_down(move |event: &KeyDownEvent, _, cx| Self::handle_key_down(&key_weak, event, cx));

        let box_view = if self.pager_in_use {
            let changed_weak = weak_self.clone();
            let ended_weak = weak_self.clone();
            box_view
                .gesture(
                    ElementId::Name(self.element_name("drag")),
                    DragGesture::new(PostRowMediaPagerScrollView::DRAG_MINIMUM_DISTANCE)
                        .on_changed(move |value: &DragGestureValue, _, cx| {
                            changed_weak
                                .update(cx, |this, cx| {
                                    let claimed = this.apply_pager_step(
                                        WheelStep::Moved(point(value.translation.width, value.translation.height)),
                                        cx,
                                    );
                                    this.drag_did_move = this.drag_did_move || claimed;
                                })
                                .ok();
                        })
                        .on_ended(move |value: &DragGestureValue, _, cx| {
                            ended_weak
                                .update(cx, |this, cx| {
                                    this.apply_pager_step(
                                        WheelStep::Ended(point(
                                            value.predicted_end_translation.width,
                                            value.predicted_end_translation.height,
                                        )),
                                        cx,
                                    );
                                })
                                .ok();
                        }),
                )
                .into_any_element()
        } else {
            box_view.into_any_element()
        };

        let frame_view = self.frame_view.clone();
        let hover_weak = weak_self.clone();
        let measure_weak = weak_self;
        let mut root = div()
            .id(self.element_name("root"))
            .relative()
            .w(frame_size.width)
            .h(frame_size.height)
            .overflow_hidden()
            .on_mouse_move({
                let move_weak = hover_weak.clone();
                move |_: &MouseMoveEvent, _, cx| {
                    move_weak
                        .update(cx, |this, cx| {
                            if !this.is_hovered {
                                this.is_hovered = true;
                                cx.notify();
                            }
                        })
                        .ok();
                }
            })
            .on_hover(move |hovered: &bool, _, cx| {
                let hovered = *hovered;
                hover_weak
                    .update(cx, |this, cx| {
                        this.is_hovered = hovered;
                        cx.notify();
                    })
                    .ok();
            })
            .child(frame_view.probe())
            .child(measure(self.element_name("measure"), move |bounds: Bounds<Pixels>, _, cx| {
                measure_weak.update(cx, |this, cx| this.window_frame_did_change(bounds, cx)).ok();
            }))
            .child(box_view);

        if !self.plate_is_hidden
            && let (Some(plate), Some(track)) =
                (&layout.music_plate, self.prepared.as_ref().and_then(|prepared| prepared.seed.track.as_ref()))
        {
            root = root.child(
                div().absolute().inset_0().child(
                    PostRowMusicPlateView::new(self.element_name("plate"))
                        .configure(track, plate)
                        .set_visible_in_viewport(self.is_visible_in_viewport),
                ),
            );
        }
        {
            let target_weak = self.weak_self.clone();
            let open_weak = self.weak_self.clone();
            root = root.child(
                div().absolute().inset_0().child(
                    PostRowLinkZonesView::new(self.element_name("link-zones"))
                        .target(Rc::new(move |cx: &App| {
                            target_weak.upgrade().and_then(|view| view.read(cx).link_zones_target(cx))
                        }))
                        .on_open(Rc::new(move |zone: &MediaLinkZone, _: &mut Window, cx: &mut App| {
                            let Some(post_id) = open_weak
                                .upgrade()
                                .and_then(|view| view.read(cx).prepared.as_ref().map(|prepared| prepared.id.clone()))
                            else {
                                return;
                            };
                            let Some(url) = Url::from_string(&zone.url) else {
                                return;
                            };
                            PostInteractionsStore::shared(cx).update(cx, |store, cx| store.record_link_open(&post_id, cx));
                            InAppLinkBrowser::route(&url, cx);
                        })),
                ),
            );
        }
        if self.bubbles_leaf.read(cx).is_mounted() {
            let opacity = self.bubbles_alpha.frame_value(RiseClock::monotonic(), window);
            root = root.child(div().absolute().inset_0().opacity(opacity).child(self.bubbles_leaf.clone()));
        }
        if !self.sound_button_is_hidden
            && let Some(frame) = layout.sound_button
        {
            root = root.child(
                div()
                    .absolute()
                    .left(frame.origin.x)
                    .top(frame.origin.y)
                    .w(frame.size.width)
                    .h(frame.size.height)
                    .child(PostRowSoundButton::new(self.element_name("sound")).set_muted(self.is_muted).on_tap({
                        let weak_self = self.weak_self.clone();
                        move |cx| Self::toggle_feed_sound(&weak_self, cx)
                    })),
            );
        }

        let origin = layout.corner_row_origin;
        if self.repost_leaf.read(cx).is_mounted() {
            root = root.child(
                div()
                    .absolute()
                    .left(origin.x)
                    .top((origin.y - self.repost_sticker_size.height).max(px(0.0)))
                    .w((frame_size.width - origin.x).max(px(0.0)))
                    .h(self.repost_sticker_size.height)
                    .child(self.repost_leaf.clone()),
            );
        }
        let chips = self.corner_row_rects(&layout);
        let mut chip_rects = chips.into_iter();
        if !self.journey_chip_is_hidden
            && let Some(frame) = chip_rects.next()
        {
            root = root.child(
                div()
                    .absolute()
                    .left(frame.origin.x)
                    .top(frame.origin.y)
                    .w(frame.size.width)
                    .h(frame.size.height)
                    .child(
                        PostRowTagsChipView::new(self.element_name("journey-chip"))
                            .set_symbol("map.fill")
                            .accessibility_label(tr("map_journey_open"))
                            .on_tap({
                                let weak_self = self.weak_self.clone();
                                move |cx| Self::journey_chip_tapped(&weak_self, cx)
                            }),
                    ),
            );
        }
        if !self.chip_is_hidden
            && let Some(frame) = chip_rects.next()
        {
            root = root.child(
                div()
                    .absolute()
                    .left(frame.origin.x)
                    .top(frame.origin.y)
                    .w(frame.size.width)
                    .h(frame.size.height)
                    .child(
                        PostRowTagsChipView::new(self.element_name("tags-chip"))
                            .on_tap({
                                let weak_self = self.weak_self.clone();
                                move |cx| Self::chip_tapped(&weak_self, cx)
                            })
                            .on_long_press({
                                let weak_self = self.weak_self.clone();
                                move |cx| Self::chip_long_pressed(&weak_self, cx)
                            }),
                    ),
            );
        }

        if self.is_hovered && self.pager_in_use {
            let page_count = self.pages.len();
            let inset = px(Self::ARROW_INSET);
            let (leading_delta, trailing_delta) = if self.is_mirrored { (1, -1) } else { (-1, 1) };
            let shows_leading = if self.is_mirrored { self.current_page + 1 < page_count } else { self.current_page > 0 };
            let shows_trailing = if self.is_mirrored { self.current_page > 0 } else { self.current_page + 1 < page_count };
            if shows_leading {
                root = root.child(
                    div().absolute().left(inset).top_0().bottom_0().flex().items_center().child(self.arrow(
                        "previous",
                        "chevron.left",
                        leading_delta,
                    )),
                );
            }
            if shows_trailing {
                root = root.child(
                    div().absolute().right(inset).top_0().bottom_0().flex().items_center().child(self.arrow(
                        "next",
                        "chevron.right",
                        trailing_delta,
                    )),
                );
            }
        }

        root.into_any_element()
    }
}

impl PostRowMediaHosting for Entity<PostRowMediaView> {
    fn host_view(&self) -> AnyView {
        AnyView::from(self.clone())
    }

    fn anchor_view(&self, cx: &App) -> Option<WindowScreenFrameReaderFrameView> {
        self.read(cx).anchor_view()
    }

    fn set_anchor_lifted(&self, is_lifted: bool, cx: &mut App) {
        self.update(cx, |view, cx| view.set_anchor_lifted(is_lifted, cx));
    }

    fn set_row_frame_view(&self, row_frame_view: Rc<PostListFocusFrameView>, cx: &mut App) {
        self.update(cx, |view, _| view.set_row_frame_view(row_frame_view));
    }

    fn set_on_double_tap(&self, on_double_tap: Option<PostRowMediaDoubleTap>, cx: &mut App) {
        self.update(cx, |view, _| view.on_double_tap = on_double_tap);
    }

    fn configure(
        &self,
        prepared: &Arc<PreparedPostRow>,
        layout: &PostRowMediaBoxLayout,
        context: &PostRowMainContext,
        anchors: Rc<dyn PostRowAnchorRegistering>,
        cx: &mut App,
    ) {
        self.update(cx, |view, cx| view.configure(prepared, layout, context, anchors, cx));
    }

    fn prepare_for_row_reuse(&self, cx: &mut App) {
        self.update(cx, |view, cx| view.prepare_for_row_reuse(cx));
    }

    fn set_hosted_leaves_active(&self, is_active: bool, cx: &mut App) {
        self.update(cx, |view, cx| view.set_hosted_leaves_active(is_active, cx));
    }

    fn set_animated_media_visible_in_viewport(&self, is_visible: bool, cx: &mut App) {
        self.update(cx, |view, cx| view.set_animated_media_visible_in_viewport(is_visible, cx));
    }
}

