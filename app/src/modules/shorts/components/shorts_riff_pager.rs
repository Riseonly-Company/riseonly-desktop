use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, Entity, FocusHandle, FontWeight, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, MouseDownEvent, ParentElement, Pixels, Point, RenderOnce, ScrollDelta, ScrollWheelEvent,
    SharedString, Size, StatefulInteractiveElement, Styled, Task, Window, black, div, point, px, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_media::video::av_player::AVLayerVideoGravity;
use rise_platform::wheel_gesture::{WHEEL_SETTLE_DELAY, WheelGestureTracker, WheelStep};
use rise_theme::alpha;
use rise_ui::IconUi;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::drag_gesture::{DragGesture, DragGestureModifiers, DragGestureValue};
use rise_ui::swiftui::environment::LayoutDirection;
use rise_ui::swiftui::motion::ease_out;
use rise_ui::swiftui::shapes::SwiftUiShapes;

use crate::core::ui::clever_image_ui::{CleverImageCachePolicy, CleverImageUi, ContentMode};
use crate::modules::post::components::post::post_media_cache_policy::PostMediaCachePolicy;
use crate::modules::post::shared::post_media_slides::PostMediaSlide;
use crate::modules::post::stores::post_actions::post_types::{GetPostFeedResponse, PostMediaSizeAspectRatio};
use crate::modules::shorts::components::shorts_feed_player::{ShortsFeedPlayer, ShortsPlaybackController};

/// dots up to the dot limit and a counter after, 30 dots are wider than the screen
#[derive(IntoElement)]
pub struct RiffPageIndicator {
    count: usize,
    current: i64,
}

impl RiffPageIndicator {
    pub const DOT_LIMIT: usize = 10;

    pub fn new(count: usize, current: i64) -> Self {
        Self { count, current }
    }
}

impl RenderOnce for RiffPageIndicator {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let uses_dots = self.count <= Self::DOT_LIMIT;
        let body = if uses_dots {
            div()
                .flex()
                .flex_row()
                .gap(px(5.0))
                .px(px(8.0))
                .py(px(5.0))
                .children((0..self.count).map(|index| {
                    div()
                        .size(px(6.0))
                        .circle()
                        .bg(alpha(white(), if index as i64 == self.current { 0.95 } else { 0.45 }))
                }))
        } else {
            let shown = self.current.max(0).min(self.count as i64 - 1) + 1;
            div().px(px(10.0)).py(px(4.0)).child(
                MainText::new(format!("{shown}/{}", self.count))
                    .px(12.0)
                    .font_weight(FontWeight::SEMIBOLD)
                    .monospaced_digits(true)
                    .color(white()),
            )
        };
        body.capsule().bg(alpha(black(), if uses_dots { 0.28 } else { 0.45 }))
    }
}

/// what the pager keeps between frames
#[derive(Default)]
struct ShortsRiffPagerMemory {
    video_aspect: Option<f32>,
    measured_image_aspects: HashMap<i64, f32>,
}

type PagerAction = Rc<dyn Fn(&mut Window, &mut App)>;
/// callback with the drawn aspect of a slide by media url
type DrawnAspectAction = Rc<dyn Fn(&str, f32, &mut Window, &mut App)>;
type PagerPageContent = Rc<dyn Fn(usize, &mut Window, &mut App) -> AnyElement>;

/// a mounted video holds its first frame while inactive, only the selected page may play
#[derive(IntoElement)]
pub struct ShortsRiffPager {
    id: ElementId,
    post: Rc<GetPostFeedResponse>,
    slides: Vec<PostMediaSlide>,
    size: Size<Pixels>,
    page: Binding<Option<i64>>,
    is_active: bool,
    is_muted: bool,
    paused: bool,
    playback: Entity<ShortsPlaybackController>,
    corner_radius: f32,
    paging_enabled: bool,
    on_video_tap: PagerAction,
    on_drawn_aspect: DrawnAspectAction,
    holds_video_at_end: bool,
}

impl ShortsRiffPager {
    const TAP_SLOP: f32 = 8.0;

