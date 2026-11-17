use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;

use futures::channel::oneshot;
use gpui::{
    AnyElement, AnyView, App, AppContext, AvailableSpace, Bounds, BoxShadow, ClickEvent, ContentMask, Context, Div, Element, ElementId, Empty,
    Entity, FontWeight, GlobalElementId, Hsla, InspectorElementId, InteractiveElement, IntoElement, LayoutId,
    ParentElement, Pixels, PromptButton, PromptLevel, Render, RenderOnce, SharedString, Stateful, StatefulInteractiveElement,
    StyleRefinement, Styled, Subscription, Task, TransformationMatrix, WeakEntity, Window, black, canvas, div, point, px, size, white,
};
use rise_canon::{RiseGuideAnchor, RiseGuideSignal};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::{tr, tr_plural};
use rise_platform::haptics::selection_changed;
use rise_platform::pasteboard::UIPasteboard;
use rise_theme::AppTheme;
use rise_theme::ui_color_system::UIColor;
use rise_ui::bottom_sheet_ui::bottom_sheet_metrics::BottomSheetMetrics;
use rise_ui::context_menu_ui::ContextMenuItem;
use rise_ui::expandable_text_ui::ExpandableTextUi;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::guide_overlay_ui::guide_overlay_window::GuideOverlayHost;
use rise_ui::icon_ui::IconUi;
use rise_ui::main_text::{MainText, TextAlignT};
use rise_ui::message_bubble_geometry::MessageBubbleRadii;
use rise_ui::official_badge::OfficialBadge;
use rise_ui::page_header_toolbar_button::PageHeaderMetrics;
use rise_ui::scroll_chrome_shadow_ui::ScrollChromeShadowStyle;
use rise_ui::scroll_edge_blur_ui::ScrollEdgeBlurFalloff;
use rise_ui::skeleton_ui::{SkeletonLine, SkeletonShimmerExt};
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::button_style::{PlainButton, plain};
use rise_ui::swiftui::context_menu::ContextMenuModifiers;
use rise_ui::swiftui::environment::{EnvKey, LayoutDirection, env, env_scope};
use rise_ui::swiftui::lifecycle::{on_change, use_task};
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::linear_gradient::LinearGradient;
use rise_ui::swiftui::modifiers::{ModifierExt, ViewModifier};
use rise_ui::swiftui::rounded_gradient::{rounded_gradient_fill, rounded_gradient_stroke_border};
use rise_ui::swiftui::shapes::{Shape, centred_stroke};
use rise_ui::swiftui::stacks::{hstack, spacer_min_w, vstack};
use rise_ui::swiftui::transition::UnitPoint;
use rise_ui::view_anchor_probe::{ViewAnchorProbe, ViewAnchorProbeExt};
use rise_ui::window_screen_frame_reader::WindowScreenFrameReader;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::guides::guide_facts::GuideFacts;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::media_viewer::media_viewer_presentation_state::{
    MediaViewerPresentationState, media_viewer_presentation,
};
use crate::core::ui::bottom_sheet_ui::bottom_sheet_ui::BottomSheetUi;
use crate::core::ui::premium_badge::{PremiumBadge, PremiumBadgeSource};
use crate::core::ui::premium_name_color::PremiumNameColor;
use crate::core::widgets::notifier::notifier_store::Notifier;
use crate::core::widgets::sheets::share_sheet::ShareSheetStore;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::backdrop::chat_decorative_backdrop::{
    ChatBackdropLayers, ChatBackdropLayout,
};
use crate::modules::chat::stores::chats::chat_actions::chat_types::User;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::gifts::components::gift_worn_cosmetics::{GiftWornCosmeticsModifiers, GiftWornProfileEffect};
use crate::modules::gifts::stores::gift_appearance::gift_appearance_interactions::gift_appearance_interactions::GiftAppearanceInteractionsStore;
use crate::modules::gifts::stores::gifts::gifts_interactions::gifts_interactions::{
    GiftRecipient, GiftsInteractionsStore,
};
use crate::modules::gifts::engine::rise_gifts_engine_models::RiseGiftItemKind;
use crate::modules::music::shared::utils::music::featured_profile_track_store::FeaturedProfileTrackStore;
use crate::modules::music::shared::utils::music::music_models::TrackModel;
use crate::modules::story::components::story_avatar_ring::StoryAvatarRing;
use crate::modules::story::components::story_composer_host::StoryComposerHost;
use crate::modules::story::stores::story::story_actions::story_wire_types::HighlightScope;
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::user::components::premium_badge_picker::premium_badge_picker_popover::{
    PremiumBadgePickerPopover, PremiumBadgePopoverLayout,
};
use crate::modules::user::components::profile_content::profile_content::ProfileViewportLayoutPolicy;
use crate::modules::user::components::profile_content::profile_scroll_coordinator::ProfileScrollMetrics;
use crate::modules::user::components::profile_top::profile_about_sheet::ProfileAboutSheet;
use crate::modules::user::components::profile_top::profile_avatar_album::{
    ProfileAlbumAvatarImage, ProfileAvatarAlbum, ProfileAvatarAlbumSync,
};
use crate::modules::user::components::profile_top::profile_featured_track_row::ProfileFeaturedTrackRow;
use crate::modules::user::components::profile_top::profile_featured_tracks_sheet::{
    ProfileFeaturedTracksPlayback, ProfileFeaturedTracksSheet,
};
use crate::modules::user::components::profile_top::profile_highlights::{
    ProfileHighlights, ProfileHighlightsLayoutPolicy,
};
use crate::modules::user::components::social::social_links_preview_row::SocialLinksPreviewRow;
use crate::modules::user::components::social::social_media_links_sheet::SocialMediaLinksSheet;
use crate::modules::user::engine::core::rise_user_block_wire_models::RiseBlockScope;
use crate::modules::user::shared::social_platform::SocialLink;
use crate::modules::user::stores::user::user_actions::user_types::{
    FollowButtonState, ProfileModel, UserListRelationship,
};
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;
use crate::modules::user::stores::user_profile_photos::user_profile_photos_services::user_profile_photos_services::UserProfilePhotosServicesStore;

const BANNER_HEIGHT: f32 = 250.0;
const AVATAR_SIZE: f32 = 88.0;
const AVATAR_HANG: f32 = 46.0;
const FEATURED_TRACKS_FRACTIONS: [f32; 2] = [0.62, 0.92];
const HIDDEN_COUNT: &str = "\u{2013}";
const USER_TAG_PREFIX: &str = "@";
const FULLY_ROUND_RADIUS: f32 = 1000.0;

pub enum ProfileBannerStretchPolicy {}

impl ProfileBannerStretchPolicy {
    pub fn scale(height: f32, overscroll: f32) -> f32 {
        if height <= 1.0 || overscroll <= 0.5 {
            return 1.0;
        }
        (height + overscroll) / height
    }
}

pub enum ProfileTopFadeTargetPolicy {}

impl ProfileTopFadeTargetPolicy {
    pub fn uses_tail_fallback(is_foreign: bool, has_highlights: bool) -> bool {
        is_foreign && !has_highlights
    }
}

/// rows fade by their top edge and are gone when it reaches the header
/// a row taller than the fade path is only masked under the header
pub enum ProfileTopScrollFadePolicy {}

impl ProfileTopScrollFadePolicy {
    pub fn fade_distance() -> f32 {
        ProfileHighlightsLayoutPolicy::ROW_HEIGHT
    }

    pub fn fully_hidden_min_y(pinned_tabs_top: f32, outer_spacing: f32) -> f32 {
        pinned_tabs_top - outer_spacing
    }

    pub fn is_tall(height: f32) -> bool {
        height > Self::fade_distance()
    }

    pub fn opacity(frame_min_y: f32, frame_max_y: f32, pinned_tabs_top: f32, outer_spacing: f32) -> f32 {
        let height = frame_max_y - frame_min_y;
        if Self::is_tall(height) {
            return 1.0;
        }
        let hidden_min_y = Self::fully_hidden_min_y(pinned_tabs_top, outer_spacing);
        let progress = (frame_min_y - hidden_min_y) / height.max(1.0);
        progress.clamp(0.0, 1.0)
    }

    pub fn mask_cut(frame_min_y: f32) -> f32 {
        Self::fully_hidden_min_y(ProfileScrollMetrics::PINNED_TABS_TOP, ProfileViewportLayoutPolicy::OUTER_SPACING)
            - frame_min_y
    }
}

