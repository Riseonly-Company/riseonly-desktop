use std::collections::HashMap;
use std::rc::Rc;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Bounds, BoxShadow, Context, ElementId, Entity, FontWeight, ImageSource, InteractiveElement,
    IntoElement, ObjectFit, ParentElement, Pixels, RenderOnce, StatefulInteractiveElement, Styled, StyledImage,
    Subscription, Window, black, div, img, point, px, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_media::video::av_player::AVLayerVideoGravity;
use rise_theme::{ThemeStore, alpha};
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::lifecycle::LifecycleModifiers;
use rise_ui::swiftui::material::Material;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::ease_in_out;
use rise_ui::swiftui::progress_view::progress_view;
use rise_ui::swiftui::shapes::{SwiftUiShapes, pass_through};
use rise_ui::swiftui::transition::{Presence, Transition};

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::{MainTab, RootRoute};
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::ui::clever_image_disk_cache::CleverImageBitmapLoader;
use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::core::ui::media_blur_fill_ui::MediaBlurFillUi;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::shared::post_media_box::PostMediaBox;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::shorts::components::feed_sound_button::FeedSoundButton;
use crate::modules::shorts::components::shorts_feed_player::{ShortsFeedPlayer, ShortsPlaybackController};
use crate::modules::shorts::components::shorts_media_prefetch::ShortsMediaPrefetchPolicy;
use crate::modules::shorts::components::shorts_zoom_namespace::{ShortsZoomNamespaceKey, ShortsZoomSource};
use crate::modules::shorts::stores::shorts::shorts_services::shorts_services::ShortsServicesStore;

/// which inline cell holds focus,
/// keyed by cell instance not post id since one reel can be mounted in two lists
#[derive(Default)]
pub struct ShortsFeedFocusCoordinator {
    active_token: Option<String>,
    frames: HashMap<String, Bounds<Pixels>>,
}

impl ShortsFeedFocusCoordinator {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self::default())
    }

    pub fn active_token(&self) -> Option<&str> {
        self.active_token.as_deref()
    }

    /// reports the frame in window coordinates
    pub fn report(&mut self, token: &str, frame: Bounds<Pixels>, cx: &mut Context<Self>) {
        if f32::from(frame.size.height) <= 1.0 {
            return;
        }
        self.frames.insert(token.to_owned(), frame);
        self.recompute(cx);
    }

    pub fn remove(&mut self, token: &str, cx: &mut Context<Self>) {
        if self.frames.remove(token).is_none() {
            return;
        }
        self.recompute(cx);
    }

    fn recompute(&mut self, cx: &mut Context<Self>) {
        let screen_height = f32::from(PostMediaBox::viewport().height);
        let focus_y = screen_height * 0.5;
        let mut best: Option<&String> = None;
        let mut best_distance = f32::MAX;
        for (token, frame) in &self.frames {
            let min_y = f32::from(frame.origin.y);
            let height = f32::from(frame.size.height);
            let max_y = min_y + height;
            let visible_height = max_y.min(screen_height) - min_y.max(0.0);
            if visible_height < height * 0.34 {
                continue;
            }
            let distance = (min_y + height * 0.5 - focus_y).abs();
            if distance < best_distance {
                best_distance = distance;
                best = Some(token);
            }
        }
        if best != self.active_token.as_ref() {
            self.active_token = best.cloned();
            cx.notify();
        }
    }
}

#[derive(IntoElement)]
pub struct ShortsInlineFeedCell {
    post: Rc<GetPostFeedResponse>,
}

impl ShortsInlineFeedCell {
    pub fn new(post: impl Into<Rc<GetPostFeedResponse>>) -> Self {
        Self { post: post.into() }
    }

    fn element_id(&self, suffix: &str) -> ElementId {
        ElementId::Name(format!("shorts-inline-{}-{suffix}", self.post.id).into())
    }

    fn server_aspect(post: &GetPostFeedResponse) -> Option<f32> {
        PostMediaBox::aspect(post.video_size.as_ref()).or_else(|| PostMediaBox::aspect(post.image_sizes.first()))
    }

