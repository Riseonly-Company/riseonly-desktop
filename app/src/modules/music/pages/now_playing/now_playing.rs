use crate::core::media_viewer::media_viewer_presentation_state::media_viewer_presentation;
use std::rc::Rc;
use std::sync::{Arc, LazyLock};

use gpui::{
    App, BoxShadow, Context, ElementId, Empty, Entity, FocusHandle, FontWeight, Global, Hsla, KeyDownEvent, PromptButton, PromptLevel,
    Render, SharedString, StyleRefinement, Subscription, Svg, TextRun, WeakEntity, Window, anchored, black, font, point, relative,
    rgb,
    transparent_black, white,
};
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::ns_cache::NSCache;
use rise_i18n::{FormatArg, tr, tr_args, tr_plural_with};
use rise_theme::AppTheme;
use rise_ui::liquid_chrome::LiquidChrome;
use rise_ui::secondary_text::SecondaryText;
use rise_theme::FONT_FAMILY;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::swiftui::page_refresh::PageFocus;
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::IconUi;
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::main_text::{MainText, TextAlignT, ignores_text_size_delta};
use rise_ui::rise_symbol::RiseSymbol;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::blur::BlurModifiers;
use rise_ui::swiftui::modifiers::{ModifierExt, ViewModifier};
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{TransitionModifiers, UnitPoint};
use rise_ui::window_cover::WindowCoverFrame;
use rise_widgets::window_controls_band;
use gpui::Pixels;
use rise_widgets::side_panel::side_panel_preferences::SidePanelPreferences;
use rise_widgets::side_panel::side_panel_ui::{ToggleSidePanel, install_key_bindings};

use crate::app::aside::aside_registry::AsideSection;
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::core::widgets::navigations::bottom_chrome_insets::BottomChromeInsetsModifiers;
use crate::core::widgets::notifier::notifier_store::{
    Notifier, NotifierIcon, NotifierPosition, NotifierRetryOptions, NotifierSendOptions, NotifierType,
};
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::sheets::share_sheet::{MusicTrackSharePolicy, ShareSheetStore};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::music::pages::now_playing::music_live_playback_seek_bar::MusicLivePlaybackSeekBar;
use crate::modules::music::pages::now_playing::now_playing_aside_hooks::now_playing_aside_hooks;
use crate::modules::music::pages::playlist_detail::music_playlist_listener_cluster::MusicPlaylistListenerCluster;
use crate::modules::music::pages::track::track_context_menu_row::{MusicLocalTrackEditPolicy, MusicTrackReportPolicy};
use crate::modules::music::shared::utils::music::music_models::{
    MusicLocalTrackTransferPhase, MusicRepeatMode, MusicRepeatModeExt, TrackModel,
};
use crate::modules::music::shared::utils::music::music_player_state::{
    MusicListeningBlockedControlExt, MusicPlayerState,
};
use crate::modules::music::shared::utils::music::music_track_download_store::MusicTrackDownloadStore;
use crate::modules::music::shared::utils::music::music_track_file_export::MusicTrackSavePolicy;
use crate::core::lib::canon_format::RiseFormat;
use crate::modules::music::stores::music_actions::music_actions::MusicActionsStore;
use crate::modules::music::stores::music_actions::music_types::MusicPlaylistListener;
use crate::modules::music::stores::music_actions::music_wire_types::WireArtistDTO;
use crate::modules::music::stores::music_interactions::music_interactions::MusicInteractionsStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::music::widgets::bottomsheets::now_playing::music_queue_sheet::MusicQueueSheet;
use crate::modules::music::widgets::bottomsheets::playlists::add_to_playlist_sheet::AddToPlaylistSheet;
use crate::modules::music::widgets::bottomsheets::playlists::edit_local_track_sheet::EditLocalTrackSheet;
use crate::modules::music::widgets::music_cover_placeholder::MusicCoverMedia;
use crate::modules::music::widgets::music_exclusive_badge::MusicExclusiveBadge;
use crate::modules::music::widgets::music_marquee_title::MusicMarqueeTitle;
use crate::modules::music::widgets::music_mini_player_bottom_padding::observed_music_player;
use crate::modules::music::widgets::music_track_cover_image::{MusicCoverImage, MusicTrackCoverImageModifiers};

const ACCENT_COLOR: u32 = 0xE84393;
const BOTTOM_SLOT_ICON_SIZE: f32 = 22.;
const BOTTOM_SLOT_LABEL_HEIGHT: f32 = 25.;
const BOTTOM_SLOT_DISABLED_OPACITY: f32 = 0.35;
const BOTTOM_SLOT_HEIGHT: f32 = 52.;
const BOTTOM_SLOT_HEIGHT_GROWTH: f32 = 14.;
const BOTTOM_ROW_HORIZONTAL_PADDING: f32 = 14.;
const BOTTOM_CAPTION_MAX_SIZE: f32 = 10.;
const BOTTOM_CAPTION_MIN_SIZE: f32 = 8.;
const BOTTOM_CAPTION_FALLBACK_SCALE: f32 = 0.8;
const COVER_SLOT_INSET: f32 = 8.;
const COVER_CORNER_RADIUS: f32 = 20.;
const QUEUE_SHEET_FRACTIONS: [f32; 2] = [0.55, 0.88];
const PLAYER_COLUMN_MAX_WIDTH: f32 = 480.;
const QUEUE_PANEL_CONTEXT: &str = "music_queue";
const ASIDE_COVER_MAX_SIDE: f32 = 320.;
const ASIDE_COVER_HEIGHT_SHARE: f32 = 0.32;
const ASIDE_TOP_PADDING: f32 = 20.;
const ASIDE_HORIZONTAL_PADDING: f32 = 16.;
const ASIDE_CARD_RADIUS: f32 = 12.;
const ASIDE_ARTIST_PHOTO_HEIGHT: f32 = 160.;
const ASIDE_ARTIST_BIO_LINES: usize = 3;
const ASIDE_QUEUE_MIN_HEIGHT: f32 = 260.;
const ASIDE_HEADER_MAX_SHARE: f32 = 0.62;

static BOTTOM_CAPTION_SIZE_CACHE: LazyLock<NSCache<String, f32>> = LazyLock::new(|| {
    let cache = NSCache::new();
    cache.set_count_limit(16);
    cache
});

fn child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

fn icon(name: &str, size: f32, color: Hsla) -> Option<Svg> {
    IconUi::sized(name, px(size), color, name.ends_with(".fill"))
}

fn accent() -> Hsla {
    Hsla::from(rgb(ACCENT_COLOR))
}

#[derive(IntoElement)]
struct MusicPlaybackFailureNotice {
    id: ElementId,
}

impl RenderOnce for MusicPlaybackFailureNotice {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let player = observed_music_player(child_id(&self.id, "player"), window, cx);
        let announced_failure = {
            let player = player.read(cx);
            (player.last_playback_error().is_some() && !player.is_preparing_playback)
                .then(|| player.playback_failure_serial())
        };
        on_change(child_id(&self.id, "failure"), announced_failure, false, window, cx, move |_, failure, _, cx| {
            if failure.is_none() {
                return;
            }
            let position = if player.read(cx).show_now_playing() { NotifierPosition::Top } else { NotifierPosition::Bottom };
            Notifier::send_retry(
                tr("music_preview_playback_failed"),
                NotifierRetryOptions { position, ..NotifierRetryOptions::default() },
                |_| {},
                move |cx| {
                    player.update(cx, |player, cx| {
                        if player.is_playing || player.is_preparing_playback {
                            return;
                        }
                        player.toggle_play_pause(cx);
                    });
                },
                cx,
            );
        });
        div().size_0()
    }
}

/// one dialog slot per window, window prompt has none so two hosts
/// seeing the same blocked control would stack two dialogs
#[derive(Default)]
struct MusicListeningControlAlertPresence {
    is_presenting: bool,
}

impl Global for MusicListeningControlAlertPresence {}

/// window dialog stands for the alert, any answer clears the control like the binding setter does
pub struct MusicListeningControlAlert {
    id: ElementId,
    is_active: bool,
}

