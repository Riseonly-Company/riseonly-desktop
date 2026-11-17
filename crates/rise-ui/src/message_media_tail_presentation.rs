use std::hash::{Hash, Hasher};

use gpui::{Background, Bounds, Hsla, Pixels, RenderImage, Rgba, Window, point, px, size};

use crate::message_bubble_geometry::{
    MessageBubbleGeometry, MessageBubblePath, MessageBubbleRadii, MessageBubbleTailSide,
};
use crate::swiftui::animated::{Components, Lerp};

fn unit(value: f32) -> f32 {
    1.0f32.min(0.0f32.max(value)) + 0.0
}

/// straight alpha color sampled from the media edge beside a bubble tail
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MessageMediaTailColor {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
}

impl Eq for MessageMediaTailColor {}

impl Hash for MessageMediaTailColor {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.red.to_bits().hash(state);
        self.green.to_bits().hash(state);
        self.blue.to_bits().hash(state);
        self.alpha.to_bits().hash(state);
    }
}

impl MessageMediaTailColor {
    pub fn new(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self {
            red: unit(red),
            green: unit(green),
            blue: unit(blue),
            alpha: unit(alpha),
        }
    }

    pub fn opaque(red: f32, green: f32, blue: f32) -> Self {
        Self::new(red, green, blue, 1.0)
    }

    pub fn rgba(&self) -> Rgba {
        Rgba {
            r: self.red,
            g: self.green,
            b: self.blue,
            a: self.alpha,
        }
    }

    pub fn color(&self) -> Hsla {
        self.rgba().into()
    }

