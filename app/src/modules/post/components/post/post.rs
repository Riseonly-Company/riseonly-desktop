use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Arc, Weak};
use std::time::Duration;

use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, Empty, Entity, FocusHandle, FontWeight, Hsla, ImageSource,
    KeyDownEvent, MouseButton, MouseDownEvent, ObjectFit, Pixels, Point, RenderImage, ScrollWheelEvent, SharedString, Stateful, Svg, Task,
    Window, img, relative, white,
};
use rise_canon::RiseGuideAnchor;
use rise_catalog::MusicTrackStatus as RiseMusicTrackStatus;
use rise_core::foundation::foundation_calendar::{Calendar, CalendarIdentifier, TimeZone};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::date_format::date_formatter::{DateFormatter, Style};
use rise_i18n::date_format::iso8601_date_formatter::{ISO8601DateFormatter, ISO8601DateFormatterOptions};
use rise_i18n::date_format::relative_date_time_formatter::{RelativeDateTimeFormatter, UnitsStyle};
use rise_i18n::language_store::Locale;
use rise_i18n::{AppDateFormatter, tr};
use rise_platform::haptics::{ImpactStyle, impact, selection_changed};
use rise_platform::pasteboard::UIPasteboard;
use rise_platform::wheel_gesture::{WHEEL_SETTLE_DELAY, WheelGestureTracker, WheelStep};
use rise_theme::AppTheme;
use rise_ui::IconUi;
use rise_ui::context_menu_ui::ContextMenuItem;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::{HoldContextMenuAction, HoldContextMenuAlignment};
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::main_text::MainText;
use rise_ui::official_badge::OfficialBadge;
use rise_ui::page_header_toolbar_button::PageHeaderIconButton;
use rise_ui::pressable_ui::PressableUi;
use rise_ui::swiftui::alpha_shadow::AlphaShadow;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::blur::blur_aware_image_source;
use rise_ui::swiftui::context_menu::ContextMenuModifiers;
use rise_ui::swiftui::motion::{ease_out, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::{Presence, Transition, TransitionModifiers};
use rise_ui::window_screen_frame_reader::{MediaCellScreenFrameRegistry, WindowScreenFrameReader};

use crate::app::router::in_app_link_browser::InAppLinkBrowser;
use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::media_viewer::media_viewer_image_cache::MediaViewerImageMemoryCache;
use crate::core::media_viewer::media_viewer_open::MediaViewerOpen;
use crate::core::media_viewer::media_viewer_presentation_state::{
    MediaViewerGalleryScope, MediaViewerPresentationHandlers, MediaViewerPresentationState, media_viewer_presentation,
};
use crate::core::media_viewer::media_viewer_types::{
    MediaViewerChrome, MediaViewerFrameKeys, MediaViewerGalleryItem, MediaViewerHeroSkin,
};
use crate::core::shared_element_transition::kernel::shared_element_types::SharedElementPart;
use crate::core::shared_element_transition::runtime::shared_element_lift_board::SharedElementLiftModifiers;
use crate::core::ui::clever_image_disk_cache::{CleverImage, CleverImageBitmapLoader, CleverImageWorkLane};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::ui::collaborator_avatars_ui::{CollaboratorAvatarsUi, CollaboratorNamesLineUi};
use crate::core::ui::formatted_text_ui::{FormattedTextAccent, FormattedTextOptions, FormattedTextStyle};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_presentation::{
    HoldContextMenuPresentationActions, HoldContextMenuPresentationOnAction,
};
use crate::core::ui::hold_context_menu_ui::hold_context_menu_tap_presentation::{
    HoldContextMenuTapPresentationModifiers, HoldContextMenuTapSource,
};
use crate::core::ui::media_link_areas_overlay::{MediaLinkAreasOverlay, MediaLinkZone};
use crate::core::ui::media_blur_fill_ui::MediaBlurFillUi;
use crate::core::ui::pager_back_swipe_yield::PagerBackSwipeYield;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::premium_name_color::PremiumNameColor;
use crate::core::ui::rich_text::rich_text_view::RichTextView;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::core::widgets::notifier::notifier_store::Notifier;
use crate::core::widgets::sheets::report_sheet::ReportSheetStore;
use crate::core::widgets::sheets::share_sheet::{SharePayload, ShareSheetStore};
use crate::modules::analytics::stores::engagement::engagement_actions::engagement_types::EngagementSurface;
use crate::modules::analytics::stores::engagement::engagement_interactions::engagement_interactions::EngagementInteractionsStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::RemoteMediaURL;
use crate::modules::guides::shared::guide_sandbox::GuideSandbox;
use crate::modules::moderation::stores::moderation_actions::moderation_types::ReportTarget;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::post::components::collaborators::post_collaborator_header_ui::{
    PostCoauthorInviteBanner, PostCollaboratorHeaderPresentation,
};
use crate::modules::post::components::collaborators::post_collaborator_menu::PostCollaboratorMenu;
use crate::modules::post::components::collaborators::post_collaborators_presenter::PostCollaboratorsPresenter;
use crate::modules::post::components::collaborators::post_tag_bubbles_overlay::{
    PostMediaCornerChrome, PostMediaTagsState, PostTagBubblesLayer, PostTagsChip,
};
use crate::modules::post::components::download::post_download_policy::PostMediaFocus;
use crate::modules::post::components::download::post_download_store::PostDownloadSheetStore;
use crate::modules::post::components::post::post_achievement_media::{
    PostAchievementCatalog, PostAchievementCopy, PostAchievementMedia,
};
use crate::modules::post::components::post::post_feed_inline_video_page::{
    PostCarouselAmbientMusicGate, PostFeedInlineVideoPage,
};
use crate::modules::post::components::post::post_caption_clamp_ui::PostCaptionClampUi;
use crate::modules::post::components::post::post_likes_summary::PostLikesSummary;
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::components::post::post_media_gallery_builder::PostMediaGalleryBuilder;
use crate::modules::post::components::post::post_music_plate_overlay::PostMusicPlateOverlay;
use crate::modules::post::components::post::post_music_strip::PostMusicStrip;
use crate::modules::post::components::post_visibility_reader::PostVisibilityReader;
use crate::modules::post::components::repost::repost_overlay::{
    RepostOverlay, RepostOverlayMetrics, RepostStickerStyle,
};
use crate::modules::post::engine::rise_post_engine_models::{
    RisePostContentKind, RisePostContentKindResolve, RisePostSearchKind, RisePostViewSurface,
};
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_row_header_view::PostRowDateText;
use crate::modules::post::shared::post_media_box::PostMediaBox;
use crate::modules::post::shared::post_media_slides::PostMediaSlide;
use crate::modules::post::stores::post_actions::post_types::{
    GetPostFeedResponse, PostMediaSize, PostModerationBanner,
};
use crate::modules::post::stores::post_interactions::post_interactions::{
    PostInteractionsStore, PostInteractionsStoreShareReceiptChannel,
};
use crate::modules::shorts::components::feed_sound_button::FeedSoundButton;
use crate::modules::shorts::components::shorts_inline_feed_cell::ShortsInlineFeedCell;
use crate::modules::shorts::components::shorts_riff_pager::RiffPageIndicator;
use crate::modules::sparks::components::sparks_icon_view::SparksBurstIcon;
use crate::modules::sparks::stores::sparks::sparks_actions::sparks_types::SparksTarget;
use crate::modules::sparks::stores::sparks::sparks_interactions::sparks_interactions::SparksInteractionsStore;

type PostCallback = Rc<dyn Fn(&mut Window, &mut App)>;

fn child_id(id: &ElementId, name: impl Into<SharedString>) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), name.into())
}

fn symbol(name: &str, side: f32, color: Hsla) -> Option<Svg> {
    IconUi::sized(name, px(side), color, name.ends_with(".fill"))
}

fn push_route(nav_store: &Option<Entity<NavigationStore>>, route: RootRoute, cx: &mut App) {
    if let Some(nav_store) = nav_store {
        nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
    }
}

/// the long press owns the right click inside a card with its own right click menu, so the
/// card menu must not open over it
fn owns_secondary_click(content: impl IntoElement) -> Div {
    div().on_mouse_down(MouseButton::Right, |_: &MouseDownEvent, _, cx| cx.stop_propagation()).child(content)
}

