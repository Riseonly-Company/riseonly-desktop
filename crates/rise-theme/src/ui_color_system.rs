use gpui::{Hsla, rgb};

use crate::Appearance;

/// one appearance of a system color, an srgb value and its opacity
#[derive(Clone, Copy, PartialEq, Debug)]
struct UIColorComponents {
    rgb: u32,
    alpha: f32,
}

impl UIColorComponents {
    const fn new(rgb: u32, alpha: f32) -> Self {
        Self { rgb, alpha }
    }

    const fn opaque(rgb: u32) -> Self {
        Self { rgb, alpha: 1.0 }
    }

    fn hsla(self) -> Hsla {
        let mut value = rgb(self.rgb);
        value.a = self.alpha;
        value.into()
    }
}

/// dynamic system colors with light and dark values as a phone resolves them
/// phone values on purpose and not macos ones, so a ported screen looks the same on every host
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct UIColor {
    light: UIColorComponents,
    dark: UIColorComponents,
}

impl UIColor {
    const fn dynamic(light: UIColorComponents, dark: UIColorComponents) -> Self {
        Self { light, dark }
    }

    const fn opaque(light: u32, dark: u32) -> Self {
        Self::dynamic(
            UIColorComponents::opaque(light),
            UIColorComponents::opaque(dark),
        )
    }

    pub fn resolved_color(self, appearance: Appearance) -> Hsla {
        match appearance {
            Appearance::Light => self.light.hsla(),
            Appearance::Dark => self.dark.hsla(),
        }
    }

    pub const fn label() -> Self {
        Self::opaque(0x000000, 0xFFFFFF)
    }

    pub const fn secondary_label() -> Self {
        Self::dynamic(
            UIColorComponents::new(0x3C3C43, 0.6),
            UIColorComponents::new(0xEBEBF5, 0.6),
        )
    }

    pub const fn tertiary_label() -> Self {
        Self::dynamic(
            UIColorComponents::new(0x3C3C43, 0.3),
            UIColorComponents::new(0xEBEBF5, 0.3),
        )
    }

    pub const fn quaternary_label() -> Self {
        Self::dynamic(
            UIColorComponents::new(0x3C3C43, 0.18),
            UIColorComponents::new(0xEBEBF5, 0.18),
        )
    }

    pub const fn placeholder_text() -> Self {
        Self::dynamic(
            UIColorComponents::new(0x3C3C43, 0.3),
            UIColorComponents::new(0xEBEBF5, 0.3),
        )
    }

    pub const fn link() -> Self {
        Self::opaque(0x007AFF, 0x0984FF)
    }

    pub const fn system_fill() -> Self {
        Self::dynamic(
            UIColorComponents::new(0x787880, 0.2),
            UIColorComponents::new(0x787880, 0.36),
        )
    }

    pub const fn secondary_system_fill() -> Self {
        Self::dynamic(
            UIColorComponents::new(0x787880, 0.16),
            UIColorComponents::new(0x787880, 0.32),
        )
    }

    pub const fn tertiary_system_fill() -> Self {
        Self::dynamic(
            UIColorComponents::new(0x767680, 0.12),
            UIColorComponents::new(0x767680, 0.24),
        )
    }

    pub const fn quaternary_system_fill() -> Self {
        Self::dynamic(
            UIColorComponents::new(0x747480, 0.08),
            UIColorComponents::new(0x767680, 0.18),
        )
    }

    pub const fn system_background() -> Self {
        Self::opaque(0xFFFFFF, 0x000000)
    }

    pub const fn secondary_system_background() -> Self {
        Self::opaque(0xF2F2F7, 0x1C1C1E)
    }

    pub const fn tertiary_system_background() -> Self {
        Self::opaque(0xFFFFFF, 0x2C2C2E)
    }

    pub const fn system_grouped_background() -> Self {
        Self::opaque(0xF2F2F7, 0x000000)
    }

    pub const fn secondary_system_grouped_background() -> Self {
        Self::opaque(0xFFFFFF, 0x1C1C1E)
    }

    pub const fn tertiary_system_grouped_background() -> Self {
        Self::opaque(0xF2F2F7, 0x2C2C2E)
    }

    pub const fn separator() -> Self {
        Self::dynamic(
            UIColorComponents::new(0x3C3C43, 0.29),
            UIColorComponents::new(0x545458, 0.6),
        )
    }

    pub const fn opaque_separator() -> Self {
        Self::opaque(0xC6C6C8, 0x38383A)
    }

    pub const fn system_red() -> Self {
        Self::opaque(0xFF383C, 0xFF4245)
    }

    pub const fn system_orange() -> Self {
        Self::opaque(0xFF8D28, 0xFF9230)
    }

    pub const fn system_yellow() -> Self {
        Self::opaque(0xFFCC00, 0xFFD600)
    }

    pub const fn system_green() -> Self {
        Self::opaque(0x34C759, 0x30D158)
    }

    pub const fn system_mint() -> Self {
        Self::opaque(0x00C8B3, 0x00DAC3)
    }

    pub const fn system_teal() -> Self {
        Self::opaque(0x00C3D0, 0x00D2E0)
    }

    pub const fn system_cyan() -> Self {
        Self::opaque(0x00C0E8, 0x3CD3FE)
    }

    pub const fn system_blue() -> Self {
        Self::opaque(0x0088FF, 0x0091FF)
    }

    pub const fn system_indigo() -> Self {
        Self::opaque(0x6155F5, 0x6D7CFF)
    }

    pub const fn system_purple() -> Self {
        Self::opaque(0xCB30E0, 0xDB34F2)
    }

    pub const fn system_pink() -> Self {
        Self::opaque(0xFF2D55, 0xFF375F)
    }

    pub const fn system_brown() -> Self {
        Self::opaque(0xAC7F5E, 0xB78A66)
    }

    pub const fn system_gray() -> Self {
        Self::opaque(0x8E8E93, 0x8E8E93)
    }

    pub const fn system_gray2() -> Self {
        Self::opaque(0xAEAEB2, 0x636366)
    }

    pub const fn system_gray3() -> Self {
        Self::opaque(0xC7C7CC, 0x48484A)
    }

    pub const fn system_gray4() -> Self {
        Self::opaque(0xD1D1D6, 0x3A3A3C)
    }

    pub const fn system_gray5() -> Self {
        Self::opaque(0xE5E5EA, 0x2C2C2E)
    }

    pub const fn system_gray6() -> Self {
        Self::opaque(0xF2F2F7, 0x1C1C1E)
    }
}