    #[allow(clippy::too_many_arguments)]
    pub fn new(
        id: impl Into<ElementId>,
        post: Rc<GetPostFeedResponse>,
        slides: Vec<PostMediaSlide>,
        size: Size<Pixels>,
        page: Binding<Option<i64>>,
        is_active: bool,
        is_muted: bool,
        paused: bool,
        playback: Entity<ShortsPlaybackController>,
        corner_radius: f32,
        paging_enabled: bool,
        on_video_tap: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            post,
            slides,
            size,
            page,
            is_active,
            is_muted,
            paused,
            playback,
            corner_radius,
            paging_enabled,
            on_video_tap: Rc::new(on_video_tap),
            on_drawn_aspect: Rc::new(|_, _, _, _| {}),
            holds_video_at_end: false,
        }
    }

    pub fn holds_video_at_end(mut self, holds_video_at_end: bool) -> Self {
        self.holds_video_at_end = holds_video_at_end;
        self
    }

    /// aspect of a slide once drawn, so the cell puts link zones on the rect the slide really takes
    pub fn on_drawn_aspect(mut self, on_drawn_aspect: impl Fn(&str, f32, &mut Window, &mut App) + 'static) -> Self {
        self.on_drawn_aspect = Rc::new(on_drawn_aspect);
        self
    }

    pub fn resolved_index(page: Option<i64>, count: usize) -> i64 {
        if count == 0 {
            return 0;
        }
        page.unwrap_or(0).max(0).min(count as i64 - 1)
    }

    fn slide_id(id: &ElementId, suffix: String) -> ElementId {
        ElementId::NamedChild(Arc::new(id.clone()), SharedString::from(suffix))
    }
}

struct ShortsRiffPagerPages {
    id: ElementId,
    post: Rc<GetPostFeedResponse>,
    slides: Vec<PostMediaSlide>,
    size: Size<Pixels>,
    is_active: bool,
    is_muted: bool,
    paused: bool,
    playback: Entity<ShortsPlaybackController>,
    corner_radius: f32,
    is_own: bool,
    policy: CleverImageCachePolicy,
    on_video_tap: PagerAction,
    on_drawn_aspect: DrawnAspectAction,
    holds_video_at_end: bool,
    memory: Entity<ShortsRiffPagerMemory>,
}

impl ShortsRiffPagerPages {
    fn slide_page(&self, slide: &PostMediaSlide, is_current: bool, cx: &App) -> AnyElement {
        match slide {
            PostMediaSlide::Image { url, size, image_index } => {
                self.image_page(url, size.as_ref().and_then(|size| size.aspect_ratio()), *image_index, cx)
            }
            PostMediaSlide::Video { url, thumbnail, .. } => self.video_page(url, thumbnail.clone(), is_current, cx),
        }
    }

    fn image_page(&self, url: &str, server_aspect: Option<f64>, image_index: i64, cx: &App) -> AnyElement {
        let aspect = server_aspect
            .map(|aspect| aspect as f32)
            .or_else(|| self.memory.read(cx).measured_image_aspects.get(&image_index).copied());
        let gravity = aspect.map(|aspect| ShortsFeedPlayer::gravity(aspect, self.size));
        let memory = self.memory.clone();
        let on_drawn_aspect = self.on_drawn_aspect.clone();
        let drawn_url = url.to_owned();
        div()
            .relative()
            .w(self.size.width)
            .h(self.size.height)
            .rounded_rect(self.corner_radius)
            .overflow_hidden()
            .bg(black())
            .child(
                div().absolute().inset_0().opacity(if gravity.is_none() { 0.0 } else { 1.0 }).child(
                    CleverImageUi::new(url.to_owned())
                        .id(ShortsRiffPager::slide_id(&self.id, format!("image-{image_index}")))
                        .content_mode(if gravity == Some(AVLayerVideoGravity::ResizeAspectFill) {
                            ContentMode::Fill
                        } else {
                            ContentMode::Fit
                        })
                        .corner_radius(self.corner_radius)
                        .persist_to_disk(self.is_own)
                        .cache_policy(self.policy)
                        .on_natural_size(move |natural, window, cx| {
                            if natural.width <= 0.0 || natural.height <= 0.0 {
                                return;
                            }
                            on_drawn_aspect(&drawn_url, natural.width / natural.height, window, cx);
                            memory.update(cx, |memory, cx| {
                                if memory.measured_image_aspects.contains_key(&image_index) {
                                    return;
                                }
                                memory.measured_image_aspects.insert(image_index, natural.width / natural.height);
                                cx.notify();
                            });
                        }),
                ),
            )
            .into_any_element()
    }