/// icon button hover hint on desktop
fn with_hint(content: impl IntoElement, id: ElementId, label: String) -> Stateful<Div> {
    div().id(id).tooltip_label(label).child(content)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PostDoubleTapSurface {
    Media,
    Copy,
}

struct PostState {
    like_bump_scale: Animated<f32>,
    favorite_bump_scale: Animated<f32>,
    double_tap_heart_surface: Presence<PostDoubleTapSurface>,
    double_tap_heart_target: Option<PostDoubleTapSurface>,
    tags_state: Entity<PostMediaTagsState>,
    like_settle: Option<Task<()>>,
    favorite_settle: Option<Task<()>>,
    heart_settle: Option<Task<()>>,
    is_caption_expanded: bool,
    is_options_menu_presented: bool,
    is_date_menu_presented: bool,
}

#[derive(IntoElement)]
pub struct Post {
    id: ElementId,
    post: Rc<GetPostFeedResponse>,
    is_preview: bool,
    feed_row: bool,
    view_surface: RisePostViewSurface,
    treat_as_own_post: bool,
    records_visibility: bool,
    carries_guide_anchors: bool,
    on_open_comments: PostCallback,
    on_avatar_tap: PostCallback,
    on_like: PostCallback,
    on_fav: PostCallback,
}

impl Post {
    pub fn new(id: impl Into<ElementId>, post: impl Into<Rc<GetPostFeedResponse>>) -> Self {
        Self {
            id: id.into(),
            post: post.into(),
            is_preview: false,
            feed_row: false,
            view_surface: RisePostViewSurface::Feed,
            treat_as_own_post: false,
            records_visibility: true,
            carries_guide_anchors: false,
            on_open_comments: Rc::new(|_, _| {}),
            on_avatar_tap: Rc::new(|_, _| {}),
            on_like: Rc::new(|_, _| {}),
            on_fav: Rc::new(|_, _| {}),
        }
    }

    pub fn is_preview(mut self, is_preview: bool) -> Self {
        self.is_preview = is_preview;
        self
    }

    pub fn feed_row(mut self, feed_row: bool) -> Self {
        self.feed_row = feed_row;
        self
    }

    pub fn view_surface(mut self, view_surface: RisePostViewSurface) -> Self {
        self.view_surface = view_surface;
        self
    }

    pub fn treat_as_own_post(mut self, treat_as_own_post: bool) -> Self {
        self.treat_as_own_post = treat_as_own_post;
        self
    }

    pub fn records_visibility(mut self, records_visibility: bool) -> Self {
        self.records_visibility = records_visibility;
        self
    }

    /// the post a page is about as opposed to the feed under it
    /// a guide points at its menu
    pub fn carries_guide_anchors(mut self, carries_guide_anchors: bool) -> Self {
        self.carries_guide_anchors = carries_guide_anchors;
        self
    }

    pub fn on_open_comments(mut self, on_open_comments: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_open_comments = Rc::new(on_open_comments);
        self
    }

    pub fn on_avatar_tap(mut self, on_avatar_tap: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_avatar_tap = Rc::new(on_avatar_tap);
        self
    }

    pub fn on_like(mut self, on_like: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_like = Rc::new(on_like);
        self
    }

    pub fn on_fav(mut self, on_fav: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_fav = Rc::new(on_fav);
        self
    }

    pub fn music_line_text(tm: &TrackModel) -> String {
        if tm.status == RiseMusicTrackStatus::OriginalAudio.wire() {
            return format!("{} \u{b7} {}", tm.artist, tr("original_audio_title"));
        }
        format!("{} - {}", tm.artist, tm.title)
    }

    /// a riff whose only media is one video keeps the inline player card
    /// any other post with media pages slides in one carousel
    fn uses_inline_short_card(&self) -> bool {
        self.post.is_riff && self.post.images.is_empty() && self.post.has_video_media()
    }

    fn has_written_content(&self) -> bool {
        !self.post.title.is_empty() || !self.post.content.is_empty() || !self.post.hashtags.is_empty()
    }

    fn resolved_content_kind(&self) -> RisePostContentKind {
        RisePostContentKind::resolve(
            Some(self.post.content_kind.as_str()),
            &self.post.images,
            self.post.video.as_deref(),
            self.post.is_short,
            self.post.is_image_reel,
            self.post.track_id.as_deref(),
        )
    }

    fn author_id(&self) -> String {
        self.post.author.id.clone().unwrap_or_else(|| self.post.author_id.clone())
    }

    fn viewer_is_author(&self, cx: &mut App) -> bool {
        self.treat_as_own_post || PostMediaCachePolicy::is_own_post(&self.post, cx)
    }

    fn fire_impression_if_needed(
        post: &GetPostFeedResponse,
        uses_inline_short_card: bool,
        records_visibility: bool,
        is_preview: bool,
        feed_row: bool,
        cx: &mut App,
    ) {
        if !records_visibility || is_preview || !feed_row {
            return;
        }
        let post_id = post.rise_view_target_post_id();
        let author_id = post.author.id.clone().unwrap_or_else(|| post.author_id.clone());
        let surface = if uses_inline_short_card { EngagementSurface::ShortsInline } else { EngagementSurface::Feed };
        let is_short = post.is_short;
        EngagementInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.impression(&post_id, &author_id, surface, -1, is_short, 500, cx);
        });
    }

    fn handle_double_tap_like(
        state: &Entity<PostState>,
        surface: PostDoubleTapSurface,
        is_liked: bool,
        is_preview: bool,
        on_like: &PostCallback,
        window: &mut Window,
        cx: &mut App,
    ) {
        if is_preview {
            return;
        }
        let _ = impact(ImpactStyle::Light);
        if !is_liked {
            on_like(window, cx);
            Self::bump_like(state, cx);
        }
        let now = RiseClock::monotonic();
        let reduce_motion = cx.reduce_motion();
        let weak = state.downgrade();
        let executor = cx.background_executor().clone();
        let settle = cx.spawn(async move |cx| {
            executor.timer(Duration::from_secs_f64(0.55)).await;
            let _ = weak.update(cx, |state, cx| {
                if state.double_tap_heart_target != Some(surface) {
                    return;
                }
                state.double_tap_heart_target = None;
                let reduce_motion = cx.reduce_motion();
                state.double_tap_heart_surface.set(None, ease_out(0.22), RiseClock::monotonic(), reduce_motion);
                cx.notify();
            });
        });
        state.update(cx, |state, cx| {
            state.double_tap_heart_target = Some(surface);
            state.double_tap_heart_surface.set(Some(surface), spring(0.28, 0.55), now, reduce_motion);
            state.heart_settle = Some(settle);
            cx.notify();
        });
    }

    fn handle_like_button(state: &Entity<PostState>, on_like: &PostCallback, window: &mut Window, cx: &mut App) {
        let _ = selection_changed();
        Self::bump_like(state, cx);
        on_like(window, cx);
    }

    fn handle_fav_button(state: &Entity<PostState>, on_fav: &PostCallback, window: &mut Window, cx: &mut App) {
        let _ = selection_changed();
        Self::bump_favorite(state, cx);
        on_fav(window, cx);
    }

    fn bump_like(state: &Entity<PostState>, cx: &mut App) {
        let now = RiseClock::monotonic();
        let reduce_motion = cx.reduce_motion();
        let weak = state.downgrade();
        let executor = cx.background_executor().clone();
        let settle = cx.spawn(async move |cx| {
            executor.timer(Duration::from_secs_f64(0.14)).await;
            let _ = weak.update(cx, |state, cx| {
                let reduce_motion = cx.reduce_motion();
                state.like_bump_scale.set(1.0, spring(0.26, 0.78), RiseClock::monotonic(), reduce_motion);
                cx.notify();
            });
        });
        state.update(cx, |state, cx| {
            state.like_bump_scale.set(1.12, spring(0.22, 0.62), now, reduce_motion);
            state.like_settle = Some(settle);
            cx.notify();
        });
    }

    fn bump_favorite(state: &Entity<PostState>, cx: &mut App) {
        let now = RiseClock::monotonic();
        let reduce_motion = cx.reduce_motion();
        let weak = state.downgrade();
        let executor = cx.background_executor().clone();
        let settle = cx.spawn(async move |cx| {
            executor.timer(Duration::from_secs_f64(0.14)).await;
            let _ = weak.update(cx, |state, cx| {
                let reduce_motion = cx.reduce_motion();
                state.favorite_bump_scale.set(1.0, spring(0.26, 0.78), RiseClock::monotonic(), reduce_motion);
                cx.notify();
            });
        });
        state.update(cx, |state, cx| {
            state.favorite_bump_scale.set(1.1, spring(0.22, 0.62), now, reduce_motion);
            state.favorite_settle = Some(settle);
            cx.notify();
        });
    }

    fn double_tap_heart(
        state: &Entity<PostState>,
        surface: PostDoubleTapSurface,
        theme: &AppTheme,
        window: &Window,
        cx: &App,
    ) -> Option<AnyElement> {
        let presence = &state.read(cx).double_tap_heart_surface;
        let now = RiseClock::monotonic();
        if presence.value(now) != Some(&surface) {
            return None;
        }
        let color = theme.primary._100;
        presence
            .render_transitioned(now, window, |_| {
                div().children(IconUi::sized_with_shadow(
                    ("post-double-tap-heart", state.entity_id()),
                    "heart.fill",
                    px(84.0),
                    color,
                    true,
                    AlphaShadow { color: gpui::black().opacity(0.35), radius: 14.0, x: 0.0, y: 6.0 },
                    cx,
                ))
            })
            .map(|heart| {
                div().absolute().inset_0().flex().items_center().justify_center().child(heart).into_any_element()
            })
    }

    /// one double click on a surface
    /// the like goes through the same path as the button
    fn double_tap_action(&self, state: &Entity<PostState>, surface: PostDoubleTapSurface) -> PostCallback {
        let state = state.clone();
        let is_liked = self.post.is_liked;
        let is_preview = self.is_preview;
        let on_like = self.on_like.clone();
        Rc::new(move |window, cx| {
            Self::handle_double_tap_like(&state, surface, is_liked, is_preview, &on_like, window, cx);
        })
    }

    fn moderation_banner(
        &self,
        banner: PostModerationBanner,
        horizontal_pad: f32,
        theme: &AppTheme,
    ) -> impl IntoElement + use<> {
        let is_preview = self.is_preview;
        let tone = if banner.is_error() { theme.semantic.error_100 } else { theme.primary._100 };
        div().w_full().px(px(horizontal_pad)).pb(px(if is_preview { 4.0 } else { 6.0 })).child(
            hstack(6.0)
                .w_full()
                .px(px(10.0))
                .py(px(if is_preview { 4.0 } else { 7.0 }))
                .bg(tone.opacity(0.1))
                .continuous_corners(8.0)
                .children(symbol(banner.symbol_name(), if is_preview { 10.0 } else { 12.0 }, tone))
                .child(
                    MainText::new(tr(banner.text_key()))
                        .px(if is_preview { 9.0 } else { 12.0 })
                        .font_weight(FontWeight::MEDIUM)
                        .color(tone)
                        .number_of_lines(1),
                )
                .child(spacer_min_w(0.0)),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn post_copy(
        &self,
        name: &str,
        theme: &AppTheme,
        title_px: f32,
        content_px: f32,
        shows_placeholder_title: bool,
        state: &Entity<PostState>,
        nav_store: &Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) -> Div {
        let post = &self.post;
        let is_preview = self.is_preview;
        let mut column = vstack(if is_preview { 0.0 } else { 8.0 }).align(HorizontalAlignment::Leading).w_full();
        if shows_placeholder_title || !post.title.is_empty() {
            let title = if post.title.is_empty() { tr("preview_post_title") } else { post.title.clone() };
            column = column.child(
                div().w_full().child(
                    MainText::new(title)
                        .px(title_px)
                        .font_weight(if self.feed_row && !is_preview { FontWeight::SEMIBOLD } else { FontWeight::BOLD })
                        .number_of_lines(if is_preview { 3 } else { 0 }),
                ),
            );
        }
        if is_preview {
            let content = if post.content.is_empty() { tr("preview_post_content") } else { post.content.clone() };
            column = column
                .child(div().w_full().child(MainText::new(content).px(content_px).number_of_lines(6)));
        } else if !post.content.is_empty() {
            let open_hashtag = self.open_hashtag_action(nav_store);
            let style = self.content_text_style(content_px, theme, cx);
            let is_expanded = Binding::entity(
                state.clone(),
                |state: &PostState| state.is_caption_expanded,
                |state: &mut PostState, is_expanded: bool, cx: &mut Context<PostState>| {
                    state.is_caption_expanded = is_expanded;
                    cx.notify();
                },
            );
            column = column.child(PostCaptionClampUi::new(
                child_id(&self.id, format!("{name}-caption-clamp")),
                style.font_size,
                style.line_spacing,
                theme.text.secondary,
                is_expanded,
                RichTextView::new(child_id(&self.id, format!("{name}-rich-text")), post.content.clone(), style)
                    .options(FormattedTextOptions::post_caption())
                    .on_hashtag_tap(move |tag, _, cx| open_hashtag(tag, cx)),
            ));
        }
        if !post.hashtags.is_empty() {
            column = column.child(self.hashtags_row(name, content_px, theme, nav_store));
        }
        column
    }

    /// avatar stack opens the list read only
    /// coauthor edits live in the post menu not in a gesture on avatars
    fn open_coauthors_sheet(post_id: &str, is_preview: bool, cx: &mut App) {
        if is_preview {
            return;
        }
        PostCollaboratorsPresenter::shared(cx).update(cx, |presenter, cx| {
            presenter.present_coauthors(post_id, false, cx);
        });
    }

    /// track_model is parsed once by the callers pass, it runs three url parses
    fn header(
        &self,
        state: &Entity<PostState>,
        track_model: Option<&TrackModel>,
        theme: &AppTheme,
        nav_store: &Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) -> Div {
        let post = &self.post;
        let is_preview = self.is_preview;
        let collaborators = if is_preview { None } else { PostCollaboratorHeaderPresentation::make(post) };
        let post_id = post.id.clone();

        let avatar = if let Some(collaborators) = collaborators.as_ref() {
            let press_post_id = post_id.clone();
            let hold_post_id = post_id.clone();
            CollaboratorAvatarsUi::new(child_id(&self.id, "collaborator-avatars"), collaborators.author_avatar.clone())
                .coauthors(collaborators.coauthor_avatars.clone())
                .total_count(collaborators.total_count)
                .size(40.0)
                .on_press(move |_, cx| Self::open_coauthors_sheet(&press_post_id, is_preview, cx))
                .on_long_press(move |_, cx| Self::open_coauthors_sheet(&hold_post_id, is_preview, cx))
                .into_any_element()
        } else {
            let is_deleted = post.author.is_deleted;
            let on_avatar_tap = self.on_avatar_tap.clone();
            let open_author: PostCallback = Rc::new(move |window, cx| {
                if !is_deleted {
                    on_avatar_tap(window, cx);
                }
            });
            let on_open_profile = open_author.clone();
            let on_press = open_author;
            UserLogoUi::new(child_id(&self.id, "author-logo"))
                .source(SharedString::from(post.author.more.logo.clone()))
                .name(SharedString::from(post.author.name.clone()))
                .user_id(post.author.id.clone().map(SharedString::from))
                .is_deleted(is_deleted)
                .size(if is_preview { 20.0 } else { 40.0 })
                .is_button(true)
                .show_avatar_context_menu(!is_preview)
                .on_open_profile(move |window, cx| on_open_profile(window, cx))
                .on_press(move |window, cx| on_press(window, cx))
                .into_any_element()
        };

        let mut name_row = hstack(if is_preview { 3.0 } else { 5.0 }).min_w_0().max_w_full();
        if let Some(collaborators) = collaborators.as_ref() {
            let person_nav = nav_store.clone();
            let more_post_id = post_id.clone();
            name_row = name_row.child(
                CollaboratorNamesLineUi::new(
                    child_id(&self.id, "collaborator-names"),
                    collaborators.author_avatar.clone(),
                    collaborators.first_coauthor().clone(),
                    collaborators.total_count,
                )
                .px(16.0)
                .on_person(move |person, _, cx| {
                    let route =
                        RootRoute::UserPage { user_id: Some(person.id.clone()), tag: Some(person.tag.clone()) };
                    push_route(&person_nav, route, cx);
                })
                .on_more(move |_, cx| Self::open_coauthors_sheet(&more_post_id, is_preview, cx)),
            );
        } else {
            name_row = name_row.child(
                MainText::new(post.author.display_name())
                    .px(if is_preview { 10.0 } else { 16.0 })
                    .color(PremiumNameColor::color_for_user(post.author.id.as_deref(), Some(&post.author.more), cx))
                    .number_of_lines(1),
            );
        }
        if collaborators.is_none() && post.author.more.is_official {
            name_row = name_row.child(OfficialBadge::new().size(px(if is_preview { 13.0 } else { 18.0 })));
        }
        if collaborators.is_none() && post.author.more.is_premium {
            name_row = name_row.child(
                PremiumBadge::new()
                    .user_id(post.author.id.clone().map(SharedString::from))
                    .emoji(post.author.more.premium_badge_emoji.clone().map(SharedString::from))
                    .size(if is_preview { 13.0 } else { 18.0 }),
            );
        }

        let mut details = vstack(2.0).align(HorizontalAlignment::Leading).min_w_0().child(name_row);
        if self.feed_row && !is_preview {
            let mut line = hstack(4.0).min_w_0().max_w_full();
            let display_tag = post.author.display_tag();
            if !display_tag.is_empty() {
                line = line
                    .child(
                        MainText::new(format!("@{display_tag}"))
                            .px(12.0)
                            .color(theme.text.secondary)
                            .number_of_lines(1),
                    )
                    .child(MainText::new("\u{b7}").px(12.0).color(theme.text.secondary));
            }
            if let Some(created_at) = PostDateFormatting::date(&post.created_at) {
                let exact = PostDateFormatting::exact(&created_at);
                let relative_date = PostDateFormatting::relative(&created_at);
                let secondary = theme.text.secondary;
                let is_presented = Binding::entity(
                    state.clone(),
                    |state: &PostState| state.is_date_menu_presented,
                    |state: &mut PostState, is_presented: bool, cx: &mut Context<PostState>| {
                        state.is_date_menu_presented = is_presented;
                        cx.notify();
                    },
                );
                let title = exact.clone();
                let actions: HoldContextMenuPresentationActions = Rc::new(move |_: &App| {
                    vec![HoldContextMenuAction::new("copy_date", title.clone()).system_image("calendar")]
                });
                let on_action: HoldContextMenuPresentationOnAction =
                    Rc::new(move |_: String, _: &mut Window, cx: &mut App| UIPasteboard::general().set_string(&exact, cx));
                let source_text = relative_date.clone();
                let source: HoldContextMenuTapSource = Rc::new(move |_: &mut Window, _: &mut App| {
                    MainText::new(source_text.clone()).px(12.0).color(secondary).into_any_element()
                });
                let held_state = state.clone();
                line = line.child(
                    div()
                        .on_mouse_down(MouseButton::Right, move |_: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            let _ = impact(ImpactStyle::Medium);
                            held_state.update(cx, |state, cx| {
                                state.is_date_menu_presented = true;
                                cx.notify();
                            });
                        })
                        .child(MainText::new(relative_date).px(12.0).color(secondary))
                        .tap_context_menu(
                            child_id(&self.id, "created-at-menu"),
                            is_presented,
                            HoldContextMenuAlignment::Leading,
                            false,
                            false,
                            false,
                            None,
                            actions,
                            on_action,
                            source,
                        ),
                );
            }
            details = details.child(line);
        }
        if !is_preview && let Some(track_model) = track_model {
            details = details.child(self.header_music_line(track_model, theme, nav_store));
        }

        let is_deleted = post.author.is_deleted;
        let on_avatar_tap = self.on_avatar_tap.clone();
        let profile_user_id = self.author_id();
        let target_post_id = post.rise_view_target_post_id();
        let date_state = state.clone();

        hstack(10.0)
            .w_full()
            .child(avatar)
            .child(
                details.id(child_id(&self.id, "author-details")).cursor_pointer().on_click(
                    move |_: &ClickEvent, window, cx| {
                        if is_deleted || date_state.read(cx).is_date_menu_presented {
                            return;
                        }
                        on_avatar_tap(window, cx);
                        EngagementInteractionsStore::shared(cx).update(cx, |store, cx| {
                            store.profile_view(&profile_user_id, Some(&target_post_id), cx);
                        });
                    },
                ),
            )
            .child(spacer())
            .child(self.post_options_menu(state, theme, nav_store, cx))
    }

    fn header_music_line(
        &self,
        tm: &TrackModel,
        theme: &AppTheme,
        nav_store: &Option<Entity<NavigationStore>>,
    ) -> impl IntoElement + use<> {
        let is_preview = self.is_preview;
        let nav_store = nav_store.clone();
        let track_id = tm.id.clone();
        let source_selection = self.post.music_use_selection();
        plain(child_id(&self.id, "header-music"))
            .disabled(is_preview)
            .on_press(move |_, cx| {
                if is_preview {
                    return;
                }
                cx.stop_propagation();
                let route = RootRoute::AudioPage {
                    track_id: track_id.clone(),
                    source_selection: source_selection.clone(),
                };
                push_route(&nav_store, route, cx);
            })
            .child(
                hstack(4.0)
                    .children(symbol("music.note", 11.0, theme.text.secondary))
                    .child(
                        MainText::new(Self::music_line_text(tm))
                            .px(12.0)
                            .color(theme.text.secondary)
                            .number_of_lines(1),
                    ),
            )
    }

    fn post_options_is_mine(post: &GetPostFeedResponse, treat_as_own_post: bool, cx: &mut App) -> bool {
        treat_as_own_post || PostMediaCachePolicy::is_own_post(post, cx)
    }

    fn post_options_acts_as_author(post: &GetPostFeedResponse, treat_as_own_post: bool, cx: &mut App) -> bool {
        Self::post_options_is_mine(post, treat_as_own_post, cx)
            || AuthServicesStore::shared(cx).read(cx).viewer_acts_as_author()
            || GuideSandbox::shared(cx).update(cx, |sandbox, cx| sandbox.viewer_acts_as_author(&post.id, cx))
    }

    fn post_options_actions(
        post: &Rc<GetPostFeedResponse>,
        treat_as_own_post: bool,
        cx: &mut App,
    ) -> Vec<HoldContextMenuAction> {
        let is_mine = Self::post_options_is_mine(post, treat_as_own_post, cx);
        let acts_as_author = Self::post_options_acts_as_author(post, treat_as_own_post, cx);
        let mut actions = vec![
            HoldContextMenuAction::new("copy", tr("ctx_copy")).system_image("doc.on.doc"),
            HoldContextMenuAction::new("share", tr("ctx_share")).system_image("square.and.arrow.up"),
        ];
        if post.allows_download_entry(cx) {
            actions.push(HoldContextMenuAction::new("download", tr("post_download_action")).system_image("arrow.down.to.line"));
        }
        actions.push(
            HoldContextMenuAction::new(
                "repost",
                if post.is_reposted { tr("repost_delete_action") } else { tr("repost_action") },
            )
            .system_image("arrow.2.squarepath"),
        );
        if acts_as_author {
            actions.push(
                HoldContextMenuAction::new("stats", tr("post_stats_menu_item")).system_image("chart.bar").section_break(true),
            );
            actions.push(HoldContextMenuAction::new("edit", tr("ctx_edit")).system_image("pencil"));
            actions.push(HoldContextMenuAction::new("delete", tr("ctx_delete")).system_image("trash").destructive(true));
        }
        if !is_mine {
            actions.push(
                HoldContextMenuAction::new("not_interested", tr("shorts_more_not_interested"))
                    .system_image("hand.thumbsdown")
                    .section_break(true),
            );
            actions.push(
                HoldContextMenuAction::new("report", tr("ctx_report"))
                    .system_image("exclamationmark.bubble")
                    .destructive(true)
                    .section_break(true),
            );
        }
        for (index, item) in PostCollaboratorMenu::items(post, acts_as_author, cx).into_iter().enumerate() {
            actions.push(
                HoldContextMenuAction::new(format!("collab.{}", item.id), item.title.clone())
                    .system_image(item.system_image.clone())
                    .destructive(item.destructive)
                    .section_break(index == 0),
            );
        }
        actions
    }

    fn handle_post_options_action(
        id: &str,
        post: &Rc<GetPostFeedResponse>,
        treat_as_own_post: bool,
        nav_store: &Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) {
        match id {
            "copy" => {
                let link = SharePayload::Post((**post).clone())
                    .share_url()
                    .map(|url| url.absolute_string().to_owned())
                    .unwrap_or_default();
                UIPasteboard::general().set_string(&link, cx);
                Notifier::copied(cx);
                let post = post.clone();
                PostInteractionsStore::shared(cx).update(cx, |store, cx| {
                    store.record_share(&post, PostInteractionsStoreShareReceiptChannel::LinkCopy, cx);
                });
            }
            "share" => {
                let post = (**post).clone();
                ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_post(post, cx));
            }
            "download" => {
                let focus_index = PostMediaFocus::page(&post.id, cx);
                let post = (**post).clone();
                PostDownloadSheetStore::shared(cx).update(cx, |store, cx| store.start(post, focus_index, false, 0.0, cx));
            }
            "repost" => {
                let post = post.clone();
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_repost(&post, None, cx));
            }
            "stats" => push_route(nav_store, RootRoute::PostStats { post_id: post.rise_view_target_post_id() }, cx),
            "edit" => push_route(nav_store, RootRoute::EditPost { post: (**post).clone() }, cx),
            "delete" => {
                let post = post.clone();
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.delete_own_post(&post, None, cx));
            }
            "not_interested" => {
                let post = post.clone();
                PostInteractionsStore::shared(cx).update(cx, |store, cx| store.not_interested(&post, cx));
            }
            "report" => {
                if let Ok(post_id) = post.id.parse::<i64>() {
                    ReportSheetStore::shared(cx)
                        .update(cx, |store, cx| store.present(ReportTarget::Post(post_id), None, None, cx));
                }
            }
            _ => {
                let Some(item_id) = id.strip_prefix("collab.") else {
                    return;
                };
                let acts_as_author = Self::post_options_acts_as_author(post, treat_as_own_post, cx);
                let action = PostCollaboratorMenu::items(post, acts_as_author, cx)
                    .into_iter()
                    .find(|item| item.id == item_id)
                    .map(|item| item.action.clone());
                if let Some(action) = action {
                    action(cx);
                }
            }
        }
    }

    fn post_options_items(
        post: &Rc<GetPostFeedResponse>,
        treat_as_own_post: bool,
        nav_store: &Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) -> Vec<ContextMenuItem> {
        Self::post_options_actions(post, treat_as_own_post, cx)
            .into_iter()
            .enumerate()
            .map(|(index, action)| {
                let post = post.clone();
                let nav_store = nav_store.clone();
                let id = action.id.clone();
                let item = ContextMenuItem::new(index as i64, action.title, action.system_image, action.id, move |_, cx| {
                    Self::handle_post_options_action(&id, &post, treat_as_own_post, &nav_store, cx)
                })
                .begins_section(action.section_break);
                if action.destructive { item.destructive(true).native_menu_destructive_icon_matches_label() } else { item }
            })
            .collect()
    }

    fn post_options_menu(
        &self,
        state: &Entity<PostState>,
        theme: &AppTheme,
        nav_store: &Option<Entity<NavigationStore>>,
        cx: &mut App,
    ) -> AnyElement {
        let listed = Self::post_options_actions(&self.post, self.treat_as_own_post, cx);
        let actions: HoldContextMenuPresentationActions = Rc::new(move |_: &App| listed.clone());
        let acted_post = self.post.clone();
        let treat_as_own_post = self.treat_as_own_post;
        let acted_nav = nav_store.clone();
        let on_action: HoldContextMenuPresentationOnAction = Rc::new(move |id: String, _: &mut Window, cx: &mut App| {
            Self::handle_post_options_action(&id, &acted_post, treat_as_own_post, &acted_nav, cx);
        });
        let secondary = theme.text.secondary;
        let glyph = move || div().size(px(28.0)).flex().items_center().justify_center().children(symbol("ellipsis", 16.0, secondary));
        let source: HoldContextMenuTapSource = Rc::new(move |_: &mut Window, _: &mut App| glyph().into_any_element());
        let is_presented = Binding::entity(
            state.clone(),
            |state: &PostState| state.is_options_menu_presented,
            |state: &mut PostState, is_presented: bool, cx: &mut Context<PostState>| {
                state.is_options_menu_presented = is_presented;
                cx.notify();
            },
        );
        let open_state = state.clone();
        div()
            .id(child_id(&self.id, "options-button"))
            .cursor_pointer()
            .on_click(move |_: &ClickEvent, _, cx| {
                open_state.update(cx, |state, cx| {
                    state.is_options_menu_presented = true;
                    cx.notify();
                });
            })
            .child(glyph())
            .guide_anchor(self.carries_guide_anchors.then_some(RiseGuideAnchor::PostOptionsMenu))
            .tap_context_menu(
                child_id(&self.id, "options-menu"),
                is_presented,
                HoldContextMenuAlignment::Trailing,
                false,
                false,
                false,
                None,
                actions,
                on_action,
                source,
            )
            .into_any_element()
    }

    fn hashtags_row(
        &self,
        name: &str,
        px_size: f32,
        theme: &AppTheme,
        nav_store: &Option<Entity<NavigationStore>>,
    ) -> Div {
        let open_hashtag = self.open_hashtag_action(nav_store);
        let text = self.post.hashtags.iter().map(|tag| format!("#{tag}")).collect::<Vec<_>>().join(" ");
        let mut style = FormattedTextStyle::app_standard(theme, px_size);
        style.base_color = theme.primary._100;
        style.hashtag_color = Some(theme.primary._100);
        div().w_full().child(
            RichTextView::new(child_id(&self.id, format!("{name}-hashtags")), text, style)
                .on_hashtag_tap(move |tag, _, cx| open_hashtag(tag, cx)),
        )
    }

    fn content_text_style(&self, px_size: f32, theme: &AppTheme, cx: &mut App) -> FormattedTextStyle {
        let mut style = FormattedTextStyle::app_standard(theme, px_size);
        style.hashtag_color = Some(theme.primary._100);
        if let Some(id) = self.post.shared_achievement_id()
            && let Some(achievement) = PostAchievementCatalog::achievement(id, cx)
            && let Some(color) = PostAchievementCopy::rarity_color(achievement.rarity_tier())
        {
            style.accent = Some(FormattedTextAccent {
                word: PostAchievementCopy::rarity_name(achievement.rarity_tier()),
                color,
            });
        }
        style
    }

    fn open_hashtag_action(&self, nav_store: &Option<Entity<NavigationStore>>) -> Rc<dyn Fn(&str, &mut App)> {
        let is_preview = self.is_preview;
        let is_thread = self.resolved_content_kind() == RisePostContentKind::Thread;
        let nav_store = nav_store.clone();
        Rc::new(move |tag, cx| {
            if is_preview {
                return;
            }
            let kind = if is_thread { RisePostSearchKind::Threads } else { RisePostSearchKind::Publications };
            push_route(&nav_store, RootRoute::HashtagSearch { tag: tag.to_owned(), kind }, cx);
        })
    }

    fn post_actions(&self, state: &Entity<PostState>, cx: &App) -> PostActions {
        let (like_scale, favorite_scale) = {
            let now = RiseClock::monotonic();
            let read = state.read(cx);
            (read.like_bump_scale.value(now), read.favorite_bump_scale.value(now))
        };
        let like_state = state.clone();
        let on_like = self.on_like.clone();
        let fav_state = state.clone();
        let on_fav = self.on_fav.clone();
        let on_open_comments = self.on_open_comments.clone();
        PostActions::new(self.post.clone())
            .is_preview(self.is_preview)
            .feed_row(self.feed_row)
            .like_reaction_scale(like_scale)
            .favorite_reaction_scale(favorite_scale)
            .on_like(move |window, cx| Self::handle_like_button(&like_state, &on_like, window, cx))
            .on_fav(move |window, cx| Self::handle_fav_button(&fav_state, &on_fav, window, cx))
            .on_comments(move |window, cx| on_open_comments(window, cx))
    }

    /// text block
    /// double click likes like the media under it
    fn double_tappable_copy(
        &self,
        state: &Entity<PostState>,
        copy: Div,
        theme: &AppTheme,
        window: &Window,
        cx: &App,
    ) -> impl IntoElement + use<> {
        let double_tap = self.double_tap_action(state, PostDoubleTapSurface::Copy);
        div()
            .id(child_id(&self.id, "copy"))
            .relative()
            .w_full()
            .on_click(move |event: &ClickEvent, window, cx| {
                if event.click_count() == 2 {
                    double_tap(window, cx);
                }
            })
            .child(copy.shared_element_anchor(self.post.id.clone(), SharedElementPart::Text))
            .children(Self::double_tap_heart(state, PostDoubleTapSurface::Copy, theme, window, cx))
    }

    fn standard_body(
        &self,
        state: &Entity<PostState>,
        theme: &AppTheme,
        nav_store: &Option<Entity<NavigationStore>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        let post = &self.post;
        let is_preview = self.is_preview;
        let feed_row = self.feed_row;
        let title_px = if is_preview {
            11.0
        } else if feed_row {
            17.0
        } else {
            20.0
        };
        let content_px = if is_preview { 9.0 } else { 15.0 };
        let horizontal_pad = if feed_row && !is_preview { 12.0 } else { 10.0 };
        let shows_likes_summary = !is_preview && !post.liked_by.is_empty() && post.likes_count > 0;
        let shared_achievement_id = post.shared_achievement_id();
        let has_visual_media = post.media_slide_count() > 0 || shared_achievement_id.is_some();
        let track_model = if is_preview { None } else { post.track_model() };
        let tags_state = state.read(cx).tags_state.clone();

        let mut column = vstack(0.0).align(HorizontalAlignment::Leading).w_full().child(
            self.header(state, track_model.as_ref(), theme, nav_store, cx)
                .px(px(horizontal_pad))
                .pt(px(10.0))
                .pb(px(if is_preview { 5.0 } else { 6.0 })),
        );

        if let Some(banner) = post.moderation_banner() {
            column = column.child(self.moderation_banner(banner, horizontal_pad, theme));
        }

        if is_preview {
            column = column.child(
                self.post_copy("preview-copy", theme, title_px, content_px, true, state, nav_store, cx)
                    .px(px(horizontal_pad))
                    .pb(px(if has_visual_media { 8.0 } else { 6.0 })),
            );
        }

        if !is_preview && let Some(achievement_id) = shared_achievement_id {
            let double_tap = self.double_tap_action(state, PostDoubleTapSurface::Media);
            column = column.child(
                div()
                    .id(child_id(&self.id, "achievement"))
                    .relative()
                    .w_full()
                    .on_click(move |event: &ClickEvent, window, cx| {
                        if event.click_count() == 2 {
                            double_tap(window, cx);
                        }
                    })
                    .child(
                        PostAchievementMedia::new(format!("post-achievement-{}", post.id), achievement_id)
                            .author_id(Some(self.author_id()))
                            .shared_element_anchor(post.id.clone(), SharedElementPart::Media),
                    )
                    .when(post.is_reposted, |achievement| {
                        achievement.child(
                            div()
                                .absolute()
                                .left(px(18.0))
                                .bottom(px(RepostOverlayMetrics::MEDIA_BOTTOM_SAFE_ZONE))
                                .child(RepostOverlay::new(post.clone()).style(RepostStickerStyle::Feed)),
                        )
                    })
                    .children(Self::double_tap_heart(state, PostDoubleTapSurface::Media, theme, window, cx)),
            );
        }

        if !is_preview && shared_achievement_id.is_none() && post.media_slide_count() == 0 && track_model.is_some() {
            column = column.child(
                div().w_full().px(px(horizontal_pad)).pb(px(4.0)).child(
                    PostMusicStrip::new(post.clone()).shared_element_anchor(post.id.clone(), SharedElementPart::Media),
                ),
            );
        }

        if !is_preview && shared_achievement_id.is_none() && post.media_slide_count() > 0 {
            let viewer_is_author = self.viewer_is_author(cx);
            column = column.child(
                div()
                    .relative()
                    .w_full()
                    .child(PostMediaStripView {
                        id: child_id(&self.id, "media-strip"),
                        post: post.clone(),
                        tags_state: tags_state.clone(),
                        viewer_is_author,
                        on_double_tap: self.double_tap_action(state, PostDoubleTapSurface::Media),
                    })
                    .children(Self::double_tap_heart(state, PostDoubleTapSurface::Media, theme, window, cx)),
            );
        }

        if !is_preview && self.has_written_content() {
            let copy = self.post_copy("copy", theme, title_px, content_px, false, state, nav_store, cx);
            column = column.child(
                div()
                    .w_full()
                    .px(px(horizontal_pad))
                    .pt(px(if has_visual_media { 8.0 } else { 5.0 }))
                    .pb(px(2.0))
                    .child(self.double_tappable_copy(state, copy, theme, window, cx)),
            );
        }

        if !is_preview && post.is_pending_coauthor_invite() {
            let on_avatar_tap = self.on_avatar_tap.clone();
            column = column.child(
                div().w_full().px(px(horizontal_pad)).pt(px(14.0)).pb(px(2.0)).child(
                    PostCoauthorInviteBanner::new(post.clone())
                        .on_open_author(move |window, cx| on_avatar_tap(window, cx)),
                ),
            );
        }

        column = column.child(
            div()
                .w_full()
                .px(px(horizontal_pad))
                .pt(px(if feed_row && !is_preview { 7.0 } else { 4.0 }))
                .pb(px(if is_preview {
                    8.0
                } else if shows_likes_summary {
                    2.0
                } else {
                    10.0
                }))
                .child(self.post_actions(state, cx)),
        );

        if shows_likes_summary {
            column = column.child(
                div()
                    .w_full()
                    .px(px(f32::max(5.0, horizontal_pad - 5.0)))
                    .pb(px(10.0))
                    .child(PostLikesSummary::new(post.clone())),
            );
        }

        let card = PostCardClip { radius: (!feed_row).then_some(theme.radius._100) }
            .body(column.bg(if feed_row { theme.bg._100 } else { theme.bg._200 }));
        let mut wrapper = div().relative().w_full().py(px(if feed_row || is_preview { 0.0 } else { 5.0 }));
        if !is_preview && !self.uses_inline_short_card() && post.music_audio_url().is_some() {
            wrapper = wrapper.child(
                div().absolute().inset_0().child(PostCarouselAmbientMusicGate::new(format!("post-ambient-music-{}", post.id), post.clone(), tags_state)),
            );
        }
        wrapper.child(card)
    }

    fn short_feed_body(
        &self,
        state: &Entity<PostState>,
        theme: &AppTheme,
        nav_store: &Option<Entity<NavigationStore>>,
        window: &mut Window,
        cx: &mut App,
    ) -> Div {
        let post = &self.post;
        let feed_row = self.feed_row;
        let horizontal_pad = if feed_row { 12.0 } else { 10.0 };
        let track_model = post.track_model();
        let tags_state = state.read(cx).tags_state.clone();
        let viewer_is_author = self.viewer_is_author(cx);

        let mut column = vstack(0.0).align(HorizontalAlignment::Leading).w_full().child(
            self.header(state, track_model.as_ref(), theme, nav_store, cx)
                .px(px(horizontal_pad))
                .pt(px(10.0))
                .pb(px(6.0)),
        );

        if let Some(banner) = post.moderation_banner() {
            column = column.child(self.moderation_banner(banner, horizontal_pad, theme));
        }

        column = column.child(PostInlineShortMediaView {
            post: post.clone(),
            tags_state,
            viewer_is_author,
        });

        if self.has_written_content() {
            let copy =
                self.post_copy("copy", theme, if feed_row { 17.0 } else { 20.0 }, 15.0, false, state, nav_store, cx);
            column = column.child(
                div()
                    .w_full()
                    .px(px(horizontal_pad))
                    .pt(px(8.0))
                    .pb(px(2.0))
                    .child(self.double_tappable_copy(state, copy, theme, window, cx)),
            );
        }

        if post.is_pending_coauthor_invite() {
            let on_avatar_tap = self.on_avatar_tap.clone();
            column = column.child(
                div().w_full().px(px(horizontal_pad)).pt(px(14.0)).pb(px(2.0)).child(
                    PostCoauthorInviteBanner::new(post.clone())
                        .on_open_author(move |window, cx| on_avatar_tap(window, cx)),
                ),
            );
        }

        column = column.child(
            div()
                .w_full()
                .px(px(horizontal_pad))
                .pt(px(5.0))
                .pb(px(if post.liked_by.is_empty() { 10.0 } else { 2.0 }))
                .child(self.post_actions(state, cx).is_preview(false)),
        );

        if !post.liked_by.is_empty() {
            column = column.child(
                div()
                    .w_full()
                    .px(px(f32::max(5.0, horizontal_pad - 5.0)))
                    .pb(px(10.0))
                    .child(PostLikesSummary::new(post.clone())),
            );
        }

        PostCardClip { radius: (!feed_row).then_some(theme.radius._100) }
            .body(column.bg(if feed_row { theme.bg._100 } else { theme.bg._200 }))
    }

}

