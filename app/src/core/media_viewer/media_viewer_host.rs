use std::cell::{Cell, RefCell};
use std::io::Cursor;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    AnyElement, AnyView, App, AppContext, Bounds, Context, ElementId, Entity, FocusHandle, Focusable, FontWeight, Global,
    InteractiveElement, IntoElement, KeyBinding, MouseMoveEvent, ObjectFit, ParentElement, Pixels, Render, RenderOnce,
    Keystroke, ScrollWheelEvent, SharedString, Size, StatefulInteractiveElement, Styled, StyleRefinement, StyledImage,
    Subscription, Task, TextRun,
    WeakEntity, Window,
    prelude::FluentBuilder, actions, black, canvas, div, img, linear_color_stop, linear_gradient, point, px, size, white,
};
use rise_core::foundation::foundation_calendar::{Calendar, CalendarComponent};
use rise_core::foundation::foundation_date::{Date, RiseClock, TimeInterval};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::date_format::date_formatter::DateFormatter;
use rise_i18n::{Locale, tr, tr_plural};
use rise_platform::file_dialog::{self, FileFilter, FilterLabel, SaveOutcome, SaveRequest};
use rise_platform::reveal_in_file_manager::{self, ExecutableConsent};
use rise_platform::haptics::{NotificationKind, notification};
use rise_platform::pasteboard::UIPasteboard;
use rise_platform::paths::downloads_directory;
use rise_platform::wheel_gesture::{WHEEL_SETTLE_DELAY, WheelGestureTracker, WheelStep};
use rise_theme::ThemeStore;
use rise_ui::IconUi;
use rise_ui::window_cover::WindowCoverFrame;
use rise_ui::frame_request::{opaque_cover_layer, release_opaque_cover};
use rise_ui::context_menu_ui::ContextMenuItem;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::message_bubble_geometry::MessageBubbleRadii;
use rise_ui::message_media_tail_presentation::{MessageMediaTailCapShape, MessageMediaTailGeometry};
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::page_header_toolbar_button::{PageHeaderMetrics, PageHeaderToolbarSlot};
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::context_menu::ContextMenuModifiers;
use rise_ui::swiftui::drag_gesture::{DragGesture, DragGestureModifiers, DragGestureValue};
use rise_ui::swiftui::environment::{LayoutDirection, env_scope};
use rise_ui::swiftui::environment_values::{ColorScheme, ColorSchemeKey};
use rise_ui::swiftui::font::{Font, FontDesign, SwiftUiFont};
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::minimum_scale_factor::visual_line_count;
use rise_ui::swiftui::motion::{MotionSpec, ease_in_out, ease_out, spring};
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::window_screen_frame_reader::MediaCellScreenFrameRegistry;
use rise_widgets::overlay::blocking_loader_store::BlockingLoaderStore;

use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media::rise_media_transfer_hub::MediaViewerVideoCacheKey;
use crate::core::media_viewer::media_save_to_library::{MediaSaveToLibrary, MediaSaveToLibraryResult};
use crate::core::media_viewer::media_viewer_countdown_badge::MediaViewerCountdownBadge;
use crate::core::media_viewer::media_viewer_filmstrip::MediaViewerFilmstrip;
use crate::core::media_viewer::media_viewer_image_cache::{MediaViewerBitmapLoader, MediaViewerImageMemoryCache};
use crate::core::media_viewer::media_viewer_presentation_state::{
    MediaViewerBoundAction, MediaViewerGalleryScope, MediaViewerPresentationRoute, MediaViewerPresentationState,
};
use crate::core::media_viewer::media_viewer_types::{
    MediaViewerDeleteMode, MediaViewerFilmstripMetrics, MediaViewerGalleryItem, MediaViewerGalleryItemKind,
    MediaViewerHeroSkin, MediaViewerTransition, PostDownloadRef,
};
use crate::core::media_viewer::media_viewer_video_player::{
    MediaViewerVideoController, MediaViewerVideoControllerPlaybackStatus, MediaViewerVideoPlayer,
};
use crate::core::media_viewer::media_viewer_video_scrubber::MediaViewerVideoScrubber;
use crate::core::media_viewer::media_zoomable_image_view::{MediaZoomViewBuilder, MediaZoomablePage};
use crate::core::preferences::media_viewer_playback_preferences::MediaViewerPlaybackPreferences;
use crate::core::ui::animated_gif_view::{AnimatedGifContentMode, AnimatedGifView};
use crate::core::ui::animated_media_playback_coordinator::AnimatedMediaPlaybackKind;
use crate::core::ui::clever_image_disk_cache::{
    CleverImage, CleverImageBitmapEncoder, CleverImageDiskCache, CleverImageRenderMemoryCache,
};
use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::core::widgets::navigations::bottom_chrome_insets::BottomChromeInsetsModifiers;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierSendOptions, NotifierStore, NotifierType,
};
use crate::modules::chat::components::chat::bubble::media_letterbox_photo::{MediaAspectFit, MediaLetterboxPhoto};
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::post::components::download::post_download_store::PostDownloadSheetStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;
use rise_ui::swiftui::content_mode::ContentMode;

pub const KEY_CONTEXT: &str = "MediaViewerHost";

actions!(media_viewer_host, [CloseViewer, PreviousPage, NextPage, SaveMedia, CopyMedia, ToggleFullScreen]);

struct MediaViewerHostKeyBindingsInstalled;

impl Global for MediaViewerHostKeyBindingsInstalled {}

/// safe to call twice, every render of the host already calls it
pub fn install_key_bindings(cx: &mut App) {
    if cx.has_global::<MediaViewerHostKeyBindingsInstalled>() {
        return;
    }
    cx.set_global(MediaViewerHostKeyBindingsInstalled);
    let context = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("escape", CloseViewer, context),
        KeyBinding::new("left", PreviousPage, context),
        KeyBinding::new("right", NextPage, context),
        KeyBinding::new("secondary-s", SaveMedia, context),
        KeyBinding::new("secondary-c", CopyMedia, context),
        KeyBinding::new("f11", ToggleFullScreen, context),
        KeyBinding::new("ctrl-cmd-f", ToggleFullScreen, context),
    ]);
}

/// draws only the presentations routed to it so one state never plays a video twice
/// on desktop the viewer is a layer over the window
#[derive(IntoElement)]
pub struct MediaViewerHost {
    state: Entity<MediaViewerPresentationState>,
    route: MediaViewerPresentationRoute,
}

impl MediaViewerHost {
    pub fn new(state: Entity<MediaViewerPresentationState>, route: MediaViewerPresentationRoute) -> Self {
        Self { state, route }
    }

    pub fn inline(state: Entity<MediaViewerPresentationState>) -> Self {
        Self::new(state, MediaViewerPresentationRoute::Inline)
    }

    /// an escape reached the window root while the viewer is up and focus is outside it
    /// viewer takes focus and gets the key after this dispatch
    /// strayed_only is the capture phase so menus over the viewer hear escape first
    pub fn redirect_orphan_escape(keystroke: &Keystroke, strayed_only: bool, window: &mut Window, cx: &mut App) -> bool {
        Self::redirect_orphan_key(keystroke, strayed_only, window, cx)
    }

    pub fn redirect_orphan_arrow(keystroke: &Keystroke, window: &mut Window, cx: &mut App) -> bool {
        const ARROW_KEYS: [&str; 2] = ["left", "right"];
        if keystroke.modifiers.modified() || !ARROW_KEYS.contains(&keystroke.key.as_str()) {
            return false;
        }
        Self::redirect_orphan_key(keystroke, false, window, cx)
    }

    fn redirect_orphan_key(keystroke: &Keystroke, strayed_only: bool, window: &mut Window, cx: &mut App) -> bool {
        let Some(screen) = cx.try_global::<MediaViewerKeysOwner>().and_then(|owner| owner.0.upgrade()) else {
            return false;
        };
        let (handle, taken_at, interactive) = {
            let s = screen.read(cx);
            (s.focus_handle.clone(), s.keys_taken_at, s.is_interactive(cx))
        };
        if !interactive || !handle.is_in_rendered_frame(window) || handle.contains_focused(window, cx) {
            return false;
        }
        if MediaViewerScreen::sheet_above(taken_at, window, cx) {
            return false;
        }
        let strayed = MediaViewerScreen::focus_strayed_below(window, cx);
        if strayed_only && !strayed {
            return false;
        }
        tracing::warn!(
            target: "riseonly::X1357",
            strayed,
            strayed_only,
            focused = ?window.focused(cx),
            key = %keystroke.key,
            "orphan key back to the viewer"
        );
        window.focus(&handle, cx);
        let redispatched = keystroke.clone();
        window.defer(cx, move |window, cx| {
            window.dispatch_keystroke(redispatched, cx);
        });
        true
    }
}

/// the inline host as its own view, drawn by the root as a cached host
pub struct MediaViewerInlineHostView {
    state: Entity<MediaViewerPresentationState>,
}

impl MediaViewerInlineHostView {
    pub fn new(state: Entity<MediaViewerPresentationState>) -> Self {
        Self { state }
    }
}

impl Render for MediaViewerInlineHostView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(MediaViewerHost::inline(self.state.clone()))
    }
}

thread_local! {
    static VIEWER_HOLDS_OPAQUE_COVER: Cell<bool> = const { Cell::new(false) };
    static VIEWER_LAST_FRAME_SIGNATURE: Cell<Option<(bool, bool, bool, i32, i32, i32, bool)>> = const { Cell::new(None) };
}

/// ends the viewer opaque cover if it laid one and wakes what it held
fn release_viewer_opaque_cover(window: &Window, cx: &mut App) {
    if VIEWER_HOLDS_OPAQUE_COVER.with(|holds| holds.replace(false)) {
        release_opaque_cover(window, cx);
    }
}

/// the viewer screen that last took the keys, root hands it an Escape no viewer node heard
struct MediaViewerKeysOwner(WeakEntity<MediaViewerScreen>);

impl Global for MediaViewerKeysOwner {}

impl RenderOnce for MediaViewerHost {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        install_key_bindings(cx);
        let shows = {
            let snapshot = self.state.read(cx);
            !snapshot.items().is_empty() && snapshot.presentation_route() == self.route
        };
        if !shows {
            release_viewer_opaque_cover(window, cx);
            return div().into_any_element();
        }
        let key = match self.route {
            MediaViewerPresentationRoute::Inline => "media-viewer-screen-inline",
            MediaViewerPresentationRoute::Modal => "media-viewer-screen-modal",
        };
        let state = self.state.clone();
        let route = self.route;
        let screen = window.use_keyed_state(key, cx, move |window, cx| MediaViewerScreen::new(state, route, window, cx));
        div().absolute().inset_0().child(screen).occludes_bottom_chrome().into_any_element()
    }
}

enum HeroMetrics {}

impl HeroMetrics {
    const MOVE_DURATION: f64 = MediaViewerTransition::MOVE_DURATION;

    fn move_curve() -> MotionSpec {
        MediaViewerTransition::move_animation()
    }
}

enum MediaViewerScreenMetrics {}

impl MediaViewerScreenMetrics {
    const CHROME_FADE: f64 = 0.18;
    const DRAG_MINIMUM_DISTANCE: f32 = 22.0;
    const DRAG_START: f32 = 12.0;
    const DRAG_DOMINANCE: f32 = 0.72;
    const DRAG_FULL_PROGRESS: f32 = 240.0;
    const DRAG_SCALE_LOSS: f32 = 0.18;
    const DRAG_BACKDROP_LOSS: f32 = 0.55;
    const DRAG_CLOSE_DISTANCE: f32 = 130.0;
    const SNAP_BACK_RESPONSE: f64 = 0.34;
    const SNAP_BACK_DAMPING: f64 = 0.86;
    const PAGE_BADGE_LINGER: Duration = Duration::from_secs(1);
    const ACTION_BUTTON_SIZE: f32 = 54.0;
    const CAPTION_TITLE_LINE_LIMIT: usize = 2;
    const CAPTION_LINE_LIMIT: usize = 3;
    const CAPTION_HORIZONTAL_PADDING: f32 = 18.0;
    const CAPTION_SHOW_RESPONSE: f64 = 0.34;
    const CAPTION_SHOW_DAMPING: f64 = 0.88;
    const CAPTION_HIDE: f64 = 0.2;
    const PAGER_ALIGN_ATTEMPTS: i64 = 8;
    const PLAYBACK_RATES: [f32; 4] = [0.5, 1.0, 1.5, 2.0];
    const HOVER_CHROME_TOP_ZONE: f32 = 120.0;
    const HOVER_CHROME_BOTTOM_ZONE: f32 = 160.0;
    const PAGE_SWIPE_THRESHOLD: f32 = 0.2;
}

/// pages the pager by offset in pages
/// every page is one viewport wide with no spacing so page n starts at n widths
pub struct MediaViewerPagerScrollHandle {
    offset: Animated<f32>,
    wheel: WheelGestureTracker,
    wheel_base: f32,
    settle_task: Option<Task<()>>,
}

impl MediaViewerPagerScrollHandle {
    fn new() -> Self {
        Self {
            offset: Animated::new(0.0),
            wheel: WheelGestureTracker::new(),
            wheel_base: 0.0,
            settle_task: None,
        }
    }

    pub fn show(&mut self, page: usize, count: usize) -> bool {
        if page >= count {
            return false;
        }
        self.offset.jump(page as f32);
        true
    }
}

struct MediaViewerScreen {
    state: Entity<MediaViewerPresentationState>,
    route: MediaViewerPresentationRoute,
    focus_handle: FocusHandle,
    keys_taken_at: Duration,
    drag_translation: Animated<Size<Pixels>>,
    backdrop_opacity: Animated<f32>,
    close_opacity: Animated<f32>,
    veils_content: bool,
    close_work: Option<Task<()>>,
    open_work: Option<Task<()>>,
    closing_presentation_id: Option<Uuid>,
    content_scale: Animated<f32>,
    shows_chrome: bool,
    shows_chrome_opacity: Animated<f32>,
    chrome_visible_opacity: Animated<f32>,
    open_progress: Animated<f32>,
    scroll_page_id: Option<usize>,
    pager_jump_target: Option<usize>,
    pager_scroll: MediaViewerPagerScrollHandle,
    pager_aligned: bool,
    pager_ready: bool,
    page_badge_recent: bool,
    page_badge_opacity: Animated<f32>,
    page_badge_task: Option<Task<()>>,
    page_badge_key: PageBadgeTimerKey,
    bottom_bar_height: Rc<Cell<f32>>,
    caption_expandable_memo: RefCell<Option<(String, String, f32, bool)>>,
    overlay_screen_frame: Bounds<Pixels>,
    vertical_drag_active: bool,
    is_closing_hero: bool,
    media_zoomed: bool,
    video_fills_screen: bool,
    shows_caption_overlay: bool,
    caption_overlay_progress: Animated<f32>,
    playback_control_dropped: bool,
    video_controller: Entity<MediaViewerVideoController>,
    video_scrubber: Entity<MediaViewerVideoScrubber>,
    is_speed_menu_presented: bool,
    is_overflow_menu_presented: bool,
    is_delete_menu_presented: bool,
    filmstrip: Option<(String, Entity<MediaViewerFilmstrip>)>,
    last_presentation_id: Option<Uuid>,
    last_is_presented: bool,
    last_dismissal_request_id: Option<Uuid>,
    last_current_index: usize,
    last_current_item_id: Option<String>,
    last_items_count: usize,
    last_video_playing: bool,
    last_video_status: MediaViewerVideoControllerPlaybackStatus,
    last_video_rate: f32,
    _subscriptions: Vec<Subscription>,
}