/// fades_across_own_height off keeps the highlights rule, used for highlights and avatar rows
#[derive(Clone, Copy)]
struct ProfileTopScrollFadeModifier {
    is_enabled: bool,
    fades_across_own_height: bool,
    masks_only: bool,
}

impl ProfileTopScrollFadeModifier {
    fn new(is_enabled: bool) -> Self {
        Self { is_enabled, fades_across_own_height: true, masks_only: false }
    }

    fn fades_across_own_height(mut self, fades_across_own_height: bool) -> Self {
        self.fades_across_own_height = fades_across_own_height;
        self
    }

    fn masks_only(mut self, masks_only: bool) -> Self {
        self.masks_only = masks_only;
        self
    }

    /// opacity to draw the row with, plus whether the tall row mask applies
    fn resolve(self, frame_min_y: f32, frame_max_y: f32) -> (Option<f32>, Option<ProfileTopTallRowMask>) {
        if self.masks_only {
            return (None, Some(ProfileTopTallRowMask { always: true }));
        }
        if self.fades_across_own_height {
            let opacity = ProfileTopScrollFadePolicy::opacity(
                frame_min_y,
                frame_max_y,
                ProfileScrollMetrics::PINNED_TABS_TOP,
                ProfileViewportLayoutPolicy::OUTER_SPACING,
            );
            return (Some(opacity), Some(ProfileTopTallRowMask { always: false }));
        }
        let opacity = ProfileHighlightsLayoutPolicy::scroll_opacity(
            frame_max_y,
            ProfileScrollMetrics::PINNED_TABS_TOP,
            ProfileViewportLayoutPolicy::OUTER_SPACING,
        );
        (Some(opacity), None)
    }
}

impl ViewModifier for ProfileTopScrollFadeModifier {
    fn body(self, content: AnyElement, _window: &mut Window, cx: &mut App) -> AnyElement {
        if !self.is_enabled {
            return content;
        }
        ProfileTopScrollFadeElement {
            content: div().w_full().child(content),
            modifier: self,
            mask_surface: rise_ui::theme(cx).bg._100,
        }
        .into_any_element()
    }
}

/// hides only the part of a tall row above the header line, soft edge below
#[derive(Clone, Copy)]
struct ProfileTopTallRowMask {
    always: bool,
}

impl ProfileTopTallRowMask {
    /// header line in row coordinates, none if the row isnt masked
    fn cut(self, frame_min_y: f32, height: f32) -> Option<f32> {
        (self.always || ProfileTopScrollFadePolicy::is_tall(height))
            .then(|| ProfileTopScrollFadePolicy::mask_cut(frame_min_y))
    }
}

/// what a faded row draws, rounded so a move that changes nothing on screen keeps the cached head
#[derive(Clone, Copy, PartialEq, Eq)]
struct ProfileTopScrollFadeLook {
    opacity: Option<i32>,
    cut: Option<i32>,
}

impl ProfileTopScrollFadeLook {
    fn of(opacity: Option<f32>, cut: Option<f32>, height: f32) -> Self {
        let fade = ProfileTopScrollFadePolicy::fade_distance();
        Self {
            opacity: opacity.map(|opacity| (opacity * 200.0).round() as i32),
            cut: cut.map(|cut| (cut.clamp(-fade, height.max(-fade)) * 2.0).round() as i32),
        }
    }
}

struct ProfileTopScrollFadeElement {
    content: Div,
    modifier: ProfileTopScrollFadeModifier,
    mask_surface: Hsla,
}

impl IntoElement for ProfileTopScrollFadeElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ProfileTopScrollFadeElement {
    type RequestLayoutState = <Div as Element>::RequestLayoutState;
    type PrepaintState = (<Div as Element>::PrepaintState, Option<f32>);

    fn id(&self) -> Option<ElementId> {
        Element::id(&self.content)
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        Element::source_location(&self.content)
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        Element::request_layout(&mut self.content, id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let viewport_top = f32::from(window.content_mask().bounds.top());
        let frame_min_y = f32::from(bounds.top()) - viewport_top;
        let frame_max_y = f32::from(bounds.bottom()) - viewport_top;
        let (opacity, mask) = self.modifier.resolve(frame_min_y, frame_max_y);
        if let Some(opacity) = opacity {
            self.content.style().opacity = Some(opacity);
        }
        let cut = mask.and_then(|mask| mask.cut(frame_min_y, frame_max_y - frame_min_y));
        let modifier = self.modifier;
        let view = window.current_view();
        let drawn = ProfileTopScrollFadeLook::of(opacity, cut, frame_max_y - frame_min_y);
        window.on_replayed_move(move |offset, mask_offset, window, cx| {
            let shift = f32::from(offset.y) - f32::from(mask_offset.y);
            let (min_y, max_y) = (frame_min_y + shift, frame_max_y + shift);
            let (opacity, mask) = modifier.resolve(min_y, max_y);
            let cut = mask.and_then(|mask| mask.cut(min_y, max_y - min_y));
            if ProfileTopScrollFadeLook::of(opacity, cut, max_y - min_y) != drawn {
                window.defer(cx, move |_, cx| cx.notify(view));
            }
        });
        (Element::prepaint(&mut self.content, id, inspector_id, bounds, request_layout, window, cx), cut)
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (content_prepaint, cut) = prepaint;
        let content = &mut self.content;
        let Some(cut) = *cut else {
            Element::paint(content, id, inspector_id, bounds, request_layout, content_prepaint, window, cx);
            return;
        };
        let visible_top = bounds.top() + px(cut.clamp(0.0, f32::from(bounds.size.height)));
        let visible = Bounds::from_corners(point(bounds.left(), visible_top), bounds.bottom_right());
        let clip = Bounds::from_corners(
            point(window.content_mask().bounds.left(), visible.top()),
            point(window.content_mask().bounds.right(), visible.bottom()),
        );
        window.with_content_mask(Some(ContentMask { bounds: clip }), |window| {
            Element::paint(content, id, inspector_id, bounds, request_layout, content_prepaint, window, cx);
        });
        let fade = ProfileTopScrollFadePolicy::fade_distance();
        if cut + fade <= 0.0 {
            return;
        }
        let edge = Bounds::new(point(bounds.left(), bounds.top() + px(cut)), size(bounds.size.width, px(fade)));
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            LinearGradient::colors(
                [self.mask_surface, self.mask_surface.opacity(0.0)],
                UnitPoint::TOP,
                UnitPoint::BOTTOM,
            )
            .paint(edge, LayoutDirection::current(), window);
        });
    }
}

/// edge under the floating header, colourless blur with a theme color scrim
/// nothing at rest, fades in as the banner goes under
pub enum ProfileTopScrollShadePolicy {}

impl ProfileTopScrollShadePolicy {
    pub const REVEAL_DISTANCE: f32 = 56.0;

    pub fn overflow() -> f32 {
        ScrollChromeShadowStyle::list_shell_blurred_header().overflow_below_header
    }

    pub fn height() -> f32 {
        ProfileScrollMetrics::PINNED_TABS_TOP + Self::overflow()
    }

    /// opacity from the banner bottom in scroll view space
    pub fn opacity(banner_max_y: f32, pinned_tabs_top: f32) -> f32 {
        let progress = (pinned_tabs_top + Self::REVEAL_DISTANCE - banner_max_y) / Self::REVEAL_DISTANCE;
        progress.clamp(0.0, 1.0)
    }

    /// how far the edge moves up so its fade ends on the tabs top, never below it
    pub fn lift(frame_max_y: f32, pinned_tabs_top: f32, outer_spacing: f32) -> f32 {
        let overflow = Self::overflow();
        let tabs_top = frame_max_y + outer_spacing;
        overflow.min((pinned_tabs_top + overflow - tabs_top).max(0.0))
    }
}

struct ProfileTopScrollShadeElement {
    frame: Div,
    banner_height: f32,
}

impl ProfileTopScrollShadeElement {
    fn new(banner_height: f32) -> Self {
        Self { frame: div().absolute().inset_0(), banner_height }
    }

    fn shade(cx: &App) -> Div {
        let height = ProfileTopScrollShadePolicy::height();
        let header_fraction = (height - ProfileTopScrollShadePolicy::overflow()).max(0.0) / height;
        let page = rise_ui::theme(cx).bg._100;
        let stops = ScrollEdgeBlurFalloff::tint_stops(page, header_fraction, true);
        div()
            .relative()
            .w_full()
            .h(px(height))
            .child(div().absolute().inset_0().child(LinearGradient::new(stops, UnitPoint::TOP, UnitPoint::BOTTOM)))
    }
}

impl IntoElement for ProfileTopScrollShadeElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ProfileTopScrollShadeElement {
    type RequestLayoutState = <Div as Element>::RequestLayoutState;
    type PrepaintState = (<Div as Element>::PrepaintState, Option<AnyElement>);

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        Element::request_layout(&mut self.frame, id, inspector_id, window, cx)
    }

    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let frame_prepaint = Element::prepaint(&mut self.frame, id, inspector_id, bounds, request_layout, window, cx);
        let viewport_top = f32::from(window.content_mask().bounds.top());
        let frame_min_y = f32::from(bounds.top()) - viewport_top;
        let frame_max_y = f32::from(bounds.bottom()) - viewport_top;
        let opacity = ProfileTopScrollShadePolicy::opacity(
            frame_min_y + self.banner_height,
            ProfileScrollMetrics::PINNED_TABS_TOP,
        );
        if opacity <= 0.0 {
            return (frame_prepaint, None);
        }
        let offset_y = -frame_min_y
            - ProfileTopScrollShadePolicy::lift(
                frame_max_y,
                ProfileScrollMetrics::PINNED_TABS_TOP,
                ProfileViewportLayoutPolicy::OUTER_SPACING,
            );
        let mut shade = Self::shade(cx);
        shade.style().opacity = Some(opacity);
        let mut shade = shade.into_any_element();
        shade.layout_as_root(
            size(
                AvailableSpace::Definite(bounds.size.width),
                AvailableSpace::Definite(px(ProfileTopScrollShadePolicy::height())),
            ),
            window,
            cx,
        );
        shade.prepaint_at(point(bounds.left(), bounds.top() + px(offset_y)), window, cx);
        (frame_prepaint, Some(shade))
    }

    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let (frame_prepaint, shade) = prepaint;
        Element::paint(&mut self.frame, id, inspector_id, bounds, request_layout, frame_prepaint, window, cx);
        if let Some(shade) = shade.as_mut() {
            shade.paint(window, cx);
        }
    }
}

struct ProfileBannerStretchElement {
    child: AnyElement,
    scale: f32,
}

impl IntoElement for ProfileBannerStretchElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ProfileBannerStretchElement {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        (self.child.request_layout(window, cx), ())
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let viewport_top = f32::from(window.content_mask().bounds.top());
        self.scale = ProfileBannerStretchPolicy::scale(
            f32::from(bounds.size.height),
            f32::from(bounds.top()) - viewport_top,
        );
        self.child.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        if self.scale == 1.0 {
            self.child.paint(window, cx);
            return;
        }
        let scale_factor = window.scale_factor();
        let anchor = point(bounds.center().x, bounds.bottom());
        let matrix = TransformationMatrix::unit()
            .translate(anchor.scale(scale_factor))
            .scale(size(self.scale, self.scale))
            .translate(anchor.scale(-scale_factor));
        window.with_transformation(matrix, |window| self.child.paint(window, cx));
    }
}

/// view state plus the stores body reads, each store notifies this entity
struct ProfileTopState {
    show_about: bool,
    show_social_links: bool,
    show_story_composer: bool,
    show_featured_tracks: bool,
    featured_track_id: Option<String>,
    featured_tracks_lowest_fraction: f32,
    premium_badge_anchor: ViewAnchorProbe,
    remove_friend_alert: Option<Task<()>>,
    avatar_registry_id: Option<String>,
    hero_hidden: bool,
    story_profile: Option<(String, bool)>,
    story_facts: ProfileTopStoryFacts,
    _observers: [Subscription; 5],
}

/// story facts the profile head draws
#[derive(Clone, Copy, Default, PartialEq)]
struct ProfileTopStoryFacts {
    has_active: bool,
    has_unseen: bool,
    publishing_own: bool,
}

impl ProfileTopStoryFacts {
    fn read(profile_id: &str, treat_as_foreign: bool, cx: &mut App) -> Self {
        let service = StoryServicesStore::shared(cx);
        let service = service.read(cx);
        let publishing =
            service.is_publishing || service.is_shared_story_background_activity() || service.publish_ring_is_settling;
        let has_active = service.has_active_stories(Some(profile_id), cx);
        let has_unseen = service.has_unseen_stories(Some(profile_id), cx);
        let publishing_own = publishing && {
            let auth = AuthServicesStore::shared(cx);
            match auth.read(cx).profile().map(|profile| profile.id.as_str()).filter(|my_id| !my_id.is_empty()) {
                Some(my_id) => profile_id == my_id,
                None => !treat_as_foreign,
            }
        };
        Self { has_active, has_unseen, publishing_own }
    }
}

impl ProfileTopState {
    fn new(media_presenter: &Entity<MediaViewerPresentationState>, cx: &mut Context<Self>) -> Self {
        let auth = AuthServicesStore::shared(cx);
        let story_service = StoryServicesStore::shared(cx);
        let featured_track_store = FeaturedProfileTrackStore::shared(cx);
        let user_services = UserServicesStore::shared(cx);
        Self {
            show_about: false,
            show_social_links: false,
            show_story_composer: false,
            show_featured_tracks: false,
            featured_track_id: None,
            featured_tracks_lowest_fraction: FEATURED_TRACKS_FRACTIONS[0],
            premium_badge_anchor: ViewAnchorProbe::new(),
            remove_friend_alert: None,
            avatar_registry_id: None,
            hero_hidden: false,
            story_profile: None,
            story_facts: ProfileTopStoryFacts::default(),
            _observers: [
                cx.observe(&auth, |state: &mut Self, _, cx| {
                    state.refresh_story_facts(cx);
                    cx.notify();
                }),
                cx.observe(&story_service, |state: &mut Self, _, cx| {
                    if state.refresh_story_facts(cx) {
                        cx.notify();
                    }
                }),
                cx.observe(&featured_track_store, |_, _, cx| cx.notify()),
                cx.observe(&user_services, |_, _, cx| cx.notify()),
                cx.observe(media_presenter, |state: &mut Self, viewer, cx| {
                    let next = Self::hero_hidden(viewer.read(cx), state.avatar_registry_id.as_deref());
                    if next != state.hero_hidden {
                        state.hero_hidden = next;
                        cx.notify();
                    }
                }),
            ],
        }
    }

    /// rereads story facts of the drawn profile, true if they changed
    fn refresh_story_facts(&mut self, cx: &mut Context<Self>) -> bool {
        let Some((profile_id, treat_as_foreign)) = self.story_profile.clone() else {
            return false;
        };
        let next = ProfileTopStoryFacts::read(&profile_id, treat_as_foreign, cx);
        if next == self.story_facts {
            return false;
        }
        self.story_facts = next;
        true
    }

    fn hero_hidden(viewer: &MediaViewerPresentationState, registry_id: Option<&str>) -> bool {
        viewer.is_presented()
            && registry_id.is_some_and(|id| viewer.hero_lift_id() == Some(id))
            && viewer.bubble_cell_hidden()
    }
}

#[derive(IntoElement)]
pub struct ProfileTop {
    id: ElementId,
    pub profile: ProfileModel,
    pub is_user: bool,
    pub is_resolving: bool,
    pub counts_resolving: bool,
    pub effect_replay: i64,
}

impl ProfileTop {
    pub fn new(id: impl Into<ElementId>, profile: ProfileModel) -> Self {
        Self { id: id.into(), profile, is_user: false, is_resolving: false, counts_resolving: false, effect_replay: 0 }
    }

    pub fn effect_replay(mut self, effect_replay: i64) -> Self {
        self.effect_replay = effect_replay;
        self
    }

    pub fn is_user(mut self, is_user: bool) -> Self {
        self.is_user = is_user;
        self
    }

    pub fn is_resolving(mut self, is_resolving: bool) -> Self {
        self.is_resolving = is_resolving;
        self
    }

    pub fn counts_resolving(mut self, counts_resolving: bool) -> Self {
        self.counts_resolving = counts_resolving;
        self
    }

    pub fn copy(value: &str, cx: &mut App) {
        UIPasteboard::general().set_string(value, cx);
        Notifier::copied(cx);
    }
}

