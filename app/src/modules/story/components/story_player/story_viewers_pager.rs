use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, Bounds, ClickEvent, Context, ElementId, Empty, Entity, FontWeight, Hsla, IntoElement, ListAlignment,
    MouseButton, MouseDownEvent, Pixels, Point, Render, RenderOnce, ScrollDelta, ScrollWheelEvent, SharedString,
    StyleRefinement, Subscription, Svg, Task, Window, black, point, size, white,
};
use rise_core::foundation::foundation_date::{Date, RiseClock};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_platform::wheel_gesture::{WHEEL_SETTLE_DELAY, WheelGestureTracker, WheelStep};
use rise_theme::{AppTheme, ThemeStore};
use rise_ui::error_text_ui::system_red;
use rise_ui::MainText;
use rise_ui::icon_ui::IconUi;
use rise_ui::list_ui::list_ui::ListUi;
use rise_ui::search_field_ui::SearchFieldUi;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::{MotionSpec, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::scroll_view::scroll_view;

use crate::app::router::navigation_store::{NavigationStore, NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::sai::modules::sai_ws::sai_cache_revision_tracker::SaiCacheRevisionTracker;
use crate::core::lib::sai::modules::sai_ws::sai_ws_pending_registry::SaiWsPendingRegistry;
use crate::core::ui::premium_badge::PremiumBadge;
use crate::core::ui::premium_name_color::PremiumNameColor;
use crate::core::ui::sai_pagination_prefetch::{SaiPaginationPrefetchModifiers, SaiPaginationPrefetchOptions};
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::post::components::post::post_music_plate_overlay::MusicPlateOverlay;
use crate::modules::story::components::story_player::story_media_view::StoryMediaView;
use crate::modules::story::components::story_thumbnail_ui::StoryThumbnailUi;
use crate::modules::story::stores::story::story_actions::story_wire_types::{StoryDTO, StoryDTOExt, StoryViewerDTO};
use crate::modules::story::stores::story::story_interactions::story_interactions::StoryInteractionsStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::user::components::profile_content::pages::grid_posts::grid_posts::ProfileThumbnailDecodePolicy;
use crate::modules::user::components::user_list_skeleton::UserListSkeleton;

/// callback of the owner
pub type StoryViewersPagerAction = Rc<dyn Fn(&mut Window, &mut App)>;

type StoryViewersHeaderHeightHandler = Rc<dyn Fn(f32, &mut Window, &mut App)>;
type StoryViewersOpenViewerHandler = Rc<dyn Fn(&StoryViewerDTO, &mut Window, &mut App)>;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum StoryViewersTab {
    All,
    Contacts,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PagerAxis {
    Undecided,
    Horizontal,
    Vertical,
}

/// which drag of the pager a wheel gesture stands in for
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum WheelSurface {
    Band,
    Sheet,
}

#[derive(Clone, Copy, Debug)]
struct WheelSession {
    surface: WheelSurface,
    start_location: Point<Pixels>,
    is_recognized: bool,
}

/// filters shared by every sheet page so paging to another story keeps tab and query
/// separate entity so pages observe it and not the per frame gesture state
struct StoryViewersPagerFilters {
    tab: StoryViewersTab,
    liked_only: bool,
    query: String,
}

struct StoryViewersPagerState {
    geo: Option<Bounds<Pixels>>,
    displayed_position: Animated<f32>,
    drag_axis: PagerAxis,
    collapse_start: Option<f32>,
    sheet_header_height: f32,
    band_was_dragged: bool,
    band_hovered: bool,
    wheel: WheelGestureTracker,
    wheel_session: Option<WheelSession>,
    wheel_settle: Option<Task<()>>,
    media_aspects: HashMap<i64, f32>,
}

impl StoryViewersPagerState {
    fn new(index: i64) -> Self {
        Self {
            geo: None,
            displayed_position: Animated::new(index as f32),
            drag_axis: PagerAxis::Undecided,
            collapse_start: None,
            sheet_header_height: 0.0,
            band_was_dragged: false,
            band_hovered: false,
            wheel: WheelGestureTracker::new(),
            wheel_session: None,
            wheel_settle: None,
            media_aspects: HashMap::new(),
        }
    }
}

#[derive(IntoElement)]
pub struct StoryViewersPager {
    id: ElementId,
    stories: Vec<StoryDTO>,
    index: Binding<i64>,
    viewers_progress: Entity<Animated<f32>>,
    progress: f32,
    owns_active_media: bool,
    top_inset: f32,
    bottom_inset: f32,
    screen_height: f32,
    allows_hit_testing: bool,
    on_request_dismiss: StoryViewersPagerAction,
    on_dismiss: StoryViewersPagerAction,
}

impl StoryViewersPager {
    /// viewers_progress is an animated value the owner keeps so every reader follows the settle spring
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        stories: Vec<StoryDTO>,
        index: Binding<i64>,
        viewers_progress: Entity<Animated<f32>>,
        progress: f32,
        top_inset: f32,
        bottom_inset: f32,
        on_request_dismiss: impl Fn(&mut Window, &mut App) + 'static,
        on_dismiss: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            stories,
            index,
            viewers_progress,
            progress,
            owns_active_media: false,
            top_inset,
            bottom_inset,
            screen_height: 0.0,
            allows_hit_testing: true,
            on_request_dismiss: Rc::new(on_request_dismiss),
            on_dismiss: Rc::new(on_dismiss),
        }
    }

    pub fn owns_active_media(mut self, owns_active_media: bool) -> Self {
        self.owns_active_media = owns_active_media;
        self
    }

    pub fn screen_height(mut self, screen_height: f32) -> Self {
        self.screen_height = screen_height;
        self
    }

    /// gpui cant switch a subtree off from outside, so the pager takes this and drops its own hit areas
    pub fn allows_hit_testing(mut self, allows_hit_testing: bool) -> Self {
        self.allows_hit_testing = allows_hit_testing;
        self
    }
}

impl RenderOnce for StoryViewersPager {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let index = self.index.get(cx);
        let state = window.use_keyed_state(child_id(&self.id, "state"), cx, move |_, _| StoryViewersPagerState::new(index));
        let filters = window.use_keyed_state(child_id(&self.id, "filters"), cx, |_, _| StoryViewersPagerFilters {
            tab: StoryViewersTab::All,
            liked_only: false,
            query: String::new(),
        });

        let parts = StoryViewersPagerParts {
            id: self.id,
            stories: Rc::new(self.stories),
            index: self.index,
            viewers_progress: self.viewers_progress,
            progress: self.progress,
            owns_active_media: self.owns_active_media,
            top_inset: self.top_inset,
            bottom_inset: self.bottom_inset,
            screen_height: self.screen_height,
            allows_hit_testing: self.allows_hit_testing,
            on_request_dismiss: self.on_request_dismiss,
            on_dismiss: self.on_dismiss,
            state,
            filters,
            service: StoryServicesStore::shared(cx),
            nav_store: env::<NavigationStoreKey>(cx),
        };

        parts.sync_displayed_position(index, cx);

        let active_story_id = parts.active_story_id(index);
        let ensure_parts = parts.clone();
        on_change(
            parts.child_id("active-story-id"),
            active_story_id,
            true,
            window,
            cx,
            move |_, id, _, cx| ensure_parts.ensure_loaded(*id, cx),
        );

        let measure_state = parts.state.clone();
        let geometry = measure(parts.child_id("geometry"), move |bounds, _, cx| {
            measure_state.update(cx, |state, cx| {
                state.geo = Some(bounds);
                cx.notify();
            });
        });

        let clamped_progress = parts.clamped_progress();
        let root = div()
            .relative()
            .size_full()
            .overflow_hidden()
            .opacity(if clamped_progress <= 0.001 && !parts.owns_active_media { 0.0 } else { 1.0 })
            .child(geometry);

        let Some(geo) = parts.state.read(cx).geo else {
            return root.into_any_element();
        };
        let width = f32::from(geo.size.width);
        let height = f32::from(geo.size.height);

        let wheel_parts = parts.clone();
        root.id(parts.child_id("root"))
            .on_scroll_wheel(move |event: &ScrollWheelEvent, window, cx| wheel_parts.handle_scroll_wheel(event, window, cx))
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(black().opacity((clamped_progress / 0.18).min(1.0)))
                    .when(parts.allows_hit_testing, |layer| layer.block_mouse_except_scroll()),
            )
            .child(parts.sheet_strip(width, height, window, cx))
            .child(parts.cards_layer(width, height, &theme, window, cx))
            .into_any_element()
    }
}

