use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{
    AnyElement, App, Bounds, BoxShadow, Context, ElementId, Entity, FontWeight, Hsla, IntoElement, MouseButton,
    MouseDownEvent, ObjectFit, Pixels, Point, RenderOnce, SharedString, Size, StyledImage, Subscription, Task, Window, black, img,
    point, px, size, svg, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_platform::haptics::{ImpactStyle, impact, selection_changed};
use rise_ui::expandable_text_ui::{EdgeInsets, ExpandableTextHashtags, ExpandableTextInlineLinks, ExpandableTextUi};
use rise_ui::liquid_chrome::{LiquidChromeModifiers, LiquidGlassContrast};
use rise_ui::loader_ui::{LoaderUi, LoaderUiSize, LoaderUiType};
use rise_ui::official_badge::OfficialBadge;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::font::Font;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::motion::{MotionSpec, ease_in_out, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Presence, Transition, UnitPoint};
use rise_ui::{IconUi, MainText, TextAlignT};

use crate::app::router::in_app_link_browser::InAppLinkBrowser;
use crate::app::router::main_tab_chrome::{MainTabPageHost, MainTabPageHostKey};
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::canon_identity::{RiseAuthorIdentity, RiseIdentity};
use crate::core::media::rise_media_foreground_gate::RiseMediaForegroundGate;
use crate::core::shared_element_transition::kernel::shared_element_types::SharedElementPart;
use crate::core::shared_element_transition::runtime::shared_element_lift_board::SharedElementLiftModifiers;
use crate::core::shared_element_transition::renderer::shared_element_video_frame_grabber::SharedElementPlayerLayerView;
use crate::core::ui::clever_image_ui::{CleverImageUi, ContentMode};
use crate::core::ui::media_link_areas_overlay::{MediaLinkAreasOverlay, MediaLinkZone};
use crate::core::ui::collaborator_avatars_ui::{CollaboratorAvatar, CollaboratorAvatarsUi, CollaboratorNamesLineUi};
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::analytics::stores::engagement::engagement_interactions::engagement_interactions::EngagementInteractionsStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::comment::stores::comment_services::comments_sheet_motion_relay::CommentsSheetMotionRelay;
use crate::modules::comment::widgets::comments_sheet::CommentsSheet;
use crate::modules::comment::widgets::comments_sheet_progress_fade::CommentsSheetProgressFade;
use crate::modules::media_picker::editor::editor_image_layout::EditorImageLayout;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::music::widgets::music_marquee_title::MusicMarqueeTitle;
use crate::modules::post::components::collaborators::post_collaborator_header_ui::{
    PostCoauthorInviteBanner, PostCollaboratorHeaderPresentation,
};
use crate::modules::post::components::collaborators::post_collaborators_presenter::PostCollaboratorsPresenter;
use crate::modules::post::components::collaborators::post_tag_bubbles_overlay::{
    PostMediaTagsState, PostTagBubblesLayer, PostTagsChip,
};
use crate::modules::post::components::post::post::Post;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::components::post::post_music_plate_overlay::PostMusicPlateOverlay;
use crate::modules::post::components::post::post_music_playback_controller::PostMusicPlaybackController;
use crate::modules::post::components::post_visibility_reader::PostVisibilityReader;
use crate::modules::post::components::repost::repost_overlay::{RepostOverlay, RepostStickerStyle};
use crate::modules::post::engine::rise_post_engine_models::{RisePostSearchKind, RisePostViewSurface};
use crate::modules::post::shared::post_media_slides::PostMediaSlide;
use crate::modules::post::stores::post_actions::post_types::{
    GetPostFeedResponse, PostCountVisibility, PostMediaSizeAspectRatio,
};
use crate::modules::post::stores::post_interactions::post_interactions::PostInteractionsStore;
use crate::modules::shorts::components::shorts_audio_cover_button::ShortsAudioCoverButton;
use crate::modules::shorts::components::shorts_comments_compact_layout::ShortsCommentsCompactLayoutGeometry;
use crate::modules::shorts::components::shorts_autoscroll::{ShortsAutoscroll, ShortsAutoscrollPreferences};
use crate::modules::shorts::components::shorts_feed_player::{ShortsFeedPlayer, ShortsPlaybackController, ShortsPlaybackEnded};
use crate::modules::shorts::components::shorts_follow_button::ShortsFollowButton;
use crate::modules::shorts::components::shorts_hold_recognizer::ShortsHoldRecognizerHost;
use crate::modules::shorts::components::shorts_more_sheet::ShortsMoreSheetStore;
use crate::modules::shorts::components::shorts_pinch_zoom::ShortsPinchZoomLayer;
use crate::modules::shorts::components::shorts_riff_pager::{RiffPageIndicator, ShortsRiffPager};
use crate::modules::shorts::components::shorts_scrubber::{ShortsScrubPillLevel, ShortsScrubTimePill, ShortsScrubber};
use crate::modules::shorts::stores::shorts::shorts_interactions::shorts_interactions::ShortsInteractionsStore;
use crate::modules::shorts::pages::short_viewer::short_viewer_shared_element_destination::ShortViewerSharedElementDestination;
use crate::modules::sparks::components::sparks_icon_view::SparksBurstIcon;
use crate::modules::sparks::stores::sparks::sparks_actions::sparks_types::SparksTarget;
use crate::modules::sparks::stores::sparks::sparks_interactions::sparks_interactions::SparksInteractionsStore;

fn scoped_id(part: &str, post_id: &str) -> ElementId {
    ElementId::Name(format!("shorts-feed-cell-{part}-{post_id}").into())
}

fn nav_push(nav_store: &Option<Entity<NavigationStore>>, route: RootRoute, cx: &mut App) {
    if let Some(nav_store) = nav_store {
        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
    }
}

fn chrome_shadow() -> Vec<BoxShadow> {
    vec![BoxShadow {
        color: black().opacity(0.28),
        offset: point(px(0.0), px(2.0)),
        blur_radius: px(6.0),
        spread_radius: px(0.0),
        inset: false,
    }]
}

const RAIL_SHADOW_TAPS: [(f32, f32); 9] = [
    (0.0, 1.0),
    (-1.5, 1.0),
    (1.5, 1.0),
    (0.0, -0.5),
    (0.0, 2.5),
    (-1.0, 0.0),
    (1.0, 0.0),
    (-1.0, 2.0),
    (1.0, 2.0),
];
const RAIL_SHADOW_TAP_OPACITY: f32 = 0.16;

fn shadowed_glyph(symbol: &str, side: f32, color: Hsla) -> Div {
    let is_filled = symbol.ends_with(".fill");
    let shadow_color = black().opacity(RAIL_SHADOW_TAP_OPACITY);
    div()
        .relative()
        .flex_none()
        .size(px(side))
        .children(RAIL_SHADOW_TAPS.iter().filter_map(|&(dx, dy)| {
            IconUi::sized(symbol, px(side), shadow_color, is_filled)
                .map(|glyph| glyph.absolute().left(px(dx)).top(px(dy)))
        }))
        .children(IconUi::sized(symbol, px(side), color, is_filled).map(|glyph| glyph.absolute().left_0().top_0()))
}

fn shadowed_text(label: &str, text_size: f32, monospaced: bool, scales_down: bool) -> Div {
    let text = |color: Hsla| {
        let text = MainText::new(label.to_owned())
            .px(text_size)
            .font_weight(FontWeight::SEMIBOLD)
            .color(color)
            .monospaced_digits(monospaced)
            .number_of_lines(1);
        if scales_down { text.minimum_scale_factor(0.8) } else { text }
    };
    div()
        .relative()
        .children(RAIL_SHADOW_TAPS.iter().map(|&(dx, dy)| {
            div().absolute().left(px(dx)).top(px(dy)).child(text(black().opacity(RAIL_SHADOW_TAP_OPACITY)))
        }))
        .child(text(white()))
}

/// own hit box so a press on a control doesnt also reach the cell pause tap,
/// gpui runs click of every hovered ancestor
fn blocks_cell_tap(content: impl IntoElement) -> Div {
    div().flex().min_w_0().occlude().child(content)
}

/// swipe back up to a riff continues where it was the first time, later returns start it over
/// lives as long as the feed screen
#[derive(Clone, Default)]
pub struct ShortsReturnRewind {
    upward_returns: HashMap<String, i64>,
    left: HashSet<String>,
    tokens: HashMap<String, i64>,
}

impl ShortsReturnRewind {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn token(&self, post_id: &str) -> i64 {
        self.tokens.get(post_id).copied().unwrap_or(0)
    }

    pub fn forget_returns(&mut self) {
        self.upward_returns.clear();
        self.left.clear();
    }

    pub fn move_to(&mut self, old_id: Option<&str>, new_id: &str, order: &[String]) {
        let Some(old_id) = old_id.filter(|old_id| *old_id != new_id) else {
            return;
        };
        let (Some(old_index), Some(new_index)) =
            (order.iter().position(|id| id == old_id), order.iter().position(|id| id == new_id))
        else {
            return;
        };
        if new_index < old_index {
            if self.left.contains(new_id) {
                *self.upward_returns.entry(new_id.to_owned()).or_insert(0) += 1;
            }
        } else if self.upward_returns.get(old_id).copied().unwrap_or(0) > 0 {
            *self.tokens.entry(old_id.to_owned()).or_insert(0) += 1;
        }
        self.left.insert(old_id.to_owned());
    }
}

pub struct ShortsFeedCellState {
    is_muted: bool,
    is_paused: bool,
    paused_presence: Presence<()>,
    like_bump_scale: Animated<f32>,
    like_bump_settle: Option<Task<()>>,
    playback: Entity<ShortsPlaybackController>,
    tags_state: Entity<PostMediaTagsState>,
    riff_page: Option<i64>,
    is_pinching: bool,
    is_holding: bool,
    tap_suppressed_until: Duration,
    drawn_aspects: HashMap<String, f32>,
    chrome_opacity: Animated<f32>,
    cell_size: Size<Pixels>,
    cell_origin: Point<Pixels>,
    shown_post: Option<Rc<GetPostFeedResponse>>,
    was_active: bool,
    tab_host_observer: Option<Subscription>,
    scrubber: Entity<ShortsScrubber>,
    scrub_pill: Entity<ShortsScrubTimePill>,
    player_layer_view: Option<SharedElementPlayerLayerView>,
    _playback_layer: Subscription,
    _autoscroll: [Subscription; 3],
}

impl ShortsFeedCellState {
    fn new(cx: &mut Context<Self>) -> Self {
        let playback = cx.new(|_| ShortsPlaybackController::new());
        let scrubber = cx.new(|cx| ShortsScrubber::new(playback.clone(), cx));
        let scrub_pill = cx.new(|cx| ShortsScrubTimePill::new(playback.clone(), cx));
        let preferences = ShortsAutoscrollPreferences::shared(cx);
        let more_sheet = ShortsMoreSheetStore::shared(cx);
        let autoscroll = [
            cx.subscribe(&playback, |_, _, _: &ShortsPlaybackEnded, cx| cx.notify()),
            cx.observe(&preferences, |this: &mut Self, _, cx| {
                if this.was_active {
                    cx.notify();
                }
            }),
            cx.observe(&more_sheet, |this: &mut Self, _, cx| {
                if this.was_active {
                    cx.notify();
                }
            }),
        ];
        let playback_layer = cx.observe(&playback, |this, playback, cx| {
            let layer = playback.read(cx).player_layer_view();
            let same = match (&this.player_layer_view, &layer) {
                (Some(shown), Some(next)) => shown.same_layer(next),
                (None, None) => true,
                _ => false,
            };
            if !same {
                this.player_layer_view = layer;
                cx.notify();
            }
        });
        Self {
            is_muted: false,
            is_paused: false,
            paused_presence: Presence::new(None)
                .with_transition(Transition::scale(0.85).combined(Transition::opacity())),
            like_bump_scale: Animated::new(1.0),
            like_bump_settle: None,
            playback,
            tags_state: cx.new(PostMediaTagsState::new),
            riff_page: Some(0),
            is_pinching: false,
            is_holding: false,
            tap_suppressed_until: Duration::ZERO,
            drawn_aspects: HashMap::new(),
            chrome_opacity: Animated::new(1.0),
            cell_size: Size::default(),
            cell_origin: Point::default(),
            shown_post: None,
            was_active: false,
            tab_host_observer: None,
            scrubber,
            scrub_pill,
            player_layer_view: None,
            _playback_layer: playback_layer,
            _autoscroll: autoscroll,
        }
    }

    fn observe_tab_host(&mut self, host: MainTabPageHost, cx: &mut Context<Self>) {
        if self.tab_host_observer.is_some() {
            return;
        }
        let shown = host.is_shown(cx);
        self.playback.update(cx, |playback, cx| playback.set_held(!shown, cx));
        let selection = host.selection.clone();
        self.tab_host_observer = Some(cx.observe(&selection, move |this, _, cx| {
            let shown = host.is_shown(cx);
            this.playback.update(cx, |playback, cx| playback.set_held(!shown, cx));
            let Some(post) = this.shown_post.clone() else {
                return;
            };
            let is_active = shown && this.was_active;
            let state = cx.entity();
            cx.defer(move |cx| ShortsFeedCell::sync_slide_music(&post, is_active, &state, cx));
        }));
    }

    /// toggles pause with a short ease animation
    fn toggle_paused(&mut self, cx: &mut Context<Self>) {
        self.is_paused = !self.is_paused;
        let value = self.is_paused.then_some(());
        self.paused_presence.set_in(value, ease_in_out(0.18), RiseClock::monotonic(), cx);
        cx.notify();
    }

    /// unpauses with no animation so the controls leave at once
    fn clear_paused(&mut self, cx: &mut Context<Self>) {
        self.is_paused = false;
        self.paused_presence.set_in(None, None::<MotionSpec>, RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn toggle_muted(&mut self, cx: &mut Context<Self>) {
        self.is_muted = !self.is_muted;
        cx.notify();
    }

    fn hides_chrome(&self) -> bool {
        self.is_pinching || self.is_holding
    }

    fn animate_chrome(&mut self, spec: MotionSpec, cx: &mut Context<Self>) {
        let target = if self.hides_chrome() { 0.0 } else { 1.0 };
        self.chrome_opacity.set_in(target, spec, RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn set_pinching(&mut self, active: bool, cx: &mut Context<Self>) {
        self.is_pinching = active;
        self.animate_chrome(ease_in_out(0.16), cx);
    }

    fn set_holding(&mut self, holding: bool, cx: &mut Context<Self>) {
        if holding {
            self.tap_suppressed_until = Duration::MAX;
        } else {
            if !self.is_holding {
                return;
            }
            self.tap_suppressed_until = RiseClock::monotonic() + Duration::from_secs_f64(0.3);
        }
        self.is_holding = holding;
        self.animate_chrome(ease_in_out(0.2), cx);
    }

    fn accepts_tap(&self) -> bool {
        RiseClock::monotonic() >= self.tap_suppressed_until
    }

    fn adopt_drawn_aspect(&mut self, url: &str, aspect: f32, cx: &mut Context<Self>) {
        if !aspect.is_finite() || aspect <= 0.0 || self.drawn_aspects.get(url) == Some(&aspect) {
            return;
        }
        self.drawn_aspects.insert(url.to_owned(), aspect);
        cx.notify();
    }

    fn bump_like(&mut self, cx: &mut Context<Self>) {
        self.like_bump_scale.set_in(1.18, spring(0.22, 0.62), RiseClock::monotonic(), cx);
        self.like_bump_settle = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs_f64(0.14)).await;
            let _ = this.update(cx, |this: &mut Self, cx: &mut Context<Self>| {
                this.like_bump_scale.set_in(1.0, spring(0.26, 0.78), RiseClock::monotonic(), cx);
                cx.notify();
            });
        }));
        cx.notify();
    }
}

#[derive(IntoElement)]
pub struct ShortsFeedCell {
    post: Rc<GetPostFeedResponse>,
    is_active: bool,
    bottom_inset: f32,
    progress_floor: Option<f32>,
    view_surface: RisePostViewSurface,
    shared_element_destination: Option<Rc<ShortViewerSharedElementDestination>>,
    rewind_token: i64,
    on_auto_advance: Option<ShortsAutoAdvance>,
}

/// moves the riffs feed to the next riff when auto scroll is done with this one
pub type ShortsAutoAdvance = Rc<dyn Fn(&mut App)>;

impl ShortsFeedCell {
    const PAGE_INDICATOR_HEADER_CLEARANCE: f32 = 62.0;
    const HOLD_DELAY: f64 = 0.22;
    const PROGRESS_SLOT: f32 = 36.0;
    const PROGRESS_HEIGHT: f32 = 22.0;
    const CAPTION_FLOOR_GAP: f32 = 12.0;

    pub fn new(post: impl Into<Rc<GetPostFeedResponse>>, is_active: bool, bottom_inset: f32) -> Self {
        Self {
            post: post.into(),
            is_active,
            bottom_inset,
            progress_floor: None,
            view_surface: RisePostViewSurface::Shorts,
            shared_element_destination: None,
            rewind_token: 0,
            on_auto_advance: None,
        }
    }

    pub fn on_auto_advance(mut self, on_auto_advance: Option<ShortsAutoAdvance>) -> Self {
        self.on_auto_advance = on_auto_advance;
        self
    }

    /// next carousel page, or the next riff after the last page
    fn advance_autoscroll(
        post: &GetPostFeedResponse,
        state: &Entity<ShortsFeedCellState>,
        on_auto_advance: Option<&ShortsAutoAdvance>,
        cx: &mut App,
    ) {
        let media = ShortsCellMedia::new(post, state.read(cx).riff_page);
        if media.uses_pager && media.page_index + 1 < media.slides.len() as i64 {
            let next = media.page_index + 1;
            state.update(cx, |state, cx| {
                state.riff_page = Some(next);
                cx.notify();
            });
        } else if let Some(on_auto_advance) = on_auto_advance {
            on_auto_advance(cx);
        }
    }

    pub fn rewind_token(mut self, rewind_token: i64) -> Self {
        self.rewind_token = rewind_token;
        self
    }

    pub fn progress_floor(mut self, progress_floor: Option<f32>) -> Self {
        self.progress_floor = progress_floor;
        self
    }

    pub fn view_surface(mut self, view_surface: RisePostViewSurface) -> Self {
        self.view_surface = view_surface;
        self
    }

    pub fn shared_element_destination(mut self, destination: Option<Rc<ShortViewerSharedElementDestination>>) -> Self {
        self.shared_element_destination = destination;
        self
    }

    /// photo pages track plays under its own owner key, so the video page player leaving doesnt stop it
    fn slide_music_key(post: &GetPostFeedResponse) -> String {
        format!("{}#slides", post.id)
    }

    fn display_tag(post: &GetPostFeedResponse) -> Option<String> {
        let tag = trimming_whitespaces_and_newlines(&post.author.tag);
        (!tag.is_empty()).then(|| tag.to_owned())
    }

    fn post_author(post: &GetPostFeedResponse) -> RiseAuthorIdentity {
        RiseIdentity::author(Some(post.author_id.as_str()), None, post.author.id.as_deref())
    }

    /// compares with the resolved viewer id not the profile id,
    /// profile survives an account switch and claimed other reels as mine
    fn authored_by_me(post: &GetPostFeedResponse, viewer: Option<&str>) -> bool {
        RiseIdentity::is_own(&Self::post_author(post), viewer, None)
    }

    fn avatar_source(post: &GetPostFeedResponse, authored_by_me: bool, cx: &mut App) -> String {
        let logo = trimming_whitespaces_and_newlines(&post.author.more.logo);
        if !logo.is_empty() {
            return logo.to_owned();
        }
        if authored_by_me {
            let auth = AuthServicesStore::shared(cx);
            return auth
                .read(cx)
                .profile()
                .map(|me| me.avatar_url.clone().unwrap_or_else(|| me.more.logo.clone()))
                .unwrap_or_default();
        }
        logo.to_owned()
    }

    fn has_sound_control(post: &GetPostFeedResponse, media: &ShortsCellMedia) -> bool {
        if post.shows_upload_overlay() {
            return false;
        }
        media.shows_video() || (media.uses_pager && post.music_audio_url().is_some())
    }

    /// photo pages loop the attached track, video page leaves sound to its player
    fn sync_slide_music(post: &GetPostFeedResponse, is_active: bool, state: &Entity<ShortsFeedCellState>, cx: &mut App) {
        let state = state.read(cx);
        let media = ShortsCellMedia::new(post, state.riff_page);
        if !media.uses_pager {
            return;
        }
        let is_muted = state.is_muted;
        let key = Self::slide_music_key(post);
        let is_foreground = RiseMediaForegroundGate::shared(cx).read(cx).is_foreground();
        let url = (is_active && !post.shows_upload_overlay() && !is_muted && !media.shows_video() && is_foreground)
            .then(|| post.music_audio_url())
            .flatten();
        let controller = PostMusicPlaybackController::shared(cx);
        controller.update(cx, |controller, cx| match url {
            Some(url) => {
                let _ = controller.play_ambient(
                    &key,
                    &url,
                    post.track_id.as_deref(),
                    post.music_segment_start_ms(),
                    post.music_segment_end_ms(),
                    true,
                    true,
                    false,
                    cx,
                );
            }
            None => controller.stop(&key, cx),
        });
    }

    /// tags address a slide by its position among all media, so the page index works on the video page too
    fn sync_slide_tags(post: &GetPostFeedResponse, state: &Entity<ShortsFeedCellState>, cx: &mut App) {
        let state = state.read(cx);
        let media = ShortsCellMedia::new(post, state.riff_page);
        if !media.uses_pager {
            return;
        }
        let tags_state = state.tags_state.clone();
        tags_state.update(cx, |tags_state, cx| tags_state.set_slide(media.page_index, cx));
    }

    /// writes the slide an exit flight takes into the viewer destination
    fn report_slide(
        post: &GetPostFeedResponse,
        destination: Option<&ShortViewerSharedElementDestination>,
        state: &Entity<ShortsFeedCellState>,
        cx: &App,
    ) {
        let Some(destination) = destination else {
            return;
        };
        let media = ShortsCellMedia::new(post, state.read(cx).riff_page);
        destination.update_slide(&post.id, media.page_index);
    }

    fn toggle_pause(post: &GetPostFeedResponse, state: &Entity<ShortsFeedCellState>, cx: &mut App) {
        let media = ShortsCellMedia::new(post, state.read(cx).riff_page);
        if post.shows_upload_overlay() || !media.shows_video() {
            return;
        }
        state.update(cx, |state, cx| state.toggle_paused(cx));
    }

    fn is_comments_engaged(post: &GetPostFeedResponse, is_active: bool, cx: &mut App) -> bool {
        let relay = CommentsSheetMotionRelay::shared(cx);
        is_active && relay.read(cx).engaged_post_id() == Some(post.id.as_str())
    }

    fn open_coauthors_sheet(post_id: &str, cx: &mut App) {
        PostCollaboratorsPresenter::shared(cx).update(cx, |presenter, cx| {
            presenter.present_coauthors(post_id, false, cx);
        });
    }

    fn open_comments_from_caption(post: &GetPostFeedResponse, cx: &mut App) {
        if post.shows_upload_overlay() {
            return;
        }
        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.open_comments(post, true, cx));
    }

    fn open_author_profile(
        post: &GetPostFeedResponse,
        nav_store: &Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) {
        let viewer = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        if Self::authored_by_me(post, viewer.as_deref()) {
            nav_push(nav_store, RootRoute::MyProfile, cx);
            return;
        }
        let author = Self::post_author(post);
        let Some(resolved_id) = author.wire_id() else {
            return;
        };
        let route = RootRoute::UserPage { user_id: Some(resolved_id.to_owned()), tag: Self::display_tag(post) };
        nav_push(nav_store, route, cx);
        let target = post.rise_view_target_post_id();
        EngagementInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.profile_view(resolved_id, Some(target.as_str()), cx));
    }

    fn handle_like(post: &GetPostFeedResponse, state: &Entity<ShortsFeedCellState>, cx: &mut App) {
        if post.shows_upload_overlay() {
            return;
        }
        let _ = selection_changed();
        state.update(cx, |state, cx| state.bump_like(cx));
        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_like(post, cx));
    }

    fn hold_like(post: &GetPostFeedResponse, state: &Entity<ShortsFeedCellState>, cx: &mut App) {
        if post.shows_upload_overlay() {
            return;
        }
        let viewer = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        if post.likes_count <= 0 || !post.shows_count(|visibility| visibility.likes, viewer.as_deref()) {
            Self::handle_like(post, state, cx);
            return;
        }
        let _ = impact(ImpactStyle::Light);
        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.open_post_likes(post, cx));
    }

    fn apply_holding(
        post: &GetPostFeedResponse,
        is_active: bool,
        state: &Entity<ShortsFeedCellState>,
        holding: bool,
        cx: &mut App,
    ) {
        if holding && (!is_active || post.shows_upload_overlay() || Self::is_comments_engaged(post, is_active, cx)) {
            return;
        }
        state.update(cx, |state, cx| state.set_holding(holding, cx));
    }

    /// counter label under a rail button, nothing when the author hid it or its zero
    fn rail_count(
        post: &GetPostFeedResponse,
        value: i64,
        shown: fn(&PostCountVisibility) -> bool,
        viewer: Option<&str>,
    ) -> Option<String> {
        if value <= 0 || !post.shows_count(shown, viewer) {
            return None;
        }
        Some(RiseFormat::compact_count(value))
    }

    fn mute_button(
        post_id: &str,
        slot: &str,
        state: &Entity<ShortsFeedCellState>,
        is_muted: bool,
        is_enabled: bool,
    ) -> AnyElement {
        let state = state.clone();
        let symbol = if is_muted { "speaker.slash.fill" } else { "speaker.wave.2.fill" };
        let accessibility_label = tr(if is_muted { "feed_sound_unmute" } else { "feed_sound_mute" });
        div()
            .id(scoped_id(&format!("{slot}-label"), post_id))
            .aria_label(accessibility_label.clone())
            .tooltip_label(accessibility_label)
            .flex_none()
            .size(px(42.0))
            .circle()
            .shadow(chrome_shadow())
            .occlude()
            .child(
                plain(scoped_id(slot, post_id))
                    .size(px(42.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .disabled(!is_enabled)
                    .on_press(move |_, cx| state.update(cx, |state, cx| state.toggle_muted(cx)))
                    .children(IconUi::sized(symbol, px(15.0), white(), true))
                    .liquid_chrome_circle(None)
                    .liquid_glass_contrast(LiquidGlassContrast::Media),
            )
            .into_any_element()
    }

    fn paused_controls(post: &GetPostFeedResponse, state: &Entity<ShortsFeedCellState>, is_muted: bool) -> Div {
        let toggle_post = post.clone();
        let toggle_state = state.clone();
        vstack(12.0)
            .child(
                div().flex_none().size(px(58.0)).circle().shadow(chrome_shadow()).occlude().child(
                    plain(scoped_id("paused-play", &post.id))
                        .size(px(58.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .on_press(move |_, cx| Self::toggle_pause(&toggle_post, &toggle_state, cx))
                        .children(IconUi::sized("play.fill", px(24.0), white(), true))
                        .liquid_chrome_circle(None)
                        .liquid_glass_contrast(LiquidGlassContrast::Media),
                ),
            )
            .child(Self::mute_button(&post.id, "paused-mute", state, is_muted, true))
    }

    #[allow(clippy::too_many_arguments)]
    fn media_layer(
        post: &Rc<GetPostFeedResponse>,
        media: &ShortsCellMedia,
        cell_size: Size<Pixels>,
        is_active: bool,
        is_own_post: bool,
        engaged: bool,
        holds_video_at_end: bool,
        corner_radius: f32,
        state: &Entity<ShortsFeedCellState>,
        cx: &App,
    ) -> AnyElement {
        let is_uploading = post.shows_upload_overlay();
        let snapshot = state.read(cx);
        let (is_muted, is_paused) = (snapshot.is_muted, snapshot.is_paused || snapshot.is_holding);
        let drawn = Self::drawn_frame(media, &snapshot.drawn_aspects, cell_size);
        let playback = snapshot.playback.clone();

        let content: AnyElement = if is_uploading {
            ShortsPendingLoaderOverlay { post: post.clone() }.into_any_element()
        } else if media.uses_pager {
            let page_state = state.clone();
            let page = Binding::entity(
                page_state,
                |state: &ShortsFeedCellState| state.riff_page,
                |state: &mut ShortsFeedCellState, page, _| state.riff_page = page,
            );
            let tap_post = post.clone();
            let tap_state = state.clone();
            let aspect_state = state.clone();
            ShortsRiffPager::new(
                scoped_id("pager", &post.id),
                post.clone(),
                media.slides.clone(),
                cell_size,
                page,
                is_active,
                is_muted,
                is_paused,
                playback,
                corner_radius,
                !engaged,
                move |_: &mut Window, cx: &mut App| {
                    if !Self::is_comments_engaged(&tap_post, is_active, cx) {
                        Self::toggle_pause(&tap_post, &tap_state, cx);
                    }
                },
            )
            .holds_video_at_end(holds_video_at_end)
            .on_drawn_aspect(move |url: &str, aspect: f32, _: &mut Window, cx: &mut App| {
                aspect_state.update(cx, |state, cx| state.adopt_drawn_aspect(url, aspect, cx));
            })
            .into_any_element()
        } else if let Some(url) = media.single_video_url.clone() {
            let aspect_url = url.clone();
            let aspect_state = state.clone();
            ShortsFeedPlayer::new(scoped_id("player", &post.id), url, post.video_thumbnail.clone(), is_active && !is_uploading)
                .shows_thumbnail_while_loading(false)
                .post_id(Some(post.rise_view_target_post_id()))
                .author_id(Some(post.author.id.clone().unwrap_or_else(|| post.author_id.clone())))
                .is_muted(is_muted)
                .paused(is_paused)
                .playback(Some(playback))
                .holds_at_end(holds_video_at_end)
                .page_size(cell_size)
                .corner_radius(corner_radius)
                .on_aspect(move |aspect: f32, _: &mut Window, cx: &mut App| {
                    aspect_state.update(cx, |state, cx| state.adopt_drawn_aspect(&aspect_url, aspect, cx));
                })
                .music_overlay(post.shorts_music_overlay())
                .persists_post_media_to_disk(is_own_post)
                .post_image_cache_policy(PostMediaCachePolicy::image_policy(is_own_post))
                .into_any_element()
        } else {
            div().size_full().bg(black()).into_any_element()
        };

        let plate = (!is_uploading && post.track_plate.is_some()).then(|| {
            div().absolute().inset_0().child(
                PostMusicPlateOverlay::new(post.clone())
                    .media_aspect(drawn.map(|(aspect, _)| aspect))
                    .media_fills(drawn.is_none_or(|(_, fills)| fills)),
            )
        });
        let link_areas = drawn.filter(|_| !is_uploading && !post.link_areas.is_empty()).map(|(aspect, fills)| {
            let link_post_id = post.id.clone();
            div().absolute().inset_0().child(
                MediaLinkAreasOverlay::new(
                    scoped_id("link-areas", &post.id),
                    MediaLinkZone::zones_post(&post.link_areas, media.page_index),
                    Rc::new(move |zone: &MediaLinkZone, _, _: &mut Window, cx: &mut App| {
                        let Some(url) = Url::from_string(&zone.url) else {
                            return;
                        };
                        PostInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.record_link_open(&link_post_id, cx));
                        InAppLinkBrowser::route(&url, cx);
                    }),
                )
                .media_aspect(Some(aspect))
                .media_fills(fills),
            )
        });

        div()
            .relative()
            .w(cell_size.width)
            .h(cell_size.height)
            .child(div().absolute().inset_0().child(content))
            .children(plate)
            .children(link_areas)
            .into_any_element()
    }

    fn drawn_frame(
        media: &ShortsCellMedia,
        drawn_aspects: &HashMap<String, f32>,
        page: Size<Pixels>,
    ) -> Option<(f32, bool)> {
        let fill = |aspect: f32| ShortsFeedPlayer::fills(aspect, page);
        if let Some(url) = &media.single_video_url {
            return drawn_aspects.get(url).map(|&aspect| (aspect, fill(aspect)));
        }
        match media.current_slide()? {
            PostMediaSlide::Image { url, size, .. } => {
                let gravity_aspect = size
                    .as_ref()
                    .and_then(|size| size.aspect_ratio())
                    .map(|aspect| aspect as f32)
                    .or_else(|| drawn_aspects.get(url).copied())?;
                Some((drawn_aspects.get(url).copied().unwrap_or(gravity_aspect), fill(gravity_aspect)))
            }
            PostMediaSlide::Video { url, .. } => drawn_aspects.get(url).map(|&aspect| (aspect, fill(aspect))),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn chrome_layer(
        post: &Rc<GetPostFeedResponse>,
        media: &ShortsCellMedia,
        cell_size: Size<Pixels>,
        safe_top: f32,
        bottom_padding: f32,
        progress_bottom_padding: f32,
        view_surface: RisePostViewSurface,
        state: &Entity<ShortsFeedCellState>,
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        let is_uploading = post.shows_upload_overlay();
        let snapshot = state.read(cx);
        let is_muted = snapshot.is_muted;
        let tags_state = snapshot.tags_state.clone();
        let scrubber = snapshot.scrubber.clone();
        let scrub_pill = snapshot.scrub_pill.clone();

        let gradients = pass_through()
            .absolute()
            .inset_0()
            .flex()
            .flex_col()
            .child(
                LinearGradient::colors([black().opacity(0.55), black().opacity(0.0)], UnitPoint::TOP, UnitPoint::BOTTOM)
                    .view()
                    .w_full()
                    .h(px(120.0)),
            )
            .child(spacer())
            .child(
                LinearGradient::colors([black().opacity(0.0), black().opacity(0.82)], UnitPoint::TOP, UnitPoint::BOTTOM)
                    .view()
                    .w_full()
                    .h(px(280.0)),
            );

        let indicator = (media.uses_pager && media.slides.len() > 1 && !is_uploading).then(|| {
            pass_through()
                .absolute()
                .inset_0()
                .pt(px(safe_top + Self::PAGE_INDICATOR_HEADER_CLEARANCE))
                .flex()
                .flex_col()
                .items_center()
                .child(RiffPageIndicator::new(media.slides.len(), media.page_index))
        });

        let now = RiseClock::monotonic();
        let paused = (media.shows_video() && !is_uploading)
            .then(|| {
                state.read(cx).paused_presence.render_transitioned(now, window, |_| {
                    Self::paused_controls(post, state, is_muted)
                })
            })
            .flatten()
            .map(|controls| {
                div().absolute().inset_0().flex().items_center().justify_center().child(controls)
            });

        let viewer = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire();
        let meta = Self::bottom_meta(post, &tags_state, &scrub_pill, viewer.as_deref(), is_uploading, cx);
        let rail_sound = (media.uses_pager && !media.shows_video() && post.music_audio_url().is_some() && !is_uploading)
            .then_some(is_muted);
        let rail = Self::action_rail(post, view_surface, state, viewer.as_deref(), rail_sound, window, cx);
        let shows_scrubber = media.shows_video() && !is_uploading;
        let scrubber = shows_scrubber.then(|| {
            pass_through()
                .absolute()
                .inset_0()
                .flex()
                .flex_col()
                .justify_end()
                .px(px(12.0))
                .pb(px(progress_bottom_padding))
                .child(div().relative().w_full().child(ShortsScrubber::element(scrubber)).child(
                    ShortsScrubTimePill::level_marker(
                        &scrub_pill,
                        ShortsScrubPillLevel::Scrubber,
                        scoped_id("scrub-level-bar", &post.id),
                    ),
                ))
        });
        let scrub_pill = shows_scrubber.then(|| ShortsScrubTimePill::element(scrub_pill, scoped_id("scrub-pill", &post.id)));

        div()
            .relative()
            .w(cell_size.width)
            .h(cell_size.height)
            .child(gradients)
            .children(indicator)
            .child(div().absolute().inset_0().child(PostTagBubblesLayer::new(post.clone(), tags_state)))
            .children(paused)
            .child(
                pass_through()
                    .absolute()
                    .inset_0()
                    .flex()
                    .flex_col()
                    .pb(px(bottom_padding))
                    .child(spacer_min_h(0.0))
                    .child(
                        hstack(12.0)
                            .align(VerticalAlignment::Bottom)
                            .px(px(14.0))
                            .child(meta)
                            .child(rail),
                    )
                    .when(media.contains_video(), |column| {
                        column.child(div().flex_none().w_full().h(px(Self::PROGRESS_SLOT)))
                    }),
            )
            .children(scrubber)
            .children(scrub_pill)
    }

    fn bottom_meta(
        post: &Rc<GetPostFeedResponse>,
        tags_state: &Entity<PostMediaTagsState>,
        scrub_pill: &Entity<ShortsScrubTimePill>,
        viewer: Option<&str>,
        is_uploading: bool,
        cx: &mut App,
    ) -> Div {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let display_tag = Self::display_tag(post);

        let banner = post.moderation_banner().map(|banner| {
            let tint = if banner.is_error() { theme.semantic.error_100 } else { theme.primary._100 };
            hstack(6.0)
                .px(px(10.0))
                .py(px(7.0))
                .bg(black().opacity(0.45))
                .rounded_rect(8.0)
                .children(IconUi::sized(banner.symbol_name(), px(12.0), tint, banner.symbol_name().ends_with(".fill")))
                .child(
                    MainText::new(tr(banner.text_key()))
                        .px(12.0)
                        .font_weight(FontWeight::MEDIUM)
                        .color(white())
                        .number_of_lines(2),
                )
        });

        let chips = (post.is_reposted || !post.tagged_users.is_empty()).then(|| {
            let is_own = PostMediaCachePolicy::is_own_post(post, cx);
            let repost = post.is_reposted.then(|| {
                div().flex_none().occlude().child(RepostOverlay::new(post.clone()).style(RepostStickerStyle::Feed))
            });
            hstack(10.0)
                .align(VerticalAlignment::Bottom)
                .flex_none()
                .pl(px(4.0))
                .pb(px(2.0))
                .children(repost)
                .child(blocks_cell_tap(PostTagsChip::new(post.clone(), tags_state.clone()).viewer_is_author(is_own)))
        });

        let author_row = Self::author_row(post, viewer, is_uploading, &nav_store, display_tag.as_deref(), cx);

        let invite = post.is_pending_coauthor_invite().then(|| {
            let open_post = post.clone();
            let open_nav = nav_store.clone();
            blocks_cell_tap(
                PostCoauthorInviteBanner::new(post.clone())
                    .on_dark(true)
                    .on_open_author(move |_, cx| Self::open_author_profile(&open_post, &open_nav, cx)),
            )
            .w_full()
            .pt(px(4.0))
        });

        let music = (!is_uploading)
            .then(|| post.track_model())
            .flatten()
            .map(|track| Self::music_line(post, &track, &nav_store, scrub_pill));

        vstack(10.0)
            .align(HorizontalAlignment::Leading)
            .flex_1()
            .min_w_0()
            .children(banner)
            .children(chips)
            .child(author_row)
            .children(invite)
            .children(Self::caption_block(post, &nav_store, scrub_pill))
            .children(music)
    }

    fn author_row(
        post: &Rc<GetPostFeedResponse>,
        viewer: Option<&str>,
        is_uploading: bool,
        nav_store: &Option<Entity<NavigationStore>>,
        display_tag: Option<&str>,
        cx: &mut App,
    ) -> Div {
        let authored_by_me = Self::authored_by_me(post, viewer);

        let identity: AnyElement = if let Some(collaborators) = PostCollaboratorHeaderPresentation::make(post) {
            let press_id = post.id.clone();
            let hold_id = post.id.clone();
            let more_id = post.id.clone();
            let person_nav = nav_store.clone();
            hstack(10.0)
                .min_w_0()
                .child(blocks_cell_tap(
                    CollaboratorAvatarsUi::new(scoped_id("collaborators", &post.id), collaborators.author_avatar.clone())
                        .coauthors(collaborators.coauthor_avatars.clone())
                        .total_count(collaborators.total_count)
                        .size(40.0)
                        .ring_color(white().opacity(0.35))
                        .on_press(move |_, cx| Self::open_coauthors_sheet(&press_id, cx))
                        .on_long_press(move |_, cx| Self::open_coauthors_sheet(&hold_id, cx)),
                ))
                .child(
                    vstack(2.0)
                        .align(HorizontalAlignment::Leading)
                        .min_w_0()
                        .child(blocks_cell_tap(
                            CollaboratorNamesLineUi::new(
                                scoped_id("collaborator-names", &post.id),
                                collaborators.author_avatar.clone(),
                                collaborators.first_coauthor().clone(),
                                collaborators.total_count,
                            )
                            .px(15.0)
                            .font_weight(FontWeight::BOLD)
                            .color(white())
                            .on_person(move |person: &CollaboratorAvatar, _, cx| {
                                let route = RootRoute::UserPage {
                                    user_id: Some(person.id.clone()),
                                    tag: Some(person.tag.clone()),
                                };
                                nav_push(&person_nav, route, cx);
                            })
                            .on_more(move |_, cx| Self::open_coauthors_sheet(&more_id, cx)),
                        )
                        .max_w_full())
                        .children(display_tag.map(Self::tag_line)),
                )
                .into_any_element()
        } else {
            let open_post = post.clone();
            let open_nav = nav_store.clone();
            let source = Self::avatar_source(post, authored_by_me, cx);
            blocks_cell_tap(
                plain(scoped_id("author", &post.id))
                    .min_w_0()
                    .on_press(move |_, cx| Self::open_author_profile(&open_post, &open_nav, cx))
                    .child(
                        hstack(10.0)
                            .min_w_0()
                            .child(
                                div()
                                    .relative()
                                    .flex_none()
                                    .size(px(40.0))
                                    .child(
                                        UserLogoUi::new(scoped_id("author-logo", &post.id))
                                            .source(SharedString::from(source))
                                            .name(SharedString::from(post.author.name.clone()))
                                            .user_id(post.author.id.clone().map(SharedString::from))
                                            .size(40.0),
                                    )
                                    .child(centred_stroke(white().opacity(0.35), 1.0, Shape::Circle)),
                            )
                            .child(Self::names_block(post, display_tag)),
                    ),
            )
            .into_any_element()
        };

        let sparks = (!is_uploading
            && (!authored_by_me || post.sparks_total > 0)
            && post.shows_sparks_button(viewer))
        .then(|| ShortsSparksButton { post: post.clone(), is_own_post: authored_by_me, viewer: viewer.map(str::to_owned) });

        let follow = (!authored_by_me).then(|| {
            blocks_cell_tap(
                ShortsFollowButton::new(post.clone()).on_dark(true).is_enabled(!is_uploading).compacts_when_tight(true),
            )
        });

        hstack(10.0).w_full().child(identity).children(sparks).children(follow)
    }

    fn names_block(post: &GetPostFeedResponse, display_tag: Option<&str>) -> Div {
        let more = &post.author.more;
        let mut names = hstack(5.0).min_w_0().max_w_full().child(
            MainText::new(post.author.name.clone())
                .px(15.0)
                .font_weight(FontWeight::BOLD)
                .color(white())
                .number_of_lines(1),
        );
        if more.is_official {
            names = names.child(OfficialBadge::new().size(px(15.0)));
        }
        if more.is_premium {
            names = names.child(
                PremiumBadge::new()
                    .user_id(post.author.id.clone().map(SharedString::from))
                    .emoji(more.premium_badge_emoji.clone().map(SharedString::from))
                    .size(16.0),
            );
        }
        vstack(2.0).align(HorizontalAlignment::Leading).min_w_0().child(names).children(display_tag.map(Self::tag_line))
    }

    fn tag_line(tag: &str) -> Div {
        div().flex().min_w_0().max_w_full().child(
            MainText::new(format!("@{tag}")).px(12.0).color(white().opacity(0.82)).number_of_lines(1),
        )
    }

    fn music_line(
        post: &Rc<GetPostFeedResponse>,
        track: &TrackModel,
        nav_store: &Option<Entity<NavigationStore>>,
        scrub_pill: &Entity<ShortsScrubTimePill>,
    ) -> Div {
        let open_post = post.clone();
        let open_nav = nav_store.clone();
        let track_id = track.id.clone();
        blocks_cell_tap(
            plain(scoped_id("music-line", &post.id))
                .w_full()
                .min_w_0()
                .on_press(move |_, cx| {
                    let route = RootRoute::AudioPage {
                        track_id: track_id.clone(),
                        source_selection: open_post.music_use_selection(),
                    };
                    nav_push(&open_nav, route, cx);
                })
                .child(
                    hstack(6.0)
                        .w_full()
                        .min_w_0()
                        .child(div().flex_none().children(IconUi::sized("music.note", px(12.0), white(), false)))
                        .child(
                            MusicMarqueeTitle::new(scoped_id("music-title", &post.id), Post::music_line_text(track))
                                .font(Font::system_weight(13.0, FontWeight::MEDIUM))
                                .foreground(white())
                                .line_height(18.0)
                                .shows_copy_menu(false),
                        ),
                ),
        )
        .w_full()
        .relative()
        .child(ShortsScrubTimePill::level_marker(
            scrub_pill,
            ShortsScrubPillLevel::Music,
            scoped_id("scrub-level-music", &post.id),
        ))
    }

    fn caption_block(
        post: &Rc<GetPostFeedResponse>,
        nav_store: &Option<Entity<NavigationStore>>,
        scrub_pill: &Entity<ShortsScrubTimePill>,
    ) -> Option<Div> {
        let caption = trimming_whitespaces_and_newlines(&post.title);
        let body = trimming_whitespaces_and_newlines(&post.content);
        let has_body = !body.is_empty() || !post.hashtags.is_empty();
        if caption.is_empty() && !has_body {
            return None;
        }
        let title = (!caption.is_empty()).then(|| {
            let open_post = post.clone();
            blocks_cell_tap(
                plain(scoped_id("caption-title", &post.id))
                    .w_full()
                    .on_press(move |_, cx| Self::open_comments_from_caption(&open_post, cx))
                    .child(
                        MainText::new(caption.to_owned())
                            .px(14.0)
                            .tac(TextAlignT::Left)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(white())
                            .number_of_lines(1),
                    ),
            )
            .w_full()
            .relative()
            .child(ShortsScrubTimePill::level_marker(
                scrub_pill,
                ShortsScrubPillLevel::Title,
                scoped_id("scrub-level-title", &post.id),
            ))
        });
        let text = has_body.then(|| {
            let tag_post = post.clone();
            let tag_nav = nav_store.clone();
            let expand_post = post.clone();
            blocks_cell_tap(
                ExpandableTextUi::new(
                    scoped_id("caption-body", &post.id),
                    body.to_owned(),
                    white().opacity(0.92),
                    white().opacity(0.7),
                )
                .hashtags(
                    ExpandableTextHashtags::new(post.hashtags.clone(), white())
                        .weight(FontWeight::SEMIBOLD)
                        .on_tap(move |tag: &str, _, cx| {
                            if tag_post.shows_upload_overlay() {
                                return;
                            }
                            let route = RootRoute::HashtagSearch {
                                tag: tag.to_owned(),
                                kind: RisePostSearchKind::Publications,
                            };
                            nav_push(&tag_nav, route, cx);
                        }),
                )
                .inline_links(ExpandableTextInlineLinks::new(white()).underline(true))
                .font_size(14.0)
                .line_limit(if caption.is_empty() { 2 } else { 1 })
                .on_expand_request(move |_, cx| Self::open_comments_from_caption(&expand_post, cx))
                .haptics(false),
            )
            .w_full()
            .relative()
            .child(ShortsScrubTimePill::level_marker(
                scrub_pill,
                ShortsScrubPillLevel::Body,
                scoped_id("scrub-level-body", &post.id),
            ))
        });
        Some(vstack(6.0).align(HorizontalAlignment::Leading).w_full().children(title).children(text))
    }

    fn action_rail(
        post: &Rc<GetPostFeedResponse>,
        view_surface: RisePostViewSurface,
        state: &Entity<ShortsFeedCellState>,
        viewer: Option<&str>,
        sound: Option<bool>,
        window: &Window,
        cx: &App,
    ) -> Div {
        let theme = rise_ui::theme(cx);
        let primary = theme.primary._100;
        let like_scale = state.read(cx).like_bump_scale.frame_value(RiseClock::monotonic(), window);
        let nav_store = env::<NavigationStoreKey>(cx);

        let like_post = post.clone();
        let like_state = state.clone();
        let likes_post = post.clone();
        let comments_post = post.clone();
        let repost_post = post.clone();
        let share_post = post.clone();
        let more_post = post.clone();
        let cover_post = post.clone();
        let hold_post = post.clone();
        let hold_state = state.clone();
        let media_index = state.read(cx).riff_page.unwrap_or(0);
        let sound_state = state.clone();
        let sound_button = sound.map(|is_muted| {
            ShortsActionButton::new(
                scoped_id("rail-sound", &post.id),
                if is_muted { "speaker.slash" } else { "speaker.wave.3" },
                None,
                move |_, cx| sound_state.update(cx, |state, cx| state.toggle_muted(cx)),
            )
            .accessibility_name(tr(if is_muted { "feed_sound_unmute" } else { "feed_sound_mute" }))
        });

        vstack(6.0)
            .flex_none()
            .w(px(ShortsActionButton::RAIL_WIDTH))
            .pb(px(2.0))
            .children(sound_button)
            .child(
                ShortsActionButton::new(
                    scoped_id("rail-like", &post.id),
                    if post.is_liked { "heart.fill" } else { "heart" },
                    Self::rail_count(post, post.likes_count, |visibility| visibility.likes, viewer),
                    move |_, cx| Self::handle_like(&like_post, &like_state, cx),
                )
                .tint(if post.is_liked { primary } else { white() })
                .scale(like_scale)
                .on_label_tap(move |_, cx| {
                    if likes_post.shows_upload_overlay() {
                        return;
                    }
                    PostInteractionsStore::shared(cx).update(cx, |store, cx| store.open_post_likes(&likes_post, cx));
                })
                .on_hold(move |_, cx| Self::hold_like(&hold_post, &hold_state, cx))
                .hold_action_name(tr("post_likes_title"))
                .accessibility_name(tr(if post.is_liked { "ctx_unlike" } else { "ctx_like" })),
            )
            .child(ShortsActionButton::new(
                scoped_id("rail-comments", &post.id),
                "bubble.right",
                Self::rail_count(post, post.comments_count, |visibility| visibility.comments, viewer),
                move |_, cx| {
                    if comments_post.shows_upload_overlay() {
                        return;
                    }
                    PostInteractionsStore::shared(cx).update(cx, |store, cx| store.open_comments(&comments_post, false, cx));
                },
            ))
            .child(
                ShortsActionButton::new(
                    scoped_id("rail-repost", &post.id),
                    "arrow.2.squarepath",
                    Self::rail_count(post, post.repost_count, |visibility| visibility.reposts, viewer),
                    move |_, cx| {
                        if repost_post.shows_upload_overlay() {
                            return;
                        }
                        let _ = selection_changed();
                        PostInteractionsStore::shared(cx)
                            .update(cx, |store, cx| store.toggle_repost(&repost_post, None, cx));
                    },
                )
                .tint(if post.is_reposted { primary } else { white() }),
            )
            .child(ShortsActionButton::new(
                scoped_id("rail-share", &post.id),
                "paperplane",
                Self::rail_count(post, post.share_count, |visibility| visibility.shares, viewer),
                move |_, cx| {
                    if share_post.shows_upload_overlay() {
                        return;
                    }
                    let share = (*share_post).clone();
                    ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_post(share, cx));
                },
            ))
            .child(ShortsActionButton::new(
                scoped_id("rail-more", &post.id),
                "ellipsis",
                None,
                move |_, cx| {
                    if more_post.shows_upload_overlay() {
                        return;
                    }
                    let _ = selection_changed();
                    ShortsMoreSheetStore::shared(cx).update(cx, |store, cx| {
                        store.present(&more_post, view_surface != RisePostViewSurface::Shorts, media_index, cx);
                    });
                },
            ))
            .child(
                blocks_cell_tap(ShortsAudioCoverButton::new(
                    scoped_id("rail-cover", &post.id),
                    post.clone(),
                    move |_: &mut Window, cx: &mut App| {
                        let Some(track_id) = cover_post.track_id.clone().filter(|id| !id.is_empty()) else {
                            return;
                        };
                        let route = RootRoute::AudioPage {
                            track_id,
                            source_selection: cover_post.music_use_selection(),
                        };
                        nav_push(&nav_store, route, cx);
                    },
                ))
                .pt(px(6.0)),
            )
    }
}

impl RenderOnce for ShortsFeedCell {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let post = self.post;
        let is_active = self.is_active;
        let view_surface = self.view_surface;
        let destination = self.shared_element_destination;
        let rewind_token = self.rewind_token;
        let state = window.use_keyed_state(
            scoped_id("state", &post.id),
            cx,
            |_, cx: &mut Context<ShortsFeedCellState>| ShortsFeedCellState::new(cx),
        );

        let tab_host = env::<MainTabPageHostKey>(cx);
        state.update(cx, |state, cx| {
            state.shown_post = Some(post.clone());
            state.was_active = is_active;
            if let Some(host) = tab_host {
                state.observe_tab_host(host, cx);
            }
        });

        let is_own_post = PostMediaCachePolicy::is_own_post(&post, cx);
        let is_uploading = post.shows_upload_overlay();
        let engaged = Self::is_comments_engaged(&post, is_active, cx);
        let is_foreground = RiseMediaForegroundGate::shared(cx).read(cx).is_foreground();
        let (riff_page, is_muted, is_paused, cell_size, cell_origin, chrome_opacity, playback, hides_chrome, player_layer) = {
            let snapshot = state.read(cx);
            (
                snapshot.riff_page,
                snapshot.is_muted,
                snapshot.is_paused,
                snapshot.cell_size,
                snapshot.cell_origin,
                snapshot.chrome_opacity.frame_value(RiseClock::monotonic(), window),
                snapshot.playback.clone(),
                snapshot.hides_chrome(),
                snapshot.player_layer_view.clone(),
            )
        };
        let media = ShortsCellMedia::new(&post, riff_page);
        let on_auto_advance = self.on_auto_advance.clone();
        let autoscroll_armed = is_active
            && !is_uploading
            && !engaged
            && !is_paused
            && !hides_chrome
            && ShortsAutoscrollPreferences::shared(cx).read(cx).is_enabled()
            && !ShortsMoreSheetStore::shared(cx).read(cx).is_presented();
        let has_next_page = media.uses_pager && media.page_index + 1 < media.slides.len() as i64;
        let holds_video_at_end = autoscroll_armed && (has_next_page || on_auto_advance.is_some());
        {
            let (advance_post, advance_state, advance) = (post.clone(), state.clone(), on_auto_advance.clone());
            let shows_video = media.shows_video();
            let ended_count = playback.read(cx).ended_count();
            on_change(scoped_id("change-ended", &post.id), ended_count, false, window, cx, move |_, _, _, cx| {
                if autoscroll_armed && shows_video {
                    Self::advance_autoscroll(&advance_post, &advance_state, advance.as_ref(), cx);
                }
            });
        }
        {
            let photo_key = (autoscroll_armed
                && media.uses_pager
                && !media.shows_video()
                && (has_next_page || on_auto_advance.is_some()))
            .then(|| format!("{}#{}", post.id, media.page_index));
            let armed = photo_key.is_some();
            let (timer_post, timer_state, advance) = (post.clone(), state.clone(), on_auto_advance.clone());
            use_task(scoped_id("autoscroll-photo", &post.id), photo_key, window, cx, async move |cx| {
                if !armed {
                    return;
                }
                cx.background_executor().timer(ShortsAutoscroll::photo_duration()).await;
                let _ = cx.update(|_, cx| Self::advance_autoscroll(&timer_post, &timer_state, advance.as_ref(), cx));
            });
        }

        if let Some(destination) = &destination {
            destination.update_player_layer(&post.id, player_layer);
        }

        on_change(
            scoped_id("change-chrome-hidden", &post.id),
            is_active && hides_chrome,
            false,
            window,
            cx,
            move |_, hidden, _, cx| {
                ShortsInteractionsStore::shared(cx).update(cx, |store, cx| store.feed_chrome_hidden_changed(*hidden, cx));
            },
        );
        {
            let (post_for_change, state_for_change) = (post.clone(), state.clone());
            on_change(scoped_id("change-active", &post.id), is_active, false, window, cx, move |_, active, _, cx| {
                if !*active {
                    state_for_change.update(cx, |state, cx| state.clear_paused(cx));
                }
                Self::sync_slide_music(&post_for_change, *active, &state_for_change, cx);
            });
        }
        {
            let (post_for_change, state_for_change) = (post.clone(), state.clone());
            on_change(scoped_id("change-muted", &post.id), is_muted, false, window, cx, move |_, _, _, cx| {
                Self::sync_slide_music(&post_for_change, is_active, &state_for_change, cx);
            });
        }
        {
            let playback_for_change = playback.clone();
            on_change(scoped_id("change-rewind", &post.id), rewind_token, false, window, cx, move |_, _, _, cx| {
                playback_for_change.update(cx, |playback, cx| playback.seek(0.0, true, cx));
            });
        }
        {
            let (post_for_change, state_for_change, destination) = (post.clone(), state.clone(), destination.clone());
            on_change(scoped_id("change-page", &post.id), riff_page, false, window, cx, move |_, _, _, cx| {
                state_for_change.update(cx, |state, cx| state.clear_paused(cx));
                Self::sync_slide_tags(&post_for_change, &state_for_change, cx);
                Self::sync_slide_music(&post_for_change, is_active, &state_for_change, cx);
                Self::report_slide(&post_for_change, destination.as_deref(), &state_for_change, cx);
            });
        }
        {
            let (post_for_change, state_for_change) = (post.clone(), state.clone());
            on_change(scoped_id("change-foreground", &post.id), is_foreground, false, window, cx, move |_, _, _, cx| {
                Self::sync_slide_music(&post_for_change, is_active, &state_for_change, cx);
            });
        }

        let safe_top = EditorImageLayout::resolved_safe_area_insets(EdgeInsets::new(0.0, 0.0, 0.0, 0.0)).top;
        let layout = engaged.then(|| ShortsCommentsCompactLayoutGeometry {
            cell_size,
            cell_origin,
            window_height: f32::from(window.viewport_size().height),
            safe_top,
            resting_fraction: CommentsSheet::INITIAL_SNAP_FRACTION,
        });
        let bottom_padding = 34.0_f32.max(self.bottom_inset + 12.0);
        let progress_floor = self.progress_floor.unwrap_or(self.bottom_inset);
        let progress_bottom_padding =
            ((bottom_padding + Self::PROGRESS_SLOT + progress_floor) / 2.0 - Self::PROGRESS_HEIGHT / 2.0).max(0.0);
        let caption_bottom_padding = if media.contains_video() {
            bottom_padding
        } else {
            bottom_padding.max(progress_floor + Self::CAPTION_FLOOR_GAP)
        };

        let pinch_state = state.clone();
        let (motion_post, motion_state, motion_media) = (post.clone(), state.clone(), media.clone());
        let media_content = ShortsPinchZoomLayer::new(
            scoped_id("pinch", &post.id),
            cell_size,
            !is_uploading && !engaged,
            move |active: bool, _: &mut Window, cx: &mut App| {
                pinch_state.update(cx, |state, cx| state.set_pinching(active, cx));
            },
            move |zoomed_size: Size<Pixels>, _: &mut Window, _: &mut App| {
                ShortsCommentsMotionLayer {
                    layout,
                    cell_size: zoomed_size,
                    content: Box::new(move |corner_radius: f32, drawn_size: Size<Pixels>, cx: &App| {
                        Self::media_layer(
                            &motion_post,
                            &motion_media,
                            drawn_size,
                            is_active,
                            is_own_post,
                            engaged,
                            holds_video_at_end,
                            corner_radius,
                            &motion_state,
                            cx,
                        )
                    }),
                }
                .into_any_element()
            },
        )
        .shared_element_anchor(post.id.clone(), SharedElementPart::Media);
        let hold_post = post.clone();
        let hold_state = state.clone();
        let hold_layer = ShortsHoldRecognizerHost::new(
            scoped_id("hold", &post.id),
            !is_uploading && !engaged,
            Self::HOLD_DELAY,
            move |holding: bool, cx: &mut App| Self::apply_holding(&hold_post, is_active, &hold_state, holding, cx),
        );

        let chrome = Self::chrome_layer(
            &post,
            &media,
            cell_size,
            safe_top,
            caption_bottom_padding,
            progress_bottom_padding,
            view_surface,
            &state,
            window,
            cx,
        );
        let chrome = div()
            .opacity(chrome_opacity)
            .child(CommentsSheetProgressFade::new().engaged(engaged).hides_with_sheet(true).child(chrome.into_any_element()))
            .shared_element_lift(post.id.clone(), SharedElementPart::Text);

        let sound_control = layout.filter(|_| Self::has_sound_control(&post, &media)).map(|layout| {
            div()
                .w(cell_size.width)
                .h(cell_size.height)
                .flex()
                .justify_end()
                .child(ShortsCommentsSoundControlLayer {
                    post_id: post.id.clone(),
                    layout,
                    is_muted,
                    state: state.clone(),
                })
                .shared_element_lift(post.id.clone(), SharedElementPart::Text)
        });

        let tap_post = post.clone();
        let tap_state = state.clone();
        let measure_state = state.clone();
        let appear_post = post.clone();
        let appear_state = state.clone();
        let appear_destination = destination.clone();
        let disappear_post = post.clone();
        let disappear_destination = destination.clone();
        let disappear_state = state.clone();

        div()
            .id(scoped_id("root", &post.id))
            .relative()
            .size_full()
            .bg(black())
            .on_click(move |_, _, cx| {
                if !tap_state.read(cx).accepts_tap() {
                    return;
                }
                let media = ShortsCellMedia::new(&tap_post, tap_state.read(cx).riff_page);
                if tap_post.shows_upload_overlay()
                    || media.uses_pager
                    || !media.shows_video()
                    || Self::is_comments_engaged(&tap_post, is_active, cx)
                {
                    return;
                }
                tap_state.update(cx, |state, cx| state.toggle_paused(cx));
            })
            .child(
                div().absolute().inset_0().child(
                    PostVisibilityReader::new(
                        scoped_id("visibility", &post.id),
                        post.rise_view_target_post_id(),
                        view_surface,
                    )
                    .enabled(is_active && !is_uploading),
                ),
            )
            .child(pass_through().absolute().inset_0().child(hold_layer))
            .child(div().absolute().inset_0().child(media_content))
            .child(div().absolute().inset_0().child(chrome))
            .children(sound_control.map(|control| div().absolute().inset_0().child(control)))
            .child(measure(scoped_id("size", &post.id), move |bounds: Bounds<Pixels>, _, cx| {
                measure_state.update(cx, |state, cx| {
                    let size_changed = state.cell_size != bounds.size;
                    let origin_changed = state.cell_origin != bounds.origin;
                    state.cell_size = bounds.size;
                    state.cell_origin = bounds.origin;
                    if size_changed || (origin_changed && engaged) {
                        cx.notify();
                    }
                });
            }))
            .on_appear(scoped_id("lifecycle", &post.id), move |_, cx| {
                Self::sync_slide_tags(&appear_post, &appear_state, cx);
                Self::sync_slide_music(&appear_post, is_active, &appear_state, cx);
                Self::report_slide(&appear_post, appear_destination.as_deref(), &appear_state, cx);
            })
            .and_on_disappear(move |_, cx| {
                let key = Self::slide_music_key(&disappear_post);
                PostMusicPlaybackController::shared(cx).update(cx, |controller, cx| controller.stop(&key, cx));
                if let Some(destination) = &disappear_destination {
                    destination.update_player_layer(&disappear_post.id, None);
                }
                if is_active && disappear_state.read(cx).hides_chrome() {
                    ShortsInteractionsStore::shared(cx).update(cx, |store, cx| store.feed_chrome_hidden_changed(false, cx));
                }
            })
    }
}

#[derive(IntoElement)]
struct ShortsSparksButton {
    post: Rc<GetPostFeedResponse>,
    is_own_post: bool,
    viewer: Option<String>,
}

struct ShortsSparksButtonState {
    taps: i64,
}

impl ShortsSparksButton {
    fn tap(post: &GetPostFeedResponse, is_own_post: bool, taps: &Entity<ShortsSparksButtonState>, cx: &mut App) {
        if post.is_temp {
            return;
        }
        let target = SparksTarget::Post { post_id: post.id.clone() };
        let store = SparksInteractionsStore::shared(cx);
        if is_own_post {
            store.update(cx, |store, cx| store.open_send_sheet(target, true, None, cx));
        } else if store.update(cx, |store, cx| store.tap(&target, Some(post.author_id.as_str()), cx)) {
            taps.update(cx, |state, cx| {
                state.taps = state.taps.wrapping_add(1);
                cx.notify();
            });
        }
    }

    const SPARKS_ICON_SHADOW_ASSET: &'static str = "images/SparksIconSmall.svg";

    fn hold(post: &GetPostFeedResponse, is_own_post: bool, cx: &mut App) {
        if post.is_temp {
            return;
        }
        let target = SparksTarget::Post { post_id: post.id.clone() };
        SparksInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.open_send_sheet(target, is_own_post, Some(post.author_id.as_str()), cx);
        });
    }
}

