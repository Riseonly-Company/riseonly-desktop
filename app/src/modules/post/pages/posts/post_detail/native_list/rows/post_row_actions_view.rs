use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::{Rc, Weak};
use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext, Bounds, ClickEvent, Context, Entity, FocusHandle, FontWeight, Hsla,
    InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement, Pixels, Render, Role, Size,
    StatefulInteractiveElement, Styled, Window, div, px, size,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_i18n::tr;
use rise_ui::swiftui::animated::{Animated, MotionTime};
use rise_ui::swiftui::rich_text::RichFont;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::TransitionModifiers;

use crate::core::lib::canon_format::RiseFormat;
use crate::modules::post::pages::posts::post_detail::native_list::contracts::post_list_contracts::{
    PostRowActionRouting, PostRowActionsLayout, PostRowMainContext, PostRowSeedCounts,
};
use crate::modules::post::pages::posts::post_detail::native_list::model::post_row_layout_calculator::{
    PostRowMetrics, PostRowTextMetrics,
};
use crate::modules::post::pages::posts::post_detail::native_list::rows::post_native_row_view::{
    PostRowChromeFonts, PostRowChromeSpring, PostRowChromeSymbols, PostRowTextLineView, PostRowTextLineViewAlignment,
    post_row_frame,
};
use crate::modules::sparks::components::sparks_icon_view::SparksBurstIcon;
use crate::modules::sparks::stores::sparks::sparks_actions::sparks_types::SparksTarget;