fn child_id(id: &ElementId, name: &'static str) -> ElementId {
    ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name))
}

fn symbol(name: &str, side: f32, color: Hsla) -> Option<Svg> {
    IconUi::sized(name, px(side), color, name.ends_with(".fill"))
}

#[derive(Clone)]
struct StoryViewersPagerParts {
    id: ElementId,
    stories: Rc<Vec<StoryDTO>>,
    index: Binding<i64>,
    viewers_progress: Entity<Animated<f32>>,
    progress: f32,
    owns_active_media: bool,
    top_inset: f32,
    bottom_inset: f32,
    screen_height: f32,
    allows_hit_testing: bool,
    on_request_dismiss: StoryViewersPagerAction,
    on_dismiss: StoryViewersPagerAction,
    state: Entity<StoryViewersPagerState>,
    filters: Entity<StoryViewersPagerFilters>,
    service: Entity<StoryServicesStore>,
    nav_store: Option<Entity<NavigationStore>>,
}

impl StoryViewersPagerParts {
    const CARD_ASPECT: f32 = 0.6;
    const SIDE_GAP: f32 = 10.0;

    fn child_id(&self, name: &'static str) -> ElementId {
        child_id(&self.id, name)
    }

    fn named_id(&self, name: String) -> ElementId {
        ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::from(name))
    }

    fn stories_count(&self) -> i64 {
        self.stories.len() as i64
    }

    fn active_story(&self, index: i64) -> Option<&StoryDTO> {
        if index < 0 || index >= self.stories_count() {
            return self.stories.first();
        }
        self.stories.get(index as usize)
    }

    fn active_story_id(&self, index: i64) -> i64 {
        self.active_story(index).map(|story| story.id).unwrap_or(0)
    }

    fn clamped_progress(&self) -> f32 {
        self.progress.clamp(0.0, 1.0)
    }

    fn sync_displayed_position(&self, index: i64, cx: &mut App) {
        self.state.update(cx, |state, _| {
            if state.drag_axis != PagerAxis::Horizontal && *state.displayed_position.target() != index as f32 {
                state.displayed_position.jump(index as f32);
            }
        });
    }

    fn ensure_loaded(&self, story_id: i64, cx: &mut App) {
        if story_id == 0 || self.service.update(cx, |service, cx| service.story_viewers_loaded(story_id, cx)) {
            return;
        }
        StoryInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_story_viewers(story_id, None, cx);
        });
    }

    fn lerp(a: f32, b: f32, t: f32) -> f32 {
        a + (b - a) * t.clamp(0.0, 1.0)
    }

    fn stable_height(&self, geo_height: f32) -> f32 {
        if self.screen_height > 0.0 { self.screen_height } else { geo_height }
    }

    fn safe_zone_height(&self, h: f32) -> f32 {
        (h - self.top_inset - self.bottom_inset).max(1.0)
    }

    /// geometry comes from the passed progress not the current one,
    /// the sheet needs the open detent geometry for constant height
    fn center_card_width(w: f32, p: f32) -> f32 {
        if p <= 0.5 {
            return Self::lerp(w, w * 0.66, p / 0.5);
        }
        Self::lerp(w * 0.66, w * 0.4, (p - 0.5) / 0.5)
    }

    fn side_card_width(w: f32, p: f32) -> f32 {
        Self::center_card_width(w, p) * 0.8
    }

    fn center_card_height(&self, w: f32, h: f32, p: f32) -> f32 {
        let portrait = Self::center_card_width(w, p) / Self::CARD_ASPECT;
        if p <= 0.5 {
            return Self::lerp(self.safe_zone_height(h), w * 0.66 / Self::CARD_ASPECT, p / 0.5);
        }
        portrait
    }

    fn card_center_y(&self, w: f32, h: f32, p: f32) -> f32 {
        let start_y = self.top_inset + self.safe_zone_height(h) / 2.0;
        let open_y = self.top_inset + 8.0 + (w * 0.66 / Self::CARD_ASPECT) / 2.0;
        if p <= 0.5 {
            return Self::lerp(start_y, open_y, p / 0.5);
        }
        let strip_y = self.top_inset + 8.0 + self.center_card_height(w, h, p) / 2.0;
        Self::lerp(open_y, strip_y, (p - 0.5) / 0.5)
    }

    fn sheet_top(&self, w: f32, h: f32, p: f32) -> f32 {
        let card_bottom = self.card_center_y(w, h, p) + self.center_card_height(w, h, p) / 2.0 + 14.0;
        card_bottom.max(self.top_inset)
    }

    /// pager size and stable height, once the pager has been measured
    fn geometry(&self, cx: &App) -> Option<(f32, f32)> {
        let geo = self.state.read(cx).geo?;
        Some((f32::from(geo.size.width), self.stable_height(f32::from(geo.size.height))))
    }

    fn cards_layer(&self, w: f32, geo_height: f32, theme: &AppTheme, window: &mut Window, cx: &mut App) -> AnyElement {
        let h = self.stable_height(geo_height);
        let now = RiseClock::monotonic();
        let pos = self.state.read(cx).displayed_position.frame_value(now, window);
        let index = self.index.get(cx);
        let p = self.clamped_progress();
        let corner_radius = Self::lerp(0.0, 16.0, (p / 0.4).min(1.0));

        let mut layer = pass_through().absolute().top_0().left_0().w(px(w)).h(px(h));
        for story_idx in Self::visible_card_indexes(index) {
            if story_idx >= 0 && story_idx < self.stories_count() {
                let story = &self.stories[story_idx as usize];
                layer = layer.child(self.story_card(story, story_idx as f32 - pos, w, h, corner_radius, theme, window, cx));
            }
        }

        let band = (p > 0.05 && self.allows_hit_testing).then(|| {
            let down_state = self.state.clone();
            let hover_state = self.state.clone();
            let tap_parts = self.clone();
            let close = self.state.read(cx).band_hovered.then(|| self.close_control(w, h, p));
            div().absolute().top_0().left_0().child(
                div()
                    .id(self.child_id("band"))
                    .relative()
                    .w(px(w))
                    .h(px(self.band_gesture_height(w, h)))
                    .on_hover(move |hovered: &bool, _, cx| {
                        let hovered = *hovered;
                        hover_state.update(cx, |state, cx| {
                            if state.band_hovered != hovered {
                                state.band_hovered = hovered;
                                cx.notify();
                            }
                        });
                    })
                    .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, _, cx| {
                        down_state.update(cx, |state, _| state.band_was_dragged = false);
                    })
                    .on_click(move |event: &ClickEvent, window, cx| tap_parts.handle_band_tap(event, window, cx))
                    .children(close)
                    .gesture(self.child_id("band-drag"), self.unified_drag()),
            )
        });

        div().absolute().inset_0().child(layer).children(band).into_any_element()
    }

    fn handle_band_tap(&self, event: &ClickEvent, window: &mut Window, cx: &mut App) {
        if self.state.read(cx).band_was_dragged {
            return;
        }
        let Some(geo) = self.state.read(cx).geo else {
            return;
        };
        let w = f32::from(geo.size.width);
        let center_w = Self::center_card_width(w, self.clamped_progress());
        let half = center_w / 2.0;
        let dx = f32::from(event.position().x - geo.origin.x) - w / 2.0;
        let index = self.index.get(cx);
        if dx > half {
            self.tap_card(index + 1, window, cx);
        } else if dx < -half {
            self.tap_card(index - 1, window, cx);
        } else {
            self.settle_viewers(0.0, cx);
        }
    }

    fn close_control(&self, w: f32, h: f32, p: f32) -> AnyElement {
        const SIDE: f32 = 30.0;
        const INSET: f32 = 8.0;
        let card_right = w / 2.0 + Self::center_card_width(w, p) / 2.0;
        let card_top = self.card_center_y(w, h, p) - self.center_card_height(w, h, p) / 2.0;
        let parts = self.clone();
        div()
            .absolute()
            .left(px(card_right - SIDE - INSET))
            .top(px(card_top + INSET))
            .occlude()
            .child(
                plain(self.child_id("close"))
                    .on_press(move |_, cx| parts.settle_viewers(0.0, cx))
                    .w(px(SIDE))
                    .h(px(SIDE))
                    .rounded_full()
                    .bg(black().opacity(0.55))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(symbol("xmark", 14.0, white())),
            )
            .into_any_element()
    }

    fn band_gesture_height(&self, w: f32, h: f32) -> f32 {
        self.sheet_top(w, h, self.clamped_progress())
    }

    fn visible_card_indexes(index: i64) -> [i64; 5] {
        [index - 2, index - 1, index, index + 1, index + 2]
    }

    #[allow(clippy::too_many_arguments)]
    fn story_card(
        &self,
        story: &StoryDTO,
        distance: f32,
        screen_w: f32,
        screen_h: f32,
        corner_radius: f32,
        theme: &AppTheme,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let p = self.clamped_progress();
        let d = distance.abs().min(1.0);
        let center_w = Self::center_card_width(screen_w, p);
        let center_h = self.center_card_height(screen_w, screen_h, p);
        let side_w = Self::side_card_width(screen_w, p);
        let side_h = side_w / Self::CARD_ASPECT;

        let card_w = Self::lerp(center_w, side_w, d);
        let card_h = Self::lerp(center_h, side_h, d);

        let step = center_w / 2.0 + Self::SIDE_GAP + side_w / 2.0;
        let x = screen_w / 2.0 + distance * step;
        let y = self.card_center_y(screen_w, screen_h, p);

        let owns_center = self.owns_active_media && distance.abs() < 0.5;
        let appear = if owns_center { 1.0 } else { ((p - 0.04) / 0.16).min(1.0) };
        let alpha = (Self::lerp(1.0, 0.86, d) * appear).clamp(0.0, 1.0);

        let poster_dimension = ProfileThumbnailDecodePolicy::maximum_pixel_dimension(
            size(px(screen_w * 0.66), px(screen_w * 0.66 / Self::CARD_ASPECT)),
            window.scale_factor(),
        );

        pass_through()
            .absolute()
            .left(px(x - card_w / 2.0))
            .top(px(y - card_h / 2.0))
            .w(px(card_w))
            .h(px(card_h))
            .opacity(alpha)
            .child(
                zstack(Alignment::Bottom)
                    .size_full()
                    .child(self.card_media(story, card_w, card_h, corner_radius, poster_dimension, theme))
                    .children(self.card_music_plate(story, cx))
                    .child(
                        div()
                            .pb(px(8.0))
                            .opacity(((p - 0.12) / 0.25).clamp(0.0, 1.0))
                            .child(self.card_counts(story, cx)),
                    ),
            )
            .into_any_element()
    }

    fn card_media(
        &self,
        story: &StoryDTO,
        card_w: f32,
        card_h: f32,
        corner_radius: f32,
        poster_dimension: i64,
        theme: &AppTheme,
    ) -> AnyElement {
        let cover = story.media_source();
        if cover.is_empty() {
            return div()
                .w(px(card_w))
                .h(px(card_h))
                .continuous_corners(corner_radius)
                .bg(theme.bg._200)
                .into_any_element();
        }
        if story.media_type.to_lowercase() == "video" {
            return StoryThumbnailUi::from_story(story, card_w, card_h, poster_dimension)
                .corner_radius(corner_radius)
                .id(self.named_id(format!("card-thumbnail-{}", story.id)))
                .into_any_element();
        }
        div()
            .w(px(card_w))
            .h(px(card_h))
            .clip_shape(Shape::RoundedRectangle { corner_radius, style: RoundedCornerStyle::Continuous })
            .child(
                StoryMediaView::new(self.named_id(format!("card-media-{}", story.id)), cover, 0.0)
                    .show_blurred_fill(true)
                    .pins_aspect_fit(true)
                    .on_media_geometry({
                        let state = self.state.clone();
                        let story_id = story.id;
                        move |aspect, _, _, cx| {
                            let Some(aspect) = aspect else {
                                return;
                            };
                            state.update(cx, |state, cx| {
                                if state.media_aspects.get(&story_id) != Some(&aspect) {
                                    state.media_aspects.insert(story_id, aspect);
                                    cx.notify();
                                }
                            });
                        }
                    }),
            )
            .into_any_element()
    }

    fn card_music_plate(&self, story: &StoryDTO, cx: &App) -> Option<AnyElement> {
        let plate = story.track_plate.as_ref()?;
        let is_photo = !story.media_source().is_empty() && story.media_type.to_lowercase() != "video";
        let aspect = if is_photo { self.state.read(cx).media_aspects.get(&story.id).copied() } else { None };
        Some(
            pass_through()
                .absolute()
                .inset_0()
                .child(
                    MusicPlateOverlay::new(
                        SharedString::from(format!("viewers-card-plate-{}", story.id)),
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
                    .media_fills(!is_photo),
                )
                .into_any_element(),
        )
    }

    fn card_counts(&self, story: &StoryDTO, cx: &mut App) -> Div {
        let (total, likes) = self.service.update(cx, |service, cx| {
            (service.story_viewers_total(story.id, cx), service.own_story_like_count(story, cx))
        });
        let heart = system_red(cx);
        hstack(10.0)
            .px(px(10.0))
            .py(px(5.0))
            .capsule()
            .bg(black().opacity(0.45))
            .child(
                hstack(4.0)
                    .children(symbol("eye.fill", 11.0, white()))
                    .child(
                        MainText::new(story.view_count.max(total).to_string())
                            .px(12.0)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(white()),
                    ),
            )
            .child(
                hstack(4.0)
                    .children(symbol("heart.fill", 11.0, heart))
                    .child(MainText::new(likes.to_string()).px(12.0).font_weight(FontWeight::SEMIBOLD).color(white())),
            )
    }

    fn tap_card(&self, target: i64, window: &mut Window, cx: &mut App) {
        if target < 0 || target >= self.stories_count() || target == self.index.get(cx) {
            return;
        }
        let _ = selection_changed();
        self.animate_to_index(target, spring(0.36, 0.86), window, cx);
    }

    /// animates to the target index and resets the horizontal drag
    fn animate_to_index(&self, target: i64, spec: MotionSpec, window: &mut Window, cx: &mut App) {
        self.index.set(target, window, cx);
        self.state.update(cx, |state, cx| {
            state.displayed_position.set_in(target as f32, spec, RiseClock::monotonic(), cx);
            cx.notify();
        });
    }

    fn sheet_strip(&self, w: f32, geo_height: f32, window: &mut Window, cx: &mut App) -> AnyElement {
        let h = self.stable_height(geo_height);
        let (pos, is_collapsing) = {
            let state = self.state.read(cx);
            (state.displayed_position.value(RiseClock::monotonic()), state.collapse_start.is_some())
        };
        let index = self.index.get(cx);
        let p = self.clamped_progress();
        let open_top = self.sheet_top(w, h, 1.0);
        let panel_height = (h - open_top).max(0.0);
        let panel_top = self.sheet_top(w, h, p);
        let content_alpha = ((p - 0.32) / 0.3).clamp(0.0, 1.0);
        let hit_testable = p > 0.45 || is_collapsing;

        let mut pages = div().relative().w(px(w)).h(px(panel_height)).overflow_hidden();
        for story_idx in Self::visible_sheet_indexes(index) {
            if story_idx >= 0 && story_idx < self.stories_count() {
                let story_id = self.stories[story_idx as usize].id;
                pages = pages.child(
                    div()
                        .absolute()
                        .top_0()
                        .left(px((story_idx as f32 - pos) * w))
                        .w(px(w))
                        .h(px(panel_height))
                        .child(self.sheet_page(story_id, w, panel_height, window, cx)),
                );
            }
        }
        let pages = pages.when(!hit_testable && self.allows_hit_testing, |pages| {
            pages.child(div().absolute().inset_0().block_mouse_except_scroll())
        });

        div()
            .absolute()
            .left_0()
            .top(px(panel_top))
            .w(px(w))
            .h(px(panel_height))
            .opacity(content_alpha)
            .when(!self.allows_hit_testing && p <= 0.02, |strip| strip.invisible())
            .child(pages.simultaneous_gesture(
                self.child_id("sheet-drag"),
                self.sheet_drag(),
                if hit_testable { GestureMask::All } else { GestureMask::None },
            ))
            .into_any_element()
    }

    fn sheet_drag(&self) -> DragGesture {
        let changed = self.clone();
        let ended = self.clone();
        DragGesture::new(12.0)
            .on_changed(move |value: &DragGestureValue, _, cx| changed.sheet_drag_changed(value, cx))
            .on_ended(move |value: &DragGestureValue, window, cx| ended.sheet_drag_ended(value, window, cx))
    }

    fn sheet_drag_changed(&self, value: &DragGestureValue, cx: &mut App) {
        let Some((w, h)) = self.geometry(cx) else {
            return;
        };
        let dx = f32::from(value.translation.width);
        let dy = f32::from(value.translation.height);
        if self.state.read(cx).drag_axis == PagerAxis::Undecided {
            if dx.abs() > dy.abs() * 1.4 && dx.abs() > 12.0 {
                self.state.update(cx, |state, _| state.drag_axis = PagerAxis::Horizontal);
            } else {
                let moves_sheet = self.sheet_drag_moves_sheet(f32::from(value.start_location.y), cx);
                let start = *self.viewers_progress.read(cx).target();
                self.state.update(cx, |state, _| {
                    state.drag_axis = PagerAxis::Vertical;
                    if moves_sheet {
                        state.collapse_start = Some(start);
                    }
                });
            }
        }
        let (axis, collapse_start) = {
            let state = self.state.read(cx);
            (state.drag_axis, state.collapse_start)
        };
        match axis {
            PagerAxis::Horizontal => self.follow_horizontal_drag(dx, w, cx),
            PagerAxis::Vertical => {
                if let Some(start) = collapse_start {
                    self.follow_vertical_drag(start, dy, h, cx);
                }
            }
            PagerAxis::Undecided => {}
        }
    }

    fn sheet_drag_ended(&self, value: &DragGestureValue, window: &mut Window, cx: &mut App) {
        let (axis, start) = self.take_drag(cx);
        if axis == PagerAxis::Horizontal {
            self.settle_horizontal_drag(value, window, cx);
            return;
        }
        if axis == PagerAxis::Vertical
            && let Some(start) = start
        {
            self.settle_vertical_drag(value, start, window, cx);
        }
    }

    fn sheet_drag_moves_sheet(&self, start_y: f32, cx: &mut App) -> bool {
        let index = self.index.get(cx);
        let story_id = self.active_story_id(index);
        let viewers = self.service.update(cx, |service, cx| service.story_viewers(story_id, cx));
        let filters = self.filters.read(cx);
        let rows = StoryViewersSheetPage::filter(viewers, filters.tab, filters.liked_only, &filters.query);
        if rows.is_empty() {
            return true;
        }
        start_y <= self.state.read(cx).sheet_header_height
    }

    fn visible_sheet_indexes(index: i64) -> [i64; 3] {
        [index - 1, index, index + 1]
    }

    /// page takes no gesture progress and is a cached view, it updates only when what it draws changes
    fn sheet_page(&self, story_id: i64, width: f32, height: f32, window: &mut Window, cx: &mut App) -> AnyElement {
        let filters = self.filters.clone();
        let bottom_inset = self.bottom_inset;
        let page = window.use_keyed_state(self.named_id(format!("sheet-page-{story_id}")), cx, move |_, cx: &mut Context<StoryViewersSheetPage>| {
            StoryViewersSheetPage::new(story_id, width, height, bottom_inset, filters, cx)
        });

        let header_state = self.state.clone();
        let on_header_height: StoryViewersHeaderHeightHandler = Rc::new(move |height: f32, _: &mut Window, cx: &mut App| {
            header_state.update(cx, |state, cx| {
                if (state.sheet_header_height - height).abs() > 0.5 {
                    state.sheet_header_height = height;
                    cx.notify();
                }
            });
        });
        let open_parts = self.clone();
        let on_open_viewer: StoryViewersOpenViewerHandler = Rc::new(move |viewer: &StoryViewerDTO, window: &mut Window, cx: &mut App| {
            let _ = selection_changed();
            (open_parts.on_dismiss)(window, cx);
            if let Some(nav_store) = &open_parts.nav_store {
                let route = RootRoute::UserPage { user_id: Some(viewer.id.clone()), tag: Some(viewer.tag.clone()) };
                nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
            }
        });
        page.update(cx, |page, cx| {
            page.on_header_height = Some(on_header_height);
            page.on_open_viewer = Some(on_open_viewer);
            if page.width != width || page.height != height || page.bottom_inset != bottom_inset {
                page.width = width;
                page.height = height;
                page.bottom_inset = bottom_inset;
                cx.notify();
            }
        });

        page.cached(StyleRefinement::default().size_full()).into_any_element()
    }

    fn unified_drag(&self) -> DragGesture {
        let changed = self.clone();
        let ended = self.clone();
        DragGesture::new(6.0)
            .on_changed(move |value: &DragGestureValue, _, cx| changed.unified_drag_changed(value, cx))
            .on_ended(move |value: &DragGestureValue, window, cx| ended.unified_drag_ended(value, window, cx))
    }

    fn unified_drag_changed(&self, value: &DragGestureValue, cx: &mut App) {
        let Some((w, h)) = self.geometry(cx) else {
            return;
        };
        let dx = f32::from(value.translation.width);
        let dy = f32::from(value.translation.height);
        let current = *self.viewers_progress.read(cx).target();
        let (axis, collapse_start) = self.state.update(cx, |state, _| {
            state.band_was_dragged = true;
            if state.drag_axis == PagerAxis::Undecided {
                if dx.abs() > dy.abs() && dx.abs() > 6.0 {
                    state.drag_axis = PagerAxis::Horizontal;
                } else if dy.abs() > dx.abs() && dy.abs() > 6.0 {
                    state.drag_axis = PagerAxis::Vertical;
                    state.collapse_start = Some(current);
                }
            }
            (state.drag_axis, state.collapse_start)
        });
        match axis {
            PagerAxis::Horizontal => self.follow_horizontal_drag(dx, w, cx),
            PagerAxis::Vertical => self.follow_vertical_drag(collapse_start.unwrap_or(current), dy, h, cx),
            PagerAxis::Undecided => {}
        }
    }

    fn unified_drag_ended(&self, value: &DragGestureValue, window: &mut Window, cx: &mut App) {
        let (axis, start) = self.take_drag(cx);
        match axis {
            PagerAxis::Horizontal => self.settle_horizontal_drag(value, window, cx),
            PagerAxis::Vertical => {
                let start = start.unwrap_or(*self.viewers_progress.read(cx).target());
                self.settle_vertical_drag(value, start, window, cx);
            }
            PagerAxis::Undecided => {}
        }
    }

    fn take_drag(&self, cx: &mut App) -> (PagerAxis, Option<f32>) {
        self.state.update(cx, |state, _| {
            let taken = (state.drag_axis, state.collapse_start);
            state.drag_axis = PagerAxis::Undecided;
            state.collapse_start = None;
            taken
        })
    }

    fn follow_horizontal_drag(&self, dx: f32, w: f32, cx: &mut App) {
        let index = self.index.get(cx);
        let mut d = dx;
        if index <= 0 && d > 0.0 {
            d *= 0.35;
        }
        if index >= self.stories_count() - 1 && d < 0.0 {
            d *= 0.35;
        }
        self.state.update(cx, |state, cx| {
            state.displayed_position.jump(index as f32 - d / w.max(1.0));
            cx.notify();
        });
    }

    fn follow_vertical_drag(&self, start: f32, dy: f32, h: f32, cx: &mut App) {
        let span = h * 0.62;
        let value = (start - dy / span).clamp(0.0, 1.0);
        self.viewers_progress.update(cx, |progress, cx| {
            progress.jump(value);
            cx.notify();
        });
    }

    fn settle_horizontal_drag(&self, value: &DragGestureValue, window: &mut Window, cx: &mut App) {
        let Some((w, _)) = self.geometry(cx) else {
            return;
        };
        let dx = f32::from(value.translation.width);
        let predicted = f32::from(value.predicted_end_translation.width);
        let threshold = w * 0.22;
        let index = self.index.get(cx);
        let mut target = index;
        if dx < -threshold || predicted < -w * 0.5 {
            target = (self.stories_count() - 1).min(index + 1);
        } else if dx > threshold || predicted > w * 0.5 {
            target = (index - 1).max(0);
        }
        if target != index {
            let _ = selection_changed();
        }
        self.animate_to_index(target, spring(0.34, 0.86), window, cx);
    }

    fn settle_vertical_drag(&self, value: &DragGestureValue, start: f32, window: &mut Window, cx: &mut App) {
        let dy = f32::from(value.translation.height);
        let predicted = f32::from(value.predicted_end_translation.height);
        let down_hard = dy > 120.0 || predicted > 320.0;
        let up_hard = dy < -60.0 || predicted < -200.0;
        let viewers_progress = *self.viewers_progress.read(cx).target();
        if down_hard && start <= 0.12 && viewers_progress < 0.2 {
            (self.on_request_dismiss)(window, cx);
        } else if down_hard {
            self.settle_viewers(0.0, cx);
        } else if up_hard {
            self.settle_viewers(1.0, cx);
        } else {
            let detent = if viewers_progress > 0.78 {
                1.0
            } else if viewers_progress >= 0.4 {
                0.5
            } else {
                0.0
            };
            self.settle_viewers(detent, cx);
        }
    }

    /// sheet stays mounted at progress 0, unmounting after close would rebuild it on the next swipe
    fn settle_viewers(&self, target: f32, cx: &mut App) {
        self.viewers_progress.update(cx, |progress, cx| {
            progress.set_in(target, spring(0.36, 0.86), RiseClock::monotonic(), cx);
            cx.notify();
        });
    }

    fn handle_scroll_wheel(&self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut App) {
        if !self.allows_hit_testing {
            return;
        }
        let event = Self::horizontal_from_shift(event);
        let line_height = window.line_height();
        let feed = self.state.update(cx, |state, _| state.wheel.feed(&event, line_height));
        if self.apply_wheel_step(feed.step, event.position, window, cx) {
            cx.stop_propagation();
        }
        if let Some(token) = feed.settle {
            let parts = self.clone();
            let position = event.position;
            let task = window.spawn(cx, async move |cx| {
                cx.background_executor().timer(WHEEL_SETTLE_DELAY).await;
                let _ = cx.update(|window, cx| {
                    let step = parts.state.update(cx, |state, _| state.wheel.settle(token));
                    if let Some(step) = step {
                        parts.apply_wheel_step(step, position, window, cx);
                    }
                });
            });
            self.state.update(cx, |state, _| state.wheel_settle = Some(task));
        }
    }

    fn horizontal_from_shift(event: &ScrollWheelEvent) -> ScrollWheelEvent {
        let mut event = event.clone();
        if event.modifiers.shift {
            event.delta = match event.delta {
                ScrollDelta::Pixels(delta) if f32::from(delta.x) == 0.0 => {
                    ScrollDelta::Pixels(point(delta.y, delta.x))
                }
                ScrollDelta::Lines(delta) if delta.x == 0.0 => ScrollDelta::Lines(point(delta.y, delta.x)),
                other => other,
            };
        }
        event
    }

    /// true when the event belongs to the pager and not the list under it
    fn apply_wheel_step(&self, step: WheelStep, position: Point<Pixels>, window: &mut Window, cx: &mut App) -> bool {
        match step {
            WheelStep::Ignored => false,
            WheelStep::Moved(translation) => self.wheel_moved(translation, position, cx),
            WheelStep::Ended(translation) => {
                let Some(session) = self.take_recognized_wheel_session(cx) else {
                    return false;
                };
                let value = Self::wheel_value(session.start_location, translation);
                let claimed = self.wheel_is_claimed(cx);
                match session.surface {
                    WheelSurface::Band => self.unified_drag_ended(&value, window, cx),
                    WheelSurface::Sheet => self.sheet_drag_ended(&value, window, cx),
                }
                claimed
            }
            WheelStep::Cancelled => {
                if self.take_recognized_wheel_session(cx).is_none() {
                    return false;
                }
                let claimed = self.wheel_is_claimed(cx);
                let (axis, start) = self.take_drag(cx);
                match (axis, start) {
                    (PagerAxis::Horizontal, _) => {
                        let index = self.index.get(cx);
                        self.animate_to_index(index, spring(0.34, 0.86), window, cx);
                    }
                    (PagerAxis::Vertical, Some(start)) => self.settle_viewers(start, cx),
                    _ => {}
                }
                claimed
            }
        }
    }

    fn wheel_moved(&self, translation: Point<Pixels>, position: Point<Pixels>, cx: &mut App) -> bool {
        if translation == Point::default() {
            return false;
        }
        let session = match self.state.read(cx).wheel_session {
            Some(session) => session,
            None => {
                let Some(session) = self.begin_wheel_session(position, cx) else {
                    return false;
                };
                session
            }
        };
        let dx = f32::from(translation.x);
        let dy = f32::from(translation.y);
        let minimum_distance = match session.surface {
            WheelSurface::Band => 6.0,
            WheelSurface::Sheet => 12.0,
        };
        let is_recognized = session.is_recognized || (dx * dx + dy * dy).sqrt() >= minimum_distance;
        self.state.update(cx, |state, _| {
            state.wheel_session = Some(WheelSession { is_recognized, ..session });
        });
        if !is_recognized {
            return false;
        }
        let value = Self::wheel_value(session.start_location, translation);
        match session.surface {
            WheelSurface::Band => self.unified_drag_changed(&value, cx),
            WheelSurface::Sheet => self.sheet_drag_changed(&value, cx),
        }
        self.wheel_is_claimed(cx)
    }

    fn begin_wheel_session(&self, position: Point<Pixels>, cx: &App) -> Option<WheelSession> {
        let geo = self.state.read(cx).geo?;
        let (w, h) = self.geometry(cx)?;
        let p = self.clamped_progress();
        let local = point(position.x - geo.origin.x, position.y - geo.origin.y);
        let sheet_top = self.sheet_top(w, h, p);
        if f32::from(local.y) < sheet_top {
            return (p > 0.05).then_some(WheelSession {
                surface: WheelSurface::Band,
                start_location: local,
                is_recognized: false,
            });
        }
        (p > 0.45).then_some(WheelSession {
            surface: WheelSurface::Sheet,
            start_location: point(local.x, local.y - px(sheet_top)),
            is_recognized: false,
        })
    }

    fn take_recognized_wheel_session(&self, cx: &mut App) -> Option<WheelSession> {
        self.state.update(cx, |state, _| state.wheel_session.take()).filter(|session| session.is_recognized)
    }

    /// vertical wheel over the rows belongs to the list, any other recognized gesture moves the pager
    fn wheel_is_claimed(&self, cx: &App) -> bool {
        let state = self.state.read(cx);
        state.drag_axis == PagerAxis::Horizontal || state.collapse_start.is_some()
    }

    fn wheel_value(start_location: Point<Pixels>, translation: Point<Pixels>) -> DragGestureValue {
        DragGestureValue {
            start_location,
            location: point(start_location.x + translation.x, start_location.y + translation.y),
            translation: size(translation.x, translation.y),
            predicted_end_translation: size(translation.x, translation.y),
            velocity: size(px(0.0), px(0.0)),
        }
    }
}

