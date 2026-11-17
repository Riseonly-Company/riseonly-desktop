use gpui::{
    AnyElement, App, BoxShadow, Div, Hsla, IntoElement, ParentElement, RenderOnce, Styled, Window,
    div, hsla, point, px,
};
use rise_theme::ui_color_system::UIColor;
use rise_theme::{AppTheme, RiseThemeAppearance, ThemeStore, alpha};

use crate::swiftui::environment::{EnvKey, LayoutDirection, env, env_scope, layout_direction};
use crate::swiftui::environment_values::{
    ColorScheme, ColorSchemeKey, accessibility_reduce_transparency, color_scheme,
};
use crate::swiftui::material::{Material, UIBlurEffectStyle};
use crate::swiftui::modifiers::{ModifierExt, ViewModifier};
use crate::swiftui::shapes::{Shape, centred_stroke};

/// opt in for white controls drawn over photos or video
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum LiquidGlassContrast {
    #[default]
    Standard,
    Media,
}

struct LiquidGlassContrastKey;

impl EnvKey for LiquidGlassContrastKey {
    type Value = LiquidGlassContrast;

    fn default_value() -> Self::Value {
        LiquidGlassContrast::Standard
    }
}

struct NavGlassBlurIntensityKey;

impl EnvKey for NavGlassBlurIntensityKey {
    type Value = f32;

    fn default_value() -> Self::Value {
        LiquidChrome::DEFAULT_NAV_GLASS_BLUR_INTENSITY
    }
}

/// how many chrome surfaces enclose the subtree
/// a pill inside a chrome card is one step lighter or it would vanish into the card
struct ChromeSurfaceDepthKey;

impl EnvKey for ChromeSurfaceDepthKey {
    type Value = u32;

    fn default_value() -> Self::Value {
        0
    }
}

pub fn chrome_surface_depth(cx: &App) -> u32 {
    env::<ChromeSurfaceDepthKey>(cx)
}

pub fn liquid_glass_contrast(cx: &App) -> LiquidGlassContrast {
    env::<LiquidGlassContrastKey>(cx)
}

pub fn nav_glass_blur_intensity(cx: &App) -> f32 {
    env::<NavGlassBlurIntensityKey>(cx)
}

struct GlassFollowsSwitchKey;

impl EnvKey for GlassFollowsSwitchKey {
    type Value = bool;

    fn default_value() -> Self::Value {
        true
    }
}

pub fn glass_follows_switch(cx: &App) -> bool {
    env::<GlassFollowsSwitchKey>(cx)
}

/// outline a glass surface is clipped to, filled with and stroked along
#[derive(Clone, Copy, Debug, PartialEq)]
enum GlassOutline {
    Shape(Shape),
    Uneven {
        top_leading_radius: f32,
        bottom_leading_radius: f32,
        bottom_trailing_radius: f32,
        top_trailing_radius: f32,
    },
}

impl GlassOutline {
    fn round<E: Styled>(self, element: E, direction: LayoutDirection) -> E {
        self.outset_round(element, 0.0, direction)
    }

    fn outset_round<E: Styled>(self, element: E, distance: f32, direction: LayoutDirection) -> E {
        match self {
            Self::Shape(Shape::RoundedRectangle { corner_radius, style }) => {
                Shape::RoundedRectangle { corner_radius: corner_radius + distance, style }.round(element)
            }
            Self::Shape(shape) => shape.round(element),
            Self::Uneven { top_leading_radius, bottom_leading_radius, bottom_trailing_radius, top_trailing_radius } => {
                let (top_left, top_right, bottom_left, bottom_right) = match direction {
                    LayoutDirection::LeftToRight => {
                        (top_leading_radius, top_trailing_radius, bottom_leading_radius, bottom_trailing_radius)
                    }
                    LayoutDirection::RightToLeft => {
                        (top_trailing_radius, top_leading_radius, bottom_trailing_radius, bottom_leading_radius)
                    }
                };
                element
                    .rounded_tl(px(top_left + distance))
                    .rounded_tr(px(top_right + distance))
                    .rounded_bl(px(bottom_left + distance))
                    .rounded_br(px(bottom_right + distance))
            }
        }
    }

    /// stroke as an overlay centered on the outline
    fn stroke(self, color: Hsla, line_width: f32, direction: LayoutDirection) -> Div {
        match self {
            Self::Shape(shape) => centred_stroke(color, line_width, shape),
            Self::Uneven { .. } => {
                let half = line_width / 2.;
                self.outset_round(div().absolute().inset(px(-half)), half, direction)
                    .border(px(line_width))
                    .border_color(color)
            }
        }
    }
}

