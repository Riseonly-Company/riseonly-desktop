use std::cell::Cell;
use std::rc::{Rc, Weak};

use gpui::{
    App, Bounds, Context, ElementId, Entity, FontWeight, IntoElement, Pixels, Point, Size, Subscription, Window,
    black, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_media::video::av_player::AVLayerVideoGravity;
use rise_theme::{ThemeStore, alpha};
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::material::Material;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{Presence, Transition};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey};
use crate::app::router::navigation_types::{MainTab, RootRoute};
use crate::core::media::rise_media_foreground_gate::RiseMediaForegroundGate;
use crate::core::media_viewer::media_viewer_presentation_state::{MediaViewerPresentationState, media_viewer_presentation};
use crate::core::ui::custom_emoji_glyph::CustomEmojiGlyph;
use crate::core::ui::premium_badge::PremiumBadgeMetrics;
use crate::modules::post::components::post::post_feed_inline_video_page::PostFeedVideoGlyph;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::components::post::post_music_playback_controller::PostMusicPlaybackController;
use crate::modules::post::components::repost::repost_overlay::{RepostOverlay, RepostStickerStyle};
use crate::modules::post::pages::posts::post_detail::native_list::behaviour::post_list_playback_focus::PostListFocusFrameView;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostListFocusReporting, PostListHostedEnvironment, PostListHostedEnvironmentModifiers, PostListPlaybackToken,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_layout_calculator::PostRowMetrics;
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_media_overlays::PostRowMediaMetrics;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::shorts::components::feed_sound_button::FeedSoundButton;
use crate::modules::shorts::components::shorts_feed_player::{
    ShortsFeedPlayer, ShortsMusicOverlay, ShortsPlaybackController,
};
use crate::modules::shorts::components::shorts_inline_feed_cell::ShortsFeedFocusCoordinator;
use crate::modules::shorts::components::shorts_zoom_namespace::{ShortsZoomNamespaceKey, ShortsZoomSource};
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::StickerItem;

fn child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(std::sync::Arc::new(id.clone()), name.into())
}

/// which touches a hosted leaf takes
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PostRowLeafHitPolicy {
    #[default]
    None,
    Everywhere,
    Rects,
}

/// one leaf over a native row part
/// plain gpui content so nothing to swap, unmount frees the leaf state and any player in it
pub struct PostRowHostedLeafView<Leaf: IntoElement + Clone + 'static> {
    pub hit_policy: PostRowLeafHitPolicy,
    pub interactive_rects: Vec<Bounds<Pixels>>,
    pub is_hidden: bool,
    pub alpha: f32,
    hosted_view: Option<(Leaf, PostListHostedEnvironment)>,
    bounds_size: Rc<Cell<Option<Size<Pixels>>>>,
}

impl<Leaf: IntoElement + Clone + 'static> PostRowHostedLeafView<Leaf> {
    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            hit_policy: PostRowLeafHitPolicy::None,
            interactive_rects: Vec::new(),
            is_hidden: false,
            alpha: 1.0,
            hosted_view: None,
            bounds_size: Rc::new(Cell::new(None)),
        }
    }

    pub fn is_mounted(&self) -> bool {
        self.hosted_view.is_some()
    }

    /// creates the hosted content, an already mounted leaf is updated in place
    pub fn mount(&mut self, leaf: Leaf, environment: &PostListHostedEnvironment, cx: &mut Context<Self>) {
        self.install(leaf, environment, cx);
    }

    /// re renders a mounted leaf with new inputs, keyed state inside survives
    pub fn update(&mut self, leaf: Leaf, environment: &PostListHostedEnvironment, cx: &mut Context<Self>) {
        if !self.is_mounted() {
            return;
        }
        self.install(leaf, environment, cx);
    }

    /// removes the hosted content, its state and any player are released
    pub fn unmount(&mut self, cx: &mut Context<Self>) {
        if self.hosted_view.take().is_some() {
            cx.notify();
        }
    }

    /// true when a touch at this local point would reach the leaf
    pub fn takes_touch(&self, at: Point<Pixels>) -> bool {
        if !self.is_mounted() || self.is_hidden || self.alpha <= 0.01 {
            return false;
        }
        match self.hit_policy {
            PostRowLeafHitPolicy::None => false,
            PostRowLeafHitPolicy::Everywhere => self.bounds_size.get().is_some_and(|size| {
                at.x >= px(0.0) && at.y >= px(0.0) && at.x < size.width && at.y < size.height
            }),
            PostRowLeafHitPolicy::Rects => self.interactive_rects.iter().any(|rect| rect.contains(&at)),
        }
    }

    fn install(&mut self, leaf: Leaf, environment: &PostListHostedEnvironment, cx: &mut Context<Self>) {
        self.hosted_view = Some((leaf, environment.clone()));
        cx.notify();
    }
}