/// feed style actions, like and favorite and sparks when shown
/// comments and repost in equal cells cell frames come from
/// the actions layout and content is centered in each cell
pub struct PostRowActionsView {
    like_content_scale: Animated<f32>,
    like_icon: Option<(&'static str, Hsla)>,
    like_count_view: Entity<PostRowTextLineView>,
    like_focus: FocusHandle,
    like_count_focus: FocusHandle,
    favorite_cell: PostRowActionCell,
    comments_cell: PostRowActionCell,
    repost_cell: PostRowActionCell,
    sparks_cell: PostRowActionCell,
    is_sparks_cell_shown: bool,

    actions_layout: Option<PostRowActionsLayout>,
    post_id: Option<String>,
    router: Option<Weak<dyn PostRowActionRouting>>,
    likes: i64,
    like_count_width: f32,
    display_scale: f32,
    like_bump_generation: u64,
    favorite_bump_generation: u64,
    sparks_taps: i64,
    like_label: &'static str,
}

impl PostRowActionsView {
    pub const ICON_POINT_SIZE: f32 = 18.0;
    const LIKE_ICON_SIDE: f32 = 30.0;
    const LIKE_SPACING: f32 = 3.0;
    const LIKE_COUNT_MINIMUM_WIDTH: f32 = 22.0;

    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            like_content_scale: Animated::new(1.0),
            like_icon: None,
            like_count_view: cx.new(|_| PostRowTextLineView::new()),
            like_focus: cx.focus_handle().tab_stop(true),
            like_count_focus: cx.focus_handle().tab_stop(true),
            favorite_cell: PostRowActionCell::new(cx),
            comments_cell: PostRowActionCell::new(cx),
            repost_cell: PostRowActionCell::new(cx),
            sparks_cell: PostRowActionCell::new(cx),
            is_sparks_cell_shown: false,
            actions_layout: None,
            post_id: None,
            router: None,
            likes: 0,
            like_count_width: 0.0,
            display_scale: 3.0,
            like_bump_generation: 0,
            favorite_bump_generation: 0,
            sparks_taps: 0,
            like_label: "ctx_like",
        }
    }

    pub fn configure(
        &mut self,
        post_id: &str,
        counts: &PostRowSeedCounts,
        layout: &PostRowActionsLayout,
        context: &PostRowMainContext,
        cx: &mut Context<Self>,
    ) {
        self.post_id = Some(post_id.to_owned());
        self.router = Some(Rc::downgrade(&context.router));
        self.actions_layout = Some(layout.clone());
        self.likes = if counts.shows_likes { counts.likes } else { 0 };
        self.display_scale = context.spec.display_scale;

        let palette = &context.palette;
        let muted = palette.text_secondary;
        let scale = self.display_scale;
        let font =
            PostRowChromeFonts::text(PostRowMetrics::ACTIONS_COUNT_FONT_SIZE, FontWeight::NORMAL, &context.spec);

        let like_text = if counts.shows_likes { RiseFormat::compact_count(counts.likes) } else { String::new() };
        self.like_icon = Some((
            if counts.is_liked { "heart.fill" } else { "heart" },
            if counts.is_liked { palette.primary100 } else { muted },
        ));
        self.like_count_width = PostRowActionCountWidth::width(&like_text, &font, scale);
        self.like_count_view.update(cx, |view, cx| {
            view.set_text(&like_text, font, muted, PostRowTextLineViewAlignment::Leading, cx)
        });

        let favorite_text = RiseFormat::compact_count(counts.favorites);
        self.favorite_cell.configure_symbol(
            if counts.is_favorited { "bookmark.fill" } else { "bookmark" },
            if counts.is_favorited { palette.primary100 } else { muted },
            &favorite_text,
            font,
            muted,
            scale,
            cx,
        );
        let comments_text = if counts.shows_comments { RiseFormat::compact_count(counts.comments) } else { String::new() };
        self.comments_cell.configure_symbol("bubble.right", muted, &comments_text, font, muted, scale, cx);
        let repost_text = if counts.shows_reposts { RiseFormat::compact_count(counts.reposts) } else { String::new() };
        self.repost_cell.configure_symbol(
            "arrow.2.squarepath",
            if counts.is_reposted { palette.primary100 } else { muted },
            &repost_text,
            font,
            muted,
            scale,
            cx,
        );

        self.like_label = if counts.is_liked { "ctx_unlike" } else { "ctx_like" };
        if counts.is_liked {
            x251_mark(|| format!("x251 configure post={post_id} liked=true likes={}", counts.likes));
        }

        if counts.shows_sparks {
            let sparks_text =
                if counts.shows_sparks_total { RiseFormat::compact_count(counts.sparks_total) } else { String::new() };
            self.sparks_cell.configure_image(Self::ICON_POINT_SIZE, &sparks_text, font, muted, scale, cx);
        }
        self.is_sparks_cell_shown = counts.shows_sparks;

        cx.notify();
    }

    pub fn prepare_for_reuse(&mut self, cx: &mut Context<Self>) {
        self.cancel_bumps(cx);
        self.post_id = None;
        self.actions_layout = None;
    }

    pub fn bump_like(&mut self, cx: &mut Context<Self>) {
        self.like_bump_generation = self.like_bump_generation.wrapping_add(1);
        PostRowChromeSpring::animate(&mut self.like_content_scale, 1.12, 0.22, 0.62, cx);
        self.bump_settle(self.like_bump_generation, true, cx);
        cx.notify();
    }

    pub fn cancel_bumps(&mut self, cx: &mut Context<Self>) {
        self.like_bump_generation = self.like_bump_generation.wrapping_add(1);
        self.favorite_bump_generation = self.favorite_bump_generation.wrapping_add(1);
        PostRowChromeSpring::reset(&mut self.like_content_scale, 1.0);
        PostRowChromeSpring::reset(&mut self.favorite_cell.content_scale, 1.0);
        cx.notify();
    }

    fn bump_favorite(&mut self, cx: &mut Context<Self>) {
        self.favorite_bump_generation = self.favorite_bump_generation.wrapping_add(1);
        PostRowChromeSpring::animate(&mut self.favorite_cell.content_scale, 1.1, 0.22, 0.62, cx);
        self.bump_settle(self.favorite_bump_generation, false, cx);
        cx.notify();
    }

    fn bump_settle(&self, generation: u64, is_like: bool, cx: &mut Context<Self>) {
        let executor = cx.background_executor().clone();
        cx.spawn(async move |this, cx| {
            executor.timer(Duration::from_secs_f64(0.14)).await;
            let _ = this.update(cx, |this, cx| {
                let current = if is_like { this.like_bump_generation } else { this.favorite_bump_generation };
                if current != generation {
                    return;
                }
                let scale =
                    if is_like { &mut this.like_content_scale } else { &mut this.favorite_cell.content_scale };
                PostRowChromeSpring::animate(scale, 1.0, 0.26, 0.78, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn acting_router(&self) -> Option<(Rc<dyn PostRowActionRouting>, String)> {
        let router = self.router.as_ref().and_then(Weak::upgrade)?;
        Some((router, self.post_id.clone()?))
    }

    fn handle_like_tap(&mut self, cx: &mut Context<Self>) {
        x251_mark(|| {
            format!("x251 tap post={:?} router={}", self.post_id, self.router.as_ref().is_some_and(|r| r.upgrade().is_some()))
        });
        let Some((router, post_id)) = self.acting_router() else {
            return;
        };
        self.bump_like(cx);
        router.toggle_like(&post_id, cx);
    }

    /// opens the likes list, only when the count is positive
    fn handle_likes_open(&mut self, cx: &mut Context<Self>) {
        if self.likes <= 0 {
            return;
        }
        if let Some((router, post_id)) = self.acting_router() {
            router.open_likes(&post_id, cx);
        }
    }

    fn handle_favorite(&mut self, cx: &mut Context<Self>) {
        let Some((router, post_id)) = self.acting_router() else {
            return;
        };
        self.bump_favorite(cx);
        router.toggle_favorite(&post_id, cx);
    }

    fn handle_comments(&mut self, cx: &mut Context<Self>) {
        if let Some((router, post_id)) = self.acting_router() {
            router.open_comments(&post_id, cx);
        }
    }

    fn handle_repost(&mut self, cx: &mut Context<Self>) {
        x251_mark(|| {
            format!("x251 repost tap post={:?}", self.post_id)
        });
        if let Some((router, post_id)) = self.acting_router() {
            router.toggle_repost(&post_id, cx);
        }
    }

    fn handle_sparks_tap(&mut self, cx: &mut Context<Self>) {
        let Some((router, post_id)) = self.acting_router() else {
            return;
        };
        if router.sparks_tap(&post_id, cx) {
            self.sparks_taps = self.sparks_taps.wrapping_add(1);
            cx.notify();
        }
    }

    fn handle_sparks_hold(&mut self, cx: &mut Context<Self>) {
        if let Some((router, post_id)) = self.acting_router() {
            router.sparks_hold(&post_id, cx);
        }
    }

    fn like_cell(&self, frame: Bounds<Pixels>, now: MotionTime, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let scale = self.display_scale;
        let width = f32::from(frame.size.width);
        let height = f32::from(frame.size.height);
        let count_width =
            if self.like_count_width > 0.0 { Self::LIKE_COUNT_MINIMUM_WIDTH.max(self.like_count_width) } else { 0.0 };
        let content_width =
            Self::LIKE_ICON_SIDE + if count_width > 0.0 { Self::LIKE_SPACING } else { 0.0 } + count_width;
        let origin_x = PostRowTextMetrics::pixel_round((width - content_width) / 2.0, scale);
        let item_y = PostRowTextMetrics::pixel_round((height - Self::LIKE_ICON_SIDE) / 2.0, scale);
        let like_icon = self.like_icon.and_then(|(name, tint)| {
            PostRowChromeSymbols::image(name, Self::ICON_POINT_SIZE, FontWeight::NORMAL)
                .map(|icon| icon.text_color(tint))
        });
        let mut like_count = div()
            .id("post-row-like-count")
            .absolute()
            .left(px(Self::LIKE_ICON_SIDE + Self::LIKE_SPACING))
            .top(px(item_y))
            .w(px(count_width))
            .h(px(Self::LIKE_ICON_SIDE));
        if self.likes > 0 {
            like_count = like_count
                .track_focus(&self.like_count_focus)
                .role(Role::Button)
                .aria_label(tr("post_likes_title"))
                .cursor_pointer()
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.handle_likes_open(cx);
                }));
        }
        let like_label = tr(self.like_label);
        let content = div()
            .absolute()
            .left(px(origin_x))
            .top(px(0.0))
            .w(px(content_width))
            .h(px(height))
            .child(
                div()
                    .id("post-row-like")
                    .track_focus(&self.like_focus)
                    .role(Role::Button)
                    .aria_label(like_label.clone())
                    .absolute()
                    .left(px(0.0))
                    .top(px(item_y))
                    .size(px(Self::LIKE_ICON_SIDE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .tooltip_label(like_label)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.handle_like_tap(cx);
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, _: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.handle_likes_open(cx);
                        }),
                    )
                    .children(like_icon),
            )
            .child(like_count.child(self.like_count_view.clone()));
        let scale_value = self.like_content_scale.frame_value(now, window);
        post_row_frame(frame)
            .child(content.scale_effect(scale_value))
            .into_any_element()
    }
}

impl Render for PostRowActionsView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(layout) = self.actions_layout.clone() else {
            return div().into_any_element();
        };
        let now = RiseClock::monotonic();
        let mut actions = div().relative().size_full().child(self.like_cell(layout.like, now, window, cx));

        actions = actions.child(
            self.favorite_cell
                .view("post-row-favorite", layout.favorite, None, now, window)
                .aria_label(tr("post_stats_favorites"))
                .tooltip_label(tr("post_stats_favorites"))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.handle_favorite(cx);
                })),
        );
        if self.is_sparks_cell_shown
            && let (Some(frame), Some(post_id)) = (layout.sparks, self.post_id.clone())
        {
            let icon = SparksBurstIcon::new(Self::ICON_POINT_SIZE, SparksTarget::Post { post_id }, self.sparks_taps);
            actions = actions.child(
                self.sparks_cell
                    .view("post-row-sparks", frame, Some(icon.into_any_element()), now, window)
                    .aria_label(tr("sparks_send_title"))
                    .tooltip_label(tr("sparks_send_title"))
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.handle_sparks_tap(cx);
                    }))
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, _: &MouseDownEvent, _, cx| {
                            cx.stop_propagation();
                            this.handle_sparks_hold(cx);
                        }),
                    ),
            );
        }
        actions = actions.child(
            self.comments_cell
                .view("post-row-comments", layout.comments, None, now, window)
                .aria_label(tr("post_stats_comments"))
                .tooltip_label(tr("post_stats_comments"))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.handle_comments(cx);
                })),
        );
        actions = actions.child(
            self.repost_cell
                .view("post-row-repost", layout.repost, None, now, window)
                .aria_label(tr("post_stats_reposts"))
                .tooltip_label(tr("post_stats_reposts"))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    cx.stop_propagation();
                    this.handle_repost(cx);
                })),
        );

        actions.into_any_element()
    }
}