impl RenderOnce for Post {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let nav_store = env::<NavigationStoreKey>(cx);
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, cx| PostState {
            like_bump_scale: Animated::new(1.0),
            favorite_bump_scale: Animated::new(1.0),
            double_tap_heart_surface: Presence::new(None)
                .with_transition(Transition::scale(0.4).combined(Transition::opacity())),
            double_tap_heart_target: None,
            tags_state: cx.new(PostMediaTagsState::new),
            like_settle: None,
            favorite_settle: None,
            heart_settle: None,
            is_caption_expanded: false,
            is_options_menu_presented: false,
            is_date_menu_presented: false,
        });

        {
            let now = RiseClock::monotonic();
            let read = state.read(cx);
            if read.like_bump_scale.is_running(now)
                || read.favorite_bump_scale.is_running(now)
                || read.double_tap_heart_surface.is_running(now)
            {
                window.request_animation_frame();
            }
        }

        let uses_inline_short_card = self.uses_inline_short_card();
        let body = if uses_inline_short_card && !self.is_preview {
            self.short_feed_body(&state, &theme, &nav_store, window, cx)
        } else {
            self.standard_body(&state, &theme, &nav_store, window, cx)
        };

        let body = if self.is_preview {
            body.into_any_element()
        } else {
            let items = Self::post_options_items(&self.post, self.treat_as_own_post, &nav_store, cx);
            body.context_menu(child_id(&self.id, "card-menu"), items).into_any_element()
        };

        let impression_post = self.post.clone();
        let records_visibility = self.records_visibility;
        let is_preview = self.is_preview;
        let feed_row = self.feed_row;
        let visibility = records_visibility.then(|| {
            div().absolute().inset_0().child(
                PostVisibilityReader::new(
                    child_id(&self.id, "visibility"),
                    self.post.rise_view_target_post_id(),
                    self.view_surface,
                )
                .enabled(!is_preview && feed_row),
            )
        });

        div().relative().w_full().children(visibility).child(body).on_appear(
            child_id(&self.id, "impression"),
            move |_, cx| {
                Post::fire_impression_if_needed(
                    &impression_post,
                    uses_inline_short_card,
                    records_visibility,
                    is_preview,
                    feed_row,
                    cx,
                );
            },
        )
    }
}

