use std::cell::Cell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use gpui::{Hsla, Pixels, Rgba, black, px, white};
use rise_canon::rise_theme_tokens::parse_color;

use super::rise_theme_preferences::RiseThemeAppearance;
use super::theme_block_color::ThemeBlockColor;
use super::theme_palette::ThemePalette;
use crate::tokens::auth::AuthMetrics;
use crate::tokens::avatar::AvatarMetrics;
use crate::tokens::comment::CommentMetrics;
use crate::tokens::density::Density;
use crate::tokens::feed::FeedMetrics;
use crate::tokens::grouped::GroupedMetrics;
use crate::tokens::header::HeaderMetrics;
use crate::tokens::icon::IconMetrics;
use crate::tokens::list::ListMetrics;
use crate::tokens::material::{Material, MaterialTokens, PaintedMaterial};
use crate::tokens::modal::ModalMetrics;
use crate::tokens::panel::PanelMetrics;
use crate::tokens::profile::ProfileMetrics;
use crate::tokens::shell::ShellMetrics;
use crate::tokens::spacing::SpacingValues;
use crate::tokens::story::StoryMetrics;
use crate::tokens::tabs::TabsMetrics;
use crate::tokens::typography::Typography;

pub type Appearance = RiseThemeAppearance;

const LEGIBLE_CACHE_LIMIT: usize = 64;

type ColorKey = [u8; 4];

#[derive(Default, Debug)]
struct InkCaches {
    legible: HashMap<ColorKey, Hsla>,
    content_on: HashMap<ColorKey, Hsla>,
}

/// built and held by ThemeStore
/// never build it from a palette inside render, that parses forty colors every frame
#[derive(Clone, Debug)]
pub struct AppTheme {
    pub appearance: Appearance,
    pub density: Density,
    pub bg: BackgroundColors,
    pub border: BorderColors,
    pub button: ButtonTheme,
    pub primary: PrimaryColors,
    pub semantic: SemanticColors,
    pub text: TextColors,
    pub input: InputTheme,
    pub radius: RadiusValues,
    pub gradient_primary: (Hsla, Hsla),
    pub background_hex: String,
    pub background_luminance: f64,
    pub auth: AuthMetrics,
    pub avatar: AvatarMetrics,
    pub comment: CommentMetrics,
    pub feed: FeedMetrics,
    pub grouped: GroupedMetrics,
    pub header: HeaderMetrics,
    pub icon: IconMetrics,
    pub list: ListMetrics,
    pub material: MaterialTokens,
    pub modal: ModalMetrics,
    pub panel: PanelMetrics,
    pub profile: ProfileMetrics,
    pub shell: ShellMetrics,
    pub spacing: SpacingValues,
    pub story: StoryMetrics,
    pub tabs: TabsMetrics,
    pub typography: Typography,
    caches: Arc<Mutex<InkCaches>>,
}

thread_local! {
    static FOCUS_RING_COLOR: Cell<Hsla> = const { Cell::new(Hsla { h: 0.0, s: 0.0, l: 0.0, a: 0.0 }) };
}

impl AppTheme {
    /// mirrors this theme keyboard focus ring color (primary._100) for readers with no context
    /// called wherever the theme global is installed
    pub fn publish_focus_ring_color(&self) {
        let color = self.primary._100;
        FOCUS_RING_COLOR.with(|cell| cell.set(color));
    }

    /// keyboard focus ring color of the installed theme, transparent before any install
    pub fn focus_ring_color() -> Hsla {
        FOCUS_RING_COLOR.with(Cell::get)
    }

