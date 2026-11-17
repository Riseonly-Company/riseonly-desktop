use gpui::{
    AnyElement, App, BACKDROP_BLUR_SUPPORTED, Hsla, IntoElement, ParentElement, RenderOnce, Styled, Window, black, div, px,
};

use crate::swiftui::linear_gradient::{GradientStop, LinearGradient};
use crate::swiftui::shapes::pass_through;
use crate::swiftui::stacks::{Alignment, VerticalAlignment};
use crate::swiftui::transition::UnitPoint;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollChromeShadowStyleAnchor {
    FromTop,
    FromBottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScrollChromeShadowStyleColorRole {
    Fixed,
    Surface,
    Appearance,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ScrollChromeShadowStyle {
    pub anchor: ScrollChromeShadowStyleAnchor,
    pub extent: f32,
    pub overflow_below_header: f32,
    pub peak_opacity: f64,
    pub color: Hsla,
    pub color_role: ScrollChromeShadowStyleColorRole,
    pub mid_location: f32,
    pub mid_opacity_factor: f64,
    pub top_plateau_end: f32,
    pub top_plateau_opacity_factor: f64,
    pub opaque_through_header: bool,
    pub blurs_backdrop: bool,
}

impl Default for ScrollChromeShadowStyle {
    fn default() -> Self {
        Self {
            anchor: ScrollChromeShadowStyleAnchor::FromTop,
            extent: 128.0,
            overflow_below_header: 52.0,
            peak_opacity: 0.82,
            color: black(),
            color_role: ScrollChromeShadowStyleColorRole::Fixed,
            mid_location: 0.58,
            mid_opacity_factor: 0.68,
            top_plateau_end: 0.14,
            top_plateau_opacity_factor: 0.96,
            opaque_through_header: false,
            blurs_backdrop: false,
        }
    }
}

impl ScrollChromeShadowStyle {
    pub fn list_shell_header() -> Self {
        Self {
            anchor: ScrollChromeShadowStyleAnchor::FromTop,
            extent: 25.0,
            overflow_below_header: 30.0,
            peak_opacity: 0.68,
            color: black(),
            color_role: ScrollChromeShadowStyleColorRole::Surface,
            mid_location: 0.58,
            mid_opacity_factor: 0.7,
            top_plateau_end: 0.16,
            top_plateau_opacity_factor: 0.97,
            opaque_through_header: false,
            blurs_backdrop: false,
        }
    }

    /// for settings screens and a title bar with a search field or tab strip below it
    /// list_shell_header thins out toward the bottom so rows read through
    /// this one stays solid behind the whole header
    pub fn list_shell_tall_header() -> Self {
        Self {
            anchor: ScrollChromeShadowStyleAnchor::FromTop,
            extent: 25.0,
            overflow_below_header: 14.0,
            peak_opacity: 1.0,
            color: black(),
            color_role: ScrollChromeShadowStyleColorRole::Surface,
            mid_location: 0.9,
            mid_opacity_factor: 0.92,
            top_plateau_end: 0.22,
            top_plateau_opacity_factor: 1.0,
            opaque_through_header: true,
            blurs_backdrop: false,
        }
    }

    /// top scrim over a blur of the rows beneath, header of every scrolling page
    /// over a real blur it thins to the blurred header strength so rows show through as blur
    pub fn over_backdrop_blur(self) -> Self {
        if self.anchor != ScrollChromeShadowStyleAnchor::FromTop {
            return self;
        }
        let mut style = self;
        style.blurs_backdrop = true;
        if BACKDROP_BLUR_SUPPORTED {
            style.peak_opacity = style.peak_opacity.min(Self::list_shell_blurred_header().peak_opacity);
            style.opaque_through_header = false;
        }
        style
    }

    /// the chat list
    /// rows stay visible behind the title bar and folder tabs and leave under a shadow that keeps
    /// the buttons apart
    pub fn list_shell_blurred_header() -> Self {
        Self {
            anchor: ScrollChromeShadowStyleAnchor::FromTop,
            extent: 25.0,
            overflow_below_header: 22.0,
            peak_opacity: 0.4,
            color: black(),
            color_role: ScrollChromeShadowStyleColorRole::Appearance,
            mid_location: 0.55,
            mid_opacity_factor: 0.5,
            top_plateau_end: 0.16,
            top_plateau_opacity_factor: 0.96,
            opaque_through_header: false,
            blurs_backdrop: true,
        }
    }

    /// fade under a floating bottom button so content scrolls out from under it
    /// surface because the button sits on the page background and not over media
    pub fn floating_bottom_action() -> Self {
        Self {
            anchor: ScrollChromeShadowStyleAnchor::FromBottom,
            extent: 112.0,
            overflow_below_header: 0.0,
            peak_opacity: 0.7,
            color: black(),
            color_role: ScrollChromeShadowStyleColorRole::Surface,
            mid_location: 0.5,
            mid_opacity_factor: 0.55,
            top_plateau_end: 0.16,
            top_plateau_opacity_factor: 0.98,
            opaque_through_header: false,
            blurs_backdrop: false,
        }
    }

    pub fn chat_composer_scroll_fade() -> Self {
        Self {
            anchor: ScrollChromeShadowStyleAnchor::FromBottom,
            extent: 112.0,
            overflow_below_header: 0.0,
            peak_opacity: 0.58,
            color: black(),
            color_role: ScrollChromeShadowStyleColorRole::Fixed,
            mid_location: 0.48,
            mid_opacity_factor: 0.42,
            top_plateau_end: 0.14,
            top_plateau_opacity_factor: 0.88,
            opaque_through_header: false,
            blurs_backdrop: false,
        }
    }
}

#[derive(IntoElement)]
pub struct ScrollChromeShadowUi {
    style: ScrollChromeShadowStyle,
    solid_fraction: Option<f32>,
}

impl ScrollChromeShadowUi {
    pub fn new(style: ScrollChromeShadowStyle) -> Self {
        Self {
            style,
            solid_fraction: None,
        }
    }

    pub fn solid_fraction(mut self, solid_fraction: Option<f32>) -> Self {
        self.solid_fraction = solid_fraction;
        self
    }

    fn resolved_color(&self, cx: &App) -> Hsla {
        match self.style.color_role {
            ScrollChromeShadowStyleColorRole::Fixed => self.style.color,
            ScrollChromeShadowStyleColorRole::Surface => crate::theme(cx).bg._100,
            ScrollChromeShadowStyleColorRole::Appearance => {
                if crate::theme(cx).appearance == rise_theme::Appearance::Dark { black() } else { gpui::white() }
            }
        }
    }

    pub fn stops(&self, cx: &App) -> Vec<GradientStop> {
        let style = &self.style;
        let peak = style.peak_opacity.clamp(0.0, 1.0) as f32;
        let mid_f = style.mid_opacity_factor.clamp(0.0, 1.0) as f32;
        let mid_loc = style.mid_location.clamp(0.02, 0.98);
        let plateau_end = style.top_plateau_end.clamp(0.02, 0.22);
        let plateau_f = style.top_plateau_opacity_factor.clamp(0.0, 1.0) as f32;
        let plateau_loc = plateau_end.min(mid_loc - 0.04);
        let color = self.resolved_color(cx);

        if let Some(solid_fraction) = self.solid_fraction
            && style.anchor == ScrollChromeShadowStyleAnchor::FromTop
        {
            let solid_end = solid_fraction.clamp(0.0, 0.98);
            return vec![
                GradientStop::new(color.opacity(peak), 0.0),
                GradientStop::new(color.opacity(peak), solid_end),
                GradientStop::new(color.opacity(peak * 0.5), solid_end + (1.0 - solid_end) * 0.45),
                GradientStop::new(color.opacity(0.0), 1.0),
            ];
        }

        let c_peak = color.opacity(peak);
        let c_plateau = color.opacity(peak * plateau_f);
        let c_mid = color.opacity(peak * mid_f);
        let c_clear = color.opacity(0.0);

        match style.anchor {
            ScrollChromeShadowStyleAnchor::FromTop => vec![
                GradientStop::new(c_peak, 0.0),
                GradientStop::new(c_plateau, plateau_loc),
                GradientStop::new(c_mid, mid_loc),
                GradientStop::new(c_clear, 1.0),
            ],
            ScrollChromeShadowStyleAnchor::FromBottom => vec![
                GradientStop::new(c_clear, 0.0),
                GradientStop::new(c_mid, 1.0 - mid_loc),
                GradientStop::new(c_plateau, 1.0 - plateau_loc),
                GradientStop::new(c_peak, 1.0),
            ],
        }
    }
}

impl RenderOnce for ScrollChromeShadowUi {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let stops = self.stops(cx);
        pass_through()
            .size_full()
            .child(LinearGradient::new(stops, UnitPoint::TOP, UnitPoint::BOTTOM))
    }
}

pub trait ScrollChromeShadowModifiers: IntoElement + Sized {
    fn scroll_chrome_shadow(
        self,
        style: Option<ScrollChromeShadowStyle>,
        height: f32,
        alignment: Alignment,
    ) -> AnyElement {
        let Some(style) = style.filter(|_| height > 0.0) else {
            return self.into_any_element();
        };
        let shadow = div()
            .absolute()
            .left_0()
            .right_0()
            .h(px(height))
            .child(ScrollChromeShadowUi::new(style));
        let shadow = match alignment.vertical() {
            VerticalAlignment::Bottom => shadow.bottom_0(),
            VerticalAlignment::Center => shadow.top_0().bottom_0().my_auto(),
            VerticalAlignment::Top | VerticalAlignment::FirstTextBaseline => shadow.top_0(),
        };
        div()
            .relative()
            .child(shadow)
            .child(self)
            .into_any_element()
    }
}

impl<E: IntoElement> ScrollChromeShadowModifiers for E {}
