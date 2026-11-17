use gpui::{AnyElement, App, Bounds, Hsla, IntoElement, PathBuilder, Pixels, Point, RenderOnce, Size, Window, black, canvas, fill, point, rgb, size, white};
use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_date::{Date, TimeInterval};
use rise_i18n::tr;
use rise_theme::theme::theme_environment::theme_store;
use rise_ui::IconUi;
use rise_ui::main_text::MainText;
use rise_ui::swiftui::prelude::*;

use crate::core::lib::canon_format::RiseFormat;
use crate::modules::chat::components::chat::async_render::async_text_bubble_types::AsyncTextBubbleDeliveryState;
use crate::modules::chat::components::chat::bubble::bubble_corners::TelegramReactionPalette;
use crate::modules::chat::components::chat::bubble::message_pending_clock_icon::MessagePendingClockIcon;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::message::message_actions::message_types::{GetMessageMessage, MessageDeliveryStatus};

pub enum MessageDeliveryStatusGeometry {}

impl MessageDeliveryStatusGeometry {
    pub const REFERENCE_CHECK_WIDTH: f32 = 11.0;
    pub const REFERENCE_CHECK_HEIGHT: f32 = 9.0;
    pub const REFERENCE_DOUBLE_CHECK_WIDTH: f32 = 17.0;

    pub fn size(state: AsyncTextBubbleDeliveryState, base_size: Pixels) -> Size<Pixels> {
        let base_size = f32::from(base_size);
        let scale = (base_size / Self::REFERENCE_CHECK_WIDTH).max(0.01);
        match state {
            AsyncTextBubbleDeliveryState::None => size(px(0.0), px(0.0)),
            AsyncTextBubbleDeliveryState::Sending | AsyncTextBubbleDeliveryState::Failed => {
                size(px(base_size), px(base_size))
            }
            AsyncTextBubbleDeliveryState::Sent => size(
                px(Self::REFERENCE_CHECK_WIDTH * scale),
                px(Self::REFERENCE_CHECK_HEIGHT * scale),
            ),
            AsyncTextBubbleDeliveryState::Delivered | AsyncTextBubbleDeliveryState::Read => size(
                px(Self::REFERENCE_DOUBLE_CHECK_WIDTH * scale),
                px(Self::REFERENCE_CHECK_HEIGHT * scale),
            ),
        }
    }

    /// every live state takes the widest glyph room so sent to read never moves the time or wraps it
    /// glyph sits at the leading edge, the double check only adds its second stroke on the right
    pub fn slot_size(state: AsyncTextBubbleDeliveryState, base_size: Pixels) -> Size<Pixels> {
        if state == AsyncTextBubbleDeliveryState::None {
            return size(px(0.0), px(0.0));
        }
        let base_size = f32::from(base_size);
        let scale = (base_size / Self::REFERENCE_CHECK_WIDTH).max(0.01);
        size(
            px(base_size.max(Self::REFERENCE_DOUBLE_CHECK_WIDTH * scale)),
            px(base_size.max(Self::REFERENCE_CHECK_HEIGHT * scale)),
        )
    }
}

/// one tick of the delivery mark drawn in rect
struct TelegramMessageCheckShape {
    partial: bool,
}

impl TelegramMessageCheckShape {
    const LINE_WIDTH: f32 = 0.99;

    fn path(&self, rect: Bounds<Pixels>) -> Vec<Point<Pixels>> {
        let scale_x = f32::from(rect.size.width) / MessageDeliveryStatusGeometry::REFERENCE_CHECK_WIDTH;
        let scale_y = f32::from(rect.size.height) / MessageDeliveryStatusGeometry::REFERENCE_CHECK_HEIGHT;
        let at = |x: f32, y: f32| point(rect.left() + px(x * scale_x), rect.top() + px(y * scale_y));
        if self.partial {
            vec![at(1.5, 8.0), at(8.0, 1.0)]
        } else {
            vec![at(1.0, 5.0), at(3.951_570_5, 7.951_570_5), at(10.5, 1.0)]
        }
    }

    /// strokes the tick, line width 0.99 with round cap and join
    fn stroke(&self, rect: Bounds<Pixels>, color: Hsla, window: &mut Window) {
        let points = self.path(rect);
        let mut builder = PathBuilder::stroke(px(Self::LINE_WIDTH));
        builder.add_polygon(&points, false);
        if let Ok(path) = builder.build() {
            window.paint_path(path, color);
        }
        let half = Self::LINE_WIDTH / 2.0;
        for vertex in points {
            let dot = Bounds::new(
                point(vertex.x - px(half), vertex.y - px(half)),
                size(px(Self::LINE_WIDTH), px(Self::LINE_WIDTH)),
            );
            window.paint_quad(fill(dot, color).corner_radii(px(half)));
        }
    }
}

