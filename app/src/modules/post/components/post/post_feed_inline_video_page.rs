use std::rc::Rc;

use gpui::{App, Bounds, BoxShadow, Context, ElementId, Empty, Entity, Pixels, SharedString, Subscription, Window, black, point, white};
use rise_core::foundation::foundation_uuid::Uuid;
use rise_media::video::av_player::AVLayerVideoGravity;
use rise_ui::icon_ui::IconUi;
use rise_ui::swiftui::prelude::*;

use crate::core::media::rise_media_foreground_gate::RiseMediaForegroundGate;
use crate::core::media_viewer::media_viewer_presentation_state::{MediaViewerPresentationState, media_viewer_presentation};
use crate::core::shared_element_transition::kernel::shared_element_types::SharedElementSurface;
use crate::core::shared_element_transition::runtime::shared_element_lift_board::SharedElementSurfaceKey;
use crate::modules::post::components::collaborators::post_tag_bubbles_overlay::PostMediaTagsState;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::components::post::post_music_playback_controller::PostAmbientMusicDriver;
use crate::modules::post::pages::posts::post_detail::post_list_shared_element_capture::PostListSharedElementCapture;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::shorts::components::feed_sound_button::FeedSoundButton;
use crate::modules::shorts::components::shorts_feed_player::{ShortsFeedPlayer, ShortsMusicOverlay, ShortsPlaybackController};
use crate::modules::shorts::components::shorts_inline_feed_cell::ShortsFeedFocusCoordinator;
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;

/// carousel video page laid over the page poster until the first frame covers it
/// same inline gates as the shorts inline cell
/// one focus winner across all inline surfaces
#[derive(IntoElement)]
pub struct PostFeedInlineVideoPage {
    id: SharedString,
    post: Rc<GetPostFeedResponse>,
    url: String,
    thumbnail: Option<String>,
    is_current_page: bool,
    fills: bool,
    is_viewer_presented: bool,
    on_aspect: Option<Rc<dyn Fn(f32, &mut Window, &mut App)>>,
}

impl PostFeedInlineVideoPage {
    /// fills is the carousels fill rule for this page, same one its poster is drawn by
    pub fn new(
        id: impl Into<SharedString>,
        post: impl Into<Rc<GetPostFeedResponse>>,
        url: impl Into<String>,
        thumbnail: Option<String>,
        is_current_page: bool,
        fills: bool,
        is_viewer_presented: bool,
    ) -> Self {
        Self {
            id: id.into(),
            post: post.into(),
            url: url.into(),
            thumbnail,
            is_current_page,
            fills,
            is_viewer_presented,
            on_aspect: None,
        }
    }

    /// file aspect with rotation applied
    /// reported only for a page without a poster, otherwise the page stays laid out by the poster shape
    pub fn on_aspect(mut self, on_aspect: impl Fn(f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_aspect = Some(Rc::new(on_aspect));
        self
    }

    fn element_id(&self, suffix: &str) -> ElementId {
        ElementId::Name(format!("{}-{suffix}", self.id).into())
    }

    /// the track synced to the video under its own owner key
    /// the photo pages ambient driver owns post.id and a page swipe runs one sides stop and the
    /// others start in no fixed order
    fn music_overlay(&self) -> Option<ShortsMusicOverlay> {
        let overlay = self.post.shorts_music_overlay()?;
        Some(ShortsMusicOverlay {
            post_id: format!("{}#inline", self.post.id),
            url: overlay.url,
            track_id: overlay.track_id,
            start_ms: overlay.start_ms,
            end_ms: overlay.end_ms,
        })
    }
}

/// view state plus the stores its body reads
struct PostFeedInlineVideoPageState {
    focus_token: String,
    playback: Entity<ShortsPlaybackController>,
    viewer_generation: u64,
    viewer_was_presented: bool,
    _subscriptions: Vec<Subscription>,
}

impl PostFeedInlineVideoPageState {
    fn new(viewer: Entity<MediaViewerPresentationState>, cx: &mut Context<Self>) -> Self {
        let shorts_store = ShortsServicesStore::shared(cx);
        let focus = ShortsFeedFocusCoordinator::shared(cx);
        let foreground_gate = RiseMediaForegroundGate::shared(cx);
        let viewer_was_presented = viewer.read(cx).is_presented();
        Self {
            focus_token: Uuid::new_v4().uuid_string(),
            playback: cx.new(|_| ShortsPlaybackController::new()),
            viewer_generation: 0,
            viewer_was_presented,
            _subscriptions: vec![
                cx.observe(&shorts_store, |_, _, cx| cx.notify()),
                cx.observe(&focus, |_, _, cx| cx.notify()),
                cx.observe(&foreground_gate, |_, _, cx| cx.notify()),
                cx.observe(&viewer, |state: &mut Self, viewer, cx| {
                    let is_presented = viewer.read(cx).is_presented();
                    if state.viewer_was_presented && !is_presented {
                        state.viewer_generation = state.viewer_generation.wrapping_add(1);
                    }
                    state.viewer_was_presented = is_presented;
                    cx.notify();
                }),
            ],
        }
    }
}

impl RenderOnce for PostFeedInlineVideoPage {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let viewer = media_viewer_presentation(cx);
        let state = window.use_keyed_state(
            self.element_id("state"),
            cx,
            move |_, cx: &mut Context<PostFeedInlineVideoPageState>| PostFeedInlineVideoPageState::new(viewer, cx),
        );
        let focus = ShortsFeedFocusCoordinator::shared(cx);
        let shorts_store = ShortsServicesStore::shared(cx);
        let focus_token = state.read(cx).focus_token.clone();
        let playback = state.read(cx).playback.clone();
        let viewer_generation = state.read(cx).viewer_generation;