impl RenderOnce for ShortsSparksButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let post = self.post;
        let is_own_post = self.is_own_post;
        let taps = window.use_keyed_state(scoped_id("sparks-taps", &post.id), cx, |_, _| ShortsSparksButtonState { taps: 0 });
        let label = (post.sparks_total > 0
            && post.shows_count(|visibility| visibility.sparks, self.viewer.as_deref()))
        .then(|| RiseFormat::compact_count(post.sparks_total));
        let tap_post = post.clone();
        let tap_taps = taps.clone();
        let hold_post = post.clone();

        let accessibility_label = tr(if is_own_post { "sparks_top_senders_title" } else { "sparks_send_title" });
        div()
            .id(scoped_id("sparks-label", &post.id))
            .aria_label(accessibility_label.clone())
            .tooltip_label(accessibility_label)
            .flex_none()
            .occlude()
            .on_mouse_down(MouseButton::Right, move |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                Self::hold(&hold_post, is_own_post, cx);
            })
            .child(
                plain(scoped_id("sparks", &post.id))
                    .px(px(4.0))
                    .min_h(px(40.0))
                    .flex()
                    .items_center()
                    .gap(px(5.0))
                    .on_press(move |_, cx| Self::tap(&tap_post, is_own_post, &tap_taps, cx))
                    .child(
                        div()
                            .relative()
                            .flex_none()
                            .child(
                                svg()
                                    .path(Self::SPARKS_ICON_SHADOW_ASSET)
                                    .absolute()
                                    .left_0()
                                    .top(px(1.0))
                                    .size(px(22.0))
                                    .text_color(black().opacity(0.35)),
                            )
                            .child(
                                SparksBurstIcon::new(
                                    22.0,
                                    SparksTarget::Post { post_id: post.id.clone() },
                                    taps.read(cx).taps,
                                )
                                .is_live(!post.is_temp),
                            ),
                    )
                    .children(label.map(|label| shadowed_text(&label, 13.0, true, false))),
            )
    }
}