    pub fn darkened(&self, by_black_overlay_opacity: f32) -> Self {
        let multiplier = 1.0 - 1.0f32.min(0.0f32.max(by_black_overlay_opacity));
        Self::new(
            self.red * multiplier,
            self.green * multiplier,
            self.blue * multiplier,
            self.alpha,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MessageMediaTailEdgeColors {
    pub leading: MessageMediaTailColor,
    pub trailing: MessageMediaTailColor,
}

impl MessageMediaTailEdgeColors {
    pub fn color(&self, side: MessageBubbleTailSide) -> Option<MessageMediaTailColor> {
        match side {
            MessageBubbleTailSide::Leading => Some(self.leading),
            MessageBubbleTailSide::Trailing => Some(self.trailing),
            MessageBubbleTailSide::None => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MessageMediaTailPresentation {
    pub side: MessageBubbleTailSide,
    pub color: MessageMediaTailColor,
    pub radii: MessageBubbleRadii,
}

pub enum MessageMediaTailGeometry {}

impl MessageMediaTailGeometry {
    pub const LEADING_EXTENSION_WIDTH: Pixels = px(4.0);
    pub const TRAILING_EXTENSION_WIDTH: Pixels = px(5.0);
    pub const MASK_REFERENCE_HEIGHT: Pixels = px(8.0);
    pub const CAP_HEIGHT: Pixels = px(7.0);
    pub const TERMINAL_OVERLAY_BLACK_OPACITY: f32 = 0.34;

    pub fn extension_width(side: MessageBubbleTailSide) -> Pixels {
        match side {
            MessageBubbleTailSide::Leading => Self::LEADING_EXTENSION_WIDTH,
            MessageBubbleTailSide::Trailing => Self::TRAILING_EXTENSION_WIDTH,
            MessageBubbleTailSide::None => px(0.0),
        }
    }

    pub fn eligible_side(
        draws_tail: bool,
        tail_side: MessageBubbleTailSide,
        bottom_leading_radius: Pixels,
        bottom_trailing_radius: Pixels,
    ) -> MessageBubbleTailSide {
        if !draws_tail {
            return MessageBubbleTailSide::None;
        }
        match tail_side {
            MessageBubbleTailSide::Leading if bottom_leading_radius >= px(14.0) => {
                MessageBubbleTailSide::Leading
            }
            MessageBubbleTailSide::Trailing if bottom_trailing_radius >= px(14.0) => {
                MessageBubbleTailSide::Trailing
            }
            _ => MessageBubbleTailSide::None,
        }
    }

    /// media sized slice of the tail corner
    /// normal corner stays at the given media radius and only the adjacent tail asset goes beyond
    /// the image bounds
    pub fn content_mask_path(
        media_bounds: Bounds<Pixels>,
        radii: MessageBubbleRadii,
        tail_side: MessageBubbleTailSide,
    ) -> MessageBubblePath {
        if tail_side == MessageBubbleTailSide::None {
            return MessageBubbleGeometry::path(
                media_bounds,
                radii,
                MessageBubbleTailSide::None,
                false,
            );
        }
        Self::tail_bubble_path(media_bounds, radii, tail_side)
    }

    /// cap path in local coordinates of the tiny side specific fill 4 x 7 leading or 5 x 7 trailing
    /// through the eight point reference mask
    pub fn cap_path_for_media(
        media_bounds: Bounds<Pixels>,
        radii: MessageBubbleRadii,
        side: MessageBubbleTailSide,
    ) -> MessageBubblePath {
        if side == MessageBubbleTailSide::None {
            return MessageBubblePath::new();
        }
        let cap_width = Self::extension_width(side);
        let cap_min_x = if side == MessageBubbleTailSide::Leading {
            media_bounds.left() - cap_width
        } else {
            media_bounds.right()
        };
        let cap_min_y = media_bounds.bottom() - Self::CAP_HEIGHT;
        Self::tail_bubble_path(media_bounds, radii, side).scaled_and_translated(
            1.0,
            1.0,
            -cap_min_x,
            -cap_min_y,
        )
    }

    /// canonical terminal cap used by the media viewer hero
    /// terminal media corners are always the full 16 point grouped corners, none for radii is that default
    pub fn cap_path(
        rect: Bounds<Pixels>,
        side: MessageBubbleTailSide,
        radii: Option<MessageBubbleRadii>,
    ) -> MessageBubblePath {
        if side == MessageBubbleTailSide::None
            || rect.size.width <= px(0.0)
            || rect.size.height <= px(0.0)
        {
            return MessageBubblePath::new();
        }
        let radii = radii.unwrap_or(MessageBubbleRadii::uniform(px(16.0)));
        let media_bounds = Bounds::new(point(px(0.0), px(0.0)), size(px(200.0), px(100.0)));
        let local_path = Self::cap_path_for_media(media_bounds, radii, side);
        let cap_width = Self::extension_width(side);
        local_path.scaled_and_translated(
            rect.size.width / cap_width,
            rect.size.height / Self::CAP_HEIGHT,
            rect.left(),
            rect.top(),
        )
    }

    fn tail_bubble_path(
        media_bounds: Bounds<Pixels>,
        radii: MessageBubbleRadii,
        tail_side: MessageBubbleTailSide,
    ) -> MessageBubblePath {
        let tail_allowance = MessageBubbleGeometry::TAIL_INSET;
        let bubble_bounds = match tail_side {
            MessageBubbleTailSide::Leading => Bounds::new(
                point(media_bounds.left() - tail_allowance, media_bounds.top()),
                size(media_bounds.size.width + tail_allowance, media_bounds.size.height),
            ),
            MessageBubbleTailSide::Trailing => Bounds::new(
                media_bounds.origin,
                size(media_bounds.size.width + tail_allowance, media_bounds.size.height),
            ),
            MessageBubbleTailSide::None => {
                return MessageBubbleGeometry::path(
                    media_bounds,
                    radii,
                    MessageBubbleTailSide::None,
                    false,
                );
            }
        };
        MessageBubbleGeometry::path(bubble_bounds, radii, tail_side, true)
    }
}

fn paint_contour(path: MessageBubblePath, fill: impl Into<Background>, window: &mut Window) {
    if let Some(path) = path.fill_path() {
        window.paint_path(path, fill);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MessageMediaTailCapShape {
    pub side: MessageBubbleTailSide,
    pub radii: MessageBubbleRadii,
}

impl MessageMediaTailCapShape {
    pub fn new(side: MessageBubbleTailSide) -> Self {
        Self {
            side,
            radii: MessageBubbleRadii::uniform(px(16.0)),
        }
    }

    pub fn path(&self, rect: Bounds<Pixels>) -> MessageBubblePath {
        MessageMediaTailGeometry::cap_path(rect, self.side, Some(self.radii))
    }

    pub fn paint(&self, rect: Bounds<Pixels>, fill: impl Into<Background>, window: &mut Window) {
        paint_contour(self.path(rect), fill, window);
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MessageMediaContentShape {
    pub radii: MessageBubbleRadii,
    pub tail_side: MessageBubbleTailSide,
}

impl MessageMediaContentShape {
    pub fn path(&self, rect: Bounds<Pixels>) -> MessageBubblePath {
        MessageMediaTailGeometry::content_mask_path(rect, self.radii, self.tail_side)
    }

    pub fn paint(&self, rect: Bounds<Pixels>, fill: impl Into<Background>, window: &mut Window) {
        paint_contour(self.path(rect), fill, window);
    }
}

/// the hero clip, its four radii are its animatable data
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MessageMediaHeroClipShape {
    pub radii: MessageBubbleRadii,
}

impl Lerp for MessageMediaHeroClipShape {
    fn components(&self) -> Components {
        smallvec::smallvec![
            self.radii.top_leading.as_f32(),
            self.radii.top_trailing.as_f32(),
            self.radii.bottom_leading.as_f32(),
            self.radii.bottom_trailing.as_f32(),
        ]
    }

    fn from_components(components: &[f32]) -> Self {
        let at = |index: usize| px(components.get(index).copied().unwrap_or(0.0));
        Self {
            radii: MessageBubbleRadii {
                top_leading: at(0),
                top_trailing: at(1),
                bottom_leading: at(2),
                bottom_trailing: at(3),
            },
        }
    }
}

impl MessageMediaHeroClipShape {
    pub fn path(&self, rect: Bounds<Pixels>) -> MessageBubblePath {
        MessageBubbleGeometry::path(rect, self.radii, MessageBubbleTailSide::None, false)
    }

    pub fn paint(&self, rect: Bounds<Pixels>, fill: impl Into<Background>, window: &mut Window) {
        paint_contour(self.path(rect), fill, window);
    }
}

/// decoded picture the sampler reads, pixels are four bytes each straight alpha
/// in blue green red alpha order as gpui decodes them
#[derive(Clone, Copy, Debug)]
pub struct MessageMediaTailBitmap<'a> {
    pub width: usize,
    pub height: usize,
    pub bgra: &'a [u8],
}

impl<'a> MessageMediaTailBitmap<'a> {
    pub fn new(width: usize, height: usize, bgra: &'a [u8]) -> Option<Self> {
        let needed = width.checked_mul(height)?.checked_mul(4)?;
        (width > 0 && height > 0 && bgra.len() >= needed).then_some(Self {
            width,
            height,
            bgra,
        })
    }

    pub fn from_render_image(image: &'a RenderImage) -> Option<Self> {
        let frame = image.size(0);
        let bytes = image.as_bytes(0)?;
        Self::new(
            usize::try_from(frame.width.0).ok()?,
            usize::try_from(frame.height.0).ok()?,
            bytes,
        )
    }

    fn pixel(&self, x: usize, y: usize) -> Option<[u8; 4]> {
        let start = (y * self.width + x) * 4;
        let bytes = self.bgra.get(start..start + 4)?;
        Some([bytes[2], bytes[1], bytes[0], bytes[3]])
    }
}

/// a point or size in the sampler own target space, in points
#[derive(Clone, Copy, Debug, PartialEq)]
struct SampleRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl SampleRect {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

pub enum MessageMediaTailColorSampler {}

impl MessageMediaTailColorSampler {
    const BACKDROP_SCALE: f64 = 1.08;
    const BACKDROP_BLACK_OVERLAY_OPACITY: f32 = 0.24;

    pub fn visible_bottom_edge_colors(
        foreground: MessageMediaTailBitmap<'_>,
        backdrop: Option<MessageMediaTailBitmap<'_>>,
        target_width: f64,
        target_height: f64,
        uses_fit_foreground: bool,
    ) -> Option<MessageMediaTailEdgeColors> {
        if !(target_width >= 2.0 && target_height >= 2.0) {
            return None;
        }
        let target = (target_width, target_height);
        let leading = Self::visible_color(
            (0.0, target_height - 1.0),
            foreground,
            backdrop,
            target,
            uses_fit_foreground,
        )?;
        let trailing = Self::visible_color(
            (target_width - 1.0, target_height - 1.0),
            foreground,
            backdrop,
            target,
            uses_fit_foreground,
        )?;
        Some(MessageMediaTailEdgeColors { leading, trailing })
    }

    fn clear() -> MessageMediaTailColor {
        MessageMediaTailColor::new(0.0, 0.0, 0.0, 0.0)
    }

    fn black() -> MessageMediaTailColor {
        MessageMediaTailColor::opaque(0.0, 0.0, 0.0)
    }

    fn visible_color(
        at: (f64, f64),
        foreground: MessageMediaTailBitmap<'_>,
        backdrop: Option<MessageMediaTailBitmap<'_>>,
        target: (f64, f64),
        uses_fit_foreground: bool,
    ) -> Option<MessageMediaTailColor> {
        if uses_fit_foreground {
            let fit_rect = Self::aspect_rect(foreground, target, false);
            let backdrop_color = Self::visible_backdrop_color(at, backdrop, target);
            if !fit_rect.contains(at.0, at.1) {
                return Some(backdrop_color);
            }
            let foreground_color = Self::sample(foreground, fit_rect, at).unwrap_or_else(Self::clear);
            return Some(Self::composite(foreground_color, backdrop_color));
        }
        let fill_rect = Self::aspect_rect(foreground, target, true);
        let foreground_color = Self::sample(foreground, fill_rect, at).unwrap_or_else(Self::clear);
        Some(Self::composite(foreground_color, Self::black()))
    }

    fn visible_backdrop_color(
        at: (f64, f64),
        backdrop: Option<MessageMediaTailBitmap<'_>>,
        target: (f64, f64),
    ) -> MessageMediaTailColor {
        let Some(backdrop) = backdrop else {
            return Self::black();
        };
        let center = (target.0 * 0.5, target.1 * 0.5);
        let unscaled = (
            center.0 + (at.0 - center.0) / Self::BACKDROP_SCALE,
            center.1 + (at.1 - center.1) / Self::BACKDROP_SCALE,
        );
        let fill_rect = Self::aspect_rect(backdrop, target, true);
        let sampled = Self::sample(backdrop, fill_rect, unscaled).unwrap_or_else(Self::clear);
        Self::composite(sampled, Self::black()).darkened(Self::BACKDROP_BLACK_OVERLAY_OPACITY)
    }

    fn composite(
        foreground: MessageMediaTailColor,
        background: MessageMediaTailColor,
    ) -> MessageMediaTailColor {
        let alpha = 1.0f32.min(0.0f32.max(foreground.alpha));
        MessageMediaTailColor::new(
            foreground.red * alpha + background.red * (1.0 - alpha),
            foreground.green * alpha + background.green * (1.0 - alpha),
            foreground.blue * alpha + background.blue * (1.0 - alpha),
            1.0,
        )
    }

    fn aspect_rect(
        image: MessageMediaTailBitmap<'_>,
        target: (f64, f64),
        uses_fill: bool,
    ) -> SampleRect {
        let image_width = image.width as f64;
        let image_height = image.height as f64;
        let width_scale = target.0 / image_width.max(1.0);
        let height_scale = target.1 / image_height.max(1.0);
        let scale = if uses_fill {
            width_scale.max(height_scale)
        } else {
            width_scale.min(height_scale)
        };
        let width = image_width * scale;
        let height = image_height * scale;
        SampleRect {
            x: (target.0 - width) * 0.5,
            y: (target.1 - height) * 0.5,
            width,
            height,
        }
    }

    fn sample(
        image: MessageMediaTailBitmap<'_>,
        display_rect: SampleRect,
        at: (f64, f64),
    ) -> Option<MessageMediaTailColor> {
        if !(display_rect.width > 0.0 && display_rect.height > 0.0) {
            return None;
        }
        let unit_x = (at.0 - display_rect.x) / display_rect.width;
        let unit_y = (at.1 - display_rect.y) / display_rect.height;
        let pixel_x = Self::pixel_index(unit_x, image.width);
        let pixel_y = Self::pixel_index(unit_y, image.height);
        let [red, green, blue, alpha] = image.pixel(pixel_x, pixel_y)?;
        let alpha = f32::from(alpha) / 255.0;
        if alpha <= 0.0 {
            return None;
        }
        Some(MessageMediaTailColor::new(
            f32::from(red) / 255.0,
            f32::from(green) / 255.0,
            f32::from(blue) / 255.0,
            alpha,
        ))
    }

    fn pixel_index(unit: f64, extent: usize) -> usize {
        let raw = (unit * extent as f64).floor();
        let last = extent.saturating_sub(1);
        if raw.is_nan() || raw <= 0.0 {
            0
        } else if raw >= last as f64 {
            last
        } else {
            raw as usize
        }
    }
}

#[cfg(test)]
#[path = "message_media_tail_presentation_tests.rs"]
mod tests;