impl MediaViewerScreen {
    fn new(
        state: Entity<MediaViewerPresentationState>,
        route: MediaViewerPresentationRoute,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let video_controller = cx.new(MediaViewerVideoController::new);
        let video_scrubber = {
            let controller = video_controller.clone();
            cx.new(|cx| MediaViewerVideoScrubber::new(controller, &mut *window, cx))
        };
        let focus_handle = cx.focus_handle();
        let mut subscriptions = vec![
            cx.observe_in(&state, window, |this: &mut Self, _, window, cx| this.state_did_change(window, cx)),
            cx.on_focus_out(&focus_handle, window, |_, _, window, cx| {
                cx.defer_in(window, |this: &mut Self, window, cx| this.reclaim_keys_from_page_below(window, cx));
            }),
            cx.observe(&video_controller, |this: &mut Self, controller, cx| {
                let (playing, status, rate) = {
                    let c = controller.read(cx);
                    (c.is_playing(), c.status(), c.playback_rate())
                };
                if playing != this.last_video_playing || status != this.last_video_status || rate != this.last_video_rate {
                    this.last_video_playing = playing;
                    this.last_video_status = status;
                    this.last_video_rate = rate;
                    cx.notify();
                }
            }),
        ];
        let preferences = MediaViewerPlaybackPreferences::shared(cx);
        subscriptions.push(cx.observe(&preferences, |_, _, cx| cx.notify()));
        if route == MediaViewerPresentationRoute::Inline {
            let notifier = NotifierStore::shared(cx);
            subscriptions.push(cx.observe(&notifier, |_, _, cx| cx.notify()));
        }
        let (presentation_id, is_presented, dismissal_request_id, current_index, current_item_id, items_count) = {
            let s = state.read(cx);
            (
                s.presentation_id(),
                s.is_presented(),
                s.dismissal_request_id(),
                s.current_index(),
                Self::current_item_in(s).map(|item| item.id.clone()),
                s.items().len(),
            )
        };
        let (last_video_playing, last_video_status, last_video_rate) = {
            let c = video_controller.read(cx);
            (c.is_playing(), c.status(), c.playback_rate())
        };
        let mut screen = Self {
            state,
            route,
            focus_handle,
            keys_taken_at: Duration::ZERO,
            drag_translation: Animated::new(size(px(0.0), px(0.0))),
            backdrop_opacity: Animated::new(0.0),
            close_opacity: Animated::new(1.0),
            veils_content: false,
            close_work: None,
            open_work: None,
            closing_presentation_id: None,
            content_scale: Animated::new(1.0),
            shows_chrome: true,
            shows_chrome_opacity: Animated::new(1.0),
            chrome_visible_opacity: Animated::new(1.0),
            open_progress: Animated::new(0.0),
            scroll_page_id: None,
            pager_jump_target: None,
            pager_scroll: MediaViewerPagerScrollHandle::new(),
            pager_aligned: false,
            pager_ready: false,
            page_badge_recent: true,
            page_badge_opacity: Animated::new(0.0),
            page_badge_task: None,
            page_badge_key: PageBadgeTimerKey { position: None, ready: false },
            bottom_bar_height: Rc::new(Cell::new(0.0)),
            caption_expandable_memo: RefCell::new(None),
            overlay_screen_frame: Bounds::default(),
            vertical_drag_active: false,
            is_closing_hero: false,
            media_zoomed: false,
            video_fills_screen: false,
            shows_caption_overlay: false,
            caption_overlay_progress: Animated::new(0.0),
            playback_control_dropped: false,
            video_controller,
            video_scrubber,
            is_speed_menu_presented: false,
            is_overflow_menu_presented: false,
            is_delete_menu_presented: false,
            filmstrip: None,
            last_presentation_id: presentation_id,
            last_is_presented: is_presented,
            last_dismissal_request_id: dismissal_request_id,
            last_current_index: current_index,
            last_current_item_id: current_item_id,
            last_items_count: items_count,
            last_video_playing,
            last_video_status,
            last_video_rate,
            _subscriptions: subscriptions,
        };
        screen.overlay_screen_frame = Bounds::new(point(px(0.0), px(0.0)), window.viewport_size());
        screen.reset_presentation_motion(window, cx);
        if is_presented {
            screen.take_keys(window, cx);
        }
        screen
    }

    fn now() -> Duration {
        RiseClock::monotonic()
    }

    /// runs f on the next turn of the main thread
    fn next_turn(cx: &mut Context<Self>, f: impl FnOnce(&mut Self, &mut Context<Self>) + 'static) {
        cx.spawn(async move |this, cx| {
            this.update(cx, |this, cx| f(this, cx)).ok();
        })
        .detach();
    }

    fn current_item_in(state: &MediaViewerPresentationState) -> Option<&MediaViewerGalleryItem> {
        let items = state.items();
        let count = items.len();
        if count == 0 {
            return None;
        }
        items.get(state.current_index().min(count - 1))
    }

    fn current_gallery_item(&self, cx: &App) -> Option<MediaViewerGalleryItem> {
        Self::current_item_in(self.state.read(cx)).cloned()
    }

    fn presentation_id(&self, cx: &App) -> Option<Uuid> {
        self.state.read(cx).presentation_id()
    }

    fn is_live(&self, id: Uuid, cx: &App) -> bool {
        let state = self.state.read(cx);
        state.is_active_presentation(id) && !state.is_closing_presentation(id)
    }

    fn state_did_change(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (presentation_id, is_presented, dismissal_request_id, current_index, current_item_id, items_count) = {
            let s = self.state.read(cx);
            (
                s.presentation_id(),
                s.is_presented(),
                s.dismissal_request_id(),
                s.current_index(),
                Self::current_item_in(s).map(|item| item.id.clone()),
                s.items().len(),
            )
        };

        if presentation_id != self.last_presentation_id {
            tracing::warn!(target: "riseonly::X1364", "presentation {:?} -> {:?} presented={is_presented}", self.last_presentation_id, presentation_id);
            self.last_presentation_id = presentation_id;
            if presentation_id.is_some() && is_presented {
                self.reset_presentation_motion(window, cx);
                self.take_keys(window, cx);
            }
        }
        if is_presented != self.last_is_presented {
            self.last_is_presented = is_presented;
            if is_presented {
                self.take_keys(window, cx);
            } else {
                self.cancel_close_work(cx);
                self.set_pager_ready(false, cx);
                self.media_zoomed = false;
                self.shows_caption_overlay = false;
                self.caption_overlay_progress.jump(0.0);
                self.video_controller.update(cx, |c, cx| c.reset(cx));
            }
        }
        if dismissal_request_id != self.last_dismissal_request_id {
            self.last_dismissal_request_id = dismissal_request_id;
            if dismissal_request_id.is_some() {
                self.state.update(cx, |s, cx| s.consume_dismissal_request(cx));
                self.animate_close(None, cx);
            }
        }
        if items_count != self.last_items_count {
            self.last_items_count = items_count;
            self.items_count_did_change(cx);
        }
        let current_index = self.state.read(cx).current_index();
        if current_index != self.last_current_index {
            self.last_current_index = current_index;
            if self.scroll_page_id != Some(current_index) {
                self.scroll_page_id = Some(current_index);
            }
            if self.pager_aligned {
                self.pager_scroll.offset.set_in(current_index as f32, ease_out(0.24), Self::now(), cx);
            }
        }
        if current_item_id != self.last_current_item_id {
            self.last_current_item_id = current_item_id;
            self.media_zoomed = false;
            self.shows_caption_overlay = false;
            self.caption_overlay_progress.jump(0.0);
            self.video_fills_screen = false;
            if self.pager_ready {
                self.sync_hero_to_current_page(cx);
                self.configure_playback_for_current_item(cx);
            }
        }
        self.sync_filmstrip(window, cx);
        self.restart_page_badge_timer_if_needed(cx);
        cx.notify();
    }

    fn items_count_did_change(&mut self, cx: &mut Context<Self>) {
        let count = self.state.read(cx).items().len();
        let max_index = count.saturating_sub(1);
        if self.state.read(cx).current_index() > max_index {
            self.state.update(cx, |s, cx| s.set_current_index(max_index, cx));
        }
        let target = self.state.read(cx).current_index();
        self.scroll_page_id = Some(target);
        self.pager_aligned = false;
        let Some(id) = self.presentation_id(cx) else { return };
        if !self.pager_ready {
            return;
        }
        Self::next_turn(cx, move |this: &mut Self, cx: &mut Context<Self>| {
            if !this.is_live(id, cx) || !this.pager_ready || target >= this.state.read(cx).items().len() {
                return;
            }
            this.pager_jump_target = Some(target);
            this.consume_pager_jump(cx);
        });
    }

    fn cancel_close_work(&mut self, cx: &mut Context<Self>) {
        self.close_work = None;
        if let Some(id) = self.closing_presentation_id.take() {
            tracing::warn!(target: "riseonly::X801", "close work cancelled id={id}");
            self.state.update(cx, |s, cx| s.cancel_close(id, cx));
        }
    }

    fn reset_presentation_motion(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_close_work(cx);
        self.video_controller.update(cx, |c, cx| c.reset(cx));
        self.set_shows_chrome(true, false, cx);
        self.shows_caption_overlay = false;
        self.caption_overlay_progress.jump(0.0);
        self.playback_control_dropped = false;
        self.close_opacity.jump(1.0);
        self.veils_content = false;
        self.drag_translation.jump(size(px(0.0), px(0.0)));
        self.content_scale.jump(1.0);
        self.vertical_drag_active = false;
        self.media_zoomed = false;
        self.video_fills_screen = false;
        self.scroll_page_id = Some(self.state.read(cx).current_index());
        self.animate_open(window, cx);
        let Some(id) = self.presentation_id(cx) else { return };
        Self::next_turn(cx, move |this: &mut Self, cx: &mut Context<Self>| {
            if !this.is_live(id, cx) {
                return;
            }
            this.state.update(cx, |s, cx| s.set_bubble_cell_hidden(true, cx));
        });
    }