/// hit area is the full rail width by finger height,
/// the counter below is its own target when a tap is passed
#[derive(IntoElement)]
struct ShortsActionButton {
    id: ElementId,
    system_image: &'static str,
    label: Option<String>,
    tint: Hsla,
    scale: f32,
    action: ButtonAction,
    on_label_tap: Option<ButtonAction>,
    on_hold: Option<ButtonAction>,
    hold_action_name: Option<String>,
    accessibility_name: Option<String>,
}

impl ShortsActionButton {
    const RAIL_WIDTH: f32 = 52.0;
    const GLYPH_HIT_LEAD: f32 = 8.0;
    const LABEL_HIT_TAIL: f32 = 6.0;

    fn new(
        id: ElementId,
        system_image: &'static str,
        label: Option<String>,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id,
            system_image,
            label,
            tint: white(),
            scale: 1.0,
            action: Rc::new(action),
            on_label_tap: None,
            on_hold: None,
            hold_action_name: None,
            accessibility_name: None,
        }
    }

    fn on_hold(mut self, on_hold: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_hold = Some(Rc::new(on_hold));
        self
    }

    fn hold_action_name(mut self, hold_action_name: impl Into<String>) -> Self {
        self.hold_action_name = Some(hold_action_name.into());
        self
    }

    fn accessibility_name(mut self, accessibility_name: impl Into<String>) -> Self {
        self.accessibility_name = Some(accessibility_name.into());
        self
    }

    fn tint(mut self, tint: Hsla) -> Self {
        self.tint = tint;
        self
    }

    fn scale(mut self, scale: f32) -> Self {
        self.scale = scale;
        self
    }

    fn on_label_tap(mut self, on_label_tap: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_label_tap = Some(Rc::new(on_label_tap));
        self
    }
}

