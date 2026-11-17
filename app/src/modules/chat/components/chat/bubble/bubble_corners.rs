use gpui::{Corners, Hsla, Pixels, px};
use rise_ui::message_bubble_geometry::{MessageBubbleRadii, MessageBubbleShape, MessageBubbleTailSide};
use rise_ui::swiftui::environment::LayoutDirection;

/// per corner radii of a bubble, plus the tail it carries
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BubbleCorners {
    pub top_leading: Pixels,
    pub top_trailing: Pixels,
    pub bottom_leading: Pixels,
    pub bottom_trailing: Pixels,
    pub tail_side: MessageBubbleTailSide,
    pub draws_tail: bool,
}

impl BubbleCorners {
    /// builds the corners with tail side and draws tail flag
    pub fn new(
        top_leading: Pixels,
        top_trailing: Pixels,
        bottom_leading: Pixels,
        bottom_trailing: Pixels,
        tail_side: MessageBubbleTailSide,
        draws_tail: bool,
    ) -> Self {
        Self {
            top_leading,
            top_trailing,
            bottom_leading,
            bottom_trailing,
            tail_side,
            draws_tail,
        }
    }

    /// corners with no tail, tail side none and draws tail false
    pub fn radii(top_leading: Pixels, top_trailing: Pixels, bottom_leading: Pixels, bottom_trailing: Pixels) -> Self {
        Self::new(
            top_leading,
            top_trailing,
            bottom_leading,
            bottom_trailing,
            MessageBubbleTailSide::None,
            false,
        )
    }

    pub fn uniform(r: Pixels) -> Self {
        Self::radii(r, r, r, r)
    }

    pub fn inset_all(&self, inset: Pixels) -> Self {
        Self::new(
            (self.top_leading - inset).max(px(0.0)),
            (self.top_trailing - inset).max(px(0.0)),
            (self.bottom_leading - inset).max(px(0.0)),
            (self.bottom_trailing - inset).max(px(0.0)),
            self.tail_side,
            self.draws_tail,
        )
    }

    pub fn media_top_only(&self, inset: Pixels) -> Self {
        Self::new(
            (self.top_leading - inset).max(px(0.0)),
            (self.top_trailing - inset).max(px(0.0)),
            px(0.0),
            px(0.0),
            self.tail_side,
            self.draws_tail,
        )
    }

    pub fn media_middle(&self) -> Self {
        Self::new(px(0.0), px(0.0), px(0.0), px(0.0), self.tail_side, self.draws_tail)
    }

    pub fn media_bottom_only(&self, inset: Pixels) -> Self {
        Self::new(
            px(0.0),
            px(0.0),
            (self.bottom_leading - inset).max(px(0.0)),
            (self.bottom_trailing - inset).max(px(0.0)),
            self.tail_side,
            self.draws_tail,
        )
    }

    fn message_bubble_radii(&self) -> MessageBubbleRadii {
        MessageBubbleRadii {
            top_leading: self.top_leading,
            top_trailing: self.top_trailing,
            bottom_leading: self.bottom_leading,
            bottom_trailing: self.bottom_trailing,
        }
    }

    pub fn bubble_shape(&self) -> MessageBubbleShape {
        MessageBubbleShape {
            radii: self.message_bubble_radii(),
            tail_side: self.tail_side,
            draws_tail: self.draws_tail,
        }
    }

    pub fn circular_shape(&self) -> MessageBubbleShape {
        MessageBubbleShape {
            radii: self.message_bubble_radii(),
            tail_side: MessageBubbleTailSide::None,
            draws_tail: false,
        }
    }

    /// corner radii as a gpui corners value, order follows the layout direction
    pub fn uneven_shape(&self, layout_direction: LayoutDirection) -> Corners<Pixels> {
        if layout_direction.is_right_to_left() {
            Corners {
                top_left: self.top_trailing,
                top_right: self.top_leading,
                bottom_right: self.bottom_leading,
                bottom_left: self.bottom_trailing,
            }
        } else {
            Corners {
                top_left: self.top_leading,
                top_right: self.top_trailing,
                bottom_right: self.bottom_trailing,
                bottom_left: self.bottom_leading,
            }
        }
    }
}

/// light or dark verdict for a fill, shared by reaction chips and meta plate
pub enum TelegramReactionPalette {}

impl TelegramReactionPalette {
    pub fn is_dark_background(color: Hsla) -> bool {
        let rgba = color.to_rgb();
        rgba.r * 0.2126 + rgba.g * 0.7152 + rgba.b * 0.0722 < 0.55
    }
}