    /// builds the theme from a palette plus the desktop density multiplier on lengths
    /// and the appearance the painted materials resolve for
    pub fn new(palette: &ThemePalette, appearance: Appearance, density: Density) -> Self {
        let c = |hex: &str| color(hex);
        let l = |value: f64| density.scale(value as f32);
        let page = c(&palette.bg_100);
        let page_ink = palette.readable_text_100();
        let ink_flipped = palette.page_ink_flipped();
        let surface = |hex: &str| follow_page(c(hex), page, page_ink, ink_flipped);
        let level = |hex: &str, dark: &str, light: &str| {
            palette.level_following_block(hex, dark, light).map(|hex| c(&hex)).unwrap_or_else(|| surface(hex))
        };
        let stock_dark = ThemePalette::default_dark();
        let stock_light = ThemePalette::default_light();
        let btn_bg_300 = level(&palette.btn_bg_300, &stock_dark.btn_bg_300, &stock_light.btn_bg_300);
        let text_100 = c(&palette.text_100);

        Self {
            appearance,
            density,
            bg: BackgroundColors {
                _000: surface(&palette.bg_000),
                _100: page,
                _200: if palette.block_color_is_auto() { c(&palette.auto_block_hex()) } else { surface(&palette.bg_200) },
                _300: level(&palette.bg_300, &stock_dark.bg_300, &stock_light.bg_300),
                _400: level(&palette.bg_400, &stock_dark.bg_400, &stock_light.bg_400),
                _500: level(&palette.bg_500, &stock_dark.bg_500, &stock_light.bg_500),
                _600: level(&palette.bg_600, &stock_dark.bg_600, &stock_light.bg_600),
                _700: surface(&palette.bg_700),
            },
            border: BorderColors {
                _100: surface(&palette.border_100),
                _200: surface(&palette.border_200),
                _300: surface(&palette.border_300),
                _400: surface(&palette.border_400),
                _500: surface(&palette.border_500),
                _600: surface(&palette.border_600),
            },
            button: ButtonTheme {
                bg_000: surface(&palette.btn_bg_000),
                bg_100: surface(&palette.btn_bg_100),
                bg_200: surface(&palette.btn_bg_200),
                bg_300: btn_bg_300,
                bg_400: level(&palette.btn_bg_400, &stock_dark.btn_bg_400, &stock_light.btn_bg_400),
                bg_500: level(&palette.btn_bg_500, &stock_dark.btn_bg_500, &stock_light.btn_bg_500),
                bg_600: level(&palette.btn_bg_600, &stock_dark.btn_bg_600, &stock_light.btn_bg_600),
                content_300: readable_ink(surface(&palette.btn_bg_300), text_100),
                height_100: l(palette.btn_height_100),
                height_200: l(palette.btn_height_200),
                height_300: l(palette.btn_height_300),
                height_400: l(palette.btn_height_400),
                height_500: l(palette.btn_height_500),
                height_600: l(palette.btn_height_600),
                radius_000: l(palette.btn_radius_000),
                radius_100: l(palette.btn_radius_100),
                radius_200: l(palette.btn_radius_200),
                radius_300: l(palette.btn_radius_300),
            },
            primary: PrimaryColors {
                _100: c(&palette.primary_100),
                _200: c(&palette.primary_200),
                _300: c(&palette.primary_300),
            },
            semantic: SemanticColors {
                success_100: c(&palette.success_100),
                success_200: c(&palette.success_200),
                success_300: c(&palette.success_300),
                error_100: c(&palette.error_100),
                error_200: c(&palette.error_200),
                error_300: c(&palette.error_300),
            },
            text: TextColors {
                primary: page_ink,
                secondary: palette.readable_secondary_100(),
            },
            input: InputTheme {
                bg_100: surface(&palette.input_bg_100),
                bg_200: surface(&palette.input_bg_200),
                bg_300: surface(&palette.input_bg_300),
                border_300: surface(&palette.input_border_300),
                height_300: l(palette.input_height_300),
                radius_300: l(palette.input_radius_300),
                padding_x: density.scale(18.0),
                caret: c(&palette.primary_100),
                caret_width: density.scale(2.0),
                marked_underline: density.scale(1.0),
                selection: alpha(c(&palette.primary_200), 0.30),
            },
            radius: RadiusValues {
                _100: l(palette.radius_100),
                _200: l(palette.radius_200),
                _300: l(palette.radius_300),
                _400: l(palette.radius_400),
                _500: l(palette.radius_500),
                _600: l(palette.radius_600),
            },
            gradient_primary: (c(&palette.gradient_primary_start), c(&palette.gradient_primary_end)),
            background_hex: palette.bg_100.clone(),
            background_luminance: relative_luminance_of(c(&palette.bg_100)),
            auth: AuthMetrics::new(density),
            avatar: AvatarMetrics::new(density),
            comment: CommentMetrics::new(density),
            feed: FeedMetrics::new(density),
            grouped: GroupedMetrics::new(density),
            header: HeaderMetrics::new(density),
            icon: IconMetrics::new(density),
            list: ListMetrics::new(density),
            material: MaterialTokens::new(palette, appearance, density),
            modal: ModalMetrics::new(density),
            panel: PanelMetrics::new(density),
            profile: ProfileMetrics::new(density),
            shell: ShellMetrics::new(density),
            spacing: SpacingValues::new(density),
            story: StoryMetrics::new(density),
            tabs: TabsMetrics::new(density),
            typography: Typography::new(density),
            caches: Arc::default(),
        }
    }

