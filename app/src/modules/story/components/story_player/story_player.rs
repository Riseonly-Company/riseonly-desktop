use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, AnyView, App, AppContext as _, BoxShadow, Context, ElementId, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, KeyDownEvent, KeystrokeEvent, MouseButton, MouseUpEvent, ParentElement, Pixels, Point, Render,
    RenderOnce, ScrollWheelEvent, StyleRefinement, canvas,
    SharedString, Size, StatefulInteractiveElement as _, Styled, Subscription, Task, WeakEntity, Window, black, div,
    point, px, relative, transparent_black, white,
};
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_core::rise_app_events::{RiseAppEvents, RiseSubscription};
use rise_i18n::tr;
use rise_media::video::av_player::AVPlayer;
use rise_platform::haptics::{ImpactStyle, NotificationKind, impact, notification, selection_changed};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::context_menu_ui::{ContextMenuItem, ContextMenuUi};
use rise_ui::error_text_ui::system_red;
use rise_ui::input_ui::input_ui::InputUi;
use rise_ui::input_ui::{InputUiEvent, InputUiState};
use rise_ui::liquid_chrome::{LiquidChromeModifiers, LiquidGlassContrast};
use rise_ui::main_text::MainText;
use rise_ui::native_confirm_alert::NativeConfirmAlert;
use rise_ui::pressable_ui::PressableUi;
use rise_ui::screen_capture_protected_ui::ScreenCaptureProtectedUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::drag_gesture::{DragGesture, DragGestureModifiers, DragGestureValue};
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::lifecycle::{LifecycleModifiers, on_change, use_task};
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::{ease_in_out, ease_out, interactive_spring, spring};
use rise_ui::swiftui::progress_view::progress_view;
use rise_ui::swiftui::repeating_timer::RepeatingTimer;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::UnitPoint;

use crate::app::router::in_app_link_browser::InAppLinkBrowser;
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{MainTab, RootRoute};
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::utilities::sai_ws_slice_binding::SaiSliceState;
use crate::core::lib::canon_identity::{RiseAuthorIdentity, RiseIdentity};
use crate::core::shared_element_transition::kernel::shared_element_motion::SharedElementMotion;
use crate::core::shared_element_transition::overlay::shared_element_overlay_contracts::{
    SharedElementOverlayDestination, SharedElementOverlayPhase, SharedElementOverlayProbe, SharedElementOverlayRegistry,
};
use crate::core::shared_element_transition::overlay::shared_element_overlay_driver::SharedElementOverlayDriver;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::clever_image_disk_cache::CleverImageWorkLane;
use crate::core::ui::media_link_areas_overlay::{MediaLinkAreasOverlay, MediaLinkOpenHint, MediaLinkZone};
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::{
    NotifierPosition, NotifierSendOptions, NotifierStore, NotifierType,
};
use crate::core::widgets::overlay::premium_upsell_store::PremiumUpsellStore;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::media_picker::api::media_picker_result::MediaPickerTrackSelection;
use crate::modules::music::pages::audio_page::audio_page::AudioPage;
use crate::modules::post::components::post::post_music_plate_overlay::{MediaFrameRect, MusicPlateOverlay};
use crate::modules::shorts::components::shorts_pinch_zoom::ShortsPinchZoomLayer;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::post::components::post::post_music_playback_controller::PostMusicPlaybackController;
use crate::modules::story::components::story_composer_host::StoryComposerHost;
use crate::modules::story::components::story_player::story_media_view::{
    StoryImageAspectProbe, StoryMediaPrefetcher, StoryMediaView,
};
use crate::core::media::rise_media_foreground_gate::RiseMediaForegroundGate;
use crate::core::media_viewer::media_viewer_video_scrubber::rounded_rect_path;
use crate::modules::story::components::story_player::story_pending_upload_overlay::StoryPendingUploadOverlay;
use crate::modules::story::components::story_player::story_player_shared_element_destination::StoryPlayerSharedElementDestination;
use crate::modules::story::components::story_player::story_video_segment::StoryVideoSegment;
use crate::modules::story::components::story_player::story_viewers_pager::StoryViewersPager;
use crate::modules::story::shared::story_download_store::StoryDownloadStore;
use crate::modules::story::shared::story_stealth_preference::StoryStealthPreference;
use crate::modules::story::stores::story::story_actions::story_types::{
    StoryPendingPublish, StoryPlaybackPolicy, StoryPlayerEntrance, StoryPlayerGroupLayout, StoryPlayerPresentationStyle,
    StoryPlayerRequest,
};
use crate::modules::story::stores::story::story_actions::story_wire_types::{
    StoryAuthorDTO, StoryDTO, StoryDTOExt, StoryGroupDTO, StoryViewerDTO,
};
use crate::modules::story::stores::story::story_actions::story_actions::StoryActionsStore;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::user::engine::rise_meaningful_activity::RiseMeaningfulActivity;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SAI_WS_DID_CONNECT;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

const WHEEL_DISMISS_DISTANCE: f32 = 120.0;
const WHEEL_SWIPE_GAP: Duration = Duration::from_millis(300);

enum StoryAudioDestination {
    Route(RootRoute),
    Story(MediaPickerTrackSelection),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DragAxis {
    Undecided,
    Dismiss,
    Viewers,
    Horizontal,
}

#[derive(IntoElement)]
pub struct StoryAudioLabel {
    story: StoryDTO,
    track_id: String,
    on_tap: Rc<dyn Fn(&mut Window, &mut App)>,
}

impl StoryAudioLabel {
    pub fn new(story: StoryDTO, track_id: String, on_tap: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        Self { story, track_id, on_tap: Rc::new(on_tap) }
    }

    fn title(&self, cx: &mut App) -> String {
        let music = MusicServicesStore::shared(cx);
        let track = music.update(cx, |music, cx| music.sound_track(&self.track_id, cx));
        let plate = self.story.track_plate.as_ref();
        let name = track
            .as_ref()
            .and_then(|track| track.title.clone())
            .or_else(|| plate.map(|plate| plate.title.clone()))
            .unwrap_or_default();
        let artist = track
            .as_ref()
            .and_then(|track| track.artist.as_ref())
            .map(|artist| artist.stage_name.clone())
            .or_else(|| plate.map(|plate| plate.artist.clone()))
            .unwrap_or_default();
        if track.as_ref().and_then(|track| track.status.as_deref()) == Some(RiseMusicTrackStatus::OriginalAudio.wire()) {
            return if artist.is_empty() {
                tr("original_audio_title")
            } else {
                format!("{} \u{b7} {artist}", tr("original_audio_title"))
            };
        }
        if name.is_empty() {
            return if artist.is_empty() { tr("audio_page_title") } else { artist };
        }
        if artist.is_empty() { name } else { format!("{artist} \u{b7} {name}") }
    }
}

impl RenderOnce for StoryAudioLabel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let title = self.title(cx);
        let track_id = self.track_id.clone();
        use_task(
            SharedString::from(format!("story-audio-label-task-{track_id}")),
            track_id.clone(),
            window,
            cx,
            async move |cx| {
                let Ok(load) = cx.update(|_, cx| {
                    MusicInteractionsStore::shared(cx).update(cx, |music, cx| music.load_sound_track(&track_id, false, cx))
                }) else {
                    return;
                };
                load.await;
            },
        );
        let on_tap = self.on_tap;
        plain("story-audio-label")
            .on_press(move |window, cx| on_tap(window, cx))
            .w_full()
            .flex()
            .items_center()
            .gap(px(4.0))
            .children(IconUi::sized("music.note", px(10.0), white(), false).map(|icon| icon.flex_none()))
            .child(div().min_w_0().child(MainText::new(title).px(11.0).color(white()).number_of_lines(1)))
    }
}

/// only the progress bars observe it,
/// it ticks up to 50 times a second and repainting the player kept main thread busy
pub struct StoryPlaybackClock {
    pub value: f64,
    pub video_durations: HashMap<String, f64>,
    pub video_players: HashMap<String, WeakEntity<AVPlayer>>,
}

impl StoryPlaybackClock {
    fn new() -> Self {
        Self { value: 0.0, video_durations: HashMap::new(), video_players: HashMap::new() }
    }
}

pub struct StoryProgressBars {
    count: usize,
    index: usize,
    clock: Entity<StoryPlaybackClock>,
    shown_filled: Option<f32>,
    painted: Rc<Cell<bool>>,
    width: Rc<Cell<f32>>,
    _clock_observation: Subscription,
}

const STORY_PROGRESS_BAR_HEIGHT: f32 = 2.5;
const STORY_PROGRESS_FILL_STEP: f32 = 0.5;

impl StoryProgressBars {
    fn new(clock: Entity<StoryPlaybackClock>, cx: &mut Context<Self>) -> Self {
        let observation = cx.observe(&clock, |this, clock, cx| this.clock_did_change(clock, cx));
        Self {
            count: 0,
            index: 0,
            clock,
            shown_filled: None,
            painted: Rc::new(Cell::new(false)),
            width: Rc::new(Cell::new(0.0)),
            _clock_observation: observation,
        }
    }

    fn set_position(&mut self, count: usize, index: usize, cx: &mut Context<Self>) {
        if self.count == count && self.index == index {
            return;
        }
        self.count = count;
        self.index = index;
        cx.notify();
    }

    fn clock_did_change(&mut self, clock: Entity<StoryPlaybackClock>, cx: &mut Context<Self>) {
        let filled = Self::filled_width(clock.read(cx).value, self.width.get());
        if self.shown_filled == Some(filled) {
            return;
        }
        let path = self
            .painted
            .get()
            .then(|| rounded_rect_path(0.0, 0.0, filled, STORY_PROGRESS_BAR_HEIGHT, Point::default()))
            .flatten();
        let entity_id = cx.entity_id();
        let key = Self::swap_key(entity_id.as_u64());
        let swapped = path.is_some_and(|path| {
            cx.with_window(entity_id, |window, _| window.swap_paths(vec![(key, path, white().into())])).unwrap_or(false)
        });
        if swapped {
            self.shown_filled = Some(filled);
        } else {
            self.painted.set(false);
            cx.notify();
        }
    }

    fn filled_width(fraction: f64, width: f32) -> f32 {
        ((fraction.clamp(0.0, 1.0) as f32 * width) / STORY_PROGRESS_FILL_STEP).round() * STORY_PROGRESS_FILL_STEP
    }

    fn swap_key(entity: u64) -> u64 {
        entity ^ (7 << 56)
    }

    /// cached host the player places where the bars go
    fn element(bars: Entity<Self>) -> AnyElement {
        div()
            .w_full()
            .h(px(STORY_PROGRESS_BAR_HEIGHT))
            .child(
                AnyView::from(bars)
                    .cached(StyleRefinement::default().w_full().h(px(STORY_PROGRESS_BAR_HEIGHT)))
                    .replay_when_moved(),
            )
            .into_any_element()
    }
}

impl Render for StoryProgressBars {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        gpui::hear_entity_through_events_only(self.clock.entity_id());
        let current = self.clock.read(cx).value;
        let index = self.index;
        let filled = Self::filled_width(current, self.width.get());
        self.shown_filled = Some(filled);
        let (painted, width) = (self.painted.clone(), self.width.clone());
        painted.set(false);
        let swap_key = Self::swap_key(cx.entity_id().as_u64());
        div().w_full().h(px(STORY_PROGRESS_BAR_HEIGHT)).flex().gap(px(4.0)).children((0..self.count).map(move |idx| {
            let bar = div().relative().flex_1().h_full().rounded_full().bg(white().opacity(0.3));
            match idx.cmp(&index) {
                std::cmp::Ordering::Less => bar.child(div().size_full().rounded_full().bg(white())),
                std::cmp::Ordering::Greater => bar,
                std::cmp::Ordering::Equal => {
                    let (painted, width) = (painted.clone(), width.clone());
                    bar.child(
                        canvas(|_, _, _| (), move |bounds, (), window, _| {
                            let bar_width = f32::from(bounds.size.width);
                            width.set(bar_width);
                            let filled = Self::filled_width(current, bar_width);
                            let fill = rounded_rect_path(0.0, 0.0, filled, STORY_PROGRESS_BAR_HEIGHT, bounds.origin);
                            painted.set(fill.is_some());
                            if let Some(fill) = fill {
                                window.paint_swappable_path(fill, white(), swap_key, bounds.origin);
                            }
                        })
                        .absolute()
                        .left_0()
                        .top_0()
                        .size_full(),
                    )
                }
            }
        }))
    }
}

/// what the main video segment last received so a render pushes only real changes
#[derive(Clone, PartialEq)]
struct VideoSegmentSync {
    story_id: i64,
    media: String,
    is_paused: bool,
    size: Size<Pixels>,
}

pub struct StoryPlayer {
    request: StoryPlayerRequest,
    meaningful_activity_source: Uuid,
    meaningful_activity_mounted: bool,
    did_start_playback: bool,
    meaningful_ready_story_id: Option<i64>,

    navigation_store: Option<Entity<NavigationStore>>,
    service: Entity<StoryServicesStore>,
    user_service: Entity<UserServicesStore>,
    auth_service: Entity<AuthServicesStore>,
    share_sheet: Entity<ShareSheetStore>,
    premium_upsell: Entity<PremiumUpsellStore>,

    group_index: usize,
    story_index: usize,
    playback_clock: Entity<StoryPlaybackClock>,
    progress_bars: Entity<StoryProgressBars>,
    media_aspect: Option<f32>,
    media_fills: bool,
    media_geometry_story_id: Option<i64>,
    is_paused: bool,
    scene_is_active: bool,
    is_holding_to_pause: bool,
    is_user_paused: bool,
    user_pause_alpha: Animated<f32>,
    link_hint: Option<MediaLinkZone>,
    link_sheet_open: bool,
    link_hint_point: Point<Pixels>,
    link_hint_alpha: Animated<f32>,
    link_layer_origin: Point<Pixels>,
    is_pinching: bool,
    pinch_in_press: bool,
    chrome_alpha: Animated<f32>,
    drag_offset: Animated<f32>,
    presentation_distance: f32,
    is_presenting: bool,
    did_start_presentation: bool,
    is_dismissing: bool,
    feed_tail_open: bool,
    feed_tail_waiting: bool,
    geo_size: Size<Pixels>,
    stable_size: Size<Pixels>,
    stable_inset_top: Pixels,
    viewers_progress: Entity<Animated<f32>>,
    viewers_progress_target: f32,
    is_viewers_pager_mounted: bool,
    viewed_story_ids: HashSet<i64>,
    reply_input: Entity<InputUiState>,
    reply_focused: bool,
    horizontal_drag_offset: Animated<f32>,
    is_cube_transitioning: bool,
    is_opening_chat: bool,
    is_story_menu_presented: bool,
    audio_story: Option<StoryDTO>,
    show_audio_sheet: bool,
    audio_destination: Option<StoryAudioDestination>,
    show_audio_story_composer: bool,
    audio_story_composer_active: bool,
    audio_story_track_seed: Option<MediaPickerTrackSelection>,
    finished_video_story_id: Option<i64>,
    story_pending_deletion: Option<StoryDTO>,
    stealth_enabled: bool,
    upcoming_prefetch_task: Option<Task<()>>,
    upcoming_prefetch_wanted: bool,
    overlay_veiled: bool,
    overlay_chrome_hidden: bool,
    overlay_chrome_alpha: Animated<f32>,
    overlay_content_opacity: Animated<f32>,
    shared_element_destination: Rc<StoryPlayerSharedElementDestination>,
    chrome_enabled: bool,

    hold_task: Option<Task<()>>,
    press_start: Option<Duration>,
    drag_axis: DragAxis,
    bar_drag_axis: DragAxis,
    movement_cancelled_hold: bool,
    owner_bar_origin: Option<Point<Pixels>>,

    wheel_dismiss_travel: f32,
    wheel_last_event: Option<Duration>,

    timer: Option<RepeatingTimer>,
    video_segment: Option<Entity<StoryVideoSegment>>,
    video_segment_sync: Option<VideoSegmentSync>,
    preview_video_segment: Option<Entity<StoryVideoSegment>>,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
    _socket_did_connect: (RiseSubscription, Task<()>),
}

impl StoryPlayer {
    const TICK: f64 = 0.02;