    fn animate_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.presentation_id(cx) else { return };
        if !self.state.read(cx).is_active_presentation(id) {
            return;
        }
        self.is_closing_hero = false;
        if self.state.read(cx).source_frame().is_none() {
            self.open_progress.jump(1.0);
            self.backdrop_opacity.jump(1.0);
            self.set_pager_ready(true, cx);
            return;
        }
        self.set_pager_ready(false, cx);
        self.open_progress.jump(0.0);
        self.backdrop_opacity.jump(0.0);
        window.on_next_frame({
            let this = cx.entity().downgrade();
            move |_, cx| {
                this.update(cx, |this, cx| {
                    if !this.is_live(id, cx) {
                        return;
                    }
                    let now = Self::now();
                    this.open_progress.set_in(1.0, HeroMetrics::move_curve(), now, cx);
                    this.backdrop_opacity.set_in(1.0, HeroMetrics::move_curve(), now, cx);
                    cx.notify();
                })
                .ok();
            }
        });
        let landing = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(HeroMetrics::MOVE_DURATION)).await;
            this.update(cx, |this, cx| {
                if !this.is_live(id, cx) {
                    return;
                }
                this.handoff_hero_bitmap_to_pager_cache(cx);
                this.set_pager_ready(true, cx);
                if !this.vertical_drag_active && *this.backdrop_opacity.target() < 1.0 {
                    this.backdrop_opacity.set_in(1.0, ease_out(0.22), Self::now(), cx);
                }
                cx.notify();
            })
            .ok();
        });
        self.open_work = Some(landing);
    }

    fn set_pager_ready(&mut self, ready: bool, cx: &mut Context<Self>) {
        if self.pager_ready == ready {
            return;
        }
        tracing::warn!(target: "riseonly::X1364", "pager ready={ready}");
        self.pager_ready = ready;
        if ready {
            self.align_pager_on_mount(cx);
            self.sync_hero_to_current_page(cx);
            self.configure_playback_for_current_item(cx);
        } else {
            self.filmstrip = None;
            self.video_controller.update(cx, |c, cx| c.pause(cx));
        }
        self.restart_page_badge_timer_if_needed(cx);
    }

    fn handoff_hero_bitmap_to_pager_cache(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.current_gallery_item(cx) else { return };
        if let Some(warm) = self.state.read(cx).hero_warm_image(&item.id) {
            MediaViewerImageMemoryCache::store(
                warm,
                &item.display_source(),
                SaiAccountCacheScope::active_user_id().as_deref(),
            );
        } else {
            let source = item.display_source();
            self.state.update(cx, |s, cx| s.adopt_warm_image(&source, item.image_cache_policy, cx));
        }
    }

    fn complete_flyback_handoff(&mut self, cx: &mut Context<Self>) {
        self.state.update(cx, |s, cx| s.complete_flyback_handoff(cx));
        self.backdrop_opacity.jump(0.0);
        self.drag_translation.jump(size(px(0.0), px(0.0)));
        self.content_scale.jump(1.0);
        self.set_pager_ready(false, cx);
        self.vertical_drag_active = false;
        self.is_closing_hero = false;
    }

    fn begin_flyback_close(&mut self, cx: &mut Context<Self>) {
        self.sync_hero_to_current_page(cx);
        self.prepare_flyback_hero_bitmap(cx);
        self.is_closing_hero = true;
        self.open_progress.jump(1.0);
        self.set_pager_ready(false, cx);
    }

    fn sync_hero_to_current_page(&mut self, cx: &mut Context<Self>) {
        if !self.state.read(cx).is_presented() {
            return;
        }
        let Some(item) = self.current_gallery_item(cx) else { return };
        let lift_id = item.frame_registry_id.clone();
        let frame = self.state.read(cx).resolved_source_frame(&lift_id);
        let mut skin = MediaViewerHeroSkin::new(item.display_source(), item.uses_fit_letterbox);
        skin.media_tail = MediaCellScreenFrameRegistry::media_tail_presentation(&lift_id);
        skin.source_media_radii = MediaCellScreenFrameRegistry::source_media_radii(&lift_id);
        self.state
            .update(cx, |s, cx| s.sync_hero_target(&lift_id, frame, skin, item.image_cache_policy, cx));
    }

    fn can_fly_back_to_current_bubble(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(item) = self.current_gallery_item(cx) else { return false };
        MediaViewerPresentationState::prepare_flyback(&self.state, &item.frame_registry_id, cx);
        self.state
            .read(cx)
            .resolved_flyback_source_frame(&item.frame_registry_id, self.overlay_screen_frame)
            .is_some()
    }

    fn prepare_flyback_hero_bitmap(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.current_gallery_item(cx) else { return };
        let source = item.display_source();
        let cached = MediaViewerImageMemoryCache::image(
            &source,
            SaiAccountCacheScope::active_user_id().as_deref(),
            item.image_cache_policy,
        );
        match cached {
            Some(cached) => self.state.update(cx, |s, cx| s.set_hero_warm_ui_image(Some(cached), &source, cx)),
            None => self.state.update(cx, |s, cx| s.seed_warm_image_from_cache(cx)),
        }
    }

    /// unanimated because the glass circle ignores an
    /// opacity fade and would stay full until the close ends
    fn drop_playback_control(&mut self) {
        self.playback_control_dropped = true;
    }

    fn schedule_close(&mut self, id: Uuid, after: f64, completion: Option<MediaViewerBoundAction>, cx: &mut Context<Self>) {
        self.closing_presentation_id = Some(id);
        let work = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(after)).await;
            this.update(cx, |this, cx| {
                if !this.state.read(cx).is_closing_presentation(id) {
                    let state = this.state.read(cx);
                    tracing::warn!(target: "riseonly::X801", "close skipped id={id} active={} presented={} current={:?}", state.is_active_presentation(id), state.is_presented(), state.presentation_id());
                    return;
                }
                tracing::warn!(target: "riseonly::X801", "close handoff id={id}");
                this.closing_presentation_id = None;
                this.complete_flyback_handoff(cx);
                if let Some(completion) = completion.as_ref() {
                    let app: &mut App = cx;
                    completion(app);
                }
                cx.notify();
                this.close_work = None;
            })
            .ok();
        });
        self.close_work = Some(work);
    }

    fn animate_close(&mut self, completion: Option<MediaViewerBoundAction>, cx: &mut Context<Self>) {
        let Some(id) = self.state.update(cx, |s, cx| s.begin_close(cx)) else {
            tracing::warn!(target: "riseonly::X2255", "close refused, no live presentation");
            return;
        };
        self.drop_playback_control();
        if self.state.read(cx).closes_without_flyback() {
            self.close_view_limited(id, completion, cx);
            return;
        }
        let can_fly_back = self.can_fly_back_to_current_bubble(cx);
        tracing::warn!(target: "riseonly::X801", "animate close id={id} fly_back={can_fly_back}");
        let now = Self::now();
        if can_fly_back {
            self.begin_flyback_close(cx);
            self.open_progress.set_in(0.0, HeroMetrics::move_curve(), now, cx);
            self.backdrop_opacity.set_in(0.0, HeroMetrics::move_curve(), now, cx);
            self.schedule_close(id, HeroMetrics::MOVE_DURATION * 0.97, completion, cx);
        } else {
            self.is_closing_hero = false;
            self.backdrop_opacity.set_in(0.0, ease_out(0.22), now, cx);
            self.close_opacity.set_in(0.0, ease_out(0.22), now, cx);
            self.schedule_close(id, 0.24, completion, cx);
        }
        cx.notify();
    }

    /// file goes under black at once, then the chat swaps the tile for the service line
    /// a fade made the chat commit the line under a half transparent photo and bubbles jumped
    fn close_view_limited(&mut self, id: Uuid, completion: Option<MediaViewerBoundAction>, cx: &mut Context<Self>) {
        self.is_closing_hero = false;
        self.veils_content = true;
        self.shows_caption_overlay = false;
        self.caption_overlay_progress.jump(0.0);
        self.drag_translation.jump(size(px(0.0), px(0.0)));
        self.content_scale.jump(1.0);
        self.backdrop_opacity.jump(1.0);
        self.closing_presentation_id = Some(id);
        let registry_id = self.current_gallery_item(cx).map(|item| item.frame_registry_id);
        let start = Self::now();
        self.await_view_limited_line(
            id,
            registry_id,
            start + Duration::from_secs_f64(0.26),
            start + Duration::from_secs_f64(1.5),
            completion,
            cx,
        );
        cx.notify();
    }

    fn await_view_limited_line(
        &mut self,
        id: Uuid,
        registry_id: Option<String>,
        earliest: Duration,
        deadline: Duration,
        completion: Option<MediaViewerBoundAction>,
        cx: &mut Context<Self>,
    ) {
        let work = cx.spawn(async move |this, cx| {
            let mut settled_frames = 0_i64;
            loop {
                let fades = this.update(cx, |this, cx| {
                    if !this.state.read(cx).is_closing_presentation(id) {
                        return None;
                    }
                    let tile_on_screen = registry_id.as_deref().is_some_and(|registry_id| {
                        this.state
                            .read(cx)
                            .resolved_flyback_source_frame(registry_id, this.overlay_screen_frame)
                            .is_some()
                    });
                    settled_frames = if tile_on_screen { 0 } else { settled_frames + 1 };
                    let now = Self::now();
                    Some((settled_frames > 2 && now >= earliest) || now >= deadline)
                });
                match fades {
                    Ok(Some(true)) => break,
                    Ok(Some(false)) => {}
                    _ => return,
                }
                cx.background_executor().timer(Duration::from_secs_f64(1.0 / 60.0)).await;
            }
            this.update(cx, |_, cx| {
                Self::next_turn(cx, move |this: &mut Self, cx: &mut Context<Self>| {
                    if !this.state.read(cx).is_closing_presentation(id) {
                        return;
                    }
                    let now = Self::now();
                    this.backdrop_opacity.set_in(0.0, ease_out(0.22), now, cx);
                    this.close_opacity.set_in(0.0, ease_out(0.22), now, cx);
                    this.schedule_close(id, 0.24, completion, cx);
                    cx.notify();
                });
            })
            .ok();
        });
        self.close_work = Some(work);
    }

    fn close_after_drag(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.state.update(cx, |s, cx| s.begin_close(cx)) else { return };
        self.drop_playback_control();
        if self.state.read(cx).closes_without_flyback() {
            self.close_view_limited(id, None, cx);
            return;
        }
        let can_fly_back = self.can_fly_back_to_current_bubble(cx);
        let now = Self::now();
        let zero = size(px(0.0), px(0.0));
        if can_fly_back {
            self.begin_flyback_close(cx);
            let curve = HeroMetrics::move_curve();
            self.open_progress.set_in(0.0, curve, now, cx);
            self.drag_translation.set_in(zero, curve, now, cx);
            self.content_scale.set_in(1.0, curve, now, cx);
            self.backdrop_opacity.set_in(0.0, curve, now, cx);
            self.schedule_close(id, HeroMetrics::MOVE_DURATION * 0.97, None, cx);
        } else {
            self.is_closing_hero = false;
            let curve = ease_out(0.24);
            self.drag_translation.set_in(zero, curve, now, cx);
            self.content_scale.set_in(1.0, curve, now, cx);
            self.backdrop_opacity.set_in(0.0, curve, now, cx);
            self.close_opacity.set_in(0.0, curve, now, cx);
            self.schedule_close(id, 0.26, None, cx);
        }
        cx.notify();
    }

    fn seed_pager_image(&self, item: &MediaViewerGalleryItem, cx: &App) -> Option<CleverImage> {
        let current = self.current_gallery_item(cx)?;
        if current.id != item.id {
            return None;
        }
        if let Some(warm) = self.state.read(cx).hero_warm_image(&item.id) {
            return Some(warm);
        }
        MediaViewerImageMemoryCache::image(
            &item.display_source(),
            SaiAccountCacheScope::active_user_id().as_deref(),
            item.image_cache_policy,
        )
    }

    fn chat_row_preview_image(item: &MediaViewerGalleryItem) -> Option<CleverImage> {
        if item.image_cache_policy == CleverImageCachePolicy::PrivateEphemeral {
            return None;
        }
        CleverImageRenderMemoryCache::latest_image(
            &CleverImageDiskCache::normalized_cache_key(&item.display_source()),
            SaiAccountCacheScope::active_user_id().as_deref(),
            SaiAccountCacheScope::active_epoch().as_deref(),
        )
    }

    fn align_pager_on_mount(&mut self, cx: &mut Context<Self>) {
        self.pager_aligned = false;
        let target = self.state.read(cx).current_index();
        self.align_pager(target, MediaViewerScreenMetrics::PAGER_ALIGN_ATTEMPTS, cx);
    }

    fn consume_pager_jump(&mut self, cx: &mut Context<Self>) {
        let Some(target) = self.pager_jump_target.take() else { return };
        self.align_pager(target, MediaViewerScreenMetrics::PAGER_ALIGN_ATTEMPTS, cx);
    }

    fn align_pager(&mut self, target: usize, attempts: i64, cx: &mut Context<Self>) {
        let count = self.state.read(cx).items().len();
        let Some(id) = self.presentation_id(cx) else { return };
        if target >= count || !self.state.read(cx).is_active_presentation(id) {
            return;
        }
        let shown = self.pager_scroll.show(target, count);
        if self.scroll_page_id != Some(target) {
            self.scroll_page_id = Some(target);
        }
        if self.state.read(cx).current_index() != target {
            self.state.update(cx, |s, cx| s.set_current_index(target, cx));
        }
        if shown || attempts <= 1 {
            self.pager_aligned = true;
            return;
        }
        Self::next_turn(cx, move |this: &mut Self, cx: &mut Context<Self>| {
            if !this.is_live(id, cx) || !this.pager_ready {
                return;
            }
            let index = this.state.read(cx).current_index();
            this.align_pager(index, attempts - 1, cx);
        });
    }

    /// a settled swipe reports its page
    fn pager_did_settle(&mut self, page: usize, cx: &mut Context<Self>) {
        let count = self.state.read(cx).items().len();
        if !self.pager_aligned || count == 0 {
            return;
        }
        let settled = page.min(count - 1);
        self.scroll_page_id = Some(settled);
        if self.state.read(cx).current_index() != settled {
            self.state.update(cx, |s, cx| s.set_current_index(settled, cx));
        }
    }

    /// 1 when the next page is on the right and -1 in rtl where the pager is mirrored
    fn forward_sign() -> f32 {
        if LayoutDirection::current().is_right_to_left() { -1.0 } else { 1.0 }
    }

    fn page_by(&mut self, delta: i64, cx: &mut Context<Self>) {
        if !self.pager_ready || self.vertical_drag_active || !self.is_interactive(cx) {
            return;
        }
        let count = self.state.read(cx).items().len() as i64;
        let current = self.state.read(cx).current_index() as i64;
        let target = (current + delta).clamp(0, (count - 1).max(0)) as usize;
        if target as i64 == current {
            return;
        }
        self.pager_scroll.offset.set_in(target as f32, ease_out(0.24), Self::now(), cx);
        self.pager_did_settle(target, cx);
        cx.notify();
    }

    fn handle_scroll_wheel(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pager_ready || self.media_zoomed || !self.is_interactive(cx) {
            return;
        }
        let feed = self.pager_scroll.wheel.feed(event, window.line_height());
        self.apply_wheel_step(feed.step, window, cx);
        if let Some(token) = feed.settle {
            let task = cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(WHEEL_SETTLE_DELAY).await;
                this.update_in(cx, |this: &mut Self, window, cx| {
                    if let Some(step) = this.pager_scroll.wheel.settle(token) {
                        this.apply_wheel_step(step, window, cx);
                    }
                })
                .ok();
            });
            self.pager_scroll.settle_task = Some(task);
        }
    }

    fn apply_wheel_step(&mut self, step: WheelStep, _window: &mut Window, cx: &mut Context<Self>) {
        let width = f32::from(self.overlay_screen_frame.size.width).max(1.0);
        let count = self.state.read(cx).items().len();
        match step {
            WheelStep::Ignored => {}
            WheelStep::Moved(translation) => {
                let dx = f32::from(translation.x);
                let dy = f32::from(translation.y);
                if self.vertical_drag_active || (dy > MediaViewerScreenMetrics::DRAG_START && dy > dx.abs() * MediaViewerScreenMetrics::DRAG_DOMINANCE) {
                    self.dismiss_drag_changed(dx, dy, cx);
                    return;
                }
                let base = self.state.read(cx).current_index() as f32;
                self.pager_scroll.wheel_base = base;
                let max = count.saturating_sub(1) as f32;
                self.pager_scroll.offset.jump((base - Self::forward_sign() * dx / width).clamp(0.0, max));
                cx.notify();
            }
            WheelStep::Ended(translation) => {
                if self.vertical_drag_active {
                    self.dismiss_drag_ended(f32::from(translation.y), cx);
                    return;
                }
                let base = self.pager_scroll.wheel_base;
                let moved = -Self::forward_sign() * f32::from(translation.x) / width;
                let target = if moved > MediaViewerScreenMetrics::PAGE_SWIPE_THRESHOLD {
                    base + 1.0
                } else if moved < -MediaViewerScreenMetrics::PAGE_SWIPE_THRESHOLD {
                    base - 1.0
                } else {
                    base
                };
                let target = target.clamp(0.0, count.saturating_sub(1) as f32);
                self.pager_scroll.offset.set_in(target, ease_out(0.24), Self::now(), cx);
                self.pager_did_settle(target as usize, cx);
                cx.notify();
            }
            WheelStep::Cancelled => {
                if self.vertical_drag_active {
                    self.dismiss_drag_ended(0.0, cx);
                    return;
                }
                let base = self.state.read(cx).current_index() as f32;
                self.pager_scroll.offset.set_in(base, ease_out(0.24), Self::now(), cx);
                cx.notify();
            }
        }
    }

    fn configure_playback_for_current_item(&mut self, cx: &mut Context<Self>) {
        let item = self.current_gallery_item(cx);
        let Some(item) = item.filter(|item| self.pager_ready && item.kind == MediaViewerGalleryItemKind::Video) else {
            self.video_controller.update(cx, |c, cx| c.reset(cx));
            return;
        };
        let loops = MediaViewerPlaybackPreferences::shared(cx).read(cx).loops_video();
        let cache_key = MediaViewerVideoCacheKey::resolve(&item.url);
        self.video_controller.update(cx, |c, cx| {
            c.set_loops_playback(loops, cx);
            c.load(&item.url, true, item.persist_media_to_disk, cache_key, cx);
        });
    }

    fn set_shows_chrome(&mut self, shows: bool, animated: bool, cx: &App) {
        self.shows_chrome = shows;
        let spec = animated.then(|| ease_in_out(MediaViewerScreenMetrics::CHROME_FADE));
        self.shows_chrome_opacity.set_in(if shows { 1.0 } else { 0.0 }, spec, Self::now(), cx);
    }

    fn toggle_chrome(&mut self, cx: &mut Context<Self>) {
        if self.shows_caption_overlay {
            return;
        }
        let shows = !self.shows_chrome;
        tracing::warn!(target: "riseonly::X2255", shows, "viewer chrome toggled");
        self.set_shows_chrome(shows, true, cx);
        cx.notify();
    }

    fn is_interactive(&self, cx: &App) -> bool {
        self.state.read(cx).is_presented() && self.closing_presentation_id.is_none()
    }

    fn take_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.keys_taken_at = Self::now();
        let owner = MediaViewerKeysOwner(cx.weak_entity());
        cx.set_global(owner);
        window.focus(&self.focus_handle, cx);
    }

    fn sheet_above(keys_taken_at: Duration, window: &Window, cx: &App) -> bool {
        ScreenFocus::top_sheet_presented_at(window, cx).is_some_and(|at| at > keys_taken_at)
            && ScreenFocus::top_sheet(window, cx).is_some_and(|sheet| sheet.contains_focused(window, cx))
    }

    /// true when no node holds the keys or the page under the viewer or a field on it does
    fn focus_strayed_below(window: &Window, cx: &App) -> bool {
        let page_holds_keys =
            ScreenFocus::last_screen(window, cx).is_some_and(|page| page.contains_focused(window, cx));
        let field_below = ScreenFocus::text_field_is_focused(window, cx)
            && window.focused(cx).is_some_and(|focused| focused.is_in_rendered_frame(window));
        ScreenFocus::is_idle(window, cx) || page_holds_keys || field_below
    }

    fn reclaim_keys_from_page_below(&self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_interactive(cx) || self.focus_handle.contains_focused(window, cx) {
            return;
        }
        if Self::sheet_above(self.keys_taken_at, window, cx) {
            return;
        }
        if Self::focus_strayed_below(window, cx) {
            tracing::warn!(target: "riseonly::X1357", focused = ?window.focused(cx), "viewer takes the keys back");
            window.focus(&self.focus_handle, cx);
        }
    }

    fn handle_mouse_move(&mut self, event: &MouseMoveEvent, cx: &mut Context<Self>) {
        if self.shows_chrome || self.shows_caption_overlay || !self.pager_ready || !self.is_interactive(cx) {
            return;
        }
        let height = f32::from(self.overlay_screen_frame.size.height);
        let y = f32::from(event.position.y - self.overlay_screen_frame.origin.y);
        if y <= MediaViewerScreenMetrics::HOVER_CHROME_TOP_ZONE
            || y >= height - MediaViewerScreenMetrics::HOVER_CHROME_BOTTOM_ZONE
        {
            self.set_shows_chrome(true, true, cx);
            cx.notify();
        }
    }

    fn dismiss_drag_changed(&mut self, dx: f32, dy: f32, cx: &mut Context<Self>) {
        if self.media_zoomed || self.closing_presentation_id.is_some() {
            return;
        }
        if !self.vertical_drag_active {
            if !(dy > MediaViewerScreenMetrics::DRAG_START && dy > dx.abs() * MediaViewerScreenMetrics::DRAG_DOMINANCE) {
                return;
            }
            self.vertical_drag_active = true;
        }
        let down = dy.max(0.0);
        self.drag_translation.jump(size(px(0.0), px(down)));
        let progress = (down / MediaViewerScreenMetrics::DRAG_FULL_PROGRESS).clamp(0.0, 1.0);
        self.content_scale.jump(1.0 - progress * MediaViewerScreenMetrics::DRAG_SCALE_LOSS);
        let backdrop = if self.state.read(cx).closes_without_flyback() {
            1.0
        } else {
            1.0 - progress * MediaViewerScreenMetrics::DRAG_BACKDROP_LOSS
        };
        self.backdrop_opacity.jump(backdrop);
        cx.notify();
    }

    fn dismiss_drag_ended(&mut self, dy: f32, cx: &mut Context<Self>) {
        if !self.vertical_drag_active {
            return;
        }
        self.vertical_drag_active = false;
        if dy > MediaViewerScreenMetrics::DRAG_CLOSE_DISTANCE {
            self.close_after_drag(cx);
        } else {
            let snap = spring(MediaViewerScreenMetrics::SNAP_BACK_RESPONSE, MediaViewerScreenMetrics::SNAP_BACK_DAMPING);
            let now = Self::now();
            self.drag_translation.set_in(size(px(0.0), px(0.0)), snap, now, cx);
            self.content_scale.set_in(1.0, snap, now, cx);
            self.backdrop_opacity.set_in(1.0, snap, now, cx);
        }
        cx.notify();
    }

    fn vertical_dismiss_gesture(&self, cx: &Context<Self>) -> DragGesture {
        let changed = cx.entity().downgrade();
        let ended = changed.clone();
        DragGesture::new(MediaViewerScreenMetrics::DRAG_MINIMUM_DISTANCE)
            .on_changed(move |value: &DragGestureValue, _, cx| {
                changed
                    .update(cx, |this, cx| {
                        this.dismiss_drag_changed(f32::from(value.translation.width), f32::from(value.translation.height), cx)
                    })
                    .ok();
            })
            .on_ended(move |value: &DragGestureValue, _, cx| {
                ended
                    .update(cx, |this, cx| this.dismiss_drag_ended(f32::from(value.translation.height), cx))
                    .ok();
            })
    }

    /// rail exists only with something to scrub and only once the flight landed
    /// up to fifty decodes stay off the frames the hero flies on
    fn shows_filmstrip(&self, cx: &App) -> bool {
        self.pager_ready && self.state.read(cx).filmstrip_range().len() > 1
    }

    fn sync_filmstrip(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.shows_filmstrip(cx) {
            self.filmstrip = None;
            return;
        }
        let width = window.viewport_size().width;
        let key = self.state.read(cx).filmstrip_group_key();
        if let Some((current_key, rail)) = &self.filmstrip
            && *current_key == key
        {
            rail.update(cx, |rail, cx| rail.host_did_change(width, cx));
            return;
        }
        let this = cx.entity().downgrade();
        let on_select = Rc::new(move |index: usize, window: &mut Window, cx: &mut App| {
            this.update(cx, |this, cx| {
                tracing::warn!(
                    target: "riseonly::X1364",
                    "filmstrip select index={index} current={} pager_ready={} chrome={}",
                    this.state.read(cx).current_index(),
                    this.pager_ready,
                    this.shows_chrome
                );
                this.scroll_page_id = Some(index);
                this.pager_jump_target = Some(index);
                this.consume_pager_jump(cx);
                tracing::warn!(target: "riseonly::X1364", focused = ?window.focused(cx), "filmstrip select keys");
                this.take_keys(window, cx);
                cx.defer_in(window, |this: &mut Self, window, cx| {
                    if this.is_interactive(cx) && !this.focus_handle.contains_focused(window, cx) {
                        tracing::warn!(target: "riseonly::X1364", focused = ?window.focused(cx), "filmstrip keys retaken");
                        this.take_keys(window, cx);
                    }
                });
                cx.notify();
            })
            .ok();
        });
        let state = self.state.clone();
        let rail = cx.new(|cx| MediaViewerFilmstrip::new(state, width, on_select, cx));
        self.filmstrip = Some((key, rail));
    }

    fn gallery_counter(&self, cx: &App) -> Option<(i64, i64)> {
        let state = self.state.read(cx);
        let current_id = Self::current_item_in(state).map(|item| item.id.as_str());
        if state.gallery_scope() == MediaViewerGalleryScope::Paged {
            let position = state.gallery_position()?;
            let total = state.gallery_total()?;
            if position <= 0 || position > total || state.gallery_counter_item_id() != current_id {
                return None;
            }
            return Some((position, total));
        }
        let count = state.items().len();
        if count <= 1 {
            return None;
        }
        Some((state.current_index().min(count - 1) as i64 + 1, count as i64))
    }

    /// position changes only while paging so the timer restarts with it
    /// a hero open readies the pager after the flight so the second counts from then
    fn restart_page_badge_timer_if_needed(&mut self, cx: &mut Context<Self>) {
        let key = PageBadgeTimerKey { position: self.gallery_counter(cx).map(|(position, _)| position), ready: self.pager_ready };
        if key == self.page_badge_key {
            return;
        }
        self.page_badge_key = key;
        self.page_badge_recent = true;
        self.page_badge_task = None;
        self.sync_page_badge_opacity(cx);
        if !self.pager_ready {
            return;
        }
        self.page_badge_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(MediaViewerScreenMetrics::PAGE_BADGE_LINGER).await;
            this.update(cx, |this, cx| {
                this.page_badge_recent = false;
                this.sync_page_badge_opacity(cx);
                cx.notify();
            })
            .ok();
        }));
    }

    fn sync_page_badge_opacity(&mut self, cx: &App) {
        let visible = self.gallery_counter(cx).is_some() && self.pager_ready && self.page_badge_recent;
        let target = if visible { 1.0 } else { 0.0 };
        self.page_badge_opacity
            .set_in(target, ease_in_out(MediaViewerScreenMetrics::CHROME_FADE), Self::now(), cx);
    }

    fn shows_center_playback_control(&self, cx: &App) -> bool {
        let status = self.video_controller.read(cx).status();
        self.shows_chrome
            && self.pager_ready
            && !self.playback_control_dropped
            && self.current_gallery_item(cx).is_some_and(|item| item.kind == MediaViewerGalleryItemKind::Video)
            && status != MediaViewerVideoControllerPlaybackStatus::Failed
            && status != MediaViewerVideoControllerPlaybackStatus::Loading
            && !self.shows_caption_overlay
    }

    fn overflow_menu_has_items(&self, item: &MediaViewerGalleryItem, cx: &App) -> bool {
        let state = self.state.read(cx);
        Self::is_savable(item)
            || state.can_set_as_main(item)
            || state.can_delete_profile_photo(item)
            || state.can_show_in_context(item)
            || state.can_reply(item)
            || item.can_delete
            || state.can_report(item)
    }

    fn is_savable(item: &MediaViewerGalleryItem) -> bool {
        item.can_save
    }

    fn delete_title(&self, item: &MediaViewerGalleryItem, cx: &App) -> String {
        if self.state.read(cx).album_item_count(item).is_none() {
            tr("ctx_delete")
        } else {
            tr("ctx_delete_album")
        }
    }

    /// an album is one message so deleting one picture deletes all, the count is confirmed first
    /// a single picture is deleted at once
    fn perform_delete(&mut self, item: MediaViewerGalleryItem, mode: MediaViewerDeleteMode, cx: &mut Context<Self>) {
        let Some(album_count) = self.state.read(cx).album_item_count(&item) else {
            self.commit_delete(item, mode, cx);
            return;
        };
        let this = cx.entity().downgrade();
        NativeConfirmAlert::present_delete(
            &tr("media_viewer_delete_album_title"),
            &tr_plural("media_viewer_delete_album_message", album_count as i64),
            None,
            None,
            false,
            None,
            None,
            move |cx| {
                this.update(cx, |this, cx| {
                    let still_shown = {
                        let state = this.state.read(cx);
                        state.is_presented() && state.items().iter().any(|row| row.id == item.id)
                    };
                    if still_shown {
                        this.commit_delete(item, mode, cx);
                    }
                })
                .ok();
            },
            cx,
        );
    }

    fn commit_delete(&mut self, item: MediaViewerGalleryItem, mode: MediaViewerDeleteMode, cx: &mut Context<Self>) {
        let delete = self.state.read(cx).delete_action(&item, mode);
        if self.state.read(cx).gallery_scope() == MediaViewerGalleryScope::Paged {
            let app: &mut App = cx;
            delete(app);
            return;
        }
        if self.state.read(cx).remaining_item_count(&item.message_id) == 0 {
            self.animate_close(Some(delete), cx);
            return;
        }
        {
            let app: &mut App = cx;
            delete(app);
        }
        self.state.update(cx, |s, cx| s.remove_items_for_deleted_message(&item.message_id, cx));
        self.scroll_page_id = Some(self.state.read(cx).current_index());
        self.sync_hero_to_current_page(cx);
        self.configure_playback_for_current_item(cx);
        cx.notify();
    }

    fn perform_context_action(&mut self, action: Option<MediaViewerBoundAction>, cx: &mut Context<Self>) {
        let Some(action) = action else {
            tracing::warn!(target: "riseonly::X2255", "context action unavailable, viewer stays");
            return;
        };
        self.animate_close(Some(action), cx);
    }

    fn save_current_media(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.current_gallery_item(cx) else { return };
        if !Self::is_savable(&item) {
            return;
        }
        if let Some(post_ref) = item.post_download.clone() {
            self.save_post_media(post_ref, cx);
            return;
        }
        let offset = self.save_toast_offset();
        cx.spawn(async move |_, cx| {
            if item.kind == MediaViewerGalleryItemKind::Video {
                let progress_toast_id: Rc<Cell<Option<Uuid>>> = Rc::new(Cell::new(None));
                let progress_slot = Rc::clone(&progress_toast_id);
                let progress_toast = cx.spawn(async move |cx| {
                    cx.background_executor()
                        .timer(Duration::from_secs_f64(BlockingLoaderStore::APPEARANCE_DELAY))
                        .await;
                    cx.update(|cx| progress_slot.set(Some(Self::announce_save_in_progress(offset, cx))));
                });
                let result = MediaSaveToLibrary::save_video(
                    &item.url,
                    item.persist_media_to_disk,
                    MediaViewerVideoCacheKey::resolve(&item.url),
                    cx,
                )
                .await;
                drop(progress_toast);
                cx.update(|cx| {
                    if let Some(id) = progress_toast_id.get() {
                        Notifier::dismiss(id, cx);
                    }
                    MediaSaveToLibrary::announce(result, NotifierPosition::Bottom, offset, cx);
                });
                return;
            }
            let full_source = RemoteMediaURL::optional_normalized(Some(&item.url)).unwrap_or_else(|| item.display_source());
            let Some(image) = Self::resolve_download_image(&full_source, &item, cx).await else {
                let _ = notification(NotificationKind::Error);
                cx.update(|cx| MediaSaveToLibrary::announce(MediaSaveToLibraryResult::Failed, NotifierPosition::Bottom, offset, cx));
                return;
            };
            let result = MediaSaveToLibrary::save(image, cx).await;
            cx.update(|cx| MediaSaveToLibrary::announce(result, NotifierPosition::Bottom, offset, cx));
        })
        .detach();
    }

    /// desktop save from the overflow menu, native save panel picks the place and bytes go there
    /// a post keeps its own download flow
    fn save_current_media_as_file(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.current_gallery_item(cx) else { return };
        if !Self::is_savable(&item) {
            return;
        }
        if let Some(post_ref) = item.post_download.clone() {
            self.save_post_media(post_ref, cx);
            return;
        }
        let offset = self.save_toast_offset();
        let is_video = item.kind == MediaViewerGalleryItemKind::Video;
        let (label, extension) = if is_video {
            (FilterLabel::VIDEO, MediaSaveToLibrary::export_extension(&item.url, true))
        } else {
            (FilterLabel::IMAGES, MediaSaveToLibrary::export_extension(&item.url, false))
        };
        let mut request = SaveRequest::new(downloads_directory().unwrap_or_default(), FileFilter::new(label, [extension]));
        if let Some(name) = MediaSaveToLibrary::export_name(&item.url, is_video) {
            request = request.with_suggested_name(name);
        }
        let outcome = file_dialog::save(cx, &request);
        cx.spawn(async move |_, cx| {
            let destination = match outcome.await {
                SaveOutcome::Chosen(destination) => destination,
                SaveOutcome::Cancelled => return,
                SaveOutcome::Failed(error) => {
                    tracing::warn!(target: "riseonly", %error, "viewer save panel failed");
                    cx.update(|cx| MediaSaveToLibrary::announce_export(MediaSaveToLibraryResult::Failed, is_video, offset, cx));
                    return;
                }
            };
            let opened_path = destination.clone();
            let result = if is_video {
                let progress_toast_id: Rc<Cell<Option<Uuid>>> = Rc::new(Cell::new(None));
                let progress_slot = Rc::clone(&progress_toast_id);
                let progress_toast = cx.spawn(async move |cx| {
                    cx.background_executor()
                        .timer(Duration::from_secs_f64(BlockingLoaderStore::APPEARANCE_DELAY))
                        .await;
                    cx.update(|cx| progress_slot.set(Some(Self::announce_save_in_progress(offset, cx))));
                });
                let result = MediaSaveToLibrary::export_video(
                    &item.url,
                    item.persist_media_to_disk,
                    MediaViewerVideoCacheKey::resolve(&item.url),
                    destination,
                    cx,
                )
                .await;
                drop(progress_toast);
                if let Some(id) = progress_toast_id.get() {
                    cx.update(|cx| {
                        Notifier::dismiss(id, cx);
                    });
                }
                result
            } else {
                let full_source =
                    RemoteMediaURL::optional_normalized(Some(&item.url)).unwrap_or_else(|| item.display_source());
                match Self::resolve_download_image(&full_source, &item, cx).await {
                    Some(image) => MediaSaveToLibrary::export_image(image, destination, cx).await,
                    None => {
                        let _ = notification(NotificationKind::Error);
                        MediaSaveToLibraryResult::Failed
                    }
                }
            };
            let saved = matches!(result, MediaSaveToLibraryResult::Saved);
            cx.update(|cx| {
                MediaSaveToLibrary::announce_export(result, is_video, offset, cx);
                if saved
                    && let Err(error) = reveal_in_file_manager::open_with_system(cx, &opened_path, ExecutableConsent::Refuse)
                {
                    tracing::warn!(target: "riseonly", %error, "saved viewer media did not open");
                }
            });
        })
        .detach();
    }

    /// the live post decides, the author may have switched downloads off since the viewer opened
    fn save_post_media(&mut self, post_ref: PostDownloadRef, cx: &mut Context<Self>) {
        let offset = self.save_toast_offset();
        let post = PostServicesStore::shared(cx)
            .update(cx, |store, cx| store.resolved_post(&post_ref.post_id, cx))
            .or_else(|| ShortsServicesStore::shared(cx).update(cx, |store, cx| store.resolved_short(&post_ref.post_id, cx)))
            .or_else(|| PostDownloadSheetStore::shared(cx).read(cx).viewer_post(&post_ref.post_id));
        let Some(post) = post.filter(|post| post.allows_download_entry(cx)) else {
            MediaSaveToLibrary::announce(MediaSaveToLibraryResult::Failed, NotifierPosition::Bottom, offset, cx);
            return;
        };
        PostDownloadSheetStore::shared(cx).update(cx, |store, cx| store.start(post, post_ref.index, true, offset, cx));
    }

    /// toast stays up with duration 0 until the save ends and the result replaces it
    fn announce_save_in_progress(offset: f32, cx: &mut App) -> Uuid {
        Notifier::send(
            tr("common_loading"),
            NotifierSendOptions {
                r#type: NotifierType::Info,
                position: NotifierPosition::Bottom,
                offset,
                icon: NotifierIcon::System("arrow.down.to.line".to_owned()),
                duration: 0.0,
                ..NotifierSendOptions::default()
            },
            cx,
        )
    }

    fn save_toast_offset(&self) -> f32 {
        if !self.shows_chrome {
            return 0.0;
        }
        self.bottom_bar_height.get().max(0.0)
    }

    async fn resolve_download_image(
        source: &str,
        item: &MediaViewerGalleryItem,
        cx: &gpui::AsyncApp,
    ) -> Option<CleverImage> {
        if let Some(cached) = MediaViewerImageMemoryCache::image(
            source,
            SaiAccountCacheScope::active_user_id().as_deref(),
            item.image_cache_policy,
        ) {
            return Some(cached);
        }
        MediaViewerBitmapLoader::load_into_memory(source, true, item.persist_media_to_disk, item.image_cache_policy, cx)
            .await
    }

    fn copy_current_media(&mut self, cx: &mut Context<Self>) {
        let Some(item) = self.current_gallery_item(cx) else { return };
        if item.kind == MediaViewerGalleryItemKind::Video || !Self::is_savable(&item) {
            return;
        }
        cx.spawn(async move |_, cx| {
            let source = RemoteMediaURL::optional_normalized(Some(&item.url)).unwrap_or_else(|| item.display_source());
            let Some(image) = Self::resolve_download_image(&source, &item, cx).await else { return };
            let Some(rgba) = CleverImageBitmapEncoder::rgba(&image) else { return };
            let png = RiseAccountRuntime::current()
                .spawn_blocking(move || {
                    let mut bytes = Vec::new();
                    image::DynamicImage::ImageRgba8(rgba)
                        .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
                        .ok()
                        .map(|_| bytes)
                })
                .await
                .ok()
                .flatten();
            let Some(png) = png else { return };
            let image = gpui::Image::from_bytes(gpui::ImageFormat::Png, png);
            cx.update(|cx| UIPasteboard::general().set_image(&image, cx));
        })
        .detach();
    }

    fn format_header_date(ts: TimeInterval) -> String {
        let date = Date::from_time_interval_since_1970(ts);
        let calendar = Calendar::current();
        let mut formatter = DateFormatter::new();
        formatter.set_locale(Locale::app_locale().identifier());
        if calendar.is_date_in_today(&date) {
            formatter.set_date_format(tr("date_today_at_format"));
        } else if calendar.is_date_in_yesterday(&date) {
            formatter.set_date_format(tr("date_yesterday_at_format"));
        } else if calendar.is_date_equal_to(&date, &Date::now(), CalendarComponent::Year) {
            formatter.set_date_format(tr("date_day_month_at_format"));
        } else {
            formatter.set_date_format(tr("date_day_month_year_at_format"));
        }
        formatter.string_from(&date)
    }

    fn has_caption(item: &MediaViewerGalleryItem) -> bool {
        !trimming_whitespaces_and_newlines(&item.caption_title).is_empty() || !trimming_whitespaces_and_newlines(&item.caption).is_empty()
    }

    fn caption_is_expandable(&self, item: &MediaViewerGalleryItem, window: &Window) -> bool {
        let width = (f32::from(window.viewport_size().width) - MediaViewerScreenMetrics::CAPTION_HORIZONTAL_PADDING * 2.0).max(1.0);
        if let Some((title, caption, memo_width, expandable)) = self.caption_expandable_memo.borrow().as_ref()
            && *title == item.caption_title
            && *caption == item.caption
            && (*memo_width - width).abs() < 0.5
        {
            return *expandable;
        }
        let exceeds = |text: &str, font: Font, limit: usize| -> bool {
            if text.is_empty() {
                return false;
            }
            let run = TextRun {
                len: text.len(),
                font: gpui::Font {
                    family: font.family(),
                    features: font.features(),
                    fallbacks: None,
                    weight: font.weight,
                    style: font.style(),
                },
                color: gpui::Hsla::default(),
                background_color: None,
                underline: None,
                strikethrough: None,
            };
            window
                .text_system()
                .shape_text(SharedString::from(text.to_owned()), px(font.size), &[run], Some(px(width)), None)
                .map(|lines| visual_line_count(&lines) > limit)
                .unwrap_or(false)
        };
        let expandable = exceeds(
            &item.caption_title,
            Font::system_weight(17.0, FontWeight::SEMIBOLD),
            MediaViewerScreenMetrics::CAPTION_TITLE_LINE_LIMIT,
        ) || exceeds(&item.caption, Font::system_size(15.0), MediaViewerScreenMetrics::CAPTION_LINE_LIMIT);
        *self.caption_expandable_memo.borrow_mut() = Some((item.caption_title.clone(), item.caption.clone(), width, expandable));
        expandable
    }

    fn set_caption_overlay(&mut self, shows: bool, cx: &mut Context<Self>) {
        self.shows_caption_overlay = shows;
        let spec = if shows {
            spring(MediaViewerScreenMetrics::CAPTION_SHOW_RESPONSE, MediaViewerScreenMetrics::CAPTION_SHOW_DAMPING)
        } else {
            ease_out(MediaViewerScreenMetrics::CAPTION_HIDE)
        };
        self.caption_overlay_progress.set_in(if shows { 1.0 } else { 0.0 }, spec, Self::now(), cx);
        cx.notify();
    }
}