    /// painted form of material, valid only once the backing is MaterialBacking Painted
    pub fn painted_material(&self, material: Material) -> PaintedMaterial {
        self.material.get(material)
    }

    pub fn dark() -> Self {
        Self::new(&ThemePalette::default_dark(), Appearance::Dark, Density::default())
    }

    pub fn light() -> Self {
        Self::new(&ThemePalette::default_light(), Appearance::Light, Density::default())
    }

    /// pushes a user picked color toward white on dark or black on light until it stands out
    /// so dark navy on the dark theme stays legible
    pub fn legible(&self, color: Hsla) -> Hsla {
        let key = color_key(color);
        if let Some(cached) = self.with_caches(|caches| caches.legible.get(&key).copied()) {
            return cached;
        }
        let result = self.compute_legible(color);
        self.with_caches(|caches| remember(&mut caches.legible, key, result));
        result
    }

    /// ink for text and loaders drawn on fill
    /// a fill close to the background (contrast under 1.5) keeps the theme text color
    /// otherwise white while it reaches 3:1 else black
    pub fn content_on(&self, fill: Hsla) -> Hsla {
        let key = color_key(fill);
        if let Some(cached) = self.with_caches(|caches| caches.content_on.get(&key).copied()) {
            return cached;
        }
        let result = self.compute_content_on(fill);
        self.with_caches(|caches| remember(&mut caches.content_on, key, result));
        result
    }

    fn with_caches<R>(&self, f: impl FnOnce(&mut InkCaches) -> R) -> R {
        let mut guard = self.caches.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        f(&mut guard)
    }

    fn compute_content_on(&self, fill: Hsla) -> Hsla {
        let Rgba { mut r, mut g, mut b, a } = fill.to_rgb();
        if a < 1.0 {
            let background = self.bg._100.to_rgb();
            let alpha = a.max(0.0);
            r = r * alpha + background.r * (1.0 - alpha);
            g = g * alpha + background.g * (1.0 - alpha);
            b = b * alpha + background.b * (1.0 - alpha);
        }
        let fill_luminance = relative_luminance(r, g, b);
        if contrast_ratio(fill_luminance, self.background_luminance) < 1.5 {
            return self.text.primary;
        }
        if contrast_ratio(fill_luminance, 1.0) >= 3.0 { white() } else { black() }
    }

    fn compute_legible(&self, color: Hsla) -> Hsla {
        let Rgba { r, g, b, a } = color.to_rgb();
        let background = self.background_luminance;
        if contrast_ratio(background, relative_luminance(r, g, b)) >= 3.0 {
            return color;
        }
        let target: f32 = if background < 0.5 { 1.0 } else { 0.0 };
        for step in 1..=10 {
            let t = step as f32 / 10.0;
            let mr = r + (target - r) * t;
            let mg = g + (target - g) * t;
            let mb = b + (target - b) * t;
            if contrast_ratio(background, relative_luminance(mr, mg, mb)) >= 3.0 {
                return Rgba { r: mr, g: mg, b: mb, a }.into();
            }
        }
        if target == 1.0 { white() } else { black() }
    }
}

