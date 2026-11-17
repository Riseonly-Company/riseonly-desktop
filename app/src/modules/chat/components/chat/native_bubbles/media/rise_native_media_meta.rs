use gpui::{App, Bounds, Pixels, Size, bounds, point, px, size};
use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_date::Date;
use rise_i18n::tr;

use crate::core::lib::canon_format::RiseFormat;
use crate::modules::chat::components::chat::async_render::async_text_bubble_preparer::AsyncTextBubblePreparer;
use crate::modules::chat::components::chat::async_render::async_text_bubble_types::{
    AsyncTextBubbleDeliveryState, AsyncTextBubbleLine,
};
use crate::modules::chat::components::chat::bubble::message_meta::MessageDeliveryStatusGeometry;
use crate::modules::chat::components::chat::native_bubbles::media::rise_native_media_layout::RiseNativeMediaLayout;
use crate::modules::chat::components::chat::native_bubbles::sticker::native_interactive_message_cell::NativeMessageMetaView;
use crate::modules::chat::components::chat::native_bubbles::voice::native_voice_note_kit::NativeVoiceNoteMetrics;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RiseNativeMediaMetaModelStyle {
    Inline,
    Overlay,
}

#[derive(Clone, Debug)]
pub struct RiseNativeMediaMetaModelRun {
    pub line: AsyncTextBubbleLine,
    pub frame: Bounds<Pixels>,
    pub ascender: f32,
}

/// meta measured once, a row of edited mark and time and pin and delivery slot
/// each centred, texts use the meta size plus the text size delta rounded up to pixel
#[derive(Clone, Debug)]
pub struct RiseNativeMediaMetaModel {
    pub style: RiseNativeMediaMetaModelStyle,
    pub is_own: bool,
    pub edited_text: Option<String>,
    pub time_text: String,
    pub edited: Option<RiseNativeMediaMetaModelRun>,
    pub time: RiseNativeMediaMetaModelRun,
    pub pin_frame: Option<Bounds<Pixels>>,
    pub status: AsyncTextBubbleDeliveryState,
    pub status_frame: Option<Bounds<Pixels>>,
    pub content_size: Size<Pixels>,
}

impl RiseNativeMediaMetaModel {
    /// time zone identifier none reads the live system zone
    pub fn new(
        message: &GetMessageMessage,
        is_own: bool,
        style: RiseNativeMediaMetaModelStyle,
        text_delta: f32,
        time_zone_identifier: Option<&str>,
        cx: &App,
    ) -> Self {
        let font_size = (MessageSizes::meta_font_size() + text_delta).max(1.0);
        let font = AsyncTextBubblePreparer::system_font();
        let text_system = cx.text_system().clone();
        let text_height = NativeVoiceNoteMetrics::text_height(font_size);

        let measure = |string: &str| -> (AsyncTextBubbleLine, f32) {
            let line = AsyncTextBubblePreparer::make_line(string, &font, px(font_size), &text_system);
            let width = RiseNativeMediaLayout::pixel_ceil(f32::from(line.typographic_width()));
            (line, width)
        };

        let edited_text = message.edited_at.is_some().then(|| tr("chat_msg_edited"));
        let zone = time_zone_identifier.map_or_else(|| TimeZone::current().identifier(), str::to_owned);
        let time_text = RiseFormat::clock_in_time_zone(
            &Date::from_time_interval_since_1970(message.created_at),
            &zone,
            RiseFormat::uses_twelve_hour_clock(),
        );
        let status = if is_own { Self::delivery_state(message) } else { AsyncTextBubbleDeliveryState::None };
        let icon = px(MessageSizes::meta_icon_font_size());
        let slot = MessageDeliveryStatusGeometry::slot_size(status, icon);
        let pin_size = message.is_pinned.then(NativeVoiceNoteMetrics::pin_size);

        let edited_measure = edited_text.as_deref().map(measure);
        let time_measure = measure(&time_text);
        let height = text_height.max(f32::from(slot.height)).max(pin_size.unwrap_or(0.0));
        let spacing = MessageSizes::meta_content_spacing();
        let mut x: f32 = 0.0;
        let mut place = |width: f32, child_height: f32| -> Bounds<Pixels> {
            if x > 0.0 {
                x += spacing;
            }
            let frame = bounds(point(px(x), px((height - child_height) / 2.0)), size(px(width), px(child_height)));
            x += width;
            frame
        };
        let ascender = f32::from(time_measure.0.ascent);
        let edited = edited_measure.map(|(line, width)| RiseNativeMediaMetaModelRun {
            frame: place(width, text_height),
            ascender: f32::from(line.ascent),
            line,
        });
        let time = RiseNativeMediaMetaModelRun {
            frame: place(time_measure.1, text_height),
            line: time_measure.0,
            ascender,
        };
        let pin_frame = pin_size.map(|side| place(side, side));
        let status_frame = if status != AsyncTextBubbleDeliveryState::None {
            let slot_frame = place(f32::from(slot.width), f32::from(slot.height));
            let glyph = MessageDeliveryStatusGeometry::size(status, icon);
            Some(bounds(
                point(slot_frame.origin.x, slot_frame.origin.y + (slot.height - glyph.height) / 2.0),
                glyph,
            ))
        } else {
            None
        };
        Self {
            style,
            is_own,
            edited_text,
            time_text,
            edited,
            time,
            pin_frame,
            status,
            status_frame,
            content_size: size(px(x), px(height)),
        }
    }

    pub fn plate_size(&self) -> Size<Pixels> {
        size(
            self.content_size.width + px(MessageSizes::meta_overlay_horizontal_padding() * 2.0),
            self.content_size.height + px(MessageSizes::meta_overlay_vertical_padding() * 2.0),
        )
    }

    /// status icon state, a failed row is also pending so failure is asked first
    pub fn delivery_state(message: &GetMessageMessage) -> AsyncTextBubbleDeliveryState {
        NativeMessageMetaView::delivery_state(message)
    }
}
