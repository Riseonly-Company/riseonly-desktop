use gpui::{Styled, TextAlign};

use crate::swiftui::environment::LayoutDirection;

/// text alignment, leading and trailing follow the writing direction
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextAlignment {
    #[default]
    Leading,
    Center,
    Trailing,
}

impl TextAlignment {
    /// physical alignment gpui lays lines out with for one writing direction
    pub fn resolve(self, direction: LayoutDirection) -> TextAlign {
        let rtl = direction.is_right_to_left();
        match self {
            Self::Leading if rtl => TextAlign::Right,
            Self::Leading => TextAlign::Left,
            Self::Center => TextAlign::Center,
            Self::Trailing if rtl => TextAlign::Left,
            Self::Trailing => TextAlign::Right,
        }
    }
}

/// truncation mode, a line limited text truncates at the tail unless told otherwise
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TruncationMode {
    Head,
    #[default]
    Tail,
    Middle,
}

impl TruncationMode {
    /// puts the ellipsis where it belongs, gpui only truncates text that has an overflow mode
    pub fn apply<E: Styled>(self, element: E) -> E {
        match self {
            Self::Head => element.text_ellipsis_start(),
            Self::Tail => element.text_ellipsis(),
            Self::Middle => element.text_ellipsis_middle(),
        }
    }
}