impl RenderOnce for ShortsActionButton {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let action = self.action.clone();
        let label_action = self.on_label_tap.unwrap_or_else(|| self.action.clone());
        let label_id = ElementId::NamedChild(Arc::new(self.id.clone()), "label".into());
        let glyph = shadowed_glyph(self.system_image, 25.0 * self.scale, self.tint);
        let button = plain(self.id.clone())
            .w(px(Self::RAIL_WIDTH))
            .pt(px(Self::GLYPH_HIT_LEAD))
            .flex()
            .justify_center()
            .on_press(move |window, cx| action(window, cx))
            .child(div().size(px(32.0)).flex().items_center().justify_center().child(glyph));
        let glyph_slot = match self.on_hold {
            Some(on_hold) => {
                let accessibility_name = self.accessibility_name.unwrap_or_default();
                let hold_name = self.hold_action_name.unwrap_or_default();
                let hint = if hold_name.is_empty() {
                    accessibility_name.clone()
                } else {
                    format!("{accessibility_name} \u{00b7} {hold_name}")
                };
                div()
                    .id(ElementId::NamedChild(Arc::new(self.id), "hold".into()))
                    .aria_label(accessibility_name)
                    .tooltip_label(hint)
                    .occlude()
                    .on_mouse_down(MouseButton::Right, move |_: &MouseDownEvent, window, cx| {
                        cx.stop_propagation();
                        on_hold(window, cx);
                    })
                    .child(button)
            }
            None => div().id(ElementId::NamedChild(Arc::new(self.id), "slot".into())).occlude().child(button),
        };
        vstack(1.0)
            .child(glyph_slot)
            .children(self.label.map(|label| {
                div().occlude().mb(px(-Self::LABEL_HIT_TAIL)).child(
                    plain(label_id)
                        .w(px(Self::RAIL_WIDTH))
                        .h(px(18.0 + Self::LABEL_HIT_TAIL))
                        .pb(px(Self::LABEL_HIT_TAIL))
                        .flex()
                        .items_center()
                        .justify_center()
                        .on_press(move |window, cx| label_action(window, cx))
                        .child(shadowed_text(&label, 12.0, false, true)),
                )
            }))
    }
}