fn icon(name: &str, side: f32) -> impl IntoElement {
    div().flex().items_center().justify_center().children(IconUi::sized(name, px(side), white(), true))
}

fn element_id(prefix: &str, id: &str) -> ElementId {
    ElementId::Name(SharedString::from(format!("{prefix}-{id}")))
}

fn smoothstep(x: f32) -> f32 {
    let t = x.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn interpolate_rect(source: Bounds<Pixels>, target: Bounds<Pixels>, t: f32) -> Bounds<Pixels> {
    let lerp = |a: Pixels, b: Pixels| a + (b - a) * t;
    Bounds::new(
        point(lerp(source.origin.x, target.origin.x), lerp(source.origin.y, target.origin.y)),
        size(lerp(source.size.width, target.size.width), lerp(source.size.height, target.size.height)),
    )
}

fn screen_rect_to_overlay_local(screen: Bounds<Pixels>, overlay_screen: Bounds<Pixels>) -> Bounds<Pixels> {
    if overlay_screen.size.width <= px(1.0) || overlay_screen.size.height <= px(1.0) {
        return screen;
    }
    Bounds::new(
        point(screen.origin.x - overlay_screen.origin.x, screen.origin.y - overlay_screen.origin.y),
        screen.size,
    )
}

impl MediaViewerScreen {
    fn resolved_hero_ui_image(&self, item: &MediaViewerGalleryItem, cx: &App) -> Option<CleverImage> {
        let state = self.state.read(cx);
        if let Some(warm) = state.hero_warm_image(&item.id) {
            return Some(warm);
        }
        let raw = state.hero_skin().map(|skin| skin.display_source.clone()).unwrap_or_else(|| item.display_source());
        MediaViewerImageMemoryCache::image(&raw, SaiAccountCacheScope::active_user_id().as_deref(), item.image_cache_policy)
    }

    fn resolved_aspect_ratio(item: &MediaViewerGalleryItem, cached: Option<&CleverImage>) -> f32 {
        if item.width > 1.0 && item.height > 1.0 {
            return item.width / item.height;
        }
        if let Some(cached) = cached {
            let frame = cached.size(0);
            let (w, h) = (frame.width.0 as f32, frame.height.0 as f32);
            if w > 1.0 && h > 1.0 {
                return w / h;
            }
        }
        1.0
    }

    fn hero_flight_layer(
        &self,
        viewport: Size<Pixels>,
        source_screen: Bounds<Pixels>,
        item: &MediaViewerGalleryItem,
        window: &Window,
        cx: &App,
    ) -> AnyElement {
        let now = Self::now();
        let source_local = screen_rect_to_overlay_local(source_screen, self.overlay_screen_frame);
        let hero_skin = self.state.read(cx).hero_skin().cloned();
        let url = hero_skin.as_ref().map(|skin| skin.display_source.clone()).unwrap_or_else(|| item.display_source());
        let cached = self.resolved_hero_ui_image(item, cx);
        let aspect = Self::resolved_aspect_ratio(item, cached.as_ref());
        let target = MediaAspectFit::centered_fit_rect_aspect(aspect, viewport);

        let t = self.open_progress.frame_value(now, window).clamp(0.0, 1.0);
        let u = smoothstep(t);
        let rect = interpolate_rect(source_local, target, u);
        let scale = self.content_scale.frame_value(now, window);
        let drag = self.drag_translation.frame_value(now, window);
        let w = f32::from(rect.size.width).max(1.0);
        let h = f32::from(rect.size.height).max(1.0);
        let source_radii = hero_skin
            .as_ref()
            .and_then(|skin| skin.source_media_radii)
            .unwrap_or(MessageBubbleRadii::uniform(px(14.0)));
        let fade = 1.0 - u;
        let source_decoration_opacity = (1.0 - u / 0.3).clamp(0.0, 1.0);
        let uses_letterbox = hero_skin.as_ref().is_some_and(|skin| skin.uses_fit_foreground);
        let letterbox_blur = if uses_letterbox { 1.0 - u } else { 0.0 };
        let fit_opacity = if uses_letterbox { 1.0 } else { u };
        let r#box = size(px(w), px(h));

        let mut visual = div()
            .absolute()
            .inset_0()
            .rounded_tl(source_radii.top_leading * fade)
            .rounded_tr(source_radii.top_trailing * fade)
            .rounded_bl(source_radii.bottom_leading * fade)
            .rounded_br(source_radii.bottom_trailing * fade)
            .overflow_hidden()
            .child(
                div().absolute().inset_0().opacity(fit_opacity).child(
                    MediaLetterboxPhoto::new(element_id("media-viewer-hero-fit", &item.id), url.clone(), r#box)
                        .cached_image(cached.clone())
                        .blur_opacity(letterbox_blur)
                        .persist_to_disk(item.persist_media_to_disk)
                        .cache_policy(item.image_cache_policy),
                ),
            );
        if !uses_letterbox {
            let fill: AnyElement = match cached.clone() {
                Some(cached) => img(cached).size_full().object_fit(ObjectFit::Cover).into_any_element(),
                None => CleverImageUi::new(url.clone())
                    .id(element_id("media-viewer-hero-fill", &item.id))
                    .content_mode(ContentMode::Fill)
                    .corner_radius(0.0)
                    .persist_to_disk(item.persist_media_to_disk)
                    .cache_policy(item.image_cache_policy)
                    .into_any_element(),
            };
            visual = visual.child(div().absolute().inset_0().bg(black()).opacity(1.0 - fit_opacity).child(fill));
        }

        let mut flight = div().absolute().inset_0().child(visual);
        if let Some(tail) = hero_skin.as_ref().and_then(|skin| skin.media_tail) {
            let gradient_height = h.min(64.0);
            flight = flight.child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom_0()
                    .h(px(gradient_height))
                    .opacity(source_decoration_opacity)
                    .bg(linear_gradient(
                        180.0,
                        linear_color_stop(black().opacity(0.0), 0.0),
                        linear_color_stop(
                            black().opacity(MessageMediaTailGeometry::TERMINAL_OVERLAY_BLACK_OPACITY),
                            1.0,
                        ),
                    )),
            );
            let cap_width = MessageMediaTailGeometry::extension_width(tail.side);
            let cap_height = MessageMediaTailGeometry::CAP_HEIGHT;
            let cap_x = if tail.side == rise_ui::message_bubble_geometry::MessageBubbleTailSide::Leading {
                -cap_width
            } else {
                px(w)
            };
            let shape = MessageMediaTailCapShape { side: tail.side, radii: tail.radii };
            let fill = tail.color.color();
            flight = flight.child(
                div()
                    .absolute()
                    .left(cap_x)
                    .top(px(h) - cap_height)
                    .w(cap_width)
                    .h(cap_height)
                    .opacity(source_decoration_opacity)
                    .child(canvas(|_, _, _| {}, move |bounds, _, window, _| shape.paint(bounds, fill, window)).size_full()),
            );
        }

        let scaled_w = w * scale;
        let scaled_h = h * scale;
        let center_x = f32::from(rect.origin.x) + w / 2.0 + f32::from(drag.width);
        let center_y = f32::from(rect.origin.y) + h / 2.0 + f32::from(drag.height);
        let shadow = gpui::BoxShadow {
            color: black().opacity(0.28 * fade),
            offset: point(px(0.0), px(6.0 * fade)),
            blur_radius: px(14.0 * fade),
            spread_radius: px(0.0),
            inset: false,
        };
        div()
            .absolute()
            .left(px(center_x - scaled_w / 2.0))
            .top(px(center_y - scaled_h / 2.0))
            .w(px(scaled_w))
            .h(px(scaled_h))
            .shadow(vec![shadow])
            .child(flight)
            .into_any_element()
    }

    fn page_content(
        &self,
        item: &MediaViewerGalleryItem,
        is_current: bool,
        page_size: Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cached = MediaViewerImageMemoryCache::image(
            &item.display_source(),
            SaiAccountCacheScope::active_user_id().as_deref(),
            item.image_cache_policy,
        )
        .or_else(|| self.seed_pager_image(item, cx))
        .or_else(|| Self::chat_row_preview_image(item));
        let this = cx.entity().downgrade();
        let on_zoom_changed = {
            let this = this.clone();
            Rc::new(move |zoomed: bool, cx: &mut App| {
                if is_current {
                    this.update(cx, |this, cx| {
                        this.media_zoomed = zoomed;
                        cx.notify();
                    })
                    .ok();
                }
            })
        };
        let on_single_tap = {
            let this = this.clone();
            Rc::new(move |_: &mut Window, cx: &mut App| {
                this.update(cx, |this, cx| this.toggle_chrome(cx)).ok();
            })
        };
        let animation_rect =
            MediaAspectFit::centered_fit_rect_aspect(Self::resolved_aspect_ratio(item, cached.as_ref()), page_size);
        match item.kind {
            MediaViewerGalleryItemKind::Image => MediaZoomablePage {
                id: element_id("media-viewer-page", &item.id),
                source: item.display_source(),
                seed_image: cached,
                is_current,
                persist_to_disk: item.persist_media_to_disk,
                cache_policy: item.image_cache_policy,
                on_zoom_changed,
                on_single_tap,
            }
            .into_any_element(),
            MediaViewerGalleryItemKind::Animation => {
                let url = if trimming_whitespaces_and_newlines(&item.url).is_empty() { item.display_source() } else { item.url.clone() };
                div()
                    .id(element_id("media-viewer-animation-page", &item.id))
                    .relative()
                    .size_full()
                    .child(
                        div()
                            .absolute()
                            .left(animation_rect.origin.x)
                            .top(animation_rect.origin.y)
                            .w(animation_rect.size.width)
                            .h(animation_rect.size.height)
                            .child(
                                AnimatedGifView::new(element_id("media-viewer-animation", &item.id), url)
                                    .content_mode(AnimatedGifContentMode::ScaleAspectFit)
                                    .playback_kind(AnimatedMediaPlaybackKind::OpenedAnimation)
                                    .persist_to_disk(item.persist_media_to_disk)
                                    .cache_policy(item.image_cache_policy),
                            ),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_chrome(cx)))
                    .into_any_element()
            }
            MediaViewerGalleryItemKind::Video => {
                let is_playing = self.video_controller.read(cx).is_playing();
                let playback_control: Option<MediaZoomViewBuilder> =
                    (is_current && self.shows_center_playback_control(cx)).then(|| {
                        let controller = self.video_controller.clone();
                        Rc::new(move |_: &mut Window, cx: &mut App| {
                            Self::video_center_playback_control(&controller, cx).into_any_element()
                        }) as MediaZoomViewBuilder
                    });
                let controller = self.video_controller.clone();
                let page_count = self.state.read(cx).items().len();
                let page_index = self.state.read(cx).current_index();
                MediaViewerVideoPlayer {
                    id: element_id("media-viewer-video", &item.id),
                    url: item.url.clone(),
                    thumbnail_url: item.thumbnail_url.clone(),
                    aspect_ratio: item.aspect_ratio(),
                    is_current,
                    fills_screen: self.video_fills_screen,
                    controller: self.video_controller.clone(),
                    persist_poster_to_disk: item.persist_media_to_disk,
                    poster_cache_policy: item.image_cache_policy,
                    playback_control,
                    playback_control_label: if is_playing { tr("media_viewer_pause") } else { tr("media_viewer_play") },
                    on_playback_toggle: Rc::new(move |_: &mut Window, cx: &mut App| {
                        if is_current {
                            controller.update(cx, |c, cx| c.toggle_playback(cx));
                        }
                    }),
                    on_zoom_changed,
                    on_single_tap,
                    has_previous_page: page_index > 0,
                    has_next_page: page_index + 1 < page_count,
                }
                .into_any_element()
            }
        }
    }

    fn video_center_playback_control(controller: &Entity<MediaViewerVideoController>, cx: &App) -> impl IntoElement {
        let is_playing = controller.read(cx).is_playing();
        let label = if is_playing { tr("media_viewer_pause") } else { tr("media_viewer_play") };
        let controller = controller.clone();
        LiquidGlassButton::new("media-viewer-center-playback", LiquidGlassButtonShape::Circle)
            .size(MediaViewerVideoPlayer::PLAYBACK_CONTROL_SIZE)
            .blur_intensity(Some(0.76))
            .accessibility_label(Some(label))
            .on_press(move |_, cx| controller.update(cx, |c, cx| c.toggle_playback(cx)))
            .child(
                div()
                    .ml(px(if is_playing { 0.0 } else { 3.0 }))
                    .child(icon(if is_playing { "pause.fill" } else { "play.fill" }, 42.0)),
            )
    }

    fn pager_layer(&self, viewport: Size<Pixels>, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let now = Self::now();
        let w = f32::from(viewport.width).max(1.0);
        let h = f32::from(viewport.height).max(1.0);
        let offset = self.pager_scroll.offset.frame_value(now, window);
        let scale = self.content_scale.frame_value(now, window);
        let drag = self.drag_translation.frame_value(now, window);
        let (items, current_index) = {
            let state = self.state.read(cx);
            (state.items().to_vec(), state.current_index())
        };
        let first = offset.floor().max(0.0) as usize;
        let last = (offset.ceil() as usize).min(items.len().saturating_sub(1));
        let overflow_items = self.current_gallery_item(cx).map(|item| self.overflow_menu_items(&item, cx));
        let forward = Self::forward_sign();
        let mut layer = div().absolute().inset_0().overflow_hidden();
        for (index, item) in items.iter().enumerate().take(last + 1).skip(first) {
            let left = (index as f32 - offset) * w * forward;
            let page_w = w * scale;
            let page_h = h * scale;
            let mut page = div()
                .id(element_id("media-viewer-pager-slot", &item.id))
                .absolute()
                .left(px(left + (w - page_w) / 2.0 + f32::from(drag.width)))
                .top(px((h - page_h) / 2.0 + f32::from(drag.height)))
                .w(px(page_w))
                .h(px(page_h))
                .child(self.page_content(item, index == current_index, size(px(page_w), px(page_h)), cx))
                .gesture(element_id("media-viewer-dismiss-drag", &item.id), self.vertical_dismiss_gesture(cx))
                .into_any_element();
            if index == current_index
                && let Some(menu_items) = overflow_items.clone()
            {
                page = page.context_menu(element_id("media-viewer-page-menu", &item.id), menu_items).into_any_element();
            }
            layer = layer.child(page);
        }
        layer.into_any_element()
    }

    fn top_bar(&self, safe_top: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let item = self.current_gallery_item(cx)?;
        let has_title = !trimming_whitespaces_and_newlines(&item.sender_name).is_empty();
        let has_date = item.created_at > 0.0;
        let back = LiquidGlassButton::new("media-viewer-back", LiquidGlassButtonShape::Circle)
            .size(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE)
            .accessibility_label(Some(tr("back")))
            .on_press({
                let this = cx.entity().downgrade();
                move |_, cx| {
                    this.update(cx, |this, cx| this.animate_close(None, cx)).ok();
                }
            })
            .child(icon("chevron.backward", 16.0));
        let mut row = div().flex().flex_row().items_center().gap(px(12.0)).child(back);
        row = row.child(div().flex_1().min_w(px(4.0)));
        if has_title || has_date {
            let mut column = div().flex().flex_col().items_center();
            if has_title {
                column = column.child(
                    div()
                        .swiftui_font(Font::system_weight(18.0, FontWeight::SEMIBOLD))
                        .text_color(white())
                        .whitespace_nowrap()
                        .line_clamp(1)
                        .text_ellipsis()
                        .child(item.sender_name.clone()),
                );
            }
            if has_date {
                let font = if has_title {
                    Font::system_weight(14.0, FontWeight::NORMAL)
                } else {
                    Font::system_weight(16.0, FontWeight::SEMIBOLD)
                };
                column = column.child(
                    div()
                        .swiftui_font(font)
                        .text_color(white().opacity(if has_title { 0.58 } else { 1.0 }))
                        .whitespace_nowrap()
                        .line_clamp(1)
                        .text_ellipsis()
                        .child(Self::format_header_date(item.created_at)),
                );
            }
            if !(has_title && has_date) {
                column = column.min_h(px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE - 14.0)).justify_center();
            }
            row = row.child(
                div()
                    .px(px(24.0))
                    .py(px(7.0))
                    .max_w(px(260.0))
                    .child(column)
                    .liquid_chrome_capsule_with(0.12, true, Some(0.84), false),
            );
        }
        row = row.child(div().flex_1().min_w(px(4.0)));
        row = row.child(
            LiquidGlassButton::new("media-viewer-full-screen", LiquidGlassButtonShape::Circle)
                .size(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE)
                .on_press(|window, _| window.toggle_fullscreen())
                .child(icon("arrow.up.left.and.arrow.down.right", 15.0)),
        );
        if self.overflow_menu_has_items(&item, cx) {
            row = row.child(self.overflow_menu(&item, cx));
        } else {
            let side = px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE);
            row = row.child(div().w(side).h(side));
        }
        let mut bar = div().flex().flex_col().gap(px(6.0)).child(row);
        if self.state.read(cx).items().len() > 1 {
            bar = bar.child(div().flex().justify_center().child(self.page_badge(cx)));
        }
        let covered = MediaViewerYieldsToTopNotifier { enabled: self.route == MediaViewerPresentationRoute::Inline }.covered(cx);
        Some(
            div()
                .w_full()
                .px(px(16.0))
                .pt(px(safe_top + 8.0))
                .pb(px(12.0))
                .bg(linear_gradient(
                    180.0,
                    linear_color_stop(black().opacity(0.5), 0.0),
                    linear_color_stop(black().opacity(0.0), 1.0),
                ))
                .opacity(if covered { 0.0 } else { 1.0 })
                .child(bar)
                .into_any_element(),
        )
    }

    fn page_badge(&self, cx: &App) -> impl IntoElement {
        let counter = self.gallery_counter(cx);
        let text = counter
            .map(|(position, total)| format!("{} {} {}", position, tr("rtc_of"), total))
            .unwrap_or_default();
        let opacity = self.page_badge_opacity.value(Self::now());
        div().opacity(opacity).child(
            div()
                .px(px(12.0))
                .py(px(5.0))
                .swiftui_font(Font::system(12.0, FontWeight::SEMIBOLD, FontDesign::Rounded).monospaced_digit())
                .text_color(white().opacity(0.88))
                .child(text)
                .liquid_chrome_capsule_with(0.12, true, Some(0.58), false),
        )
    }

    fn overflow_menu(&self, item: &MediaViewerGalleryItem, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity();
        let is_presented = Binding::entity(
            this.clone(),
            |screen: &Self| screen.is_overflow_menu_presented,
            |screen: &mut Self, value: bool, _: &mut Context<Self>| screen.is_overflow_menu_presented = value,
        );
        let weak = this.downgrade();
        let actions: HoldContextMenuPresentationActions = {
            let weak = weak.clone();
            let row = item.clone();
            Rc::new(move |cx: &App| {
                weak.upgrade().map(|screen| screen.read(cx).overflow_actions(&row, cx)).unwrap_or_default()
            })
        };
        let on_action: HoldContextMenuPresentationOnAction = {
            let weak = weak.clone();
            let row = item.clone();
            Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
                weak.update(cx, |screen, cx| screen.handle_overflow_action(&id, row.clone(), cx)).ok();
            })
        };
        let source: HoldContextMenuTapSource =
            Rc::new(|_: &mut Window, _: &mut App| Self::overflow_glyph().into_any_element());
        let menu = plain("media-viewer-overflow")
            .on_press(move |_, cx| {
                weak.update(cx, |screen, cx| {
                    screen.is_overflow_menu_presented = true;
                    cx.notify();
                })
                .ok();
            })
            .child(Self::overflow_glyph())
            .tap_context_menu(
                "media-viewer-overflow-menu",
                is_presented,
                HoldContextMenuAlignment::Trailing,
                true,
                false,
                false,
                None,
                actions,
                on_action,
                source,
            );
        div().id("media-viewer-overflow-hit").occlude().child(menu).into_any_element()
    }

    fn overflow_glyph() -> PageHeaderToolbarSlot {
        PageHeaderToolbarSlot::new(icon("ellipsis", 17.0))
    }

    fn overflow_actions(&self, item: &MediaViewerGalleryItem, cx: &App) -> Vec<HoldContextMenuAction> {
        let state = self.state.read(cx);
        let mut actions = Vec::new();
        if Self::is_savable(item) {
            actions.push(
                HoldContextMenuAction::new(OverflowActionID::SAVE, tr("ctx_save"))
                    .system_image("square.and.arrow.down"),
            );
            if item.post_download.is_none() {
                actions.push(
                    HoldContextMenuAction::new(OverflowActionID::SAVE_TO_PHOTOS, tr("media_viewer_save_to_gallery"))
                        .system_image("photo.on.rectangle"),
                );
            }
        }
        if state.can_set_as_main(item) {
            actions.push(
                HoldContextMenuAction::new(OverflowActionID::SET_AS_MAIN, tr("profile_photos_set_as_main"))
                    .system_image("person.crop.circle.badge.checkmark"),
            );
        }
        if state.can_delete_profile_photo(item) {
            actions.push(
                HoldContextMenuAction::new(OverflowActionID::DELETE_PROFILE_PHOTO, tr("profile_photos_delete"))
                    .system_image("trash")
                    .destructive(true),
            );
        }
        if state.can_show_in_context(item) {
            actions.push(
                HoldContextMenuAction::new(OverflowActionID::SHOW_IN_CONTEXT, tr("ctx_show_in_chat"))
                    .system_image("bubble.left.and.bubble.right"),
            );
        }
        if state.can_reply(item) {
            actions.push(
                HoldContextMenuAction::new(OverflowActionID::REPLY, tr("ctx_reply"))
                    .system_image("arrowshape.turn.up.left"),
            );
        }
        if item.can_delete {
            if item.deletes_from_favourites || (item.can_delete_for_me && item.can_delete_for_everyone) {
                actions.push(
                    HoldContextMenuAction::new(OverflowActionID::DELETE, self.delete_title(item, cx))
                        .system_image("trash")
                        .destructive(true)
                        .submenu(Self::delete_mode_actions(item)),
                );
            } else {
                let id = if item.can_delete_for_everyone {
                    OverflowActionID::DELETE_FOR_EVERYONE
                } else {
                    OverflowActionID::DELETE_FOR_ME
                };
                actions.push(
                    HoldContextMenuAction::new(id, self.delete_title(item, cx)).system_image("trash").destructive(true),
                );
            }
        }
        if state.can_report(item) {
            actions.push(
                HoldContextMenuAction::new(OverflowActionID::REPORT, tr("media_viewer_report_post"))
                    .system_image("exclamationmark.bubble")
                    .destructive(true),
            );
        }
        actions
    }

    fn handle_overflow_action(&mut self, id: &str, item: MediaViewerGalleryItem, cx: &mut Context<Self>) {
        tracing::warn!(
            target: "riseonly::X2255",
            action = %id,
            interactive = self.is_interactive(cx),
            closing = ?self.closing_presentation_id,
            "viewer overflow action"
        );
        match id {
            OverflowActionID::SAVE => self.save_current_media_as_file(cx),
            OverflowActionID::SAVE_TO_PHOTOS => self.save_current_media(cx),
            OverflowActionID::SET_AS_MAIN => MediaViewerPresentationState::set_as_main(&self.state, &item, cx),
            OverflowActionID::DELETE_PROFILE_PHOTO => {
                MediaViewerPresentationState::delete_profile_photo(&self.state, &item, cx)
            }
            OverflowActionID::SHOW_IN_CONTEXT => {
                let action = self.state.read(cx).show_in_context_action(&item);
                self.perform_context_action(action, cx);
            }
            OverflowActionID::REPLY => {
                let action = self.state.read(cx).reply_action(&item);
                self.perform_context_action(action, cx);
            }
            OverflowActionID::DELETE_FOR_EVERYONE => self.perform_delete(item, MediaViewerDeleteMode::ForEveryone, cx),
            OverflowActionID::DELETE_FOR_ME => self.perform_delete(item, MediaViewerDeleteMode::ForMe, cx),
            OverflowActionID::DELETE_FROM_FAVOURITES => {
                let mode = if item.can_delete_for_everyone {
                    MediaViewerDeleteMode::ForEveryone
                } else {
                    MediaViewerDeleteMode::ForMe
                };
                self.perform_delete(item, mode, cx);
            }
            OverflowActionID::REPORT => MediaViewerPresentationState::report(&self.state, &item, cx),
            _ => {}
        }
    }

    fn overflow_menu_items(&self, item: &MediaViewerGalleryItem, cx: &mut Context<Self>) -> Vec<ContextMenuItem> {
        let mut flat = Vec::new();
        for action in self.overflow_actions(item, cx) {
            match action.submenu.clone() {
                Some(submenu) => flat.extend(submenu),
                None => flat.push(action),
            }
        }
        let this = cx.entity().downgrade();
        flat.into_iter()
            .enumerate()
            .map(|(index, action)| {
                let this = this.clone();
                let row = item.clone();
                let id = action.id.clone();
                ContextMenuItem::new(index as i64, action.title, action.system_image, action.id, move |_, cx| {
                    this.update(cx, |screen, cx| screen.handle_overflow_action(&id, row.clone(), cx)).ok();
                })
                .destructive(action.destructive)
            })
            .collect()
    }

    fn delete_mode_actions(item: &MediaViewerGalleryItem) -> Vec<HoldContextMenuAction> {
        if item.deletes_from_favourites {
            return vec![
                HoldContextMenuAction::new(OverflowActionID::DELETE_FROM_FAVOURITES, tr("ctx_delete_from_favourites"))
                    .system_image("bookmark.slash")
                    .destructive(true),
            ];
        }
        let mut actions = Vec::new();
        if item.can_delete_for_everyone {
            actions.push(
                HoldContextMenuAction::new(OverflowActionID::DELETE_FOR_EVERYONE, tr("ctx_delete_for_everyone"))
                    .system_image("trash")
                    .destructive(true),
            );
        }
        if item.can_delete_for_me {
            actions.push(
                HoldContextMenuAction::new(OverflowActionID::DELETE_FOR_ME, tr("ctx_delete_for_me"))
                    .system_image("trash")
                    .destructive(true),
            );
        }
        actions
    }

    fn bottom_bar(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let item = self.current_gallery_item(cx)?;
        let shows_bottom_bar = self.state.read(cx).chrome().shows_bottom_bar;
        let shows_filmstrip = self.shows_filmstrip(cx);
        if !(shows_bottom_bar || item.kind == MediaViewerGalleryItemKind::Video || shows_filmstrip) {
            self.bottom_bar_height.set(0.0);
            return None;
        }
        let mut column = div().flex().flex_col().gap(px(12.0));
        if shows_bottom_bar && Self::has_caption(&item) {
            column = column.child(self.caption_preview(&item, window, cx));
        }
        if shows_filmstrip && let Some((key, rail)) = &self.filmstrip {
            column = column.child(
                div()
                    .id(SharedString::from(format!("media-viewer-filmstrip-{key}")))
                    .h(MediaViewerFilmstripMetrics::HEIGHT)
                    .occlude()
                    .child(
                        AnyView::from(rail.clone())
                            .cached(StyleRefinement::default().w_full().h(MediaViewerFilmstripMetrics::HEIGHT)),
                    ),
            );
        }
        if item.kind == MediaViewerGalleryItemKind::Video {
            column = column.child(
                div().px(px(16.0)).child(
                    AnyView::from(self.video_scrubber.clone())
                        .cached(StyleRefinement::default().w_full().h(px(MediaViewerVideoScrubber::HEIGHT))),
                ),
            );
        }
        if shows_bottom_bar {
            column = column.child(div().px(px(18.0)).child(self.bottom_action_bar(&item, window, cx)));
        }
        let height_slot = Rc::clone(&self.bottom_bar_height);
        Some(
            div()
                .relative()
                .w_full()
                .pt(px(18.0))
                .pb(px(12.0_f32.max(MediaViewerSafeArea::bottom_inset())))
                .bg(linear_gradient(
                    180.0,
                    linear_color_stop(black().opacity(0.0), 0.0),
                    linear_color_stop(black().opacity(0.72), 1.0),
                ))
                .child(column)
                .child(measure("media-viewer-bottom-bar-measure", move |bounds, _, _| {
                    height_slot.set(f32::from(bounds.size.height));
                }))
                .into_any_element(),
        )
    }

    fn bottom_action_bar(&self, item: &MediaViewerGalleryItem, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let side = px(MediaViewerScreenMetrics::ACTION_BUTTON_SIZE);
        let state = self.state.read(cx);
        let mut row = div().flex().flex_row().items_center();
        if state.can_forward(item) {
            let label = if state.album_item_count(item).is_none() { tr("ctx_share") } else { tr("ctx_forward_album") };
            let row_item = item.clone();
            let host = self.state.clone();
            row = row.child(
                LiquidGlassButton::new("media-viewer-forward", LiquidGlassButtonShape::Circle)
                    .size(MediaViewerScreenMetrics::ACTION_BUTTON_SIZE)
                    .accessibility_label(Some(label))
                    .on_press(move |_, cx| MediaViewerPresentationState::forward(&host, &row_item, cx))
                    .child(icon("arrowshape.turn.up.right", 24.0)),
            );
        } else {
            row = row.child(div().w(side).h(side));
        }
        row = row.child(div().flex_1().min_w(px(10.0)));
        if item.kind == MediaViewerGalleryItemKind::Video {
            row = row.child(self.video_settings_pill(window, cx));
        } else if Self::is_savable(item) {
            let this = cx.entity().downgrade();
            row = row.child(
                LiquidGlassButton::new("media-viewer-save", LiquidGlassButtonShape::Circle)
                    .size(MediaViewerScreenMetrics::ACTION_BUTTON_SIZE)
                    .accessibility_label(Some(tr("ctx_save")))
                    .on_press(move |_, cx| {
                        this.update(cx, |this, cx| this.save_current_media_as_file(cx)).ok();
                    })
                    .child(icon("arrow.down.to.line.compact", 22.0)),
            );
        }
        row = row.child(div().flex_1().min_w(px(10.0)));
        let (accessory, can_delete_profile_photo) = {
            let app: &App = cx;
            let state = self.state.read(app);
            let accessory = state.accessory().filter(|accessory| (accessory.applies)(item, app)).cloned();
            (accessory, state.can_delete_profile_photo(item))
        };
        if let Some(accessory) = accessory {
            let row_item = item.clone();
            row = row.child(
                LiquidGlassButton::new("media-viewer-accessory", LiquidGlassButtonShape::Circle)
                    .size(MediaViewerScreenMetrics::ACTION_BUTTON_SIZE)
                    .accessibility_label(Some(accessory.accessibility_label.clone()))
                    .on_press(move |_, cx| (accessory.perform)(&row_item, cx))
                    .child(icon(&accessory.system_image, 22.0)),
            );
        } else if item.can_delete {
            row = row.child(self.delete_control(item, cx));
        } else if can_delete_profile_photo {
            let row_item = item.clone();
            let host = self.state.clone();
            let error = ThemeStore::shared(cx).read(cx).current_theme().semantic.error_100;
            let button = plain("media-viewer-delete-profile-photo")
                .w(side)
                .h(side)
                .flex()
                .items_center()
                .justify_center()
                .on_press(move |_, cx| MediaViewerPresentationState::delete_profile_photo(&host, &row_item, cx))
                .children(IconUi::sized("trash", px(24.0), error, false));
            row = row.child(
                div()
                    .id("media-viewer-delete-profile-photo-hit")
                    .occlude()
                    .tooltip_label(tr("profile_photos_delete"))
                    .child(div().child(button).liquid_chrome_circle(Some(0.84))),
            );
        } else {
            row = row.child(div().w(side).h(side));
        }
        row.into_any_element()
    }

    fn delete_glyph() -> gpui::Div {
        let side = px(MediaViewerScreenMetrics::ACTION_BUTTON_SIZE);
        div().w(side).h(side).flex().items_center().justify_center().child(icon("trash", 24.0))
    }

    fn delete_control(&self, item: &MediaViewerGalleryItem, cx: &mut Context<Self>) -> AnyElement {
        let title = self.delete_title(item, cx);
        if item.can_delete_for_me || item.can_delete_for_everyone {
            let this = cx.entity();
            let is_presented = Binding::entity(
                this.clone(),
                |screen: &Self| screen.is_delete_menu_presented,
                |screen: &mut Self, value: bool, _: &mut Context<Self>| screen.is_delete_menu_presented = value,
            );
            let weak = this.downgrade();
            let actions: HoldContextMenuPresentationActions = {
                let row = item.clone();
                Rc::new(move |_: &App| Self::delete_mode_actions(&row))
            };
            let on_action: HoldContextMenuPresentationOnAction = {
                let weak = weak.clone();
                let row = item.clone();
                Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
                    weak.update(cx, |screen, cx| screen.handle_overflow_action(&id, row.clone(), cx)).ok();
                })
            };
            let source: HoldContextMenuTapSource =
                Rc::new(|_: &mut Window, _: &mut App| div().child(Self::delete_glyph()).liquid_chrome_circle(Some(0.84)).into_any_element());
            let button = plain("media-viewer-delete")
                .on_press(move |_, cx| {
                    weak.update(cx, |screen, cx| {
                        screen.is_delete_menu_presented = true;
                        cx.notify();
                    })
                    .ok();
                })
                .child(Self::delete_glyph());
            let menu = div().child(button).liquid_chrome_circle(Some(0.84)).tap_context_menu(
                "media-viewer-delete-menu",
                is_presented,
                HoldContextMenuAlignment::Trailing,
                true,
                false,
                false,
                None,
                actions,
                on_action,
                source,
            );
            return div().id("media-viewer-delete-hit").occlude().tooltip_label(title).child(menu).into_any_element();
        }
        let this = cx.entity().downgrade();
        let row = item.clone();
        let mode = if item.can_delete_for_everyone { MediaViewerDeleteMode::ForEveryone } else { MediaViewerDeleteMode::ForMe };
        LiquidGlassButton::new("media-viewer-delete", LiquidGlassButtonShape::Circle)
            .size(MediaViewerScreenMetrics::ACTION_BUTTON_SIZE)
            .accessibility_label(Some(title))
            .on_press(move |_, cx| {
                this.update(cx, |screen, cx| screen.perform_delete(row.clone(), mode, cx)).ok();
            })
            .child(icon("trash", 24.0))
            .into_any_element()
    }

    fn video_settings_pill(&self, _window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let fills = self.video_fills_screen;
        let divider = || div().w(px(0.5)).h(px(24.0)).bg(white().opacity(0.16));
        let fill_toggle = div()
            .id("media-viewer-video-fill")
            .w(px(52.0))
            .h(px(52.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .child(icon(
                if fills { "rectangle.arrowtriangle.2.outward" } else { "rectangle.arrowtriangle.2.inward" },
                20.0,
            ))
            .on_click(cx.listener(|this: &mut Self, _, _, cx| {
                this.video_fills_screen = !this.video_fills_screen;
                cx.notify();
            }));
        div()
            .flex()
            .flex_row()
            .items_center()
            .child(fill_toggle)
            .child(divider())
            .child(self.playback_speed_menu(cx))
            .child(divider())
            .child(MediaViewerLoopToggle { controller: self.video_controller.clone() })
            .liquid_chrome_capsule_with(0.12, true, Some(0.84), false)
    }

    fn playback_speed_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity();
        let is_presented = Binding::entity(
            this.clone(),
            |screen: &Self| screen.is_speed_menu_presented,
            |screen: &mut Self, value: bool, _: &mut Context<Self>| screen.is_speed_menu_presented = value,
        );
        let weak = this.downgrade();
        let actions: HoldContextMenuPresentationActions = {
            let controller = self.video_controller.clone();
            Rc::new(move |cx: &App| Self::playback_rate_actions(controller.read(cx).playback_rate()))
        };
        let on_action: HoldContextMenuPresentationOnAction = {
            let controller = self.video_controller.clone();
            Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
                let Some(rate) =
                    MediaViewerScreenMetrics::PLAYBACK_RATES.into_iter().find(|rate| Self::playback_rate_action_id(*rate) == id)
                else {
                    return;
                };
                controller.update(cx, |c, cx| c.set_playback_rate(rate, cx));
            })
        };
        let source: HoldContextMenuTapSource =
            Rc::new(|_: &mut Window, _: &mut App| Self::playback_speed_glyph().into_any_element());
        let menu = plain("media-viewer-speed")
            .on_press(move |_, cx| {
                weak.update(cx, |screen, cx| {
                    screen.is_speed_menu_presented = true;
                    cx.notify();
                })
                .ok();
            })
            .child(Self::playback_speed_glyph())
            .tap_context_menu(
                "media-viewer-speed-menu",
                is_presented,
                HoldContextMenuAlignment::Trailing,
                true,
                false,
                false,
                None,
                actions,
                on_action,
                source,
            );
        div()
            .id("media-viewer-speed-hit")
            .occlude()
            .tooltip_label(tr("media_viewer_playback_speed"))
            .child(menu)
            .into_any_element()
    }

    fn playback_speed_glyph() -> gpui::Div {
        div().w(px(52.0)).h(px(52.0)).flex().items_center().justify_center().child(icon("gearshape", 21.0))
    }

    fn playback_rate_actions(current: f32) -> Vec<HoldContextMenuAction> {
        MediaViewerScreenMetrics::PLAYBACK_RATES
            .into_iter()
            .map(|rate| {
                HoldContextMenuAction::new(Self::playback_rate_action_id(rate), Self::playback_rate_title(rate))
                    .checked((current - rate).abs() < 0.01)
            })
            .collect()
    }

    fn playback_rate_action_id(rate: f32) -> String {
        format!("rate_{rate}")
    }

    fn playback_rate_title(rate: f32) -> String {
        if rate == 1.0 {
            tr("media_viewer_speed_normal")
        } else {
            format!("{}x", rate)
        }
    }

    fn caption_preview(&self, item: &MediaViewerGalleryItem, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let expandable = self.caption_is_expandable(item, window);
        let mut column = div()
            .flex()
            .flex_col()
            .gap(px(5.0))
            .w_full()
            .px(px(MediaViewerScreenMetrics::CAPTION_HORIZONTAL_PADDING))
            .text_color(white());
        if !item.caption_title.is_empty() {
            column = column.child(
                div()
                    .swiftui_font(Font::system_weight(17.0, FontWeight::SEMIBOLD))
                    .line_clamp(2)
                    .text_ellipsis()
                    .child(item.caption_title.clone()),
            );
        }
        if !item.caption.is_empty() {
            column = column.child(
                div().swiftui_font(Font::system_size(15.0)).line_clamp(3).text_ellipsis().child(item.caption.clone()),
            );
        }
        if expandable {
            column = column.child(
                div()
                    .swiftui_font(Font::system_weight(14.0, FontWeight::SEMIBOLD))
                    .text_color(white().opacity(0.64))
                    .child(tr("media_viewer_more")),
            );
        }
        let hidden = self.shows_caption_overlay;
        div()
            .id("media-viewer-caption-preview")
            .when(hidden, |this| this.opacity(0.0))
            .child(column)
            .on_click(cx.listener(move |this: &mut Self, _, _, cx| {
                if expandable {
                    this.set_caption_overlay(true, cx);
                }
            }))
            .into_any_element()
    }

    fn caption_overlay(&self, item: &MediaViewerGalleryItem, cx: &mut Context<Self>) -> AnyElement {
        let close_button = {
            let this = cx.entity().downgrade();
            LiquidGlassButton::new("media-viewer-caption-close", LiquidGlassButtonShape::Circle)
                .size(48.0)
                .accessibility_label(Some(tr("close")))
                .on_press(move |_, cx| {
                    this.update(cx, |this, cx| this.set_caption_overlay(false, cx)).ok();
                })
                .child(icon("xmark", 18.0))
        };
        let mut text = div().flex().flex_col().gap(px(16.0)).w_full().px(px(24.0)).pb(px(28.0)).text_color(white());
        if !item.caption_title.is_empty() {
            text = text.child(div().swiftui_font(Font::system_weight(28.0, FontWeight::BOLD)).child(item.caption_title.clone()));
        }
        if !item.caption.is_empty() {
            text = text.child(div().swiftui_font(Font::system_size(19.0)).line_height(px(19.0 * 1.2 + 5.0)).child(item.caption.clone()));
        }
        div()
            .id("media-viewer-caption-overlay")
            .absolute()
            .inset_0()
            .bg(black().opacity(0.76))
            .on_click(cx.listener(|this: &mut Self, _, _, cx| this.set_caption_overlay(false, cx)))
            .child(
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .px(px(18.0))
                            .pt(px(10.0))
                            .pb(px(12.0))
                            .child(div().flex_1())
                            .child(close_button),
                    )
                    .child(div().id("media-viewer-caption-scroll").flex_1().overflow_y_scroll().child(text)),
            )
            .into_any_element()
    }
}