impl<Leaf: IntoElement + Clone + 'static> Render for PostRowHostedLeafView<Leaf> {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let hosted = self
            .hosted_view
            .as_ref()
            .filter(|_| !self.is_hidden)
            .map(|(leaf, environment)| leaf.clone().applying_post_list_hosted_environment(environment));
        let bounds_size = self.bounds_size.clone();
        let reader = measure(("post-row-hosted-leaf-frame", cx.entity_id().as_u64()), move |bounds, _, _| {
            bounds_size.set(Some(bounds.size));
        });
        match self.hit_policy {
            PostRowLeafHitPolicy::Everywhere => div()
                .absolute()
                .inset_0()
                .block_mouse_except_scroll()
                .opacity(self.alpha)
                .child(reader)
                .children(hosted)
                .into_any_element(),
            PostRowLeafHitPolicy::None | PostRowLeafHitPolicy::Rects => pass_through()
                .absolute()
                .inset_0()
                .opacity(self.alpha)
                .child(reader)
                .children(hosted)
                .into_any_element(),
        }
    }
}

/// a hidden tab keeps its cached frame so the leaf isnt drawn again and gets no disappear, so
/// the player pauses through its playback controller in the same turn the list gives up focus
fn hold_unless_focused(
    playback: &Entity<ShortsPlaybackController>,
    token: &str,
    focus: &Entity<ShortsFeedFocusCoordinator>,
    cx: &mut App,
) {
    let held = focus.read(cx).active_token() != Some(token);
    playback.update(cx, |playback, cx| playback.set_held(held, cx));
}

/// what a leaf body reads from the shared stores, the leaf
/// keeps the last copy and draws again only on change
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct PostRowLeafInputs {
    is_focused: bool,
    is_viewer_presented: bool,
    inline_playback_enabled: bool,
    is_muted: bool,
    is_foreground: bool,
}

/// the stores a leaf body reads, held so an observer can read them outside a render
struct PostRowLeafSources {
    shorts_store: Entity<ShortsServicesStore>,
    focus: Entity<ShortsFeedFocusCoordinator>,
    foreground_gate: Entity<RiseMediaForegroundGate>,
    media_viewer: Option<Entity<MediaViewerPresentationState>>,
}

impl PostRowLeafSources {
    fn inputs(&self, focus_token: &str, cx: &App) -> PostRowLeafInputs {
        let shorts_store = self.shorts_store.read(cx);
        PostRowLeafInputs {
            is_focused: self.focus.read(cx).active_token() == Some(focus_token),
            is_viewer_presented: self.media_viewer.as_ref().is_some_and(|viewer| viewer.read(cx).is_presented()),
            inline_playback_enabled: shorts_store.inline_feed_playback_enabled(),
            is_muted: shorts_store.feed_audio_muted,
            is_foreground: self.foreground_gate.read(cx).is_foreground(),
        }
    }
}

struct PostRowInlineVideoLeafState {
    playback: Entity<ShortsPlaybackController>,
    focus_token: String,
    sources: PostRowLeafSources,
    inputs: PostRowLeafInputs,
    _subscriptions: Vec<Subscription>,
}

impl PostRowInlineVideoLeafState {
    fn new(focus_token: String, cx: &mut Context<Self>) -> Self {
        let sources = PostRowLeafSources {
            shorts_store: ShortsServicesStore::shared(cx),
            focus: ShortsFeedFocusCoordinator::shared(cx),
            foreground_gate: RiseMediaForegroundGate::shared(cx),
            media_viewer: Some(media_viewer_presentation(cx)),
        };
        let inputs = sources.inputs(&focus_token, cx);
        let mut subscriptions = vec![
            cx.observe(&sources.shorts_store, |this: &mut Self, _, cx| this.inputs_did_change(cx)),
            cx.observe(&sources.focus, |this: &mut Self, focus, cx| {
                hold_unless_focused(&this.playback, &this.focus_token, &focus, cx);
                this.inputs_did_change(cx);
            }),
            cx.observe(&sources.foreground_gate, |this: &mut Self, _, cx| this.inputs_did_change(cx)),
        ];
        if let Some(media_viewer) = sources.media_viewer.as_ref() {
            subscriptions.push(cx.observe(media_viewer, |this: &mut Self, _, cx| this.inputs_did_change(cx)));
        }
        Self {
            playback: cx.new(|_| ShortsPlaybackController::new()),
            focus_token,
            sources,
            inputs,
            _subscriptions: subscriptions,
        }
    }

