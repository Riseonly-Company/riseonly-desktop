use gpui::{App, ElementId, SharedString, Window};
use rise_core::foundation::foundation_date::RiseClock;
use rise_ui::IconUi;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::page_header_toolbar_button::PageHeaderMetrics;
use rise_ui::swiftui::animated::Animated;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;

const PENCIL_SIZE: f32 = 16.0;
const CHECKMARK_SIZE: f32 = 17.0;
const HIDDEN_GLYPH_SCALE: f32 = 0.82;

struct ChatsReorderToggleMotion {
    progress: Animated<f32>,
}

#[derive(IntoElement)]
pub struct ChatsReorderToggleButton {
    id: ElementId,
    is_reorder_mode: bool,
    on_toggle: ButtonAction,
}

impl ChatsReorderToggleButton {
    pub fn new(
        id: impl Into<ElementId>,
        is_reorder_mode: bool,
        on_toggle: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Self { id: id.into(), is_reorder_mode, on_toggle: std::rc::Rc::new(on_toggle) }
    }

    fn glyph_layer(symbol: &str, size: f32, color: gpui::Hsla, progress: f32) -> Div {
        let scale = HIDDEN_GLYPH_SCALE + (1.0 - HIDDEN_GLYPH_SCALE) * progress;
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .opacity(progress.clamp(0.0, 1.0))
            .children(IconUi::sized(symbol, px(size * scale), color, false))
    }
}

impl RenderOnce for ChatsReorderToggleButton {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = rise_ui::theme(cx).clone();
        let target = if self.is_reorder_mode { 1.0 } else { 0.0 };
        let now = RiseClock::monotonic();
        let state_id = SharedString::from(format!("{}-motion", self.id));
        let motion = window.use_keyed_state(state_id, cx, |_, _| ChatsReorderToggleMotion {
            progress: Animated::new(target),
        });
        motion.update(cx, |motion, cx| {
            if *motion.progress.target() != target {
                motion.progress.set_in(target, spring(0.28, 0.86), now, cx);
            }
        });
        let progress = motion.read(cx).progress.frame_value(now, window);
        let size = PageHeaderMetrics::TOOLBAR_BUTTON_SIZE;
        let on_toggle = self.on_toggle;

        LiquidGlassButton::new(self.id, LiquidGlassButtonShape::Circle)
            .size(size)
            .tint(self.is_reorder_mode.then_some(theme.primary._100))
            .on_press(move |window, cx| on_toggle(window, cx))
            .child(
                div()
                    .relative()
                    .w(px(size))
                    .h(px(size))
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .circle()
                            .bg(theme.primary._100)
                            .opacity(progress.clamp(0.0, 1.0)),
                    )
                    .child(Self::glyph_layer("pencil", PENCIL_SIZE, theme.text.primary, 1.0 - progress))
                    .child(Self::glyph_layer("checkmark", CHECKMARK_SIZE, theme.content_on(theme.primary._100), progress)),
            )
    }
}
