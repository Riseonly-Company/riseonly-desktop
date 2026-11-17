use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, Hsla, InteractiveElement, IntoElement, ParentElement, RenderOnce, Rgba,
    StatefulInteractiveElement, Styled, Window, black, div, prelude::FluentBuilder, px, white,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_platform::haptics::{ImpactStyle, impact};
use rise_ui::IconUi;
use rise_ui::hold_context_menu_ui::hold_context_menu_frame_keys::{
    HoldContextMenuHitTarget, HoldContextMenuHitTargets, HoldContextMenuHitTargetsKey,
};
use rise_ui::hold_context_menu_ui::hold_context_menu_glass::{HoldContextMenuGlass, HoldContextMenuShadow};
use rise_ui::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAlignment;
use rise_ui::swiftui::animated::{Animated, MotionTime};
use rise_ui::swiftui::environment::reduce_motion;
use rise_ui::swiftui::motion::{MotionSpec, Spring};
use rise_ui::swiftui::shapes::{Shape, centred_stroke};
use rise_ui::swiftui::transition::{TransitionModifiers, UnitPoint};

use crate::core::ui::reaction_emoji_glyph::ReactionEmojiGlyph;

/// called with the picked emoji or the owner action of the expand key
pub type HoldContextMenuEmojiPick = Rc<dyn Fn(String, &mut Window, &mut App)>;
pub type HoldContextMenuEmojiExpand = Rc<dyn Fn(&mut Window, &mut App)>;

const APPEAR_HIDDEN_SCALE: f32 = 0.45;
const EXPAND_HOVER_SCALE: f32 = 1.12;
const APPEAR_STAGGER: f64 = 0.02;
const APPEAR_SPRING: Spring = Spring::new(0.34, 0.72);
const HIGHLIGHT_SPRING: Spring =
    Spring::new(HoldContextMenuMetrics::HIGHLIGHT_RESPONSE, HoldContextMenuMetrics::HIGHLIGHT_DAMPING);

/// scale and fade of one slot, the appear spring with its stagger and the hover spring on top
struct EmojiSlotMotion {
    hovered: bool,
    scale: Animated<f32>,
    opacity: Animated<f32>,
}

impl EmojiSlotMotion {
    fn new() -> Self {
        Self {
            hovered: false,
            scale: Animated::new(APPEAR_HIDDEN_SCALE),
            opacity: Animated::new(0.0),
        }
    }

    fn sync(&mut self, hovered: bool, hover_scale: f32, index: usize, now: MotionTime, reduce_motion: bool) {
        let target = if hovered { hover_scale } else { 1.0 };
        let spec = if hovered != self.hovered {
            MotionSpec::spring(HIGHLIGHT_SPRING)
        } else {
            MotionSpec::spring(APPEAR_SPRING).delay(index as f64 * APPEAR_STAGGER)
        };
        self.hovered = hovered;
        self.scale.set(target, spec, now, reduce_motion);
        self.opacity
            .set(1.0, MotionSpec::spring(APPEAR_SPRING).delay(index as f64 * APPEAR_STAGGER), now, reduce_motion);
    }
}

#[derive(Default)]
struct HoldContextMenuEmojiBarMotion {
    slots: Vec<EmojiSlotMotion>,
}

#[derive(IntoElement)]
pub struct HoldContextMenuEmojiBar {
    id: ElementId,
    pub emojis: Vec<String>,
    pub alignment: HoldContextMenuAlignment,
    pub publishes_interaction_frames: bool,
    pub hit_targets: Option<HoldContextMenuHitTargets>,
    pub is_revealed: bool,
    pub hovered_emoji: Option<String>,
    pub on_select: HoldContextMenuEmojiPick,
    pub on_expand: Option<HoldContextMenuEmojiExpand>,
}

impl HoldContextMenuEmojiBar {
    pub const EXPAND_KEY: &'static str = "__emoji_expand__";