    fn inputs_did_change(&mut self, cx: &mut Context<Self>) {
        let inputs = self.sources.inputs(&self.focus_token, cx);
        if inputs != self.inputs {
            self.inputs = inputs;
            cx.notify();
        }
    }

    fn set_focus_token(&mut self, focus_token: String, cx: &App) {
        self.inputs = self.sources.inputs(&focus_token, cx);
        self.focus_token = focus_token;
    }
}

/// current video page of a native gallery, gated by an injected focus token
/// the list reports the page frame to the focus coordinator so there is no frame reporter here
#[derive(IntoElement, Clone)]
pub struct PostRowInlineVideoLeaf {
    id: ElementId,
    post: Rc<GetPostFeedResponse>,
    url: String,
    thumbnail: Option<String>,
    fills: bool,
    focus_token: String,
    shows_sound_button: bool,
}

impl PostRowInlineVideoLeaf {
    pub fn new(
        id: impl Into<ElementId>,
        post: Rc<GetPostFeedResponse>,
        url: impl Into<String>,
        thumbnail: Option<String>,
        fills: bool,
        focus_token: impl Into<String>,
        shows_sound_button: bool,
    ) -> Self {
        Self {
            id: id.into(),
            post,
            url: url.into(),
            thumbnail,
            fills,
            focus_token: focus_token.into(),
            shows_sound_button,
        }
    }

    /// same owner key as the inline video page, the photo pages ambient driver owns post.id
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

impl RenderOnce for PostRowInlineVideoLeaf {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let token = self.focus_token.clone();
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, cx| PostRowInlineVideoLeafState::new(token, cx));
        if state.read(cx).focus_token != self.focus_token {
            let token = self.focus_token.clone();
            state.update(cx, |state, cx| state.set_focus_token(token, cx));
        }
        let (playback, inputs) = {
            let state = state.read(cx);
            (state.playback.clone(), state.inputs)
        };
        let mounts_player = inputs.is_focused && !inputs.is_viewer_presented && !self.post.shows_upload_overlay();
        let is_active = mounts_player && inputs.inline_playback_enabled && inputs.is_foreground;

        let player = mounts_player.then(|| {
            let is_own = PostMediaCachePolicy::is_own_post(&self.post, cx);
            let is_muted = inputs.is_muted;
            let player = ShortsFeedPlayer::new(
                child_id(&self.id, "player"),
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
            pass_through().absolute().inset_0().overflow_hidden().child(player)
        });

        div()
            .relative()
            .size_full()
            .children(player)
            .when(!is_active, |leaf| leaf.child(PostFeedVideoGlyph))
            .when(self.shows_sound_button, |leaf| {
                leaf.child(
                    div()
                        .absolute()
                        .right(px(PostRowMediaMetrics::OVERLAY_INSET))
                        .bottom(px(PostRowMediaMetrics::OVERLAY_INSET))
                        .child(FeedSoundButton::new()),
                )
            })
    }
}

struct PostRowRiffCardLeafState {
    playback: Entity<ShortsPlaybackController>,
    focus_token: String,
    sources: PostRowLeafSources,
    inputs: PostRowLeafInputs,
    ended: bool,
    end_card: Presence<()>,
    _subscriptions: Vec<Subscription>,
}