impl Focusable for MediaViewerScreen {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MediaViewerScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let now = Self::now();
        let viewport = window.viewport_size();
        self.overlay_screen_frame = Bounds::new(point(px(0.0), px(0.0)), viewport);
        if self.pager_ready && self.filmstrip.is_none() && self.shows_filmstrip(cx) {
            self.sync_filmstrip(window, cx);
        }

        let (has_hero_flight, source_frame, items_empty, is_presented) = {
            let state = self.state.read(cx);
            (state.source_frame().is_some(), state.source_frame(), state.items().is_empty(), state.is_presented())
        };
        let show_hero = has_hero_flight && !self.pager_ready;
        let chrome_interactive = !has_hero_flight || self.pager_ready;
        let backdrop = self.backdrop_opacity.frame_value(now, window);
        let chrome_visible = !self.veils_content && (chrome_interactive || (!self.is_closing_hero && backdrop > 0.0));
        self.chrome_visible_opacity.set_in(
            if chrome_visible { 1.0 } else { 0.0 },
            ease_in_out(MediaViewerScreenMetrics::CHROME_FADE),
            now,
            cx,
        );
        let chrome_opacity =
            self.shows_chrome_opacity.frame_value(now, window) * self.chrome_visible_opacity.frame_value(now, window);
        let _badge = self.page_badge_opacity.frame_value(now, window);
        let close_opacity = self.close_opacity.frame_value(now, window);
        self.reclaim_keys_from_page_below(window, cx);