    /// constructor with publishesInteractionFrames, hitTargets
    /// isRevealed and onExpand at their defaults
    /// the builders below set them
    pub fn new(
        id: impl Into<ElementId>,
        emojis: Vec<String>,
        alignment: HoldContextMenuAlignment,
        hovered_emoji: Option<String>,
        on_select: HoldContextMenuEmojiPick,
    ) -> Self {
        Self {
            id: id.into(),
            emojis,
            alignment,
            publishes_interaction_frames: true,
            hit_targets: None,
            is_revealed: true,
            hovered_emoji,
            on_select,
            on_expand: None,
        }
    }

    pub fn publishes_interaction_frames(mut self, publishes_interaction_frames: bool) -> Self {
        self.publishes_interaction_frames = publishes_interaction_frames;
        self
    }

    pub fn hit_targets(mut self, hit_targets: Option<HoldContextMenuHitTargets>) -> Self {
        self.hit_targets = hit_targets;
        self
    }

    pub fn is_revealed(mut self, is_revealed: bool) -> Self {
        self.is_revealed = is_revealed;
        self
    }

    pub fn on_expand(mut self, on_expand: Option<HoldContextMenuEmojiExpand>) -> Self {
        self.on_expand = on_expand;
        self
    }

    fn hit_target(&self, key: &str) -> Option<HoldContextMenuHitTarget> {
        if !self.publishes_interaction_frames {
            return None;
        }
        self.hit_targets
            .clone()
            .map(|registry| HoldContextMenuHitTarget::new(registry, HoldContextMenuHitTargetsKey::Emoji(key.to_owned())))
    }