impl MusicListeningControlAlert {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into(), is_active: true }
    }

    pub fn is_active(mut self, is_active: bool) -> Self {
        self.is_active = is_active;
        self
    }

    fn present(player: Entity<MusicPlayerState>, window: &mut Window, cx: &mut App) {
        if cx.default_global::<MusicListeningControlAlertPresence>().is_presenting {
            return;
        }
        let message = match player.read(cx).blocked_listening_control {
            Some(control) => tr_args(
                "music_live_control_locked_specific",
                &[FormatArg::Text(control.localized_name().into())],
            ),
            None => tr("music_live_control_locked_message"),
        };
        cx.default_global::<MusicListeningControlAlertPresence>().is_presenting = true;
        let answer = window.prompt(
            PromptLevel::Warning,
            &tr("music_live_control_locked_title"),
            Some(message.as_str()),
            &[
                PromptButton::ok(tr("music_live_request_permissions")),
                PromptButton::new(tr("music_live_leave")),
                PromptButton::cancel(tr("cancel")),
            ],
            cx,
        );
        cx.spawn(async move |cx| {
            let answer = answer.await.ok();
            let _ = cx.update(|cx| {
                cx.default_global::<MusicListeningControlAlertPresence>().is_presenting = false;
            });
            let _ = player.update(cx, |player, cx| {
                match answer {
                    Some(0) => player.request_blocked_listening_permission(cx),
                    Some(1) => player.leave_current_playlist_listening_session(cx),
                    _ => {}
                }
                player.blocked_listening_control = None;
                cx.notify();
            });
        })
        .detach();
    }
}

impl ViewModifier for MusicListeningControlAlert {
    fn body(self, content: AnyElement, window: &mut Window, cx: &mut App) -> AnyElement {
        let player = observed_music_player(child_id(&self.id, "player"), window, cx);
        let is_presented = self.is_active && player.read(cx).blocked_listening_control.is_some();
        on_change(child_id(&self.id, "presented"), is_presented, true, window, cx, move |_, is_presented, window, cx| {
            if *is_presented {
                Self::present(player, window, cx);
            }
        });
        content
    }
}

struct NowPlayingState {
    show_add_to_playlist: bool,
    show_edit_local_track: bool,
    track_menu_presented_in: Option<ElementId>,
    queue_sheet_lowest_fraction: f32,
    is_queue_panel_open: bool,
    artwork_scale: Animated<f32>,
    focus: FocusHandle,
}

impl NowPlayingState {
    fn new(is_playing: bool, cx: &mut Context<Self>) -> Self {
        install_key_bindings(cx);
        Self {
            show_add_to_playlist: false,
            show_edit_local_track: false,
            track_menu_presented_in: None,
            queue_sheet_lowest_fraction: QUEUE_SHEET_FRACTIONS[0],
            is_queue_panel_open: SidePanelPreferences::standard().is_open(QUEUE_PANEL_CONTEXT),
            artwork_scale: Animated::new(NowPlaying::artwork_scale(is_playing)),
            focus: cx.focus_handle(),
        }
    }

    fn set_queue_panel_open(&mut self, is_open: bool, cx: &mut Context<Self>) {
        self.is_queue_panel_open = is_open;
        SidePanelPreferences::standard().remember_open(QUEUE_PANEL_CONTEXT, is_open);
        cx.notify();
    }
}

/// the windows player state so the right panel player and the full player share one
/// track menu, one add to playlist sheet and one artwork animation
struct NowPlayingSharedState(WeakEntity<NowPlayingState>);

impl Global for NowPlayingSharedState {}

#[derive(IntoElement)]
pub struct NowPlaying {
    id: ElementId,
}

/// desktop only, the full player as a root layer of its own
/// position publication, seek bar tick and cover spring redraw only this layer
/// a redraw elsewhere replays it
pub struct NowPlayingHost {
    _frontmost_observation: Subscription,
}

impl NowPlayingHost {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let frontmost_observation = match now_playing_aside_hooks() {
            Some(hooks) => (hooks.observe_frontmost)(cx),
            None => Subscription::new(|| {}),
        };
        Self { _frontmost_observation: frontmost_observation }
    }
}

impl Render for NowPlayingHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full().child(NowPlaying::new("root-now-playing"))
    }
}

/// desktop only, the music sections right panel shows the queue
/// a track of a registered artist adds its cover and artist card above it
/// transport is the bottom strip, pages it opens stay in the panel
#[derive(IntoElement)]
pub struct NowPlayingAside {
    id: ElementId,
}

impl NowPlayingAside {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into() }
    }
}

impl RenderOnce for NowPlayingAside {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let player = observed_music_player(child_id(&self.id, "player"), window, cx);
        let shared = cx.try_global::<NowPlayingSharedState>().and_then(|shared| shared.0.upgrade());
        let state = match shared {
            Some(state) => state,
            None => {
                let is_playing = player.read(cx).is_playing;
                window.use_keyed_state(child_id(&self.id, "state"), cx, move |_, cx| NowPlayingState::new(is_playing, cx))
            }
        };
        let parts = NowPlayingParts {
            id: self.id.clone(),
            player: player.clone(),
            state,
            nav_store: env::<NavigationStoreKey>(cx),
            is_aside: true,
        };
        let track = player.read(cx).current_track.clone();
        match track {
            Some(track) => parts.aside_content(track, window, cx),
            None => div()
                .size_full()
                .bg(rise_ui::theme(cx).bg._100)
                .child(MusicQueueSheet::new(child_id(&self.id, "queue")).list_only().follows_theme())
                .into_any_element(),
        }
    }
}

/// desktop only, a block of the full player drawn by a view of its own
/// a slot replays its last frame unless an entity it read or its size changed
/// so backdrop, cover and queue arent laid out again for a moving playhead or sliding sheet
struct NowPlayingSlot {
    content: Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>,
}

impl Render for NowPlayingSlot {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self.content.clone();
        content(window, cx)
    }
}

/// what every part of the sheet body shares
#[derive(Clone)]
struct NowPlayingParts {
    id: ElementId,
    player: Entity<MusicPlayerState>,
    state: Entity<NowPlayingState>,
    nav_store: Option<Entity<NavigationStore>>,
    is_aside: bool,
}