    fn video_page(&self, url: &str, thumbnail: Option<String>, is_current: bool, cx: &App) -> AnyElement {
        let aspect_memory = self.memory.clone();
        let on_drawn_aspect = self.on_drawn_aspect.clone();
        let drawn_url = url.to_owned();
        let on_video_tap = self.on_video_tap.clone();
        let post = &self.post;
        div()
            .relative()
            .w(self.size.width)
            .h(self.size.height)
            .child(
                ShortsFeedPlayer::new(
                    ShortsRiffPager::slide_id(&self.id, "video".to_owned()),
                    url.to_owned(),
                    thumbnail,
                    self.is_active && is_current,
                )
                .shows_thumbnail_while_loading(false)
                .post_id(Some(post.rise_view_target_post_id()))
                .author_id(Some(post.author.id.clone().unwrap_or_else(|| post.author_id.clone())))
                .is_muted(self.is_muted)
                .paused(self.paused)
                .playback(Some(self.playback.clone()))
                .initial_aspect(self.memory.read(cx).video_aspect)
                .holds_at_end(self.holds_video_at_end && is_current)
                .page_size(self.size)
                .corner_radius(self.corner_radius)
                .on_aspect(move |aspect, window, cx| {
                    aspect_memory.update(cx, |memory, cx| {
                        memory.video_aspect = Some(aspect);
                        cx.notify();
                    });
                    on_drawn_aspect(&drawn_url, aspect, window, cx);
                })
                .music_overlay(post.shorts_music_overlay())
                .persists_post_media_to_disk(self.is_own)
                .post_image_cache_policy(self.policy)
                .releases_player_on_disappear(true),
            )
            .child(
                div()
                    .id(ShortsRiffPager::slide_id(&self.id, "video-tap".to_owned()))
                    .absolute()
                    .inset_0()
                    .on_click(move |event: &ClickEvent, window, cx| {
                        if let ClickEvent::Mouse(click) = event {
                            let dx = f32::from(click.up.position.x - click.down.position.x);
                            let dy = f32::from(click.up.position.y - click.down.position.y);
                            if dx.abs() > ShortsRiffPager::TAP_SLOP || dy.abs() > ShortsRiffPager::TAP_SLOP {
                                return;
                            }
                        }
                        on_video_tap(window, cx);
                    }),
            )
            .into_any_element()
    }
}

impl RenderOnce for ShortsRiffPager {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let memory = window.use_keyed_state(Self::slide_id(&self.id, "memory".to_owned()), cx, |_, _| {
            ShortsRiffPagerMemory::default()
        });
        let is_own = PostMediaCachePolicy::is_own_post(&self.post, cx);
        let policy = PostMediaCachePolicy::image_policy(is_own);
        let count = self.slides.len();
        let current = Self::resolved_index(self.page.get(cx), count) as usize;
        let scroller_id = Self::slide_id(&self.id, "scroller".to_owned());
        let size = self.size;
        let page = self.page;
        let paging_enabled = self.paging_enabled;
        let pages = Rc::new(ShortsRiffPagerPages {
            id: self.id,
            post: self.post,
            slides: self.slides,
            size,
            is_active: self.is_active,
            is_muted: self.is_muted,
            paused: self.paused,
            playback: self.playback,
            corner_radius: self.corner_radius,
            is_own,
            policy,
            on_video_tap: self.on_video_tap,
            on_drawn_aspect: self.on_drawn_aspect,
            holds_video_at_end: self.holds_video_at_end,
            memory,
        });

        if count <= 1 {
            return match pages.slides.first() {
                Some(only) => pages.slide_page(only, true, cx),
                None => div().w(size.width).h(size.height).bg(black()).into_any_element(),
            };
        }
        ShortsRiffPagingScroller::new(
            scroller_id,
            count,
            page,
            paging_enabled,
            size,
            Rc::new(move |index: usize, _: &mut Window, cx: &mut App| match pages.slides.get(index) {
                Some(slide) => pages.slide_page(slide, index == current, cx),
                None => div().size_full().bg(black()).into_any_element(),
            }),
        )
        .into_any_element()
    }
}

#[derive(IntoElement)]
pub struct ShortsRiffPagingScroller {
    id: ElementId,
    count: usize,
    page: Binding<Option<i64>>,
    paging_enabled: bool,
    size: Size<Pixels>,
    page_content: PagerPageContent,
}