    fn emoji_button(&self, emoji: &str, index: usize, scale: f32, opacity: f32) -> AnyElement {
        let glyph = div()
            .w(px(HoldContextMenuMetrics::EMOJI_SLOT_WIDTH))
            .h(px(HoldContextMenuMetrics::EMOJI_SLOT_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .opacity(opacity)
            .child(ReactionEmojiGlyph::new(emoji.to_owned(), HoldContextMenuMetrics::EMOJI_SIZE).is_playing(self.is_revealed))
            .scale_effect_anchored(scale, UnitPoint::BOTTOM);
        let on_select = self.on_select.clone();
        let picked = emoji.to_owned();
        div()
            .id(("hold-context-menu-emoji", index))
            .relative()
            .flex_none()
            .cursor_pointer()
            .child(glyph)
            .children(self.hit_target(emoji))
            .on_click(move |_, window, cx| {
                let _ = impact(ImpactStyle::Medium);
                on_select(picked.clone(), window, cx);
            })
            .into_any_element()
    }

    fn expand_button(&self, scale: f32, opacity: f32, cx: &App) -> AnyElement {
        let is_hovered = self.hovered_emoji.as_deref() == Some(Self::EXPAND_KEY);
        let ink = rise_ui::theme(cx).text.primary;
        let circle = div()
            .w(px(HoldContextMenuMetrics::EMOJI_EXPAND_BUTTON_SIZE))
            .h(px(HoldContextMenuMetrics::EMOJI_EXPAND_BUTTON_SIZE))
            .rounded_full()
            .bg(ink.opacity(if is_hovered { 0.22 } else { 0.10 }))
            .flex()
            .items_center()
            .justify_center()
            .children(IconUi::sized("chevron.down", px(13.0), ink.opacity(0.92), false));
        let slot = div()
            .w(px(HoldContextMenuMetrics::EMOJI_SLOT_WIDTH))
            .h(px(HoldContextMenuMetrics::EMOJI_SLOT_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .opacity(opacity)
            .child(circle)
            .scale_effect(scale);
        let on_expand = self.on_expand.clone();
        div()
            .id("hold-context-menu-emoji-expand")
            .relative()
            .flex_none()
            .cursor_pointer()
            .child(slot)
            .children(self.hit_target(Self::EXPAND_KEY))
            .on_click(move |_, window, cx| {
                let _ = impact(ImpactStyle::Light);
                if let Some(on_expand) = on_expand.as_ref() {
                    on_expand(window, cx);
                }
            })
            .into_any_element()
    }
}

impl RenderOnce for HoldContextMenuEmojiBar {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let reduce_motion = reduce_motion(cx);
        let slot_count = self.emojis.len() + usize::from(self.on_expand.is_some());
        let motion = window.use_keyed_state(self.id.clone(), cx, |_, _| HoldContextMenuEmojiBarMotion::default());
        let frames: Vec<(f32, f32)> = motion.update(cx, |motion, _| {
            motion.slots.resize_with(slot_count, EmojiSlotMotion::new);
            motion
                .slots
                .iter_mut()
                .enumerate()
                .map(|(index, slot)| {
                    let (hovered, hover_scale) = match self.emojis.get(index) {
                        Some(emoji) => (
                            self.hovered_emoji.as_deref() == Some(emoji.as_str()),
                            HoldContextMenuMetrics::EMOJI_HOVER_SCALE,
                        ),
                        None => (self.hovered_emoji.as_deref() == Some(Self::EXPAND_KEY), EXPAND_HOVER_SCALE),
                    };
                    slot.sync(hovered, hover_scale, index, now, reduce_motion);
                    (slot.scale.frame_value(now, window), slot.opacity.frame_value(now, window))
                })
                .collect()
        });

        let mut row = div()
            .flex()
            .flex_row()
            .items_center()
            .px(px(HoldContextMenuMetrics::EMOJI_BAR_HORIZONTAL_PADDING))
            .py(px(HoldContextMenuMetrics::EMOJI_BAR_VERTICAL_PADDING))
            .opacity(if self.is_revealed { 1.0 } else { 0.0 });
        for (index, emoji) in self.emojis.iter().enumerate() {
            let (scale, opacity) = frames.get(index).copied().unwrap_or((1.0, 1.0));
            row = row.child(self.emoji_button(emoji, index, scale, opacity));
        }
        if self.on_expand.is_some() {
            let (scale, opacity) = frames.get(self.emojis.len()).copied().unwrap_or((1.0, 1.0));
            row = row.child(self.expand_button(scale, opacity, cx));
        }

        let revealed_opacity = if self.is_revealed { 1.0 } else { 0.0 };
        let tail = div()
            .absolute()
            .bottom_0()
            .map(|tail| match self.alignment {
                HoldContextMenuAlignment::Trailing => tail.right_0(),
                HoldContextMenuAlignment::Leading => tail.left_0(),
            })
            .opacity(revealed_opacity)
            .child(HoldContextMenuTailDots::new(self.alignment));

        let bar = div()
            .id(self.id.clone())
            .relative()
            .flex_none()
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .opacity(revealed_opacity)
                    .child(HoldContextMenuShadow::new(f32::INFINITY).radius(18.0).opacity(0.24).offset_y(8.0)),
            )
            .child(
                HoldContextMenuGlass::new((self.id.clone(), "glass"))
                    .tint(black().opacity(0.12))
                    .corner_radius(f32::INFINITY)
                    .is_revealed(self.is_revealed)
                    .opaque(true),
            )
            .child(row)
            .child(
                centred_stroke(
                    white().opacity(HoldContextMenuMetrics::PANEL_STROKE_OPACITY as f32),
                    1.0,
                    Shape::Capsule,
                )
                .opacity(revealed_opacity),
            )
            .child(tail);

        div()
            .w_full()
            .flex()
            .map(|frame| match self.alignment {
                HoldContextMenuAlignment::Leading => frame.justify_start(),
                HoldContextMenuAlignment::Trailing => frame.justify_end(),
            })
            .child(bar)
    }
}

#[derive(IntoElement)]
pub struct HoldContextMenuTailDots {
    pub alignment: HoldContextMenuAlignment,
}

impl HoldContextMenuTailDots {
    pub fn new(alignment: HoldContextMenuAlignment) -> Self {
        Self { alignment }
    }

    fn dot_fill() -> Hsla {
        Rgba { r: 0.15, g: 0.15, b: 0.18, a: 0.95 }.into()
    }

    fn dot(side: f32, stroke_opacity: f32, line_width: f32) -> gpui::Div {
        div()
            .relative()
            .flex_none()
            .w(px(side))
            .h(px(side))
            .rounded_full()
            .bg(Self::dot_fill())
            .child(centred_stroke(white().opacity(stroke_opacity), line_width, Shape::Circle))
    }
}

impl RenderOnce for HoldContextMenuTailDots {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let trailing = self.alignment == HoldContextMenuAlignment::Trailing;
        let small = Self::dot(5.0, 0.08, 0.6);
        let small = if trailing { small.mr(px(8.0)) } else { small.ml(px(8.0)) };
        let stack = div()
            .flex()
            .flex_col()
            .gap(px(2.0))
            .map(|stack| if trailing { stack.items_end() } else { stack.items_start() })
            .relative()
            .top(px(16.0))
            .child(Self::dot(12.0, 0.10, 0.8))
            .child(small);
        if trailing { stack.mr(px(26.0)) } else { stack.ml(px(26.0)) }
    }
}

/// expanded reactions for a chat that allows only some
/// same panel chrome and grid as the full picker with just the allowed list
#[derive(IntoElement)]
pub struct HoldContextMenuAllowedReactionsPanel {
    id: ElementId,
    pub emojis: Vec<String>,
    pub on_pick: HoldContextMenuEmojiPick,
}

impl HoldContextMenuAllowedReactionsPanel {
    const COLUMNS: usize = 8;