/// white at opacity, the color every glass stroke and tint here is made of
fn white(opacity: f64) -> Hsla {
    hsla(0., 0., 1., opacity as f32)
}

fn black(opacity: f64) -> Hsla {
    hsla(0., 0., 0., opacity as f32)
}

/// color of a white to white gradient stroke
/// gpui strokes with one color only so the mean keeps the hairline as bright on average
pub fn gradient_stroke_color(start_opacity: f64, end_opacity: f64) -> Hsla {
    white((start_opacity + end_opacity) / 2.)
}

fn clamped_intensity(value: f32) -> f32 {
    value.clamp(0., 1.)
}

pub enum LiquidGlassThemeVeil {}

impl LiquidGlassThemeVeil {
    pub const OPACITY: f64 = 0.24;
    pub const REDUCED_TRANSPARENCY_OPACITY: f64 = 0.85;
}

/// dark backdrop under white glyphs over media
#[derive(Clone, Copy)]
struct LiquidGlassContrastBackdrop {
    shape: GlassOutline,
    #[allow(dead_code)]
    themed: bool,
}

impl LiquidGlassContrastBackdrop {
    /// backdrop layer or nothing outside a media subtree
    /// clipped is true when the caller clips the layer to its outline
    fn layer(self, clipped: bool, cx: &App) -> Option<Div> {
        if liquid_glass_contrast(cx) != LiquidGlassContrast::Media {
            return None;
        }
        let reduce_transparency = accessibility_reduce_transparency(cx);
        let fill = black(if reduce_transparency { 1. } else { 0.45 });
        let mut layer = div().absolute().inset_0().bg(fill);
        if !clipped {
            layer = layer.shadow(vec![BoxShadow {
                color: black(0.28),
                offset: point(px(0.), px(2.)),
                blur_radius: px(6.),
                spread_radius: px(0.),
                inset: false,
            }]);
        }
        Some(self.shape.round(layer, layout_direction(cx)))
    }
}

impl ViewModifier for LiquidGlassContrastBackdrop {
    fn body(self, content: AnyElement, _window: &mut Window, cx: &mut App) -> AnyElement {
        match self.layer(false, cx) {
            Some(layer) => div().relative().child(layer).child(content).into_any_element(),
            None => content,
        }
    }
}

struct LiquidGlassMediaScheme {
    active: bool,
}

impl ViewModifier for LiquidGlassMediaScheme {
    fn body(self, content: AnyElement, _window: &mut Window, _cx: &mut App) -> AnyElement {
        if self.active {
            env_scope::<ColorSchemeKey>(Some(ColorScheme::Dark)).child(content).into_any_element()
        } else {
            content
        }
    }
}

#[derive(IntoElement)]
struct NavGlassBlurBackdrop {
    intensity: f32,
    depth: u32,
    clip: GlassOutline,
}

impl RenderOnce for NavGlassBlurBackdrop {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let layer = div().absolute().inset_0();
        if clamped_intensity(self.intensity) <= 0. {
            return layer;
        }
        let store = ThemeStore::shared(cx);
        let (resolved_appearance, themed_fill) = {
            let store = store.read(cx);
            (store.resolved_appearance(), LiquidChrome::surface_fill(store.current_theme(), self.depth))
        };
        let forced_dark = color_scheme(cx) == ColorScheme::Dark && resolved_appearance == RiseThemeAppearance::Light;
        let fill = if forced_dark {
            UIColor::secondary_system_background().resolved_color(RiseThemeAppearance::Dark)
        } else {
            themed_fill
        };
        self.clip.round(layer.bg(fill), layout_direction(cx))
    }
}

pub enum LiquidChrome {}

impl LiquidChrome {
    pub const DEFAULT_NAV_GLASS_BLUR_INTENSITY: f32 = 1.;

    pub fn plain_fill_opacity(style: UIBlurEffectStyle) -> f64 {
        match style {
            UIBlurEffectStyle::SystemUltraThinMaterialLight | UIBlurEffectStyle::SystemUltraThinMaterialDark => 0.72,
            UIBlurEffectStyle::SystemThinMaterialLight | UIBlurEffectStyle::SystemThinMaterialDark => 0.8,
            UIBlurEffectStyle::SystemMaterialLight | UIBlurEffectStyle::SystemMaterialDark => 0.86,
            UIBlurEffectStyle::SystemThickMaterialLight | UIBlurEffectStyle::SystemThickMaterialDark => 0.92,
            UIBlurEffectStyle::SystemChromeMaterialLight | UIBlurEffectStyle::SystemChromeMaterialDark => 0.96,
        }
    }
    pub const HEADER_GLASS_BLUR_INTENSITY: f32 = 0.3;