    fn poster_memory_aspect(post: &GetPostFeedResponse, cx: &mut App) -> Option<f32> {
        let thumbnail = trimming_whitespaces_and_newlines(post.video_thumbnail.as_deref().unwrap_or(""));
        if thumbnail.is_empty() {
            return None;
        }
        let is_own = PostMediaCachePolicy::is_own_post(post, cx);
        let image = CleverImageBitmapLoader::memory_image(
            thumbnail,
            is_own,
            SaiAccountCacheScope::active_user_id().as_deref(),
            SaiAccountCacheScope::active_epoch().as_deref(),
            Some(ShortsMediaPrefetchPolicy::POSTER_PIXEL_DIMENSION),
            PostMediaCachePolicy::image_policy(is_own),
            None,
            true,
        )?;
        let size = image.size(0);
        let (width, height) = (size.width.0, size.height.0);
        if width <= 0 || height <= 0 {
            return None;
        }
        Some(width as f32 / height as f32)
    }

    fn resolve_media_layout(post: &GetPostFeedResponse, asset_aspect: Option<f32>, cx: &mut App) -> MediaLayout {
        let natural =
            asset_aspect.or_else(|| Self::poster_memory_aspect(post, cx)).or_else(|| Self::server_aspect(post));
        let box_aspect = PostMediaBox::display_aspect(natural);
        MediaLayout {
            box_aspect,
            fills_box: PostMediaBox::fills(natural, box_aspect),
        }
    }

    fn blurred_fill_source(post: &GetPostFeedResponse, fills_box: bool) -> Option<String> {
        if fills_box {
            return None;
        }
        let thumbnail = trimming_whitespaces_and_newlines(post.video_thumbnail.as_deref().unwrap_or(""));
        (!thumbnail.is_empty()).then(|| thumbnail.to_owned())
    }

    fn loader_status_text(post: &GetPostFeedResponse) -> String {
        if post.is_compressing {
            return format!("{} {}%", tr("chat_media_upload_processing"), post.compression_progress as i64);
        }
        if post.upload_progress >= 99.5 {
            return tr("chat_media_upload_sending");
        }
        if post.upload_progress > 0.0 {
            return format!("{} {}%", tr("shorts_uploading"), post.upload_progress as i64);
        }
        tr("shorts_uploading")
    }

    fn loader_thumbnail(post: &GetPostFeedResponse, cx: &mut App) -> gpui::AnyElement {
        let local_path = post
            .local_preview_url
            .as_deref()
            .and_then(Url::from_string)
            .filter(Url::is_file_url)
            .and_then(|url| url.to_file_path())
            .filter(|path| path.exists());
        if let Some(path) = local_path {
            return img(ImageSource::from(path)).size_full().object_fit(ObjectFit::Cover).into_any_element();
        }
        let thumbnail = trimming_whitespaces_and_newlines(post.video_thumbnail.as_deref().unwrap_or(""));
        if thumbnail.is_empty() {
            return div().size_full().bg(black()).into_any_element();
        }
        let is_own = PostMediaCachePolicy::is_own_post(post, cx);
        CleverImageUi::new(thumbnail.to_owned())
            .content_mode(rise_ui::swiftui::content_mode::ContentMode::Fill)
            .corner_radius(0.0)
            .persist_to_disk(is_own)
            .cache_policy(PostMediaCachePolicy::image_policy(is_own))
            .into_any_element()
    }

