use gpui::{
    App, BoxShadow, Div, ElementId, Hsla, IntoElement, ParentElement, Pixels, RenderOnce, Styled, Window, black, div,
    point, px, rgb,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_theme::{ThemeStore, alpha};

use crate::hold_context_menu_ui::hold_context_menu_effect_animator::HoldContextMenuEffectAnimator;
use crate::hold_context_menu_ui::hold_context_menu_metrics::HoldContextMenuMetrics;
use crate::hold_context_menu_ui::hold_context_menu_types::HoldSourceBubbleClip;
use crate::swiftui::environment::reduce_motion;
use crate::swiftui::environment_values::{ColorScheme, accessibility_reduce_transparency, color_scheme};
use crate::swiftui::material::UIBlurEffectStyle;

const THICK_MATERIAL_DARK: u32 = 0x1C1C1E;
const THICK_MATERIAL_LIGHT: u32 = 0xF2F2F7;

/// rounds the layer like the clip or like a rounded rect of corner_radius
/// gpui clamps every corner to half the shorter side when painting
fn contour(layer: Div, corner_radius: f32, bubble_clip: Option<HoldSourceBubbleClip>) -> Div {
    match bubble_clip {
        Some(clip) => {
            let corners = clip.uneven_shape();
            layer
                .rounded_tl(corners.top_left)
                .rounded_tr(corners.top_right)
                .rounded_br(corners.bottom_right)
                .rounded_bl(corners.bottom_left)
        }
        None => layer.rounded(px(corner_radius)),
    }
}

/// explicit contour so the shadow isnt derived from the moving message or glass subtree
/// shadow follows layout only, never flight position or opacity
#[derive(IntoElement)]
pub struct HoldContextMenuShadow {
    pub corner_radius: f32,
    pub radius: f32,
    pub opacity: f32,
    pub offset_y: f32,
    pub bubble_clip: Option<HoldSourceBubbleClip>,
}

impl Default for HoldContextMenuShadow {
    fn default() -> Self {
        Self {
            corner_radius: 18.0,
            radius: 22.0,
            opacity: 0.28,
            offset_y: 12.0,
            bubble_clip: None,
        }
    }
}

impl HoldContextMenuShadow {
    pub fn new(corner_radius: f32) -> Self {
        Self { corner_radius, ..Self::default() }
    }

    pub fn radius(mut self, radius: f32) -> Self {
        self.radius = radius;
        self
    }

    pub fn opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity;
        self
    }

    pub fn offset_y(mut self, offset_y: f32) -> Self {
        self.offset_y = offset_y;
        self
    }

    pub fn bubble_clip(mut self, bubble_clip: Option<HoldSourceBubbleClip>) -> Self {
        self.bubble_clip = bubble_clip;
        self
    }
}

impl RenderOnce for HoldContextMenuShadow {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let shadow = BoxShadow {
            color: black().opacity(self.opacity),
            offset: point(Pixels::ZERO, px(self.offset_y)),
            blur_radius: px(self.radius),
            spread_radius: Pixels::ZERO,
            inset: false,
        };
        contour(div().absolute().inset_0(), self.corner_radius, self.bubble_clip).shadow(vec![shadow])
    }
}

/// glass panel behind the action list, kept outside alpha groups
/// it animates on its own while text, borders and shadows fade separately
#[derive(IntoElement)]
pub struct HoldContextMenuGlass {
    id: ElementId,
    pub tint: Hsla,
    pub corner_radius: f32,
    pub is_revealed: bool,
    pub is_opaque: bool,
    scheme: Option<ColorScheme>,
}

impl HoldContextMenuGlass {
    /// id keys the effect animator, which lives as long as the glass is drawn
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            tint: black().opacity(0.12),
            corner_radius: 0.0,
            is_revealed: true,
            is_opaque: false,
            scheme: None,
        }
    }

    pub fn tint(mut self, tint: Hsla) -> Self {
        self.tint = tint;
        self
    }

    pub fn corner_radius(mut self, corner_radius: f32) -> Self {
        self.corner_radius = corner_radius;
        self
    }

    pub fn is_revealed(mut self, is_revealed: bool) -> Self {
        self.is_revealed = is_revealed;
        self
    }

    /// paints the material at full strength
    /// without a backdrop blur the translucent material leaves text under the panel readable
    pub fn opaque(mut self, is_opaque: bool) -> Self {
        self.is_opaque = is_opaque;
        self
    }

    /// pins the glass to this color scheme whatever the app theme is, for labels that dont follow the theme
    pub fn color_scheme(mut self, scheme: ColorScheme) -> Self {
        self.scheme = Some(scheme);
        self
    }
}

impl RenderOnce for HoldContextMenuGlass {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let now = RiseClock::monotonic();
        let reduce_motion = reduce_motion(cx);
        let revealed = self.is_revealed;
        let animator = window.use_keyed_state(self.id.clone(), cx, |_, _| HoldContextMenuEffectAnimator::new());
        let fraction = animator.update(cx, |animator, cx| {
            let animated = animator.target().is_some();
            let duration = if revealed {
                HoldContextMenuMetrics::OPEN_RESPONSE
            } else {
                HoldContextMenuMetrics::CLOSE_RESPONSE
            };
            animator.set_revealed(revealed, animated, duration, None, now, reduce_motion);
            animator.revealed_fraction(now, window, cx)
        });

        let store = ThemeStore::shared(cx);
        let (appearance, theme_background) = {
            let store = store.read(cx);
            (store.resolved_appearance(), store.current_theme().bg._100)
        };
        let scheme = self.scheme.unwrap_or_else(|| color_scheme(cx));
        let background = if scheme == ColorScheme::from_appearance(appearance) {
            theme_background
        } else {
            match scheme {
                ColorScheme::Light => Hsla::from(rgb(THICK_MATERIAL_LIGHT)),
                ColorScheme::Dark => Hsla::from(rgb(THICK_MATERIAL_DARK)),
            }
        };
        let effect = if self.is_opaque || accessibility_reduce_transparency(cx) {
            alpha(background, fraction)
        } else {
            let style = match scheme {
                ColorScheme::Light => UIBlurEffectStyle::SystemThickMaterialLight,
                ColorScheme::Dark => UIBlurEffectStyle::SystemThickMaterialDark,
            };
            alpha(background, style.painted_opacity() * fraction)
        };
        let tint = alpha(self.tint, self.tint.a * fraction);

        div()
            .absolute()
            .inset_0()
            .rounded(px(self.corner_radius))
            .overflow_hidden()
            .bg(effect)
            .child(div().absolute().inset_0().rounded(px(self.corner_radius)).bg(tint))
    }
}