impl ShortsRiffPagingScroller {
    /// desktop face of the swipe, a mouse cant drag sideways so hover arrow buttons and arrow keys page
    fn arrow(
        id: &ElementId,
        name: &'static str,
        glyph: &str,
        step: i64,
        controller: &Entity<ShortsRiffPagingController>,
    ) -> impl IntoElement + use<> {
        let controller = controller.clone();
        div().on_mouse_down(MouseButton::Left, |_: &MouseDownEvent, _, cx| cx.stop_propagation()).child(
            plain(ElementId::NamedChild(Arc::new(id.clone()), SharedString::new_static(name)))
                .on_press(move |window, cx| {
                    controller.update(cx, |controller, cx| controller.step_slot(step, window, cx));
                })
                .child(
                    div()
                        .size(px(28.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .children(IconUi::sized(glyph, px(14.0), white(), false))
                        .liquid_chrome_circle(None),
                ),
        )
    }

    pub fn new(
        id: impl Into<ElementId>,
        count: usize,
        page: Binding<Option<i64>>,
        paging_enabled: bool,
        size: Size<Pixels>,
        page_content: PagerPageContent,
    ) -> Self {
        Self { id: id.into(), count, page, paging_enabled, size, page_content }
    }
}

impl RenderOnce for ShortsRiffPagingScroller {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let controller = window.use_keyed_state(self.id.clone(), cx, |_, _| ShortsRiffPagingController::new());
        let resolved = ShortsRiffPager::resolved_index(self.page.get(cx), self.count) as usize;
        let right_to_left = LayoutDirection::current() == LayoutDirection::RightToLeft;
        let width = f32::from(self.size.width).max(1.0);
        let page = self.page;
        controller.update(cx, |controller, cx| {
            controller.update(self.count, resolved, self.paging_enabled, right_to_left, width, page, cx);
        });

        let (offset, low, high) = {
            let controller = controller.read(cx);
            let offset = controller.offset.frame_value(RiseClock::monotonic(), window);
            let (low, high) = controller.mount_window(offset);
            (offset, low, high)
        };
        let mut pager = div().relative().w(self.size.width).h(self.size.height).overflow_hidden();
        for index in low..=high {
            let left = (controller.read(cx).slot(index) as f32 - offset) * width;
            pager = pager.child(
                div()
                    .absolute()
                    .top_0()
                    .left(px(left))
                    .w(self.size.width)
                    .h(self.size.height)
                    .child((self.page_content)(index, window, cx)),
            );
        }

        let focus_id = ElementId::NamedChild(Arc::new(self.id.clone()), SharedString::new_static("focus"));
        let focus = window.use_keyed_state::<FocusHandle>(focus_id, cx, |_, cx| cx.focus_handle());
        let focus = focus.read(cx).clone();
        let (slot, is_hovered) = {
            let controller = controller.read(cx);
            (controller.slot(controller.shown_page), controller.is_hovered)
        };
        let shows_arrows = is_hovered && self.paging_enabled;
        let last_slot = self.count.saturating_sub(1);
        let previous_arrow = (shows_arrows && slot > 0).then(|| {
            div()
                .absolute()
                .left(px(10.0))
                .top_0()
                .bottom_0()
                .flex()
                .items_center()
                .child(Self::arrow(&self.id, "previous", "chevron.left", -1, &controller))
        });
        let next_arrow = (shows_arrows && slot < last_slot).then(|| {
            div()
                .absolute()
                .right(px(10.0))
                .top_0()
                .bottom_0()
                .flex()
                .items_center()
                .child(Self::arrow(&self.id, "next", "chevron.right", 1, &controller))
        });

        let hover_controller = controller.clone();
        let key_controller = controller.clone();
        let focus_on_press = focus.clone();
        let wheel_controller = controller.clone();
        let changed_controller = controller.clone();
        let ended_controller = controller;
        pager
            .children(previous_arrow)
            .children(next_arrow)
            .id(self.id.clone())
            .track_focus(&focus)
            .on_mouse_down(MouseButton::Left, move |_: &MouseDownEvent, window, cx| {
                window.focus(&focus_on_press, cx);
            })
            .on_hover(move |hovered, _, cx| {
                let hovered = *hovered;
                hover_controller.update(cx, |controller, cx| {
                    if controller.is_hovered != hovered {
                        controller.is_hovered = hovered;
                        cx.notify();
                    }
                });
            })
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                let step = match event.keystroke.key.as_str() {
                    "left" => -1,
                    "right" => 1,
                    _ => return,
                };
                let moved = key_controller.update(cx, |controller, cx| controller.step_slot(step, window, cx));
                if moved {
                    cx.stop_propagation();
                }
            })
            .on_scroll_wheel(move |event: &ScrollWheelEvent, window, cx| {
                wheel_controller.update(cx, |controller, cx| controller.handle_scroll_wheel(event, window, cx));
            })
            .gesture(
                ElementId::NamedChild(Arc::new(self.id), SharedString::new_static("drag")),
                DragGesture::new(ShortsRiffPagingController::DRAG_MINIMUM_DISTANCE)
                    .on_changed(move |value: &DragGestureValue, window, cx| {
                        changed_controller.update(cx, |controller, cx| {
                            controller.apply_step(
                                WheelStep::Moved(point(value.translation.width, value.translation.height)),
                                window,
                                cx,
                            );
                        });
                    })
                    .on_ended(move |value: &DragGestureValue, window, cx| {
                        ended_controller.update(cx, |controller, cx| {
                            controller.apply_step(
                                WheelStep::Ended(point(
                                    value.predicted_end_translation.width,
                                    value.predicted_end_translation.height,
                                )),
                                window,
                                cx,
                            );
                        });
                    }),
            )
    }
}