        let is_focused = self.is_current_page && focus.read(cx).active_token() == Some(focus_token.as_str());
        let mounts_player = is_focused && !self.is_viewer_presented && !self.post.shows_upload_overlay();
        let is_active = mounts_player
            && shorts_store.read(cx).inline_feed_playback_enabled()
            && RiseMediaForegroundGate::shared(cx).read(cx).is_foreground();

        let flight_session = match env::<SharedElementSurfaceKey>(cx) {
            Some(SharedElementSurface::PostList { session }) => Some(session),
            _ => None,
        };
        if let Some(session) = flight_session {
            let layer = if mounts_player { playback.read(cx).player_layer_view() } else { None };
            PostListSharedElementCapture::update_player_layer(session, &self.post.id, layer);
        }
        let leaving_post_id = self.post.id.clone();

        let change_focus = focus.clone();
        let change_token = focus_token.clone();
        on_change(self.element_id("current"), self.is_current_page, false, window, cx, move |_, current, _, cx| {
            if !*current {
                change_focus.update(cx, |focus, cx| focus.remove(&change_token, cx));
            }
        });

        let shows_sound_button =
            self.is_current_page && self.post.track_model().is_none() && !self.post.shows_upload_overlay();

        let player = mounts_player.then(|| {
            let is_own = PostMediaCachePolicy::is_own_post(&self.post, cx);
            let is_muted = shorts_store.read(cx).feed_audio_muted;
            let mut player = ShortsFeedPlayer::new(
                self.element_id("player"),
                self.url.clone(),
                self.thumbnail.clone(),
                is_active,
            )
            .is_muted(is_muted)
            .playback(Some(playback))
            .gravity_override(Some(if self.fills {
                AVLayerVideoGravity::ResizeAspectFill
            } else {
                AVLayerVideoGravity::ResizeAspect
            }))
            .draws_opaque_background(false)
            .music_overlay(self.music_overlay())
            .persists_post_media_to_disk(is_own)
            .post_image_cache_policy(PostMediaCachePolicy::image_policy(is_own))
            .releases_player_on_disappear(true);
            if let (None, Some(on_aspect)) = (&self.thumbnail, self.on_aspect.clone()) {
                player = player.on_aspect(move |aspect, window, cx| on_aspect(aspect, window, cx));
            }
            pass_through().absolute().inset_0().overflow_hidden().child(player)
        });

        let report_focus = focus.clone();
        let report_token = focus_token.clone();
        let focus_reporter = self.is_current_page.then(|| {
            measure(self.element_id(&format!("focus-{viewer_generation}")), move |frame: Bounds<Pixels>, _, cx| {
                report_focus.update(cx, |focus, cx| focus.report(&report_token, frame, cx));
            })
        });

        let disappear_id = self.element_id("disappear");
        div()
            .relative()
            .size_full()
            .children(focus_reporter)
            .children(player)
            .when(!is_active, |page| page.child(PostFeedVideoGlyph))
            .when(shows_sound_button, |page| {
                page.child(div().absolute().right(px(12.0)).bottom(px(12.0)).child(FeedSoundButton::new()))
            })
            .on_disappear(disappear_id, move |_, cx| {
                focus.update(cx, |focus, cx| focus.remove(&focus_token, cx));
                if let Some(session) = flight_session {
                    PostListSharedElementCapture::update_player_layer(session, &leaving_post_id, None);
                }
            })
    }
}

/// mounts the feed ambient track driver except while the carousel shows its video page
/// that page owns the row focus and the track
/// two focus reporters for one row would fight
#[derive(IntoElement)]
pub struct PostCarouselAmbientMusicGate {
    id: SharedString,
    post: Rc<GetPostFeedResponse>,
    tags_state: Entity<PostMediaTagsState>,
}

impl PostCarouselAmbientMusicGate {
    pub fn new(
        id: impl Into<SharedString>,
        post: impl Into<Rc<GetPostFeedResponse>>,
        tags_state: Entity<PostMediaTagsState>,
    ) -> Self {
        Self { id: id.into(), post: post.into(), tags_state }
    }
}

/// observable on the tag slide this views body reads
struct PostCarouselAmbientMusicGateObserver {
    _tags_state: Subscription,
}

impl RenderOnce for PostCarouselAmbientMusicGate {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let observed = self.tags_state.clone();
        window.use_keyed_state(
            ElementId::Name(format!("{}-observer", self.id).into()),
            cx,
            move |_, cx: &mut Context<PostCarouselAmbientMusicGateObserver>| PostCarouselAmbientMusicGateObserver {
                _tags_state: cx.observe(&observed, |_, _, cx| cx.notify()),
            },
        );
        if self.post.has_video_media() && self.tags_state.read(cx).slide == -1 {
            return Empty.into_any_element();
        }
        PostAmbientMusicDriver::new(format!("{}-driver", self.id), self.post).into_any_element()
    }
}

#[derive(IntoElement)]
pub struct PostFeedVideoGlyph;

impl RenderOnce for PostFeedVideoGlyph {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        pass_through().absolute().inset_0().flex().items_center().justify_center().child(
            pass_through()
                .flex()
                .items_center()
                .justify_center()
                .size(px(54.0))
                .rounded_full()
                .bg(black().opacity(0.45))
                .shadow(vec![BoxShadow {
                    color: black().opacity(0.28),
                    offset: point(px(0.0), px(2.0)),
                    blur_radius: px(12.0),
                    spread_radius: px(0.0),
                    inset: false,
                }])
                .children(IconUi::sized("play.fill", px(22.0), white(), true)),
        )
    }
}