enum PostRowActionCellIcon {
    Symbol { name: &'static str, tint: Hsla },
    Image,
}

/// one action cell, icon then 6 pt gap then count
/// centered in the cell
/// content_scale carries the bump
pub struct PostRowActionCell {
    pub content_scale: Animated<f32>,
    icon: PostRowActionCellIcon,
    count_view: Entity<PostRowTextLineView>,
    focus_handle: FocusHandle,
    icon_size: Size<Pixels>,
    count_width: f32,
    display_scale: f32,
}

impl PostRowActionCell {
    const SPACING: f32 = 6.0;

    pub fn new(cx: &mut App) -> Self {
        Self {
            content_scale: Animated::new(1.0),
            icon: PostRowActionCellIcon::Image,
            count_view: cx.new(|_| PostRowTextLineView::new()),
            focus_handle: cx.focus_handle().tab_stop(true),
            icon_size: size(px(0.0), px(0.0)),
            count_width: 0.0,
            display_scale: 3.0,
        }
    }

    pub fn configure_image(
        &mut self,
        side: f32,
        count: &str,
        font: RichFont,
        text_color: Hsla,
        scale: f32,
        cx: &mut App,
    ) {
        self.icon = PostRowActionCellIcon::Image;
        self.icon_size = size(px(side), px(side));
        self.configure_count(count, font, text_color, scale, cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn configure_symbol(
        &mut self,
        symbol: &'static str,
        tint: Hsla,
        count: &str,
        font: RichFont,
        text_color: Hsla,
        scale: f32,
        cx: &mut App,
    ) {
        self.icon = PostRowActionCellIcon::Symbol { name: symbol, tint };
        self.icon_size =
            PostRowTextMetrics::symbol_size(symbol, PostRowActionsView::ICON_POINT_SIZE, FontWeight::NORMAL);
        self.configure_count(count, font, text_color, scale, cx);
    }

    fn configure_count(&mut self, count: &str, font: RichFont, text_color: Hsla, scale: f32, cx: &mut App) {
        self.count_width = PostRowActionCountWidth::width(count, &font, scale);
        self.count_view.update(cx, |view, cx| {
            view.set_text(count, font, text_color, PostRowTextLineViewAlignment::Leading, cx)
        });
        self.display_scale = scale;
    }

    /// the cell at its frame, in the actions view coords
    pub fn view(
        &self,
        id: &'static str,
        frame: Bounds<Pixels>,
        image: Option<AnyElement>,
        now: MotionTime,
        window: &Window,
    ) -> gpui::Stateful<gpui::Div> {
        let width = f32::from(frame.size.width);
        let height = f32::from(frame.size.height);
        let icon_width = f32::from(self.icon_size.width);
        let icon_height = f32::from(self.icon_size.height);
        let spacing = if self.count_width > 0.0 { Self::SPACING } else { 0.0 };
        let content_width = icon_width + spacing + self.count_width;
        let origin_x = PostRowTextMetrics::pixel_round((width - content_width) / 2.0, self.display_scale);
        let icon_y = PostRowTextMetrics::pixel_round((height - icon_height) / 2.0, self.display_scale);

        let symbol = match &self.icon {
            PostRowActionCellIcon::Symbol { name, tint } => {
                PostRowChromeSymbols::image(name, PostRowActionsView::ICON_POINT_SIZE, FontWeight::NORMAL)
                    .map(|icon| icon.text_color(*tint).into_any_element())
            }
            PostRowActionCellIcon::Image => image,
        };
        let content = div()
            .absolute()
            .left(px(origin_x))
            .top(px(0.0))
            .w(px(content_width))
            .h(px(height))
            .child(
                div()
                    .absolute()
                    .left(px(0.0))
                    .top(px(icon_y))
                    .w(px(icon_width))
                    .h(px(icon_height))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(symbol),
            )
            .child(
                div()
                    .absolute()
                    .left(px(icon_width + Self::SPACING))
                    .top(px(0.0))
                    .w(px(self.count_width))
                    .h(px(height))
                    .child(self.count_view.clone()),
            );
        let scale = self.content_scale.frame_value(now, window);
        post_row_frame(frame)
            .id(id)
            .track_focus(&self.focus_handle)
            .role(Role::Button)
            .cursor_pointer()
            .child(content.scale_effect(scale))
    }
}

/// widths of compact count strings, memoized since counts repeat across rows
enum PostRowActionCountWidth {}

thread_local! {
    static POST_ROW_ACTION_COUNT_WIDTHS: RefCell<HashMap<(String, u32, u32, u32), f32>> =
        RefCell::new(HashMap::new());
}

impl PostRowActionCountWidth {
    fn width(text: &str, font: &RichFont, scale: f32) -> f32 {
        let key = (text.to_owned(), font.size.to_bits(), font.weight.0.to_bits(), scale.to_bits());
        POST_ROW_ACTION_COUNT_WIDTHS.with_borrow_mut(|cache| {
            if let Some(hit) = cache.get(&key) {
                return *hit;
            }
            let width = PostRowTextMetrics::width(text, font, scale);
            if cache.len() >= 512 {
                cache.clear();
            }
            cache.insert(key, width);
            width
        })
    }
}

fn x251_mark(stage: impl FnOnce() -> String) {
    tracing::warn!(target: "riseonly::X251", "{}", stage());
}
