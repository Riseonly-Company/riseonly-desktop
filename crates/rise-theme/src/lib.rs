pub mod tokens;

pub use tokens::auth::AuthMetrics;
pub use tokens::density::Density;
pub use tokens::icon::IconMetrics;
pub use tokens::material::{Material, MaterialBacking, MaterialTokens, PaintedMaterial};
pub use tokens::palette::ThemePalette;
pub use tokens::shell::ShellMetrics;
pub use tokens::spacing::SpacingValues;
pub use tokens::typography::{FONT_FAMILY, SHIPPED_WEIGHTS, TextStyleToken, Typography};

use gpui::{Hsla, Pixels, Rgba, px, rgb};

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Appearance {
    Light,
    #[default]
    Dark,
}

/// Colours and lengths already resolved for one appearance and one density.
///
/// Built once per theme change, never per frame: parsing hex and multiplying
/// densities inside a render pass is exactly the work the performance contract
/// keeps off the frame path.
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
    pub auth: AuthMetrics,
    pub icon: IconMetrics,
    pub material: MaterialTokens,
    pub shell: ShellMetrics,
    pub spacing: SpacingValues,
    pub typography: Typography,
}

impl AppTheme {
    pub fn new(palette: &ThemePalette, appearance: Appearance, density: Density) -> Self {
        let c = color;
        let l = |value: f32| density.scale(value);

        Self {
            appearance,
            density,
            bg: BackgroundColors {
                _000: c(&palette.bg_000),
                _100: c(&palette.bg_100),
                _200: c(&palette.bg_200),
                _300: c(&palette.bg_300),
                _400: c(&palette.bg_400),
                _500: c(&palette.bg_500),
                _600: c(&palette.bg_600),
                _700: c(&palette.bg_700),
            },
            border: BorderColors {
                _100: c(&palette.border_100),
                _200: c(&palette.border_200),
                _300: c(&palette.border_300),
                _400: c(&palette.border_400),
                _500: c(&palette.border_500),
                _600: c(&palette.border_600),
            },
            button: ButtonTheme {
                bg_000: c(&palette.btn_bg_000),
                bg_100: c(&palette.btn_bg_100),
                bg_200: c(&palette.btn_bg_200),
                bg_300: c(&palette.btn_bg_300),
                bg_400: c(&palette.btn_bg_400),
                bg_500: c(&palette.btn_bg_500),
                bg_600: c(&palette.btn_bg_600),
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
                primary: c(&palette.text_100),
                secondary: c(&palette.secondary_100),
            },
            input: InputTheme {
                bg_100: c(&palette.input_bg_100),
                bg_200: c(&palette.input_bg_200),
                bg_300: c(&palette.input_bg_300),
                border_300: c(&palette.input_border_300),
                height_300: l(palette.input_height_300),
                radius_300: l(palette.input_radius_300),
                padding_x: l(18.0),
                caret: c(&palette.primary_100),
                caret_width: l(2.0),
                marked_underline: l(1.0),
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
            gradient_primary: (
                c(&palette.gradient_primary_start),
                c(&palette.gradient_primary_end),
            ),
            auth: AuthMetrics::new(density),
            icon: IconMetrics::new(density),
            material: MaterialTokens::new(palette, appearance, density),
            shell: ShellMetrics::new(density),
            spacing: SpacingValues::new(density),
            typography: Typography::new(density),
        }
    }

    /// The painted form of `material`, for the branch where nothing native is
    /// drawing it.
    ///
    /// A caller reaches this only after `rise-platform` has said the backing is
    /// [`MaterialBacking::Painted`]. Nothing here knows what the machine can
    /// host, and nothing here may decide it.
    pub fn painted_material(&self, material: Material) -> PaintedMaterial {
        self.material.get(material)
    }

    pub fn dark() -> Self {
        Self::new(
            &ThemePalette::default_dark(),
            Appearance::Dark,
            Density::default(),
        )
    }

    pub fn light() -> Self {
        Self::new(
            &ThemePalette::default_light(),
            Appearance::Light,
            Density::default(),
        )
    }
}

impl gpui::Global for AppTheme {}

#[derive(Clone, Copy, Debug)]
#[allow(non_snake_case)]
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

/// The iOS palette carries the first six. The last three are desktop-only and
/// have no counterpart in the reference, because a phone has no caret, no IME
/// underline and no pointer selection: the reference's text fields are UIKit's,
/// and UIKit draws all three itself.
#[derive(Clone, Copy, Debug)]
pub struct InputTheme {
    pub bg_100: Hsla,
    pub bg_200: Hsla,
    pub bg_300: Hsla,
    pub border_300: Hsla,
    pub height_300: Pixels,
    pub radius_300: Pixels,
    /// Inside the field, left and right.
    ///
    /// Its own token rather than "whatever the radius is": the radius became a
    /// pill and the text would have followed it out to 30pt of dead space on
    /// each side.
    pub padding_x: Pixels,
    pub caret: Hsla,
    pub caret_width: Pixels,
    /// The mark an input method draws under uncommitted text. The only
    /// underline in the kit, which is why it is here rather than in a general
    /// stroke scale that would have exactly one member.
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

pub fn color(hex: &str) -> Hsla {
    parse_hex(hex)
        .unwrap_or(Rgba {
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 1.0,
        })
        .into()
}

fn parse_hex(hex: &str) -> Option<Rgba> {
    let digits: String = hex.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    let value = u32::from_str_radix(&digits, 16).ok()?;

    match digits.len() {
        3 => {
            let r = (value >> 8) & 0xF;
            let g = (value >> 4) & 0xF;
            let b = value & 0xF;
            Some(rgb((r * 17) << 16 | (g * 17) << 8 | (b * 17)))
        }
        6 => Some(rgb(value)),
        8 => {
            let alpha = ((value >> 24) & 0xFF) as f32 / 255.0;
            let mut base = rgb(value & 0x00FF_FFFF);
            base.a = alpha;
            Some(base)
        }
        _ => None,
    }
}

pub fn length(value: f32) -> Pixels {
    px(value)
}

/// The same colour at a different opacity.
///
/// Lives here because constructing an `Hsla` is constructing a colour, and a
/// component that spelled `Hsla { a: 0.0, ..token }` would be one struct literal
/// away from spelling a whole colour.
pub fn alpha(color: Hsla, alpha: f32) -> Hsla {
    Hsla {
        a: alpha.clamp(0.0, 1.0),
        ..color
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn six_digit_hex_parses() {
        let parsed = parse_hex("#FF5A5A").unwrap();
        assert!((parsed.r - 1.0).abs() < 1e-3);
        assert!((parsed.g - 0.352).abs() < 1e-2);
        assert!((parsed.b - 0.352).abs() < 1e-2);
        assert!((parsed.a - 1.0).abs() < 1e-6);
    }

    #[test]
    fn three_digit_hex_expands() {
        let short = parse_hex("#F00").unwrap();
        let long = parse_hex("#FF0000").unwrap();
        assert!((short.r - long.r).abs() < 1e-6);
        assert!((short.g - long.g).abs() < 1e-6);
        assert!((short.b - long.b).abs() < 1e-6);
    }

    #[test]
    fn eight_digit_hex_carries_alpha_in_the_high_byte() {
        let parsed = parse_hex("#80FF0000").unwrap();
        assert!((parsed.a - 0.502).abs() < 1e-2);
        assert!((parsed.r - 1.0).abs() < 1e-3);
    }

    #[test]
    fn malformed_hex_falls_back_to_opaque_black_instead_of_panicking() {
        assert!(parse_hex("not-a-colour").is_none());
        let fallback = color("not-a-colour");
        assert!((fallback.a - 1.0).abs() < 1e-6);
    }

    #[test]
    fn density_scales_lengths_but_never_colours() {
        let palette = ThemePalette::default_dark();
        let normal = AppTheme::new(&palette, Appearance::Dark, Density::default());
        let dense = AppTheme::new(&palette, Appearance::Dark, Density::new(1.25));

        assert_eq!(normal.button.height_300, px(45.0));
        assert_eq!(dense.button.height_300, px(45.0 * 1.25));
        assert_eq!(normal.primary._100, dense.primary._100);
    }

    #[test]
    fn the_reference_geometry_is_preserved_at_default_density() {
        let theme = AppTheme::dark();
        assert_eq!(theme.button.height_300, px(45.0));
        assert_eq!(theme.input.height_300, px(45.0));
        assert_eq!(theme.radius._300, px(10.0));
    }
}
