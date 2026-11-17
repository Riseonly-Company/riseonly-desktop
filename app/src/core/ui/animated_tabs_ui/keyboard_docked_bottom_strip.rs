use gpui::{AnyElement, App, Window};
use rise_ui::swiftui::prelude::*;

use crate::app::router::main_tab_chrome::main_tab_bar_underlap;

pub enum KeyboardDockedStripMetrics {}

impl KeyboardDockedStripMetrics {
    pub const GAP: f32 = 8.0;
}

pub trait KeyboardDockedBottomStripModifiers: IntoElement + Sized {
    /// pins strip to the bottom, small gap above the home indicator while the keyboard is down
    /// and right above the keyboard when its up
    /// scroll views inside get the strip height as bottom margin
    fn keyboard_docked_bottom_strip(
        self,
        is_visible: bool,
        strip_height: f32,
        stands_on_tab_bar: bool,
        strip: impl IntoElement,
    ) -> KeyboardDockedBottomStrip {
        KeyboardDockedBottomStrip {
            content: self.into_any_element(),
            is_visible,
            strip_height,
            stands_on_tab_bar,
            strip: strip.into_any_element(),
        }
    }
}

impl<E: IntoElement> KeyboardDockedBottomStripModifiers for E {}

#[derive(IntoElement)]
pub struct KeyboardDockedBottomStrip {
    content: AnyElement,
    is_visible: bool,
    strip_height: f32,
    stands_on_tab_bar: bool,
    strip: AnyElement,
}

impl KeyboardDockedBottomStrip {
    /// what the strip stands on with the keyboard down, from the window bottom edge
    fn resting_base(&self, tab_bar_underlap: f32) -> f32 {
        let underlap = if self.stands_on_tab_bar { tab_bar_underlap } else { 0.0 };
        f32::max(underlap, Self::window_safe_bottom())
    }

    pub fn reserved_height(&self, tab_bar_underlap: f32) -> f32 {
        let underlap = if self.stands_on_tab_bar { tab_bar_underlap } else { 0.0 };
        self.strip_height + KeyboardDockedStripMetrics::GAP + f32::max(0.0, self.resting_base(tab_bar_underlap) - underlap)
    }

    /// why: desktop windows have no software keyboard so the strip always sits where it does with the
    /// keyboard down
    fn lift(&self, tab_bar_underlap: f32) -> f32 {
        self.resting_base(tab_bar_underlap) + KeyboardDockedStripMetrics::GAP
    }

    /// why: a desktop window has no home indicator or other bottom safe area
    fn window_safe_bottom() -> f32 {
        0.0
    }
}

impl RenderOnce for KeyboardDockedBottomStrip {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tab_bar_underlap = main_tab_bar_underlap(cx);
        let lift = self.lift(tab_bar_underlap);
        let margin = if self.is_visible { self.reserved_height(tab_bar_underlap) } else { 0.0 };
        let root = div().relative().size_full().child(self.content.content_margins_bottom(margin));
        if !self.is_visible {
            return root;
        }
        root.child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom(px(lift))
                .flex()
                .flex_col()
                .items_center()
                .child(div().w_full().block_mouse_except_scroll().child(self.strip)),
        )
    }
}