        let mut root = div()
            .id("media-viewer-screen")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .absolute()
            .inset_0()
            .opacity(close_opacity)
            .on_action(cx.listener(|this: &mut Self, _: &CloseViewer, _, cx| {
                if this.shows_caption_overlay {
                    this.set_caption_overlay(false, cx);
                } else if this.is_interactive(cx) {
                    this.animate_close(None, cx);
                }
            }))
            .on_action(cx.listener(|this: &mut Self, _: &PreviousPage, _, cx| this.page_by(-Self::forward_sign() as i64, cx)))
            .on_action(cx.listener(|this: &mut Self, _: &NextPage, _, cx| this.page_by(Self::forward_sign() as i64, cx)))
            .on_action(cx.listener(|this: &mut Self, _: &SaveMedia, _, cx| {
                if this.is_interactive(cx) {
                    this.save_current_media_as_file(cx);
                }
            }))
            .on_action(cx.listener(|this: &mut Self, _: &CopyMedia, _, cx| {
                if this.is_interactive(cx) {
                    this.copy_current_media(cx);
                }
            }))
            .on_action(cx.listener(|_, _: &ToggleFullScreen, window, _| window.toggle_fullscreen()))
            .on_scroll_wheel(cx.listener(|this: &mut Self, event: &ScrollWheelEvent, window, cx| {
                this.handle_scroll_wheel(event, window, cx)
            }))
            .on_mouse_move(cx.listener(|this: &mut Self, event: &MouseMoveEvent, window, cx| {
                this.reclaim_keys_from_page_below(window, cx);
                this.handle_mouse_move(event, cx)
            }));
        if is_presented && self.closing_presentation_id.is_none() {
            root = root.occlude();
        }
        let opaque = is_presented
            && self.closing_presentation_id.is_none()
            && !self.is_closing_hero
            && self.pager_ready
            && !show_hero
            && backdrop >= 0.999
            && close_opacity >= 0.999;
        let frame_signature = (
            opaque,
            self.pager_ready,
            show_hero,
            (backdrop * 10.0).round() as i32,
            (close_opacity * 10.0).round() as i32,
            (chrome_opacity * 10.0).round() as i32,
            self.veils_content,
        );
        if VIEWER_LAST_FRAME_SIGNATURE.with(|last| last.replace(Some(frame_signature))) != Some(frame_signature) {
            tracing::warn!(
                target: "riseonly::X1583",
                "viewer frame opaque={opaque} pager_ready={} show_hero={show_hero} backdrop={backdrop:.2} close_opacity={close_opacity:.2} chrome={chrome_opacity:.2} veils={} viewport={:?} items_empty={items_empty} presented={is_presented} closing={:?}",
                self.pager_ready,
                self.veils_content,
                viewport,
                self.closing_presentation_id,
            );
        }
        if opaque {
            VIEWER_HOLDS_OPAQUE_COVER.with(|holds| holds.set(true));
            root = root.child(opaque_cover_layer());
        } else {
            release_viewer_opaque_cover(window, cx);
        }