/// one sheet page with tabs, search and the viewers list of a single story
/// separate view taking no gesture progress so it stays cached
struct StoryViewersSheetPage {
    story_id: i64,
    width: f32,
    height: f32,
    bottom_inset: f32,
    tab: Binding<StoryViewersTab>,
    liked_only: Binding<bool>,
    query: Binding<String>,
    on_header_height: Option<StoryViewersHeaderHeightHandler>,
    on_open_viewer: Option<StoryViewersOpenViewerHandler>,
    service: Entity<StoryServicesStore>,
    list: ListUi,
    list_ids: Vec<String>,
    _subscriptions: Vec<Subscription>,
}

impl StoryViewersSheetPage {
    fn new(
        story_id: i64,
        width: f32,
        height: f32,
        bottom_inset: f32,
        filters: Entity<StoryViewersPagerFilters>,
        cx: &mut Context<Self>,
    ) -> Self {
        let service = StoryServicesStore::shared(cx);
        let cache_revisions = SaiCacheRevisionTracker::shared(cx);
        let pending_requests = SaiWsPendingRegistry::shared(cx);
        let theme_store = ThemeStore::shared(cx);
        Self {
            story_id,
            width,
            height,
            bottom_inset,
            tab: Binding::entity(
                filters.clone(),
                |filters: &StoryViewersPagerFilters| filters.tab,
                |filters, value, _| filters.tab = value,
            ),
            liked_only: Binding::entity(
                filters.clone(),
                |filters: &StoryViewersPagerFilters| filters.liked_only,
                |filters, value, _| filters.liked_only = value,
            ),
            query: Binding::entity(
                filters.clone(),
                |filters: &StoryViewersPagerFilters| filters.query.clone(),
                |filters, value, _| filters.query = value,
            ),
            on_header_height: None,
            on_open_viewer: None,
            service: service.clone(),
            list: ListUi::new(rise_ui::theme(cx), 0, ListAlignment::Top),
            list_ids: Vec::new(),
            _subscriptions: vec![
                cx.observe(&filters, |_, _, cx| cx.notify()),
                cx.observe(&service, |_, _, cx| cx.notify()),
                cx.observe(&cache_revisions, |_, _, cx| cx.notify()),
                cx.observe(&pending_requests, |_, _, cx| cx.notify()),
                cx.observe(&theme_store, |_, _, cx| cx.notify()),
            ],
        }
    }