#[derive(IntoElement)]
struct PostMediaStripView {
    id: ElementId,
    post: Rc<GetPostFeedResponse>,
    tags_state: Entity<PostMediaTagsState>,
    viewer_is_author: bool,
    on_double_tap: PostCallback,
}

impl PostMediaStripView {
    fn remote_slides(post: &GetPostFeedResponse) -> Vec<PostMediaSlide> {
        post.media_slides()
            .into_iter()
            .filter_map(|slide| match slide {
                PostMediaSlide::Image { url, size, image_index } => {
                    let trimmed = trimming_whitespaces_and_newlines(&url).to_owned();
                    if !trimmed.to_lowercase().starts_with("http") {
                        return None;
                    }
                    Some(PostMediaSlide::Image { url: trimmed, size, image_index })
                }
                PostMediaSlide::Video { ref url, .. } => url.to_lowercase().starts_with("http").then_some(slide),
            })
            .collect()
    }
}

impl RenderOnce for PostMediaStripView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let memo = window.use_keyed_state(child_id(&self.id, "slides-memo"), cx, |_, _| PostRemoteSlidesMemo::default());
        let slides = memo.update(cx, |memo, _| memo.slides(&self.post, PostMediaStripView::remote_slides));
        if slides.is_empty() {
            return Empty.into_any_element();
        }
        let post = self.post;
        let has_track = post.track.as_ref().and_then(|track| track.id.as_ref()).is_some_and(|id| !id.is_empty());
        div()
            .relative()
            .w_full()
            .child(PostFeedImageGallery {
                id: child_id(&self.id, "gallery"),
                post: post.clone(),
                slides,
                tags_state: self.tags_state.clone(),
                on_double_tap: self.on_double_tap,
            })
            .child(div().absolute().inset_0().child(PostMusicPlateOverlay::new(post.clone())))
            .child(div().absolute().inset_0().child(PostTagBubblesLayer::new(post.clone(), self.tags_state.clone())))
            .when(has_track, |strip| {
                strip.child(div().absolute().left_0().right_0().bottom_0().child(PostMediaSoundBar))
            })
            .child(div().absolute().left_0().bottom_0().child(PostMediaCornerRow {
                id: child_id(&self.id, "corner"),
                post,
                tags_state: self.tags_state,
                viewer_is_author: self.viewer_is_author,
            }))
            .into_any_element()
    }
}

