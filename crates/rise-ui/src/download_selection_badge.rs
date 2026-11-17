use gpui::{
    App, ElementId, FontWeight, Hsla, IntoElement, ParentElement, RenderOnce, Rgba, Styled, Window, div, hsla, px,
    white,
};
use rise_core::foundation::foundation_date::RiseClock;

use crate::IconUi;
use crate::swiftui::animated::Animated;
use crate::swiftui::motion::ease_out;

/// 26 pt circle with a 1.5 pt white ring, red fill with the pick order when selected
/// a checkmark when theres no order to show
#[derive(IntoElement)]
pub struct DownloadSelectionBadge {
    id: ElementId,
    is_selected: bool,
    number: Option<usize>,
}

impl DownloadSelectionBadge {
    pub const DIAMETER: f32 = 26.0;
    pub const TOUCH_OUTSET: f32 = 11.0;
    const RING_WIDTH: f32 = 1.5;
    const GLYPH_SIZE: f32 = 11.0;
    const ANIMATION_DURATION: f64 = 0.15;

    pub fn new(id: impl Into<ElementId>, is_selected: bool) -> Self {
        Self { id: id.into(), is_selected, number: None }
    }

    pub fn number(mut self, number: Option<usize>) -> Self {
        self.number = number;
        self
    }

    fn selected_fill() -> Hsla {
        Rgba { r: 1.0, g: 0.25, b: 0.25, a: 1.0 }.into()
    }

    fn mix(from: Hsla, to: Hsla, progress: f32) -> Hsla {
        let from: Rgba = from.into();
        let to: Rgba = to.into();
        let lerp = |a: f32, b: f32| a + (b - a) * progress;
        Rgba { r: lerp(from.r, to.r), g: lerp(from.g, to.g), b: lerp(from.b, to.b), a: lerp(from.a, to.a) }.into()
    }
}

impl RenderOnce for DownloadSelectionBadge {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let target = if self.is_selected { 1_f32 } else { 0_f32 };
        let state = window.use_keyed_state(self.id.clone(), cx, |_, _| Animated::new(target));
        let now = RiseClock::monotonic();
        let progress = state.update(cx, |animated, cx| {
            animated.set_in(target, ease_out(Self::ANIMATION_DURATION), now, cx);
            animated.frame_value(now, window)
        });
        let fill = Self::mix(hsla(0.0, 0.0, 0.0, 0.35), Self::selected_fill(), progress);
        let ring = hsla(0.0, 0.0, 1.0, 0.85 + 0.15 * progress);
        let glyph = if self.is_selected {
            match self.number {
                Some(number) => Some(
                    div()
                        .text_size(px(Self::GLYPH_SIZE))
                        .font_weight(FontWeight::BOLD)
                        .text_color(white())
                        .child(number.to_string())
                        .into_any_element(),
                ),
                None => IconUi::sized("checkmark", px(Self::GLYPH_SIZE), white(), false)
                    .map(IntoElement::into_any_element),
            }
        } else {
            None
        };
        div()
            .size(px(Self::DIAMETER))
            .flex_none()
            .rounded_full()
            .bg(fill)
            .border(px(Self::RING_WIDTH))
            .border_color(ring)
            .flex()
            .items_center()
            .justify_center()
            .children(glyph)
    }
}