impl NowPlaying {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self { id: id.into() }
    }

    fn artwork_scale(is_playing: bool) -> f32 {
        if is_playing { 1. } else { 0.88 }
    }

    fn top_handle_clearance(safe_top: f32) -> f32 {
        safe_top
            + BottomSheetMetrics::HANDLE_TOP_PADDING
            + BottomSheetMetrics::HANDLE_HEIGHT
            + BottomSheetMetrics::HANDLE_BOTTOM_PADDING
    }

    /// the panels close button or a row may hold keyboard focus
    /// once the panel leaves the frame keys dispatch from the window root and the cover escape went dead
    fn set_queue_panel_open(state: &Entity<NowPlayingState>, is_open: bool, window: &mut Window, cx: &mut App) {
        let focus = state.update(cx, |state, cx| {
            state.set_queue_panel_open(is_open, cx);
            state.focus.clone()
        });
        if !is_open {
            window.focus(&focus, cx);
        }
    }

    /// why: a press on a control inside the cover could leave focus empty or on the window root
    /// then escape went past the player
    /// only focus on an ancestor comes back, a menu or sheet over the player keeps its own
    fn reclaim_focus(focus: &FocusHandle, window: &mut Window, cx: &mut App) {
        if !focus.is_in_rendered_frame(window) || focus.contains_focused(window, cx) {
            return;
        }
        let is_off_the_key_path = match window.focused(cx) {
            None => true,
            Some(focused) => focused.contains(focus, window) || !focused.is_in_rendered_frame(window),
        };
        if is_off_the_key_path {
            window.focus(focus, cx);
        }
    }

    fn queue_panel_fits(window: &Window, cx: &App) -> bool {
        let required = f32::from(rise_ui::theme(cx).panel.default_width) + PLAYER_COLUMN_MAX_WIDTH;
        f32::from(window.viewport_size().width) >= required
    }

    fn queue_sheet_snap_fractions(lowest: f32) -> Vec<f32> {
        let top = QUEUE_SHEET_FRACTIONS[1];
        if lowest < top { vec![lowest, top] } else { vec![lowest] }
    }

    fn update_queue_sheet_lowest_fraction(
        state: &Entity<NowPlayingState>,
        content_height: f32,
        window: &mut Window,
        cx: &mut App,
    ) {
        let screen_height = f32::from(window.viewport_size().height);
        if screen_height <= 0. {
            return;
        }
        let chrome = BottomSheetMetrics::HANDLE_TOP_PADDING
            + BottomSheetMetrics::HANDLE_HEIGHT
            + BottomSheetMetrics::HANDLE_BOTTOM_PADDING;
        let needed = ((content_height + chrome) / screen_height).min(1.);
        let fraction = QUEUE_SHEET_FRACTIONS[0].max(needed);
        state.update(cx, |state, cx| {
            if (fraction - state.queue_sheet_lowest_fraction).abs() <= 0.005 {
                return;
            }
            state.queue_sheet_lowest_fraction = fraction;
            cx.notify();
        });
    }

    fn present_pending_transition_alert(player: Entity<MusicPlayerState>, window: &mut Window, cx: &mut App) {
        let message = tr("music_live_switch_session_message");
        let answer = window.prompt(
            PromptLevel::Warning,
            &tr("music_live_switch_session_title"),
            Some(message.as_str()),
            &[PromptButton::ok(tr("music_live_switch_session_confirm")), PromptButton::cancel(tr("cancel"))],
            cx,
        );
        cx.spawn(async move |cx| {
            let answer = answer.await.ok();
            let _ = player.update(cx, |player, cx| {
                if answer == Some(0) {
                    player.confirm_pending_listening_session_transition(cx);
                } else {
                    player.cancel_pending_listening_session_transition(cx);
                }
            });
        })
        .detach();
    }

    fn bottom_caption_size(captions: &[String], slot_width: f32, text_delta: f32, window: &Window) -> f32 {
        let key = format!("{}|{}|{}", captions.join("\u{1F}"), slot_width.round() as i64, text_delta);
        if let Some(cached) = BOTTOM_CAPTION_SIZE_CACHE.object(&key) {
            return cached;
        }

        let face = font(FONT_FAMILY);
        let rendered_max = (BOTTOM_CAPTION_MAX_SIZE + text_delta).max(1.);
        let mut widest_caption = 0_f32;
        for caption in captions.iter().filter(|caption| !caption.is_empty()) {
            let runs = [TextRun {
                len: caption.len(),
                font: face.clone(),
                color: black(),
                background_color: None,
                underline: None,
                strikethrough: None,
            }];
            let width = window
                .text_system()
                .shape_line(SharedString::from(caption.clone()), px(rendered_max), &runs, None)
                .width();
            widest_caption = widest_caption.max(f32::from(width));
        }
        let available = slot_width - 4.;
        let mut size = BOTTOM_CAPTION_MAX_SIZE;
        if widest_caption > available && available > 0. {
            let fitting_rendered = rendered_max * available / widest_caption;
            let scaled = fitting_rendered - text_delta;
            size = BOTTOM_CAPTION_MIN_SIZE.max((scaled * 2.).floor() / 2.).min(BOTTOM_CAPTION_MAX_SIZE);
        }

        BOTTOM_CAPTION_SIZE_CACHE.set_object(size, key);
        size
    }

    fn notify_top(title: String, content: Option<String>, r#type: NotifierType, symbol: &str, cx: &mut App) {
        Notifier::send(
            title,
            NotifierSendOptions {
                content,
                r#type,
                position: NotifierPosition::Top,
                icon: NotifierIcon::System(symbol.to_owned()),
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    fn show_device_draft_notice(cx: &mut App) {
        Self::notify_top(
            tr("music_local_upload_not_sent_title"),
            Some(tr("music_local_upload_not_sent_message")),
            NotifierType::Warning,
            "exclamationmark.triangle.fill",
            cx,
        );
    }
}

impl NowPlayingParts {
    fn child(&self, name: &'static str) -> ElementId {
        child_id(&self.id, name)
    }

    fn track_child(&self, name: &str, track: &TrackModel) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::from(format!("{name}-{}", track.id)))
    }

    fn listening_playlist_id(&self, cx: &App) -> Option<String> {
        self.player.read(cx).listening_playlist_id().map(str::to_owned)
    }

    /// content reads what it shows from the player and state itself, those reads redraw the slot
    /// a new closure alone doesnt, nothing it captures changes without them
    fn slot(
        &self,
        name: &'static str,
        style: StyleRefinement,
        window: &mut Window,
        cx: &mut App,
        content: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> AnyElement {
        let content: Rc<dyn Fn(&mut Window, &mut App) -> AnyElement> = Rc::new(content);
        let first = content.clone();
        let slot = window.with_global_id(self.child(name), |global_id, window| {
            window.with_element_state(global_id, |held: Option<Entity<NowPlayingSlot>>, _| {
                let slot = held.unwrap_or_else(|| cx.new(|_| NowPlayingSlot { content: first }));
                (slot.clone(), slot)
            })
        });
        slot.update(cx, |slot, _| slot.content = content);
        slot.cached(style).replay_when_moved().into_any_element()
    }

    fn push(&self, route: RootRoute, cx: &mut App) {
        self.player.update(cx, |player, cx| player.set_show_now_playing(false, cx));
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn aside_content(&self, track: TrackModel, window: &mut Window, cx: &mut App) -> AnyElement {
        let artist_id = (track.artist_is_registered && !track.artist_id.is_empty()).then(|| track.artist_id.clone());
        on_change(self.child("aside-artist-request"), artist_id.clone(), true, window, cx, |_, artist_id, _, cx| {
            if let Some(artist_id) = artist_id {
                MusicInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.open_now_playing_artist(artist_id, cx));
            }
        });

        let queue = div()
            .w_full()
            .flex_1()
            .min_h(px(ASIDE_QUEUE_MIN_HEIGHT))
            .child(MusicQueueSheet::new(self.child("aside-queue")).list_only().follows_theme());
        let background = rise_ui::theme(cx).bg._100;
        let body = div().id(self.child("aside-body")).size_full().flex().flex_col().overflow_hidden().bg(background);
        let Some(artist_id) = artist_id else {
            return body.child(queue).into_any_element();
        };

        let artist = MusicActionsStore::slices(cx)
            .now_playing_artist_sai
            .data()
            .and_then(|data| data.artist)
            .filter(|artist| artist.id == artist_id);
        let viewer_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let theme = rise_ui::theme(cx).clone();
        let width = f32::from(SidePanelPreferences::standard().width(&theme.panel));
        let viewport_height = f32::from(window.viewport_size().height);
        let cover_side = (width - ASIDE_HORIZONTAL_PADDING * 2.)
            .min(viewport_height * ASIDE_COVER_HEIGHT_SHARE)
            .min(ASIDE_COVER_MAX_SIDE)
            .max(120.);

        let header = vstack(0.)
            .w_full()
            .px(px(ASIDE_HORIZONTAL_PADDING))
            .pt(px(ASIDE_TOP_PADDING))
            .pb(px(12.))
            .child(
                div().w_full().flex().justify_center().child(
                    div()
                        .size(px(cover_side))
                        .continuous_corners(ASIDE_CARD_RADIUS)
                        .overflow_hidden()
                        .child(self.cover_art(&track, ASIDE_CARD_RADIUS, "aside-cover")),
                ),
            )
            .child(
                vstack(2.)
                    .w_full()
                    .pt(px(14.))
                    .pb(px(16.))
                    .items_start()
                    .child(MainText::new(track.title.clone()).px(18.).font_weight(FontWeight::BOLD).color(theme.text.primary).line_limit(1))
                    .child(SecondaryText::new(track.artist.clone()).px(14.).color(theme.text.secondary).line_limit(1)),
            )
            .children(artist.map(|artist| self.aside_artist_card(artist, viewer_id.as_deref(), &theme)));

        body.child(
            div()
                .id(self.child("aside-artist-scroll"))
                .w_full()
                .flex_shrink_1()
                .min_h_0()
                .max_h(relative(ASIDE_HEADER_MAX_SHARE))
                .overflow_y_scroll()
                .child(header),
        )
        .child(queue)
        .into_any_element()
    }

    fn aside_artist_card(&self, artist: WireArtistDTO, viewer_id: Option<&str>, theme: &AppTheme) -> AnyElement {
        let is_own_artist = !artist.user_id.is_empty() && viewer_id == Some(artist.user_id.as_str());
        let monthly_listeners = artist.monthly_listeners.max(0);
        let listeners = tr_plural_with(
            "music_monthly_listeners",
            monthly_listeners,
            FormatArg::Text(RiseFormat::compact_count(monthly_listeners).into()),
        );
        let bio = trimming_whitespaces_and_newlines(&artist.bio).to_owned();
        let photo_id = ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::from(format!("aside-artist-photo-{}", artist.id)));
        let has_photo = MusicCoverMedia::has_remote_cover(&artist.avatar_url);

        let photo = div()
            .relative()
            .w_full()
            .h(px(ASIDE_ARTIST_PHOTO_HEIGHT))
            .overflow_hidden()
            .bg(LiquidChrome::surface_fill(theme, 1))
            .when(has_photo, |photo| {
                photo.child(
                    MusicCoverImage::new(photo_id, artist.avatar_url.clone())
                        .size(None::<f32>)
                        .corner_radius(0.)
                        .fills_container(true)
                        .shows_placeholder_note(false),
                )
            })
            .child(
                LinearGradient::colors([black().opacity(0.55), transparent_black()], UnitPoint::TOP, UnitPoint::BOTTOM)
                    .view()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .h(px(64.)),
            )
            .child(
                div()
                    .absolute()
                    .top(px(14.))
                    .left(px(16.))
                    .right(px(16.))
                    .child(MainText::new(tr("music_artist_bio")).px(16.).font_weight(FontWeight::BOLD).color(white()).line_limit(1)),
            );

        let parts = self.clone();
        let route_artist_id = artist.id.clone();
        let name = plain(self.child("aside-artist-name"))
            .w_full()
            .on_press(move |_, cx| {
                parts.push(RootRoute::MusicArtistProfile { artist_id: route_artist_id.clone() }, cx);
            })
            .child(
                MainText::new(artist.stage_name.clone())
                    .px(17.)
                    .font_weight(FontWeight::BOLD)
                    .color(theme.text.primary)
                    .line_limit(1),
            );

        let follow_artist_id = artist.id.clone();
        let follow = plain(self.child("aside-artist-follow"))
            .on_press(move |_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.toggle_artist_follow(&follow_artist_id, cx));
            })
            .child(
                div()
                    .px(px(18.))
                    .py(px(8.))
                    .capsule()
                    .bg(LiquidChrome::surface_fill(theme, 1))
                    .border_1()
                    .border_color(theme.border._100)
                    .swiftui_font(Font::system_weight(14.0, FontWeight::SEMIBOLD))
                    .text_color(theme.text.primary)
                    .child(if artist.is_following { tr("music_unfollow") } else { tr("music_follow") }),
            );

        let details = vstack(8.)
            .w_full()
            .p(px(16.))
            .items_start()
            .child(name)
            .child(SecondaryText::new(listeners).px(14.).color(theme.text.secondary).line_limit(1))
            .when(!is_own_artist, |details| details.child(follow))
            .when(!bio.is_empty(), |details| {
                details.child(SecondaryText::new(bio).px(14.).color(theme.text.secondary).line_limit(ASIDE_ARTIST_BIO_LINES))
            });

        vstack(0.)
            .w_full()
            .continuous_corners(ASIDE_CARD_RADIUS)
            .overflow_hidden()
            .bg(LiquidChrome::surface_fill(theme, 0))
            .child(photo)
            .child(details)
            .into_any_element()
    }

    fn sheet_body(&self, window: &mut Window, cx: &mut App) -> AnyElement {
        match self.player.read(cx).current_track.clone() {
            Some(track) => self.now_playing_content(track, window, cx),
            None => div().w_full().h(window.viewport_size().height).into_any_element(),
        }
    }

    fn now_playing_content(&self, track: TrackModel, window: &mut Window, cx: &mut App) -> AnyElement {
        let is_liked =
            MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.is_liked(&track, cx));
        let cover = WindowCoverFrame::resolve(window, cx);
        let viewport = cover.size;
        let theme = rise_ui::theme(cx);
        let safe_top = f32::from(cover.safe_top);
        let panel_metrics_width = SidePanelPreferences::standard().width(&theme.panel);
        let is_panel_visible = self.state.read(cx).is_queue_panel_open && NowPlaying::queue_panel_fits(window, cx);
        let controls_band = window_controls_band(window, cx);
        let panel_width = if is_panel_visible { f32::from(panel_metrics_width) } else { 0. };

        let screen_h = f32::from(viewport.height);
        let width = (f32::from(viewport.width) - panel_width).min(PLAYER_COLUMN_MAX_WIDTH);
        let handle_clearance = NowPlaying::top_handle_clearance(safe_top);
        let bottom_pad = 28.;
        let controls_reserve = 330. + BOTTOM_SLOT_HEIGHT_GROWTH;
        let cover_slot_height = (screen_h - handle_clearance - controls_reserve - bottom_pad).max(120.);
        let cover_side = (width - 64.).min(cover_slot_height - COVER_SLOT_INSET * 2.);

        let backdrop_parts = self.clone();
        let backdrop =
            self.slot("backdrop-slot", StyleRefinement::default().absolute().inset_0(), window, cx, move |_, cx| {
                let Some(track) = backdrop_parts.player.read(cx).current_track.clone() else {
                    return Empty.into_any_element();
                };
                div()
                    .size_full()
                    .overflow_hidden()
                    .child(div().size_full().child(backdrop_parts.backdrop_art(&track, "backdrop-cover")).blur(50.))
                    .child(backdrop_parts.now_playing_backdrop_overlay(&track))
                    .into_any_element()
            });
        let cover_parts = self.clone();
        let cover_slot = self.slot("cover-slot", StyleRefinement::default().size_full(), window, cx, move |window, cx| {
            let Some(track) = cover_parts.player.read(cx).current_track.clone() else {
                return Empty.into_any_element();
            };
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(cover_parts.cover(&track, cover_side, window, cx))
                .into_any_element()
        });
        let queue_parts = self.clone();
        let queue_panel = is_panel_visible.then(|| {
            self.slot(
                "queue-panel-slot",
                StyleRefinement::default().w(px(panel_width)).h_full().flex_none(),
                window,
                cx,
                move |_, _| queue_parts.queue_panel(panel_width, controls_band).into_any_element(),
            )
        });

        let player_column = div()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .items_center()
            .child(self.handle_band(handle_clearance))
            .child(self.close_button(controls_band))
            .child(
                div().w_full().h(px(cover_slot_height)).flex_none().child(cover_slot),
            )
            .child(
                div()
                    .w(px(width))
                    .h(px(controls_reserve))
                    .flex_none()
                    .mb(px(bottom_pad))
                    .child(self.bottom_controls(&track, is_liked, width, window, cx)),
            );

        let focus = self.state.read(cx).focus.clone();
        let cover_viewer = media_viewer_presentation(cx);
        if self.player.read(cx).show_now_playing() && !cover_viewer.read(cx).is_presented() {
            NowPlaying::reclaim_focus(&focus, window, cx);
        }
        let page_focus = PageFocus::keyed_with(self.child("page-focus"), focus.clone(), window, cx);
        let leaving_focus = page_focus.clone();
        let key_player = self.player.clone();
        let key_state = self.state.clone();
        let toggled_state = self.state.clone();
        let duration = track.duration_seconds as f64;

        div()
            .id(self.child("body"))
            .relative()
            .w_full()
            .h(px(screen_h))
            .flex()
            .flex_row()
            .track_focus(&focus)
            .capture_any_mouse_up({
                let focus = focus.clone();
                let press_player = self.player.clone();
                let cover_viewer = cover_viewer.clone();
                move |_, window, cx| {
                    let focus = focus.clone();
                    let press_player = press_player.clone();
                    let cover_viewer = cover_viewer.clone();
                    window.defer(cx, move |window, cx| {
                        if !press_player.read(cx).show_now_playing()
                            || cover_viewer.read(cx).is_presented()
                            || !focus.is_in_rendered_frame(window)
                            || focus.contains_focused(window, cx)
                            || ScreenFocus::text_field_is_focused(window, cx)
                        {
                            return;
                        }
                        let stale = window.focused(cx).map(|focused| !focused.is_in_rendered_frame(window));
                        tracing::warn!(
                            target: "riseonly::X494",
                            "press left focus off the player idle={} stale={:?}",
                            ScreenFocus::is_idle(window, cx),
                            stale
                        );
                        window.focus(&focus, cx);
                    });
                }
            })
            .on_action(move |_: &ToggleSidePanel, window, cx| {
                let is_open = !toggled_state.read(cx).is_queue_panel_open;
                NowPlaying::set_queue_panel_open(&toggled_state, is_open, window, cx);
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let modifiers = &event.keystroke.modifiers;
                if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function || modifiers.shift {
                    return;
                }
                match event.keystroke.key.as_str() {
                    "escape" if is_panel_visible => {
                        NowPlaying::set_queue_panel_open(&key_state, false, window, cx);
                    }
                    "escape" => key_player.update(cx, |player, cx| player.set_show_now_playing(false, cx)),
                    "space" => key_player.update(cx, |player, cx| player.toggle_play_pause(cx)),
                    "left" => MusicLivePlaybackSeekBar::adjust(duration, false, &key_player, cx),
                    "right" => MusicLivePlaybackSeekBar::adjust(duration, true, &key_player, cx),
                    _ => return,
                }
                cx.stop_propagation();
            })
            .child(backdrop)
            .child(player_column)
            .children(queue_panel)
            .on_appear(self.child("focus"), move |window, cx| PageFocus::did_appear(&page_focus, window, cx))
            .and_on_disappear(move |window, cx| {
                if let Some(window) = window {
                    PageFocus::will_disappear(&leaving_focus, window, cx);
                }
            })
            .into_any_element()
    }

    fn handle_band(&self, height: f32) -> impl IntoElement {
        let close_player = self.player.clone();
        div()
            .id(self.child("handle-band"))
            .w_full()
            .h(px(height))
            .flex_none()
            .cursor_pointer()
            .on_click(move |_, _, cx| close_player.update(cx, |player, cx| player.set_show_now_playing(false, cx)))
    }

    fn close_button(&self, controls_band: Pixels) -> impl IntoElement {
        let close_player = self.player.clone();
        let label = tr("close");
        div()
            .id(self.child("close-hint"))
            .absolute()
            .top(controls_band)
            .left(px(8.))
            .occlude()
            .aria_label(label.clone())
            .tooltip_label(label)
            .child(
                plain(self.child("close"))
                    .size(px(32.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .on_press(move |_, cx| close_player.update(cx, |player, cx| player.set_show_now_playing(false, cx)))
                    .children(icon("chevron.down", 16., white().opacity(0.7))),
            )
    }

    fn queue_panel(&self, panel_width: f32, controls_band: Pixels) -> impl IntoElement {
        let close_state = self.state.clone();
        let label = tr("close");
        div()
            .relative()
            .w(px(panel_width))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .bg(black())
            .child(
                hstack(0.).w_full().h(px(36.) + controls_band).pt(controls_band).flex_none().justify_end().px(px(8.)).child(
                    div().id(self.child("queue-close-hint")).aria_label(label.clone()).tooltip_label(label).child(
                        plain(self.child("queue-close"))
                            .size(px(32.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .on_press(move |window, cx| {
                                NowPlaying::set_queue_panel_open(&close_state, false, window, cx);
                            })
                            .children(icon("xmark", 14., white().opacity(0.7))),
                    ),
                ),
            )
            .child(div().flex_1().min_h_0().child(MusicQueueSheet::new(self.child("queue-panel")).list_only()))
    }

    fn cover(&self, track: &TrackModel, cover_side: f32, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let is_preparing = self.player.read(cx).is_preparing_playback;
        let scale = self.state.read(cx).artwork_scale.frame_value(RiseClock::monotonic(), window);
        let draft_label = tr("music_local_upload_not_sent_title");

        let face = div()
            .size_full()
            .continuous_corners(COVER_CORNER_RADIUS)
            .overflow_hidden()
            .child(self.cover_art(track, COVER_CORNER_RADIUS, "cover"))
            .guide_anchor(RiseGuideAnchor::NowPlayingCover);

        div()
            .relative()
            .size(px(cover_side))
            .rounded(px(COVER_CORNER_RADIUS))
            .shadow(vec![BoxShadow {
                color: black().opacity(0.45),
                offset: point(px(0.), px(14.)),
                blur_radius: px(28.),
                spread_radius: px(0.),
                inset: false,
            }])
            .child(face)
            .when(is_preparing, |cover| {
                cover.child(
                    div()
                        .absolute()
                        .inset_0()
                        .continuous_corners(COVER_CORNER_RADIUS)
                        .bg(black().opacity(0.38))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(div().child(progress_view().tint(white()).scale_effect(1.15))),
                )
            })
            .when(track.is_device_only_draft(), |cover| {
                cover.child(
                    div()
                        .id(self.child("draft-hint"))
                        .absolute()
                        .top(px(14.))
                        .right(px(14.))
                        .aria_label(draft_label.clone())
                        .tooltip_label(draft_label)
                        .child(
                            plain(self.child("draft"))
                                .size(px(38.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .border_1()
                                .border_color(white().opacity(0.22))
                                .bg(black().opacity(0.58))
                                .on_press(|_, cx| {
                                    cx.stop_propagation();
                                    NowPlaying::show_device_draft_notice(cx);
                                })
                                .children(icon("exclamationmark.triangle.fill", 15., white())),
                        ),
                )
            })
            .scale_effect(scale)
            .music_cover_media_viewer(
                &format!("track-{}", track.id),
                &track.cover_image_url,
                &track.title,
                COVER_CORNER_RADIUS,
            )
    }

    fn cover_art(&self, track: &TrackModel, corner_radius: f32, name: &str) -> MusicCoverImage {
        MusicCoverImage::new(self.track_child(name, track), track.cover_image_url.clone())
            .fallback_colors(track.cover_colors.clone())
            .size(None::<f32>)
            .corner_radius(corner_radius)
            .fills_container(true)
    }

    fn backdrop_art(&self, track: &TrackModel, name: &str) -> impl IntoElement {
        self.cover_art(track, 0., name).shows_placeholder_note(false).shows_loading_indicator(false)
    }

    fn now_playing_backdrop_overlay(&self, track: &TrackModel) -> AnyElement {
        if MusicCoverMedia::has_remote_cover(&track.cover_image_url) {
            return div().absolute().inset_0().bg(black().opacity(0.72)).into_any_element();
        }
        LinearGradient::colors(
            [
                Hsla::from(rgb(0x1A050D)).opacity(0.88),
                Hsla::from(rgb(0x3D1228)).opacity(0.82),
                black().opacity(0.78),
            ],
            UnitPoint::TOP_LEADING,
            UnitPoint::BOTTOM_TRAILING,
        )
        .view()
        .absolute()
        .inset_0()
        .into_any_element()
    }

    fn bottom_controls(
        &self,
        track: &TrackModel,
        is_liked: bool,
        width: f32,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let shows_effects = self.player.read(cx).is_music_playback();
        let shows_listeners = self.player.update(cx, |player, cx| player.can_show_playlist_listeners(cx));
        let mut captions = vec![
            tr("music_add_to_profile"),
            tr("music_queue_title"),
            tr("music_now_playing_share"),
        ];
        if shows_effects {
            captions.push(tr("music_effects_short"));
        }
        if shows_listeners {
            captions.push(tr("music_live_listeners_short"));
        }
        let text_delta = if ignores_text_size_delta(cx) { 0.0 } else { ThemeStore::text_delta_snapshot() };
        let caption_size = NowPlaying::bottom_caption_size(
            &captions,
            (width - BOTTOM_ROW_HORIZONTAL_PADDING * 2.) / captions.len() as f32,
            text_delta,
            window,
        );

        let seek_bar = if self.is_aside {
            self.seek_bar(track.duration_seconds, cx).into_any_element()
        } else {
            let parts = self.clone();
            self.slot(
                "seek-bar-slot",
                StyleRefinement::default().w_full().h(px(MusicLivePlaybackSeekBar::height())).flex_none(),
                window,
                cx,
                move |_, cx| {
                    let duration = parts.player.read(cx).current_track.as_ref().map_or(0, |track| track.duration_seconds);
                    parts.seek_bar(duration, cx).into_any_element()
                },
            )
        };

        vstack(0.)
            .size_full()
            .child(
                vstack(12.)
                    .w_full()
                    .child(self.title_row(track, is_liked, cx))
                    .child(seek_bar)
                    .child(self.transport_row(cx)),
            )
            .child(spacer_min_h(16.))
            .child(self.bottom_row(track, caption_size, shows_effects, shows_listeners, cx))
    }

    fn seek_bar(&self, duration_seconds: i64, cx: &App) -> MusicLivePlaybackSeekBar {
        MusicLivePlaybackSeekBar::new(self.child("seek-bar"), self.player.clone(), duration_seconds)
            .playlist_id(self.listening_playlist_id(cx))
            .is_covered(self.is_aside && self.player.read(cx).show_now_playing())
    }

    fn title_row(&self, track: &TrackModel, is_liked: bool, cx: &mut App) -> impl IntoElement {
        let artist_parts = self.clone();
        let artist_track = track.clone();
        let like_track_id = track.id.clone();
        let like_label = tr(if is_liked { "ctx_unlike" } else { "ctx_like" });
        let like_icon = if is_liked { "heart.fill" } else { "heart" };
        let like_color = if is_liked { accent() } else { white().opacity(0.8) };

        hstack(8.)
            .w_full()
            .px(px(32.))
            .child(
                vstack(4.)
                    .flex_1()
                    .min_w_0()
                    .items_start()
                    .child(
                        hstack(8.)
                            .w_full()
                            .child(
                                MusicMarqueeTitle::new(self.track_child("title", track), track.title.clone())
                                    .hugs_text(track.is_exclusive)
                                    .is_active(self.player.read(cx).is_playing),
                            )
                            .when(track.is_exclusive, |row| row.child(MusicExclusiveBadge::new().compact(true))),
                    )
                    .child(
                        plain(self.child("artist"))
                            .on_press(move |_, cx| {
                                if artist_track.artist_is_registered && !artist_track.artist_id.is_empty() {
                                    let route = RootRoute::MusicArtistProfile { artist_id: artist_track.artist_id.clone() };
                                    artist_parts.push(route, cx);
                                } else {
                                    NowPlaying::notify_top(
                                        tr("music_artist_not_registered"),
                                        None,
                                        NotifierType::Info,
                                        "person.crop.circle.badge.questionmark",
                                        cx,
                                    );
                                }
                            })
                            .child(MainText::new(track.artist.clone()).px(16.).color(white().opacity(0.7)).line_limit(1)),
                    ),
            )
            .child(self.now_playing_menu(track, cx))
            .child(
                div()
                    .id(self.child("like-hint"))
                    .aria_label(like_label.clone())
                    .tooltip_label(like_label)
                    .child(
                        plain(self.child("like"))
                            .size(px(44.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .on_press(move |_, cx| {
                                MusicInteractionsStore::shared(cx)
                                    .update(cx, |interactions, cx| interactions.toggle_like(&like_track_id, cx));
                            })
                            .children(icon(like_icon, 24., like_color)),
                    ),
            )
    }

    /// each control hit tests its glyph only like a plain button, the slot is just its place
    fn transport_button(
        &self,
        name: &'static str,
        label: String,
        content: impl IntoElement,
        action: impl Fn(&mut MusicPlayerState, &mut Context<MusicPlayerState>) + 'static,
    ) -> impl IntoElement {
        let player = self.player.clone();
        div()
            .id(self.child(name))
            .flex_1()
            .flex()
            .justify_center()
            .aria_label(label.clone())
            .tooltip_label(label)
            .child(
                plain(child_id(&self.child(name), "press"))
                    .on_press(move |_, cx| player.update(cx, |player, cx| action(player, cx)))
                    .child(content),
            )
    }

    fn transport_row(&self, cx: &App) -> impl IntoElement {
        let (is_shuffled, is_playing, repeat_mode) = {
            let player = self.player.read(cx);
            (player.is_shuffled(cx), player.is_playing, player.repeat_mode(cx))
        };
        let inactive = white().opacity(0.7);

        hstack(0.)
            .w_full()
            .px(px(24.))
            .child(self.transport_button(
                "shuffle",
                tr("music_shuffle"),
                div().children(icon("shuffle", 20., if is_shuffled { accent() } else { inactive })),
                |player, cx| player.toggle_shuffle(cx),
            ))
            .child(self.transport_button(
                "previous",
                tr("music_player_previous"),
                div().children(icon("backward.fill", 28., white())),
                |player, cx| player.previous(cx),
            ))
            .child(self.transport_button(
                "play",
                tr(if is_playing { "media_viewer_pause" } else { "media_viewer_play" }),
                div()
                    .size(px(72.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(white())
                    .children(icon(if is_playing { "pause.fill" } else { "play.fill" }, 28., black())),
                |player, cx| player.toggle_play_pause(cx),
            ))
            .child(self.transport_button(
                "next",
                tr("music_player_next"),
                div().children(icon("forward.fill", 28., white())),
                |player, cx| player.next(cx),
            ))
            .child(self.transport_button(
                "repeat",
                format!("{}: {}", tr("music_player_repeat"), repeat_mode.accessibility_value()),
                div().children(icon(
                    repeat_mode.icon(),
                    20.,
                    if repeat_mode != MusicRepeatMode::Off { accent() } else { inactive },
                )),
                |player, cx| player.cycle_repeat_mode(cx),
            ))
    }

    fn bottom_row(
        &self,
        track: &TrackModel,
        caption_size: f32,
        shows_effects: bool,
        shows_listeners: bool,
        cx: &mut App,
    ) -> impl IntoElement {
        let has_active_audio_effect = self.player.read(cx).has_active_audio_effect();
        let highlight = rise_ui::theme(cx).primary._100;
        let profile_track = track.clone();
        let share_track = track.clone();
        let share_player = self.player.clone();
        let queue_player = self.player.clone();
        let queue_state = self.state.clone();
        let is_aside = self.is_aside;

        let listeners = shows_listeners.then(|| self.bottom_listeners_action(caption_size, cx));

        hstack(0.)
            .w_full()
            .px(px(BOTTOM_ROW_HORIZONTAL_PADDING))
            .child(
                self.bottom_action(
                    "add-to-profile",
                    "person.crop.circle.badge.plus",
                    tr("music_add_to_profile"),
                    caption_size,
                    None,
                    track.transfer_phase.is_some(),
                    move |_, cx| {
                        let track = profile_track.clone();
                        MusicInteractionsStore::shared(cx)
                            .update(cx, |interactions, cx| interactions.add_track_to_profile(track, cx));
                    },
                )
                .guide_anchor(RiseGuideAnchor::NowPlayingAddToProfile),
            )
            .when(shows_effects, |row| {
                row.child(
                    self.bottom_action(
                        "effects",
                        if has_active_audio_effect { "waveform.path.ecg" } else { "waveform" },
                        tr("music_effects_short"),
                        caption_size,
                        has_active_audio_effect.then_some(highlight),
                        false,
                        |_, cx| {
                            MusicInteractionsStore::shared(cx)
                                .update(cx, |interactions, cx| interactions.open_audio_effects(cx));
                        },
                    )
                    .guide_anchor(RiseGuideAnchor::NowPlayingEffects),
                )
            })
            .child(self.bottom_action(
                "queue",
                "list.bullet",
                tr("music_queue_title"),
                caption_size,
                None,
                false,
                move |window, cx| {
                    if !is_aside && NowPlaying::queue_panel_fits(window, cx) {
                        let is_open = !queue_state.read(cx).is_queue_panel_open;
                        NowPlaying::set_queue_panel_open(&queue_state, is_open, window, cx);
                    } else {
                        queue_player.update(cx, |player, cx| {
                            player.show_queue_sheet = true;
                            cx.notify();
                        });
                    }
                },
            ))
            .child(self.bottom_action(
                "share",
                "square.and.arrow.up",
                tr("music_now_playing_share"),
                caption_size,
                None,
                false,
                move |_, cx| {
                    let playlist_id = share_player.read(cx).listening_playlist_id().map(str::to_owned);
                    let track = share_track.clone();
                    ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_track(track, playlist_id, cx));
                },
            ))
            .children(listeners)
    }

    #[allow(clippy::too_many_arguments)]
    fn bottom_action(
        &self,
        name: &'static str,
        symbol: &str,
        label: String,
        caption_size: f32,
        highlight: Option<Hsla>,
        disabled: bool,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> gpui::Stateful<Div> {
        let color = highlight.unwrap_or_else(|| white().opacity(0.7));
        self.bottom_slot(name, label, caption_size, highlight, disabled, action, div().children(icon(symbol, 19., color)))
    }

    /// every slot shares one fixed icon box and one fixed label box at one size for the row
    /// shrinking only long labels gave different sizes and baselines
    /// so the whole row shrinks together and captions stay on one line
    #[allow(clippy::too_many_arguments)]
    fn bottom_slot(
        &self,
        name: &'static str,
        label: String,
        caption_size: f32,
        highlight: Option<Hsla>,
        disabled: bool,
        action: impl Fn(&mut Window, &mut App) + 'static,
        icon_content: impl IntoElement,
    ) -> gpui::Stateful<Div> {
        let caption_color = highlight.map_or_else(|| white().opacity(0.5), |color| color.opacity(0.85));
        div()
            .id(self.child(name))
            .flex_1()
            .min_w_0()
            .when(disabled, |slot| slot.opacity(BOTTOM_SLOT_DISABLED_OPACITY))
            .child(
            plain(child_id(&self.child(name), "press"))
                .w_full()
                .min_h(px(BOTTOM_SLOT_HEIGHT))
                .flex()
                .flex_col()
                .items_center()
                .gap(px(5.))
                .disabled(disabled)
                .on_press(action)
                .child(
                    div()
                        .size(px(BOTTOM_SLOT_ICON_SIZE))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon_content),
                )
                .child(
                    div().w_full().min_w_0().h(px(BOTTOM_SLOT_LABEL_HEIGHT)).flex().justify_center().child(
                        MainText::new(label)
                            .px(caption_size)
                            .tac(TextAlignT::Center)
                            .color(caption_color)
                            .line_limit(1)
                            .minimum_scale_factor(BOTTOM_CAPTION_FALLBACK_SCALE),
                    ),
                ),
        )
    }

    fn current_track_listeners(&self, cx: &mut App) -> Vec<MusicPlaylistListener> {
        let (track_id, playlist_id) = {
            let player = self.player.read(cx);
            (player.current_track.as_ref().map(|track| track.id.clone()), player.listening_playlist_id().map(str::to_owned))
        };
        let (Some(track_id), Some(playlist_id)) = (track_id, playlist_id) else {
            return Vec::new();
        };
        let current_user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        MusicServicesStore::shared(cx)
            .update(cx, |services, cx| services.playlist_listening_snapshot(&playlist_id, None, cx))
            .map(|snapshot| snapshot.listeners)
            .unwrap_or_default()
            .into_iter()
            .filter(|listener| listener.track_id == track_id && Some(listener.user_id.as_str()) != current_user_id.as_deref())
            .collect()
    }

    fn bottom_listeners_action(&self, caption_size: f32, cx: &mut App) -> AnyElement {
        let listeners = self.current_track_listeners(cx);
        let hint = tr("music_live_listeners_title");
        let content = if listeners.is_empty() {
            div().children(icon("person", 19., white().opacity(0.7))).into_any_element()
        } else {
            let total_count = listeners.len() as i64;
            MusicPlaylistListenerCluster::new(self.child("listeners-cluster"), listeners, total_count)
                .size(BOTTOM_SLOT_ICON_SIZE)
                .is_loading(false)
                .into_any_element()
        };
        self.bottom_slot(
            "listeners",
            tr("music_live_listeners_short"),
            caption_size,
            None,
            false,
            |_, cx| {
                MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.open_playlist_listeners(cx));
            },
            content,
        )
        .aria_label(hint.clone())
        .tooltip_label(hint)
        .guide_anchor(RiseGuideAnchor::NowPlayingListeners)
        .into_any_element()
    }

    fn can_remove_current_track_from_playlist(&self, cx: &mut App) -> bool {
        let Some(playlist_id) = self.listening_playlist_id(cx) else {
            return false;
        };
        let Some(user_id) = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire() else {
            return false;
        };
        let playlist = MusicServicesStore::shared(cx).update(cx, |services, cx| services.playlist(&playlist_id, cx));
        playlist.and_then(|playlist| playlist.owner_user_id).as_deref() == Some(user_id.as_str())
    }

    fn now_playing_menu(&self, track: &TrackModel, cx: &mut App) -> impl IntoElement {
        let (read_owner, write_owner) = (self.id.clone(), self.id.clone());
        let is_presented = Binding::entity(
            self.state.clone(),
            move |state: &NowPlayingState| state.track_menu_presented_in.as_ref() == Some(&read_owner),
            move |state: &mut NowPlayingState, value: bool, cx| {
                if value {
                    state.track_menu_presented_in = Some(write_owner.clone());
                } else if state.track_menu_presented_in.as_ref() == Some(&write_owner) {
                    state.track_menu_presented_in = None;
                }
                cx.notify();
            },
        );
        let menu_actions = self.now_playing_menu_actions(track, cx);
        let actions: HoldContextMenuPresentationActions = Rc::new(move |_: &App| menu_actions.clone());
        let (parts, action_track) = (self.clone(), track.clone());
        let on_action: HoldContextMenuPresentationOnAction = Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            parts.handle_now_playing_menu_action(&id, &action_track, cx);
        });
        let source: HoldContextMenuTapSource =
            Rc::new(|_: &mut Window, _: &mut App| Self::now_playing_menu_glyph().into_any_element());
        let (opener, opener_id) = (self.state.clone(), self.id.clone());
        let menu = plain(self.child("menu-button"))
            .on_press(move |_, cx| {
                opener.update(cx, |state, cx| {
                    state.track_menu_presented_in = Some(opener_id.clone());
                    cx.notify();
                });
            })
            .child(Self::now_playing_menu_glyph())
            .tap_context_menu(
                self.child("menu"),
                is_presented,
                HoldContextMenuAlignment::Trailing,
                false,
                false,
                false,
                None,
                actions,
                on_action,
                source,
            );
        let label = tr("more");
        div()
            .id(self.child("menu-label"))
            .aria_label(label.clone())
            .tooltip_label(label)
            .child(menu)
    }

    fn now_playing_menu_glyph() -> Div {
        div()
            .size(px(44.))
            .flex()
            .items_center()
            .justify_center()
            .children(icon("ellipsis", 18., white().opacity(0.8)))
    }

    fn now_playing_menu_actions(&self, track: &TrackModel, cx: &mut App) -> Vec<HoldContextMenuAction> {
        let mut actions = Vec::new();
        if track.transfer_phase.is_some() {
            if !track.is_local_transfer_active() && track.is_device_only_draft() {
                let title = if track.transfer_phase == Some(MusicLocalTrackTransferPhase::WaitingForConnection) {
                    tr("music_local_upload_send_to_server")
                } else {
                    tr("music_message_import_retry")
                };
                actions.push(HoldContextMenuAction::new("retry_upload", title).system_image("arrow.up.circle"));
            }
            if MusicTrackSavePolicy::can_save(track) {
                actions.push(HoldContextMenuAction::new("save", tr("ctx_save")).system_image("arrow.down.doc"));
            }
            if track.can_remove_pending_local_track() {
                actions.push(
                    HoldContextMenuAction::new("remove_pending", tr("ctx_remove_from_playlist"))
                        .system_image("trash")
                        .destructive(true),
                );
            }
            return actions;
        }
        let is_liked = MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.is_liked(track, cx));
        actions.push(
            HoldContextMenuAction::new("like", if is_liked { tr("ctx_unlike") } else { tr("ctx_like") })
                .system_image(if is_liked { "heart.slash" } else { "heart" }),
        );
        let current_user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        if MusicLocalTrackEditPolicy::can_edit(track, current_user_id.as_deref()) {
            actions.push(HoldContextMenuAction::new("edit", tr("ctx_edit")).system_image("pencil"));
        }
        if self.can_remove_current_track_from_playlist(cx) {
            actions.push(
                HoldContextMenuAction::new("remove_from_playlist", tr("ctx_remove_from_playlist"))
                    .system_image("minus.circle"),
            );
        } else {
            actions.push(
                HoldContextMenuAction::new("add_to_playlist", tr("ctx_add_to_playlist")).system_image("music.note.list"),
            );
        }
        let playlist_id = self.listening_playlist_id(cx);
        if MusicTrackSharePolicy::can_share(track, playlist_id.as_deref()) {
            actions.push(HoldContextMenuAction::new("share", tr("ctx_share")).system_image("square.and.arrow.up"));
        }
        if MusicTrackSavePolicy::can_save(track) {
            actions.push(HoldContextMenuAction::new("save", tr("ctx_save")).system_image("arrow.down.doc"));
        }
        actions.push(
            HoldContextMenuAction::new("add_to_profile", tr("music_add_to_profile"))
                .system_image("person.crop.circle.badge.plus"),
        );
        if !track.artist_id.is_empty() {
            actions.push(HoldContextMenuAction::new("artist", tr("ctx_go_to_artist")).system_image(RiseSymbol::ARTIST));
        }
        if !track.album_id.is_empty() {
            actions.push(HoldContextMenuAction::new("album", tr("ctx_go_to_album")).system_image("square.stack"));
        }
        if MusicTrackReportPolicy::can_report(track) {
            actions.push(
                HoldContextMenuAction::new("report", tr("ctx_report"))
                    .system_image("exclamationmark.bubble")
                    .destructive(true),
            );
        }
        actions
    }

    fn handle_now_playing_menu_action(&self, id: &str, track: &TrackModel, cx: &mut App) {
        match id {
            "retry_upload" => {
                MusicInteractionsStore::shared(cx)
                    .update(cx, |interactions, cx| interactions.retry_pending_local_track(&track.id, cx));
            }
            "save" => {
                let track = track.clone();
                MusicTrackDownloadStore::shared(cx).update(cx, |store, cx| store.present_options(track, cx));
            }
            "remove_pending" => {
                let playlist_id = self.listening_playlist_id(cx);
                MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.remove_pending_local_track(playlist_id.as_deref(), &track.id, cx);
                });
                self.player.update(cx, |player, cx| {
                    player.stop_and_clear(cx);
                    player.set_show_now_playing(false, cx);
                });
            }
            "like" => {
                MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.toggle_like(&track.id, cx));
            }
            "edit" => {
                self.state.update(cx, |state, cx| {
                    state.show_edit_local_track = true;
                    cx.notify();
                });
            }
            "remove_from_playlist" => {
                let Some(playlist_id) = self.listening_playlist_id(cx) else {
                    return;
                };
                MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.remove_track_from_playlist(&playlist_id, &track.id, None, None, cx);
                });
            }
            "add_to_playlist" => {
                self.state.update(cx, |state, cx| {
                    state.show_add_to_playlist = true;
                    cx.notify();
                });
            }
            "share" => {
                let playlist_id = self.listening_playlist_id(cx);
                let track = track.clone();
                ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_track(track, playlist_id, cx));
            }
            "add_to_profile" => {
                let track = track.clone();
                MusicInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.add_track_to_profile(track, cx));
            }
            "artist" => self.push(RootRoute::MusicArtistProfile { artist_id: track.artist_id.clone() }, cx),
            "album" => self.push(RootRoute::MusicAlbumDetail { album_id: track.album_id.clone() }, cx),
            "report" => {
                let target = ReportTarget::MusicTrack(track.id.clone());
                ReportSheetStore::shared(cx).update(cx, |store, cx| store.present(target, None, None, cx));
            }
            _ => {}
        }
    }
}