    fn element_id(&self, name: impl AsRef<str>) -> ElementId {
        ElementId::Name(format!("story-viewers-sheet-{}-{}", self.story_id, name.as_ref()).into())
    }

    fn filter(viewers: Vec<StoryViewerDTO>, tab: StoryViewersTab, liked_only: bool, query: &str) -> Vec<StoryViewerDTO> {
        let q = trimming_whitespaces_and_newlines(query).to_lowercase();
        viewers
            .into_iter()
            .filter(|viewer| {
                if tab == StoryViewersTab::Contacts && !viewer.is_contact {
                    return false;
                }
                if liked_only && !viewer.liked {
                    return false;
                }
                if q.is_empty() {
                    return true;
                }
                viewer.name.to_lowercase().contains(&q) || viewer.tag.to_lowercase().contains(&q)
            })
            .collect()
    }

    fn filtered_viewers(&self, all_viewers: &[StoryViewerDTO], cx: &App) -> Vec<StoryViewerDTO> {
        Self::filter(all_viewers.to_vec(), self.tab.get(cx), self.liked_only.get(cx), &self.query.get(cx))
    }

    fn grabber(&self, theme: &AppTheme) -> Div {
        div()
            .w_full()
            .flex()
            .justify_center()
            .pt(px(8.0))
            .pb(px(8.0))
            .child(div().w(px(38.0)).h(px(5.0)).capsule().bg(theme.border._300.opacity(0.6)))
    }