#[derive(IntoElement)]
struct ShortsPendingLoaderOverlay {
    post: Rc<GetPostFeedResponse>,
}

impl ShortsPendingLoaderOverlay {
    fn ring_progress(&self) -> f64 {
        if self.post.is_compressing {
            return (self.post.compression_progress / 100.0).clamp(0.08, 1.0);
        }
        if self.post.upload_progress >= 99.5 {
            return 1.0;
        }
        (self.post.upload_progress / 100.0).clamp(0.08, 1.0)
    }

    fn ring_animated(&self) -> bool {
        !self.post.is_compressing && self.post.upload_progress <= 0.0
    }

    fn status_text(&self) -> String {
        if self.post.is_compressing {
            return format!("{} {}%", tr("chat_media_upload_processing"), self.post.compression_progress as i64);
        }
        if self.post.upload_progress >= 99.5 {
            return tr("chat_media_upload_sending");
        }
        if self.post.upload_progress > 0.0 {
            return format!("{} {}%", tr("shorts_uploading"), self.post.upload_progress as i64);
        }
        tr("shorts_uploading")
    }

    fn local_preview_path(&self) -> Option<PathBuf> {
        let url = Url::from_string(self.post.local_preview_url.as_deref()?)?;
        if !url.is_file_url() {
            return None;
        }
        url.to_file_path().filter(|path| path.is_file())
    }