impl gpui::Global for AppTheme {}

/// theme text color while it stays readable on background
/// else white or black, whichever stands out more
pub fn readable_ink(background: Hsla, preferred: Hsla) -> Hsla {
    let bg = relative_luminance_of(background);
    if !ink_flips(background, preferred) {
        return preferred;
    }
    if contrast_ratio(bg, 1.0) >= contrast_ratio(bg, 0.0) { white() } else { black() }
}

/// true when the theme text color misses 3:1 on background and readable_ink swaps it
pub fn ink_flips(background: Hsla, preferred: Hsla) -> bool {
    contrast_ratio(relative_luminance_of(background), relative_luminance_of(preferred)) < 3.0
}

const SECONDARY_MIN_CONTRAST: f64 = 2.0;
const SECONDARY_INK_ALPHA: f32 = 0.62;

pub fn readable_secondary(background: Hsla, preferred: Hsla) -> Hsla {
    let bg = relative_luminance_of(background);
    if contrast_ratio(bg, relative_luminance_of(preferred)) >= SECONDARY_MIN_CONTRAST {
        return preferred;
    }
    let ink = if contrast_ratio(bg, 1.0) >= contrast_ratio(bg, 0.0) { white() } else { black() };
    alpha(ink, SECONDARY_INK_ALPHA)
}

const SURFACE_STEP_SCALE: f32 = 2.0;
const SURFACE_STEP_MAX: f32 = 0.3;

/// paper surface of the theme after the page flipped polarity
/// while it fails 3:1 against the new ink it is rebuilt from the page
/// stepped toward ink by twice its old distance from paper (cap 0.3)
pub fn follow_page(surface: Hsla, page: Hsla, ink: Hsla, flipped: bool) -> Hsla {
    if !flipped {
        return surface;
    }
    if contrast_ratio(relative_luminance_of(surface), relative_luminance_of(ink)) >= 3.0 {
        return surface;
    }
    let s = surface.to_rgb();
    let k = ink.to_rgb();
    let p = page.to_rgb();
    let distance = ((s.r - k.r).abs() + (s.g - k.g).abs() + (s.b - k.b).abs()) / 3.0;
    let t = (distance * SURFACE_STEP_SCALE).min(SURFACE_STEP_MAX);
    Rgba { r: p.r + (k.r - p.r) * t, g: p.g + (k.g - p.g) * t, b: p.b + (k.b - p.b) * t, a: s.a }.into()
}

pub trait ThemePaletteReadable {
    fn readable_text_100(&self) -> Hsla;
    fn readable_secondary_100(&self) -> Hsla;
    fn page_ink_flipped(&self) -> bool;
    fn readable_surface(&self, hex: &str) -> Hsla;
}

impl ThemePaletteReadable for ThemePalette {
    fn readable_text_100(&self) -> Hsla {
        readable_ink(color(&self.bg_100), color(&self.text_100))
    }

    fn readable_secondary_100(&self) -> Hsla {
        readable_secondary(color(&self.bg_100), color(&self.secondary_100))
    }

    fn page_ink_flipped(&self) -> bool {
        ink_flips(color(&self.bg_100), color(&self.text_100))
    }

    fn readable_surface(&self, hex: &str) -> Hsla {
        follow_page(color(hex), color(&self.bg_100), self.readable_text_100(), self.page_ink_flipped())
    }
}

fn remember(cache: &mut HashMap<ColorKey, Hsla>, key: ColorKey, value: Hsla) {
    if cache.len() >= LEGIBLE_CACHE_LIMIT {
        cache.clear();
    }
    cache.insert(key, value);
}