    pub fn new(id: impl Into<ElementId>, emojis: Vec<String>, on_pick: HoldContextMenuEmojiPick) -> Self {
        Self { id: id.into(), emojis, on_pick }
    }
}

impl RenderOnce for HoldContextMenuAllowedReactionsPanel {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let radius = HoldContextMenuMetrics::PICKER_CORNER_RADIUS;
        let mut grid = div().flex().flex_col().gap(px(6.0)).px(px(8.0)).py(px(10.0));
        for (row_index, row) in self.emojis.chunks(Self::COLUMNS).enumerate() {
            let mut cells = div().flex().flex_row().w_full();
            for (column, emoji) in row.iter().enumerate() {
                let index = row_index * Self::COLUMNS + column;
                let on_pick = self.on_pick.clone();
                let picked = emoji.clone();
                cells = cells.child(
                    div()
                        .id(("hold-context-menu-allowed-reaction", index))
                        .flex_1()
                        .min_h(px(40.0))
                        .flex()
                        .items_center()
                        .justify_center()
                        .cursor_pointer()
                        .child(ReactionEmojiGlyph::new(emoji.clone(), 30.0))
                        .on_click(move |_, window, cx| {
                            let _ = impact(ImpactStyle::Medium);
                            on_pick(picked.clone(), window, cx);
                        }),
                );
            }
            for _ in row.len()..Self::COLUMNS {
                cells = cells.child(div().flex_1());
            }
            grid = grid.child(cells);
        }

        div()
            .id(self.id.clone())
            .relative()
            .size_full()
            .rounded(px(radius))
            .shadow(vec![gpui::BoxShadow {
                color: black().opacity(0.28),
                offset: gpui::point(px(0.0), px(12.0)),
                blur_radius: px(22.0),
                spread_radius: px(0.0),
                inset: false,
            }])
            .child(HoldContextMenuGlass::new((self.id.clone(), "glass")).corner_radius(radius).opaque(true))
            .child(
                div()
                    .id((self.id.clone(), "scroll"))
                    .relative()
                    .size_full()
                    .rounded(px(radius))
                    .overflow_y_scroll()
                    .child(grid),
            )
            .child(centred_stroke(
                white().opacity(HoldContextMenuMetrics::PANEL_STROKE_OPACITY as f32),
                1.0,
                Shape::continuous(radius),
            ))
    }
}