    fn thumbnail(&self, cx: &mut App) -> AnyElement {
        if let Some(path) = self.local_preview_path() {
            return img(path.as_path()).size_full().object_fit(ObjectFit::Cover).into_any_element();
        }
        let thumb = self.post.video_thumbnail.as_deref().map(trimming_whitespaces_and_newlines).unwrap_or("");
        if !thumb.is_empty() {
            let is_own_post = PostMediaCachePolicy::is_own_post(&self.post, cx);
            return CleverImageUi::new(SharedString::from(thumb.to_owned()))
                .content_mode(ContentMode::Fill)
                .corner_radius(0.0)
                .persist_to_disk(is_own_post)
                .cache_policy(PostMediaCachePolicy::image_policy(is_own_post))
                .into_any_element();
        }
        div().size_full().bg(black()).into_any_element()
    }
}

impl RenderOnce for ShortsPendingLoaderOverlay {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let thumbnail = self.thumbnail(cx);
        div()
            .relative()
            .size_full()
            .child(div().absolute().inset_0().overflow_hidden().child(thumbnail))
            .child(div().absolute().inset_0().bg(black().opacity(0.45)))
            .child(
                div().absolute().inset_0().flex().items_center().justify_center().child(
                    vstack(12.0)
                        .child(
                            LoaderUi::new()
                                .size(LoaderUiSize::Large)
                                .progress(self.ring_progress())
                                .r#type(LoaderUiType::Progress)
                                .ring_animated(self.ring_animated())
                                .speed(3.0),
                        )
                        .child(
                            MainText::new(self.status_text())
                                .px(14.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(white()),
                        ),
                ),
            )
    }
}