    pub const NESTED_SURFACE_LIFT: f32 = 0.06;
    pub const SELECTED_SEGMENT_LIFT: f32 = 0.08;

    /// the one fill of every button, pill tab bar and chrome card
    /// folder tabs pill a bit greyer than the page
    /// a step of the theme scale so every theme colors it
    pub fn surface_fill(theme: &AppTheme, depth: u32) -> Hsla {
        let base = theme.bg._300;
        if depth == 0 { base } else { base.blend(alpha(theme.text.primary, Self::NESTED_SURFACE_LIFT)) }
    }

    pub fn selected_segment_fill(theme: &AppTheme, depth: u32) -> Hsla {
        Self::surface_fill(theme, depth).blend(alpha(theme.text.primary, Self::SELECTED_SEGMENT_LIFT))
    }

    pub fn bar_material() -> Material {
        Material::Thin
    }

    pub fn nav_glass_ui_blur_effect_style(intensity: f32, appearance: RiseThemeAppearance) -> Option<UIBlurEffectStyle> {
        let t = clamped_intensity(intensity);
        if t <= 0. {
            return None;
        }
        let style = match appearance {
            RiseThemeAppearance::Light => match t {
                t if t < 0.2 => UIBlurEffectStyle::SystemUltraThinMaterialLight,
                t if t < 0.4 => UIBlurEffectStyle::SystemThinMaterialLight,
                t if t < 0.6 => UIBlurEffectStyle::SystemMaterialLight,
                t if t < 0.8 => UIBlurEffectStyle::SystemThickMaterialLight,
                _ => UIBlurEffectStyle::SystemChromeMaterialLight,
            },
            RiseThemeAppearance::Dark => match t {
                t if t < 0.2 => UIBlurEffectStyle::SystemUltraThinMaterialDark,
                t if t < 0.4 => UIBlurEffectStyle::SystemThinMaterialDark,
                t if t < 0.6 => UIBlurEffectStyle::SystemMaterialDark,
                t if t < 0.8 => UIBlurEffectStyle::SystemThickMaterialDark,
                _ => UIBlurEffectStyle::SystemChromeMaterialDark,
            },
        };
        Some(style)
    }

    pub fn nav_glass_material(intensity: f32) -> Material {
        let t = clamped_intensity(intensity);
        if t <= 0. {
            return Material::UltraThin;
        }
        match t {
            t if t < 0.2 => Material::UltraThin,
            t if t < 0.4 => Material::Thin,
            t if t < 0.6 => Material::Regular,
            t if t < 0.8 => Material::Thick,
            _ => Material::UltraThick,
        }
    }

    pub fn nav_glass_border_width() -> f32 {
        0.65
    }

    pub fn material_darken_opacity() -> f64 {
        0.26
    }

    pub fn bar_chrome_overlay(corner_radius: f32) -> Div {
        if corner_radius > 0. {
            centred_stroke(gradient_stroke_color(0.22, 0.06), 0.65, Shape::continuous(corner_radius))
        } else {
            centred_stroke(white(0.08), 0.5, Shape::Rectangle)
        }
    }
}

/// shared glass for every modifier below
/// content over the contrast backdrop over the painted blur
/// clipped to the outline with an optional hairline
fn glass_surface(
    content: AnyElement,
    outline: GlassOutline,
    intensity: f32,
    stroke: Option<Hsla>,
    cx: &App,
) -> AnyElement {
    let direction = layout_direction(cx);
    let depth = chrome_surface_depth(cx);
    let backdrop = LiquidGlassContrastBackdrop { shape: outline, themed: true }.layer(true, cx);
    let clipped = outline
        .round(div().relative().overflow_hidden(), direction)
        .child(NavGlassBlurBackdrop { intensity, depth, clip: outline })
        .children(backdrop)
        .child(env_scope::<ChromeSurfaceDepthKey>(depth + 1).child(content));
    let mut surface = div().relative().child(clipped);
    if let Some(color) = stroke {
        surface = surface.child(outline.stroke(color, LiquidChrome::nav_glass_border_width(), direction));
    }
    surface.into_any_element()
}

fn resolved_intensity(blur_intensity: Option<f32>, cx: &App) -> f32 {
    clamped_intensity(blur_intensity.unwrap_or_else(|| nav_glass_blur_intensity(cx)))
}