#[derive(IntoElement)]
struct TelegramMessageStatusIcon {
    state: AsyncTextBubbleDeliveryState,
    color: Hsla,
}

impl TelegramMessageStatusIcon {
    fn check_size() -> Size<Pixels> {
        size(
            px(MessageDeliveryStatusGeometry::REFERENCE_CHECK_WIDTH),
            px(MessageDeliveryStatusGeometry::REFERENCE_CHECK_HEIGHT),
        )
    }

    /// single tick or full tick with the partial one 6 points to its right
    /// both anchored top leading in the double check frame
    fn checks(double: bool, color: Hsla, width: f32) -> impl IntoElement {
        let check_size = Self::check_size();
        canvas(
            |_, _, _| {},
            move |bounds, (), window, _| {
                let first = Bounds::new(bounds.origin, check_size);
                TelegramMessageCheckShape { partial: false }.stroke(first, color, window);
                if double {
                    let second = Bounds::new(point(bounds.origin.x + px(6.0), bounds.origin.y), check_size);
                    TelegramMessageCheckShape { partial: true }.stroke(second, color, window);
                }
            },
        )
        .frame_size(width, MessageDeliveryStatusGeometry::REFERENCE_CHECK_HEIGHT)
    }
}

impl TelegramMessageStatusIcon {
    fn exclamation_mark(icon_size: f32) -> AnyElement {
        let stroke = (icon_size * 0.13).max(1.5);
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(icon_size * 0.07))
            .child(div().w(px(stroke)).h(px(icon_size * 0.36)).rounded(px(stroke / 2.0)).bg(white()))
            .child(div().w(px(stroke)).h(px(stroke)).rounded_full().bg(white()))
            .into_any_element()
    }
}