impl RenderOnce for NowPlaying {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let player = observed_music_player(child_id(&self.id, "player"), window, cx);
        let is_playing = player.read(cx).is_playing;
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, move |_, cx| {
            NowPlayingState::new(is_playing, cx)
        });
        let parts = NowPlayingParts {
            id: self.id.clone(),
            player: player.clone(),
            state: state.clone(),
            nav_store: env::<NavigationStoreKey>(cx),
            is_aside: false,
        };
        let is_shared = cx
            .try_global::<NowPlayingSharedState>()
            .and_then(|shared| shared.0.upgrade())
            .is_some_and(|shared| shared == state);
        if !is_shared {
            cx.set_global(NowPlayingSharedState(state.downgrade()));
        }

        let has_track = player.read(cx).current_track.is_some();
        let has_aside = now_playing_aside_hooks().is_some_and(|hooks| (hooks.has_frontmost)(cx));
        on_change(child_id(&self.id, "aside-default"), (has_track, has_aside), true, window, cx, |_, value, _, cx| {
            let Some(hooks) = now_playing_aside_hooks() else {
                return;
            };
            if value.0 {
                (hooks.set_default)(AsideSection::Music, RootRoute::MusicNowPlaying, cx);
            } else {
                (hooks.clear_default)(AsideSection::Music, cx);
            }
        });

        let scaled_state = state.clone();
        on_change(child_id(&self.id, "is-playing"), is_playing, false, window, cx, move |_, is_playing, _, cx| {
            let target = NowPlaying::artwork_scale(*is_playing);
            scaled_state.update(cx, |state, cx| {
                state.artwork_scale.set_in(target, spring(0.62, 0.88), RiseClock::monotonic(), cx);
                cx.notify();
            });
        });

        let (show_queue_sheet, show_audio_effects_sheet, has_pending_transition, background) = {
            let snapshot = player.read(cx);
            (
                snapshot.show_queue_sheet,
                snapshot.show_audio_effects_sheet(),
                snapshot.pending_listening_session_transition.is_some(),
                rise_ui::theme(cx).bg._100,
            )
        };
        let panel_fits = NowPlaying::queue_panel_fits(window, cx);
        let is_panel_visible = panel_fits && player.read(cx).show_now_playing() && state.read(cx).is_queue_panel_open;
        let moved_player = player.clone();
        let moved_state = state.clone();
        on_change(child_id(&self.id, "queue-request"), show_queue_sheet, false, window, cx, move |_, is_requested, _, cx| {
            if !*is_requested || !panel_fits || !moved_player.read(cx).show_now_playing() {
                return;
            }
            moved_state.update(cx, |state, cx| state.set_queue_panel_open(true, cx));
            moved_player.update(cx, |player, cx| {
                player.show_queue_sheet = false;
                cx.notify();
            });
        });

        let transition_player = player.clone();
        on_change(
            child_id(&self.id, "pending-transition"),
            has_pending_transition,
            true,
            window,
            cx,
            move |_, has_pending, window, cx| {
                if *has_pending {
                    NowPlaying::present_pending_transition_alert(transition_player, window, cx);
                }
            },
        );

        let body_parts = parts.clone();
        let full_page = BottomSheetUi::new(
            child_id(&self.id, "full-page"),
            Binding::entity(player.clone(), |player| player.show_now_playing(), |player, value, cx| {
                player.set_show_now_playing(value, cx);
            }),
            move |window, cx| body_parts.sheet_body(window, cx).occludes_bottom_chrome(),
            |_, _| Empty,
        )
        .snap_fractions(vec![1.])
        .full_page(true)
        .content_is_edge_to_edge(true)
        .full_page_hides_footer(true)
        .footer_shows_top_border(false)
        .content_scrollable(true)
        .full_page_covers_full_screen(true)
        .sheet_background(Some(black()))
        .footer_background(Some(transparent_black()));

        let queue_id = child_id(&self.id, "queue-sheet");
        let queue_state = state.clone();
        let refused_player = player.clone();
        let queue_sheet = BottomSheetUi::new(
            child_id(&self.id, "queue"),
            Binding::entity(player.clone(), |player| player.show_queue_sheet, |player, value, _| {
                player.show_queue_sheet = value;
            }),
            move |_, _| {
                let measured = queue_state.clone();
                MusicQueueSheet::new(queue_id.clone()).on_minimum_height(move |height, window, cx| {
                    NowPlaying::update_queue_sheet_lowest_fraction(&measured, height, window, cx);
                })
            },
            |_, _| Empty,
        )
        .snap_fractions(NowPlaying::queue_sheet_snap_fractions(state.read(cx).queue_sheet_lowest_fraction))
        .content_is_edge_to_edge(true)
        .footer_shows_top_border(false)
        .sheet_background(Some(black()))
        .footer_background(Some(black()))
        .on_presentation_refused(move |_, cx| {
            refused_player.update(cx, |player, cx| {
                player.show_queue_sheet = false;
                cx.notify();
            });
        });

        let add_player = player.clone();
        let add_id = child_id(&self.id, "add-to-playlist-content");
        let add_state = state.clone();
        let add_sheet = BottomSheetUi::new(
            child_id(&self.id, "add-to-playlist"),
            Binding::entity(state.clone(), |state| state.show_add_to_playlist, |state, value, _| {
                state.show_add_to_playlist = value;
            }),
            move |_, cx| {
                let Some(track) = add_player.read(cx).current_track.clone() else {
                    return div().into_any_element();
                };
                let dismissed = add_state.clone();
                AddToPlaylistSheet::new(add_id.clone(), track)
                    .on_dismiss(move |_, cx| {
                        dismissed.update(cx, |state, cx| {
                            state.show_add_to_playlist = false;
                            cx.notify();
                        });
                    })
                    .into_any_element()
            },
            |_, _| Empty,
        )
        .snap_fractions(QUEUE_SHEET_FRACTIONS.to_vec())
        .footer_shows_top_border(false)
        .fits_content(true)
        .sheet_background(Some(background));

        let edit_player = player.clone();
        let edit_id = child_id(&self.id, "edit-local-track-content");
        let edit_state = state.clone();
        let edit_sheet = BottomSheetUi::new(
            child_id(&self.id, "edit-local-track"),
            Binding::entity(state.clone(), |state| state.show_edit_local_track, |state, value, _| {
                state.show_edit_local_track = value;
            }),
            move |window, cx| {
                let Some(track) = edit_player.read(cx).current_track.clone() else {
                    return div().into_any_element();
                };
                let dismissed = edit_state.clone();
                window
                    .use_keyed_state(edit_id.clone(), cx, move |_, cx| {
                        EditLocalTrackSheet::new(
                            track,
                            move |cx: &mut App| {
                                dismissed.update(cx, |state, cx| {
                                    state.show_edit_local_track = false;
                                    cx.notify();
                                });
                            },
                            cx,
                        )
                    })
                    .into_any_element()
            },
            |_, _| Empty,
        )
        .snap_fractions(vec![0.62, 0.86])
        .dismisses_keyboard_before_backdrop_dismiss(true)
        .footer_shows_top_border(false)
        .fits_content(true)
        .sheet_background(Some(background));

        let cover = WindowCoverFrame::resolve(window, cx);
        let full_page = anchored()
            .position(cover.origin)
            .child(div().relative().w(cover.size.width).h(cover.size.height).child(full_page));
        div()
            .absolute()
            .inset_0()
            .child(MusicPlaybackFailureNotice { id: child_id(&self.id, "failure-notice") })
            .child(full_page)
            .child(queue_sheet)
            .child(add_sheet)
            .child(edit_sheet)
            .modifier(
                MusicListeningControlAlert::new(child_id(&self.id, "control-alert"))
                    .is_active(!show_queue_sheet && !is_panel_visible && !show_audio_effects_sheet),
            )
    }
}
