use std::sync::Arc;

use gpui::{Bounds, Font, LineFragment, Pixels, TextSystem, point, px, size};
use rise_core::foundation::ns_range::{NSRange, byte_to_utf16_offset};
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_ui::swiftui::font::Font as SwiftUiFontSpec;

use crate::core::lib::canon_format::RiseFormat;
use crate::modules::chat::components::chat::async_render::async_text_bubble_types::{
    AsyncTextBubbleLayoutSpec, AsyncTextBubbleLine, AsyncTextBubbleLinePlacement, AsyncTextBubblePreparedLayout,
    AsyncTextBubbleSeed, first_strong_is_rtl,
};
use crate::modules::chat::components::chat::bubble::message_meta::MessageDeliveryStatusGeometry;
use crate::modules::chat::shared::chat_search_highlight::ChatSearchHighlightPalette;

pub enum AsyncTextBubblePreparer {}

pub(crate) struct MeasuredLine {
    pub(crate) placement: AsyncTextBubbleLinePlacement,
    pub(crate) width: f32,
}

pub(crate) struct MeasuredText {
    pub(crate) lines: Vec<MeasuredLine>,
    pub(crate) width: f32,
    pub(crate) height: f32,
}

impl AsyncTextBubblePreparer {
    /// measures off the main thread, text_system is the shared app one
    pub async fn prepare(
        seed: AsyncTextBubbleSeed,
        spec: AsyncTextBubbleLayoutSpec,
        text_system: Arc<TextSystem>,
    ) -> AsyncTextBubblePreparedLayout {
        let fallback = (seed.clone(), Arc::clone(&text_system));
        let handle = RiseAccountRuntime::current()
            .spawn_blocking(move || Self::prepare_synchronously(seed, spec, &text_system));
        match handle.await {
            Ok(layout) => layout,
            Err(_) => Self::prepare_synchronously(fallback.0, spec, &fallback.1),
        }
    }