        root = root.child(
            div()
                .id("media-viewer-backdrop")
                .absolute()
                .inset_0()
                .bg(black().opacity(backdrop))
                .on_click(cx.listener(|this: &mut Self, _, _, cx| this.toggle_chrome(cx))),
        );

        if self.pager_ready && !items_empty {
            let pager = self.pager_layer(viewport, window, cx);
            root = root.child(
                div().absolute().inset_0().opacity(if self.veils_content { 0.0 } else { 1.0 }).child(pager),
            );
        }

        if show_hero
            && let Some(source_screen) = source_frame
            && let Some(item) = self.current_gallery_item(cx)
        {
            root = root.child(self.hero_flight_layer(viewport, source_screen, &item, window, cx));
        }

        let mut chrome = div().absolute().inset_0().flex().flex_col().opacity(chrome_opacity);
        if self.shows_chrome
            && let Some(top) = self.top_bar(MediaViewerSafeArea::top_inset(window, cx), cx)
        {
            chrome = chrome.child(top);
        }
        chrome = chrome.child(div().flex_1());
        if self.shows_chrome
            && let Some(bottom) = self.bottom_bar(window, cx)
        {
            chrome = chrome.child(bottom);
        }
        root = root.child(chrome);
        if !chrome_interactive {
            root = root.child(div().id("media-viewer-chrome-blocker").absolute().inset_0().occlude());
        }