#[derive(Default)]
struct PostRemoteSlidesMemo {
    inputs: Option<PostRemoteSlidesMemoInputs>,
    slides: Rc<Vec<PostMediaSlide>>,
}

#[derive(PartialEq)]
struct PostRemoteSlidesMemoInputs {
    images: Vec<String>,
    image_sizes: Vec<PostMediaSize>,
    video: Option<String>,
    video_thumbnail: Option<String>,
    video_size: Option<PostMediaSize>,
    video_position: Option<i64>,
}

impl PostRemoteSlidesMemoInputs {
    fn new(post: &GetPostFeedResponse) -> Self {
        Self {
            images: post.images.clone(),
            image_sizes: post.image_sizes.clone(),
            video: post.video.clone(),
            video_thumbnail: post.video_thumbnail.clone(),
            video_size: post.video_size.clone(),
            video_position: post.video_position,
        }
    }
}

impl PostRemoteSlidesMemo {
    fn slides(
        &mut self,
        post: &GetPostFeedResponse,
        build: fn(&GetPostFeedResponse) -> Vec<PostMediaSlide>,
    ) -> Rc<Vec<PostMediaSlide>> {
        let current = PostRemoteSlidesMemoInputs::new(post);
        if self.inputs.as_ref() != Some(&current) {
            self.slides = Rc::new(build(post));
            self.inputs = Some(current);
        }
        self.slides.clone()
    }
}

#[derive(IntoElement)]
struct PostInlineShortMediaView {
    post: Rc<GetPostFeedResponse>,
    tags_state: Entity<PostMediaTagsState>,
    viewer_is_author: bool,
}

impl RenderOnce for PostInlineShortMediaView {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let post = self.post;
        div()
            .relative()
            .w_full()
            .child(
                ShortsInlineFeedCell::new(post.clone()).shared_element_anchor(post.id.clone(), SharedElementPart::Media),
            )
            .child(div().absolute().inset_0().child(PostTagBubblesLayer::new(post.clone(), self.tags_state.clone())))
            .child(div().absolute().left_0().bottom_0().child(PostMediaCornerRow {
                id: ElementId::Name(format!("post-inline-corner-{}", post.id).into()),
                post,
                tags_state: self.tags_state,
                viewer_is_author: self.viewer_is_author,
            }))
    }
}

/// repost sticker and tags chip share one corner slot so they sit in one row
/// two overlays would draw on top of each other
#[derive(IntoElement)]
struct PostMediaCornerRow {
    id: ElementId,
    post: Rc<GetPostFeedResponse>,
    tags_state: Entity<PostMediaTagsState>,
    viewer_is_author: bool,
}

impl RenderOnce for PostMediaCornerRow {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let nav_store = env::<NavigationStoreKey>(cx);
        let post = self.post;
        let journey_post_id =
            post.map_journey_id.as_ref().and_then(|_| post.id.parse::<i64>().ok());
        let bottom_inset = PostMediaCornerChrome::bottom_inset(&post);
        hstack(10.0)
            .align(VerticalAlignment::Bottom)
            .pl(px(PostMediaCornerChrome::INSET))
            .pb(px(bottom_inset))
            .when(post.is_reposted, |row| {
                row.child(
                    div()
                        .flex()
                        .flex_col()
                        .items_start()
                        .flex_none()
                        .block_mouse_except_scroll()
                        .child(RepostOverlay::new(post.clone()).style(RepostStickerStyle::Feed)),
                )
            })
            .when_some(journey_post_id, |row, post_id| {
                row.child(with_hint(
                    PageHeaderIconButton::new(child_id(&self.id, "journey"), "map.fill", move |_, cx| {
                        push_route(&nav_store, RootRoute::MapJourney { post_id }, cx);
                    })
                    .font_size(16.0),
                    child_id(&self.id, "journey-hint"),
                    tr("map_journey_open"),
                ).block_mouse_except_scroll())
            })
            .child(PostTagsChip::new(post.clone(), self.tags_state).viewer_is_author(self.viewer_is_author))
    }
}

#[derive(IntoElement)]
struct PostMediaSoundBar;

impl RenderOnce for PostMediaSoundBar {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        hstack(8.0).w_full().p(px(12.0)).child(spacer_min_w(0.0)).child(FeedSoundButton::new())
    }
}

enum PostDateFormatting {}

/// what a new formatter would pick up at creation
/// a cached one keeps its time zone and calendar so any change builds a fresh one
#[derive(Clone, PartialEq)]
struct PostDateFormattingFormatterKey {
    locale: String,
    time_zone: String,
    calendar: CalendarIdentifier,
}

impl PostDateFormattingFormatterKey {
    fn current() -> Self {
        Self {
            locale: Locale::app_locale().identifier().to_owned(),
            time_zone: TimeZone::current().identifier(),
            calendar: Calendar::current().identifier,
        }
    }
}

thread_local! {
    static RELATIVE_FORMATTER: RefCell<Option<(PostDateFormattingFormatterKey, RelativeDateTimeFormatter)>> = const { RefCell::new(None) };
    static EXACT_FORMATTER: RefCell<Option<(PostDateFormattingFormatterKey, DateFormatter)>> = const { RefCell::new(None) };
    static ISO_FORMATTERS: (ISO8601DateFormatter, ISO8601DateFormatter) = {
        let mut fractional = ISO8601DateFormatter::new();
        fractional.set_format_options(
            ISO8601DateFormatterOptions::WITH_INTERNET_DATE_TIME
                | ISO8601DateFormatterOptions::WITH_FRACTIONAL_SECONDS,
        );
        let mut internet = ISO8601DateFormatter::new();
        internet.set_format_options(ISO8601DateFormatterOptions::WITH_INTERNET_DATE_TIME);
        (fractional, internet)
    };
}

impl PostDateFormatting {
    fn date(raw: &str) -> Option<Date> {
        let value = trimming_whitespaces_and_newlines(raw);
        if value.is_empty() {
            return None;
        }
        ISO_FORMATTERS.with(|(fractional, internet)| fractional.date_from(value).or_else(|| internet.date_from(value)))
    }

    fn relative(date: &Date) -> String {
        let now = Date::now();
        if let Some(label) = PostRowDateText::now_band_label(date, &now) {
            return label;
        }
        let key = PostDateFormattingFormatterKey::current();
        RELATIVE_FORMATTER.with(|cache| {
            let mut cache = cache.borrow_mut();
            if let Some((cached_key, formatter)) = cache.as_ref()
                && *cached_key == key
            {
                return formatter.localized_string_for(date, &now);
            }
            let mut formatter = RelativeDateTimeFormatter::new();
            formatter.set_units_style(UnitsStyle::Short);
            formatter.set_locale(Locale::app_locale().identifier());
            let text = formatter.localized_string_for(date, &now);
            *cache = Some((key, formatter));
            text
        })
    }

    fn exact(date: &Date) -> String {
        let key = PostDateFormattingFormatterKey::current();
        EXACT_FORMATTER.with(|cache| {
            let mut cache = cache.borrow_mut();
            if let Some((cached_key, formatter)) = cache.as_ref()
                && *cached_key == key
            {
                return formatter.string_from(date);
            }
            let mut formatter = AppDateFormatter::make(None, None, None);
            formatter.set_date_style(Style::Long);
            formatter.set_time_style(Style::Medium);
            let text = formatter.string_from(date);
            *cache = Some((key, formatter));
            text
        })
    }
}

struct PostCardClip {
    radius: Option<Pixels>,
}

impl PostCardClip {
    fn body(self, content: Div) -> Div {
        if let Some(radius) = self.radius { content.rounded(radius).overflow_hidden() } else { content }
    }
}

struct PostActionsState {
    sparks_taps: i64,
}

#[derive(IntoElement)]
pub struct PostActions {
    post: Rc<GetPostFeedResponse>,
    is_preview: bool,
    feed_row: bool,
    like_reaction_scale: f32,
    favorite_reaction_scale: f32,
    on_like: PostCallback,
    on_fav: PostCallback,
    on_comments: PostCallback,
}

impl PostActions {
    pub fn new(post: impl Into<Rc<GetPostFeedResponse>>) -> Self {
        Self {
            post: post.into(),
            is_preview: false,
            feed_row: false,
            like_reaction_scale: 1.0,
            favorite_reaction_scale: 1.0,
            on_like: Rc::new(|_, _| {}),
            on_fav: Rc::new(|_, _| {}),
            on_comments: Rc::new(|_, _| {}),
        }
    }

    pub fn is_preview(mut self, is_preview: bool) -> Self {
        self.is_preview = is_preview;
        self
    }

    pub fn feed_row(mut self, feed_row: bool) -> Self {
        self.feed_row = feed_row;
        self
    }

    pub fn like_reaction_scale(mut self, like_reaction_scale: f32) -> Self {
        self.like_reaction_scale = like_reaction_scale;
        self
    }

    pub fn favorite_reaction_scale(mut self, favorite_reaction_scale: f32) -> Self {
        self.favorite_reaction_scale = favorite_reaction_scale;
        self
    }

    pub fn on_like(mut self, on_like: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_like = Rc::new(on_like);
        self
    }

    pub fn on_fav(mut self, on_fav: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_fav = Rc::new(on_fav);
        self
    }

    pub fn on_comments(mut self, on_comments: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_comments = Rc::new(on_comments);
        self
    }

    fn id_for(&self, name: &str) -> ElementId {
        ElementId::Name(format!("post-actions-{}-{name}", self.post.id).into())
    }

    fn show_post_likes(post: &Rc<GetPostFeedResponse>, is_preview: bool, shows_likes: bool, cx: &mut App) {
        if is_preview || !shows_likes || post.likes_count <= 0 {
            return;
        }
        let post = post.clone();
        PostInteractionsStore::shared(cx).update(cx, |store, cx| store.open_post_likes(&post, cx));
    }

    fn hold_like(
        post: &Rc<GetPostFeedResponse>,
        is_preview: bool,
        shows_likes: bool,
        on_like: &PostCallback,
        window: &mut Window,
        cx: &mut App,
    ) {
        if is_preview {
            return;
        }
        if shows_likes && post.likes_count > 0 {
            let _ = impact(ImpactStyle::Light);
            let post = post.clone();
            PostInteractionsStore::shared(cx).update(cx, |store, cx| store.open_post_likes(&post, cx));
        } else {
            on_like(window, cx);
        }
    }

    /// same shape as the sparks
    /// press owns its release so a hold that already acted never also runs the tap
    /// right click is the desktop hold
    fn like_icon(&self, icon: &str, color: Hsla, icon_size: f32, shows_likes: bool) -> Stateful<Div> {
        let on_like = self.on_like.clone();
        let hold_on_like = self.on_like.clone();
        let hold_post = self.post.clone();
        let is_preview = self.is_preview;
        let label = tr(if self.post.is_liked { "ctx_unlike" } else { "ctx_like" });
        with_hint(owns_secondary_click(
            PressableUi::new(self.id_for("like"))
                .tap_scale(1.0)
                .long_press_duration(0.42)
                .on_press(move |window, cx| on_like(window, cx))
                .on_long_press(move |window, cx| {
                    Self::hold_like(&hold_post, is_preview, shows_likes, &hold_on_like, window, cx);
                })
                .child(div().size(px(30.0)).flex().items_center().justify_center().children(symbol(
                    icon,
                    icon_size,
                    color,
                ))),
        ), self.id_for("like-hint"), label)
    }