impl PostRowRiffCardLeafState {
    fn new(focus_token: String, cx: &mut Context<Self>) -> Self {
        let sources = PostRowLeafSources {
            shorts_store: ShortsServicesStore::shared(cx),
            focus: ShortsFeedFocusCoordinator::shared(cx),
            foreground_gate: RiseMediaForegroundGate::shared(cx),
            media_viewer: None,
        };
        let inputs = sources.inputs(&focus_token, cx);
        let subscriptions = vec![
            cx.observe(&sources.shorts_store, |this: &mut Self, _, cx| this.inputs_did_change(cx)),
            cx.observe(&sources.focus, |this: &mut Self, focus, cx| {
                hold_unless_focused(&this.playback, &this.focus_token, &focus, cx);
                this.inputs_did_change(cx);
            }),
        ];
        Self {
            playback: cx.new(|_| ShortsPlaybackController::new()),
            focus_token,
            sources,
            inputs,
            ended: false,
            end_card: Presence::new(None).with_transition(Transition::opacity()),
            _subscriptions: subscriptions,
        }
    }

    fn inputs_did_change(&mut self, cx: &mut Context<Self>) {
        let inputs = self.sources.inputs(&self.focus_token, cx);
        if inputs != self.inputs {
            self.inputs = inputs;
            cx.notify();
        }
    }

    fn set_focus_token(&mut self, focus_token: String, cx: &App) {
        self.inputs = self.sources.inputs(&focus_token, cx);
        self.focus_token = focus_token;
    }

    fn handle_ended(&mut self, cx: &mut Context<Self>) {
        self.ended = true;
        self.end_card.set_in(Some(()), ease_in_out(0.25), RiseClock::monotonic(), cx);
        cx.notify();
    }

    fn replay(&mut self, cx: &mut Context<Self>) {
        self.playback.update(cx, |playback, cx| playback.seek(0.0, true, cx));
        self.ended = false;
        self.end_card.set_in(None, ease_in_out(0.2), RiseClock::monotonic(), cx);
        cx.notify();
    }
}

/// inline riff card over the native poster and blur fill
/// with player and tap to viewer and sound switch and end card
/// box, poster and blur and focus frames belong to the row
#[derive(IntoElement, Clone)]
pub struct PostRowRiffCardLeaf {
    id: ElementId,
    post: Rc<GetPostFeedResponse>,
    focus_token: String,
    fills: bool,
    on_open: Rc<dyn Fn(&mut App)>,
}

impl PostRowRiffCardLeaf {
    pub fn new(
        id: impl Into<ElementId>,
        post: Rc<GetPostFeedResponse>,
        focus_token: impl Into<String>,
        fills: bool,
        on_open: impl Fn(&mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), post, focus_token: focus_token.into(), fills, on_open: Rc::new(on_open) }
    }

    fn video_url(&self) -> Option<String> {
        self.post.resolved_video_url().filter(|url| !url.is_empty())
    }

    fn open_more_shorts(nav_store: Option<&Entity<NavigationStore>>, cx: &mut App) {
        let Some(nav_store) = nav_store else {
            return;
        };
        nav_store.update(cx, |store, cx| {
            store.pop_to_root(cx);
            store.switch_tab(MainTab::Shorts, cx);
        });
    }

    fn end_card(
        &self,
        state: &Entity<PostRowRiffCardLeafState>,
        nav_store: Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) -> gpui::Stateful<gpui::Div> {
        let material_base = ThemeStore::shared(cx).read(cx).current_theme().bg._100;
        let on_open = self.on_open.clone();
        let replay_state = state.clone();
        div()
            .id(child_id(&self.id, "end-card"))
            .absolute()
            .inset_0()
            .block_mouse_except_scroll()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(16.0))
            .bg(alpha(material_base, Material::UltraThin.painted_opacity()))
            .cursor_pointer()
            .on_click(move |_, _, cx| on_open(cx))
            .child(
                div().id(child_id(&self.id, "more-target")).block_mouse_except_scroll().child(
                    plain(child_id(&self.id, "more"))
                        .on_press(move |_, cx| Self::open_more_shorts(nav_store.as_ref(), cx))
                        .flex()
                        .items_center()
                        .gap(px(10.0))
                        .px(px(22.0))
                        .py(px(13.0))
                        .capsule()
                        .bg(white())
                        .children(IconUi::sized("play.rectangle.fill", px(18.0), black(), true))
                        .child(
                            MainText::new(tr_args("shorts_watch_more", &[FormatArg::from(tr("riffs"))]))
                                .px(16.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(black()),
                        ),
                ),
            )
            .child(
                div().id(child_id(&self.id, "again-target")).block_mouse_except_scroll().child(
                    plain(child_id(&self.id, "again"))
                        .on_press(move |_, cx| replay_state.update(cx, |state, cx| state.replay(cx)))
                        .shadow(vec![gpui::BoxShadow {
                            color: alpha(black(), 0.4),
                            offset: gpui::point(px(0.0), px(1.0)),
                            blur_radius: px(3.0),
                            spread_radius: px(0.0),
                            inset: false,
                        }])
                        .child(
                            MainText::new(tr("shorts_watch_again"))
                                .px(16.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(white()),
                        ),
                ),
            )
    }
}