    pub fn prepare_synchronously(
        seed: AsyncTextBubbleSeed,
        spec: AsyncTextBubbleLayoutSpec,
        text_system: &Arc<TextSystem>,
    ) -> AsyncTextBubblePreparedLayout {
        let leading_inset = f32::from(spec.leading_content_inset());
        let trailing_inset = f32::from(spec.trailing_content_inset());
        let top_padding = f32::from(spec.top_padding);
        let status_icon_spacing = f32::from(spec.status_icon_spacing);
        let maximum_content_width = (f32::from(spec.maximum_width) - leading_inset - trailing_inset).max(1.0);
        let text_font = Self::system_font();
        let text_font_size = px(f32::from(spec.text_font_size).max(1.0));
        let time_font_size = px(f32::from(spec.time_font_size).max(1.0));
        let measured_text = Self::measure_text(
            &seed.text,
            &text_font,
            text_font_size,
            maximum_content_width,
            f32::from(spec.line_spacing),
            text_system,
        );
        let time_text = Self::formatted_time(&seed);
        let time_line = Self::make_line(&time_text, &text_font, time_font_size, text_system);

        let time_ascent = f32::from(time_line.ascent);
        let time_descent = f32::from(time_line.descent);
        let time_width = f32::from(time_line.typographic_width()).ceil();
        let time_height = (time_ascent + time_descent).ceil();

        let status_size = MessageDeliveryStatusGeometry::size(seed.delivery_state, spec.status_icon_size);
        let status_height_value = f32::from(status_size.height);
        let status_slot = MessageDeliveryStatusGeometry::slot_size(seed.delivery_state, spec.status_icon_size);
        let status_slot_width = f32::from(status_slot.width);
        let status_slot_height = f32::from(status_slot.height);
        let has_status = status_slot_width != 0.0 || status_slot_height != 0.0;
        let status_width = if has_status {
            status_slot_width + status_icon_spacing
        } else {
            0.0
        };

        let views_line = seed
            .view_count_text
            .as_deref()
            .map(|views| Self::make_line(views, &text_font, time_font_size, text_system));
        let views_width = views_line
            .as_ref()
            .map_or(0.0, |line| f32::from(line.typographic_width()).ceil());
        let views_icon_size = if views_line.is_none() {
            0.0
        } else {
            f32::from(spec.status_icon_size)
        };
        let views_run_width = if views_line.is_none() {
            0.0
        } else {
            views_icon_size + status_icon_spacing + views_width + status_icon_spacing * 2.0
        };

        let meta_width = views_run_width + time_width + status_width;
        let inline_meta_spacing = f32::from(spec.inline_meta_spacing);
        let last_text_line = measured_text.lines.last();
        let last_text_extent = last_text_line.map_or(0.0, |line| {
            Self::resolved_origin_x(line, maximum_content_width, measured_text.width) + line.width
        });
        let places_meta_inline = last_text_line.is_some()
            && last_text_extent + inline_meta_spacing + meta_width <= maximum_content_width;

        let content_width;
        let meta_baseline_y;
        let content_height;

        match last_text_line {
            Some(last_text_line) if places_meta_inline => {
                content_width = measured_text.width.max(last_text_extent + inline_meta_spacing + meta_width);
                meta_baseline_y = f32::from(last_text_line.placement.baseline_origin.y);
                let meta_top = meta_baseline_y - time_ascent;
                let icon_top = meta_top + (time_height - status_slot_height) * 0.5;
                content_height = measured_text
                    .height
                    .max((meta_baseline_y + time_descent).max(icon_top + status_slot_height));
            }
            _ => {
                content_width = measured_text.width.max(meta_width);
                let text_bottom = if measured_text.lines.is_empty() {
                    0.0
                } else {
                    measured_text.height
                };
                let meta_top_spacing = if measured_text.lines.is_empty() {
                    0.0
                } else {
                    f32::from(spec.meta_top_spacing)
                };
                meta_baseline_y = text_bottom + meta_top_spacing + time_ascent;
                let meta_top = meta_baseline_y - time_ascent;
                let icon_top = meta_top + (time_height - status_slot_height) * 0.5;
                content_height = measured_text
                    .height
                    .max((meta_baseline_y + time_descent).max(icon_top + status_slot_height));
            }
        }

        let natural_width = (content_width + leading_inset + trailing_inset).ceil();
        let natural_height = (content_height + top_padding + f32::from(spec.bottom_padding)).ceil();
        let bubble_size = size(
            px(f32::from(spec.minimum_width).max(natural_width)),
            px(f32::from(spec.minimum_height).max(natural_height)),
        );
        let resolved_content_width = f32::from(bubble_size.width) - leading_inset - trailing_inset;

        let text_lines: Vec<AsyncTextBubbleLinePlacement> = measured_text
            .lines
            .iter()
            .map(|measured| {
                AsyncTextBubbleLinePlacement::new(
                    measured.placement.line.clone(),
                    point(
                        px(leading_inset
                            + Self::resolved_origin_x(measured, maximum_content_width, measured_text.width)),
                        px(top_padding) + measured.placement.baseline_origin.y,
                    ),
                )
            })
            .collect();
        let meta_origin_x = leading_inset + resolved_content_width - meta_width;
        let time_baseline_origin = point(
            px(meta_origin_x + views_run_width),
            px(top_padding + meta_baseline_y),
        );
        let meta_top_y = top_padding + meta_baseline_y - time_ascent;
        let views_icon_frame = views_line.as_ref().map(|_| {
            Bounds::new(
                point(
                    px(meta_origin_x),
                    px(meta_top_y + (time_height - views_icon_size) * 0.5),
                ),
                size(px(views_icon_size), px(views_icon_size)),
            )
        });
        let views_baseline_origin = point(
            px(meta_origin_x + views_icon_size + status_icon_spacing),
            px(top_padding + meta_baseline_y),
        );

        let status_frame = if has_status {
            let meta_top = f32::from(time_baseline_origin.y) - time_ascent;
            Some(Bounds::new(
                point(
                    time_baseline_origin.x + px(time_width + status_icon_spacing),
                    px(meta_top + (time_height - status_height_value) * 0.5),
                ),
                status_size,
            ))
        } else {
            None
        };

        let highlight_rects = Self::highlight_rects(&seed.highlight_ranges, &text_lines);
        AsyncTextBubblePreparedLayout::new(
            seed,
            spec,
            bubble_size,
            text_lines.into(),
            time_line,
            time_baseline_origin,
            status_frame,
            views_line,
            views_baseline_origin,
            views_icon_frame,
            highlight_rects,
        )
    }

    /// one plate per matched word per line, clipped to the line own string range so a wrapped word
    /// gets a plate on each
    /// indices are utf-16 like the server ranges
    fn highlight_rects(ranges: &[NSRange], text_lines: &[AsyncTextBubbleLinePlacement]) -> Vec<Bounds<Pixels>> {
        if ranges.is_empty() {
            return Vec::new();
        }
        let horizontal_padding = ChatSearchHighlightPalette::HORIZONTAL_PADDING;
        let vertical_padding = ChatSearchHighlightPalette::VERTICAL_PADDING;
        let mut rects = Vec::new();
        for placement in text_lines {
            let line = &placement.line;
            let line_start = line.string_range.location;
            let line_end = line.string_range.location + line.string_range.length;
            let ascent = f32::from(line.ascent);
            let descent = f32::from(line.descent);
            for range in ranges {
                let start = range.location.max(line_start);
                let end = (range.location + range.length).min(line_end);
                if end <= start {
                    continue;
                }
                let start_x = f32::from(line.offset_for_string_index(start));
                let end_x = f32::from(line.offset_for_string_index(end));
                let min_x = start_x.min(end_x);
                let width = (end_x - start_x).abs();
                if width <= 0.5 {
                    continue;
                }
                rects.push(Bounds::new(
                    point(
                        placement.baseline_origin.x + px(min_x - horizontal_padding),
                        placement.baseline_origin.y - px(ascent + vertical_padding),
                    ),
                    size(
                        px(width + horizontal_padding * 2.0),
                        px(ascent + descent + vertical_padding * 2.0),
                    ),
                ));
            }
        }
        rects
    }