    fn like_count_button(&self, text: &str, text_px: f32, text_color: Hsla, shows_likes: bool) -> impl IntoElement + use<> {
        let post = self.post.clone();
        let is_preview = self.is_preview;
        let hint_id = self.id_for("like-count-hint");
        let button = plain(self.id_for("like-count"))
            .disabled(is_preview || !shows_likes || post.likes_count <= 0)
            .on_press(move |_, cx| Self::show_post_likes(&post, is_preview, shows_likes, cx))
            .child(
                div().frame_min(22.0, 30.0).flex().items_center().child(
                    MainText::new(text.to_owned()).px(text_px).color(text_color).number_of_lines(1),
                ),
            );
        with_hint(button, hint_id, tr("post_likes_title"))
    }

    #[allow(clippy::too_many_arguments)]
    fn like_action_item(
        &self,
        icon: &str,
        color: Hsla,
        text: &str,
        text_color: Hsla,
        icon_size: f32,
        text_px: f32,
        shows_likes: bool,
    ) -> impl IntoElement + use<> {
        hstack(3.0)
            .flex_1()
            .min_h(px(30.0))
            .justify_center()
            .child(self.like_icon(icon, color, icon_size, shows_likes))
            .when(!text.is_empty(), |item| {
                item.child(self.like_count_button(text, text_px, text_color, shows_likes))
            })
            .scale_effect(self.like_reaction_scale)
    }

    #[allow(clippy::too_many_arguments)]
    fn like_pill_item(
        &self,
        icon: &str,
        icon_color: Hsla,
        text: &str,
        icon_size: f32,
        text_px: f32,
        shows_likes: bool,
        theme: &AppTheme,
    ) -> impl IntoElement + use<> {
        hstack(3.0)
            .px(px(7.0))
            .bg(theme.button.bg_300)
            .rounded_rect(5.0)
            .child(self.like_icon(icon, icon_color, icon_size, shows_likes))
            .when(!text.is_empty(), |item| {
                item.child(self.like_count_button(text, text_px, theme.button.content_300, shows_likes))
            })
            .scale_effect(self.like_reaction_scale)
    }

    fn sparks_tap(
        post: &Rc<GetPostFeedResponse>,
        is_preview: bool,
        is_own_post: bool,
        state: &Entity<PostActionsState>,
        cx: &mut App,
    ) {
        if is_preview || post.is_temp {
            return;
        }
        let target = SparksTarget::Post { post_id: post.id.clone() };
        if is_own_post {
            SparksInteractionsStore::shared(cx).update(cx, |store, cx| store.open_send_sheet(target, true, None, cx));
            return;
        }
        let recipient_id = post.author_id.clone();
        let sent = SparksInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.tap(&target, Some(recipient_id.as_str()), cx));
        if sent {
            state.update(cx, |state, cx| {
                state.sparks_taps = state.sparks_taps.wrapping_add(1);
                cx.notify();
            });
        }
    }

    fn sparks_hold(post: &Rc<GetPostFeedResponse>, is_preview: bool, is_own_post: bool, cx: &mut App) {
        if is_preview || post.is_temp {
            return;
        }
        let target = SparksTarget::Post { post_id: post.id.clone() };
        let recipient_id = post.author_id.clone();
        SparksInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.open_send_sheet(target, is_own_post, Some(recipient_id.as_str()), cx));
    }

    /// sparks sit right of the like
    /// tap adds one through the undo toast, hold opens the send sheet
    /// the author whose send the ledger would refuse opens the senders instead
    #[allow(clippy::too_many_arguments)]
    fn sparks_item(
        &self,
        state: &Entity<PostActionsState>,
        is_own_post: bool,
        icon_size: f32,
        text_px: f32,
        text_color: Hsla,
        pill: bool,
        theme: &AppTheme,
        cx: &App,
    ) -> AnyElement {
        let post = self.post.clone();
        let shows_total = post.count_visibility.sparks || is_own_post;
        let value = if shows_total { RiseFormat::compact_count(post.sparks_total) } else { String::new() };
        let is_preview = self.is_preview;
        let label = hstack(if pill { 5.0 } else { 6.0 })
            .child(
                SparksBurstIcon::new(
                    icon_size,
                    SparksTarget::Post { post_id: post.id.clone() },
                    state.read(cx).sparks_taps,
                )
                .is_live(!is_preview),
            )
            .when(shows_total, |label| {
                label.child(
                    MainText::new(value)
                        .px(text_px)
                        .color(text_color)
                        .monospaced_digits(true)
                        .number_of_lines(1),
                )
            });
        let label = if pill {
            label.px(px(10.0)).py(px(5.0)).bg(theme.button.bg_300).capsule()
        } else {
            label.w_full().min_h(px(30.0)).justify_center()
        };

        let tap_post = post.clone();
        let tap_state = state.clone();
        let hold_post = post;
        let button = PressableUi::new(self.id_for("sparks"))
            .tap_scale(1.0)
            .long_press_duration(0.42)
            .on_press(move |_, cx| Self::sparks_tap(&tap_post, is_preview, is_own_post, &tap_state, cx))
            .on_long_press(move |_, cx| Self::sparks_hold(&hold_post, is_preview, is_own_post, cx))
            .child(label);
        let button = with_hint(owns_secondary_click(button), self.id_for("sparks-hint"), tr("sparks_send_title"));
        if pill { button.into_any_element() } else { button.flex_1().into_any_element() }
    }

    #[allow(clippy::too_many_arguments)]
    fn action_item(
        &self,
        name: &str,
        icon: &str,
        color: Hsla,
        text: &str,
        text_color: Hsla,
        icon_size: f32,
        text_px: f32,
        reaction_scale: f32,
        label: String,
        action: PostCallback,
    ) -> impl IntoElement + use<> {
        let text = text.to_owned();
        let shows_text = !text.is_empty();
        div()
            .flex_1()
            .min_h(px(30.0))
            .flex()
            .items_center()
            .justify_center()
            .child(with_hint(
                plain(self.id_for(name)).on_press(move |window, cx| action(window, cx)).child(
                    hstack(6.0).children(symbol(icon, icon_size, color)).when(shows_text, |item| {
                        item.child(MainText::new(text).px(text_px).color(text_color).number_of_lines(1))
                    }),
                ),
                self.id_for(&format!("{name}-hint")),
                label,
            ))
            .scale_effect(reaction_scale)
    }

    #[allow(clippy::too_many_arguments)]
    fn pill_item(
        &self,
        name: &str,
        icon: &str,
        icon_color: Hsla,
        text: &str,
        icon_size: f32,
        text_px: f32,
        reaction_scale: f32,
        label: String,
        action: PostCallback,
        theme: &AppTheme,
    ) -> impl IntoElement + use<> {
        let text = text.to_owned();
        let shows_text = !text.is_empty();
        let content_color = theme.button.content_300;
        let hint_id = self.id_for(&format!("{name}-hint"));
        let button = plain(self.id_for(name)).on_press(move |window, cx| action(window, cx)).child(
            hstack(5.0)
                .px(px(10.0))
                .py(px(5.0))
                .bg(theme.button.bg_300)
                .capsule()
                .children(symbol(icon, icon_size, icon_color))
                .when(shows_text, |item| {
                    item.child(MainText::new(text).px(text_px).color(content_color))
                })
                .scale_effect(reaction_scale),
        );
        with_hint(button, hint_id, label)
    }
}

impl RenderOnce for PostActions {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let state = window.use_keyed_state(self.id_for("state"), cx, |_, _| PostActionsState { sparks_taps: 0 });
        let muted = theme.text.secondary;
        let is_preview = self.is_preview;
        let feed_row = self.feed_row;
        let icon_size = if is_preview {
            12.0
        } else if feed_row {
            18.0
        } else {
            17.5
        };
        let text_px = if is_preview {
            9.0
        } else if feed_row {
            11.0
        } else {
            15.0
        };
        let post = self.post.clone();
        let liked = post.is_liked;

        let auth = AuthServicesStore::shared(cx);
        let viewer_id = auth.read(cx).effective_user_id_for_wire();
        let profile_id = auth.read(cx).profile().map(|profile| profile.id.clone());
        let is_own_post = profile_id.as_deref() == Some(post.author_id.as_str());
        let shows_likes = post.shows_count(|visibility| visibility.likes, viewer_id.as_deref());
        let shows_sparks = post.shows_sparks_button(profile_id.as_deref());

        let likes_text = if shows_likes { RiseFormat::compact_count(post.likes_count) } else { String::new() };
        let comments_text = if post.shows_count(|visibility| visibility.comments, viewer_id.as_deref()) {
            RiseFormat::compact_count(post.comments_count)
        } else {
            String::new()
        };
        let reposts_text = if post.shows_count(|visibility| visibility.reposts, viewer_id.as_deref()) {
            RiseFormat::compact_count(post.repost_count)
        } else {
            String::new()
        };
        let favorites_text = RiseFormat::compact_count(post.favorites_count);
        let like_symbol = if liked { "heart.fill" } else { "heart" };
        let favorite_symbol = if post.is_favorited { "bookmark.fill" } else { "bookmark" };
        let repost_post = post.clone();
        let repost_action: PostCallback = Rc::new(move |_, cx| {
            if is_preview {
                return;
            }
            let post = repost_post.clone();
            PostInteractionsStore::shared(cx).update(cx, |store, cx| store.toggle_repost(&post, None, cx));
        });

        if feed_row {
            hstack(5.0)
                .w_full()
                .child(self.like_action_item(
                    like_symbol,
                    if liked { theme.primary._100 } else { muted },
                    &likes_text,
                    muted,
                    icon_size,
                    text_px,
                    shows_likes,
                ))
                .child(self.action_item(
                    "favorite",
                    favorite_symbol,
                    if post.is_favorited { theme.primary._100 } else { muted },
                    &favorites_text,
                    muted,
                    icon_size,
                    text_px,
                    self.favorite_reaction_scale,
                    tr("post_stats_favorites"),
                    self.on_fav.clone(),
                ))
                .when(shows_sparks, |row| {
                    row.child(self.sparks_item(&state, is_own_post, icon_size, text_px, muted, false, &theme, cx))
                })
                .child(self.action_item(
                    "comments",
                    "bubble.right",
                    muted,
                    &comments_text,
                    muted,
                    icon_size,
                    text_px,
                    1.0,
                    tr("post_stats_comments"),
                    self.on_comments.clone(),
                ))
                .child(self.action_item(
                    "repost",
                    "arrow.2.squarepath",
                    if post.is_reposted { theme.primary._100 } else { muted },
                    &reposts_text,
                    muted,
                    icon_size,
                    text_px,
                    1.0,
                    tr("post_stats_reposts"),
                    repost_action,
                ))
        } else {
            let content = theme.button.content_300;
            hstack(5.0)
                .child(self.like_pill_item(
                    like_symbol,
                    if liked { theme.primary._100 } else { content },
                    &likes_text,
                    icon_size,
                    text_px,
                    shows_likes,
                    &theme,
                ))
                .child(self.pill_item(
                    "favorite",
                    favorite_symbol,
                    if post.is_favorited { theme.primary._100 } else { content },
                    &favorites_text,
                    icon_size,
                    text_px,
                    self.favorite_reaction_scale,
                    tr("post_stats_favorites"),
                    self.on_fav.clone(),
                    &theme,
                ))
                .when(shows_sparks, |row| {
                    row.child(self.sparks_item(&state, is_own_post, icon_size, text_px, content, true, &theme, cx))
                })
                .child(self.pill_item(
                    "comments",
                    "bubble.right",
                    content,
                    &comments_text,
                    icon_size,
                    text_px,
                    1.0,
                    tr("post_stats_comments"),
                    self.on_comments.clone(),
                    &theme,
                ))
                .child(self.pill_item(
                    "repost",
                    "arrow.2.squarepath",
                    if post.is_reposted { theme.primary._100 } else { content },
                    &reposts_text,
                    icon_size,
                    text_px,
                    1.0,
                    tr("post_stats_reposts"),
                    repost_action,
                    &theme,
                ))
        }
    }
}

const DOUBLE_CLICK_WAIT_SECONDS: f64 = 0.25;
const WHEEL_PAGE_TRAVEL: f32 = 48.0;
const PAGE_MOVE_SECONDS: f64 = 0.28;

/// aspect bookkeeping of one image gallery, kept out of observation
#[derive(Default)]
struct PostFeedGalleryAspects {
    locked: Option<f32>,
    measured: HashMap<usize, f32>,
    poster_seed: Option<f32>,
    box_width: Option<f32>,
}

struct PostFeedImageGalleryState {
    aspects: PostFeedGalleryAspects,
    page: usize,
    page_position: Animated<f32>,
    wheel: WheelGestureTracker,
    wheel_claims: Option<bool>,
    wheel_paged: bool,
    wheel_coasting: bool,
    wheel_settle: Option<Task<()>>,
    is_hovered: bool,
    pending_open: Option<Task<()>>,
    has_link_areas: bool,
}

/// photos only
/// server measures a video without its rotation matrix so a video page takes its shape from
/// the decoded poster
fn server_aspect(slides: &[PostMediaSlide], index: usize) -> Option<f32> {
    match slides.get(index)? {
        PostMediaSlide::Image { size, .. } => PostMediaBox::aspect(size.as_ref()),
        PostMediaSlide::Video { .. } => None,
    }
}