        let caption_progress = self.caption_overlay_progress.frame_value(now, window);
        if (self.shows_caption_overlay || caption_progress > 0.0)
            && let Some(item) = self.current_gallery_item(cx)
        {
            let lift = viewport.height * (1.0 - caption_progress);
            let layer = div()
                .absolute()
                .inset_0()
                .top(lift)
                .bottom(-lift)
                .opacity(caption_progress.clamp(0.0, 1.0))
                .child(self.caption_overlay(&item, cx));
            root = root.child(layer);
        }

        let countdown = {
            let state = self.state.read(cx);
            state.countdown().copied().filter(|countdown| Some(countdown.presentation_id) == state.presentation_id())
        };
        if let Some(countdown) = countdown {
            root = root.child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .items_start()
                    .child(MediaViewerCountdownBadge::new(countdown, MediaViewerSafeArea::top_inset(window, cx))),
            );
        }
        env_scope::<ColorSchemeKey>(Some(ColorScheme::Dark)).child(root)
    }
}

#[derive(IntoElement)]
struct MediaViewerLoopToggle {
    controller: Entity<MediaViewerVideoController>,
}

impl RenderOnce for MediaViewerLoopToggle {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let preferences = MediaViewerPlaybackPreferences::shared(cx);
        let loops = preferences.read(cx).loops_video();
        let accent = ThemeStore::shared(cx).read(cx).current_theme().primary._100;
        let controller = self.controller;
        div()
            .id("media-viewer-loop-toggle")
            .w(px(52.0))
            .h(px(52.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .children(IconUi::sized("repeat", px(20.0), if loops { accent } else { white() }, true))
            .on_click(move |_, _, cx| {
                preferences.update(cx, |p, cx| p.set_loops_video(!loops, cx));
                controller.update(cx, |c, cx| c.set_loops_playback(!loops, cx));
            })
    }
}

enum OverflowActionID {}

impl OverflowActionID {
    const SAVE: &'static str = "save";
    const SAVE_TO_PHOTOS: &'static str = "save_to_photos";
    const SET_AS_MAIN: &'static str = "set_as_main";
    const DELETE_PROFILE_PHOTO: &'static str = "delete_profile_photo";
    const SHOW_IN_CONTEXT: &'static str = "show_in_context";
    const REPLY: &'static str = "reply";
    const DELETE: &'static str = "delete";
    const DELETE_FOR_EVERYONE: &'static str = "delete_for_everyone";
    const DELETE_FOR_ME: &'static str = "delete_for_me";
    const DELETE_FROM_FAVOURITES: &'static str = "delete_from_favourites";
    const REPORT: &'static str = "report";
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct PageBadgeTimerKey {
    position: Option<i64>,
    ready: bool,
}

struct MediaViewerYieldsToTopNotifier {
    enabled: bool,
}

impl MediaViewerYieldsToTopNotifier {
    fn covered(&self, cx: &mut App) -> bool {
        self.enabled
            && NotifierStore::shared(cx)
                .read(cx)
                .items()
                .iter()
                .any(|item| item.position == NotifierPosition::Top)
    }
}

/// safe area insets of the key window, desktop has no notch so both are zero
/// the bars keep their padding on top
pub enum MediaViewerSafeArea {}

impl MediaViewerSafeArea {
    /// band the window own controls take over the viewer
    /// mac traffic lights stay above every layer so viewer buttons start under them
    pub fn top_inset(window: &Window, cx: &App) -> f32 {
        f32::from(WindowCoverFrame::resolve(window, cx).safe_top)
    }

    pub fn bottom_inset() -> f32 {
        0.0
    }
}