    fn resolved_origin_x(measured_line: &MeasuredLine, frame_width: f32, text_width: f32) -> f32 {
        let original_x = f32::from(measured_line.placement.baseline_origin.x);
        if original_x <= 0.0 {
            return original_x;
        }
        let line = &measured_line.placement.line;
        let maximum_offset = f32::from(line.pen_offset_for_flush(1.0, px(frame_width)));
        if maximum_offset <= 0.0 {
            return 0.0;
        }
        let flush = (original_x / maximum_offset).clamp(0.0, 1.0);
        f32::from(line.pen_offset_for_flush(flush, px(text_width)))
    }

    /// measures text at maximum_width, hard line breaks start a paragraph and each paragraph wraps
    /// at word boundaries
    /// baselines step by line height plus line_spacing per line index
    pub(crate) fn measure_text(
        text: &str,
        font: &Font,
        font_size: Pixels,
        maximum_width: f32,
        line_spacing: f32,
        text_system: &Arc<TextSystem>,
    ) -> MeasuredText {
        if text.is_empty() {
            return MeasuredText {
                lines: Vec::new(),
                width: 0.0,
                height: 0.0,
            };
        }

        let mut ranges: Vec<(usize, usize, Option<bool>)> = Vec::new();
        {
            let mut wrapper = text_system.line_wrapper(font.clone(), font_size);
            let mut paragraph_start = 0;
            let paragraphs: Vec<&str> = text.split('\n').collect();
            let paragraph_count = paragraphs.len();
            for (index, paragraph) in paragraphs.into_iter().enumerate() {
                if index + 1 == paragraph_count && paragraph.is_empty() && paragraph_count > 1 {
                    break;
                }
                let fragments = [LineFragment::text(paragraph)];
                let paragraph_is_rtl = first_strong_is_rtl(paragraph);
                let mut line_start = 0;
                for boundary in wrapper.wrap_line(&fragments, px(maximum_width)) {
                    ranges.push((paragraph_start + line_start, paragraph_start + boundary.ix, paragraph_is_rtl));
                    line_start = boundary.ix;
                }
                ranges.push((paragraph_start + line_start, paragraph_start + paragraph.len(), paragraph_is_rtl));
                paragraph_start += paragraph.len() + 1;
            }
        }
        if ranges.is_empty() {
            return MeasuredText {
                lines: Vec::new(),
                width: 0.0,
                height: 0.0,
            };
        }

        let mut lines = Vec::with_capacity(ranges.len());
        let mut maximum_line_width: f32 = 0.0;
        let mut measured_height: f32 = 0.0;

        for (index, (start, end, paragraph_is_rtl)) in ranges.into_iter().enumerate() {
            let line_text = text.get(start..end).unwrap_or_default();
            let line = AsyncTextBubbleLine::new(
                line_text,
                byte_to_utf16_offset(text, start),
                font,
                font_size,
                text_system,
            )
            .in_paragraph(paragraph_is_rtl);
            let ascent = f32::from(line.ascent);
            let descent = f32::from(line.descent);
            let line_height = ascent + descent + f32::from(line.leading);
            let width = f32::from(line.typographic_width()).ceil();
            let baseline_y = ascent + line_height * index as f32 + line_spacing * index as f32;
            let placement = AsyncTextBubbleLinePlacement::new(line, point(px(0.0), px(baseline_y)));
            lines.push(MeasuredLine { placement, width });
            maximum_line_width = maximum_line_width.max(width);
            measured_height = measured_height.max(baseline_y + descent);
        }

        MeasuredText {
            lines,
            width: maximum_width.min(maximum_line_width),
            height: measured_height.ceil(),
        }
    }

    pub(crate) fn make_line(string: &str, font: &Font, font_size: Pixels, text_system: &Arc<TextSystem>) -> AsyncTextBubbleLine {
        AsyncTextBubbleLine::new(string, 0, font, font_size, text_system)
    }

    /// system font of the kit, regular
    pub(crate) fn system_font() -> Font {
        let spec = SwiftUiFontSpec::system_size(0.0);
        Font {
            family: spec.family(),
            features: spec.features(),
            fallbacks: None,
            weight: spec.weight,
            style: spec.style(),
        }
    }

    fn formatted_time(seed: &AsyncTextBubbleSeed) -> String {
        RiseFormat::clock_in_time_zone(&seed.timestamp, &seed.time_zone_identifier, seed.twelve_hour_clock)
    }
}