type ShortsCommentsMotionContent = Box<dyn FnOnce(f32, Size<Pixels>, &App) -> AnyElement>;

#[derive(IntoElement)]
struct ShortsCommentsMotionLayer {
    layout: Option<ShortsCommentsCompactLayoutGeometry>,
    cell_size: Size<Pixels>,
    content: ShortsCommentsMotionContent,
}

impl RenderOnce for ShortsCommentsMotionLayer {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let relay = CommentsSheetMotionRelay::shared(cx);
        let (sheet_fraction, sheet_frame, progress) = {
            let relay = relay.read(cx);
            (relay.sheet_fraction(), relay.sheet_frame(), relay.progress())
        };
        let placement = self.layout.map(|layout| layout.motion_placement(sheet_fraction, sheet_frame, progress));
        let scale = placement.map_or(1.0, |placement| placement.scale);
        let offset_x = placement.map_or(0.0, |placement| placement.offset_x);
        let offset_y = placement.map_or(0.0, |placement| placement.offset_y);
        let corner_radius = placement.map_or(0.0, |placement| placement.unscaled_corner_radius * placement.scale);
        let width = f32::from(self.cell_size.width);
        let height = f32::from(self.cell_size.height);
        let drawn_size = size(px(width * scale), px(height * scale));
        let content = (self.content)(corner_radius, drawn_size, cx);
        div().relative().w(self.cell_size.width).h(self.cell_size.height).child(
            div()
                .absolute()
                .left(px(width * (1.0 - scale) / 2.0 + offset_x))
                .top(px(height * (1.0 - scale) / 2.0 + offset_y))
                .child(content),
        )
    }
}