    fn tabs_row(&self, theme: &AppTheme, cx: &mut Context<Self>) -> Div {
        let like_count = self.service.update(cx, |service, cx| service.story_viewers_like_count(self.story_id, cx));
        let liked_only = self.liked_only.get(cx);
        let heart = system_red(cx);
        let tint = if liked_only { heart } else { theme.text.secondary };
        let liked_binding = self.liked_only.clone();
        hstack(8.0)
            .w_full()
            .px(px(14.0))
            .pb(px(8.0))
            .child(self.tab_pill("tab-all", StoryViewersTab::All, tr("story_viewers_tab_all"), theme, cx))
            .child(self.tab_pill("tab-contacts", StoryViewersTab::Contacts, tr("story_viewers_tab_contacts"), theme, cx))
            .child(spacer())
            .child(
                plain(self.element_id("liked-only"))
                    .on_press(move |window, cx| {
                        let _ = selection_changed();
                        liked_binding.toggle(window, cx);
                    })
                    .px(px(12.0))
                    .py(px(7.0))
                    .capsule()
                    .bg(if liked_only { heart.opacity(0.12) } else { theme.bg._200 })
                    .child(
                        hstack(5.0)
                            .children(symbol(if liked_only { "heart.fill" } else { "heart" }, 13.0, tint))
                            .child(
                                MainText::new(like_count.to_string())
                                    .px(13.0)
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .color(tint),
                            ),
                    ),
            )
    }