fn color_key(color: Hsla) -> ColorKey {
    let Rgba { r, g, b, a } = color.to_rgb();
    let byte = |unit: f32| (unit.clamp(0.0, 1.0) * 255.0).round() as u8;
    [byte(r), byte(g), byte(b), byte(a)]
}

fn relative_luminance_of(color: Hsla) -> f64 {
    let Rgba { r, g, b, .. } = color.to_rgb();
    relative_luminance(r, g, b)
}

fn relative_luminance(r: f32, g: f32, b: f32) -> f64 {
    let linear = |channel: f32| {
        let value = f64::from(channel);
        if value <= 0.03928 { value / 12.92 } else { ((value + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

fn contrast_ratio(a: f64, b: f64) -> f64 {
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

/// parses a hex color, accepted forms and the black fallback are canon
/// so every client parses the same
pub fn color(hex: &str) -> Hsla {
    let (r, g, b, a) = parse_color(hex).unit_components();
    Rgba { r: r as f32, g: g as f32, b: b as f32, a: a as f32 }.into()
}

pub fn length(value: f32) -> Pixels {
    px(value)
}

/// same color at another opacity, alpha is clamped to 0 to 1
pub fn alpha(color: Hsla, alpha: f32) -> Hsla {
    Hsla { a: alpha.clamp(0.0, 1.0), ..color }
}

#[derive(Clone, Copy, Debug)]
pub struct BackgroundColors {
    pub _000: Hsla,
    pub _100: Hsla,
    pub _200: Hsla,
    pub _300: Hsla,
    pub _400: Hsla,
    pub _500: Hsla,
    pub _600: Hsla,
    pub _700: Hsla,
}

#[derive(Clone, Copy, Debug)]
pub struct BorderColors {
    pub _100: Hsla,
    pub _200: Hsla,
    pub _300: Hsla,
    pub _400: Hsla,
    pub _500: Hsla,
    pub _600: Hsla,
}

#[derive(Clone, Copy, Debug)]
pub struct ButtonTheme {
    pub bg_000: Hsla,
    pub bg_100: Hsla,
    pub bg_200: Hsla,
    pub bg_300: Hsla,
    pub bg_400: Hsla,
    pub bg_500: Hsla,
    pub bg_600: Hsla,
    pub content_300: Hsla,
    pub height_100: Pixels,
    pub height_200: Pixels,
    pub height_300: Pixels,
    pub height_400: Pixels,
    pub height_500: Pixels,
    pub height_600: Pixels,
    pub radius_000: Pixels,
    pub radius_100: Pixels,
    pub radius_200: Pixels,
    pub radius_300: Pixels,
}

#[derive(Clone, Copy, Debug)]
pub struct PrimaryColors {
    pub _100: Hsla,
    pub _200: Hsla,
    pub _300: Hsla,
}

#[derive(Clone, Copy, Debug)]
pub struct SemanticColors {
    pub success_100: Hsla,
    pub success_200: Hsla,
    pub success_300: Hsla,
    pub error_100: Hsla,
    pub error_200: Hsla,
    pub error_300: Hsla,
}

#[derive(Clone, Copy, Debug)]
pub struct TextColors {
    pub primary: Hsla,
    pub secondary: Hsla,
}

#[derive(Clone, Copy, Debug)]
pub struct InputTheme {
    pub bg_100: Hsla,
    pub bg_200: Hsla,
    pub bg_300: Hsla,
    pub border_300: Hsla,
    pub height_300: Pixels,
    pub radius_300: Pixels,
    pub padding_x: Pixels,
    pub caret: Hsla,
    pub caret_width: Pixels,
    pub marked_underline: Pixels,
    pub selection: Hsla,
}

#[derive(Clone, Copy, Debug)]
pub struct RadiusValues {
    pub _100: Pixels,
    pub _200: Pixels,
    pub _300: Pixels,
    pub _400: Pixels,
    pub _500: Pixels,
    pub _600: Pixels,
}
