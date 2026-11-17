use std::rc::Rc;

use gpui::{
    App, Bounds, BoxShadow, Context, ElementId, Empty, Entity, FontWeight, Hsla, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, ParentElement, PathBuilder, Pixels, RenderOnce, Size, Styled, Window, canvas, div,
    hsla, point, px, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_text::character_count;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::liquid_chrome::LiquidChromeModifiers;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::{MotionSpec, ease_in_out, spring};
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::transition::{Presence, Transition, TransitionModifiers, UnitPoint};
use rise_ui::{IconUi, MainText, TextAlignT};

use crate::app::router::navigation_store::{NavigationStoreKey, NavigationStoreSettlingPush};
use crate::app::router::navigation_types::RootRoute;
use crate::modules::post::components::collaborators::post_collaborators_presenter::PostCollaboratorsPresenter;
use crate::modules::post::components::repost::repost_overlay::RepostOverlayMetrics;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;

/// media state of one card, visible page and whether tag bubbles are open
/// separate entity so a gallery swipe redraws only the chip and bubbles not the feed row
pub struct PostMediaTagsState {
    pub slide: i64,
    pub shows_tags: bool,
    presence: Presence<()>,
}

impl PostMediaTagsState {
    pub fn fade() -> MotionSpec {
        ease_in_out(0.22)
    }

    pub fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            slide: 0,
            shows_tags: false,
            presence: Presence::new(None).with_transition(Transition::opacity()),
        }
    }

    pub fn set_slide(&mut self, value: i64, cx: &mut Context<Self>) {
        if self.slide == value {
            return;
        }
        self.slide = value;
        self.set_shows_tags(false, cx);
    }

    pub fn toggle(&mut self, cx: &mut Context<Self>) {
        self.set_shows_tags(!self.shows_tags, cx);
    }

    pub fn hide(&mut self, cx: &mut Context<Self>) {
        if !self.shows_tags {
            return;
        }
        self.set_shows_tags(false, cx);
    }

    fn set_shows_tags(&mut self, shows_tags: bool, cx: &mut Context<Self>) {
        self.shows_tags = shows_tags;
        let value = shows_tags.then_some(());
        self.presence.set_in(value, Self::fade(), RiseClock::monotonic(), cx);
        cx.notify();
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PostTagBubble {
    pub id: String,
    pub user_id: String,
    pub tag: String,
    pub left: f32,
    pub top: f32,
    pub width: f32,
    pub order: usize,
    pub is_dark: bool,
}

pub enum PostTagBubbleLayout {}

impl PostTagBubbleLayout {
    pub const PILL_HEIGHT: f32 = 24.0;
    pub const POINTER_HEIGHT: f32 = 5.0;
    pub const POINTER_WIDTH: f32 = 12.0;
    pub const EDGE_INSET: f32 = 6.0;

    /// width from character count not text measurement
    /// measuring every pill would cost a layout pass per show and a few points of error is invisible
    pub fn width(for_tag: &str) -> f32 {
        (22.0 + (character_count(for_tag) + 1) as f32 * 7.0).max(48.0)
    }

    pub fn bubbles(post: &GetPostFeedResponse, slide_index: i64, size: Size<Pixels>) -> Vec<PostTagBubble> {
        let (width, height) = (f32::from(size.width), f32::from(size.height));
        if width <= 1.0 || height <= 1.0 {
            return Vec::new();
        }

        let mut result: Vec<PostTagBubble> = Vec::new();
        for tagged in post.tagged_users(slide_index) {
            let Some(user) = &tagged.user else {
                continue;
            };
            let pill_width = Self::width(&user.tag);
            for placement in tagged.placements.iter().filter(|placement| placement.slide_index == slide_index) {
                let anchor_x = placement.x as f32 * width;
                let anchor_y = placement.y as f32 * height;
                let max_left = Self::EDGE_INSET.max(width - pill_width - Self::EDGE_INSET);
                let max_top =
                    Self::EDGE_INSET.max(height - Self::PILL_HEIGHT - Self::POINTER_HEIGHT - Self::EDGE_INSET);
                result.push(PostTagBubble {
                    id: format!("{}-{}", user.id, result.len()),
                    user_id: user.id.clone(),
                    tag: user.tag.clone(),
                    left: Self::EDGE_INSET.max(anchor_x - pill_width / 2.0).min(max_left),
                    top: Self::EDGE_INSET.max(anchor_y).min(max_top),
                    width: pill_width,
                    order: result.len(),
                    is_dark: placement.is_dark,
                });
            }
        }
        result
    }
}

/// corner chrome over the media (repost sticker, tags chip) sits a fixed inset from the edges
/// the music bar at the bottom pushes it up
pub enum PostMediaCornerChrome {}

impl PostMediaCornerChrome {
    pub const INSET: f32 = 15.0;

    pub fn bottom_inset(post: &GetPostFeedResponse) -> f32 {
        if post.track_model().is_some() { RepostOverlayMetrics::MEDIA_BOTTOM_SAFE_ZONE } else { Self::INSET }
    }
}

/// shown for every post with tagged people
/// tap reveals bubbles if the visible slide has placements, else opens the list
/// long press always opens the list
#[derive(IntoElement)]
pub struct PostTagsChip {
    post: Rc<GetPostFeedResponse>,
    state: Entity<PostMediaTagsState>,
    viewer_is_author: bool,
}

impl PostTagsChip {
    pub fn new(post: impl Into<Rc<GetPostFeedResponse>>, state: Entity<PostMediaTagsState>) -> Self {
        Self { post: post.into(), state, viewer_is_author: false }
    }

    pub fn viewer_is_author(mut self, viewer_is_author: bool) -> Self {
        self.viewer_is_author = viewer_is_author;
        self
    }

    fn open_list(post_id: &str, state: &Entity<PostMediaTagsState>, viewer_is_author: bool, cx: &mut App) {
        state.update(cx, |state, cx| state.hide(cx));
        PostCollaboratorsPresenter::shared(cx).update(cx, |presenter, cx| {
            presenter.present_tagged_people(post_id, viewer_is_author, cx);
        });
    }
}

impl RenderOnce for PostTagsChip {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        if self.post.tagged_users.is_empty() {
            return Empty.into_any_element();
        }
        let viewer_is_author = self.viewer_is_author;
        let tap_post = self.post.clone();
        let tap_state = self.state.clone();
        let hold_post_id = self.post.id.clone();
        let hold_state = self.state;
        let glyph = div()
            .size(px(28.0))
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized("person.crop.circle", px(14.0), white(), false))
            .liquid_chrome_circle(None);

        div()
            .id(ElementId::Name(format!("post-tags-chip-{}", self.post.id).into()))
            .size(px(28.0))
            .circle()
            .cursor_pointer()
            .shadow(vec![BoxShadow {
                color: hsla(0.0, 0.0, 0.0, 0.28),
                offset: point(px(0.0), px(2.0)),
                blur_radius: px(6.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            .on_click(move |_, _, cx| {
                let _ = selection_changed();
                if tap_post.has_tags(tap_state.read(cx).slide) {
                    tap_state.update(cx, |state, cx| state.toggle(cx));
                } else {
                    Self::open_list(&tap_post.id, &tap_state, viewer_is_author, cx);
                }
            })
            .on_mouse_down(MouseButton::Right, move |_: &MouseDownEvent, _, cx| {
                cx.stop_propagation();
                Self::open_list(&hold_post_id, &hold_state, viewer_is_author, cx);
            })
            .tooltip_label(tr("post_tagged_people_button_hint"))
            .child(glyph)
            .into_any_element()
    }
}

/// bubble layer over the media
/// mounted only while tags are open so the normal feed pays for no measure or pill layout
#[derive(IntoElement, Clone)]
pub struct PostTagBubblesLayer {
    post: Rc<GetPostFeedResponse>,
    state: Entity<PostMediaTagsState>,
}

impl PostTagBubblesLayer {
    pub fn new(post: impl Into<Rc<GetPostFeedResponse>>, state: Entity<PostMediaTagsState>) -> Self {
        Self { post: post.into(), state }
    }
}

impl RenderOnce for PostTagBubblesLayer {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let post = self.post;
        let tags_state = self.state;
        let media_size = window.use_keyed_state(
            ElementId::Name(format!("post-tag-bubbles-size-{}", post.id).into()),
            cx,
            |_, _| Size::<Pixels>::default(),
        );
        let measured_size = *media_size.read(cx);
        let state = tags_state.read(cx);
        let slide = state.slide;
        let dismiss_state = tags_state.clone();
        let revealed = state.presence.render_transitioned(RiseClock::monotonic(), window, |_| {
            let measure_size = media_size.clone();
            div()
                .absolute()
                .inset_0()
                .child(
                    PostTagBubblesOverlay::new(post.clone(), slide, measured_size).on_dismiss(move |_, cx| {
                        dismiss_state.update(cx, |state, cx| state.hide(cx));
                    }),
                )
                .child(measure(
                    ElementId::Name(format!("post-tag-bubbles-measure-{}", post.id).into()),
                    move |bounds: Bounds<Pixels>, _, cx| {
                        measure_size.update(cx, |size, cx| {
                            *size = bounds.size;
                            cx.notify();
                        });
                    },
                ))
        });
        match revealed {
            Some(revealed) => revealed.into_any_element(),
            None => Empty.into_any_element(),
        }
    }
}

#[derive(IntoElement)]
pub struct PostTagBubblesOverlay {
    post: Rc<GetPostFeedResponse>,
    slide_index: i64,
    media_size: Size<Pixels>,
    on_dismiss: ButtonAction,
}

impl PostTagBubblesOverlay {
    pub fn new(post: impl Into<Rc<GetPostFeedResponse>>, slide_index: i64, media_size: Size<Pixels>) -> Self {
        Self { post: post.into(), slide_index, media_size, on_dismiss: Rc::new(|_, _| {}) }
    }

    pub fn on_dismiss(mut self, on_dismiss: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_dismiss = Rc::new(on_dismiss);
        self
    }

    fn pill(bubble: &PostTagBubble, on_dismiss: ButtonAction, cx: &App) -> impl IntoElement + Styled + use<> {
        let fill: Hsla = if bubble.is_dark { hsla(0.0, 0.0, 0.08, 0.92) } else { hsla(0.0, 0.0, 1.0, 0.94) };
        let ink: Hsla = if bubble.is_dark { white() } else { hsla(0.0, 0.0, 0.06, 1.0) };
        let nav_store = env::<NavigationStoreKey>(cx);
        let user_id = bubble.user_id.clone();
        let tag = bubble.tag.clone();
        vstack(0.0)
            .id(ElementId::Name(format!("post-tag-bubble-{}", bubble.id).into()))
            .w(px(bubble.width))
            .cursor_pointer()
            .occlude()
            .on_click(move |_, window, cx| {
                on_dismiss(window, cx);
                if let Some(nav_store) = &nav_store {
                    let route = RootRoute::UserPage { user_id: Some(user_id.clone()), tag: Some(tag.clone()) };
                    nav_store.update(cx, |nav_store, cx| nav_store.push(route, NavigationStoreSettlingPush::Drop, cx));
                }
            })
            .child(PostTagPointerShape.fill(fill))
            .child(
                div()
                    .w(px(bubble.width))
                    .h(px(PostTagBubbleLayout::PILL_HEIGHT))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(fill)
                    .continuous_corners(9.0)
                    .child(
                        MainText::new(format!("@{}", bubble.tag))
                            .px(12.0)
                            .tac(TextAlignT::Center)
                            .font_weight(FontWeight::SEMIBOLD)
                            .color(ink)
                            .number_of_lines(1),
                    ),
            )
    }
}

impl RenderOnce for PostTagBubblesOverlay {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let bubbles = PostTagBubbleLayout::bubbles(&self.post, self.slide_index, self.media_size);
        let on_dismiss = self.on_dismiss;
        let backdrop_dismiss = on_dismiss.clone();
        let now = RiseClock::monotonic();

        let mut layer = div().absolute().inset_0().child(
            div()
                .id(ElementId::Name(format!("post-tag-bubbles-backdrop-{}", self.post.id).into()))
                .absolute()
                .inset_0()
                .block_mouse_except_scroll()
                .on_click(move |_, window, cx| backdrop_dismiss(window, cx)),
        );

        for bubble in &bubbles {
            let order = bubble.order;
            let appeared = window.use_keyed_state(
                ElementId::Name(format!("post-tag-bubble-appear-{}-{}", self.post.id, bubble.id).into()),
                cx,
                move |_, cx: &mut Context<Animated<f32>>| {
                    let mut appeared = Animated::new(0.0_f32);
                    appeared.set_in(1.0, spring(0.3, 0.74).delay(order as f64 * 0.03), RiseClock::monotonic(), cx);
                    appeared
                },
            );
            let progress = appeared.read(cx).frame_value(now, window);
            layer = layer.child(
                div().absolute().left(px(bubble.left)).top(px(bubble.top)).child(
                    Self::pill(bubble, on_dismiss.clone(), cx)
                        .opacity(progress.clamp(0.0, 1.0))
                        .scale_effect_anchored(0.8 + 0.2 * progress, UnitPoint::TOP),
                ),
            );
        }
        layer
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PostTagPointerShape;

impl PostTagPointerShape {
    pub fn path(self, in_rect: Bounds<Pixels>) -> PathBuilder {
        let mut path = PathBuilder::fill();
        path.move_to(point(in_rect.center().x, in_rect.top()));
        path.line_to(point(in_rect.right(), in_rect.bottom()));
        path.line_to(point(in_rect.left(), in_rect.bottom()));
        path.close();
        path
    }

    pub fn fill(self, color: Hsla) -> impl IntoElement + Styled {
        canvas(
            |_, _, _| (),
            move |bounds, (), window, _| {
                if let Ok(path) = self.path(bounds).build() {
                    window.paint_path(path, color);
                }
            },
        )
        .w(px(PostTagBubbleLayout::POINTER_WIDTH))
        .h(px(PostTagBubbleLayout::POINTER_HEIGHT))
    }
}