    /// a running open session mounts the player paused and veiled from the first frame
    pub fn new(request: StoryPlayerRequest, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let veiled = request.presentation_style == StoryPlayerPresentationStyle::Standard
            && matches!(
                request.entrance,
                StoryPlayerEntrance::SharedElement { epoch } if SharedElementOverlayDriver::is_running_epoch(epoch)
            );
        let service = StoryServicesStore::shared(cx);
        let user_service = UserServicesStore::shared(cx);
        let auth_service = AuthServicesStore::shared(cx);
        let share_sheet = ShareSheetStore::shared(cx);
        let premium_upsell = PremiumUpsellStore::shared(cx);
        let playback_clock = cx.new(|_| StoryPlaybackClock::new());
        let progress_bars = {
            let clock = playback_clock.clone();
            cx.new(|cx| StoryProgressBars::new(clock, cx))
        };
        let reply_input = cx.new(|cx| {
            let mut field = InputUiState::single_line(cx);
            field.set_chromeless(true, cx);
            field.set_placeholder(tr("story_reply_placeholder"), cx);
            field.set_placeholder_color(Some(white().opacity(0.7)), cx);
            field.set_text_color(Some(white()), cx);
            field.set_caret_color(Some(white()), cx);
            field
        });
        let reply_focus = reply_input.read(cx).focus_handle(cx);
        let viewers_progress = cx.new(|_| Animated::new(0.0_f32));
        let escape_owner = cx.weak_entity();
        let player_window = window.window_handle();
        let reply_owner = cx.weak_entity();
        let foreground_gate = RiseMediaForegroundGate::shared(cx);
        let subscriptions = vec![
            cx.intercept_keystrokes(move |event: &KeystrokeEvent, window: &mut Window, cx: &mut App| {
                let keystroke = &event.keystroke;
                if keystroke.key != "enter" || keystroke.modifiers.modified() || window.window_handle() != player_window {
                    return;
                }
                let Some(player) = reply_owner.upgrade() else {
                    return;
                };
                if player.update(cx, |this, cx| this.submit_reply_from_key(window, cx)) {
                    cx.stop_propagation();
                }
            }),
            cx.intercept_keystrokes(move |event: &KeystrokeEvent, window: &mut Window, cx: &mut App| {
                let keystroke = &event.keystroke;
                if keystroke.key != "escape" || keystroke.modifiers.modified() || window.window_handle() != player_window {
                    return;
                }
                let Some(player) = escape_owner.upgrade() else {
                    return;
                };
                if player.update(cx, |this, cx| this.handle_escape(window, cx)) {
                    cx.stop_propagation();
                }
            }),
            cx.observe(&service, |this, _, cx| {
                this.sync_request(cx);
                cx.notify();
            }),
            cx.observe(&viewers_progress, |this, progress, cx| {
                this.viewers_progress_target = *progress.read(cx).target();
                cx.notify();
            }),
            cx.observe(&user_service, |_, _, cx| cx.notify()),
            cx.observe(&auth_service, |_, _, cx| cx.notify()),
            cx.observe(&share_sheet, |_, _, cx| cx.notify()),
            cx.observe(&premium_upsell, |_, _, cx| cx.notify()),
            cx.observe(&reply_input, |this, input, cx| {
                let focused = input.read(cx).is_focused();
                if this.reply_focused != focused {
                    tracing::debug!(target: "riseonly::X1704", "story reply focus {}", focused);
                    this.reply_focused = focused;
                }
                cx.notify();
            }),
            cx.subscribe_in(&reply_input, window, |this, _, event: &InputUiEvent, window, cx| {
                if matches!(event, InputUiEvent::Submitted) {
                    this.send_reply(window, cx);
                }
            }),
            cx.on_focus(&reply_focus, window, |this, _, cx| {
                this.reply_focused = true;
                cx.notify();
            }),
            cx.on_blur(&reply_focus, window, |this, _, cx| {
                this.reply_focused = false;
                cx.notify();
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.shared_element_dismiss_drag_vanished(cx);
                    this.tap_zones_drag_vanished(cx);
                }
                cx.notify();
            }),
            cx.observe(&foreground_gate, |this, gate, cx| {
                this.scene_is_active = gate.read(cx).is_foreground();
                cx.notify();
            }),
            cx.on_release(|this, cx| this.did_disappear(cx)),
        ];
        let destination = Rc::new(StoryPlayerSharedElementDestination::new(request.request_id()));
        Self {
            request,
            meaningful_activity_source: Uuid::new_v4(),
            meaningful_activity_mounted: false,
            did_start_playback: false,
            meaningful_ready_story_id: None,
            navigation_store: None,
            service,
            user_service,
            auth_service,
            share_sheet,
            premium_upsell,
            group_index: 0,
            story_index: 0,
            playback_clock,
            progress_bars,
            media_aspect: None,
            media_fills: true,
            media_geometry_story_id: None,
            is_paused: veiled,
            scene_is_active: foreground_gate.read(cx).is_foreground(),
            is_holding_to_pause: false,
            is_user_paused: false,
            user_pause_alpha: Animated::new(0.0),
            link_hint: None,
            link_sheet_open: false,
            link_hint_point: Point::default(),
            link_hint_alpha: Animated::new(0.0),
            link_layer_origin: Point::default(),
            is_pinching: false,
            pinch_in_press: false,
            chrome_alpha: Animated::new(1.0),
            drag_offset: Animated::new(if veiled { 0.0 } else { 1_000.0 }),
            presentation_distance: 1_000.0,
            is_presenting: true,
            did_start_presentation: false,
            is_dismissing: false,
            feed_tail_open: false,
            feed_tail_waiting: false,
            geo_size: Size::default(),
            stable_size: Size::default(),
            stable_inset_top: px(0.0),
            viewers_progress,
            viewers_progress_target: 0.0,
            is_viewers_pager_mounted: false,
            viewed_story_ids: HashSet::new(),
            reply_input,
            reply_focused: false,
            horizontal_drag_offset: Animated::new(0.0),
            is_cube_transitioning: false,
            is_opening_chat: false,
            is_story_menu_presented: false,
            audio_story: None,
            show_audio_sheet: false,
            audio_destination: None,
            show_audio_story_composer: false,
            audio_story_composer_active: false,
            audio_story_track_seed: None,
            finished_video_story_id: None,
            story_pending_deletion: None,
            stealth_enabled: StoryStealthPreference::is_enabled(),
            upcoming_prefetch_task: None,
            upcoming_prefetch_wanted: false,
            overlay_veiled: veiled,
            overlay_chrome_hidden: veiled,
            overlay_chrome_alpha: Animated::new(if veiled { 0.0 } else { 1.0 }),
            overlay_content_opacity: Animated::new(1.0),
            shared_element_destination: destination,
            chrome_enabled: true,
            hold_task: None,
            press_start: None,
            drag_axis: DragAxis::Undecided,
            bar_drag_axis: DragAxis::Undecided,
            movement_cancelled_hold: false,
            owner_bar_origin: None,
            wheel_dismiss_travel: 0.0,
            wheel_last_event: None,
            timer: None,
            video_segment: None,
            video_segment_sync: None,
            preview_video_segment: None,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
            _socket_did_connect: Self::observe_socket_did_connect(cx),
        }
    }

    fn sync_request(&mut self, cx: &mut Context<Self>) {
        let live = self.service.read(cx).player_request.as_ref().filter(|live| live.id == self.request.id).cloned();
        if let Some(live) = live
            && live != self.request
        {
            let old_layout = std::mem::replace(&mut self.request, live).layout;
            if old_layout != self.request.layout {
                let new_layout = self.request.layout.clone();
                self.reanchor(&old_layout, &new_layout, cx);
            }
        }
        self.refresh_feed_tail(cx);
    }

    /// feed player loads the next tray page near its last loaded person and steps
    /// into it until has_more ends
    fn refresh_feed_tail(&mut self, cx: &mut Context<Self>) {
        let is_feed =
            self.request.presentation_style == StoryPlayerPresentationStyle::FeedCube && !self.request.owns_explicit_stories;
        self.feed_tail_open = is_feed && self.service.update(cx, |service, cx| service.feed_has_more(cx));
        if self.feed_tail_waiting && self.group_index + 1 < self.groups().len() {
            self.feed_tail_waiting = false;
            if !self.is_dismissing {
                self.go_next_group(cx);
            }
            return;
        }
        if self.feed_tail_waiting && !self.feed_tail_open {
            let pending = StoryActionsStore::shared(cx).read(cx).get_stories_feed_sai.is_pending();
            if !pending {
                self.feed_tail_waiting = false;
                if !self.is_dismissing {
                    self.dismiss(true, cx);
                }
            }
            return;
        }
        if self.feed_tail_open && self.group_index + 3 >= self.groups().len() {
            cx.defer(|cx| {
                StoryInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.load_more_stories_feed(cx));
            });
        }
    }

    fn after(seconds: f64, cx: &mut Context<Self>, work: impl FnOnce(&mut Self, &mut Context<Self>) + 'static) {
        cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(seconds)).await;
            this.update(cx, |this, cx| {
                work(this, cx);
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn press_action(
        cx: &Context<Self>,
        work: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl Fn(&mut Window, &mut App) + 'static {
        let weak = cx.weak_entity();
        move |window: &mut Window, cx: &mut App| {
            weak.update(cx, |this, cx| {
                work(this, window, cx);
                cx.notify();
            })
            .ok();
        }
    }

    fn groups(&self) -> &[StoryGroupDTO] {
        &self.request.groups
    }

    fn current_group(&self) -> Option<&StoryGroupDTO> {
        self.groups().get(self.group_index)
    }

    fn current_stories(&self) -> &[StoryDTO] {
        self.current_group().map(|group| group.stories.as_slice()).unwrap_or_default()
    }

    fn current_story(&self) -> Option<&StoryDTO> {
        self.current_stories().get(self.story_index)
    }

    fn author(&self) -> Option<&StoryAuthorDTO> {
        self.current_group()
            .and_then(|group| group.author.as_ref())
            .or_else(|| self.current_story().and_then(|story| story.author.as_ref()))
    }

    fn current_author(&self) -> RiseAuthorIdentity {
        RiseIdentity::author(
            self.current_story().map(|story| story.author_id.as_str()),
            None,
            self.author().map(|author| author.id.as_str()),
        )
    }

    fn current_author_id(&self) -> String {
        self.current_author().wire_id().unwrap_or_default().to_owned()
    }

    fn viewer_id(&self, cx: &App) -> Option<String> {
        self.auth_service.read(cx).effective_user_id_for_wire()
    }

    fn is_own_story(&self, cx: &App) -> bool {
        RiseIdentity::is_own(&self.current_author(), self.viewer_id(cx).as_deref(), None)
    }

    pub fn viewers_open(&self) -> bool {
        self.viewers_progress_target > 0.001
    }

    fn drag_offset_target(&self) -> f32 {
        *self.drag_offset.target()
    }

    fn progress(&self, cx: &App) -> f64 {
        self.playback_clock.read(cx).value
    }

    fn set_progress(&self, value: f64, cx: &mut App) {
        self.playback_clock.update(cx, |clock, cx| {
            clock.value = value;
            cx.notify();
        });
    }

    /// premium and share sheets cover the story so it waits under them
    fn playback_is_paused(&self, cx: &App) -> bool {
        self.is_paused
            || self.is_user_paused
            || self.link_hint.is_some()
            || self.link_sheet_open
            || self.audio_story.is_some()
            || self.audio_story_composer_active
            || !self.scene_is_active
            || self.premium_upsell.read(cx).prompt().is_some()
            || self.share_sheet.read(cx).is_visible()
    }

    /// premium and share sheets live above the player and take the focus with them
    fn is_covered_by_sheet(&self, cx: &App) -> bool {
        self.premium_upsell.read(cx).prompt().is_some() || self.share_sheet.read(cx).is_visible()
    }

    fn wants_viewers_pager(&self, cx: &App) -> bool {
        self.is_own_story(cx) && !self.is_current_story_pending(cx)
    }

    /// story on screen isnt created on the server yet,
    /// so viewers and likes and sharing and delete have no id
    fn pending_publish(&self, cx: &App) -> Option<StoryPendingPublish> {
        let story = self.current_story()?;
        self.service.read(cx).pending_story_publish.as_ref().filter(|pending| pending.story_id == story.id).cloned()
    }

    fn is_current_story_pending(&self, cx: &App) -> bool {
        self.pending_publish(cx).is_some()
    }

    fn register_changes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current_story_id = self.current_story().map(|story| story.id);
        self.on_change_do("current-story", current_story_id, true, window, cx, |this, _, _, cx| {
            this.finished_video_story_id = None;
            this.is_user_paused = false;
            this.user_pause_alpha.jump(0.0);
            this.link_hint = None;
            this.link_sheet_open = false;
            this.sync_shared_element_destination(cx);
        });
        let mini_app_closed = self
            .navigation_store
            .as_ref()
            .is_none_or(|navigation| navigation.read(cx).bot_mini_app_presentation().is_none());
        self.on_change_do("mini-app-closed", mini_app_closed, false, window, cx, |this, _, closed, _| {
            if *closed {
                this.link_sheet_open = false;
            }
        });
        let paused = self.playback_is_paused(cx);
        self.on_change_do("playback-paused", paused, false, window, cx, |this, _, paused, cx| {
            if *paused {
                return;
            }
            let current = this.current_story().map(|story| story.id);
            if this.finished_video_story_id.is_some() && this.finished_video_story_id == current {
                this.finished_video_story_id = None;
                this.go_next(cx);
            }
        });
        let qualifies = self.qualifies_for_meaningful_activity(cx);
        self.on_change_do("meaningful-activity", qualifies, true, window, cx, |this, _, qualifies, cx| {
            let source = this.meaningful_activity_source;
            let weak = cx.weak_entity();
            let qualifies = *qualifies;
            RiseMeaningfulActivity::shared(cx).update(cx, |activity, cx| {
                activity.set_source(
                    source,
                    qualifies,
                    move |cx| weak.upgrade().is_some_and(|player| player.read(cx).qualifies_for_meaningful_activity(cx)),
                    cx,
                );
            });
        });
        self.on_change_do("viewers-open", self.viewers_open(), false, window, cx, |this, _, open, _| {
            this.is_paused = *open || this.reply_focused;
        });
        self.on_change_do("story-index", self.story_index, false, window, cx, |this, _, _, cx| {
            if this.viewers_open() {
                this.begin_segment_no_resume(cx);
            }
            this.prefetch_current_user_stories_if_needed(cx);
        });
        let story_count = self.current_stories().len();
        self.on_change_do("story-count", story_count, false, window, cx, |this, _, _, cx| {
            this.prefetch_current_user_stories_if_needed(cx);
        });
        self.on_change_do("feed-tail", (self.group_index, self.groups().len()), true, window, cx, |this, _, _, cx| {
            this.refresh_feed_tail(cx);
        });
        self.on_change_do("reply-focused", self.reply_focused, false, window, cx, |this, _, focused, _| {
            this.is_paused = *focused || this.viewers_open();
        });
        let share_visible = self.share_sheet.read(cx).is_visible();
        self.on_change_do("share-visible", share_visible, false, window, cx, |this, _, visible, _| {
            if !*visible && !this.viewers_open() && !this.reply_focused {
                this.is_paused = false;
            }
        });
        let covered = self.is_covered_by_sheet(cx) || self.audio_story.is_some() || self.audio_story_composer_active;
        let refocus = cx.weak_entity();
        on_change(("story-covered-focus", cx.entity_id()), covered, false, window, cx, move |_, covered, window, cx| {
            if *covered {
                return;
            }
            refocus
                .update(cx, |this, cx| {
                    let is_live = this.service.read(cx).player_request.as_ref().map(|live| live.id) == Some(this.request.id);
                    if is_live && !this.is_dismissing && !this.reply_focused {
                        window.focus(&this.focus_handle, cx);
                    }
                })
                .ok();
        });
        self.on_change_do("story-menu", self.is_story_menu_presented, false, window, cx, |this, _, presented, cx| {
            if *presented {
                this.is_paused = true;
                this.stop();
            } else if this.service.read(cx).player_request.is_some()
                && !this.viewers_open()
                && !this.reply_focused
                && this.story_pending_deletion.is_none()
            {
                this.is_paused = false;
                this.restart_timer(cx);
            }
        });
        let pending_deletion = self.story_pending_deletion.as_ref().map(|story| story.id);
        self.on_change_do("pending-deletion", pending_deletion, false, window, cx, |this, _, pending, cx| {
            if pending.is_some() {
                this.is_paused = true;
                this.stop();
            } else if this.service.read(cx).player_request.is_some()
                && !this.viewers_open()
                && !this.reply_focused
                && !this.is_story_menu_presented
            {
                this.is_paused = false;
                this.restart_timer(cx);
            }
        });

        let task_key = format!("{}:{}:{}", self.group_index, self.story_index, current_story_id.unwrap_or(0));
        let foreign_author = (!self.is_own_story(cx)).then(|| self.current_author_id()).filter(|id| !id.is_empty());
        let current: Vec<StoryDTO> = self.current_story().cloned().into_iter().collect();
        use_task(("story-player-task", cx.entity_id()), task_key, window, cx, async move |cx| {
            if let Some(author_id) = foreign_author {
                cx.update(|_, cx| {
                    UserInteractionsStore::shared(cx)
                        .update(cx, |user, cx| user.load_foreign_profile_user_id(author_id, cx));
                })
                .ok();
            }
            StoryMediaPrefetcher::prefetch(&current, CleverImageWorkLane::Visible, cx).await;
        });
        let position = format!("{}:{}", self.group_index, self.story_index);
        self.on_change_do("position", position, true, window, cx, |this, _, _, cx| {
            this.warm_upcoming_stories(cx);
        });
    }

    fn on_change_do<V: Clone + PartialEq + 'static>(
        &self,
        name: &'static str,
        value: V,
        initial: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut Self, &V, &V, &mut Context<Self>) + 'static,
    ) {
        let weak = cx.weak_entity();
        on_change((name, cx.entity_id()), value, initial, window, cx, move |old, new, _, cx| {
            weak.update(cx, |this, cx| {
                action(this, old, new, cx);
                cx.notify();
            })
            .ok();
        });
    }

    fn did_appear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.meaningful_activity_mounted = true;
        window.focus(&self.focus_handle, cx);
        self.install_shared_element_destination(cx);
        if self.did_start_playback {
            self.restart_timer(cx);
        } else {
            self.did_start_playback = true;
            self.start(cx);
        }
        cx.notify();
    }

    fn did_disappear(&mut self, cx: &mut App) {
        if !self.meaningful_activity_mounted {
            return;
        }
        self.meaningful_activity_mounted = false;
        let source = self.meaningful_activity_source;
        RiseMeaningfulActivity::shared(cx).update(cx, |activity, cx| activity.remove_source(source, cx));
        self.stop();
        let destination: Rc<dyn SharedElementOverlayDestination> = self.shared_element_destination.clone();
        SharedElementOverlayDriver::abort(&destination.surface(), cx);
        SharedElementOverlayRegistry::shared(cx).update(cx, |registry, _| registry.unregister(&destination));
        self.upcoming_prefetch_task = None;
    }

    fn geometry_did_change(&mut self, geo_size: Size<Pixels>, inset_top: Pixels, cx: &mut Context<Self>) {
        if self.geo_size == geo_size && self.stable_inset_top == inset_top {
            return;
        }
        self.geo_size = geo_size;
        if self.stable_size == Size::default() {
            self.stable_size = geo_size;
            self.stable_inset_top = inset_top;
            self.shared_element_destination.set_media_size(geo_size);
            self.begin_presentation(geo_size, inset_top, cx);
        } else if self.drag_offset_target() == 0.0 && !self.is_dismissing {
            self.stable_size = geo_size;
            self.stable_inset_top = inset_top;
            self.shared_element_destination.set_media_size(geo_size);
        }
        cx.notify();
    }

    fn open_audio(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(story) = self.current_story().cloned() else {
            return;
        };
        if story.track_id.as_deref().unwrap_or_default().is_empty() || self.audio_story.is_some() {
            return;
        }
        self.drop_reply_focus(window, cx);
        self.audio_story = Some(story);
        self.stop();
        self.show_audio_sheet = true;
    }

    /// the field holds the window focus,
    /// the flag alone misses it when the focus node is outside the drawn tree
    fn reply_field_focused(&self, window: &Window, cx: &App) -> bool {
        self.reply_focused || self.reply_input.read(cx).focus_handle(cx).is_focused(window)
    }

    fn drop_reply_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reply_focused = false;
        window.focus(&self.focus_handle, cx);
    }

    fn finish_audio_sheet_dismissal(&mut self, cx: &mut Context<Self>) {
        if self.audio_story.is_none() {
            return;
        }
        let destination = self.audio_destination.take();
        self.show_audio_sheet = false;
        self.audio_story = None;
        if self.service.read(cx).player_request.as_ref().map(|live| live.id) != Some(self.request.id) {
            return;
        }
        match destination {
            Some(StoryAudioDestination::Route(route)) => {
                self.dismiss(false, cx);
                self.push_route(route, cx);
            }
            Some(StoryAudioDestination::Story(selection)) => {
                self.audio_story_track_seed = Some(selection);
                self.audio_story_composer_active = true;
                self.show_audio_story_composer = true;
            }
            None => self.resume_after_audio(cx),
        }
    }

    fn resume_after_audio(&mut self, cx: &mut Context<Self>) {
        if self.service.read(cx).player_request.as_ref().map(|live| live.id) != Some(self.request.id)
            || self.is_dismissing
            || self.viewers_open()
            || self.reply_focused
            || self.is_story_menu_presented
            || self.story_pending_deletion.is_some()
            || self.share_sheet.read(cx).is_visible()
        {
            return;
        }
        self.is_paused = false;
        self.restart_timer(cx);
    }

    fn qualifies_for_meaningful_activity(&self, cx: &App) -> bool {
        let current = self.current_story().map(|story| story.id);
        self.meaningful_activity_mounted
            && current.is_some()
            && self.meaningful_ready_story_id == current
            && !self.is_current_story_pending(cx)
            && !self.is_presenting
            && !self.is_dismissing
            && !self.overlay_veiled
            && !self.viewers_open()
            && !self.is_story_menu_presented
            && !self.is_opening_chat
            && self.story_pending_deletion.is_none()
            && !self.share_sheet.read(cx).is_visible()
            && self.audio_story.is_none()
            && !self.audio_story_composer_active
    }

    fn warm_upcoming_stories(&mut self, cx: &mut Context<Self>) {
        self.upcoming_prefetch_wanted = true;
        if self.upcoming_prefetch_task.is_some() {
            return;
        }
        self.upcoming_prefetch_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            loop {
                let upcoming = this.update(cx, |this, _| {
                    if !this.upcoming_prefetch_wanted {
                        return Vec::new();
                    }
                    this.upcoming_prefetch_wanted = false;
                    this.upcoming_stories_for_prefetch()
                });
                let Ok(upcoming) = upcoming else {
                    return;
                };
                if upcoming.is_empty() {
                    break;
                }
                StoryMediaPrefetcher::prefetch(&upcoming, CleverImageWorkLane::Speculative, cx).await;
            }
            this.update(cx, |this, _| this.upcoming_prefetch_task = None).ok();
        }));
    }

    fn player_content(&mut self, size: Size<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.request.presentation_style == StoryPlayerPresentationStyle::FeedCube {
            self.cube_content(size, window, cx)
        } else {
            self.dismissible_story_content(size, window, cx)
        }
    }

    fn dismissible_story_content(
        &mut self,
        size: Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let now = RiseClock::monotonic();
        let shown_viewers_progress = self.viewers_progress.read(cx).frame_value(now, window);
        let chrome_visibility = self.overlay_chrome_alpha.frame_value(now, window);
        let card_opacity = Self::player_card_opacity(shown_viewers_progress);
        let wants_pager = self.wants_viewers_pager(cx);
        let drag_active = self.is_viewers_drag_active();
        let owns_media = self.player_owns_media(cx);
        let media_opacity = if owns_media && !self.overlay_veiled { card_opacity } else { 0.0 };
        let chrome_enabled = !(self.is_presenting && self.overlay_chrome_hidden)
            && (self.viewers_progress_target <= 0.02 || drag_active);
        let pending = self.pending_publish(cx);
        self.chrome_enabled = chrome_enabled;

        let (pinch_changed, pinch_content) = (cx.weak_entity(), cx.weak_entity());
        let media = div().absolute().inset_0().opacity(media_opacity).child(
            ShortsPinchZoomLayer::new(
                ("story-media-pinch", cx.entity_id()),
                size,
                self.pinch_zoom_enabled(),
                move |active, _, cx| {
                    pinch_changed
                        .update(cx, |this, cx| {
                            this.pinch_active_changed(active, cx);
                            cx.notify();
                        })
                        .ok();
                },
                move |scaled, window, cx| {
                    pinch_content
                        .update(cx, |this, cx| this.media_base_layer(scaled, window, cx))
                        .unwrap_or_else(|_| div().into_any_element())
                },
            )
            .recognizes_alongside_all(true),
        );

        let pager = (wants_pager && self.is_viewers_pager_mounted).then(|| {
            let entity = cx.entity();
            let height = f32::from(size.height + self.stable_inset_top);
            StoryViewersPager::new(
                "story-viewers-pager",
                self.current_stories().to_vec(),
                Binding::entity(
                    entity,
                    |this: &Self| this.story_index as i64,
                    |this, index, _| this.story_index = index.max(0) as usize,
                ),
                self.viewers_progress.clone(),
                shown_viewers_progress,
                f32::from(self.stable_inset_top),
                0.0,
                Self::press_action(cx, |this, _, cx| this.dismiss(true, cx)),
                |_, cx| {
                    StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.dismiss_player(cx));
                },
            )
            .owns_active_media(!owns_media)
            .screen_height(height)
            .allows_hit_testing(self.viewers_progress_target > 0.02 && !drag_active)
        });
        let pager = pager.map(|pager| {
            div()
                .absolute()
                .top(-self.stable_inset_top)
                .left_0()
                .right_0()
                .bottom_0()
                .when(self.viewers_progress_target > 0.02 && !drag_active, |it| it.occlude())
                .child(pager)
        });

        let chrome = div()
            .absolute()
            .inset_0()
            .opacity(chrome_visibility * card_opacity)
            .child(self.chrome_visual(size, window, cx));

        let pending_overlay = pending.filter(|_| self.viewers_progress_target <= 0.02).map(|pending| {
            div()
                .absolute()
                .inset_0()
                .opacity(if self.overlay_veiled { 0.0 } else { card_opacity })
                .child(StoryPendingUploadOverlay::new(
                    "story-pending-upload",
                    pending.phase,
                    Self::press_action(cx, |this, _, cx| this.cancel_pending_upload(cx)),
                ))
        });

        div()
            .relative()
            .w(size.width)
            .h(size.height)
            .child(media)
            .children(pager)
            .child(chrome)
            .children(pending_overlay)
            .into_any_element()
    }

    fn cancel_pending_upload(&mut self, cx: &mut Context<Self>) {
        let _ = impact(ImpactStyle::Light);
        StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.cancel_story_publish(cx));
        self.dismiss(true, cx);
    }

    fn cube_content(&mut self, size: Size<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let width = f32::from(size.width).max(1.0);
        let now = RiseClock::monotonic();
        let clamped = self.horizontal_drag_offset.frame_value(now, window).clamp(-width, width);
        let adjacent = self.adjacent_group_index(clamped);
        let fraction = if adjacent.is_some() { (clamped.abs() / width).min(1.0) } else { 0.0 };
        let forward = clamped < 0.0;
        let shadow = |alpha: f32, x: f32| BoxShadow {
            color: black().opacity(alpha),
            offset: point(px(x), px(0.0)),
            blur_radius: px(18.0),
            spread_radius: px(0.0),
            inset: false,
        };

        let adjacent_face = adjacent.map(|adjacent_index| {
            let origin = if forward { clamped + width } else { clamped - width };
            div()
                .absolute()
                .top_0()
                .left(px(origin))
                .w(size.width)
                .h(size.height)
                .shadow(vec![shadow(0.45 * (1.0 - fraction), if forward { -10.0 } else { 10.0 })])
                .child(self.cube_preview(adjacent_index, size, window, cx))
        });
        if adjacent.is_none() {
            self.preview_video_segment = None;
        }
        let current_face = div()
            .absolute()
            .top_0()
            .left(px(clamped))
            .w(size.width)
            .h(size.height)
            .shadow(vec![shadow(0.42 * fraction, if forward { 10.0 } else { -10.0 })])
            .child(self.dismissible_story_content(size, window, cx));

        let mut faces = Vec::with_capacity(2);
        if fraction > 0.5 {
            faces.push(current_face);
            faces.extend(adjacent_face);
        } else {
            faces.extend(adjacent_face);
            faces.push(current_face);
        }

        div()
            .relative()
            .w(size.width)
            .h(size.height)
            .overflow_hidden()
            .children(faces)
            .into_any_element()
    }

    fn cube_preview(
        &mut self,
        group_index: usize,
        size: Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let group = self.groups()[group_index].clone();
        let index = self.first_unseen_index(group_index, None, cx);
        let story = group.stories.get(index).or_else(|| group.stories.first()).cloned();
        let preview_author = group.author.clone().or_else(|| story.as_ref().and_then(|story| story.author.clone()));
        let preview_author_id = story
            .as_ref()
            .map(|story| story.author_id.clone())
            .or_else(|| preview_author.as_ref().map(|author| author.id.clone()))
            .unwrap_or_default();
        let media = story.as_ref().map(|story| self.preview_media_layer(story, window, cx));
        let bars = div().w_full().flex().gap(px(4.0)).children(group.stories.iter().enumerate().map(|(offset, _)| {
            div().flex_1().h(px(2.5)).rounded_full().bg(if offset < index { white() } else { white().opacity(0.3) })
        }));
        let header = div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(
                UserLogoUi::new("story-preview-logo")
                    .source(preview_author.as_ref().map(|author| SharedString::from(author.logo.clone())))
                    .name(preview_author.as_ref().map(|author| SharedString::from(author.name.clone())))
                    .user_id(Some(SharedString::from(preview_author_id)))
                    .size(32.0)
                    .show_border(false)
                    .show_story_ring(false),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(1.0))
                    .child(
                        MainText::new(preview_author.as_ref().map(|author| author.name.clone()).unwrap_or_default())
                            .px(14.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(white())
                            .number_of_lines(1),
                    )
                    .child(
                        MainText::new(Self::story_time_line(story.as_ref()))
                            .px(11.0)
                            .color(white().opacity(0.75))
                            .number_of_lines(1),
                    ),
            )
            .child(div().flex_1())
            .children(IconUi::sized("ellipsis", px(17.0), white(), false))
            .children(IconUi::sized("xmark", px(16.0), white(), false));
        let caption = story
            .as_ref()
            .and_then(|story| story.caption.as_deref())
            .map(trimming_whitespaces_and_newlines)
            .filter(|caption| !caption.is_empty())
            .map(|caption| {
                div()
                    .w_full()
                    .px(px(16.0))
                    .pb(px(10.0))
                    .child(MainText::new(caption.to_owned()).px(15.0).color(white()).number_of_lines(6))
            });
        let bar = div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(div().flex_1().h(px(44.0)).rounded_full().border(px(1.2)).border_color(white().opacity(0.55)))
            .children(IconUi::sized("heart", px(22.0), white(), false))
            .children(IconUi::sized("bubble.left", px(22.0), white(), false))
            .children(IconUi::sized("paperplane", px(22.0), white(), false));

        div()
            .relative()
            .w(size.width)
            .h(size.height)
            .overflow_hidden()
            .bg(black())
            .children(media)
            .child(Self::top_scrim())
            .child(Self::bottom_scrim())
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .flex_col()
                    .child(div().px(px(8.0)).pt(px(6.0)).child(bars))
                    .child(div().px(px(12.0)).pt(px(7.0)).child(header))
                    .child(div().flex_1())
                    .children(caption)
                    .child(div().px(px(12.0)).pb(px(6.0)).child(bar)),
            )
            .into_any_element()
    }

    fn preview_media_layer(&mut self, story: &StoryDTO, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (media, is_video) = self.media_source(story, cx);
        let content = if is_video && !media.is_empty() {
            let segment = match &self.preview_video_segment {
                Some(segment) => {
                    let segment = segment.clone();
                    let (url, target, poster_story_id) = (segment.clone(), media.clone(), story.id);
                    cx.defer(move |cx| {
                        url.update(cx, |segment, cx| {
                            segment.poster_story_id = poster_story_id;
                            segment.set_url(target, cx);
                        })
                    });
                    segment
                }
                None => {
                    let (url, poster_story_id) = (media.clone(), story.id);
                    let segment = cx.new(|cx| {
                        let mut segment = StoryVideoSegment::new(url, true, cx);
                        segment.poster_story_id = poster_story_id;
                        segment
                    });
                    self.preview_video_segment = Some(segment.clone());
                    segment
                }
            };
            div().size_full().child(segment).into_any_element()
        } else if !media.is_empty() {
            self.preview_video_segment = None;
            div().size_full().child(StoryMediaView::new("story-preview-media", media, 0.0).shows_loader(true)).into_any_element()
        } else {
            self.preview_video_segment = None;
            Self::empty_layer(false).into_any_element()
        };
        let forbids = self.forbids_capture(story, cx);
        div()
            .absolute()
            .inset_0()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .child(ScreenCaptureProtectedUi::new("story-preview-capture", forbids, content)),
            )
            .children(self.story_music_plate(story))
            .into_any_element()
    }

    /// the author turned screenshots off so someone elses story is drawn out of captures
    fn forbids_capture(&self, story: &StoryDTO, cx: &App) -> bool {
        if story.allow_screenshots {
            return false;
        }
        !RiseIdentity::is_own(
            &RiseIdentity::author(
                Some(story.author_id.as_str()),
                None,
                story.author.as_ref().map(|author| author.id.as_str()),
            ),
            self.viewer_id(cx).as_deref(),
            None,
        )
    }

    fn begin_presentation(&mut self, size: Size<Pixels>, inset_top: Pixels, cx: &mut Context<Self>) {
        if self.did_start_presentation {
            return;
        }
        self.did_start_presentation = true;
        self.presentation_distance = f32::from(size.height + inset_top).max(1.0);
        if self.overlay_veiled {
            Self::after(0.48, cx, |this, _| {
                if !this.is_dismissing {
                    this.is_viewers_pager_mounted = true;
                }
            });
            Self::after(SharedElementMotion::WATCHDOG + 0.1, cx, |this, cx| {
                let surface = this.shared_element_destination.surface();
                if this.overlay_veiled
                    && this.is_presenting
                    && !SharedElementOverlayDriver::is_running_destination_surface(&surface)
                {
                    this.apply_shared_element_phase(SharedElementOverlayPhase::OpenAborted, cx);
                }
            });
            return;
        }
        self.drag_offset.jump(self.presentation_distance);
        self.drag_offset.set_in(0.0, spring(0.46, 0.9), RiseClock::monotonic(), cx);
        Self::after(0.48, cx, |this, _| {
            if this.is_dismissing {
                return;
            }
            this.is_presenting = false;
            this.is_viewers_pager_mounted = true;
        });
    }

    fn player_owns_media(&self, cx: &App) -> bool {
        let Some(story) = self.current_story() else {
            return true;
        };
        if !self.is_own_story(cx) || story.media_type == "video" {
            return true;
        }
        self.viewers_progress_target <= 0.001
    }

    fn player_card_opacity(viewers_progress: f32) -> f32 {
        (1.0 - viewers_progress / 0.35).max(0.0)
    }

    fn media_base_layer(&mut self, size: Size<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let story = self.current_story().cloned();
        let forbids = story.as_ref().is_some_and(|story| self.forbids_capture(story, cx));
        let content = match &story {
            Some(story) => self.media_layer(story, size, window, cx),
            None => {
                self.video_segment = None;
                self.video_segment_sync = None;
                let is_own = self.is_own_story(cx);
                Self::empty_layer(is_own).into_any_element()
            }
        };
        let chrome_opacity = self.chrome_alpha.frame_value(RiseClock::monotonic(), window);
        let plate = story
            .as_ref()
            .and_then(|story| self.story_music_plate(story))
            .map(|plate| div().absolute().inset_0().opacity(chrome_opacity).child(plate));
        div()
            .relative()
            .w(size.width)
            .h(size.height)
            .bg(black())
            .overflow_hidden()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .overflow_hidden()
                    .child(ScreenCaptureProtectedUi::new("story-media-capture", forbids, content)),
            )
            .children(plate)
            .into_any_element()
    }

    fn apply_media_geometry(&mut self, story_id: i64, aspect: Option<f32>, fills: bool) {
        self.media_geometry_story_id = Some(story_id);
        self.media_aspect = aspect;
        self.media_fills = fills;
    }

    fn story_music_plate(&self, story: &StoryDTO) -> Option<AnyElement> {
        let plate = story.track_plate.as_ref()?;
        let measured = self.media_geometry_story_id == Some(story.id);
        let aspect = if measured { self.media_aspect } else { None };
        let fills = if measured { self.media_fills } else { true };
        Some(
            div()
                .absolute()
                .inset_0()
                .child(
                    MusicPlateOverlay::new(
                        SharedString::from(format!("story-plate-{}", story.id)),
                        plate.title.clone(),
                        plate.artist.clone(),
                        plate.cover_url.clone(),
                        story.track_id.clone().unwrap_or_default(),
                        plate.is_dark,
                        plate.center_x,
                        plate.center_y,
                        plate.rotation,
                        plate.scale,
                    )
                    .media_aspect(aspect)
                    .media_fills(fills),
                )
                .into_any_element(),
        )
    }

    fn chrome_visual(&mut self, size: Size<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let chrome_opacity = self.chrome_alpha.frame_value(RiseClock::monotonic(), window);
        self.chrome_enabled = self.chrome_enabled && !self.chrome_hidden();
        let link_zones = self
            .current_story()
            .filter(|story| !story.link_areas.is_empty() && !self.chrome_hidden())
            .map(|story| self.link_zones_layer(story, cx));
        let user_pause_mark = self.user_pause_mark(chrome_opacity, window, cx);
        let link_hint = self.link_hint.clone().map(|hint| self.link_hint_layer(hint, size, window, cx));
        let content = div()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .opacity(chrome_opacity)
            .child(div().px(px(8.0)).pt(px(6.0)).child(StoryProgressBars::element(self.progress_bars.clone())))
            .child(div().px(px(12.0)).pt(px(7.0)).child(self.header(cx)))
            .child(div().flex_1())
            .children(self.caption().map(|caption| div().px(px(16.0)).pb(px(10.0)).child(caption)))
            .child(div().px(px(12.0)).pb(px(6.0)).child(self.bottom_bar(size, window, cx)));
        div()
            .relative()
            .w(size.width)
            .h(size.height)
            .child(div().absolute().inset_0().opacity(chrome_opacity).child(Self::top_scrim()))
            .child(div().absolute().inset_0().opacity(chrome_opacity).child(Self::bottom_scrim()))
            .children(link_zones)
            .child(content)
            .children(user_pause_mark)
            .children(link_hint)
            .into_any_element()
    }

    /// while pending the address is the app local copy, the row has no server address yet
    fn media_source(&self, story: &StoryDTO, cx: &App) -> (String, bool) {
        if let Some(pending) = self.service.read(cx).pending_story_media(story.id, cx) {
            return pending;
        }
        (story.media_source(), story.media_type == "video")
    }

    /// rebuilds the callbacks for the story on screen
    fn apply_video_callbacks(
        segment: &mut StoryVideoSegment,
        weak: WeakEntity<Self>,
        story_id: i64,
        media: String,
        size: Size<Pixels>,
    ) {
        segment.poster_story_id = story_id;
        {
            let (weak, media) = (weak.clone(), media.clone());
            segment.on_duration = Rc::new(move |seconds, cx| {
                let Some(player) = weak.upgrade() else {
                    return;
                };
                let clock = player.read(cx).playback_clock.clone();
                let media = media.clone();
                clock.update(cx, |clock, _| {
                    clock.video_durations.insert(media, seconds);
                });
            });
        }
        {
            let weak = weak.clone();
            segment.on_progress = Rc::new(move |fraction, cx| {
                let Some(player) = weak.upgrade() else {
                    return;
                };
                if player.read(cx).playback_is_paused(cx) {
                    return;
                }
                let clock = player.read(cx).playback_clock.clone();
                clock.update(cx, |clock, cx| {
                    clock.value = fraction.clamp(0.0, 1.0);
                    cx.notify();
                });
            });
        }
        {
            let (weak, media) = (weak.clone(), media.clone());
            segment.on_finished = Rc::new(move |cx| {
                weak.update(cx, |this, cx| {
                    if this.is_current_story_pending(cx) {
                        return;
                    }
                    let Some(current) = this.current_story().cloned() else {
                        return;
                    };
                    if this.media_source(&current, cx).0 != media {
                        return;
                    }
                    if this.playback_is_paused(cx) || this.is_pinching {
                        this.finished_video_story_id = Some(current.id);
                    } else {
                        this.go_next(cx);
                    }
                    cx.notify();
                })
                .ok();
            });
        }
        {
            let weak = weak.clone();
            segment.on_aspect = Rc::new(move |aspect, cx| {
                weak.update(cx, |this, cx| {
                    let fills = StoryVideoSegment::fills(
                        Some(aspect),
                        Size { width: f32::from(size.width), height: f32::from(size.height) },
                    );
                    this.apply_media_geometry(story_id, Some(aspect), fills);
                    this.shared_element_destination.record_video_aspect(story_id, aspect);
                    cx.notify();
                })
                .ok();
            });
        }
        {
            let (weak, media) = (weak.clone(), media.clone());
            segment.on_player = Some(Rc::new(move |player: Option<Entity<AVPlayer>>, cx| {
                let Some(this) = weak.upgrade() else {
                    return;
                };
                if let Some(player) = &player {
                    let clock = this.read(cx).playback_clock.clone();
                    let (media, downgraded) = (media.clone(), player.downgrade());
                    clock.update(cx, |clock, _| {
                        clock.video_players.insert(media, downgraded);
                    });
                }
                this.read(cx).shared_element_destination.record_video(story_id, player.as_ref());
            }));
        }
        {
            let weak = weak.clone();
            segment.on_ready_for_display = Some(Rc::new(move |ready, cx| {
                weak.update(cx, |this, cx| {
                    this.shared_element_destination.record_video_ready(story_id, ready);
                    if this.current_story().map(|story| story.id) == Some(story_id) {
                        this.meaningful_ready_story_id = ready.then_some(story_id);
                        cx.notify();
                    }
                })
                .ok();
            }));
        }
        segment.loops_at_end =
            Rc::new(move |cx| weak.upgrade().is_some_and(|player| player.read(cx).is_current_story_pending(cx)));
    }

    fn video_segment_for(
        &mut self,
        story_id: i64,
        media: &str,
        size: Size<Pixels>,
        cx: &mut Context<Self>,
    ) -> Entity<StoryVideoSegment> {
        let is_paused = self.playback_is_paused(cx);
        if self.video_segment_sync.as_ref().is_none_or(|old| old.story_id != story_id || old.media != media || old.is_paused != is_paused) {
            tracing::warn!(
                target: "riseonly::X1291",
                story_id,
                media,
                is_paused,
                paused = self.is_paused,
                user = self.is_user_paused,
                link_hint = self.link_hint.is_some(),
                link_sheet = self.link_sheet_open,
                audio_story = self.audio_story.is_some(),
                composer = self.audio_story_composer_active,
                scene_active = self.scene_is_active,
                pending = self.is_current_story_pending(cx),
                "video segment sync"
            );
        }
        let sync = VideoSegmentSync { story_id, media: media.to_owned(), is_paused, size };
        let weak = cx.weak_entity();
        if let Some(segment) = self.video_segment.clone() {
            if self.video_segment_sync.as_ref() != Some(&sync) {
                self.video_segment_sync = Some(sync);
                let (target, media) = (segment.clone(), media.to_owned());
                cx.defer(move |cx| {
                    target.update(cx, |segment, cx| {
                        Self::apply_video_callbacks(segment, weak, story_id, media.clone(), size);
                        segment.set_url(media, cx);
                        segment.set_is_paused(is_paused, cx);
                    });
                });
            }
            return segment;
        }
        let url = media.to_owned();
        let segment = cx.new(|cx| {
            let mut segment = StoryVideoSegment::new(url.clone(), is_paused, cx);
            Self::apply_video_callbacks(&mut segment, weak, story_id, url, size);
            segment
        });
        self.video_segment = Some(segment.clone());
        self.video_segment_sync = Some(sync);
        segment
    }

    fn media_layer(
        &mut self,
        story: &StoryDTO,
        size: Size<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (media, is_video) = self.media_source(story, cx);
        if is_video && !media.is_empty() {
            let segment = self.video_segment_for(story.id, &media, size, cx);
            return div().size_full().child(segment).into_any_element();
        }
        self.video_segment = None;
        self.video_segment_sync = None;
        if media.is_empty() {
            return div().size_full().bg(black()).into_any_element();
        }
        let story_id = story.id;
        let weak = cx.weak_entity();
        div()
            .size_full()
            .child(
                StoryMediaView::new("story-player-media", media, 0.0)
                    .on_media_geometry(move |aspect, fills, _, cx| {
                        weak.update(cx, |this, cx| {
                            this.apply_media_geometry(story_id, aspect, fills);
                            this.shared_element_destination.record_photo_geometry(story_id, aspect, fills);
                            if this.current_story().map(|story| story.id) == Some(story_id) {
                                this.meaningful_ready_story_id = aspect.map(|_| story_id);
                            }
                            cx.notify();
                        })
                        .ok();
                    })
                    .geometry_key(story_id)
                    .shows_loader(true),
            )
            .into_any_element()
    }

    fn empty_layer(is_own: bool) -> impl IntoElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(10.0))
            .children(IconUi::sized("photo.on.rectangle.angled", px(32.0), white().opacity(0.6), false))
            .child(
                MainText::new(tr(if is_own { "story_no_stories" } else { "chat_story_reply_expired" }))
                    .px(14.0)
                    .color(white().opacity(0.8)),
            )
    }

    fn top_scrim() -> impl IntoElement {
        div().absolute().top_0().left_0().right_0().h(px(150.0)).child(
            LinearGradient::colors([black().opacity(0.5), transparent_black()], UnitPoint::TOP, UnitPoint::BOTTOM)
                .view()
                .size_full(),
        )
    }

    fn bottom_scrim() -> impl IntoElement {
        div().absolute().bottom_0().left_0().right_0().h(px(220.0)).child(
            LinearGradient::colors([transparent_black(), black().opacity(0.6)], UnitPoint::TOP, UnitPoint::BOTTOM)
                .view()
                .size_full(),
        )
    }

    /// a press on the chrome stays on the chrome, the tap zones lie under it
    fn chrome_button(
        &self,
        id: impl Into<ElementId>,
        label: Option<String>,
        width: f32,
        height: f32,
        disabled: bool,
        content: impl IntoElement,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> AnyElement {
        div()
            .id(id.into())
            .flex_none()
            .when(self.chrome_enabled, |it| it.occlude())
            .when_some(label, |it, label| {
                let label = SharedString::from(label);
                it.aria_label(label.clone()).tooltip_label(label)
            })
            .child(
                plain("press")
                    .disabled(disabled || !self.chrome_enabled)
                    .on_press(action)
                    .w(px(width))
                    .h(px(height))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(content),
            )
            .into_any_element()
    }

    fn header(&self, cx: &mut Context<Self>) -> AnyElement {
        let author = self.author().cloned();
        let author_id = self.current_author_id();
        let story = self.current_story().cloned();
        let track_id = story.as_ref().and_then(|story| story.track_id.clone()).filter(|id| !id.is_empty());
        let name = author.as_ref().map(|author| author.name.clone()).unwrap_or_default();
        let open_profile = Rc::new(Self::press_action(cx, |this, _, cx| this.open_author_profile(cx)));

        let logo = {
            let open_profile = open_profile.clone();
            div()
                .id("story-author-logo")
                .flex_none()
                .when(self.chrome_enabled, |it| it.occlude())
                .aria_label(SharedString::from(name.clone()))
                .child(
                    plain("press").disabled(author_id.is_empty() || !self.chrome_enabled).on_press(move |window, cx| open_profile(window, cx)).child(
                        UserLogoUi::new("story-author-logo-image")
                            .source(author.as_ref().map(|author| SharedString::from(author.logo.clone())))
                            .name(author.as_ref().map(|author| SharedString::from(author.name.clone())))
                            .user_id(Some(SharedString::from(author_id.clone())))
                            .size(32.0)
                            .show_border(false)
                            .show_story_ring(false),
                    ),
                )
        };

        let premium = author.as_ref().filter(|author| author.is_premium).map(|author| {
            PremiumBadge::new()
                .user_id(Some(SharedString::from(author.id.clone())))
                .emoji(author.premium_badge_emoji.clone().map(SharedString::from))
                .size(13.0)
        });
        let name_button = {
            let open_profile = open_profile.clone();
            div().id("story-author-name").min_w_0().when(self.chrome_enabled, |it| it.occlude()).child(
                plain("press")
                    .disabled(author_id.is_empty() || !self.chrome_enabled)
                    .on_press(move |window, cx| open_profile(window, cx))
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(4.0))
                    .child(
                        div().min_w_0().overflow_hidden().child(
                            MainText::new(name)
                                .px(14.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(white())
                                .number_of_lines(1),
                        ),
                    )
                    .children(premium.map(|badge| div().flex_none().child(badge))),
            )
        };
        let title_row = div()
            .w_full()
            .min_w_0()
            .flex()
            .items_center()
            .gap(px(6.0))
            .child(name_button)
            .when(track_id.is_some(), |it| it.child(div().flex_none().child(self.story_date())));

        let subtitle = match (&story, &track_id) {
            (Some(story), Some(track_id)) => div()
                .w_full()
                .when(self.chrome_enabled, |it| it.occlude())
                .child(StoryAudioLabel::new(
                    story.clone(),
                    track_id.clone(),
                    Self::press_action(cx, |this, window, cx| this.open_audio(window, cx)),
                ))
                .into_any_element(),
            _ => self.story_date().into_any_element(),
        };

        let menu = (!self.is_current_story_pending(cx)).then(|| self.menu_button(cx));
        let close = self.chrome_button(
            "story-close",
            Some(tr("story_action_close")),
            34.0,
            34.0,
            false,
            div().children(IconUi::sized("xmark", px(16.0), white(), false)),
            Self::press_action(cx, |this, _, cx| {
                let _ = selection_changed();
                this.dismiss(true, cx);
            }),
        );

        div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(logo)
            .child(div().flex_1().min_w_0().flex().flex_col().items_start().gap(px(2.0)).child(title_row).child(subtitle))
            .children(menu.map(|menu| div().flex_none().child(menu)))
            .child(div().flex_none().child(close))
            .into_any_element()
    }

    fn story_date(&self) -> AnyElement {
        MainText::new(Self::story_time_line(self.current_story()))
            .px(11.0)
            .color(white().opacity(0.6))
            .number_of_lines(1)
            .into_any_element()
    }

    fn menu_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let author_id = self.current_author_id();
        let mut items = Vec::new();
        let delete_item = |items: &mut Vec<ContextMenuItem>, cx: &Context<Self>| {
            items.push(
                ContextMenuItem::new(
                    items.len() as i64,
                    tr("story_menu_delete"),
                    Some("trash".to_owned()),
                    "delete",
                    Self::press_action(cx, |this, _, cx| this.request_delete(cx)),
                )
                .destructive(true),
            );
        };
        if self.is_own_story(cx) {
            if let Some(story) = self.current_story()
                && self.service.read(cx).pending_story_media(story.id, cx).is_none()
            {
                items.push(ContextMenuItem::new(
                    items.len() as i64,
                    tr("post_download_action"),
                    Some("arrow.down.to.line".to_owned()),
                    "download",
                    Self::press_action(cx, |this, _, cx| {
                        let Some(story) = this.current_story().cloned() else {
                            return;
                        };
                        StoryDownloadStore::shared(cx).update(cx, |store, cx| store.start(story, 0.0, cx));
                    }),
                ));
            }
            delete_item(&mut items, cx);
        } else {
            if self.auth_service.read(cx).viewer_acts_as_author() {
                delete_item(&mut items, cx);
            }
            items.push(ContextMenuItem::new(
                items.len() as i64,
                tr("story_menu_report"),
                Some("exclamationmark.bubble".to_owned()),
                "report",
                Self::press_action(cx, |this, _, cx| {
                    let story_id = this.current_story().map(|story| story.id);
                    let author_id = this.current_author_id();
                    StoryInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.report_story(story_id, Some(author_id.as_str()), cx));
                }),
            ));
            items.push(ContextMenuItem::new(
                items.len() as i64,
                tr("story_menu_hide_user"),
                Some("eye.slash".to_owned()),
                "hide-user",
                Self::press_action(cx, |this, _, cx| {
                    let author_id = this.current_author_id();
                    StoryInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.mute_author(Some(author_id.as_str()), cx));
                }),
            ));
            let (stealth_title, stealth_icon) = if self.stealth_enabled {
                (tr("story_menu_stealth_off"), "eye")
            } else {
                (tr("story_menu_stealth_on"), "theatermasks")
            };
            items.push(ContextMenuItem::new(
                items.len() as i64,
                stealth_title,
                Some(stealth_icon.to_owned()),
                "stealth",
                Self::press_action(cx, |this, _, cx| {
                    let is_premium =
                        this.auth_service.read(cx).profile().is_some_and(|profile| profile.more.is_premium);
                    if is_premium {
                        StoryStealthPreference::set_is_enabled(!StoryStealthPreference::is_enabled());
                        this.stealth_enabled = StoryStealthPreference::is_enabled();
                        let _ = selection_changed();
                    } else {
                        PremiumUpsellStore::shared(cx).update(cx, |upsell, cx| {
                            upsell.offer_premium_feature(
                                tr("premium_upsell_reason_story_stealth"),
                                "story_stealth",
                                false,
                                cx,
                            );
                        });
                    }
                }),
            ));
            if self.relationship_state_is_known(cx) {
                let relationship = self.user_service.update(cx, |user, cx| user.current_follow(&author_id, cx));
                let is_follow_interaction_pending =
                    self.user_service.read(cx).is_follow_interaction_pending(&author_id);
                let (title, icon) = if relationship.is_friend {
                    (tr("story_menu_remove_friend"), "person.2.slash")
                } else if relationship.is_subbed {
                    (tr("story_menu_unfollow"), "person.badge.minus")
                } else {
                    (tr("story_menu_follow"), "person.badge.plus")
                };
                items.push(
                    ContextMenuItem::new(
                        items.len() as i64,
                        title,
                        Some(icon.to_owned()),
                        "relationship",
                        Self::press_action(cx, |this, _, cx| {
                            let author_id = this.current_author_id();
                            let tag = this.author().map(|author| author.tag.clone());
                            StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
                                store.toggle_author_relationship(Some(author_id.as_str()), tag.as_deref(), cx);
                            });
                        }),
                    )
                    .disabled(is_follow_interaction_pending),
                );
            } else {
                items.push(
                    ContextMenuItem::new(
                        items.len() as i64,
                        tr("story_menu_loading_relationship"),
                        Some("hourglass".to_owned()),
                        "relationship-loading",
                        |_, _| {},
                    )
                    .disabled(true),
                );
            }
        }
        let presented = Self::menu_presented_handler(cx);
        div()
            .id("story-menu-host")
            .flex_none()
            .when(self.chrome_enabled, |it| it.occlude())
            .aria_label(SharedString::from(tr("story_action_menu")))
            .tooltip_label(tr("story_action_menu"))
            .child(
                ContextMenuUi::new(
                    "story-menu",
                    items,
                    div()
                        .size(px(34.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .children(IconUi::sized("ellipsis", px(17.0), white(), false)),
                )
                .shows_item_icons()
                .on_presented_change(presented),
            )
            .into_any_element()
    }

    fn menu_presented_handler(cx: &Context<Self>) -> impl Fn(bool, &mut Window, &mut App) + 'static {
        let weak = cx.weak_entity();
        move |presented: bool, _: &mut Window, cx: &mut App| {
            weak.update(cx, |this, cx| {
                if presented {
                    this.is_paused = true;
                    this.stop();
                    this.reload_unknown_relationship(cx);
                }
                this.is_story_menu_presented = presented;
                cx.notify();
            })
            .ok();
        }
    }

    fn request_delete(&mut self, cx: &mut Context<Self>) {
        let Some(story) = self.current_story().cloned() else {
            return;
        };
        if NativeConfirmAlert::is_presenting(cx) {
            return;
        }
        self.story_pending_deletion = Some(story);
        let message = if self.service.read(cx).active_highlight_id().is_some() && self.current_stories().len() <= 1 {
            tr("highlight_remove_last_story_message")
        } else {
            tr("story_delete_confirm_message")
        };
        let (cancelled, confirmed) = (cx.weak_entity(), cx.weak_entity());
        NativeConfirmAlert::present_delete(
            &tr("story_delete_confirm_title"),
            &message,
            None,
            None,
            false,
            None,
            Some(Box::new(move |cx: &mut App| {
                cx.defer(move |cx| {
                    cancelled
                        .update(cx, |this, cx| {
                            this.story_pending_deletion = None;
                            cx.notify();
                        })
                        .ok();
                });
            })),
            move |cx| {
                confirmed
                    .update(cx, |this, cx| {
                        let Some(story) = this.story_pending_deletion.take() else {
                            return;
                        };
                        let author_id = (!this.is_own_story(cx)).then(|| this.current_author_id());
                        this.perform_delete(story.id, author_id.as_deref(), cx);
                        cx.notify();
                    })
                    .ok();
            },
            cx,
        );
    }

    fn perform_delete(&mut self, story_id: i64, author_id: Option<&str>, cx: &mut Context<Self>) {
        StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.delete_story(story_id, author_id, cx));
        self.go_after_delete(cx);
    }

    fn observe_socket_did_connect(cx: &mut Context<Self>) -> (RiseSubscription, Task<()>) {
        let (signal, mut signals) = futures::channel::mpsc::unbounded::<()>();
        let subscription = RiseAppEvents::default_center().observe(SAI_WS_DID_CONNECT, move |_| {
            let _ = signal.unbounded_send(());
        });
        let task = cx.spawn(async move |this, cx| {
            while futures::StreamExt::next(&mut signals).await.is_some() {
                if this.update(cx, |this, cx| this.reload_unknown_relationship(cx)).is_err() {
                    break;
                }
            }
        });
        (subscription, task)
    }

    fn reload_unknown_relationship(&mut self, cx: &mut Context<Self>) {
        if self.is_own_story(cx) || self.relationship_state_is_known(cx) {
            return;
        }
        let author_id = self.current_author_id();
        if author_id.is_empty() {
            return;
        }
        tracing::debug!(target: "riseonly::X1718", "story menu reloads author relationship");
        UserInteractionsStore::shared(cx).update(cx, |user, cx| user.load_foreign_profile_user_id(author_id, cx));
    }

    fn relationship_state_is_known(&self, cx: &mut Context<Self>) -> bool {
        let author_id = self.current_author_id();
        if author_id.is_empty() {
            return false;
        }
        self.user_service.update(cx, |user, cx| {
            let relationship = user.current_follow(&author_id, cx);
            relationship.is_subbed
                || relationship.is_subscriber
                || relationship.is_friend
                || user.cached_foreign_profile_user_id(&author_id, None, cx).is_some()
        })
    }

    fn caption(&self) -> Option<impl IntoElement + use<>> {
        let caption = self.current_story()?.caption.as_deref().map(trimming_whitespaces_and_newlines)?;
        if caption.is_empty() {
            return None;
        }
        Some(div().w_full().child(MainText::new(caption.to_owned()).px(15.0).color(white()).number_of_lines(6)))
    }

    fn bottom_bar(&mut self, size: Size<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if self.is_current_story_pending(cx) {
            div().into_any_element()
        } else if self.is_own_story(cx) {
            self.owner_bottom_bar(size, window, cx)
        } else {
            self.viewer_bottom_bar(cx)
        }
    }

    fn viewer_bottom_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let liked = self.current_story().is_some_and(|story| self.service.read(cx).is_story_liked(story, cx));
        let has_reply = !trimming_whitespaces_and_newlines(self.reply_input.read(cx).text()).is_empty();
        let send = has_reply.then(|| {
            self.chrome_button(
                "story-send-reply",
                Some(tr("story_action_send_reply")),
                24.0,
                24.0,
                false,
                div().children(IconUi::sized("paperplane.fill", px(16.0), white(), true)),
                Self::press_action(cx, |this, window, cx| this.send_reply(window, cx)),
            )
        });
        let reply = div()
            .flex_1()
            .min_w_0()
            .h(px(44.0))
            .px(px(16.0))
            .flex()
            .items_center()
            .gap(px(8.0))
            .rounded_full()
            .border(px(1.2))
            .border_color(white().opacity(0.55))
            .when(self.chrome_enabled, |it| it.occlude())
            .child(div().flex_1().min_w_0().child(InputUi::new(self.reply_input.clone())))
            .children(send);
        let like_color = if liked { system_red(cx) } else { white() };
        let like = self.chrome_button(
            "story-like",
            Some(tr("story_action_like")),
            40.0,
            44.0,
            false,
            div().children(IconUi::sized(if liked { "heart.fill" } else { "heart" }, px(24.0), like_color, liked)),
            Self::press_action(cx, |this, _, cx| {
                let Some(story) = this.current_story().cloned() else {
                    return;
                };
                let _ = impact(ImpactStyle::Light);
                StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_like(&story, cx));
            }),
        );
        let chat_content = if self.is_opening_chat {
            progress_view().tint(white()).into_any_element()
        } else {
            div().children(IconUi::sized("bubble.left", px(22.0), white(), false)).into_any_element()
        };
        let chat = self.chrome_button(
            "story-chat",
            Some(tr("story_action_chat")),
            40.0,
            44.0,
            self.is_opening_chat || self.current_author_id().is_empty(),
            chat_content,
            Self::press_action(cx, |this, _, cx| this.open_author_chat(cx)),
        );
        div()
            .w_full()
            .flex()
            .items_center()
            .gap(px(10.0))
            .child(reply)
            .child(like)
            .child(chat)
            .child(self.share_button(cx))
            .into_any_element()
    }

    fn share_button(&self, cx: &Context<Self>) -> AnyElement {
        self.chrome_button(
            "story-share",
            Some(tr("story_action_share")),
            40.0,
            44.0,
            false,
            div().children(IconUi::sized("paperplane", px(22.0), white(), false)),
            Self::press_action(cx, |this, _, cx| {
                let _ = selection_changed();
                this.present_share(cx);
            }),
        )
    }

    fn owner_bottom_bar(&mut self, size: Size<Pixels>, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let story = self.current_story().cloned();
        let story_id = story.as_ref().map_or(0, |story| story.id);
        let service = self.service.clone();
        let (viewers, viewers_total, like_count) = service.update(cx, |service, cx| {
            let viewers: Vec<StoryViewerDTO> = service.story_viewers(story_id, cx).into_iter().take(3).collect();
            let total = service.story_viewers_total(story_id, cx);
            let likes = story.as_ref().map_or(0, |story| service.own_story_like_count(story, cx));
            (viewers, total, likes)
        });
        let view_count = story.as_ref().map_or(0, |story| story.view_count).max(viewers_total);
        self.on_change_do("owner-story-id", story_id, true, window, cx, |_, _, id, cx| {
            if *id != 0 {
                let id = *id;
                StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.load_story_viewers(id, None, cx));
            }
        });

        let counts = div()
            .w_full()
            .h(px(44.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .when(self.chrome_enabled, |it| it.occlude())
            .when(!viewers.is_empty(), |it| it.child(Self::avatar_stack(&viewers)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .children(IconUi::sized("eye.fill", px(13.0), white(), true))
                    .child(
                        MainText::new(view_count.to_string()).px(14.0).font_weight(FontWeight::SEMIBOLD).color(white()),
                    ),
            )
            .when(like_count > 0, |it| {
                it.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(5.0))
                        .children(IconUi::sized("heart.fill", px(13.0), system_red(cx), true))
                        .child(
                            MainText::new(like_count.to_string())
                                .px(14.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(white()),
                        ),
                )
            })
            .child(div().flex_1())
            .child({
                let weak = cx.weak_entity();
                measure("story-owner-bar-origin", move |bounds, _, cx| {
                    weak.update(cx, |this, _| this.owner_bar_origin = Some(bounds.origin)).ok();
                })
            });
        let height = f32::from(size.height);
        div()
            .w_full()
            .h(px(44.0))
            .flex()
            .items_center()
            .gap(px(12.0))
            .child(
                div()
                    .id("story-viewers-bar")
                    .flex_1()
                    .min_w_0()
                    .aria_label(SharedString::from(tr("story_action_viewers")))
                    .child(counts.gesture("story-owner-bar-gesture", self.owner_bar_gesture(height, cx))),
            )
            .child(self.share_button(cx))
            .into_any_element()
    }

    fn avatar_stack(viewers: &[StoryViewerDTO]) -> impl IntoElement {
        div().flex().flex_row_reverse().children(viewers.iter().enumerate().rev().map(|(idx, viewer)| {
            div()
                .relative()
                .when(idx + 1 != viewers.len(), |it| it.mr(px(-12.0)))
                .child(
                    UserLogoUi::new(SharedString::from(format!("story-viewer-{}", viewer.id)))
                        .source(Some(SharedString::from(viewer.logo.clone())))
                        .name(Some(SharedString::from(viewer.name.clone())))
                        .user_id(Some(SharedString::from(viewer.id.clone())))
                        .size(30.0)
                        .show_border(false)
                        .show_story_ring(false),
                )
                .child(div().absolute().inset_0().rounded_full().border(px(2.0)).border_color(black()))
        }))
    }

    fn open_viewers(&mut self, cx: &mut Context<Self>) {
        let _ = selection_changed();
        self.ensure_owner_stories_loaded(cx);
        self.is_viewers_pager_mounted = true;
        self.viewers_progress_target = 1.0;
        self.viewers_progress.update(cx, |progress, cx| {
            progress.set_in(1.0, spring(0.4, 0.86), RiseClock::monotonic(), cx);
            cx.notify();
        });
        cx.notify();
    }

    fn close_viewers(&mut self, cx: &mut Context<Self>) {
        self.viewers_progress_target = 0.0;
        self.viewers_progress.update(cx, |progress, cx| {
            progress.set_in(0.0, spring(0.36, 0.86), RiseClock::monotonic(), cx);
            cx.notify();
        });
        cx.notify();
    }

    /// sheet pages the same list as the player
    /// if caller gave one, asking the server for the feed is harmful
    /// else the group rebuilds and the screen goes black
    fn ensure_owner_stories_loaded(&mut self, cx: &mut Context<Self>) {
        let author_id = self.current_author_id();
        if self.request.owns_explicit_stories || author_id.is_empty() {
            return;
        }
        StoryInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.load_user_stories(Some(author_id.as_str()), cx));
    }

    /// one owner per axis
    /// while an up swipe drives the viewers sheet the gesture layer must keep hit testing
    /// dropping it under a live gesture kills it and the sheet falls back
    fn is_viewers_drag_active(&self) -> bool {
        self.drag_axis == DragAxis::Viewers || self.bar_drag_axis == DragAxis::Viewers
    }

    fn set_holding_to_pause(&mut self, holding: bool, cx: &mut Context<Self>) {
        if self.is_holding_to_pause == holding {
            return;
        }
        let was_hidden = self.chrome_hidden();
        self.is_holding_to_pause = holding;
        self.sync_chrome_alpha(was_hidden, 0.2, cx);
    }

    /// a hold or pinch shows the picture alone
    fn chrome_hidden(&self) -> bool {
        self.is_holding_to_pause || self.is_pinching
    }

    fn sync_chrome_alpha(&mut self, was_hidden: bool, duration: f64, cx: &mut Context<Self>) {
        let hidden = self.chrome_hidden();
        if hidden != was_hidden {
            self.chrome_alpha.set_in(if hidden { 0.0 } else { 1.0 }, ease_in_out(duration), RiseClock::monotonic(), cx);
        }
    }

    /// first frame of the gesture must be empty, sheet already mounted
    /// no network, no animations
    /// hold highlight is cleared without animation or its fade fights the sheet opening
    fn begin_viewers_drag(&mut self) {
        self.hold_task = None;
        self.is_paused = true;
        self.is_holding_to_pause = false;
        self.chrome_alpha.jump(1.0);
        self.drag_offset.jump(0.0);
        self.is_viewers_pager_mounted = true;
    }

    fn update_viewers_drag(&mut self, dy: f32, height: f32, cx: &mut Context<Self>) {
        let span = (height * 0.45).max(1.0);
        let target = (-dy / span).clamp(0.0, 1.0);
        self.viewers_progress_target = target;
        self.viewers_progress.update(cx, |progress, cx| {
            progress.jump(target);
            cx.notify();
        });
    }

    /// an up swipe is an open not a step, it settles straight to the full detent
    /// the half detent stays inside the pager
    fn finish_viewers_drag(&mut self, dy: f32, predicted: f32, cx: &mut Context<Self>) {
        let target = if -dy > 60.0 || predicted < -160.0 { 1.0 } else { 0.0 };
        self.viewers_progress_target = target;
        self.viewers_progress.update(cx, |progress, cx| {
            progress.set_in(target, spring(0.38, 0.86), RiseClock::monotonic(), cx);
            cx.notify();
        });
        if target != 0.0 {
            self.ensure_owner_stories_loaded(cx);
            return;
        }
        if !self.reply_focused {
            self.is_paused = false;
        }
    }

    fn owner_bar_gesture(&self, height: f32, cx: &Context<Self>) -> DragGesture {
        let (changed, ended) = (cx.weak_entity(), cx.weak_entity());
        DragGesture::new(0.0)
            .on_changed(move |value, _, cx| {
                changed
                    .update(cx, |this, cx| {
                        this.owner_bar_changed(value, height, cx);
                        cx.notify();
                    })
                    .ok();
            })
            .on_ended(move |value, _, cx| {
                ended
                    .update(cx, |this, cx| {
                        this.owner_bar_ended(value, cx);
                        cx.notify();
                    })
                    .ok();
            })
    }

    fn owner_bar_changed(&mut self, value: &DragGestureValue, height: f32, cx: &mut Context<Self>) {
        let dx = f32::from(value.translation.width);
        let dy = f32::from(value.translation.height);
        if self.bar_drag_axis == DragAxis::Undecided {
            if dy < -8.0 && dy.abs() > dx.abs() {
                self.bar_drag_axis = DragAxis::Viewers;
                self.begin_viewers_drag();
            } else if dy > 8.0 && dy.abs() > dx.abs() {
                self.bar_drag_axis = DragAxis::Dismiss;
                self.is_paused = true;
                self.stop();
                let touch_start = self.global_screen_point(value.start_location);
                self.begin_shared_element_dismiss_drag(touch_start, cx);
            }
        }
        match self.bar_drag_axis {
            DragAxis::Viewers => self.update_viewers_drag(dy, height, cx),
            DragAxis::Dismiss => {
                if !self.follow_shared_element_dismiss_drag(value, cx) {
                    self.drag_offset.jump(dy.max(0.0));
                }
            }
            DragAxis::Undecided | DragAxis::Horizontal => {}
        }
    }

    fn owner_bar_ended(&mut self, value: &DragGestureValue, cx: &mut Context<Self>) {
        let axis = self.bar_drag_axis;
        self.bar_drag_axis = DragAxis::Undecided;
        let dx = f32::from(value.translation.width);
        let dy = f32::from(value.translation.height);
        let predicted = f32::from(value.predicted_end_translation.height);
        match axis {
            DragAxis::Viewers => self.finish_viewers_drag(dy, predicted, cx),
            DragAxis::Dismiss => {
                if self.release_shared_element_dismiss_drag(value, cx) {
                    return;
                }
                if dy > 120.0 || predicted > 340.0 {
                    self.dismiss(true, cx);
                } else {
                    self.drag_offset.set_in(0.0, interactive_spring(0.3, 0.86, 0.25), RiseClock::monotonic(), cx);
                    self.resume_after_dismiss_cancellation(cx);
                }
            }
            DragAxis::Undecided | DragAxis::Horizontal => {
                if dx.abs() <= 12.0 && dy.abs() <= 12.0 {
                    self.open_viewers(cx);
                }
            }
        }
    }

    fn tap_zones(&self, size: Size<Pixels>, cx: &Context<Self>) -> AnyElement {
        let (changed, ended) = (cx.weak_entity(), cx.weak_entity());
        let gesture = DragGesture::new(0.0)
            .on_changed(move |value, _, cx| {
                changed
                    .update(cx, |this, cx| {
                        this.tap_zones_changed(value, cx);
                        cx.notify();
                    })
                    .ok();
            })
            .on_ended(move |value, _, cx| {
                ended
                    .update(cx, |this, cx| {
                        this.tap_zones_ended(value, cx);
                        cx.notify();
                    })
                    .ok();
            });
        div()
            .absolute()
            .top(self.stable_inset_top)
            .left_0()
            .child(div().w(size.width).h(size.height).gesture("story-tap-zones", gesture))
            .into_any_element()
    }

    fn tap_zones_changed(&mut self, value: &DragGestureValue, cx: &mut Context<Self>) {
        if self.press_start.is_none() {
            self.press_start = Some(RiseClock::monotonic());
            self.drag_axis = DragAxis::Undecided;
            self.movement_cancelled_hold = false;
            self.pinch_in_press = false;
            self.hold_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
                cx.background_executor().timer(Duration::from_millis(200)).await;
                this.update(cx, |this, cx| {
                    if this.drag_offset_target() == 0.0
                        && this.drag_axis == DragAxis::Undecided
                        && !this.movement_cancelled_hold
                        && !this.pinch_in_press
                    {
                        this.is_paused = true;
                        this.set_holding_to_pause(true, cx);
                        cx.notify();
                    }
                })
                .ok();
            }));
        }
        if self.pinch_in_press {
            return;
        }
        let dy = f32::from(value.translation.height);
        let dx = f32::from(value.translation.width);

        if !self.movement_cancelled_hold && (dx.abs() > 4.0 || dy.abs() > 4.0) {
            self.movement_cancelled_hold = true;
            self.hold_task = None;
            if self.is_holding_to_pause {
                self.set_holding_to_pause(false, cx);
            }
        }

        if self.drag_axis == DragAxis::Undecided {
            if dy > 8.0 && dy > dx.abs() * 1.1 {
                self.drag_axis = DragAxis::Dismiss;
                self.hold_task = None;
                self.is_paused = true;
                self.stop();
                self.set_holding_to_pause(false, cx);
                let touch_start = self.tap_zones_screen_point(value.start_location, cx);
                self.begin_shared_element_dismiss_drag(touch_start, cx);
            } else if dy < -10.0 && dy.abs() > dx.abs() && self.is_own_story(cx) && !self.is_current_story_pending(cx) {
                self.drag_axis = DragAxis::Viewers;
                self.begin_viewers_drag();
            } else if dx.abs() > 14.0 && dx.abs() > dy.abs() {
                self.drag_axis = DragAxis::Horizontal;
                self.hold_task = None;
                self.is_paused = true;
                self.stop();
                self.set_holding_to_pause(false, cx);
                self.drag_offset.jump(0.0);
            }
        }

        match self.drag_axis {
            DragAxis::Dismiss => {
                if !self.follow_shared_element_dismiss_drag(value, cx) {
                    self.drag_offset.jump(dy.max(0.0));
                }
            }
            DragAxis::Viewers => self.update_viewers_drag(dy, f32::from(self.geo_size.height), cx),
            DragAxis::Undecided => self.drag_offset.jump(dy.max(0.0)),
            DragAxis::Horizontal => self.update_horizontal_drag(dx),
        }
    }

    fn tap_zones_ended(&mut self, value: &DragGestureValue, cx: &mut Context<Self>) {
        self.hold_task = None;
        self.set_holding_to_pause(false, cx);
        let now = RiseClock::monotonic();
        let held = self.press_start.take().map_or(0.0, |start| now.saturating_sub(start).as_secs_f64());
        self.movement_cancelled_hold = false;
        let dy = f32::from(value.translation.height);
        let dx = f32::from(value.translation.width);
        let predicted_height = f32::from(value.predicted_end_translation.height);
        let axis = self.drag_axis;
        self.drag_axis = DragAxis::Undecided;

        if self.pinch_in_press {
            self.pinch_in_press = false;
            self.settle_after_pinch_press(cx);
            return;
        }

        match axis {
            DragAxis::Dismiss => {
                if self.release_shared_element_dismiss_drag(value, cx) {
                    return;
                }
                if dy > 120.0 || predicted_height > 340.0 {
                    self.dismiss(true, cx);
                } else {
                    self.drag_offset.set_in(0.0, interactive_spring(0.3, 0.86, 0.25), now, cx);
                    self.resume_after_dismiss_cancellation(cx);
                }
                return;
            }
            DragAxis::Viewers => {
                self.finish_viewers_drag(dy, predicted_height, cx);
                return;
            }
            DragAxis::Horizontal => {
                self.finish_horizontal_drag(
                    dx,
                    f32::from(value.predicted_end_translation.width),
                    f32::from(self.geo_size.width),
                    cx,
                );
                return;
            }
            DragAxis::Undecided => {}
        }

        if self.drag_offset_target() != 0.0 {
            self.drag_offset.set_in(0.0, interactive_spring(0.3, 0.86, 0.25), now, cx);
        }

        let moved = dx.abs() > 12.0 || dy.abs() > 12.0;
        if held >= 0.2 || moved {
            if !self.viewers_open() && !self.reply_focused {
                self.is_paused = false;
            }
            return;
        }

        if f32::from(value.location.x) < f32::from(self.geo_size.width) * 0.32 {
            self.go_prev(cx);
        } else if f32::from(value.location.x) <= f32::from(self.geo_size.width) * 0.68 {
            self.toggle_user_pause(cx);
        } else if !self.is_current_story_pending(cx) {
            self.go_next(cx);
        }
    }

    fn toggle_user_pause(&mut self, cx: &mut Context<Self>) {
        self.is_user_paused = !self.is_user_paused;
        let target = if self.is_user_paused { 1.0 } else { 0.0 };
        self.user_pause_alpha.set_in(target, ease_in_out(0.18), RiseClock::monotonic(), cx);
    }

    fn pinch_zoom_enabled(&self) -> bool {
        !self.is_presenting
            && !self.is_dismissing
            && !self.is_cube_transitioning
            && self.current_story().is_some()
            && self.audio_story.is_none()
            && !self.audio_story_composer_active
            && !self.viewers_open()
            && !self.reply_focused
            && !self.is_story_menu_presented
            && self.story_pending_deletion.is_none()
            && self.drag_axis == DragAxis::Undecided
            && self.bar_drag_axis == DragAxis::Undecided
    }

    fn pinch_active_changed(&mut self, active: bool, cx: &mut Context<Self>) {
        if active {
            self.pinch_in_press = true;
            self.hold_task = None;
            self.movement_cancelled_hold = true;
            if self.drag_offset_target() != 0.0 {
                self.drag_offset.set_in(0.0, interactive_spring(0.3, 0.86, 0.25), RiseClock::monotonic(), cx);
            }
        }
        let was_hidden = self.chrome_hidden();
        self.is_pinching = active;
        self.sync_chrome_alpha(was_hidden, 0.16, cx);
        if !active {
            self.advance_if_finished_under_pinch(cx);
        }
    }

    fn advance_if_finished_under_pinch(&mut self, cx: &mut Context<Self>) {
        if self.playback_is_paused(cx) || self.is_current_story_pending(cx) {
            return;
        }
        let Some(current) = self.current_story().cloned() else {
            return;
        };
        if self.finished_video_story_id == Some(current.id) {
            self.finished_video_story_id = None;
            self.go_next(cx);
        } else if current.media_type != "video" && self.progress(cx) >= 1.0 {
            self.go_next(cx);
        }
    }

    fn settle_after_pinch_press(&mut self, cx: &mut Context<Self>) {
        if self.drag_offset_target() != 0.0 {
            self.drag_offset.set_in(0.0, interactive_spring(0.3, 0.86, 0.25), RiseClock::monotonic(), cx);
        }
        if !self.viewers_open() && !self.reply_focused {
            self.is_paused = false;
        }
    }

    fn tap_zones_drag_vanished(&mut self, cx: &mut Context<Self>) {
        if self.press_start.is_none() {
            return;
        }
        let resumes = self.drag_axis == DragAxis::Undecided && (self.pinch_in_press || self.is_holding_to_pause);
        self.hold_task = None;
        self.set_holding_to_pause(false, cx);
        self.press_start = None;
        self.movement_cancelled_hold = false;
        self.pinch_in_press = false;
        if resumes {
            self.settle_after_pinch_press(cx);
        }
    }

    fn link_geometry(&self, story_id: Option<i64>) -> (Option<f32>, bool) {
        if self.media_geometry_story_id == story_id { (self.media_aspect, self.media_fills) } else { (None, true) }
    }

    fn link_zones_layer(&self, story: &StoryDTO, cx: &Context<Self>) -> AnyElement {
        let (aspect, fills) = self.link_geometry(Some(story.id));
        let weak = cx.weak_entity();
        div()
            .absolute()
            .inset_0()
            .child(
                MediaLinkAreasOverlay::new(
                    ElementId::NamedInteger(SharedString::new_static("story-link-zones"), story.id as u64),
                    MediaLinkZone::zones_story(&story.link_areas),
                    Rc::new(move |zone, point, _, cx| {
                        weak.update(cx, |this, cx| {
                            this.link_zone_tapped(zone, point, cx);
                            cx.notify();
                        })
                        .ok();
                    }),
                )
                .media_aspect(aspect)
                .media_fills(fills),
            )
            .into_any_element()
    }

    fn link_zone_tapped(&mut self, zone: &MediaLinkZone, point: Point<Pixels>, cx: &mut Context<Self>) {
        if self.link_hint.as_ref().is_some_and(|hint| hint.id == zone.id) {
            self.open_link_hint(zone, cx);
            return;
        }
        let _ = selection_changed();
        self.link_hint_point = point;
        self.show_link_hint(zone.clone(), cx);
    }

    fn show_link_hint(&mut self, zone: MediaLinkZone, cx: &mut Context<Self>) {
        self.link_hint = Some(zone);
        self.link_hint_alpha.jump(0.0);
        self.link_hint_alpha.set_in(1.0, ease_out(0.16), RiseClock::monotonic(), cx);
    }

    fn link_hint_layer(
        &self,
        hint: MediaLinkZone,
        size: Size<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entity_id = cx.entity_id();
        let alpha = self.link_hint_alpha.frame_value(RiseClock::monotonic(), window);
        let x = self.link_hint_point.x.max(px(110.0)).min(size.width - px(110.0));
        let y = (self.link_hint_point.y - px(54.0)).max(px(90.0));
        let (tapped, measured, opened) = (cx.weak_entity(), cx.weak_entity(), cx.weak_entity());
        let backdrop_hint = hint.clone();
        let open_zone = hint.clone();
        let hint_id = ElementId::NamedInteger(SharedString::new_static("story-link-hint"), hint.id as u64);
        div()
            .absolute()
            .inset_0()
            .child(div().absolute().inset_0().occlude().on_mouse_up(MouseButton::Left, move |event: &MouseUpEvent, _, cx| {
                tapped
                    .update(cx, |this, cx| {
                        let local = point(
                            event.position.x - this.link_layer_origin.x,
                            event.position.y - this.link_layer_origin.y,
                        );
                        this.handle_link_hint_backdrop_tap(&backdrop_hint, local, size, cx);
                        cx.notify();
                    })
                    .ok();
            }))
            .child(measure(("story-link-hint-geometry", entity_id), move |bounds, _, cx| {
                measured.update(cx, |this, _| this.link_layer_origin = bounds.origin).ok();
            }))
            .child(
                div().absolute().left(x).top(y).size_0().child(
                    div()
                        .absolute()
                        .left(-size.width)
                        .w(size.width * 2.0)
                        .top(px(-100.0))
                        .h(px(200.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .opacity(alpha)
                        .child(div().occlude().child(MediaLinkOpenHint::new(
                            hint_id,
                            hint,
                            Rc::new(move |_, cx| {
                                opened
                                    .update(cx, |this, cx| {
                                        this.open_link_hint(&open_zone, cx);
                                        cx.notify();
                                    })
                                    .ok();
                            }),
                        ))),
                ),
            )
            .into_any_element()
    }

    fn handle_link_hint_backdrop_tap(
        &mut self,
        hint: &MediaLinkZone,
        at: Point<Pixels>,
        size: Size<Pixels>,
        cx: &mut Context<Self>,
    ) {
        let story = self.current_story().cloned();
        let (aspect, fills) = self.link_geometry(story.as_ref().map(|story| story.id));
        let zones = MediaLinkZone::zones_story(story.as_ref().map(|story| story.link_areas.as_slice()).unwrap_or(&[]));
        let tapped = zones
            .iter()
            .filter(|zone| zone.id == hint.id)
            .chain(zones.iter().filter(|zone| zone.id != hint.id))
            .find(|zone| MediaLinkAreasOverlay::hit(zone, at, size, aspect, fills))
            .cloned();
        let Some(tapped) = tapped else {
            self.link_hint = None;
            return;
        };
        if tapped.id == hint.id {
            self.open_link_hint(&tapped, cx);
            return;
        }
        let _ = selection_changed();
        let frame = MediaFrameRect::rect(size, aspect, fills);
        self.link_hint_point = point(
            frame.origin.x + frame.size.width * tapped.center_x as f32,
            frame.origin.y + frame.size.height * tapped.center_y as f32,
        );
        self.show_link_hint(tapped, cx);
    }

    /// story waits under the link sheet and resumes by itself when it closes
    fn open_link_hint(&mut self, zone: &MediaLinkZone, cx: &mut Context<Self>) {
        self.link_hint = None;
        let Some(url) = Url::from_string(&zone.url) else {
            return;
        };
        InAppLinkBrowser::route(&url, cx);
        self.link_sheet_open = self
            .navigation_store
            .as_ref()
            .is_some_and(|navigation| navigation.read(cx).bot_mini_app_presentation().is_some());
    }

    /// paused mark of the riff, takes its own press or a link under it would win and open its hint
    fn user_pause_mark(&self, chrome_opacity: f32, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let alpha = self.user_pause_alpha.frame_value(RiseClock::monotonic(), window);
        if !self.is_user_paused && alpha <= 0.001 {
            return None;
        }
        let glyph = div()
            .size(px(58.0))
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized("play.fill", px(24.0), white(), true))
            .liquid_chrome_circle(None)
            .liquid_glass_contrast(LiquidGlassContrast::Media);
        let interactive = self.is_user_paused && !self.chrome_hidden();
        let mark = if interactive {
            let weak = cx.weak_entity();
            div()
                .occlude()
                .child(
                    PressableUi::new(("story-user-pause", cx.entity_id()))
                        .tap_scale(0.92)
                        .on_press(move |_, cx| {
                            weak.update(cx, |this, cx| {
                                this.toggle_user_pause(cx);
                                cx.notify();
                            })
                            .ok();
                        })
                        .child(glyph),
                )
                .into_any_element()
        } else {
            glyph.into_any_element()
        };
        Some(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .size(px(58.0))
                        .rounded_full()
                        .opacity(alpha * chrome_opacity)
                        .shadow(vec![BoxShadow {
                            color: black().opacity(0.28),
                            offset: point(px(0.0), px(2.0)),
                            blur_radius: px(6.0),
                            spread_radius: px(0.0),
                            inset: false,
                        }])
                        .child(mark),
                )
                .into_any_element(),
        )
    }

    /// time and after a media edit the word edited next to it
    fn story_time_line(story: Option<&StoryDTO>) -> String {
        let time = Self::relative_time(story.map(|story| story.created_at_ms));
        match story {
            Some(story) if story.was_edited() && !time.is_empty() => format!("{time} \u{b7} {}", tr("story_edited_badge")),
            _ => time,
        }
    }

    fn adjacent_group_index(&self, offset: f32) -> Option<usize> {
        if offset.abs() <= 0.5 {
            return None;
        }
        let candidate = if offset < 0.0 { self.group_index.checked_add(1) } else { self.group_index.checked_sub(1) }?;
        (candidate < self.groups().len()).then_some(candidate)
    }

    fn update_horizontal_drag(&mut self, translation: f32) {
        if self.request.presentation_style != StoryPlayerPresentationStyle::FeedCube {
            return;
        }
        let candidate = if translation < 0.0 { self.group_index.checked_add(1) } else { self.group_index.checked_sub(1) };
        if candidate.is_some_and(|candidate| candidate < self.groups().len()) {
            self.horizontal_drag_offset.jump(translation);
        } else {
            self.horizontal_drag_offset.jump(translation * 0.12);
        }
    }

    fn finish_horizontal_drag(
        &mut self,
        translation: f32,
        predicted_translation: f32,
        width: f32,
        cx: &mut Context<Self>,
    ) {
        let forward = translation < 0.0;
        let candidate = if forward { self.group_index.checked_add(1) } else { self.group_index.checked_sub(1) }
            .filter(|candidate| *candidate < self.groups().len());
        let crossed_threshold =
            translation.abs() > (width * 0.18).max(70.0) || predicted_translation.abs() > width * 0.42;
        let now = RiseClock::monotonic();

        if self.request.presentation_style != StoryPlayerPresentationStyle::FeedCube {
            self.horizontal_drag_offset.jump(0.0);
            if crossed_threshold {
                if forward {
                    self.go_next_group(cx);
                } else {
                    self.go_prev_group(cx);
                }
            } else if !self.viewers_open() && !self.reply_focused {
                self.is_paused = false;
                self.restart_timer(cx);
            }
            return;
        }

        let Some(candidate) = candidate.filter(|_| crossed_threshold) else {
            self.horizontal_drag_offset.set_in(0.0, interactive_spring(0.3, 0.88, 0.25), now, cx);
            Self::after(0.3, cx, |this, cx| {
                if this.is_cube_transitioning || this.viewers_open() || this.reply_focused {
                    return;
                }
                this.is_paused = false;
                this.restart_timer(cx);
            });
            return;
        };

        self.is_cube_transitioning = true;
        let destination = if forward { -width.max(1.0) } else { width.max(1.0) };
        self.horizontal_drag_offset.set_in(destination, ease_in_out(0.26), now, cx);
        Self::after(0.26, cx, move |this, cx| {
            if forward {
                this.mark_group_seen_if_finished(cx);
            }
            this.group_index = candidate;
            this.story_index = this.first_unseen_index(candidate, None, cx);
            this.horizontal_drag_offset.jump(0.0);
            this.is_cube_transitioning = false;
            this.begin_segment(cx);
        });
    }

    fn drag_progress(drag_offset: f32) -> f32 {
        if drag_offset <= 0.0 {
            return 0.0;
        }
        (drag_offset / 320.0).min(1.0)
    }

    fn backdrop_opacity(&self, drag_offset: f32) -> f32 {
        if self.viewers_open() {
            return 1.0;
        }
        if self.is_presenting {
            return (1.0 - drag_offset / self.presentation_distance.max(1.0)).clamp(0.0, 1.0);
        }
        1.0 - Self::drag_progress(drag_offset)
    }

    fn resume_after_dismiss_cancellation(&mut self, cx: &mut Context<Self>) {
        if self.viewers_open() || self.reply_focused {
            return;
        }
        self.is_paused = false;
        self.restart_timer(cx);
    }

    pub fn dismiss(&mut self, animated: bool, cx: &mut Context<Self>) {
        if self.is_dismissing {
            return;
        }
        if animated && self.begin_shared_element_exit(None, cx).is_some() {
            return;
        }
        self.is_paused = true;
        self.stop();
        if animated {
            self.is_dismissing = true;
            self.is_presenting = false;
            self.drag_offset.set_in(self.presentation_distance, ease_in_out(0.32), RiseClock::monotonic(), cx);
            Self::after(0.31, cx, |_, cx| {
                StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.dismiss_player(cx));
            });
        } else {
            StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.dismiss_player(cx));
        }
    }

    /// only a standard player opened from a flying origin, tray and rings and archive and chat slide
    fn shared_element_capable(&self) -> bool {
        self.request.presentation_style == StoryPlayerPresentationStyle::Standard
            && self.request.shared_element_origin.is_some()
    }

    fn install_shared_element_destination(&mut self, cx: &mut Context<Self>) {
        if !self.shared_element_capable() {
            return;
        }
        let weak = cx.weak_entity();
        self.shared_element_destination.set_on_phase(Some(Rc::new(move |phase, cx| {
            let weak = weak.clone();
            cx.defer(move |cx| {
                weak.update(cx, |this, cx| {
                    this.apply_shared_element_phase(phase, cx);
                    cx.notify();
                })
                .ok();
            });
        })));
        self.sync_shared_element_destination(cx);
        let destination: Rc<dyn SharedElementOverlayDestination> = self.shared_element_destination.clone();
        SharedElementOverlayRegistry::shared(cx).update(cx, |registry, _| registry.register(&destination));
    }

    fn sync_shared_element_destination(&mut self, cx: &mut Context<Self>) {
        let Some(story) = self.current_story().cloned() else {
            self.shared_element_destination.set_current(None, "", false);
            return;
        };
        let (source, is_video) = self.media_source(&story, cx);
        self.shared_element_destination.set_current(Some(story.id), &source, is_video);
    }

    /// none when refused, reply focus or viewers pager or pending upload or a story no longer in the origin
    fn begin_shared_element_exit(
        &mut self,
        touch_start: Option<Point<Pixels>>,
        cx: &mut Context<Self>,
    ) -> Option<Rc<SharedElementOverlayDriver>> {
        if !self.shared_element_capable() {
            return None;
        }
        let origin = self.request.shared_element_origin.clone()?;
        let story_id = self.current_story()?.id;
        if self.reply_focused
            || self.viewers_open()
            || self.is_viewers_drag_active()
            || self.is_current_story_pending(cx)
            || self.is_opening_chat
        {
            return None;
        }
        self.sync_shared_element_destination(cx);
        let drag_offset = self.drag_offset_target();
        self.shared_element_destination.begin_exit(&origin, story_id, drag_offset, touch_start, cx)
    }

    fn begin_shared_element_dismiss_drag(&mut self, touch_start: Option<Point<Pixels>>, cx: &mut Context<Self>) {
        if touch_start.is_none() || self.shared_element_destination.interactive_exit().is_some() {
            return;
        }
        let exit = self.begin_shared_element_exit(touch_start, cx);
        self.shared_element_destination.set_interactive_exit(exit.as_ref());
    }

    fn tap_zones_screen_point(&self, location: Point<Pixels>, cx: &App) -> Option<Point<Pixels>> {
        let origin = self.shared_element_destination.media_frame(cx)?.origin;
        Some(point(origin.x + location.x, origin.y + location.y))
    }

    fn global_screen_point(&self, location: Point<Pixels>) -> Option<Point<Pixels>> {
        let origin = self.owner_bar_origin?;
        Some(point(origin.x + location.x, origin.y + location.y))
    }

    /// true while an overlay flight follows the pointer instead of drag_offset
    fn follow_shared_element_dismiss_drag(&self, value: &DragGestureValue, cx: &mut Context<Self>) -> bool {
        let Some(exit) = self.shared_element_destination.interactive_exit() else {
            return false;
        };
        exit.follow(
            point(value.translation.width, value.translation.height),
            point(value.velocity.width, value.velocity.height),
            cx,
        );
        true
    }

    fn release_shared_element_dismiss_drag(&self, value: &DragGestureValue, cx: &mut Context<Self>) -> bool {
        let Some(exit) = self.shared_element_destination.interactive_exit() else {
            return false;
        };
        self.shared_element_destination.set_interactive_exit(None);
        exit.release(
            point(value.translation.width, value.translation.height),
            point(value.velocity.width, value.velocity.height),
            cx,
        );
        true
    }

    fn shared_element_dismiss_drag_vanished(&self, cx: &mut Context<Self>) {
        let Some(exit) = self.shared_element_destination.interactive_exit() else {
            return;
        };
        self.shared_element_destination.set_interactive_exit(None);
        exit.cancel_interaction(cx);
    }

    /// discrete writes only, each runs without animation unless it fades
    fn apply_shared_element_phase(&mut self, phase: SharedElementOverlayPhase, cx: &mut Context<Self>) {
        let now = RiseClock::monotonic();
        match phase {
            SharedElementOverlayPhase::OpenLanded => {
                self.overlay_veiled = false;
                self.is_presenting = false;
                self.overlay_chrome_hidden = false;
                self.overlay_chrome_alpha.set_in(1.0, ease_out(SharedElementMotion::OVERLAY_CHROME_FADE), now, cx);
                self.resume_after_shared_element_open(cx);
            }
            SharedElementOverlayPhase::OpenFallback => {
                self.overlay_content_opacity.jump(0.0);
                self.overlay_veiled = false;
                self.overlay_chrome_hidden = false;
                self.overlay_chrome_alpha.jump(1.0);
                self.is_presenting = false;
                self.overlay_content_opacity.set_in(1.0, ease_out(SharedElementMotion::OPEN_FALLBACK_FADE), now, cx);
                self.resume_after_shared_element_open(cx);
            }
            SharedElementOverlayPhase::OpenAborted => {
                self.overlay_content_opacity.jump(1.0);
                self.overlay_veiled = false;
                self.overlay_chrome_hidden = false;
                self.overlay_chrome_alpha.jump(1.0);
                self.is_presenting = false;
                self.resume_after_shared_element_open(cx);
            }
            SharedElementOverlayPhase::ExitBegan => {
                self.overlay_veiled = true;
                self.overlay_chrome_hidden = true;
                self.overlay_chrome_alpha.jump(0.0);
                self.drag_offset.jump(0.0);
                self.is_dismissing = true;
                self.is_paused = true;
                self.stop();
            }
            SharedElementOverlayPhase::ExitCancelled => {
                self.overlay_content_opacity.jump(1.0);
                self.overlay_veiled = false;
                self.drag_offset.jump(0.0);
                self.is_dismissing = false;
                self.overlay_chrome_hidden = false;
                self.overlay_chrome_alpha.set_in(1.0, ease_out(SharedElementMotion::OVERLAY_CHROME_FADE), now, cx);
                self.resume_after_dismiss_cancellation(cx);
            }
            SharedElementOverlayPhase::ExitFinished => {
                if self.service.read(cx).player_request.as_ref().map(|live| live.id) != Some(self.request.id) {
                    return;
                }
                StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.dismiss_player(cx));
            }
        }
    }

    fn resume_after_shared_element_open(&mut self, cx: &mut Context<Self>) {
        if self.viewers_open() || self.reply_focused || self.is_story_menu_presented {
            return;
        }
        self.is_paused = false;
        self.restart_timer(cx);
    }

    fn present_share(&mut self, cx: &mut Context<Self>) {
        let Some(story) = self.current_story().cloned() else {
            return;
        };
        self.is_paused = true;
        self.share_sheet.update(cx, |sheet, cx| sheet.present_story(story, cx));
    }

    fn push_route(&self, route: RootRoute, cx: &mut App) {
        if let Some(navigation_store) = self.navigation_store.clone() {
            navigation_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn open_author_profile(&mut self, cx: &mut Context<Self>) {
        let user_id = self.current_author_id();
        if user_id.is_empty() {
            return;
        }
        let _ = selection_changed();
        let tag = self.author().map(|author| author.tag.clone());
        self.dismiss(false, cx);
        let navigation_store = self.navigation_store.clone();
        cx.defer(move |cx| {
            if let Some(navigation_store) = navigation_store {
                navigation_store.update(cx, |store, cx| {
                    store.push(
                        RootRoute::UserPage { user_id: Some(user_id), tag },
                        NavigationStoreSettlingPush::Drop,
                        cx,
                    );
                });
            }
        });
    }

    fn open_author_chat(&mut self, cx: &mut Context<Self>) {
        let author_id = self.current_author_id();
        if self.is_opening_chat || author_id.is_empty() {
            return;
        }
        let _ = selection_changed();
        self.is_opening_chat = true;
        self.is_paused = true;
        self.stop();
        let author = self.author().cloned();
        let (opened, failed) = (cx.weak_entity(), cx.weak_entity());
        let on_open = Rc::new(move |chat_id: String, cx: &mut App| {
            let opened = opened.clone();
            cx.defer(move |cx| {
                opened
                    .update(cx, |this, cx| {
                        this.is_opening_chat = false;
                        StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.dismiss_player(cx));
                        if let Some(navigation_store) = this.navigation_store.clone() {
                            cx.defer(move |cx| {
                                navigation_store.update(cx, |store, cx| {
                                    store.switch_tab(MainTab::Chats, cx);
                                    store.push(
                                        RootRoute::Chat { chat_id: Some(chat_id) },
                                        NavigationStoreSettlingPush::Drop,
                                        cx,
                                    );
                                });
                            });
                        }
                        cx.notify();
                    })
                    .ok();
            });
        });
        let on_error = Rc::new(move |_: String, cx: &mut App| {
            let failed = failed.clone();
            cx.defer(move |cx| {
                failed
                    .update(cx, |this, cx| {
                        this.is_opening_chat = false;
                        if !this.viewers_open() && !this.reply_focused {
                            this.is_paused = false;
                            this.restart_timer(cx);
                        }
                        Self::notify_fetch_error(cx);
                        cx.notify();
                    })
                    .ok();
            });
        });
        StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.open_author_chat(Some(author_id.as_str()), author.as_ref(), on_open, Some(on_error), cx);
        });
    }

    fn notify_fetch_error(cx: &mut App) {
        NotifierStore::shared(cx).update(cx, |notifier, cx| {
            notifier.send(
                tr("error_fetching_data"),
                NotifierSendOptions {
                    r#type: NotifierType::Error,
                    position: NotifierPosition::Top,
                    ..NotifierSendOptions::default()
                },
                cx,
            );
        });
    }

    fn submit_reply_from_key(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let is_live = self.service.read(cx).player_request.as_ref().map(|live| live.id) == Some(self.request.id);
        let focused = self.reply_field_focused(window, cx);
        if !is_live || !focused || self.is_dismissing || self.reply_input.read(cx).is_composing() {
            tracing::debug!(
                target: "riseonly::X837",
                "story enter refused live {} focused {} flag {} dismissing {}",
                is_live,
                focused,
                self.reply_focused,
                self.is_dismissing
            );
            return false;
        }
        tracing::debug!(target: "riseonly::X837", "story reply sent by enter");
        self.send_reply(window, cx);
        cx.notify();
        true
    }

    fn send_reply(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.reply_input.read(cx).text().to_owned();
        if trimming_whitespaces_and_newlines(&text).is_empty() {
            return;
        }
        self.reply_input.update(cx, |field, cx| field.clear(cx));
        self.drop_reply_focus(window, cx);
        let story = self.current_story().cloned();
        let author_id = self.current_author_id();
        let author = self.author().cloned();
        let (input, restored) = (self.reply_input.clone(), text.clone());
        let (refused_input, refused_text) = (input.clone(), restored.clone());
        let on_refused = Rc::new(move |cx: &mut App| {
            let (input, restored) = (refused_input.clone(), refused_text.clone());
            cx.defer(move |cx| {
                input.update(cx, |field, cx| {
                    if trimming_whitespaces_and_newlines(field.text()).is_empty() {
                        field.set_text(restored, cx);
                    }
                });
            });
        });
        let on_failure = Rc::new(move |cx: &mut App| {
            let (input, restored) = (input.clone(), restored.clone());
            cx.defer(move |cx| {
                input.update(cx, |field, cx| field.set_text(restored, cx));
                Self::notify_fetch_error(cx);
            });
        });
        StoryInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.send_story_reply(story.as_ref(), &author_id, author.as_ref(), &text, Some(on_failure), Some(on_refused), cx);
        });
        let _ = notification(NotificationKind::Success);
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        let last_group = self.groups().len().saturating_sub(1);
        self.group_index = self.request.start_group_index.min(last_group);
        self.story_index = self
            .requested_story_index(self.group_index)
            .unwrap_or_else(|| self.first_unseen_index(self.group_index, None, cx));
        self.set_progress(0.0, cx);
        self.is_paused = self.overlay_veiled;
        self.is_holding_to_pause = false;
        self.is_user_paused = false;
        self.user_pause_alpha.jump(0.0);
        self.chrome_alpha.jump(1.0);
        self.horizontal_drag_offset.jump(0.0);
        self.is_cube_transitioning = false;
        self.is_opening_chat = false;
        self.is_story_menu_presented = false;
        self.viewed_story_ids.clear();
        self.mark_visible_viewed(cx);
        self.restart_timer(cx);
        self.prefetch_current_user_stories_if_needed(cx);
    }

    fn prefetch_current_user_stories_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.current_stories().len().saturating_sub(self.story_index) > 3 {
            return;
        }
        if let Some(highlight_id) = self.service.read(cx).active_highlight_id() {
            StoryInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.load_more_highlight_stories(highlight_id, cx));
            return;
        }
        let author_id = self.current_author_id();
        if !self.request.owns_explicit_stories && !author_id.is_empty() {
            StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.load_more_user_stories(&author_id, cx));
        }
    }

    fn upcoming_stories_for_prefetch(&self) -> Vec<StoryDTO> {
        let groups = self.groups();
        if self.group_index >= groups.len() {
            return Vec::new();
        }
        let limit = StoryPlaybackPolicy::PREFETCH_COUNT.max(0) as usize;
        let mut upcoming = Vec::new();
        for (candidate, group) in groups.iter().enumerate().skip(self.group_index) {
            let start = if candidate == self.group_index {
                (self.story_index + 1).min(group.stories.len())
            } else {
                0
            };
            for story in &group.stories[start..] {
                upcoming.push(story.clone());
                if upcoming.len() == limit {
                    return upcoming;
                }
            }
        }
        upcoming
    }

    fn stop(&mut self) {
        self.timer = None;
    }

    fn requested_story_index(&self, group: usize) -> Option<usize> {
        let requested = self.request.start_story_id?;
        self.groups().get(group)?.stories.iter().position(|story| story.id == requested)
    }

    fn first_unseen_index(&self, group: usize, source: Option<&[StoryGroupDTO]>, cx: &App) -> usize {
        let groups = source.unwrap_or_else(|| self.groups());
        let Some(group) = groups.get(group) else {
            return 0;
        };
        let service = self.service.read(cx);
        if let Some(pending_id) = service.pending_story_publish.as_ref().map(|pending| pending.story_id)
            && let Some(index) = group.stories.iter().position(|story| story.id == pending_id)
        {
            return index;
        }
        group.stories.iter().position(|story| !service.is_story_viewed(story, cx)).unwrap_or(0)
    }

    fn restart_timer(&mut self, cx: &mut Context<Self>) {
        self.timer = None;
        if self.is_current_story_pending(cx) {
            return;
        }
        if self.current_story().is_some_and(|story| story.media_type == "video") {
            return;
        }
        self.timer = Some(RepeatingTimer::scheduled_timer(
            Duration::from_secs_f64(Self::TICK),
            true,
            cx,
            |this: &mut Self, cx| this.advance_image_progress(cx),
        ));
    }

    fn advance_image_progress(&mut self, cx: &mut Context<Self>) {
        if self.playback_is_paused(cx) || self.is_current_story_pending(cx) {
            return;
        }
        let Some(story) = self.current_story().filter(|story| story.media_type != "video").cloned() else {
            return;
        };
        let (source, _) = self.media_source(&story, cx);
        if !source.is_empty() && !StoryImageAspectProbe::is_loaded(&source) {
            return;
        }
        let progress = self.progress(cx) + Self::TICK / StoryPlaybackPolicy::IMAGE_DURATION;
        if progress >= 1.0 {
            self.set_progress(1.0, cx);
            if !self.is_pinching {
                self.go_next(cx);
            }
        } else {
            self.set_progress(progress, cx);
        }
    }

    fn mark_visible_viewed(&mut self, cx: &mut Context<Self>) {
        let Some(story_id) = self.current_story().map(|story| story.id) else {
            return;
        };
        if !self.viewed_story_ids.insert(story_id) {
            return;
        }
        let author_id = self.current_author_id();
        let stories = self.current_stories().to_vec();
        StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.view_story(story_id, &author_id, &stories, cx));
        let service = self.service.read(cx);
        if !author_id.is_empty() && stories.iter().all(|story| service.is_story_viewed(story, cx)) {
            StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.mark_user_stories_seen(&author_id, cx));
        }
    }

    fn go_next(&mut self, cx: &mut Context<Self>) {
        if self.story_index + 1 < self.current_stories().len() {
            self.story_index += 1;
            self.begin_segment(cx);
        } else {
            self.go_next_group(cx);
        }
    }

    fn go_prev(&mut self, cx: &mut Context<Self>) {
        if self.restart_current_if_shown_long(cx) {
            return;
        }
        self.step_back(cx);
    }

    fn step_back(&mut self, cx: &mut Context<Self>) {
        if self.story_index > 0 {
            self.story_index -= 1;
            self.begin_segment(cx);
        } else {
            self.go_prev_group(cx);
        }
    }

    fn restart_current_if_shown_long(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(story) = self.current_story().cloned() else {
            return false;
        };
        let (source, is_video) = self.media_source(&story, cx);
        let clock = self.playback_clock.read(cx);
        let duration = if is_video {
            clock.video_durations.get(&source).copied().unwrap_or(0.0)
        } else {
            StoryPlaybackPolicy::IMAGE_DURATION
        };
        if clock.value * duration < StoryPlaybackPolicy::REWIND_THRESHOLD {
            return false;
        }
        if is_video && let Some(player) = clock.video_players.get(&source).and_then(WeakEntity::upgrade) {
            player.update(cx, |player, cx| player.seek(0.0, 0.0, 0.0, None, cx));
        }
        self.begin_segment(cx);
        true
    }

    fn go_next_group(&mut self, cx: &mut Context<Self>) {
        self.mark_group_seen_if_finished(cx);
        if self.group_index + 1 < self.groups().len() {
            self.group_index += 1;
            self.story_index = self.first_unseen_index(self.group_index, None, cx);
            self.begin_segment(cx);
        } else if self.feed_tail_open {
            self.feed_tail_waiting = true;
            cx.defer(|cx| {
                StoryInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.load_more_stories_feed(cx));
            });
        } else {
            self.dismiss(true, cx);
        }
    }

    fn go_prev_group(&mut self, cx: &mut Context<Self>) {
        if self.group_index > 0 {
            self.group_index -= 1;
            self.story_index = self.first_unseen_index(self.group_index, None, cx);
        }
        self.begin_segment(cx);
    }

    fn go_after_delete(&mut self, cx: &mut Context<Self>) {
        let Some(live) = self.service.read(cx).player_request.as_ref().map(|live| live.groups.clone()) else {
            return;
        };
        if live.len() < self.groups().len() {
            if self.group_index >= live.len() {
                self.dismiss(true, cx);
                return;
            }
            self.story_index = self.first_unseen_index(self.group_index, Some(&live), cx);
            self.begin_segment(cx);
        } else if self.current_stories().len() <= 1 {
            self.go_next_group(cx);
        } else if self.story_index + 1 >= self.current_stories().len() {
            self.story_index = self.story_index.saturating_sub(1);
            self.begin_segment(cx);
        } else {
            self.begin_segment(cx);
        }
    }

    /// open request is rebuilt in place so old indices could hit another author,
    /// reader keeps the story on screen then its author then whoever is near
    fn reanchor(&mut self, old: &[StoryPlayerGroupLayout], new: &[StoryPlayerGroupLayout], cx: &mut Context<Self>) {
        if self.is_dismissing || self.is_cube_transitioning || new.is_empty() {
            return;
        }
        let Some(old_group) = old.get(self.group_index) else {
            return;
        };
        let shown_id = old_group.story_ids.get(self.story_index).copied();
        let committed_id = shown_id.filter(|id| *id < 0).and_then(|_| self.service.read(cx).settled_publish_story_id());
        if let Some(index) = new.iter().position(|group| group.author_id == old_group.author_id) {
            let ids = &new[index].story_ids;
            self.group_index = index;
            self.story_index = shown_id
                .and_then(|shown_id| ids.iter().position(|id| *id == shown_id))
                .or_else(|| committed_id.and_then(|committed| ids.iter().position(|id| *id == committed)))
                .unwrap_or_else(|| self.story_index.min(ids.len().saturating_sub(1)));
        } else if self.group_index < new.len() {
            self.story_index = self.first_unseen_index(self.group_index, None, cx);
        } else {
            if self.request.shared_element_origin.is_none() {
                self.dismiss(true, cx);
            }
            return;
        }
        let now_id = new[self.group_index].story_ids.get(self.story_index).copied();
        if now_id != shown_id {
            self.begin_segment_no_resume(cx);
        }
    }

    fn begin_segment(&mut self, cx: &mut Context<Self>) {
        self.set_progress(0.0, cx);
        self.is_user_paused = false;
        self.user_pause_alpha.jump(0.0);
        if !self.viewers_open() && !self.reply_focused {
            self.is_paused = false;
        }
        self.mark_visible_viewed(cx);
        self.restart_timer(cx);
        cx.notify();
    }

    fn begin_segment_no_resume(&mut self, cx: &mut Context<Self>) {
        self.set_progress(0.0, cx);
        self.is_user_paused = false;
        self.user_pause_alpha.jump(0.0);
        self.mark_visible_viewed(cx);
        self.restart_timer(cx);
        cx.notify();
    }

    fn mark_group_seen_if_finished(&mut self, cx: &mut Context<Self>) {
        if self.story_index + 1 < self.current_stories().len() {
            return;
        }
        let author_id = self.current_author_id();
        if !author_id.is_empty() {
            StoryInteractionsStore::shared(cx).update(cx, |store, cx| store.mark_user_stories_seen(&author_id, cx));
        }
    }

    fn relative_time(ms: Option<i64>) -> String {
        match ms.filter(|ms| *ms > 0) {
            Some(ms) => RiseFormat::elapsed(ms, (RiseClock::now().time_interval_since_1970() * 1000.0) as i64),
            None => String::new(),
        }
    }

    /// keys stay with the reply field or with a covering surface
    /// arrow keys and side buttons step only when nothing covers the story
    fn blocks_step_navigation(&self, cx: &App) -> bool {
        self.reply_focused
            || self.is_presenting
            || self.show_audio_sheet
            || self.is_covered_by_sheet(cx)
            || self.is_cube_transitioning
            || self.is_story_menu_presented
            || self.story_pending_deletion.is_some()
            || self.audio_story.is_some()
            || self.audio_story_composer_active
            || self.viewers_open()
    }

    pub fn has_previous_step(&self) -> bool {
        self.group_index > 0 || self.story_index > 0
    }

    pub fn has_next_step(&self) -> bool {
        self.story_index + 1 < self.current_stories().len()
            || self.group_index + 1 < self.groups().len()
            || self.feed_tail_open
    }

    pub fn step_from_frame_button(&mut self, forward: bool, cx: &mut Context<Self>) {
        if self.is_dismissing || self.blocks_step_navigation(cx) {
            return;
        }
        if !forward {
            self.step_back(cx);
        } else if !self.is_current_story_pending(cx) {
            self.go_next(cx);
        }
        cx.notify();
    }

    fn handle_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let key = event.keystroke.key.as_str();
        if key == "escape" && !event.keystroke.modifiers.modified() {
            if self.handle_escape(window, cx) {
                cx.stop_propagation();
            }
            return;
        }
        if self.blocks_step_navigation(cx) || event.keystroke.modifiers.modified() {
            return;
        }
        if event.is_held && matches!(key, "left" | "right" | "space") {
            return;
        }
        match key {
            "left" => self.step_back(cx),
            "right" => {
                if !self.is_current_story_pending(cx) {
                    self.go_next(cx);
                }
            }
            "space" => self.is_paused = !self.is_paused,
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// escape leaves the reply field first,
    /// then the top layer over the story,
    /// then the story
    fn handle_escape(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let is_live = self.service.read(cx).player_request.as_ref().map(|live| live.id) == Some(self.request.id);
        if !is_live || self.is_dismissing {
            return false;
        }
        if self.reply_field_focused(window, cx) {
            self.drop_reply_focus(window, cx);
            cx.notify();
            return true;
        }
        if self.show_audio_sheet {
            self.show_audio_sheet = false;
            cx.notify();
            return true;
        }
        let covered = self.is_covered_by_sheet(cx)
            || self.is_story_menu_presented
            || self.story_pending_deletion.is_some()
            || self.audio_story.is_some()
            || self.audio_story_composer_active
            || self.link_sheet_open
            || NativeConfirmAlert::is_presenting(cx);
        if !covered && !self.is_presenting && self.viewers_open() {
            self.close_viewers(cx);
            return true;
        }
        if covered || self.is_presenting || self.is_cube_transitioning {
            tracing::warn!(
                target: "riseonly::X840",
                "story escape refused covered {} presenting {} cube {} menu {} viewers {} focused {:?}",
                covered,
                self.is_presenting,
                self.is_cube_transitioning,
                self.is_story_menu_presented,
                self.viewers_open(),
                window.focused(cx),
            );
            return false;
        }
        self.dismiss(true, cx);
        cx.notify();
        true
    }

    fn handle_scroll_wheel(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_presenting
            || self.is_covered_by_sheet(cx)
            || self.viewers_open()
            || self.audio_story.is_some()
            || self.audio_story_composer_active
        {
            return;
        }
        let now = RiseClock::monotonic();
        if self.wheel_last_event.is_none_or(|last| now.saturating_sub(last) > WHEEL_SWIPE_GAP) {
            self.wheel_dismiss_travel = 0.0;
        }
        self.wheel_last_event = Some(now);
        let travel = f32::from(event.delta.pixel_delta(window.line_height()).y);
        self.wheel_dismiss_travel = (self.wheel_dismiss_travel + travel).max(0.0);
        if self.wheel_dismiss_travel > WHEEL_DISMISS_DISTANCE {
            self.wheel_dismiss_travel = 0.0;
            self.dismiss(true, cx);
            cx.notify();
        }
    }

    fn audio_sheet(&self, theme: &AppTheme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let audio_story = self.audio_story.clone()?;
        let track_id = audio_story.track_id.clone().filter(|id| !id.is_empty())?;
        let dismissed = {
            let track_id = track_id.clone();
            move |_: &mut Window, cx: &mut App| {
                let post_id = format!("audio_page:{track_id}");
                PostMusicPlaybackController::shared(cx).update(cx, |controller, cx| controller.stop(&post_id, cx));
            }
        };
        let (close, navigate, create) = (cx.weak_entity(), cx.weak_entity(), cx.weak_entity());
        let content = move |_: &mut Window, _: &mut App| {
            let (close, navigate, create) = (close.clone(), navigate.clone(), create.clone());
            AudioPage::new("story-audio-page", track_id.clone())
                .source_story(Some(audio_story.clone()))
                .is_sheet(true)
                .on_close(move |_, cx| {
                    close
                        .update(cx, |this, cx| {
                            this.show_audio_sheet = false;
                            cx.notify();
                        })
                        .ok();
                })
                .on_navigate(move |route, _, cx| {
                    navigate
                        .update(cx, |this, cx| {
                            this.audio_destination = Some(StoryAudioDestination::Route(route));
                            this.show_audio_sheet = false;
                            cx.notify();
                        })
                        .ok();
                })
                .on_create_story(move |selection, _, cx| {
                    create
                        .update(cx, |this, cx| {
                            this.audio_destination = Some(StoryAudioDestination::Story(selection));
                            this.show_audio_sheet = false;
                            cx.notify();
                        })
                        .ok();
                })
        };
        Some(
            BottomSheetUi::new(
                "story-audio-sheet",
                Binding::entity(cx.entity(), |this: &Self| this.show_audio_sheet, |this, shown, _| {
                    this.show_audio_sheet = shown;
                }),
                content,
                |_, _| div(),
            )
            .snap_fractions(vec![0.56, 1.0])
            .full_page_hides_footer(true)
            .content_scrollable(true)
            .sheet_background(Some(theme.bg._100))
            .on_dismiss(dismissed)
            .on_did_dismiss(Self::press_action(cx, |this, _, cx| this.finish_audio_sheet_dismissal(cx)))
            .on_presentation_refused(Self::press_action(cx, |this, _, cx| this.finish_audio_sheet_dismissal(cx)))
            .into_any_element(),
        )
    }
}