impl ProfileTop {
    pub fn cached_slot(self, host_id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> AnyElement {
        let host = window
            .use_keyed_state(host_id, cx, |_, cx| ProfileTopHostHolder { host: cx.new(|_| ProfileTopHostView::new()) })
            .read(cx)
            .host
            .clone();
        let props = ProfileTopProps {
            id: self.id,
            profile: self.profile,
            is_user: self.is_user,
            is_resolving: self.is_resolving,
            counts_resolving: self.counts_resolving,
            effect_replay: self.effect_replay,
        };
        let (revision, height) = host.update(cx, |host, _| {
            if host.props.as_ref() != Some(&props) {
                host.props = Some(props);
                host.revision = host.revision.wrapping_add(1);
            }
            (host.revision, host.height.get())
        });
        let view = AnyView::from(host);
        let head = match height {
            Some(height) => env_scope::<ProfileTopRevisionKey>(revision)
                .child(view.cached(StyleRefinement::default().w_full().h(height)).replay_when_moved())
                .into_any_element(),
            None => div().w_full().child(view).into_any_element(),
        };
        div()
            .relative()
            .w_full()
            .child(head)
            .child(ProfileTopScrollShadeElement::new(BANNER_HEIGHT))
            .into_any_element()
    }
}

struct ProfileTopHostHolder {
    host: Entity<ProfileTopHostView>,
}

/// what the head renders besides its stores and environment
#[derive(Clone, PartialEq)]
struct ProfileTopProps {
    id: ElementId,
    profile: ProfileModel,
    is_user: bool,
    is_resolving: bool,
    counts_resolving: bool,
    effect_replay: i64,
}

/// profile head as its own cached view so frames of stuff below the profile route dont rebuild it
struct ProfileTopHostView {
    props: Option<ProfileTopProps>,
    revision: u64,
    height: Rc<Cell<Option<Pixels>>>,
}

impl ProfileTopHostView {
    fn new() -> Self {
        Self { props: None, revision: 0, height: Rc::new(Cell::new(None)) }
    }
}

struct ProfileTopRevisionKey;

impl EnvKey for ProfileTopRevisionKey {
    type Value = u64;

    fn default_value() -> Self::Value {
        0
    }

    fn fingerprint(value: &Self::Value) -> Option<u64> {
        Some(*value)
    }
}

impl Render for ProfileTopHostView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let height = self.height.clone();
        let host = cx.entity().downgrade();
        let top = self.props.clone().map(|props| ProfileTop {
            id: props.id,
            profile: props.profile,
            is_user: props.is_user,
            is_resolving: props.is_resolving,
            counts_resolving: props.counts_resolving,
            effect_replay: props.effect_replay,
        });
        div().relative().w_full().children(top).child(
            canvas(
                move |bounds, window, cx| {
                    let measured = bounds.size.height;
                    if measured <= px(0.0) || height.get() == Some(measured) {
                        return;
                    }
                    height.set(Some(measured));
                    window.defer(cx, move |_, cx| {
                        let _ = host.update(cx, |_, cx| cx.notify());
                    });
                },
                |_, _, _, _| {},
            )
            .absolute()
            .inset_0(),
        )
    }
}

impl RenderOnce for ProfileTop {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let media_presenter = media_viewer_presentation(cx);
        let state_id = child_id(&self.id, "state");
        let state = {
            let media_presenter = media_presenter.clone();
            window.use_keyed_state(state_id, cx, move |_, cx| ProfileTopState::new(&media_presenter, cx))
        };
        let auth = AuthServicesStore::shared(cx);
        let my_id = auth.read(cx).profile().map(|profile| profile.id.clone());
        let body = ProfileTopBody {
            treat_as_foreign: treat_as_foreign(self.is_user, my_id.as_deref(), &self.profile.id),
            id: self.id,
            profile: Rc::new(self.profile),
            is_resolving: self.is_resolving,
            counts_resolving: self.counts_resolving,
            effect_replay: self.effect_replay,
            state,
            nav_store: env::<NavigationStoreKey>(cx),
            media_presenter,
        };
        body.body(&theme, window, cx)
    }
}

fn treat_as_foreign(is_user: bool, my_id: Option<&str>, profile_id: &str) -> bool {
    if !is_user {
        return false;
    }
    !my_id.is_some_and(|my_id| !my_id.is_empty() && profile_id == my_id)
}

fn child_id(id: &ElementId, name: &str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::from(name.to_owned()))
}

fn icon(symbol: &str, side: f32, color: Hsla) -> Option<gpui::Svg> {
    IconUi::sized(symbol, px(side), color, symbol.ends_with(".fill"))
}

#[derive(Clone)]
struct ProfileTopBody {
    id: ElementId,
    profile: Rc<ProfileModel>,
    is_resolving: bool,
    counts_resolving: bool,
    effect_replay: i64,
    treat_as_foreign: bool,
    state: Entity<ProfileTopState>,
    nav_store: Option<Entity<NavigationStore>>,
    media_presenter: Entity<MediaViewerPresentationState>,
}

struct RisePill {
    id: &'static str,
    icon: &'static str,
    tint: Hsla,
    value: String,
    label: String,
}

impl ProfileTopBody {
    fn element_id(&self, name: &str) -> ElementId {
        child_id(&self.id, name)
    }

    fn social_links(&self) -> Vec<SocialLink> {
        SocialLink::from(&self.profile.social_links)
    }

    fn featured_tracks(&self, cx: &mut App) -> Vec<TrackModel> {
        FeaturedProfileTrackStore::shared(cx).update(cx, |store, cx| store.tracks_for(&self.profile.id, cx))
    }

    fn has_active_stories(&self, cx: &mut App) -> bool {
        let service = StoryServicesStore::shared(cx);
        service.read(cx).has_active_stories(Some(&self.profile.id), cx)
    }

    /// story facts the head draws, kept in state so render doesnt read the story store
    fn story_facts(&self, cx: &mut App) -> ProfileTopStoryFacts {
        let key = (self.profile.id.clone(), self.treat_as_foreign);
        let state = self.state.read(cx);
        if state.story_profile.as_ref() == Some(&key) {
            return state.story_facts;
        }
        let facts = ProfileTopStoryFacts::read(&key.0, key.1, cx);
        self.state.update(cx, |state, _| {
            state.story_profile = Some(key);
            state.story_facts = facts;
        });
        facts
    }

    fn is_publishing_own_story(&self, cx: &mut App) -> bool {
        let service = StoryServicesStore::shared(cx);
        let service = service.read(cx);
        if !service.is_publishing && !service.is_shared_story_background_activity() && !service.publish_ring_is_settling
        {
            return false;
        }
        let auth = AuthServicesStore::shared(cx);
        match auth.read(cx).profile().map(|profile| profile.id.as_str()).filter(|my_id| !my_id.is_empty()) {
            Some(my_id) => self.profile.id == my_id,
            None => !self.treat_as_foreign,
        }
    }

    /// tier the profile owner placed on me, not the reverse
    /// blocking takes nothing from the blocker, this gates the blocked viewer
    fn owner_block_scope(&self, cx: &mut App) -> Option<RiseBlockScope> {
        if !self.treat_as_foreign {
            return None;
        }
        UserServicesStore::shared(cx).update(cx, |services, cx| services.owner_block_scope(&self.profile.id, cx))
    }

    fn blocks_follow_intent(&self, state: FollowButtonState, cx: &mut App) -> bool {
        if self.owner_block_scope(cx).is_none() {
            return false;
        }
        state == FollowButtonState::Subscribe || state == FollowButtonState::SubscribeBack
    }

    /// picture the header shows, a just picked photo is held as album row till upload settles
    /// for handlers and menu only, the circle observes the album itself
    fn avatar_source(&self, cx: &mut App) -> String {
        if !self.treat_as_foreign {
            let album = UserProfilePhotosServicesStore::shared(cx)
                .update(cx, |services, cx| services.album(&self.profile.id, cx));
            if let Some(pending) = album.as_ref().and_then(|album| album.main_photo()).filter(|photo| photo.is_pending)
            {
                return pending.file_url.clone();
            }
        }
        let source = self.profile.avatar_url.as_deref().unwrap_or(&self.profile.more.logo);
        trimming_whitespaces_and_newlines(source).to_owned()
    }

    fn avatar_registry_id(&self) -> String {
        ProfileAvatarAlbum::registry_id(&self.profile.id)
    }

    fn open_avatar_album(&self, cx: &mut App) {
        let avatar_source = self.avatar_source(cx);
        ProfileAvatarAlbum::open(
            &self.media_presenter,
            &self.profile.id,
            &self.profile.name,
            &avatar_source,
            !self.treat_as_foreign,
            self.profile.profile_photos_count,
            &self.avatar_registry_id(),
            AVATAR_SIZE,
            cx,
        );
    }

    fn open_stories(&self, cx: &mut App) {
        StoryInteractionsStore::shared(cx)
            .update(cx, |interactions, cx| interactions.open_stories_for_user_id(&self.profile.id, None, cx));
    }