impl RenderOnce for PostRowRiffCardLeaf {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let token = self.focus_token.clone();
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, cx| PostRowRiffCardLeafState::new(token, cx));
        if state.read(cx).focus_token != self.focus_token {
            let token = self.focus_token.clone();
            state.update(cx, |state, cx| state.set_focus_token(token, cx));
        }
        let nav_store = env::<NavigationStoreKey>(cx);
        let shorts_zoom_namespace = env::<ShortsZoomNamespaceKey>(cx);
        let (ended, playback, inputs) = {
            let state = state.read(cx);
            (state.ended, state.playback.clone(), state.inputs)
        };
        let video_url = self.video_url();
        let should_play = video_url.is_some()
            && inputs.is_focused
            && !ended
            && !self.post.shows_upload_overlay()
            && inputs.inline_playback_enabled;
        let is_own_post = PostMediaCachePolicy::is_own_post(&self.post, cx);
        let is_muted = inputs.is_muted;
        let has_video = video_url.is_some();

        let weak_state = state.downgrade();
        let player = video_url.map(|url| {
            pass_through().absolute().inset_0().overflow_hidden().child(
                ShortsFeedPlayer::new(child_id(&self.id, "player"), url, self.post.video_thumbnail.clone(), should_play)
                    .is_muted(is_muted)
                    .paused(ended)
                    .playback(Some(playback))
                    .gravity_override(Some(if self.fills {
                        AVLayerVideoGravity::ResizeAspectFill
                    } else {
                        AVLayerVideoGravity::ResizeAspect
                    }))
                    .draws_opaque_background(false)
                    .loops(false)
                    .on_ended(move |_, cx| {
                        let _ = weak_state.update(cx, |state, cx| state.handle_ended(cx));
                    })
                    .music_overlay(self.post.shorts_music_overlay())
                    .persists_post_media_to_disk(is_own_post)
                    .post_image_cache_policy(PostMediaCachePolicy::image_policy(is_own_post)),
            )
        });

        let tap_open = self.on_open.clone();
        let tap_catcher = (!ended).then(|| {
            div()
                .id(child_id(&self.id, "tap"))
                .absolute()
                .inset_0()
                .cursor_pointer()
                .on_click(move |_, _, cx| tap_open(cx))
        });
        let sound_button = (has_video && !ended).then(|| {
            div()
                .absolute()
                .right(px(PostRowMediaMetrics::OVERLAY_INSET))
                .bottom(px(PostRowMediaMetrics::OVERLAY_INSET))
                .child(FeedSoundButton::new())
        });
        let end_card = self.end_card(&state, nav_store, cx);
        let end_card = state.read(cx).end_card.render_transitioned(RiseClock::monotonic(), window, |_| end_card);

        let card = div()
            .absolute()
            .inset_0()
            .children(player)
            .children(tap_catcher)
            .children(sound_button)
            .children(end_card);
        match shorts_zoom_namespace {
            Some(namespace) => {
                let route = RootRoute::ShortViewer { post: (*self.post).clone(), origin: None, sound_track_id: None };
                ShortsZoomSource::new(route, Some(namespace)).body(card)
            }
            None => card.into_any_element(),
        }
    }
}

/// viewers repost sticker in the corner row
/// it lays out at its own size and reports it, the row aligns
/// its bottom to the corner baseline and builds the hit rect
#[derive(IntoElement, Clone)]
pub struct PostRowRepostStickerLeaf {
    id: ElementId,
    post: Rc<GetPostFeedResponse>,
    on_size: Rc<dyn Fn(Size<Pixels>, &mut App)>,
}

impl PostRowRepostStickerLeaf {
    pub fn new(
        id: impl Into<ElementId>,
        post: Rc<GetPostFeedResponse>,
        on_size: impl Fn(Size<Pixels>, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), post, on_size: Rc::new(on_size) }
    }
}