/// header buttons and pills, environment intensity is capped at the thin header blur
fn resolved_header_intensity(blur_intensity: Option<f32>, cx: &App) -> f32 {
    clamped_intensity(
        blur_intensity
            .unwrap_or_else(|| nav_glass_blur_intensity(cx).min(LiquidChrome::HEADER_GLASS_BLUR_INTENSITY)),
    )
}

fn pill_padding(content: AnyElement, horizontal_padding: f32) -> AnyElement {
    div().px(px(horizontal_padding)).py(px(5.)).child(content).into_any_element()
}

struct LiquidGlassNavCircleModifier {
    blur_intensity: Option<f32>,
}

impl ViewModifier for LiquidGlassNavCircleModifier {
    fn body(self, content: AnyElement, _window: &mut Window, cx: &mut App) -> AnyElement {
        let t = resolved_header_intensity(self.blur_intensity, cx);
        let framed = div().size(px(40.)).flex().items_center().justify_center().child(content);
        glass_surface(
            framed.into_any_element(),
            GlassOutline::Shape(Shape::Circle),
            t,
            Some(gradient_stroke_color(0.2, 0.06)),
            cx,
        )
    }
}

struct PageHeaderGlassPillModifier {
    horizontal_padding: f32,
}

impl ViewModifier for PageHeaderGlassPillModifier {
    fn body(self, content: AnyElement, window: &mut Window, cx: &mut App) -> AnyElement {
        LiquidGlassNavPillModifier { horizontal_padding: self.horizontal_padding, blur_intensity: None }.body(content, window, cx)
    }
}

struct LiquidGlassNavPillModifier {
    horizontal_padding: f32,
    blur_intensity: Option<f32>,
}

impl ViewModifier for LiquidGlassNavPillModifier {
    fn body(self, content: AnyElement, _window: &mut Window, cx: &mut App) -> AnyElement {
        let t = resolved_header_intensity(self.blur_intensity, cx);
        glass_surface(
            pill_padding(content, self.horizontal_padding),
            GlassOutline::Shape(Shape::Capsule),
            t,
            Some(gradient_stroke_color(0.18, 0.06)),
            cx,
        )
    }
}

struct LiquidChromeCapsuleModifier {
    stroke_opacity: f64,
    shows_stroke: bool,
    blur_intensity: Option<f32>,
}

impl ViewModifier for LiquidChromeCapsuleModifier {
    fn body(self, content: AnyElement, _window: &mut Window, cx: &mut App) -> AnyElement {
        let t = resolved_intensity(self.blur_intensity, cx);
        let stroke = self.shows_stroke.then(|| gradient_stroke_color(0.14, self.stroke_opacity));
        glass_surface(content, GlassOutline::Shape(Shape::Capsule), t, stroke, cx)
    }
}

struct LiquidChromeRoundedRectangleModifier {
    corner_radius: f32,
    stroke_opacity: f64,
    blur_intensity: Option<f32>,
}

impl ViewModifier for LiquidChromeRoundedRectangleModifier {
    fn body(self, content: AnyElement, _window: &mut Window, cx: &mut App) -> AnyElement {
        let t = resolved_intensity(self.blur_intensity, cx);
        glass_surface(
            content,
            GlassOutline::Shape(Shape::continuous(self.corner_radius)),
            t,
            Some(gradient_stroke_color(0.14, self.stroke_opacity)),
            cx,
        )
    }
}

struct LiquidChromeUnevenRoundedRectangleModifier {
    top_leading_radius: f32,
    bottom_leading_radius: f32,
    bottom_trailing_radius: f32,
    top_trailing_radius: f32,
    stroke_opacity: f64,
    shows_stroke: bool,
    blur_intensity: Option<f32>,
}

impl ViewModifier for LiquidChromeUnevenRoundedRectangleModifier {
    fn body(self, content: AnyElement, _window: &mut Window, cx: &mut App) -> AnyElement {
        let t = resolved_intensity(self.blur_intensity, cx);
        let outline = GlassOutline::Uneven {
            top_leading_radius: self.top_leading_radius,
            bottom_leading_radius: self.bottom_leading_radius,
            bottom_trailing_radius: self.bottom_trailing_radius,
            top_trailing_radius: self.top_trailing_radius,
        };
        let stroke = self.shows_stroke.then(|| gradient_stroke_color(0.14, self.stroke_opacity));
        glass_surface(content, outline, t, stroke, cx)
    }
}

struct LiquidChromeCircleModifier {
    blur_intensity: Option<f32>,
}