/// page 0 with no server size falls back to its poster found in memory so a warm video gets
/// its final box on first layout
fn natural_aspect(aspects: &PostFeedGalleryAspects, slides: &[PostMediaSlide], index: usize) -> Option<f32> {
    aspects
        .measured
        .get(&index)
        .copied()
        .or_else(|| server_aspect(slides, index))
        .or(if index == 0 { aspects.poster_seed } else { None })
}

/// box comes from the first frame and later pages fit into it
/// else paging would change the whole row height on every page turn
fn box_aspect(aspects: &PostFeedGalleryAspects, slides: &[PostMediaSlide]) -> f32 {
    PostMediaBox::display_aspect_for_width(aspects.locked.or_else(|| natural_aspect(aspects, slides, 0)), aspects.box_width)
}

fn bitmap_aspect(image: &RenderImage) -> Option<f32> {
    let size = image.size(0);
    (size.width.0 > 0 && size.height.0 > 0).then(|| size.width.0 as f32 / size.height.0 as f32)
}

#[derive(IntoElement)]
struct PostFeedImageGallery {
    id: ElementId,
    post: Rc<GetPostFeedResponse>,
    slides: Rc<Vec<PostMediaSlide>>,
    tags_state: Entity<PostMediaTagsState>,
    on_double_tap: PostCallback,
}

impl PostFeedImageGallery {
    fn seed_poster_aspect_from_memory(
        post: &GetPostFeedResponse,
        slides: &[PostMediaSlide],
        state: &Entity<PostFeedImageGalleryState>,
        cx: &mut App,
    ) -> f32 {
        let needs_seed = {
            let aspects = &state.read(cx).aspects;
            aspects.locked.is_none()
                && aspects.poster_seed.is_none()
                && !aspects.measured.contains_key(&0)
                && server_aspect(slides, 0).is_none()
        };
        if needs_seed && let Some(source) = slides.first().and_then(PostMediaSlide::poster_url) {
            let is_own = PostMediaCachePolicy::is_own_post(post, cx);
            let poster = CleverImageBitmapLoader::memory_image(
                source,
                is_own,
                SaiAccountCacheScope::active_user_id().as_deref(),
                SaiAccountCacheScope::active_epoch().as_deref(),
                None,
                PostMediaCachePolicy::image_policy(is_own),
                None,
                false,
            );
            if let Some(aspect) = poster.as_deref().and_then(bitmap_aspect) {
                state.update(cx, |state, _| state.aspects.poster_seed = Some(aspect));
            }
        }
        box_aspect(&state.read(cx).aspects, slides)
    }

    /// tags are stored under a photos image index, the video page has none
    fn sync_tag_slide(
        slides: &[PostMediaSlide],
        page: usize,
        tags_state: &Entity<PostMediaTagsState>,
        cx: &mut App,
    ) {
        let Some(slide) = slides.get(page) else {
            return;
        };
        let value = slide.image_index().unwrap_or(-1);
        tags_state.update(cx, |tags_state, cx| tags_state.set_slide(value, cx));
    }

    /// server measures images itself and honours exif rotation so its number stays authoritative
    /// measuring the file matters only when the server sent nothing
    fn adopt(
        state: &Entity<PostFeedImageGalleryState>,
        slides: &[PostMediaSlide],
        aspect: f32,
        index: usize,
        cx: &mut App,
    ) {
        if !aspect.is_finite() || aspect <= 0.0 {
            return;
        }
        state.update(cx, |state, cx| {
            let locks = index == 0 && state.aspects.locked.is_none() && server_aspect(slides, 0).is_none();
            if state.aspects.measured.get(&index) == Some(&aspect) && !locks {
                return;
            }
            let box_before = box_aspect(&state.aspects, slides);
            let fills_before = PostMediaBox::fills(natural_aspect(&state.aspects, slides, index), box_before);
            state.aspects.measured.insert(index, aspect);
            if locks {
                state.aspects.locked = Some(aspect);
            }
            let box_after = box_aspect(&state.aspects, slides);
            if box_after != box_before
                || PostMediaBox::fills(Some(aspect), box_after) != fills_before
                || state.has_link_areas
            {
                cx.notify();
            }
        });
    }

    /// a link baked into the shown slide opens the app link sheet at once
    /// zones sit on the rect the slide is drawn in and are keyed by slide position like the
    /// editor writes them
    fn link_zones(
        &self,
        box_value: f32,
        page: usize,
        state: &Entity<PostFeedImageGalleryState>,
        cx: &App,
    ) -> Option<MediaLinkAreasOverlay> {
        if self.post.link_areas.is_empty() || page >= self.slides.len() {
            return None;
        }
        let natural = natural_aspect(&state.read(cx).aspects, &self.slides, page)?;
        let post_id = self.post.id.clone();
        Some(
            MediaLinkAreasOverlay::new(
                child_id(&self.id, "link-zones"),
                MediaLinkZone::zones_post(&self.post.link_areas, page as i64),
                Rc::new(move |zone: &MediaLinkZone, _, _: &mut Window, cx: &mut App| {
                    let Some(url) = Url::from_string(&zone.url) else {
                        return;
                    };
                    PostInteractionsStore::shared(cx).update(cx, |store, cx| store.record_link_open(&post_id, cx));
                    InAppLinkBrowser::route(&url, cx);
                }),
            )
            .media_aspect(Some(natural))
            .media_fills(PostMediaBox::fills(Some(natural), box_value)),
        )
    }

    fn go_to_page(state: &Entity<PostFeedImageGalleryState>, target: usize, cx: &mut App) {
        let reduce_motion = cx.reduce_motion();
        state.update(cx, |state, cx| {
            if state.page == target {
                return;
            }
            state.page = target;
            state.page_position.set(target as f32, ease_out(PAGE_MOVE_SECONDS), RiseClock::monotonic(), reduce_motion);
            cx.notify();
        });
    }

