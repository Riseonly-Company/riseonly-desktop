use gpui::{App, ElementId, Hsla, IntoElement, ParentElement, RenderOnce, Styled, Window, black, div};
use rise_core::foundation::foundation_date::RiseClock;
use rise_theme::{RiseThemeAppearance, ThemeStore, alpha};

use crate::hold_context_menu_ui::hold_context_menu_effect_animator::{
    HoldContextMenuEffectAnimator, HoldContextMenuEffectCompletion,
};
use crate::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use crate::swiftui::environment::reduce_motion;
use crate::swiftui::material::UIBlurEffectStyle;

/// dim and blur behind an open hold menu, takes no clicks
/// blur is the regular material over the theme background since gpui has no backdrop filter
#[derive(IntoElement)]
pub struct HoldContextMenuBackdropView {
    id: ElementId,
    pub is_revealed: bool,
    pub solid_color: Option<Hsla>,
    pub on_hidden: Option<HoldContextMenuEffectCompletion>,
}

impl HoldContextMenuBackdropView {
    /// id keys the effect animator, which lives as long as the backdrop is drawn
    pub fn new(id: impl Into<ElementId>, is_revealed: bool) -> Self {
        Self { id: id.into(), is_revealed, solid_color: None, on_hidden: None }
    }

    pub fn solid_color(mut self, solid_color: Option<Hsla>) -> Self {
        self.solid_color = solid_color;
        self
    }

    pub fn on_hidden(mut self, on_hidden: Option<HoldContextMenuEffectCompletion>) -> Self {
        self.on_hidden = on_hidden;
        self
    }
}

impl RenderOnce for HoldContextMenuBackdropView {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let reduce_motion = reduce_motion(cx);
        let revealed = self.is_revealed;
        let on_hidden = self.on_hidden;
        let animator = window.use_keyed_state(self.id.clone(), cx, |_, _| HoldContextMenuEffectAnimator::new());
        let fraction = animator.update(cx, |animator, cx| {
            let duration = if revealed {
                HoldContextMenuMetrics::BACKDROP_FADE_IN_DURATION
            } else {
                HoldContextMenuMetrics::BACKDROP_FADE_OUT_DURATION
            };
            let completion = if revealed { None } else { on_hidden };
            animator.set_revealed(revealed, true, duration, completion, now, reduce_motion);
            animator.revealed_fraction(now, window, cx)
        });

        let dim = self.solid_color.unwrap_or_else(|| black().opacity(0.55));
        let mut layer = div().absolute().inset_0();
        if self.solid_color.is_none() {
            let store = ThemeStore::shared(cx);
            let (appearance, background) = {
                let store = store.read(cx);
                (store.resolved_appearance(), store.current_theme().bg._100)
            };
            let style = match appearance {
                RiseThemeAppearance::Light => UIBlurEffectStyle::SystemMaterialLight,
                RiseThemeAppearance::Dark => UIBlurEffectStyle::SystemMaterialDark,
            };
            layer = layer.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(alpha(background, style.painted_opacity() * fraction)),
            );
        }
        layer.child(div().absolute().inset_0().bg(alpha(dim, dim.a * fraction)))
    }
}