    fn media_loader(post: &GetPostFeedResponse, cx: &mut App) -> impl IntoElement + use<> {
        let status = Self::loader_status_text(post);
        div()
            .absolute()
            .inset_0()
            .child(div().absolute().inset_0().overflow_hidden().child(Self::loader_thumbnail(post, cx)))
            .child(div().absolute().inset_0().bg(alpha(black(), 0.45)))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap(px(12.0))
                    .child(progress_view().tint(white()))
                    .child(MainText::new(status).px(14.0).font_weight(FontWeight::SEMIBOLD).color(white())),
            )
    }

    fn open_viewer(post: &GetPostFeedResponse, nav_store: Option<&Entity<NavigationStore>>, cx: &mut App) {
        let Some(nav_store) = nav_store else {
            return;
        };
        let route = RootRoute::ShortViewer { post: post.clone(), origin: None, sound_track_id: None };
        nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::default(), cx));
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
        state: &Entity<ShortsInlineFeedCellState>,
        nav_store: Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) -> gpui::Stateful<gpui::Div> {
        let material_base = ThemeStore::shared(cx).read(cx).current_theme().bg._100;
        let open_post = self.post.clone();
        let open_nav = nav_store.clone();
        let more_nav = nav_store;
        let replay_state = state.clone();
        div()
            .id(self.element_id("end-card"))
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
            .on_click(move |_, _, cx| Self::open_viewer(&open_post, open_nav.as_ref(), cx))
            .child(
                div().id(self.element_id("more-target")).block_mouse_except_scroll().child(
                    plain(self.element_id("more"))
                        .on_press(move |_, cx| Self::open_more_shorts(more_nav.as_ref(), cx))
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
                div().id(self.element_id("again-target")).block_mouse_except_scroll().child(
                    plain(self.element_id("again"))
                        .on_press(move |_, cx| replay_state.update(cx, |state, cx| state.replay(cx)))
                        .shadow(vec![BoxShadow {
                            color: alpha(black(), 0.4),
                            offset: point(px(0.0), px(1.0)),
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

struct MediaLayout {
    box_aspect: f32,
    fills_box: bool,
}

struct ShortsInlineFeedCellState {
    focus_token: String,
    playback: Entity<ShortsPlaybackController>,
    ended: bool,
    end_card: Presence<()>,
    asset_aspect: Option<f32>,
    _subscriptions: Vec<Subscription>,
}

impl ShortsInlineFeedCellState {
    fn new(cx: &mut Context<Self>) -> Self {
        let shorts_store = ShortsServicesStore::shared(cx);
        let focus = ShortsFeedFocusCoordinator::shared(cx);
        Self {
            focus_token: Uuid::new_v4().uuid_string(),
            playback: cx.new(|_| ShortsPlaybackController::new()),
            ended: false,
            end_card: Presence::new(None).with_transition(Transition::opacity()),
            asset_aspect: None,
            _subscriptions: vec![
                cx.observe(&shorts_store, |_, _, cx| cx.notify()),
                cx.observe(&focus, |_, _, cx| cx.notify()),
            ],
        }
    }

    /// the only row height fix after display and a one off,
    /// the file tells the truth when the server frame shape is wrong
    /// no animation or neighbour rows jump under the pointer
    fn adopt(&mut self, aspect: f32, from_poster: bool, cx: &mut Context<Self>) {
        if !aspect.is_finite() || aspect <= 0.0 {
            return;
        }
        if from_poster && self.asset_aspect.is_some() {
            return;
        }
        if self.asset_aspect.is_some_and(|current| (current - aspect).abs() <= 0.001) {
            return;
        }
        self.asset_aspect = Some(aspect);
        cx.notify();
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

impl RenderOnce for ShortsInlineFeedCell {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(
            self.element_id("state"),
            cx,
            |_, cx: &mut Context<ShortsInlineFeedCellState>| ShortsInlineFeedCellState::new(cx),
        );
        let focus = ShortsFeedFocusCoordinator::shared(cx);
        let shorts_store = ShortsServicesStore::shared(cx);
        let nav_store = env::<NavigationStoreKey>(cx);
        let zoom_namespace = env::<ShortsZoomNamespaceKey>(cx);

        let (focus_token, playback, ended, asset_aspect) = {
            let state = state.read(cx);
            (state.focus_token.clone(), state.playback.clone(), state.ended, state.asset_aspect)
        };
        let post = self.post.clone();
        let layout = Self::resolve_media_layout(&post, asset_aspect, cx);
        let is_own_post = PostMediaCachePolicy::is_own_post(&post, cx);
        let image_policy = PostMediaCachePolicy::image_policy(is_own_post);
        let fills_box = layout.fills_box;

        let is_focused = focus.read(cx).active_token() == Some(focus_token.as_str());
        let video_url = post.resolved_video_url().filter(|url| !url.is_empty());
        let has_video = video_url.is_some();
        let shows_loader = post.shows_upload_overlay();
        let (feed_audio_muted, playback_enabled) = {
            let store = shorts_store.read(cx);
            (store.feed_audio_muted, store.inline_feed_playback_enabled())
        };
        let should_play = has_video && is_focused && !ended && !shows_loader && playback_enabled;

        let backdrop = Self::blurred_fill_source(&post, fills_box)
            .map(|source| MediaBlurFillUi::new(source).persist_to_disk(is_own_post).cache_policy(image_policy));

        let media = if shows_loader {
            Self::media_loader(&post, cx).into_any_element()
        } else {
            let player = video_url.map(|url| {
                let aspect_state = state.downgrade();
                let poster_state = state.downgrade();
                let ended_state = state.downgrade();
                pass_through().absolute().inset_0().overflow_hidden().child(
                    ShortsFeedPlayer::new(self.element_id("player"), url, post.video_thumbnail.clone(), should_play)
                        .is_muted(feed_audio_muted)
                        .paused(ended)
                        .playback(Some(playback.clone()))
                        .gravity_override(Some(if fills_box {
                            AVLayerVideoGravity::ResizeAspectFill
                        } else {
                            AVLayerVideoGravity::ResizeAspect
                        }))
                        .draws_opaque_background(fills_box)
                        .loops(false)
                        .on_ended(move |_, cx| {
                            ended_state.update(cx, |state, cx| state.handle_ended(cx)).ok();
                        })
                        .on_aspect(move |aspect, _, cx| {
                            aspect_state.update(cx, |state, cx| state.adopt(aspect, false, cx)).ok();
                        })
                        .on_poster_aspect(move |aspect, _, cx| {
                            poster_state.update(cx, |state, cx| state.adopt(aspect, true, cx)).ok();
                        })
                        .music_overlay(post.shorts_music_overlay())
                        .persists_post_media_to_disk(is_own_post)
                        .post_image_cache_policy(image_policy),
                )
            });
            let tap_post = post.clone();
            let tap_nav = nav_store.clone();
            let tap_catcher = (!ended).then(|| {
                div()
                    .id(self.element_id("tap"))
                    .absolute()
                    .inset_0()
                    .cursor_pointer()
                    .on_click(move |_, _, cx| Self::open_viewer(&tap_post, tap_nav.as_ref(), cx))
            });
            let sound_button = (has_video && !ended)
                .then(|| div().absolute().right(px(12.0)).bottom(px(12.0)).child(FeedSoundButton::new()));
            let end_card = self.end_card(&state, nav_store.clone(), cx);
            let end_card = state.read(cx).end_card.render_transitioned(RiseClock::monotonic(), window, |_| end_card);
            div()
                .absolute()
                .inset_0()
                .children(player)
                .children(tap_catcher)
                .children(sound_button)
                .children(end_card)
                .into_any_element()
        };

        let report_focus = focus.clone();
        let report_token = focus_token.clone();
        let focus_reporter = measure(self.element_id("focus"), move |frame: Bounds<Pixels>, _, cx| {
            report_focus.update(cx, |focus, cx| focus.report(&report_token, frame, cx));
        });

        let cell = div()
            .relative()
            .w_full()
            .aspect_ratio(layout.box_aspect)
            .overflow_hidden()
            .bg(black())
            .child(focus_reporter)
            .when_some(backdrop, |cell, backdrop| cell.child(div().absolute().inset_0().child(backdrop)))
            .child(media)
            .on_disappear(self.element_id("disappear"), move |_, cx| {
                focus.update(cx, |focus, cx| focus.remove(&focus_token, cx));
            });

        match zoom_namespace {
            Some(namespace) => {
                let route = RootRoute::ShortViewer { post: (*post).clone(), origin: None, sound_track_id: None };
                ShortsZoomSource::new(route, Some(namespace)).body(cell)
            }
            None => cell.into_any_element(),
        }
    }
}