impl ViewModifier for LiquidChromeCircleModifier {
    fn body(self, content: AnyElement, _window: &mut Window, cx: &mut App) -> AnyElement {
        let t = resolved_intensity(self.blur_intensity, cx);
        glass_surface(content, GlassOutline::Shape(Shape::Circle), t, Some(gradient_stroke_color(0.14, 0.06)), cx)
    }
}

/// liquid chrome modifiers, full versions take every param
/// the two most used also have a short form and the full one ends in _with
pub trait LiquidChromeModifiers: IntoElement + Sized {
    /// applies to glass surfaces in this subtree without changing layout
    /// media controls always take the dark glass, light glass over video went milky
    fn liquid_glass_contrast(self, contrast: LiquidGlassContrast) -> AnyElement {
        env_scope::<LiquidGlassContrastKey>(contrast)
            .child(self)
            .modifier(LiquidGlassMediaScheme { active: contrast == LiquidGlassContrast::Media })
            .into_any_element()
    }

    fn liquid_glass_contrast_backdrop(self, shape: Shape, themed: bool) -> AnyElement {
        self.modifier(LiquidGlassContrastBackdrop { shape: GlassOutline::Shape(shape), themed }).into_any_element()
    }

    fn nav_glass_blur_intensity(self, value: f32) -> AnyElement {
        env_scope::<NavGlassBlurIntensityKey>(clamped_intensity(value)).child(self).into_any_element()
    }

    fn glass_follows_switch(self, value: bool) -> AnyElement {
        env_scope::<GlassFollowsSwitchKey>(value).child(self).into_any_element()
    }

    fn liquid_glass_nav_circle(self, blur_intensity: Option<f32>) -> AnyElement {
        self.modifier(LiquidGlassNavCircleModifier { blur_intensity }).into_any_element()
    }

    /// nav pill with default horizontal_padding 11
    fn liquid_glass_nav_pill(self, horizontal_padding: f32, blur_intensity: Option<f32>) -> AnyElement {
        self.modifier(LiquidGlassNavPillModifier { horizontal_padding, blur_intensity }).into_any_element()
    }

    fn page_header_glass_pill(self, horizontal_padding: f32) -> AnyElement {
        self.modifier(PageHeaderGlassPillModifier { horizontal_padding }).into_any_element()
    }

    fn liquid_chrome_capsule(self) -> AnyElement {
        self.liquid_chrome_capsule_with(0.12, true, None, false)
    }

    /// capsule with every param
    /// legacy picks the old capsule, the only one desktop draws
    fn liquid_chrome_capsule_with(
        self,
        stroke_opacity: f64,
        shows_stroke: bool,
        blur_intensity: Option<f32>,
        _legacy: bool,
    ) -> AnyElement {
        self.modifier(LiquidChromeCapsuleModifier { stroke_opacity, shows_stroke, blur_intensity }).into_any_element()
    }

    /// rounded rectangle with the other defaults
    fn liquid_chrome_rounded_rectangle(self, corner_radius: f32) -> AnyElement {
        self.liquid_chrome_rounded_rectangle_with(corner_radius, 0.12, None, false)
    }

    /// the full rounded rectangle
    /// interactive is glass reacting to touch, the painted branch has no look for it
    fn liquid_chrome_rounded_rectangle_with(
        self,
        corner_radius: f32,
        stroke_opacity: f64,
        blur_intensity: Option<f32>,
        _interactive: bool,
    ) -> AnyElement {
        self.modifier(LiquidChromeRoundedRectangleModifier { corner_radius, stroke_opacity, blur_intensity })
            .into_any_element()
    }

    /// uneven rounded rectangle with defaults 0.12, true none false
    #[allow(clippy::too_many_arguments)]
    fn liquid_chrome_uneven_rounded_rectangle(
        self,
        top_leading_radius: f32,
        bottom_leading_radius: f32,
        bottom_trailing_radius: f32,
        top_trailing_radius: f32,
        stroke_opacity: f64,
        shows_stroke: bool,
        blur_intensity: Option<f32>,
        _interactive: bool,
    ) -> AnyElement {
        self.modifier(LiquidChromeUnevenRoundedRectangleModifier {
            top_leading_radius,
            bottom_leading_radius,
            bottom_trailing_radius,
            top_trailing_radius,
            stroke_opacity,
            shows_stroke,
            blur_intensity,
        })
        .into_any_element()
    }

    fn liquid_chrome_circle(self, blur_intensity: Option<f32>) -> AnyElement {
        self.modifier(LiquidChromeCircleModifier { blur_intensity }).into_any_element()
    }
}

impl<E: IntoElement> LiquidChromeModifiers for E {}