#[derive(IntoElement)]
struct ShortsCommentsSoundControlLayer {
    post_id: String,
    layout: ShortsCommentsCompactLayoutGeometry,
    is_muted: bool,
    state: Entity<ShortsFeedCellState>,
}

impl RenderOnce for ShortsCommentsSoundControlLayer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        const BUTTON_SIZE: f32 = 42.0;
        const INSET: f32 = 10.0;
        let relay = CommentsSheetMotionRelay::shared(cx);
        let (sheet_fraction, sheet_frame, progress) = {
            let relay = relay.read(cx);
            (relay.sheet_fraction(), relay.sheet_frame(), relay.progress())
        };
        let placement = self.layout.motion_placement(sheet_fraction, sheet_frame, progress);
        let cell_width = f32::from(self.layout.cell_size.width).max(1.0);
        let video_height = f32::from(self.layout.cell_size.height).max(1.0) * placement.scale;
        let video_width = cell_width * placement.scale;
        let fits = if placement.parks_reel {
            ((video_height - BUTTON_SIZE - INSET * 2.0) / INSET)
                .min((video_width - BUTTON_SIZE - INSET * 2.0) / INSET)
                .clamp(0.0, 1.0)
        } else {
            0.0
        };
        let trailing_padding = 14.0;
        let visibility = progress * fits;
        let is_live = fits > 0.5;
        let post_id = self.post_id;

        {
            let clear_id = post_id.clone();
            let clear_relay = relay.clone();
            on_change(scoped_id("sound-control-live", &post_id), is_live, false, window, cx, move |_, is_live, _, cx| {
                if !*is_live {
                    clear_relay.update(cx, |relay, _| relay.clear_backdrop_control(&clear_id));
                }
            });
        }
        let disappear_id = post_id.clone();
        let disappear_relay = relay.clone();

        let frame_reader = is_live.then(|| {
            let (report_id, report_relay, report_state) = (post_id.clone(), relay.clone(), self.state.clone());
            measure(scoped_id("sound-control-frame", &post_id), move |frame: Bounds<Pixels>, _, cx| {
                let action_state = report_state.clone();
                report_relay.update(cx, |relay, _| {
                    relay.set_backdrop_control(
                        &report_id,
                        frame,
                        Rc::new(move |cx: &mut App| action_state.update(cx, |state, cx| state.toggle_muted(cx))),
                    );
                });
            })
        });

        let mute_button =
            ShortsFeedCell::mute_button(&post_id, "sound-control-mute", &self.state, self.is_muted, visibility > 0.5);
        let control = div().relative().flex_none().size(px(BUTTON_SIZE)).child(mute_button).children(frame_reader);

        div()
            .pr(px(trailing_padding))
            .pt(px((placement.video_bottom_y - BUTTON_SIZE - INSET).max(0.0)))
            .opacity(visibility)
            .child(control)
            .on_disappear(scoped_id("sound-control-lifecycle", &post_id), move |_, cx| {
                disappear_relay.update(cx, |relay, _| relay.clear_backdrop_control(&disappear_id));
            })
    }
}

/// derived once per render,
/// a lone video keeps one player,
/// photos page horizontally in author order
#[derive(Clone)]
pub struct ShortsCellMedia {
    pub slides: Vec<PostMediaSlide>,
    pub uses_pager: bool,
    pub page_index: i64,
    pub single_video_url: Option<String>,
}

impl ShortsCellMedia {
    pub fn new(post: &GetPostFeedResponse, page: Option<i64>) -> Self {
        let slides = post.media_slides();
        let lone_video = slides.is_empty() || (slides.len() == 1 && slides[0].is_video());
        let page_index = ShortsRiffPager::resolved_index(page, slides.len());
        let single_video_url =
            if lone_video { post.resolved_video_url().filter(|url| !url.is_empty()) } else { None };
        Self { slides, uses_pager: !lone_video, page_index, single_video_url }
    }

    pub fn current_slide(&self) -> Option<&PostMediaSlide> {
        usize::try_from(self.page_index).ok().and_then(|index| self.slides.get(index))
    }

    /// true when a video is on screen, pause and scrubber only belong to it
    pub fn shows_video(&self) -> bool {
        if self.uses_pager {
            self.current_slide().is_some_and(PostMediaSlide::is_video)
        } else {
            self.single_video_url.is_some()
        }
    }

    pub fn contains_video(&self) -> bool {
        self.single_video_url.is_some() || self.slides.iter().any(PostMediaSlide::is_video)
    }
}