impl RenderOnce for PostRowRepostStickerLeaf {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let last_size: Entity<Option<Size<Pixels>>> =
            window.use_keyed_state(child_id(&self.id, "size"), cx, |_, _| None);
        let on_size = self.on_size;
        let reader = measure(child_id(&self.id, "reader"), move |bounds, _, cx| {
            if *last_size.read(cx) == Some(bounds.size) {
                return;
            }
            last_size.update(cx, |last_size, _| *last_size = Some(bounds.size));
            on_size(bounds.size, cx);
        });
        div().relative().flex_none().w(px(RepostStickerStyle::Feed.bubble_width())).h_full().child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .flex()
                .flex_col()
                .items_start()
                .child(RepostOverlay::new(self.post).style(RepostStickerStyle::Feed))
                .child(reader),
        )
    }
}

/// resolved custom emoji of the premium badge next to the author name, over the native poster
#[derive(IntoElement, Clone)]
pub struct PostRowPremiumGlyphLeaf {
    pub item: StickerItem,
}

impl PostRowPremiumGlyphLeaf {
    pub fn new(item: StickerItem) -> Self {
        Self { item }
    }
}

impl RenderOnce for PostRowPremiumGlyphLeaf {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        pass_through().child(CustomEmojiGlyph::new(
            self.item,
            PostRowMetrics::BADGE_SIZE * PremiumBadgeMetrics::CUSTOM_GLYPH_CANVAS_RATIO,
        ))
    }
}

/// plays the post track without a frame reader, while the row holds focus
/// inline playback is allowed, sound is on and the app is in front
pub struct PostRowAmbientMusicDriver {
    post: Option<GetPostFeedResponse>,
    token: Option<PostListPlaybackToken>,
    focus: Option<Weak<dyn PostListFocusReporting>>,
    frame_view: Option<Weak<PostListFocusFrameView>>,
    is_active: bool,
    is_suppressed: bool,
    tracked_token: Option<PostListPlaybackToken>,
    playing_post_id: Option<String>,
    observers: Vec<Subscription>,
    _release: Subscription,
}