impl Render for StoryPlayer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(navigation_store) = env::<NavigationStoreKey>(cx) {
            self.navigation_store = Some(navigation_store);
        }
        let theme = rise_ui::theme(cx).clone();
        let entity_id = cx.entity_id();
        self.register_changes(window, cx);

        let count = self.current_stories().len();
        let index = self.story_index;
        let bars = self.progress_bars.clone();
        if bars.read(cx).count != count || bars.read(cx).index != index {
            cx.defer(move |cx| bars.update(cx, |bars, cx| bars.set_position(count, index, cx)));
        }

        let size = if self.stable_size == Size::default() { self.geo_size } else { self.stable_size };
        let inset_top = self.stable_inset_top;
        let safe_inset_top = px(0.0);
        let now = RiseClock::monotonic();
        let drag_offset = self.drag_offset.frame_value(now, window);
        let backdrop = if self.overlay_veiled { 0.0 } else { self.backdrop_opacity(drag_offset) };
        let content_opacity = self.overlay_content_opacity.frame_value(now, window);

        let tap_zones_enabled = !self.is_presenting
            && self.audio_story.is_none()
            && !self.audio_story_composer_active
            && !self.is_cube_transitioning
            && !self.is_story_menu_presented
            && (self.viewers_progress_target <= 0.02 || self.is_viewers_drag_active());
        let has_geometry = size != Size::default();
        let tap_zones = (tap_zones_enabled && has_geometry).then(|| self.tap_zones(size, cx));
        let content = has_geometry.then(|| {
            div()
                .absolute()
                .left_0()
                .top(inset_top + px(drag_offset))
                .child(self.player_content(size, window, cx))
        });
        let probe = self
            .shared_element_capable()
            .then(|| SharedElementOverlayProbe::new(self.shared_element_destination.probe.clone()));
        let audio_sheet = self.audio_sheet(&theme, cx);
        let composer = StoryComposerHost::new(
            "story-audio-composer",
            Binding::entity(cx.entity(), |this: &Self| this.show_audio_story_composer, |this, shown, _| {
                this.show_audio_story_composer = shown;
            }),
        )
        .initial_track(self.audio_story_track_seed.clone())
        .on_dismissed(Self::press_action(cx, |this, _, cx| {
            this.audio_story_composer_active = false;
            this.audio_story_track_seed = None;
            this.resume_after_audio(cx);
        }));

        let measured = cx.weak_entity();
        let (appearing, disappearing) = (cx.weak_entity(), cx.weak_entity());
        div()
            .relative()
            .size_full()
            .overflow_hidden()
            .occlude()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::handle_key_down))
            .on_scroll_wheel(cx.listener(Self::handle_scroll_wheel))
            .opacity(content_opacity)
            .child(div().absolute().inset_0().bg(black().opacity(backdrop)))
            .children(tap_zones)
            .children(content)
            .children(probe.map(|probe| div().absolute().top(inset_top).left_0().size_0().child(probe)))
            .child(measure(("story-player-geometry", entity_id), move |bounds, _, cx| {
                let geo_size = Size { width: bounds.size.width, height: (bounds.size.height - safe_inset_top).max(px(0.0)) };
                measured.update(cx, |this, cx| this.geometry_did_change(geo_size, safe_inset_top, cx)).ok();
            }))
            .children(audio_sheet.map(|sheet| div().absolute().inset_0().child(sheet)))
            .child(div().absolute().top_0().left_0().size_0().child(composer))
            .on_appear(("story-player", entity_id), move |window, cx| {
                appearing.update(cx, |this, cx| this.did_appear(window, cx)).ok();
            })
            .and_on_disappear(move |_, cx| {
                if let Some(player) = disappearing.upgrade() {
                    player.update(cx, |this, cx| this.did_disappear(cx));
                }
            })
    }
}
