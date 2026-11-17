use std::sync::Arc;

use gpui::{FontFeatures, FontStyle, FontWeight, SharedString, Styled, px, relative};
use rise_platform::text_capabilities::monospaced_family;
use rise_theme::FONT_FAMILY;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum FontDesign {
    #[default]
    Default,
    Monospaced,
    Rounded,
    Serif,
}

/// text style with sizes and weights of the large content size
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum TextStyle {
    LargeTitle,
    Title,
    Title2,
    Title3,
    Headline,
    Subheadline,
    Body,
    Callout,
    Footnote,
    Caption,
    Caption2,
}

impl TextStyle {
    /// point size at the large category, the default when the user changed nothing
    pub const fn size(self) -> f32 {
        match self {
            Self::LargeTitle => 34.0,
            Self::Title => 28.0,
            Self::Title2 => 22.0,
            Self::Title3 => 20.0,
            Self::Headline | Self::Body => 17.0,
            Self::Callout => 16.0,
            Self::Subheadline => 15.0,
            Self::Footnote => 13.0,
            Self::Caption => 12.0,
            Self::Caption2 => 11.0,
        }
    }

    /// headline is the only style that isnt regular weight
    pub const fn weight(self) -> FontWeight {
        match self {
            Self::Headline => FontWeight::SEMIBOLD,
            _ => FontWeight::NORMAL,
        }
    }
}

/// font as a value, what the font modifier hands to the text inside
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Font {
    pub size: f32,
    pub weight: FontWeight,
    pub design: FontDesign,
    pub italic: bool,
    pub monospaced_digit: bool,
}

impl Font {
    /// system font of a text style
    pub const fn text_style(style: TextStyle) -> Self {
        Self {
            size: style.size(),
            weight: style.weight(),
            design: FontDesign::Default,
            italic: false,
            monospaced_digit: false,
        }
    }

    pub const fn large_title() -> Self {
        Self::text_style(TextStyle::LargeTitle)
    }

    pub const fn title() -> Self {
        Self::text_style(TextStyle::Title)
    }

    pub const fn title2() -> Self {
        Self::text_style(TextStyle::Title2)
    }

    pub const fn title3() -> Self {
        Self::text_style(TextStyle::Title3)
    }

    pub const fn headline() -> Self {
        Self::text_style(TextStyle::Headline)
    }

    pub const fn subheadline() -> Self {
        Self::text_style(TextStyle::Subheadline)
    }

    pub const fn body() -> Self {
        Self::text_style(TextStyle::Body)
    }

    pub const fn callout() -> Self {
        Self::text_style(TextStyle::Callout)
    }

    pub const fn footnote() -> Self {
        Self::text_style(TextStyle::Footnote)
    }

    pub const fn caption() -> Self {
        Self::text_style(TextStyle::Caption)
    }

    pub const fn caption2() -> Self {
        Self::text_style(TextStyle::Caption2)
    }

    pub const fn system(size: f32, weight: FontWeight, design: FontDesign) -> Self {
        Self {
            size,
            weight,
            design,
            italic: false,
            monospaced_digit: false,
        }
    }

    pub const fn system_size(size: f32) -> Self {
        Self::system(size, FontWeight::NORMAL, FontDesign::Default)
    }

    pub const fn system_weight(size: f32, weight: FontWeight) -> Self {
        Self::system(size, weight, FontDesign::Default)
    }

    pub const fn weight(mut self, weight: FontWeight) -> Self {
        self.weight = weight;
        self
    }

    pub const fn bold(self) -> Self {
        self.weight(FontWeight::BOLD)
    }

    pub const fn italic(mut self) -> Self {
        self.italic = true;
        self
    }

    /// whole run in the monospaced face, size and weight kept
    pub const fn monospaced(mut self) -> Self {
        self.design = FontDesign::Monospaced;
        self
    }

    /// digits of one width, letters untouched
    pub const fn monospaced_digit(mut self) -> Self {
        self.monospaced_digit = true;
        self
    }

    /// family gpui shapes this font with
    pub fn family(&self) -> SharedString {
        match self.design {
            FontDesign::Monospaced => SharedString::new_static(monospaced_family()),
            FontDesign::Default | FontDesign::Rounded | FontDesign::Serif => {
                SharedString::new_static(FONT_FAMILY)
            }
        }
    }

    /// opentype features the modifiers ask for
    pub fn features(&self) -> FontFeatures {
        if self.monospaced_digit {
            FontFeatures(Arc::new(vec![(TABULAR_FIGURES.to_string(), 1)]))
        } else {
            FontFeatures::default()
        }
    }

    pub fn style(&self) -> FontStyle {
        if self.italic {
            FontStyle::Italic
        } else {
            FontStyle::Normal
        }
    }
}

pub const TABULAR_FIGURES: &str = "tnum";

pub const NATURAL_LINE_HEIGHT: f32 = (1984.0 + 494.0) / 2048.0;

/// font and tracking on any element whose text inherits its style
pub trait SwiftUiFont: Styled + Sized {
    /// sets font, size and weight and face and slant and digits cascade to the text inside
    fn swiftui_font(mut self, font: Font) -> Self {
        let style = self.text_style();
        style.font_family = Some(font.family());
        style.font_size = Some(px(font.size).into());
        style.font_weight = Some(font.weight);
        style.font_style = Some(font.style());
        style.font_features = Some(font.features());
        style.line_height = Some(relative(NATURAL_LINE_HEIGHT));
        self
    }

    /// letter spacing
    fn tracking(self, _tracking: f32) -> Self {
        self
    }
}

impl<E: Styled> SwiftUiFont for E {}