    fn tab_pill(
        &self,
        name: &'static str,
        tab: StoryViewersTab,
        title: String,
        theme: &AppTheme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let active = self.tab.get(cx) == tab;
        let binding = self.tab.clone();
        plain(self.element_id(name))
            .on_press(move |window, cx| {
                let _ = selection_changed();
                binding.set(tab, window, cx);
            })
            .px(px(14.0))
            .py(px(7.0))
            .capsule()
            .bg(if active { theme.primary._100 } else { theme.bg._200 })
            .child(
                MainText::new(title)
                    .px(14.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .color(if active { theme.content_on(theme.primary._100) } else { theme.text.primary }),
            )
    }

    fn search_row(&self) -> Div {
        div().w_full().px(px(14.0)).pb(px(10.0)).child(
            SearchFieldUi::new(self.element_id("search"), self.query.clone())
                .placeholder(SharedString::from(tr("search_placeholder")))
                .uses_inline_text_field(true),
        )
    }

    fn header(&self, theme: &AppTheme, cx: &mut Context<Self>) -> Div {
        let on_header_height = self.on_header_height.clone();
        vstack(0.0)
            .relative()
            .w_full()
            .child(self.grabber(theme))
            .child(self.tabs_row(theme, cx))
            .child(self.search_row())
            .child(measure(self.element_id("header-height"), move |bounds, window, cx| {
                if let Some(on_header_height) = on_header_height {
                    on_header_height(f32::from(bounds.size.height), window, cx);
                }
            }))
    }

    fn load_next_page(service: &Entity<StoryServicesStore>, story_id: i64, cx: &mut App) {
        let Some(cursor) = service.update(cx, |service, cx| service.story_viewers_next_cursor(story_id, cx)) else {
            return;
        };
        StoryInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.load_story_viewers(story_id, Some(cursor), cx);
        });
    }

    fn content(&mut self, theme: &AppTheme, cx: &mut Context<Self>) -> Div {
        let story_id = self.story_id;
        let (all_viewers, relative_id, has_more, is_in_flight, is_loaded) =
            self.service.update(cx, |service, cx| {
                (
                    service.story_viewers(story_id, cx),
                    service.story_viewers_next_cursor(story_id, cx).map(|cursor| cursor.to_string()),
                    service.story_viewers_has_more(story_id, cx),
                    service.is_viewers_in_flight(story_id, cx),
                    service.story_viewers_loaded(story_id, cx),
                )
            });
        let is_errored = self.service.read(cx).viewers_errored().contains(&story_id);
        let viewers = self.filtered_viewers(&all_viewers, cx);
        let filtered_count = viewers.len();
        let cache_key = SharedString::from(StoryServicesStore::viewers_cache_key(story_id));

        let body = if !viewers.is_empty() {
            self.list_body(viewers, cache_key.clone(), relative_id.clone(), has_more, theme)
        } else if is_in_flight && all_viewers.is_empty() {
            self.loading_state()
        } else if is_errored && !is_loaded {
            self.error_state(theme).into_any_element()
        } else {
            self.empty_state(!all_viewers.is_empty(), theme).into_any_element()
        };

        let drain_service = self.service.clone();
        div()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(div().absolute().inset_0().child(body))
            .child(
                div().absolute().size_0().sai_pagination_drain_filtered_empty(
                    self.element_id("drain"),
                    SaiPaginationPrefetchOptions::drain_filtered_empty(filtered_count, cache_key, relative_id, has_more),
                    move |_, cx| Self::load_next_page(&drain_service, story_id, cx),
                ),
            )
    }

    fn sync_list(&mut self, ids: Vec<String>) {
        if ids == self.list_ids {
            return;
        }
        if !self.list_ids.is_empty() && ids.len() > self.list_ids.len() && ids.starts_with(&self.list_ids) {
            self.list.appended(ids.len() - self.list_ids.len());
        } else {
            self.list.replaced(ids.len());
        }
        self.list_ids = ids;
    }

    fn list_body(
        &mut self,
        viewers: Vec<StoryViewerDTO>,
        cache_key: SharedString,
        relative_id: Option<String>,
        has_more: bool,
        theme: &AppTheme,
    ) -> AnyElement {
        self.sync_list(viewers.iter().map(|viewer| viewer.id.clone()).collect());

        let item_count = viewers.len();
        let story_id = self.story_id;
        let tail_padding = self.bottom_inset + 24.0;
        let service = self.service.clone();
        let on_open_viewer = self.on_open_viewer.clone();
        let theme = theme.clone();
        self.list
            .element(move |index, _, cx| {
                let Some(viewer) = viewers.get(index) else {
                    return Empty.into_any_element();
                };
                let load_service = service.clone();
                let row = Self::row(story_id, viewer, on_open_viewer.clone(), &theme, cx);
                div()
                    .w_full()
                    .when(index + 1 == item_count, |tail| tail.pb(px(tail_padding)))
                    .child(row)
                    .sai_pagination_prefetch(
                        ElementId::Name(format!("story-viewers-sheet-{story_id}-row-prefetch-{}", viewer.id).into()),
                        SaiPaginationPrefetchOptions::new(
                            index,
                            item_count,
                            cache_key.clone(),
                            relative_id.clone(),
                            has_more,
                        )
                        .threshold(8),
                        move |_, cx| Self::load_next_page(&load_service, story_id, cx),
                    )
                    .into_any_element()
            })
            .into_any_element()
    }

    fn row(
        story_id: i64,
        viewer: &StoryViewerDTO,
        on_open_viewer: Option<StoryViewersOpenViewerHandler>,
        theme: &AppTheme,
        cx: &mut App,
    ) -> AnyElement {
        let heart = system_red(cx);
        let press_viewer = viewer.clone();
        let name_color = if viewer.is_premium { PremiumNameColor::parse(viewer.name_color.as_deref()) } else { None };
        let row_id = |name: &str| ElementId::Name(format!("story-viewers-sheet-{story_id}-{name}-{}", viewer.id).into());
        plain(row_id("row"))
            .on_press(move |window, cx| {
                if let Some(on_open_viewer) = &on_open_viewer {
                    on_open_viewer(&press_viewer, window, cx);
                }
            })
            .w_full()
            .px(px(16.0))
            .py(px(9.0))
            .child(
                hstack(12.0)
                    .w_full()
                    .child(
                        UserLogoUi::new(row_id("logo"))
                            .source(Some(SharedString::from(viewer.logo.clone())))
                            .name(Some(SharedString::from(viewer.name.clone())))
                            .user_id(Some(SharedString::from(viewer.id.clone())))
                            .size(42.0)
                            .show_story_ring(false),
                    )
                    .child(
                        vstack(2.0)
                            .items_start()
                            .child(
                                hstack(4.0)
                                    .child(
                                        MainText::new(viewer.name.clone())
                                            .px(15.0)
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .color(name_color)
                                            .number_of_lines(1),
                                    )
                                    .when(viewer.is_premium, |line| {
                                        line.child(
                                            PremiumBadge::new()
                                                .user_id(Some(SharedString::from(viewer.id.clone())))
                                                .emoji(viewer.premium_badge_emoji.clone().map(SharedString::from))
                                                .size(13.0),
                                        )
                                    }),
                            )
                            .child(
                                MainText::new(Self::relative_time(viewer.liked_at_ms.unwrap_or(viewer.viewed_at_ms)))
                                    .px(12.0)
                                    .color(theme.text.secondary)
                                    .number_of_lines(1),
                            ),
                    )
                    .child(spacer())
                    .when(viewer.liked, |line| line.children(symbol("heart.fill", 16.0, heart))),
            )
            .into_any_element()
    }

    fn loading_state(&self) -> AnyElement {
        scroll_view(self.element_id("skeleton"))
            .scroll_disabled(true)
            .shows_indicators(false)
            .child(UserListSkeleton::new().rows(8).shows_separators(false))
            .into_any_element()
    }

    fn empty_state(&self, filtered: bool, theme: &AppTheme) -> Div {
        vstack(8.0)
            .size_full()
            .pb(px(60.0))
            .child(spacer())
            .children(symbol(
                if filtered { "magnifyingglass" } else { "eye.slash" },
                28.0,
                theme.text.secondary.opacity(0.6),
            ))
            .child(
                MainText::new(tr(if filtered { "picker_no_results" } else { "story_viewers_empty" }))
                    .px(14.0)
                    .color(theme.text.secondary),
            )
            .child(spacer())
    }

    fn error_state(&self, theme: &AppTheme) -> Div {
        let story_id = self.story_id;
        vstack(8.0)
            .size_full()
            .pb(px(60.0))
            .child(spacer())
            .children(symbol("exclamationmark.triangle", 28.0, theme.semantic.error_100))
            .child(MainText::new(tr("error_fetching_data")).px(14.0).color(theme.text.secondary))
            .child(
                div().pt(px(6.0)).child(
                    plain(self.element_id("retry"))
                        .on_press(move |_, cx| {
                            StoryInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                                interactions.load_story_viewers(story_id, None, cx);
                            });
                        })
                        .px(px(18.0))
                        .py(px(9.0))
                        .bg(theme.primary._100)
                        .clip_shape(Shape::Capsule)
                        .child(
                            MainText::new(tr("common_retry"))
                                .px(14.0)
                                .font_weight(FontWeight::SEMIBOLD)
                                .color(theme.content_on(theme.primary._100)),
                        ),
                ),
            )
            .child(spacer())
    }

    fn relative_time(ms: i64) -> String {
        if ms <= 0 {
            return String::new();
        }
        RiseFormat::elapsed(ms, (Date::now().time_interval_since_1970() * 1000.0) as i64)
    }
}

impl Render for StoryViewersSheetPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        vstack(0.0)
            .w(px(self.width))
            .h(px(self.height))
            .items_start()
            .rounded_t(px(18.0))
            .bg(theme.bg._100)
            .child(self.header(&theme, cx))
            .child(self.content(&theme, cx))
    }
}