    fn open_viewer(
        post: &Rc<GetPostFeedResponse>,
        slides: &Rc<Vec<PostMediaSlide>>,
        state: &Entity<PostFeedImageGalleryState>,
        media_viewer: &Entity<MediaViewerPresentationState>,
        index: usize,
        box_aspect: f32,
        cx: &mut App,
    ) {
        let (measured, natural) = {
            let aspects = &state.read(cx).aspects;
            (aspects.measured.clone(), natural_aspect(aspects, slides, index))
        };
        let items = PostMediaGalleryBuilder::items(post, slides, &measured, cx);
        let Some(item) = items.get(index) else {
            return;
        };
        let key = MediaViewerFrameKeys::post_image(&post.id, index);
        let frame = MediaCellScreenFrameRegistry::frame(&key).unwrap_or_default();
        let skin = MediaViewerHeroSkin::new(item.display_source(), !PostMediaBox::fills(natural, box_aspect));
        let forward_post = post.clone();
        let reported_post_id = post.id.clone();
        let handlers = MediaViewerPresentationHandlers {
            on_forward: Some(Rc::new(move |_: &MediaViewerGalleryItem, cx: &mut App| {
                let post = (*forward_post).clone();
                ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_post(post, cx));
            })),
            on_report: Some(Rc::new(move |_: &MediaViewerGalleryItem, cx: &mut App| {
                if let Ok(post_id) = reported_post_id.parse::<i64>() {
                    ReportSheetStore::shared(cx)
                        .update(cx, |store, cx| store.present(ReportTarget::Post(post_id), None, None, cx));
                }
            })),
            ..Default::default()
        };
        let presenter = media_viewer.clone();
        cx.spawn(async move |cx| {
            MediaViewerOpen::present(
                presenter,
                items,
                index,
                frame,
                key,
                skin,
                MediaViewerChrome::POST,
                MediaViewerGalleryScope::Complete,
                handlers,
                cx,
            )
            .await;
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    fn media_cell(
        &self,
        slide: &PostMediaSlide,
        index: usize,
        box_aspect: f32,
        page: usize,
        state: &Entity<PostFeedImageGalleryState>,
        focus: &FocusHandle,
        media_viewer: &Entity<MediaViewerPresentationState>,
        theme: &AppTheme,
        cx: &mut App,
    ) -> AnyElement {
        let post = &self.post;
        let slides = &self.slides;
        let key = MediaViewerFrameKeys::post_image(&post.id, index);
        let is_own = PostMediaCachePolicy::is_own_post(post, cx);
        let (is_presented, is_lifted) = {
            let viewer = media_viewer.read(cx);
            (
                viewer.is_presented(),
                viewer.is_presented() && viewer.hero_lift_id() == Some(key.as_str()) && viewer.bubble_cell_hidden(),
            )
        };
        let policy = PostMediaCachePolicy::image_policy(is_own);
        let fills = PostMediaBox::fills(natural_aspect(&state.read(cx).aspects, slides, index), box_aspect);

        let mut content = div().relative().size_full().overflow_hidden();
        if let Some(source) = slide.poster_url() {
            if !fills {
                content = content.child(
                    div().absolute().inset_0().child(
                        MediaBlurFillUi::new(source.to_owned()).persist_to_disk(is_own).cache_policy(policy),
                    ),
                );
            }
            let adopt_state = state.clone();
            let adopt_slides = slides.clone();
            content = content.child(
                div().absolute().inset_0().child(PostFeedAspectImage {
                    id: child_id(&self.id, format!("image-{index}")),
                    url: source.to_owned(),
                    box_aspect,
                    persist_to_disk: is_own,
                    cache_policy: policy,
                    on_natural_aspect: Rc::new(move |aspect, _, cx| {
                        Self::adopt(&adopt_state, &adopt_slides, aspect, index, cx);
                    }),
                    theme: theme.clone(),
                }),
            );
        } else {
            content = content.bg(theme.bg._300);
        }
        if let PostMediaSlide::Video { url, thumbnail, .. } = slide {
            let adopt_state = state.clone();
            let adopt_slides = slides.clone();
            content = content.child(
                div().absolute().inset_0().child(
                    PostFeedInlineVideoPage::new(
                        format!("post-inline-video-{}-{index}", post.id),
                        post.clone(),
                        url.clone(),
                        thumbnail.clone(),
                        index == page,
                        fills,
                        is_presented,
                    )
                    .on_aspect(move |aspect, _, cx| {
                        Self::adopt(&adopt_state, &adopt_slides, aspect, index, cx);
                    }),
                ),
            );
        }

        let open_post = post.clone();
        let open_slides = slides.clone();
        let open_state = state.clone();
        let open_viewer_state = media_viewer.clone();
        let focus = focus.clone();
        let on_double_tap = self.on_double_tap.clone();
        div()
            .id(child_id(&self.id, format!("cell-{index}")))
            .relative()
            .size_full()
            .child(WindowScreenFrameReader::new(key))
            .child(content)
            .opacity(if is_lifted { 0.0 } else { 1.0 })
            .when(!is_lifted, |cell| {
                cell.on_click(move |event: &ClickEvent, window, cx| {
                    window.focus(&focus, cx);
                    if event.click_count() >= 2 {
                        open_state.update(cx, |state, _| state.pending_open = None);
                        on_double_tap(window, cx);
                        return;
                    }
                    let post = open_post.clone();
                    let slides = open_slides.clone();
                    let weak_state = open_state.downgrade();
                    let viewer = open_viewer_state.clone();
                    let executor = cx.background_executor().clone();
                    let pending = cx.spawn(async move |cx| {
                        executor.timer(Duration::from_secs_f64(DOUBLE_CLICK_WAIT_SECONDS)).await;
                        let _ = cx.update(|cx| {
                            if let Some(state) = weak_state.upgrade() {
                                Self::open_viewer(&post, &slides, &state, &viewer, index, box_aspect, cx);
                            }
                        });
                    });
                    open_state.update(cx, |state, _| state.pending_open = Some(pending));
                })
            })
            .into_any_element()
    }

    fn handle_scroll_wheel(
        event: &ScrollWheelEvent,
        count: usize,
        state: &Entity<PostFeedImageGalleryState>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let line_height = window.line_height();
        let feed = state.update(cx, |state, _| state.wheel.feed(event, line_height));
        if Self::apply_wheel_step(feed.step, count, state, cx) {
            cx.stop_propagation();
        }
        let Some(token) = feed.settle else {
            return;
        };
        let weak_state = state.downgrade();
        let executor = cx.background_executor().clone();
        let settle = cx.spawn(async move |cx| {
            executor.timer(WHEEL_SETTLE_DELAY).await;
            let _ = cx.update(|cx| {
                let Some(state) = weak_state.upgrade() else {
                    return;
                };
                let step = state.update(cx, |state, _| state.wheel.settle(token));
                if let Some(step) = step {
                    Self::apply_wheel_step(step, count, &state, cx);
                }
            });
        });
        state.update(cx, |state, _| state.wheel_settle = Some(settle));
    }

    /// returns whether the event belongs to the gallery rather than whatever lies under it
    fn apply_wheel_step(
        step: WheelStep,
        count: usize,
        state: &Entity<PostFeedImageGalleryState>,
        cx: &mut App,
    ) -> bool {
        match step {
            WheelStep::Ignored => state.read(cx).wheel_coasting,
            WheelStep::Moved(translation) => {
                state.update(cx, |state, _| state.wheel_coasting = false);
                if translation == Point::default() {
                    return false;
                }
                let dx = f32::from(translation.x);
                let dy = f32::from(translation.y);
                let claims = state.update(cx, |state, _| {
                    let page = state.page;
                    *state.wheel_claims.get_or_insert_with(|| dx.abs() > dy.abs() && !(page == 0 && dx > 0.0))
                });
                if claims {
                    Self::turn_page_from_wheel(dx, count, state, cx);
                }
                claims
            }
            WheelStep::Ended(translation) => {
                let claimed = state.read(cx).wheel_claims == Some(true);
                if claimed {
                    Self::turn_page_from_wheel(f32::from(translation.x), count, state, cx);
                }
                state.update(cx, |state, _| {
                    state.wheel_claims = None;
                    state.wheel_paged = false;
                    state.wheel_coasting = claimed;
                });
                claimed
            }
            WheelStep::Cancelled => state.update(cx, |state, _| {
                state.wheel_paged = false;
                state.wheel_claims.take() == Some(true)
            }),
        }
    }

    fn turn_page_from_wheel(
        travel: f32,
        count: usize,
        state: &Entity<PostFeedImageGalleryState>,
        cx: &mut App,
    ) {
        let (page, paged) = {
            let state = state.read(cx);
            (state.page, state.wheel_paged)
        };
        if paged || travel.abs() < WHEEL_PAGE_TRAVEL {
            return;
        }
        state.update(cx, |state, _| state.wheel_paged = true);
        let target = if travel > 0.0 { page.saturating_sub(1) } else { (page + 1).min(count - 1) };
        Self::go_to_page(state, target, cx);
    }

    fn arrow(&self, name: &str, glyph: &str, target: usize, state: &Entity<PostFeedImageGalleryState>) -> impl IntoElement + use<> {
        let state = state.clone();
        div()
            .on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| cx.stop_propagation())
            .child(plain(child_id(&self.id, name.to_owned())).on_press(move |_, cx| Self::go_to_page(&state, target, cx)).child(
            div()
                .size(px(28.0))
                .flex()
                .items_center()
                .justify_center()
                .children(symbol(glyph, 14.0, white()))
                .liquid_chrome_circle(None),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn pages(
        &self,
        box_aspect: f32,
        page: usize,
        state: &Entity<PostFeedImageGalleryState>,
        focus: &FocusHandle,
        media_viewer: &Entity<MediaViewerPresentationState>,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let slides = self.slides.clone();
        let count = slides.len();
        if count == 1 {
            return self.media_cell(&slides[0], 0, box_aspect, page, state, focus, media_viewer, theme, cx);
        }

        let now = RiseClock::monotonic();
        let position = state.read(cx).page_position.frame_value(now, window);
        let is_hovered = state.read(cx).is_hovered;

        let mut pager = div().relative().size_full().overflow_hidden();
        for (index, slide) in slides.iter().enumerate() {
            let offset = index as f32 - position;
            if offset.abs() >= 1.0 && index != page {
                continue;
            }
            let mut holder = div().absolute().top_0().w_full().h_full().left(relative(offset));
            if index == 0 {
                holder = holder.child(
                    div().absolute().inset_0().child(PagerBackSwipeYield::new(
                        child_id(&self.id, "back-swipe-yield"),
                        Some(page == 0),
                    )),
                );
            }
            pager = pager.child(holder.child(self.media_cell(
                slide,
                index,
                box_aspect,
                page,
                state,
                focus,
                media_viewer,
                theme,
                cx,
            )));
        }

        let hover_state = state.clone();
        let wheel_state = state.clone();
        let key_state = state.clone();
        div()
            .id(child_id(&self.id, "pager"))
            .relative()
            .size_full()
            .track_focus(focus)
            .on_hover(move |hovered, _, cx| {
                let hovered = *hovered;
                hover_state.update(cx, |state, cx| {
                    state.is_hovered = hovered;
                    cx.notify();
                });
            })
            .on_scroll_wheel(move |event: &ScrollWheelEvent, window, cx| {
                Self::handle_scroll_wheel(event, count, &wheel_state, window, cx);
            })
            .on_key_down(move |event: &KeyDownEvent, _, cx| {
                let page = key_state.read(cx).page;
                match event.keystroke.key.as_str() {
                    "left" if page > 0 => Self::go_to_page(&key_state, page - 1, cx),
                    "right" if page + 1 < count => Self::go_to_page(&key_state, page + 1, cx),
                    _ => return,
                }
                cx.stop_propagation();
            })
            .child(pager)
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(px(10.0))
                    .flex()
                    .justify_center()
                    .child(RiffPageIndicator::new(count, page as i64)),
            )
            .when(is_hovered && page > 0, |gallery| {
                gallery.child(
                    div()
                        .absolute()
                        .left(px(10.0))
                        .top_0()
                        .bottom_0()
                        .flex()
                        .items_center()
                        .child(self.arrow("previous", "chevron.left", page - 1, state)),
                )
            })
            .when(is_hovered && page + 1 < count, |gallery| {
                gallery.child(
                    div()
                        .absolute()
                        .right(px(10.0))
                        .top_0()
                        .bottom_0()
                        .flex()
                        .items_center()
                        .child(self.arrow("next", "chevron.right", page + 1, state)),
                )
            })
            .into_any_element()
    }
}

impl RenderOnce for PostFeedImageGallery {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let media_viewer = media_viewer_presentation(cx);
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, _| PostFeedImageGalleryState {
            aspects: PostFeedGalleryAspects::default(),
            page: 0,
            page_position: Animated::new(0.0),
            wheel: WheelGestureTracker::new(),
            wheel_claims: None,
            wheel_paged: false,
            wheel_coasting: false,
            wheel_settle: None,
            is_hovered: false,
            pending_open: None,
            has_link_areas: !self.post.link_areas.is_empty(),
        });
        let focus = window.use_keyed_state::<FocusHandle>(child_id(&self.id, "focus"), cx, |_, cx| cx.focus_handle());
        let focus = focus.read(cx).clone();
        let box_value = Self::seed_poster_aspect_from_memory(&self.post, &self.slides, &state, cx);
        let page = state.read(cx).page;

        let hero_lift_id = media_viewer.read(cx).hero_lift_id().map(str::to_owned);
        {
            let slides = self.slides.clone();
            let post_id = self.post.id.clone();
            let state = state.clone();
            on_change(
                child_id(&self.id, "hero-lift"),
                hero_lift_id,
                false,
                window,
                cx,
                move |_, lift_id, _, cx| {
                    let Some(lift_id) = lift_id.as_deref() else {
                        return;
                    };
                    if slides.len() <= 1 {
                        return;
                    }
                    let found = (0..slides.len())
                        .find(|index| lift_id == MediaViewerFrameKeys::post_image(&post_id, *index));
                    if let Some(index) = found {
                        state.update(cx, |state, cx| {
                            if state.page == index {
                                return;
                            }
                            state.page = index;
                            state.page_position.jump(index as f32);
                            cx.notify();
                        });
                    }
                },
            );
        }
        {
            let slides = self.slides.clone();
            let tags_state = self.tags_state.clone();
            let focus_post_id = self.post.id.clone();
            on_change(child_id(&self.id, "page"), page, false, window, cx, move |_, page, _, cx| {
                Self::sync_tag_slide(&slides, *page, &tags_state, cx);
                PostMediaFocus::set(&focus_post_id, *page, cx);
            });
        }

        let pages = self.pages(box_value, page, &state, &focus, &media_viewer, &theme, window, cx);
        let link_zones = self.link_zones(box_value, page, &state, cx);

        let appear_slides = self.slides.clone();
        let appear_tags = self.tags_state.clone();
        let appear_state = state.clone();
        let appear_id = child_id(&self.id, "appear");
        let width_state = state.clone();
        let width_slides = self.slides.clone();
        let width_reader = measure(child_id(&self.id, "box-width"), move |bounds, _, cx| {
            let width = f32::from(bounds.size.width);
            width_state.update(cx, |state, cx| {
                if state.aspects.box_width.is_some_and(|known| (known - width).abs() <= 0.5) {
                    return;
                }
                let before = box_aspect(&state.aspects, &width_slides);
                state.aspects.box_width = Some(width);
                if box_aspect(&state.aspects, &width_slides) != before {
                    cx.notify();
                }
            });
        });
        div()
            .relative()
            .w_full()
            .aspect_ratio(box_value)
            .overflow_hidden()
            .child(width_reader)
            .child(div().absolute().inset_0().child(pages))
            .children(link_zones.map(|zones| div().absolute().inset_0().child(zones)))
            .shared_element_anchor(self.post.id.clone(), SharedElementPart::Media)
            .on_appear(appear_id, move |_, cx| {
                let page = appear_state.update(cx, |state, cx| {
                    if state.aspects.locked.is_none()
                        && let Some(seeded) = server_aspect(&appear_slides, 0)
                    {
                        let before = box_aspect(&state.aspects, &appear_slides);
                        state.aspects.locked = Some(seeded);
                        if box_aspect(&state.aspects, &appear_slides) != before {
                            cx.notify();
                        }
                    }
                    state.page
                });
                Self::sync_tag_slide(&appear_slides, page, &appear_tags, cx);
            })
    }
}

#[derive(Default)]
struct PostFeedDrawnBitmap {
    last_drawn: Weak<RenderImage>,
    adopted: Option<CleverImage>,
}

#[derive(Default)]
struct PostFeedAspectImageState {
    image: Option<CleverImage>,
    did_fail: bool,
    drawn: PostFeedDrawnBitmap,
}

impl PostFeedAspectImageState {
    fn apply(&mut self, ui: CleverImage, cx: &mut gpui::Context<Self>) {
        let is_last_drawn = self.drawn.last_drawn.upgrade().is_some_and(|drawn| Arc::ptr_eq(&drawn, &ui));
        if self.image.is_none() && is_last_drawn {
            self.drawn.adopted = Some(ui);
        } else if !self.image.as_ref().is_some_and(|image| Arc::ptr_eq(image, &ui)) {
            self.image = Some(ui);
            cx.notify();
        }
        if self.did_fail {
            self.did_fail = false;
            cx.notify();
        }
    }
}

#[derive(IntoElement)]
struct PostFeedAspectImage {
    id: ElementId,
    url: String,
    box_aspect: f32,
    persist_to_disk: bool,
    cache_policy: CleverImageCachePolicy,
    on_natural_aspect: Rc<dyn Fn(f32, &mut Window, &mut App)>,
    theme: AppTheme,
}

impl RenderOnce for PostFeedAspectImage {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, |_, _| PostFeedAspectImageState::default());

        let immediate = {
            let read = state.read(cx);
            read.image.clone().or_else(|| read.drawn.adopted.clone()).or_else(|| {
                CleverImageBitmapLoader::memory_image(
                    &self.url,
                    self.persist_to_disk,
                    SaiAccountCacheScope::active_user_id().as_deref(),
                    SaiAccountCacheScope::active_epoch().as_deref(),
                    None,
                    self.cache_policy,
                    None,
                    false,
                )
            })
        };
        let last_drawn = immediate.as_ref().map(Arc::downgrade).unwrap_or_default();
        state.update(cx, |state, _| state.drawn.last_drawn = last_drawn);
        let did_fail = state.read(cx).did_fail;

        {
            let url = self.url.clone();
            let persist_to_disk = self.persist_to_disk;
            let cache_policy = self.cache_policy;
            let on_natural_aspect = self.on_natural_aspect.clone();
            let task_state = state.clone();
            use_task(
                child_id(&self.id, "load"),
                (self.url.clone(), persist_to_disk, cache_policy),
                window,
                cx,
                async move |cx| {
                    let loaded = CleverImageBitmapLoader::load(
                        &url,
                        persist_to_disk,
                        true,
                        SaiAccountCacheScope::active_user_id(),
                        SaiAccountCacheScope::active_epoch(),
                        None,
                        cache_policy,
                        CleverImageWorkLane::Visible,
                        &*cx,
                    )
                    .await;
                    let _ = cx.update(|window, cx| {
                        let Some(loaded) = loaded else {
                            task_state.update(cx, |state, cx| {
                                if state.image.is_none() && state.drawn.adopted.is_none() {
                                    state.did_fail = true;
                                    cx.notify();
                                }
                            });
                            return;
                        };
                        let aspect = bitmap_aspect(&loaded);
                        MediaViewerImageMemoryCache::store(
                            Arc::clone(&loaded),
                            &RemoteMediaURL::display_source(&url, None),
                            SaiAccountCacheScope::active_user_id().as_deref(),
                        );
                        task_state.update(cx, |state, cx| state.apply(loaded, cx));
                        if let Some(aspect) = aspect {
                            on_natural_aspect(aspect, window, cx);
                        }
                    });
                },
            );
        }

        if let Some(immediate) = immediate {
            let fills = PostMediaBox::fills(bitmap_aspect(&immediate), self.box_aspect);
            return img(blur_aware_image_source(ImageSource::Render(immediate)))
                .absolute()
                .inset_0()
                .size_full()
                .object_fit(if fills { ObjectFit::Cover } else { ObjectFit::Contain })
                .into_any_element();
        }
        let placeholder =
            div().size_full().bg(self.theme.bg._300).flex().items_center().justify_center();
        if did_fail {
            return placeholder
                .children(symbol("photo", 17.0, self.theme.text.secondary.opacity(0.6)))
                .into_any_element();
        }
        placeholder.child(div().child(progress_view().scale_effect(0.85))).into_any_element()
    }
}