    fn push(&self, route: RootRoute, cx: &mut App) {
        if let Some(nav_store) = &self.nav_store {
            nav_store.update(cx, |store, cx| store.push(route, NavigationStoreSettlingPush::Drop, cx));
        }
    }

    fn gamification_source(&self, cx: &mut App) -> ProfileModel {
        if !self.treat_as_foreign {
            let auth = AuthServicesStore::shared(cx);
            if let Some(me) = auth.read(cx).profile().filter(|me| me.id == self.profile.id) {
                return me.clone();
            }
        }
        (*self.profile).clone()
    }

    fn uses_tail_fade_fallback(&self, cx: &mut App) -> bool {
        let highlights = StoryServicesStore::shared(cx)
            .update(cx, |service, cx| service.highlights(&self.profile.id, HighlightScope::Profile, cx));
        ProfileTopFadeTargetPolicy::uses_tail_fallback(self.treat_as_foreign, !highlights.is_empty())
    }

    fn body(self, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let task_user_id = trimming_whitespaces_and_newlines(&self.profile.id).to_owned();
        use_task(self.element_id("task"), self.profile.id.clone(), window, cx, async move |cx| {
            if task_user_id.is_empty() {
                return;
            }
            let _ = cx.update(|_, cx| {
                StoryInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.load_stories_state(std::slice::from_ref(&task_user_id), cx));
                GiftAppearanceInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.profile_opened(Some(&task_user_id), cx));
                FeaturedProfileTrackStore::shared(cx).update(cx, |store, cx| store.load(&task_user_id, cx));
            });
        });
        let show_featured_tracks = self.state.read(cx).show_featured_tracks;
        on_change(
            self.element_id("featured-tracks-open"),
            show_featured_tracks,
            false,
            window,
            cx,
            |_, open, _, cx| GuideFacts::set(RiseGuideSignal::FeaturedTracksOpen, *open, cx),
        );

        let uses_tail_fade_fallback = self.uses_tail_fade_fallback(cx);
        let about_section = self.about_section(theme, cx);
        let rise_hud = self.rise_hud(theme, cx);
        let banner_card = self.banner_card(theme, cx);
        div()
            .relative()
            .w_full()
            .child(
                vstack(14.0)
                    .w_full()
                    .child(banner_card)
                    .child(
                        vstack(14.0)
                            .w_full()
                            .child(about_section)
                            .children(rise_hud)
                            .children(self.treat_as_foreign.then(|| {
                                self.action_buttons(theme, cx)
                                    .modifier(ProfileTopScrollFadeModifier::new(uses_tail_fade_fallback))
                            })),
                    )
                    .child(
                        ProfileHighlights::new(self.element_id("highlights"), self.profile.id.clone())
                            .is_owner(!self.treat_as_foreign)
                            .modifier(
                                ProfileTopScrollFadeModifier::new(!uses_tail_fade_fallback)
                                    .fades_across_own_height(false),
                            ),
                    ),
            )
            .child(
                div()
                    .absolute()
                    .child(self.about_sheet(theme))
                    .child(self.social_links_sheet(theme))
                    .child(self.featured_tracks_sheet(theme, cx))
                    .child(StoryComposerHost::new(
                        self.element_id("story-composer"),
                        Binding::entity(
                            self.state.clone(),
                            |state: &ProfileTopState| state.show_story_composer,
                            |state, value, _| state.show_story_composer = value,
                        ),
                    )),
            )
            .into_any_element()
    }

    fn about_sheet(&self, theme: &AppTheme) -> BottomSheetUi {
        let is_presented = Binding::entity(
            self.state.clone(),
            |state: &ProfileTopState| state.show_about,
            |state, value, _| state.show_about = value,
        );
        let body = self.clone();
        let content_presented = is_presented.clone();
        BottomSheetUi::new(
            self.element_id("about-sheet"),
            is_presented,
            move |_, _| {
                ProfileAboutSheet::new(
                    body.element_id("about-sheet-content"),
                    content_presented.clone(),
                    (*body.profile).clone(),
                )
                .achievements_user_id(body.treat_as_foreign.then(|| body.profile.id.clone()))
            },
            |_, _| Empty,
        )
        .fits_content(true)
        .footer_shows_top_border(false)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(theme.bg._100))
    }

    fn social_links_sheet(&self, theme: &AppTheme) -> BottomSheetUi {
        let body = self.clone();
        BottomSheetUi::new(
            self.element_id("social-links-sheet"),
            Binding::entity(
                self.state.clone(),
                |state: &ProfileTopState| state.show_social_links,
                |state, value, _| state.show_social_links = value,
            ),
            move |_, _| {
                SocialMediaLinksSheet::new(body.element_id("social-links-content"), body.social_links())
                    .hugs_content(true)
            },
            |_, _| Empty,
        )
        .snap_fractions(vec![0.5, 0.92])
        .fits_content(true)
        .footer_shows_top_border(false)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(theme.bg._100))
    }

    fn featured_tracks_sheet(&self, theme: &AppTheme, cx: &App) -> BottomSheetUi {
        let body = self.clone();
        let state = self.state.clone();
        BottomSheetUi::new(
            self.element_id("featured-tracks-sheet"),
            Binding::entity(
                self.state.clone(),
                |state: &ProfileTopState| state.show_featured_tracks,
                |state, value, _| state.show_featured_tracks = value,
            ),
            move |_, _| {
                let on_empty_state = state.clone();
                let minimum_height_state = state.clone();
                ProfileFeaturedTracksSheet::new(
                    body.element_id("featured-tracks-content"),
                    body.profile.id.clone(),
                    Binding::entity(
                        state.clone(),
                        |state: &ProfileTopState| state.featured_track_id.clone(),
                        |state, value, _| state.featured_track_id = value,
                    ),
                )
                .is_owner(!body.treat_as_foreign)
                .on_empty(move |_, cx| {
                    on_empty_state.update(cx, |state, cx| {
                        state.show_featured_tracks = false;
                        cx.notify();
                    });
                })
                .on_minimum_height(move |content_height, window, cx| {
                    update_featured_tracks_lowest_fraction(&minimum_height_state, content_height, window, cx);
                })
            },
            |_, _| Empty,
        )
        .snap_fractions(featured_tracks_snap_fractions(self.state.read(cx).featured_tracks_lowest_fraction))
        .footer_shows_top_border(false)
        .sheet_background(Some(theme.bg._100))
        .footer_background(Some(theme.bg._100))
    }

    fn banner_card(&self, theme: &AppTheme, cx: &mut App) -> Div {
        let banner = div()
            .w_full()
            .h(px(BANNER_HEIGHT))
            .relative()
            .rounded_b(px(26.0))
            .overflow_hidden()
            .child(
                ChatBackdropLayers::new(ChatBackdropLayout::ProfileBanner)
                    .banner_url(self.profile.cover_image_url.clone())
                    .bottom_corner_radius(26.0),
            )
            .child(div().absolute().inset_0().child(
                GiftWornProfileEffect::new(self.element_id("worn-effect"), Some(self.profile.id.clone()))
                    .replay(self.effect_replay),
            ));
        let row = hstack(14.0)
            .w_full()
            .items_start()
            .px(px(16.0))
            .child(self.avatar(theme, cx))
            .child(self.identity(theme, cx))
            .child(spacer_min_w(0.0));

        div()
            .relative()
            .w_full()
            .h(px(BANNER_HEIGHT))
            .mb(px(AVATAR_HANG))
            .child(ProfileBannerStretchElement { child: banner.into_any_element(), scale: 1.0 })
            .child(div().absolute().left_0().right_0().bottom(px(-AVATAR_HANG)).child(row))
    }

    fn avatar(&self, theme: &AppTheme, cx: &mut App) -> Div {
        let registry_id = self.avatar_registry_id();
        let hero_hidden = self.state.update(cx, |state, _| {
            if state.avatar_registry_id.as_deref() != Some(registry_id.as_str()) {
                state.avatar_registry_id = Some(registry_id.clone());
            }
            state.hero_hidden
        });
        let story_facts = self.story_facts(cx);
        let has_active_stories = story_facts.has_active;
        let has_unseen_stories = story_facts.has_unseen;
        let is_publishing_own_story = story_facts.publishing_own;
        let avatar_source = self.avatar_source(cx);
        let can_open_album = ProfileAvatarAlbum::can_open(&avatar_source);

        let mut menu = Vec::new();
        if has_active_stories {
            let body = self.clone();
            menu.push(ContextMenuItem::new(
                0,
                tr("ctx_view_story"),
                Some("play.circle".to_owned()),
                "ctx_view_story",
                move |_, cx| body.open_stories(cx),
            ));
        }
        if can_open_album {
            let body = self.clone();
            menu.push(ContextMenuItem::new(
                1,
                tr("profile_photos_open"),
                Some("photo".to_owned()),
                "profile_photos_open",
                move |_, cx| body.open_avatar_album(cx),
            ));
        }

        let image = ProfileAlbumAvatarImage::new(
            self.element_id("avatar-image"),
            self.profile.id.clone(),
            self.profile.name.clone(),
            self.profile.is_deleted,
            AVATAR_SIZE,
            !self.treat_as_foreign,
            self.profile.avatar_url.clone().unwrap_or_else(|| self.profile.more.logo.clone()),
        )
        .hidden(hero_hidden)
        .guide_anchor((!self.treat_as_foreign).then_some(RiseGuideAnchor::ProfileAvatar));

        let tap = self.clone();
        let circle = div()
            .id(self.element_id("avatar"))
            .relative()
            .size(px(AVATAR_SIZE))
            .flex_none()
            .rounded_full()
            .cursor_pointer()
            .shadow(vec![BoxShadow {
                color: black().opacity(0.22),
                offset: point(px(0.0), px(3.0)),
                blur_radius: px(8.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            .child(
                div()
                    .size_full()
                    .relative()
                    .opacity(if hero_hidden { 0.0 } else { 1.0 })
                    .child(WindowScreenFrameReader::new(registry_id).source_media_radii(Some(
                        MessageBubbleRadii::uniform(px(AVATAR_SIZE / 2.0)),
                    )))
                    .child(div().absolute().child(ProfileAvatarAlbumSync::new(
                        self.element_id("avatar-album-sync"),
                        self.profile.id.clone(),
                        self.profile.name.clone(),
                    )))
                    .child(image),
            )
            .child(centred_stroke(theme.bg._100, 2.0, Shape::Circle))
            .child(
                div().absolute().top(px(-4.0)).left(px(-4.0)).child(
                    StoryAvatarRing::new(
                        self.element_id("avatar-ring"),
                        is_publishing_own_story,
                        has_active_stories,
                        has_unseen_stories,
                        AVATAR_SIZE + 8.0,
                    )
                    .line_width(2.5),
                ),
            )
            .on_click(move |_: &ClickEvent, _, cx| {
                let _ = selection_changed();
                if tap.has_active_stories(cx) {
                    tap.open_stories(cx);
                    return;
                }
                let avatar_source = tap.avatar_source(cx);
                if !tap.treat_as_foreign && !ProfileAvatarAlbum::can_open(&avatar_source) {
                    tap.push(RootRoute::ProfileSettings, cx);
                    return;
                }
                tap.open_avatar_album(cx);
            })
            .context_menu(self.element_id("avatar-menu"), menu)
            .gift_worn_frame(self.element_id("avatar-frame"), Some(self.profile.id.clone()), AVATAR_SIZE)
            .loops(true);

        div()
            .relative()
            .flex_none()
            .size(px(AVATAR_SIZE))
            .child(circle)
            .children((!self.treat_as_foreign).then(|| self.add_story_button(theme)))
    }

    fn add_story_button(&self, theme: &AppTheme) -> Stateful<Div> {
        let body = self.clone();
        div()
            .id(self.element_id("add-story"))
            .absolute()
            .right(px(-2.0))
            .bottom(px(-2.0))
            .size(px(34.0))
            .flex()
            .items_center()
            .justify_center()
            .cursor_pointer()
            .occlude()
            .child(
                div()
                    .relative()
                    .size(px(26.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(theme.primary._100)
                    .child(centred_stroke(theme.bg._100, 2.5, Shape::Circle))
                    .children(icon("plus", 13.0, theme.content_on(theme.primary._100))),
            )
            .on_click(move |_: &ClickEvent, _, cx| {
                if body.is_publishing_own_story(cx) {
                    return;
                }
                let _ = selection_changed();
                body.state.update(cx, |state, cx| {
                    state.show_story_composer = true;
                    cx.notify();
                });
            })
    }

    fn identity(&self, theme: &AppTheme, cx: &mut App) -> Div {
        let name: AnyElement = if self.is_resolving && self.profile.name.is_empty() {
            div()
                .relative()
                .child(div().invisible().child(hidden_line(22.0)))
                .child(
                    div()
                        .absolute()
                        .left_0()
                        .top_0()
                        .bottom_0()
                        .flex()
                        .items_center()
                        .child(SkeletonLine::new(140.0, 16.0).skeleton_shimmer(None)),
                )
                .into_any_element()
        } else {
            let display_name = if self.profile.is_deleted && trimming_whitespaces_and_newlines(&self.profile.name).is_empty() {
                tr("chat_deleted_account")
            } else {
                self.profile.name.clone()
            };
            MainText::new(display_name)
                .px(22.0)
                .tac(TextAlignT::Left)
                .font_weight(FontWeight::BOLD)
                .color(PremiumNameColor::color_for_user(Some(&self.profile.id), Some(&self.profile.more), cx))
                .number_of_lines(1)
                .minimum_scale_factor(0.7)
                .into_any_element()
        };
        let is_premium = PremiumBadgeSource::is_premium(Some(&self.profile.id), self.profile.more.is_premium, cx);
        let name_row = hstack(6.0)
            .min_w_0()
            .max_w_full()
            .relative()
            .top(px(3.0))
            .child(name)
            .children(self.profile.more.is_official.then(|| OfficialBadge::new().size(px(20.0))))
            .children(is_premium.then(|| self.premium_badge(!self.treat_as_foreign, cx)));

        let tag = trimming_whitespaces_and_newlines(&self.profile.tag);
        let is_publishing_own_story = self.story_facts(cx).publishing_own;
        let waiting_for_network = is_publishing_own_story
            && StoryServicesStore::shared(cx).update(cx, |service, cx| service.is_publish_waiting_for_network(cx));
        let subtitle = if waiting_for_network {
            Some(tr("chats_status_waiting_for_network"))
        } else if !tag.is_empty() {
            Some(format!("{USER_TAG_PREFIX}{tag}"))
        } else {
            None
        };

        vstack(3.0)
            .items_start()
            .min_w_0()
            .pt(px(30.0))
            .relative()
            .top(px(14.0))
            .child(name_row)
            .children(subtitle.map(|subtitle| {
                MainText::new(subtitle)
                    .px(13.0)
                    .tac(TextAlignT::Left)
                    .color(theme.text.secondary)
                    .number_of_lines(1)
            }))
    }

    fn premium_badge(&self, can_edit: bool, cx: &App) -> AnyElement {
        let badge = PremiumBadge::new()
            .user_id(Some(SharedString::from(self.profile.id.clone())))
            .emoji(self.profile.more.premium_badge_emoji.clone().map(SharedString::from))
            .size(22.0);

        if !can_edit {
            return badge.into_any_element();
        }
        let premium_badge_anchor = self.state.read(cx).premium_badge_anchor.clone();
        let tap_anchor = premium_badge_anchor.clone();
        div()
            .id(self.element_id("premium-badge"))
            .py(px(4.0))
            .cursor_pointer()
            .aria_label(tr("settings_premium_badge_title"))
            .tooltip_label(tr("settings_premium_badge_title"))
            .child(div().relative().child(badge).view_anchor_probe(&premium_badge_anchor))
            .on_click(move |_: &ClickEvent, window, cx| {
                let anchor =
                    tap_anchor.window_frame().unwrap_or_else(|| PremiumBadgePopoverLayout::detached_anchor(window));
                PremiumBadgePickerPopover::shared(cx).update(cx, |popover, cx| popover.present(anchor, cx));
            })
            .into_any_element()
    }

    fn rise_pills(&self, theme: &AppTheme, cx: &mut App) -> Vec<RisePill> {
        let src = self.gamification_source(cx);
        let mut out = Vec::new();
        if let Some(lvl) = src.level.filter(|lvl| *lvl > 0) {
            out.push(RisePill {
                id: "level",
                icon: "arrow.up.forward.circle.fill",
                tint: theme.primary._100,
                value: lvl.to_string(),
                label: tr("rise_level"),
            });
        }
        if let Some(streak) = src.streak.filter(|streak| *streak > 0) {
            out.push(RisePill {
                id: "streak",
                icon: "flame.fill",
                tint: UIColor::system_orange().resolved_color(theme.appearance),
                value: streak.to_string(),
                label: tr("rise_streak"),
            });
        }
        if let Some(rating) = src.rating.filter(|rating| *rating > 0) {
            out.push(RisePill {
                id: "rating",
                icon: "star.fill",
                tint: UIColor::system_yellow().resolved_color(theme.appearance),
                value: rating.to_string(),
                label: tr("rise_rating"),
            });
        }
        out
    }

    fn rise_hud(&self, theme: &AppTheme, cx: &mut App) -> Option<Div> {
        if self.treat_as_foreign {
            return None;
        }
        let pills = self.rise_pills(theme, cx);
        if pills.is_empty() {
            return None;
        }
        Some(
            hstack(8.0)
                .w_full()
                .px(px(16.0))
                .children(pills.into_iter().map(|pill| {
                    hstack(5.0)
                        .id(self.element_id(pill.id))
                        .relative()
                        .px(px(12.0))
                        .py(px(7.0))
                        .child(self.profile_capsule_surface(theme))
                        .children(icon(pill.icon, 12.0, pill.tint))
                        .child(MainText::new(pill.value).px(13.0).font_weight(FontWeight::SEMIBOLD))
                        .child(MainText::new(pill.label).px(11.0).color(theme.text.secondary))
                }))
                .child(spacer_min_w(0.0)),
        )
    }

    fn stats_bar(&self, theme: &AppTheme, horizontal_padding: f32) -> Div {
        let subs_hidden = self.treat_as_foreign && self.profile.subs_count_hidden;
        let user_id = Some(self.profile.id.clone());
        let segs: [(i64, &str, bool, Option<RootRoute>); 4] = [
            (self.profile.posts_count, "stats_posts", false, None),
            (
                self.profile.subscribers_count,
                "stats_subscribers",
                false,
                Some(RootRoute::UserSubscribers { user_id: user_id.clone() }),
            ),
            (self.profile.subs_count, "stats_subs", subs_hidden, Some(RootRoute::UserSubs { user_id: user_id.clone() })),
            (self.profile.friends_count, "stats_friends", false, Some(RootRoute::UserFriends { user_id })),
        ];
        hstack(0.0).w_full().px(px(horizontal_padding)).children(segs.into_iter().map(|(amount, key, hidden, route)| {
            let body = self.clone();
            let value: AnyElement = if self.is_resolving || self.counts_resolving {
                div()
                    .relative()
                    .child(div().invisible().child(hidden_line(16.0)))
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(SkeletonLine::new(28.0, 12.0).skeleton_shimmer(None)),
                    )
                    .into_any_element()
            } else {
                MainText::new(if hidden { HIDDEN_COUNT.to_owned() } else { RiseFormat::compact_count(amount) })
                    .px(16.0)
                    .tac(TextAlignT::Center)
                    .font_weight(FontWeight::BOLD)
                    .number_of_lines(1)
                    .minimum_scale_factor(0.6)
                    .into_any_element()
            };
            plain(self.element_id(key))
                .flex_1()
                .min_w_0()
                .h(px(48.0))
                .px(px(2.0))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap(px(2.0))
                .on_press(move |_, cx| {
                    if let Some(route) = route.clone() {
                        body.push(route, cx);
                    }
                })
                .child(value)
                .child(
                    MainText::new(tr_plural(key, amount))
                        .px(10.5)
                        .tac(TextAlignT::Center)
                        .color(theme.text.secondary)
                        .number_of_lines(1)
                        .minimum_scale_factor(0.6),
                )
        }))
    }

    fn profile_capsule_surface(&self, theme: &AppTheme) -> Div {
        Self::profile_static_glass_surface(FULLY_ROUND_RADIUS, theme)
    }

    fn profile_circle_surface(&self, theme: &AppTheme) -> Div {
        Self::profile_static_glass_surface(FULLY_ROUND_RADIUS, theme)
    }

    fn profile_static_glass_surface(corner_radius: f32, theme: &AppTheme) -> Div {
        let half_line = px(0.65 / 2.0);
        div()
            .absolute()
            .inset_0()
            .child(div().absolute().inset_0().child(rounded_gradient_fill(
                LinearGradient::colors(
                    [theme.bg._400.opacity(0.78), theme.bg._200.opacity(0.70)],
                    UnitPoint::TOP_LEADING,
                    UnitPoint::BOTTOM_TRAILING,
                ),
                corner_radius,
            )))
            .child(div().absolute().inset_0().child(rounded_gradient_fill(
                LinearGradient::colors(
                    [white().opacity(0.11), white().opacity(0.025), black().opacity(0.07)],
                    UnitPoint::TOP,
                    UnitPoint::BOTTOM,
                ),
                corner_radius,
            )))
            .child(
                div().absolute().top(-half_line).left(-half_line).right(-half_line).bottom(-half_line).child(
                    rounded_gradient_stroke_border(
                        LinearGradient::colors(
                            [white().opacity(0.22), white().opacity(0.07), theme.border._300.opacity(0.72)],
                            UnitPoint::TOP_LEADING,
                            UnitPoint::BOTTOM_TRAILING,
                        ),
                        corner_radius,
                        0.65,
                    ),
                ),
            )
    }

    fn action_buttons(&self, theme: &AppTheme, cx: &mut App) -> Div {
        let chat = self.clone();
        let share = self.clone();
        hstack(8.0)
            .w_full()
            .px(px(16.0))
            .child(self.subscribe_button(theme, cx))
            .child(self.circle_surface("direct-chat", "bubble.left.and.bubble.right.fill", theme, move |_, cx| {
                chat.open_direct_chat(cx);
            }))
            .child(self.gift_menu(theme))
            .child(self.circle_surface("share", "square.and.arrow.up", theme, move |_, cx| share.present_share(cx)))
    }

    fn present_share(&self, cx: &mut App) {
        let profile = (*self.profile).clone();
        ShareSheetStore::shared(cx).update(cx, |store, cx| store.present_profile(profile, cx));
    }

    fn gift_menu(&self, theme: &AppTheme) -> PlainButton {
        let body = self.clone();
        self.circle_surface("gift", "gift.fill", theme, move |_, cx| {
            let profile = &body.profile;
            let recipient = GiftRecipient {
                id: profile.id.clone(),
                name: profile.name.clone(),
                tag: profile.tag.clone(),
                avatar: profile.avatar_url.clone().unwrap_or_else(|| profile.more.logo.clone()),
                cover: profile.cover_image_url.clone(),
            };
            GiftsInteractionsStore::shared(cx).update(cx, |store, cx| store.open_shop(recipient, RiseGiftItemKind::Item, cx));
        })
    }

    fn circle_surface(
        &self,
        key: &str,
        symbol: &str,
        theme: &AppTheme,
        action: impl Fn(&mut Window, &mut App) + 'static,
    ) -> PlainButton {
        let side = px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE);
        plain(self.element_id(key))
            .relative()
            .flex_none()
            .size(side)
            .flex()
            .items_center()
            .justify_center()
            .on_press(action)
            .child(self.profile_circle_surface(theme))
            .children(icon(symbol, 16.0, theme.text.primary))
    }

    fn shown_relationship(&self) -> UserListRelationship {
        UserListRelationship {
            is_subbed: self.profile.is_subbed,
            is_subscriber: self.profile.is_subscriber,
            is_friend: self.profile.is_friend,
        }
    }

    fn toggle_subscribe(&self, cx: &mut App) {
        let relationship = self.shown_relationship();
        UserInteractionsStore::shared(cx).update(cx, |store, cx| {
            store.toggle_subscribe_relationship(&self.profile.id, &self.profile.tag, relationship, cx);
        });
    }

    fn subscribe_button(&self, theme: &AppTheme, cx: &mut App) -> PlainButton {
        let state = self.profile.follow_button_state();
        let is_primary = state.is_primary();
        let follow_refused = self.blocks_follow_intent(state, cx);
        let content_color = if is_primary { theme.primary._100 } else { theme.text.primary };
        let body = self.clone();
        plain(self.element_id("subscribe"))
            .relative()
            .flex_1()
            .min_w_0()
            .h(px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE))
            .px(px(12.0))
            .flex()
            .items_center()
            .justify_center()
            .gap(px(6.0))
            .opacity(if follow_refused { 0.4 } else { 1.0 })
            .disabled(follow_refused)
            .on_press(move |_, cx| {
                if body.profile.id.is_empty() || follow_refused {
                    return;
                }
                if state == FollowButtonState::Friends {
                    body.show_remove_friend_confirm(cx);
                } else {
                    body.toggle_subscribe(cx);
                }
            })
            .child(self.profile_capsule_surface(theme))
            .children((state == FollowButtonState::Friends).then(|| icon("person.2.fill", 12.0, content_color)).flatten())
            .child(
                MainText::new(tr(state.title_key())).px(14.0).font_weight(FontWeight::SEMIBOLD).color(content_color),
            )
    }

    fn show_remove_friend_confirm(&self, cx: &mut App) {
        let body = self.clone();
        cx.defer(move |cx| {
            let guide = GuideOverlayHost::shared(cx);
            let host = if guide.is_presenting() {
                guide.alert_host()
            } else {
                cx.active_window().or_else(|| cx.windows().first().copied())
            };
            let buttons =
                [PromptButton::ok(tr("profile_remove_friend_confirm")), PromptButton::cancel(tr("common_cancel"))];
            let title = tr("profile_remove_friend_title");
            let message = tr("profile_remove_friend_message");
            let receiver: Option<oneshot::Receiver<usize>> = host.and_then(|host| {
                host.update(cx, |_, window, cx| {
                    window.prompt(PromptLevel::Warning, &title, Some(message.as_str()), &buttons, cx)
                })
                .ok()
            });
            let Some(receiver) = receiver else {
                return;
            };
            let confirm = body.clone();
            body.state.update(cx, |state, cx| {
                state.remove_friend_alert = Some(cx.spawn(async move |this: WeakEntity<ProfileTopState>, cx| {
                    let answer = receiver.await.ok();
                    let _ = this.update(cx, |state, cx| {
                        state.remove_friend_alert = None;
                        if answer == Some(0) && !confirm.profile.id.is_empty() {
                            confirm.toggle_subscribe(cx);
                        }
                    });
                }));
            });
        });
    }

    fn about_section(&self, theme: &AppTheme, cx: &mut App) -> Div {
        let desc = self
            .profile
            .description
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .unwrap_or_default()
            .to_owned();
        let owner_block_scope = self.owner_block_scope(cx);
        let owner_totally_blocked_me = owner_block_scope == Some(RiseBlockScope::Total);
        let social_links = self.social_links();
        let featured = self.featured_tracks(cx);

        vstack(8.0)
            .w_full()
            .items_start()
            .px(px(16.0))
            .children((!owner_totally_blocked_me).then(|| div().w_full().pt(px(3.0)).child(self.stats_bar(theme, 0.0))))
            .children((!desc.is_empty()).then(|| {
                MainText::new(tr("profile_about_title")).px(13.0).tac(TextAlignT::Left).color(theme.text.secondary).m_l(4.0)
            }))
            .children((!desc.is_empty()).then(|| {
                div()
                    .w_full()
                    .px(px(4.0))
                    .child(
                        ExpandableTextUi::new(
                            self.element_id("description"),
                            desc.clone(),
                            theme.text.primary.opacity(0.92),
                            theme.text.secondary,
                        )
                        .font_size(15.0)
                        .line_limit(4),
                    )
                    .modifier(ProfileTopScrollFadeModifier::new(true).masks_only(true))
            }))
            .children(featured.first().cloned().map(|track| {
                let body = self.clone();
                let track_id = track.id.clone();
                div()
                    .w_full()
                    .min_h(px(30.0))
                    .px(px(4.0))
                    .flex()
                    .items_center()
                    .child(
                        ProfileFeaturedTrackRow::new(self.element_id("featured-track"), track)
                            .queue(featured.clone())
                            .scope_id(ProfileFeaturedTracksPlayback::scope_id(&self.profile.id))
                            .on_open(move |_, cx| {
                                let track_id = track_id.clone();
                                body.state.update(cx, |state, cx| {
                                    state.featured_track_id = Some(track_id);
                                    state.show_featured_tracks = true;
                                    cx.notify();
                                });
                            })
                            .guide_anchor((!self.treat_as_foreign).then_some(RiseGuideAnchor::ProfileFeaturedTrack)),
                    )
            }))
            .children((featured.is_empty() && self.counts_resolving && !self.is_resolving).then(|| {
                div()
                    .w_full()
                    .min_h(px(30.0))
                    .px(px(4.0))
                    .flex()
                    .items_center()
                    .child(SkeletonLine::new(180.0, 14.0).skeleton_shimmer(None))
            }))
            .child(self.about_buttons(owner_block_scope, theme))
            .children((!social_links.is_empty()).then(|| {
                let body = self.clone();
                div()
                    .w_full()
                    .min_h(px(30.0))
                    .px(px(4.0))
                    .flex()
                    .items_center()
                    .child(SocialLinksPreviewRow::new(
                        self.element_id("social-links"),
                        social_links.clone(),
                        move |_, cx| {
                            body.state.update(cx, |state, cx| {
                                state.show_social_links = true;
                                cx.notify();
                            });
                        },
                    ))
            }))
    }

    fn about_buttons(&self, owner_block_scope: Option<RiseBlockScope>, theme: &AppTheme) -> Div {
        let more_button = owner_block_scope.is_none().then(|| {
            let body = self.clone();
            plain(self.element_id("more"))
                .relative()
                .flex_1()
                .min_w_0()
                .h(px(PageHeaderMetrics::TOOLBAR_BUTTON_SIZE))
                .px(px(12.0))
                .flex()
                .items_center()
                .gap(px(8.0))
                .on_press(move |_, cx| {
                    body.state.update(cx, |state, cx| {
                        state.show_about = true;
                        cx.notify();
                    });
                })
                .child(self.profile_capsule_surface(theme))
                .children(icon("info.circle", 15.0, theme.text.primary))
                .child(MainText::new(tr("profile_more_button")).px(14.0).font_weight(FontWeight::SEMIBOLD))
                .child(spacer_min_w(0.0))
                .children(icon(rise_ui::IconUi::trailing_chevron(), 12.0, theme.text.secondary))
        });
        let own_buttons = (!self.treat_as_foreign).then(|| {
            let edit = self.clone();
            let share = self.clone();
            [
                self.circle_surface("edit", "pencil", theme, move |_, cx| {
                    edit.push(RootRoute::ProfileSettings, cx);
                })
                .guide_anchor(RiseGuideAnchor::ProfileEdit)
                .into_any_element(),
                self.circle_surface("share-own", "square.and.arrow.up", theme, move |_, cx| share.present_share(cx))
                    .into_any_element(),
            ]
        });
        hstack(8.0).w_full().children(more_button).children(own_buttons.into_iter().flatten())
    }

    fn open_direct_chat(&self, cx: &mut App) {
        if self.profile.id.is_empty() {
            return;
        }
        let target = User {
            id: self.profile.id.clone(),
            name: self.profile.name.clone(),
            tag: self.profile.tag.clone(),
            more: self.profile.more.clone(),
            is_bot: self.profile.is_bot,
            bot_owner_id: self.profile.bot_owner_id.clone(),
            bot_is_active: self.profile.bot_is_active,
            ..User::default()
        };
        let chat_id =
            ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.direct_conversation_id(&target, cx));
        if chat_id.is_empty() {
            return;
        }
        self.push(RootRoute::Chat { chat_id: Some(chat_id) }, cx);
    }
}

/// blank line that holds its font height behind a placeholder
fn hidden_line(px_size: f32) -> MainText {
    MainText::new(" ").px(px_size).font_weight(FontWeight::BOLD).number_of_lines(1)
}

fn featured_tracks_snap_fractions(featured_tracks_lowest_fraction: f32) -> Vec<f32> {
    let top = FEATURED_TRACKS_FRACTIONS[1];
    if featured_tracks_lowest_fraction < top {
        vec![featured_tracks_lowest_fraction, top]
    } else {
        vec![top]
    }
}

fn update_featured_tracks_lowest_fraction(
    state: &Entity<ProfileTopState>,
    content_height: f32,
    window: &mut Window,
    cx: &mut App,
) {
    let screen_height = f32::from(window.viewport_size().height);
    if screen_height <= 0.0 {
        return;
    }
    let chrome = BottomSheetMetrics::HANDLE_TOP_PADDING
        + BottomSheetMetrics::HANDLE_HEIGHT
        + BottomSheetMetrics::HANDLE_BOTTOM_PADDING;
    let needed = (content_height + chrome) / screen_height;
    let fraction = FEATURED_TRACKS_FRACTIONS[1].min(FEATURED_TRACKS_FRACTIONS[0].max(needed));
    state.update(cx, |state, cx| {
        if (fraction - state.featured_tracks_lowest_fraction).abs() <= 0.005 {
            return;
        }
        state.featured_tracks_lowest_fraction = fraction;
        cx.notify();
    });
}