impl RenderOnce for TelegramMessageStatusIcon {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let icon_size = MessageSizes::meta_icon_font_size();
        match self.state {
            AsyncTextBubbleDeliveryState::None => div().into_any_element(),
            AsyncTextBubbleDeliveryState::Sending => {
                MessagePendingClockIcon::new().size(icon_size).color(self.color).into_any_element()
            }
            AsyncTextBubbleDeliveryState::Failed => zstack(Alignment::Center)
                .child(div().size_full().circle().bg(self.color))
                .child(div().size_full().circle().stroke_border(white(), (icon_size * 0.11).max(1.0)))
                .child(Self::exclamation_mark(icon_size))
                .frame_size(icon_size, icon_size)
                .into_any_element(),
            AsyncTextBubbleDeliveryState::Sent => {
                Self::checks(false, self.color, MessageDeliveryStatusGeometry::REFERENCE_CHECK_WIDTH)
                    .into_any_element()
            }
            AsyncTextBubbleDeliveryState::Delivered | AsyncTextBubbleDeliveryState::Read => {
                Self::checks(true, self.color, MessageDeliveryStatusGeometry::REFERENCE_DOUBLE_CHECK_WIDTH)
                    .into_any_element()
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MessageMetaStyle {
    #[default]
    Inline,
    Overlay,
}

#[derive(IntoElement)]
pub struct MessageMeta {
    message: GetMessageMessage,
    is_own: bool,
    is_channel: bool,
    style: MessageMetaStyle,
    show_unplayed_dot: bool,
    on_wallpaper: bool,
}

impl MessageMeta {
    pub fn new(message: GetMessageMessage) -> Self {
        Self {
            message,
            is_own: false,
            is_channel: false,
            style: MessageMetaStyle::Inline,
            show_unplayed_dot: false,
            on_wallpaper: false,
        }
    }

    pub fn is_own(mut self, is_own: bool) -> Self {
        self.is_own = is_own;
        self
    }

    pub fn is_channel(mut self, is_channel: bool) -> Self {
        self.is_channel = is_channel;
        self
    }

    pub fn style(mut self, style: MessageMetaStyle) -> Self {
        self.style = style;
        self
    }

    pub fn show_unplayed_dot(mut self, show_unplayed_dot: bool) -> Self {
        self.show_unplayed_dot = show_unplayed_dot;
        self
    }

    pub fn on_wallpaper(mut self, on_wallpaper: bool) -> Self {
        self.on_wallpaper = on_wallpaper;
        self
    }

    /// black at 45% vanishes on a dark wallpaper so dark theme gets a grey plate
    /// it also covers the sticker corner so it stays dark enough for white text
    pub fn wallpaper_plate_fill(is_dark_theme: bool) -> Hsla {
        if is_dark_theme {
            Hsla::from(rgb(0x3A3A3A)).opacity(0.7)
        } else {
            black().opacity(0.45)
        }
    }

    fn status_icon(&self, text_color: Hsla, error_color: Hsla, primary_color: Hsla) -> AnyElement {
        let state = if self.message.delivery_status == MessageDeliveryStatus::Failed {
            AsyncTextBubbleDeliveryState::Failed
        } else if self.message.is_pending_outgoing() {
            AsyncTextBubbleDeliveryState::Sending
        } else {
            match self.message.delivery_status {
                MessageDeliveryStatus::Sending => AsyncTextBubbleDeliveryState::Sending,
                MessageDeliveryStatus::Failed => AsyncTextBubbleDeliveryState::Failed,
                MessageDeliveryStatus::Sent => AsyncTextBubbleDeliveryState::Sent,
                MessageDeliveryStatus::Delivered => AsyncTextBubbleDeliveryState::Delivered,
                MessageDeliveryStatus::Read => AsyncTextBubbleDeliveryState::Read,
            }
        };
        let status_color = if state == AsyncTextBubbleDeliveryState::Failed {
            error_color
        } else if state == AsyncTextBubbleDeliveryState::Read
            && self.style != MessageMetaStyle::Overlay
            && !self.is_own
        {
            primary_color
        } else {
            text_color
        };
        let slot = MessageDeliveryStatusGeometry::slot_size(state, px(MessageSizes::meta_icon_font_size()));
        zstack(Alignment::Leading)
            .child(TelegramMessageStatusIcon { state, color: status_color })
            .frame_size(f32::from(slot.width), f32::from(slot.height))
            .into_any_element()
    }

    fn format_time(ts: TimeInterval) -> String {
        RiseFormat::clock_in_time_zone(
            &Date::from_time_interval_since_1970(ts),
            &TimeZone::current().identifier(),
            RiseFormat::uses_twelve_hour_clock(),
        )
    }
}

impl RenderOnce for MessageMeta {
    fn render(self, _window: &mut Window, cx: &mut App) -> impl IntoElement {
        let store = theme_store(cx);
        let theme = store.read(cx).current_theme();
        let text_secondary = theme.text.secondary;
        let primary_100 = theme.primary._100;
        let on_primary = theme.content_on(primary_100);
        let background_100 = theme.bg._100;
        let error_100 = theme.semantic.error_100;

        let is_overlay = self.style == MessageMetaStyle::Overlay;
        let text_color = if is_overlay {
            white()
        } else if self.is_own {
            on_primary.opacity(0.78)
        } else {
            text_secondary
        };
        let pinned_tint = if is_overlay {
            white().opacity(0.85)
        } else if self.is_own {
            on_primary.opacity(0.62)
        } else {
            text_secondary
        };
        let meta_font_size = MessageSizes::meta_font_size();
        let meta_icon_font_size = MessageSizes::meta_icon_font_size();

        let mut content = hstack(MessageSizes::meta_content_spacing()).flex_shrink_0();
        if self.show_unplayed_dot {
            let dot = if is_overlay {
                white()
            } else if self.is_own {
                on_primary
            } else {
                primary_100
            };
            content = content.child(div().circle().bg(dot).frame_size(6.0, 6.0));
        }
        if self.message.edited_at.is_some() {
            content = content.child(MainText::new(tr("chat_msg_edited")).px(meta_font_size).color(text_color));
        }
        if let (true, Some(views)) = (self.is_channel, self.message.view_count) {
            content = content
                .children(IconUi::sized("eye", px(meta_icon_font_size), text_color, false))
                .child(
                    div()
                        .pr(px(MessageSizes::meta_content_spacing()))
                        .child(MainText::new(RiseFormat::compact_count(views)).px(meta_font_size).color(text_color)),
                );
        }
        content = content.child(
            div().flex_shrink_0().whitespace_nowrap().child(
                MainText::new(Self::format_time(self.message.created_at))
                    .px(meta_font_size)
                    .color(text_color),
            ),
        );
        if self.message.is_pinned {
            content = content.children(IconUi::sized("pin.fill", px(meta_icon_font_size), pinned_tint, false));
        }
        if (self.is_own && !self.is_channel) || (self.is_channel && self.message.is_pending_outgoing()) {
            content = content.child(self.status_icon(text_color, error_100, primary_100));
        }

        if is_overlay {
            let plate = if self.on_wallpaper {
                Self::wallpaper_plate_fill(TelegramReactionPalette::is_dark_background(background_100))
            } else {
                black().opacity(0.45)
            };
            content
                .px(px(MessageSizes::meta_overlay_horizontal_padding()))
                .py(px(MessageSizes::meta_overlay_vertical_padding()))
                .capsule()
                .bg(plate)
                .into_any_element()
        } else {
            content.into_any_element()
        }
    }
}