/// mounts only the page on screen and its neighbours, reports the page once a drag settles
pub struct ShortsRiffPagingController {
    offset: Animated<f32>,
    count: usize,
    shown_page: usize,
    right_to_left: bool,
    paging_enabled: bool,
    page_width: f32,
    page: Option<Binding<Option<i64>>>,
    wheel: WheelGestureTracker,
    settle_task: Option<Task<()>>,
    claims_swipe: Option<bool>,
    coasting: bool,
    swipe_base: f32,
    is_hovered: bool,
}

impl ShortsRiffPagingController {
    const DRAG_MINIMUM_DISTANCE: f32 = 10.0;
    const SETTLE_DURATION: f64 = 0.24;
    const PAGE_SWIPE_THRESHOLD: f32 = 0.2;

    fn new() -> Self {
        Self {
            offset: Animated::new(0.0),
            count: 0,
            shown_page: 0,
            right_to_left: false,
            paging_enabled: true,
            page_width: 1.0,
            page: None,
            wheel: WheelGestureTracker::new(),
            settle_task: None,
            claims_swipe: None,
            coasting: false,
            swipe_base: 0.0,
            is_hovered: false,
        }
    }

    fn update(
        &mut self,
        count: usize,
        page: usize,
        paging_enabled: bool,
        right_to_left: bool,
        page_width: f32,
        binding: Binding<Option<i64>>,
        cx: &App,
    ) {
        self.page = Some(binding);
        self.paging_enabled = paging_enabled;
        self.page_width = page_width;
        let layout_changed = count != self.count || right_to_left != self.right_to_left;
        self.count = count;
        self.right_to_left = right_to_left;
        let is_moving = self.claims_swipe == Some(true) || self.wheel.is_tracking();
        if !is_moving && !layout_changed && page == self.shown_page + 1 {
            self.shown_page = page;
            let target = self.slot(page) as f32;
            self.offset.set_in(target, ease_out(Self::SETTLE_DURATION), RiseClock::monotonic(), cx);
        } else if !is_moving && page != self.shown_page {
            self.shown_page = page;
            self.layout_pages();
        } else if layout_changed {
            self.layout_pages();
        }
    }

    fn layout_pages(&mut self) {
        self.offset.jump(self.slot(self.shown_page) as f32);
    }

    fn slot(&self, index: usize) -> usize {
        if self.right_to_left { self.count.saturating_sub(1).saturating_sub(index) } else { index }
    }

    /// continuous page index under the viewport
    fn position(&self, offset: f32) -> f32 {
        if self.right_to_left { self.count.saturating_sub(1) as f32 - offset } else { offset }
    }

