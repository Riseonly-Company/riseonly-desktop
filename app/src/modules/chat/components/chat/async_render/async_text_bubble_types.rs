use std::sync::Arc;

use gpui::{Bounds, Font, Hsla, Pixels, Point, ShapedLine, SharedString, Size, TextRun, TextSystem, Window, point, px, size, white};
use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::ns_range::{NSRange, byte_to_utf16_range};
use rise_ui::message_bubble_geometry::{MessageBubbleGeometry, MessageBubbleTailSide};

use crate::core::lib::canon_format::RiseFormat;
use crate::modules::chat::components::chat::bubble::message_meta::MessageDeliveryStatusGeometry;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum AsyncTextBubbleDeliveryState {
    #[default]
    None,
    Sending,
    Sent,
    Delivered,
    Read,
    Failed,
}

impl AsyncTextBubbleDeliveryState {
    pub const fn raw_value(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Sending => "sending",
            Self::Sent => "sent",
            Self::Delivered => "delivered",
            Self::Read => "read",
            Self::Failed => "failed",
        }
    }

    pub fn from_raw(raw: &str) -> Option<Self> {
        match raw {
            "none" => Some(Self::None),
            "sending" => Some(Self::Sending),
            "sent" => Some(Self::Sent),
            "delivered" => Some(Self::Delivered),
            "read" => Some(Self::Read),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AsyncTextBubbleSeed {
    pub id: String,
    pub text: String,
    pub timestamp: Date,
    pub time_zone_identifier: String,
    pub twelve_hour_clock: bool,
    pub delivery_state: AsyncTextBubbleDeliveryState,
    pub view_count_text: Option<String>,
    pub highlight_ranges: Vec<NSRange>,
}

impl AsyncTextBubbleSeed {
    /// new seed with defaults: current time zone and device clock cycle, no delivery state no
    /// views no highlights
    /// builders below set the rest
    pub fn new(id: impl Into<String>, text: impl Into<String>, timestamp: Date) -> Self {
        Self {
            id: id.into(),
            text: text.into(),
            timestamp,
            time_zone_identifier: TimeZone::current().identifier(),
            twelve_hour_clock: RiseFormat::uses_twelve_hour_clock(),
            delivery_state: AsyncTextBubbleDeliveryState::None,
            view_count_text: None,
            highlight_ranges: Vec::new(),
        }
    }

    pub fn time_zone_identifier(mut self, time_zone_identifier: impl Into<String>) -> Self {
        self.time_zone_identifier = time_zone_identifier.into();
        self
    }

    pub fn twelve_hour_clock(mut self, twelve_hour_clock: bool) -> Self {
        self.twelve_hour_clock = twelve_hour_clock;
        self
    }

    pub fn delivery_state(mut self, delivery_state: AsyncTextBubbleDeliveryState) -> Self {
        self.delivery_state = delivery_state;
        self
    }

    pub fn view_count_text(mut self, view_count_text: Option<String>) -> Self {
        self.view_count_text = view_count_text;
        self
    }

    pub fn highlight_ranges(mut self, highlight_ranges: Vec<NSRange>) -> Self {
        self.highlight_ranges = highlight_ranges;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AsyncTextBubbleLayoutSpec {
    pub maximum_width: Pixels,
    pub minimum_width: Pixels,
    pub minimum_height: Pixels,
    pub horizontal_padding: Pixels,
    pub vertical_padding: Pixels,
    pub top_padding: Pixels,
    pub bottom_padding: Pixels,
    pub tail_side: MessageBubbleTailSide,
    pub tail_inset: Pixels,
    pub text_font_size: Pixels,
    pub time_font_size: Pixels,
    pub line_spacing: Pixels,
    pub inline_meta_spacing: Pixels,
    pub meta_top_spacing: Pixels,
    pub status_icon_size: Pixels,
    pub status_icon_spacing: Pixels,
}

impl Default for AsyncTextBubbleLayoutSpec {
    fn default() -> Self {
        Self::new(
            px(345.0),
            px(40.0),
            px(35.0),
            px(11.0),
            px(6.0),
            None,
            None,
            MessageBubbleTailSide::None,
            MessageBubbleGeometry::TAIL_INSET,
            px(17.0),
            px(10.0),
            px(2.0),
            px(8.0),
            px(3.0),
            px(10.0),
            px(3.0),
        )
    }
}

impl AsyncTextBubbleLayoutSpec {
    /// full initializer, top_padding and bottom_padding fall back to vertical_padding when none
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        maximum_width: Pixels,
        minimum_width: Pixels,
        minimum_height: Pixels,
        horizontal_padding: Pixels,
        vertical_padding: Pixels,
        top_padding: Option<Pixels>,
        bottom_padding: Option<Pixels>,
        tail_side: MessageBubbleTailSide,
        tail_inset: Pixels,
        text_font_size: Pixels,
        time_font_size: Pixels,
        line_spacing: Pixels,
        inline_meta_spacing: Pixels,
        meta_top_spacing: Pixels,
        status_icon_size: Pixels,
        status_icon_spacing: Pixels,
    ) -> Self {
        Self {
            maximum_width,
            minimum_width,
            minimum_height,
            horizontal_padding,
            vertical_padding,
            top_padding: top_padding.unwrap_or(vertical_padding),
            bottom_padding: bottom_padding.unwrap_or(vertical_padding),
            tail_side,
            tail_inset,
            text_font_size,
            time_font_size,
            line_spacing,
            inline_meta_spacing,
            meta_top_spacing,
            status_icon_size,
            status_icon_spacing,
        }
    }

    pub fn leading_content_inset(&self) -> Pixels {
        self.horizontal_padding
            + if self.tail_side == MessageBubbleTailSide::Leading {
                self.tail_inset
            } else {
                px(0.0)
            }
    }

    pub fn trailing_content_inset(&self) -> Pixels {
        self.horizontal_padding
            + if self.tail_side == MessageBubbleTailSide::Trailing {
                self.tail_inset
            } else {
                px(0.0)
            }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AsyncTextBubbleCorners {
    pub top_left: Pixels,
    pub top_right: Pixels,
    pub bottom_left: Pixels,
    pub bottom_right: Pixels,
    pub tail_side: MessageBubbleTailSide,
    pub draws_tail: bool,
}

impl AsyncTextBubbleCorners {
    pub fn new(
        top_left: Pixels,
        top_right: Pixels,
        bottom_left: Pixels,
        bottom_right: Pixels,
        tail_side: MessageBubbleTailSide,
        draws_tail: bool,
    ) -> Self {
        Self {
            top_left,
            top_right,
            bottom_left,
            bottom_right,
            tail_side,
            draws_tail,
        }
    }

    pub fn uniform(radius: Pixels) -> Self {
        Self::new(radius, radius, radius, radius, MessageBubbleTailSide::None, false)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AsyncTextBubbleFill {
    Solid(Hsla),
    LinearGradient {
        start: Hsla,
        end: Hsla,
        start_point: Point<Pixels>,
        end_point: Point<Pixels>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AsyncTextBubbleVisualStyle {
    pub fill: AsyncTextBubbleFill,
    pub text_color: Hsla,
    pub time_color: Hsla,
    pub status_color: Hsla,
    pub read_status_color: Hsla,
    pub failure_color: Hsla,
    pub failure_glyph_color: Hsla,
}

impl AsyncTextBubbleVisualStyle {
    /// default failure glyph color is white, failure_glyph_color sets another
    pub fn new(
        fill: AsyncTextBubbleFill,
        text_color: Hsla,
        time_color: Hsla,
        status_color: Hsla,
        read_status_color: Hsla,
        failure_color: Hsla,
    ) -> Self {
        Self {
            fill,
            text_color,
            time_color,
            status_color,
            read_status_color,
            failure_color,
            failure_glyph_color: white(),
        }
    }

    pub fn failure_glyph_color(mut self, failure_glyph_color: Hsla) -> Self {
        self.failure_glyph_color = failure_glyph_color;
        self
    }
}

/// one laid out line of bubble text, immutable and safe to build off the main thread
/// holds string range, typographic bounds caret offsets and the run data
#[derive(Clone, Debug)]
pub struct AsyncTextBubbleLine {
    pub text: SharedString,
    pub font: Font,
    pub font_size: Pixels,
    pub string_range: NSRange,
    pub ascent: Pixels,
    pub descent: Pixels,
    pub leading: Pixels,
    caret_offsets: Arc<[Pixels]>,
    trailing_whitespace_width: Pixels,
}

impl AsyncTextBubbleLine {
    /// makes a line for text which starts at utf-16 location of the measured string
    pub fn new(text: &str, location: usize, font: &Font, font_size: Pixels, text_system: &Arc<TextSystem>) -> Self {
        let font_id = text_system.resolve_font(font);
        let mut caret_offsets = Vec::with_capacity(text.chars().count() + 1);
        let mut x = px(0.0);
        caret_offsets.push(x);
        for character in text.chars() {
            x += text_system.layout_width(font_id, font_size, character);
            caret_offsets.push(x);
        }
        let trimmed = text.trim_end_matches(char::is_whitespace);
        let visible = caret_offsets[trimmed.chars().count()];
        Self {
            text: SharedString::from(text.to_owned()),
            font: font.clone(),
            font_size,
            string_range: NSRange::new(location, byte_to_utf16_range(text, 0..text.len()).length),
            ascent: text_system.ascent(font_id, font_size),
            descent: text_system.descent(font_id, font_size).abs(),
            leading: px(0.0),
            trailing_whitespace_width: x - visible,
            caret_offsets: caret_offsets.into(),
        }
    }

    pub fn typographic_width(&self) -> Pixels {
        self.caret_offsets.last().copied().unwrap_or(px(0.0))
    }

    /// x of the caret before utf-16 index of the measured string, clamped to the line
    pub fn offset_for_string_index(&self, index: usize) -> Pixels {
        let relative = index.saturating_sub(self.string_range.location);
        let mut utf16 = 0;
        for (position, character) in self.text.chars().enumerate() {
            if utf16 >= relative {
                return self.caret_offsets[position];
            }
            utf16 += character.len_utf16();
        }
        self.typographic_width()
    }

    /// where the pen starts so the line without trailing whitespace sits at flush (0 leading, 1
    /// trailing) of flush_width
    pub fn pen_offset_for_flush(&self, flush: f32, flush_width: Pixels) -> Pixels {
        let visible = self.typographic_width() - self.trailing_whitespace_width;
        ((flush_width - visible) * flush.clamp(0.0, 1.0)).max(px(0.0))
    }

    /// measured characters shaped for window in color, this is what gets drawn
    pub fn shape(&self, color: Hsla, window: &Window) -> ShapedLine {
        let runs = [TextRun {
            len: self.text.len(),
            font: self.font.clone(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        }];
        window.text_system().shape_line(self.text.clone(), self.font_size, &runs, None)
    }
}

#[derive(Clone, Debug)]
pub struct AsyncTextBubbleLinePlacement {
    pub line: AsyncTextBubbleLine,
    pub baseline_origin: Point<Pixels>,
    pub glyph_bounds: Bounds<Pixels>,
}

impl AsyncTextBubbleLinePlacement {
    pub fn new(line: AsyncTextBubbleLine, baseline_origin: Point<Pixels>) -> Self {
        let width = line.typographic_width().max(px(0.0));
        let glyph_bounds = Bounds::new(
            point(baseline_origin.x, baseline_origin.y - line.ascent),
            size(width, line.ascent + line.descent),
        );
        Self {
            line,
            baseline_origin,
            glyph_bounds,
        }
    }
}

/// immutable bubble geometry measured off the main thread, shared by Arc
#[derive(Clone, Debug)]
pub struct AsyncTextBubblePreparedLayout {
    pub seed: AsyncTextBubbleSeed,
    pub spec: AsyncTextBubbleLayoutSpec,
    pub size: Size<Pixels>,

    pub text_lines: Arc<[AsyncTextBubbleLinePlacement]>,
    pub maximum_line_ascent: Pixels,
    pub maximum_line_descent: Pixels,
    pub maximum_highlight_height: Pixels,
    pub time_line: AsyncTextBubbleLine,
    pub time_baseline_origin: Point<Pixels>,
    pub status_frame: Option<Bounds<Pixels>>,
    pub views_line: Option<AsyncTextBubbleLine>,
    pub views_baseline_origin: Point<Pixels>,
    pub views_icon_frame: Option<Bounds<Pixels>>,
    pub highlight_rects: Arc<[Bounds<Pixels>]>,
}

impl AsyncTextBubblePreparedLayout {
    /// defaults: no views run and origin zero, no icon and no highlights
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        seed: AsyncTextBubbleSeed,
        spec: AsyncTextBubbleLayoutSpec,
        size: Size<Pixels>,
        text_lines: Arc<[AsyncTextBubbleLinePlacement]>,
        time_line: AsyncTextBubbleLine,
        time_baseline_origin: Point<Pixels>,
        status_frame: Option<Bounds<Pixels>>,
        views_line: Option<AsyncTextBubbleLine>,
        views_baseline_origin: Point<Pixels>,
        views_icon_frame: Option<Bounds<Pixels>>,
        highlight_rects: Vec<Bounds<Pixels>>,
    ) -> Self {
        let maximum_line_ascent = text_lines.iter().fold(px(0.0), |maximum, placement| {
            maximum.max(placement.baseline_origin.y - placement.glyph_bounds.top())
        });
        let maximum_line_descent = text_lines.iter().fold(px(0.0), |maximum, placement| {
            maximum.max(placement.glyph_bounds.bottom() - placement.baseline_origin.y)
        });
        let maximum_highlight_height = highlight_rects
            .iter()
            .fold(px(0.0), |maximum, rect| maximum.max(rect.size.height));
        let mut highlight_rects = highlight_rects;
        highlight_rects.sort_by(|lhs, rhs| lhs.top().cmp(&rhs.top()));
        Self {
            seed,
            spec,
            size,
            text_lines,
            maximum_line_ascent,
            maximum_line_descent,
            maximum_highlight_height,
            time_line,
            time_baseline_origin,
            status_frame,
            views_line,
            views_baseline_origin,
            views_icon_frame,
            highlight_rects: highlight_rects.into(),
        }
    }

    pub fn reusing_geometry(&self, next_seed: &AsyncTextBubbleSeed) -> Option<AsyncTextBubblePreparedLayout> {
        let status_slot = MessageDeliveryStatusGeometry::slot_size(self.seed.delivery_state, self.spec.status_icon_size);
        let next_status_slot = MessageDeliveryStatusGeometry::slot_size(next_seed.delivery_state, self.spec.status_icon_size);
        if self.seed.text != next_seed.text
            || self.seed.timestamp != next_seed.timestamp
            || self.seed.time_zone_identifier != next_seed.time_zone_identifier
            || self.seed.view_count_text != next_seed.view_count_text
            || self.seed.highlight_ranges != next_seed.highlight_ranges
            || status_slot != next_status_slot
        {
            return None;
        }
        let next_status_frame = self.status_frame.map(|frame| {
            let glyph = MessageDeliveryStatusGeometry::size(next_seed.delivery_state, self.spec.status_icon_size);
            Bounds::new(point(frame.origin.x, frame.center().y - glyph.height * 0.5), glyph)
        });
        Some(Self {
            seed: next_seed.clone(),
            spec: self.spec,
            size: self.size,
            text_lines: Arc::clone(&self.text_lines),
            maximum_line_ascent: self.maximum_line_ascent,
            maximum_line_descent: self.maximum_line_descent,
            maximum_highlight_height: self.maximum_highlight_height,
            time_line: self.time_line.clone(),
            time_baseline_origin: self.time_baseline_origin,
            status_frame: next_status_frame,
            views_line: self.views_line.clone(),
            views_baseline_origin: self.views_baseline_origin,
            views_icon_frame: self.views_icon_frame,
            highlight_rects: Arc::clone(&self.highlight_rects),
        })
    }
}
