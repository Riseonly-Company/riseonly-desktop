use gpui::{Hsla, Rgba};
use rise_i18n::{plural_word, tr};

use crate::core::lib::canon_format::RiseFormat;

/// multicolor icon tiles for chat settings
pub enum ChatTileColor {}

impl ChatTileColor {
    pub fn blue() -> Hsla {
        Self::rgb(0.20, 0.52, 0.98)
    }

    pub fn orange() -> Hsla {
        Self::rgb(0.98, 0.60, 0.16)
    }

    pub fn green() -> Hsla {
        Self::rgb(0.20, 0.72, 0.45)
    }

    pub fn pink() -> Hsla {
        Self::rgb(0.96, 0.32, 0.45)
    }

    pub fn teal() -> Hsla {
        Self::rgb(0.18, 0.68, 0.71)
    }

    pub fn indigo() -> Hsla {
        Self::rgb(0.35, 0.34, 0.84)
    }

    pub fn purple() -> Hsla {
        Self::rgb(0.56, 0.27, 0.68)
    }

    pub fn gray() -> Hsla {
        Self::rgb(0.56, 0.56, 0.58)
    }

    pub fn red() -> Hsla {
        Self::rgb(0.92, 0.27, 0.30)
    }

    /// opaque srgb color from r g b
    fn rgb(r: f32, g: f32, b: f32) -> Hsla {
        Rgba { r, g, b, a: 1.0 }.into()
    }
}

pub enum ChatCountFormatter {}

impl ChatCountFormatter {
    pub fn compact(n: i64) -> String {
        RiseFormat::compact_count(n)
    }

    pub fn members(n: i64) -> String {
        let one = tr("chat_count_member_one");
        let few = tr("chat_count_member_few");
        let many = tr("chat_count_member_many");
        format!("{} {}", Self::compact(n), plural_word(n, &one, &few, &many))
    }

    pub fn subscribers(n: i64) -> String {
        let one = tr("chat_count_subscriber_one");
        let few = tr("chat_count_subscriber_few");
        let many = tr("chat_count_subscriber_many");
        format!("{} {}", Self::compact(n), plural_word(n, &one, &few, &many))
    }
}