    fn mount_window(&self, offset: f32) -> (usize, usize) {
        let position = self.position(offset);
        let last = self.count.saturating_sub(1);
        let low = (position.floor() as i64 - 1).max(0) as usize;
        let high = ((position.ceil() as i64 + 1).max(0) as usize).min(last);
        (low.min(high), high)
    }

    fn is_at_first_page_edge(&self) -> bool {
        let offset = *self.offset.target();
        if self.right_to_left {
            offset >= self.count.saturating_sub(1) as f32 - 0.01
        } else {
            offset <= 0.01
        }
    }

    fn should_begin(&self, translation: Point<Pixels>) -> bool {
        let dx = f32::from(translation.x);
        let dy = f32::from(translation.y);
        if !self.paging_enabled || dx.abs() <= dy.abs() {
            return false;
        }
        let back_sign = if self.right_to_left { -1.0 } else { 1.0 };
        !(self.is_at_first_page_edge() && dx * back_sign > 0.0)
    }

    fn handle_scroll_wheel(&mut self, event: &ScrollWheelEvent, window: &mut Window, cx: &mut Context<Self>) {
        let event = Self::horizontal_from_shift(event);
        let feed = self.wheel.feed(&event, window.line_height());
        let claimed = self.apply_step(feed.step, window, cx);
        if claimed {
            cx.stop_propagation();
        }
        if let Some(token) = feed.settle {
            self.settle_task = Some(cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(WHEEL_SETTLE_DELAY).await;
                this.update_in(cx, |this: &mut Self, window, cx| {
                    if let Some(step) = this.wheel.settle(token) {
                        this.apply_step(step, window, cx);
                    }
                })
                .ok();
            }));
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

    /// true when the event belongs to this pager and not to what lies under it
    fn apply_step(&mut self, step: WheelStep, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match step {
            WheelStep::Ignored => self.coasting,
            WheelStep::Moved(translation) => {
                self.coasting = false;
                if translation == Point::default() {
                    return false;
                }
                let claims = match self.claims_swipe {
                    Some(claims) => claims,
                    None => {
                        let claims = self.should_begin(translation);
                        self.claims_swipe = Some(claims);
                        self.swipe_base = *self.offset.target();
                        claims
                    }
                };
                if claims {
                    let last = self.count.saturating_sub(1) as f32;
                    let moved = f32::from(translation.x) / self.page_width;
                    self.offset.jump((self.swipe_base - moved).clamp(0.0, last));
                    cx.notify();
                }
                claims
            }
            WheelStep::Ended(translation) => {
                let claimed = self.claims_swipe.take() == Some(true);
                if claimed {
                    self.coasting = true;
                    let moved = -f32::from(translation.x) / self.page_width;
                    let target = if moved > Self::PAGE_SWIPE_THRESHOLD {
                        self.swipe_base + 1.0
                    } else if moved < -Self::PAGE_SWIPE_THRESHOLD {
                        self.swipe_base - 1.0
                    } else {
                        self.swipe_base
                    };
                    self.settle(target, window, cx);
                }
                claimed
            }
            WheelStep::Cancelled => {
                let claimed = self.claims_swipe.take() == Some(true);
                if claimed {
                    self.offset.set_in(self.swipe_base, ease_out(Self::SETTLE_DURATION), RiseClock::monotonic(), cx);
                    cx.notify();
                }
                claimed
            }
        }
    }

    /// moves one slot left or right like an arrow press, false at the edge or when paging is off
    fn step_slot(&mut self, step: i64, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.paging_enabled || self.count < 2 || self.claims_swipe == Some(true) {
            return false;
        }
        let target = self.slot(self.shown_page) as i64 + step;
        if target < 0 || target > self.count as i64 - 1 {
            return false;
        }
        self.settle(target as f32, window, cx);
        true
    }

    fn settle(&mut self, target_slot: f32, window: &mut Window, cx: &mut Context<Self>) {
        if self.count == 0 {
            return;
        }
        let last = (self.count - 1) as f32;
        let target_slot = target_slot.round().clamp(0.0, last);
        self.offset.set_in(target_slot, ease_out(Self::SETTLE_DURATION), RiseClock::monotonic(), cx);
        let page = self.position(target_slot).round().clamp(0.0, last) as usize;
        self.shown_page = page;
        if let Some(binding) = self.page.clone()
            && binding.get(cx) != Some(page as i64)
        {
            binding.set(Some(page as i64), window, cx);
        }
        cx.notify();
    }
}