impl PostRowAmbientMusicDriver {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            post: None,
            token: None,
            focus: None,
            frame_view: None,
            is_active: false,
            is_suppressed: false,
            tracked_token: None,
            playing_post_id: None,
            observers: Vec::new(),
            _release: cx.on_release(|this: &mut Self, cx: &mut App| {
                if let Some(playing_post_id) = this.playing_post_id.take() {
                    PostMusicPlaybackController::shared(cx)
                        .update(cx, |controller, cx| controller.stop(&playing_post_id, cx));
                }
            }),
        }
    }

    pub fn configure(
        &mut self,
        post: &GetPostFeedResponse,
        token: PostListPlaybackToken,
        focus: Rc<dyn PostListFocusReporting>,
        frame_view: Rc<PostListFocusFrameView>,
        cx: &mut Context<Self>,
    ) {
        let is_retargeted =
            self.post.as_ref().is_some_and(|current| current.id != post.id || self.token.as_ref() != Some(&token));
        if is_retargeted {
            self.end_tracking(cx);
            self.stop_playback(cx);
        }
        if self.post.as_ref() != Some(post) {
            self.post = Some(post.clone());
        }
        self.token = Some(token);
        self.focus = Some(Rc::downgrade(&focus));
        self.frame_view = Some(Rc::downgrade(&frame_view));
        self.sync(cx);
    }

    /// row visible and not fast scrolling
    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.is_active == active {
            return;
        }
        self.is_active = active;
        self.sync(cx);
    }

    /// carousel shows its video page, which owns the row focus and the track
    pub fn set_suppressed(&mut self, suppressed: bool, cx: &mut Context<Self>) {
        if self.is_suppressed == suppressed {
            return;
        }
        self.is_suppressed = suppressed;
        self.sync(cx);
    }

    pub fn stop(&mut self, cx: &mut Context<Self>) {
        self.is_active = false;
        self.end_tracking(cx);
        self.stop_playback(cx);
        self.post = None;
        self.token = None;
        self.frame_view = None;
    }

    fn focus(&self) -> Option<Rc<dyn PostListFocusReporting>> {
        self.focus.as_ref().and_then(Weak::upgrade)
    }

    fn frame_view(&self) -> Option<Rc<PostListFocusFrameView>> {
        self.frame_view.as_ref().and_then(Weak::upgrade)
    }

    fn wants_tracking(&self) -> bool {
        self.is_active
            && !self.is_suppressed
            && self.post.as_ref().is_some_and(|post| post.music_audio_url().is_some())
            && self.token.is_some()
            && self.focus().is_some()
            && self.frame_view().is_some()
    }

    fn should_play(&self, cx: &mut Context<Self>) -> bool {
        let (true, Some(token), Some(post)) = (self.tracked_token.is_some(), self.token.as_ref(), self.post.as_ref())
        else {
            return false;
        };
        if post.music_audio_url().is_none() {
            return false;
        }
        let raw_value = token.raw_value();
        let is_focused = ShortsFeedFocusCoordinator::shared(cx).read(cx).active_token() == Some(raw_value.as_str());
        let shorts_store = ShortsServicesStore::shared(cx);
        let shorts_store = shorts_store.read(cx);
        let allows_sound = shorts_store.inline_feed_playback_enabled() && !shorts_store.feed_audio_muted;
        is_focused && allows_sound && RiseMediaForegroundGate::shared(cx).read(cx).is_foreground()
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        if self.wants_tracking() {
            self.begin_tracking(cx);
        } else {
            self.end_tracking(cx);
        }
        self.evaluate(cx);
    }

    fn begin_tracking(&mut self, cx: &mut Context<Self>) {
        let (Some(token), Some(focus), Some(frame_view)) = (self.token.clone(), self.focus(), self.frame_view()) else {
            return;
        };
        if self.tracked_token.as_ref() != Some(&token) {
            if let Some(tracked_token) = self.tracked_token.as_ref() {
                focus.end_focus_tracking(tracked_token, cx);
            }
            focus.begin_focus_tracking(&token, &frame_view, cx);
            self.tracked_token = Some(token);
        }
        self.arm_if_needed(cx);
    }

    fn end_tracking(&mut self, cx: &mut Context<Self>) {
        self.observers.clear();
        if let (Some(tracked_token), Some(focus)) = (self.tracked_token.as_ref(), self.focus()) {
            focus.end_focus_tracking(tracked_token, cx);
        }
        self.tracked_token = None;
    }

    fn evaluate(&mut self, cx: &mut Context<Self>) {
        if self.should_play(cx) {
            if self.playing_post_id.is_none() {
                self.start_playback(cx);
            }
        } else {
            self.stop_playback(cx);
        }
    }

    fn foreground_did_change(&mut self, cx: &mut Context<Self>) {
        if self.tracked_token.is_none() {
            return;
        }
        if self.should_play(cx) {
            self.start_playback(cx);
        } else {
            self.stop_playback(cx);
        }
    }

    fn start_playback(&mut self, cx: &mut Context<Self>) {
        let Some(post) = self.post.as_ref() else {
            return;
        };
        let Some(url) = post.music_audio_url() else {
            return;
        };
        PostMusicPlaybackController::shared(cx).update(cx, |controller, cx| {
            controller.play_ambient(
                &post.id,
                &url,
                post.track_id.as_deref(),
                post.music_segment_start_ms(),
                post.music_segment_end_ms(),
                true,
                true,
                false,
                cx,
            );
        });
        self.playing_post_id = Some(post.id.clone());
    }

    fn stop_playback(&mut self, cx: &mut Context<Self>) {
        let Some(playing_post_id) = self.playing_post_id.take() else {
            return;
        };
        PostMusicPlaybackController::shared(cx).update(cx, |controller, cx| controller.stop(&playing_post_id, cx));
    }

    fn arm_if_needed(&mut self, cx: &mut Context<Self>) {
        if !self.observers.is_empty() {
            return;
        }
        let focus = ShortsFeedFocusCoordinator::shared(cx);
        let shorts_store = ShortsServicesStore::shared(cx);
        let foreground_gate = RiseMediaForegroundGate::shared(cx);
        self.observers = vec![
            cx.observe(&focus, |this: &mut Self, _, cx| this.observation_did_fire(cx)),
            cx.observe(&shorts_store, |this: &mut Self, _, cx| this.observation_did_fire(cx)),
            cx.observe(&foreground_gate, |this: &mut Self, _, cx| {
                this.foreground_did_change(cx);
                this.observation_did_fire(cx);
            }),
        ];
    }

    fn observation_did_fire(&mut self, cx: &mut Context<Self>) {
        if self.tracked_token.is_none() {
            return;
        }
        self.evaluate(cx);
    }
}
