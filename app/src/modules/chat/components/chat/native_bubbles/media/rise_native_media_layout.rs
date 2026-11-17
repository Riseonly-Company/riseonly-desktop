use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use gpui::{App, Bounds, Pixels, Size, bounds, point, px, size};
use rise_chat::ChatType;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_ui::message_bubble_geometry::{MessageBubbleDescriptor, MessageBubbleRadii, MessageBubbleTailSide};
use rise_ui::message_media_tail_presentation::MessageMediaTailGeometry;
use rise_ui::swiftui::font::NATURAL_LINE_HEIGHT;

use crate::modules::chat::components::chat::async_render::async_text_bubble_preparer::AsyncTextBubblePreparer;
use crate::modules::chat::components::chat::async_render::async_text_bubble_types::AsyncTextBubbleLinePlacement;
use crate::modules::chat::components::chat::async_render::chat_message_render_snapshot::ChatMessageRenderEntryGroupFlags;
use crate::modules::chat::components::chat::async_render::rise_async_text_message_row::RiseAsyncTextMessageEligibility;
use crate::modules::chat::components::chat::async_render::rise_message_list_view::RiseMessageCollectionConfiguration;
use crate::modules::chat::components::chat::bubble::message_content::{MessageContent, MessageContentKind};
use crate::modules::chat::components::chat::bubble::message_image_grid_layout::{MessageGridLayout, MessageGridLayoutEngine};
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::components::chat::native_bubbles::media::rise_native_media_meta::{
    RiseNativeMediaMetaModel, RiseNativeMediaMetaModelStyle,
};
use crate::modules::chat::components::chat::native_bubbles::sticker::native_interactive_message_cell::native_bubble_pixel_ceil;
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageMediaItem, MessageMediaKind,
};
use rise_theme::ThemeStore;

pub enum RiseNativeMediaEligibility {}

impl RiseNativeMediaEligibility {
    pub fn supports_shape(
        message: &GetMessageMessage,
        chat: &ChatInfo,
        is_own: bool,
        configuration: &RiseMessageCollectionConfiguration,
    ) -> bool {
        let items: Vec<&MessageMediaItem> =
            message.grid_media_items().into_iter().filter(|item| !item.is_cancelled).collect();
        if MessageContent::bubble_kind(message, Some(chat)) != MessageContentKind::MediaGrid
            || items.is_empty()
            || items.len() != message.media_items.len()
            || message.has_reaction_row()
            || message.reply_to.is_some()
            || message.forward_info.is_some()
            || message.story_reply.is_some()
            || message.via_bot_label_tag().is_some()
            || message.link_preview.is_some()
            || message.reply_markup.as_ref().is_some_and(|markup| markup.has_inline_keyboard())
            || !message.discussion_chat_id.as_deref().is_none_or(str::is_empty)
            || message.is_disabled_by_moderation()
            || !(chat.r#type == ChatType::Direct || (chat.r#type == ChatType::Group && is_own))
        {
            return false;
        }
        let plain_media = items.iter().all(|item| {
            matches!(item.kind, MessageMediaKind::Image | MessageMediaKind::Video)
                && !item.is_uploading
                && !item.is_compressing
                && !item.has_spoiler
                && item.view_limit.is_none()
        });
        if !plain_media {
            return false;
        }
        if message.has_text() {
            let text = message.text.as_str();
            let highlighted = configuration
                .search_highlight
                .as_ref()
                .is_some_and(|highlight| highlight.message_id == message.id);
            if text != trimming_whitespaces_and_newlines(text)
                || !RiseAsyncTextMessageEligibility::supports_plain_text(text)
                || RiseAsyncTextMessageEligibility::has_renderable_entities(message.resolved_entities())
                || highlighted
            {
                return false;
            }
        }
        true
    }
}

/// caption lines and the block size
#[derive(Clone, Debug)]
pub struct RiseNativeMediaLayoutCaption {
    pub lines: Vec<AsyncTextBubbleLinePlacement>,
    pub size: Size<Pixels>,
}

/// everything the native media row needs, computed once per message
/// no caption is the grid with 2 pt inset and a time plate
/// with caption its grid and caption and time row, grid sets width
#[derive(Clone, Debug)]
pub struct RiseNativeMediaLayout {
    pub message: GetMessageMessage,
    pub items: Vec<MessageMediaItem>,
    pub grid: MessageGridLayout,
    pub is_own: bool,
    pub reserves_avatar_space: bool,
    pub top_padding: f32,
    pub bottom_padding: f32,
    pub bubble_radii: MessageBubbleRadii,
    pub tail_side: MessageBubbleTailSide,
    pub draws_tail: bool,
    pub grid_radii: MessageBubbleRadii,
    pub media_tail_side: MessageBubbleTailSide,
    pub caption: Option<Rc<RiseNativeMediaLayoutCaption>>,
    pub meta: RiseNativeMediaMetaModel,
    pub bubble_size: Size<Pixels>,
    pub grid_frame: Bounds<Pixels>,
    pub caption_frame: Bounds<Pixels>,
    pub meta_frame: Bounds<Pixels>,
    pub row_height: f32,
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct CaptionKey {
    text: String,
    width: u32,
    font_size: u32,
    line_spacing: u32,
}

thread_local! {
    static CAPTION_CACHE: RefCell<HashMap<CaptionKey, Rc<RiseNativeMediaLayoutCaption>>> = RefCell::new(HashMap::new());
}

impl RiseNativeMediaLayout {
    pub fn has_caption(&self) -> bool {
        self.caption.is_some()
    }

    pub fn make(
        message: &GetMessageMessage,
        group: ChatMessageRenderEntryGroupFlags,
        configuration: &RiseMessageCollectionConfiguration,
        viewport_width: f32,
        cx: &mut App,
    ) -> Option<Self> {
        let chat = &configuration.chat;
        let is_own = MessageList::is_own_bubble(message, chat, cx);
        if !RiseNativeMediaEligibility::supports_shape(message, chat, is_own, configuration) {
            return None;
        }

        let items: Vec<MessageMediaItem> =
            message.grid_media_items().into_iter().filter(|item| !item.is_cancelled).cloned().collect();
        let inset = MessageSizes::media_inset();
        let maximum = MessageSizes::media_maximum_size(viewport_width);
        let available_width = MessageSizes::available_bubble_width(viewport_width, chat, is_own);
        let grid = MessageGridLayoutEngine::compute(
            &items,
            maximum.width.min(available_width - MessageSizes::bubble_tail_inset() - inset * 2.0).max(1.0),
            maximum.height,
            MessageSizes::media_mosaic_separator_width(),
            Some(message.id.as_str()),
        );
        if grid.cells.is_empty()
            || !grid.cells.iter().all(|cell| {
                cell.upload_seed.is_none() && !cell.download_seed.as_ref().is_some_and(|seed| seed.shows_idle_affordance)
            })
        {
            return None;
        }

        let descriptor = MessageBubbleDescriptor::grouped(
            is_own,
            group.is_first,
            group.is_last,
            px(MessageSizes::bubble_corner_radius()),
            px(MessageSizes::bubble_stacked_corner_radius()),
        );
        let radii = descriptor.radii;
        let has_caption = message.has_text();
        let grid_radii = if has_caption {
            MessageBubbleRadii {
                top_leading: radii.top_leading,
                top_trailing: radii.top_trailing,
                bottom_leading: px(0.0),
                bottom_trailing: px(0.0),
            }
        } else {
            radii
        };
        let media_tail_side = MessageMediaTailGeometry::eligible_side(
            descriptor.draws_tail,
            descriptor.tail_side,
            grid_radii.bottom_leading,
            grid_radii.bottom_trailing,
        );

        let top = if group.is_first {
            MessageSizes::list_first_in_group_top_padding()
        } else {
            MessageSizes::list_continuation_top_padding()
        };
        let bottom = if group.is_last { MessageSizes::list_last_in_group_bottom_padding() } else { 0.0 };
        let reserves_avatar = MessageSizes::reserves_avatar_space(chat, is_own);
        let grid_width = f32::from(grid.container_size.width);
        let grid_height = f32::from(grid.container_size.height);
        let column_width = grid_width + inset * 2.0;
        let bubble_width = MessageSizes::bubble_minimum_width().max(column_width + MessageSizes::bubble_tail_inset());
        let body_min_x = if is_own { 0.0 } else { MessageSizes::bubble_tail_inset() };
        let grid_frame = Self::rect(body_min_x + inset, inset, grid_width, grid_height);
        let meta = RiseNativeMediaMetaModel::new(
            message,
            is_own,
            if has_caption { RiseNativeMediaMetaModelStyle::Inline } else { RiseNativeMediaMetaModelStyle::Overlay },
            ThemeStore::text_delta_snapshot(),
            None,
            cx,
        );
        let meta_width = f32::from(meta.content_size.width);
        let meta_height = f32::from(meta.content_size.height);

        if !has_caption {
            let bubble_height = grid_height + inset * 2.0;
            let body = if reserves_avatar { MessageSizes::avatar_size().max(bubble_height) } else { bubble_height };
            let plate = meta.plate_size();
            let meta_frame = Self::rect(
                body_min_x + inset + grid_width - 6.0 - f32::from(plate.width)
                    + MessageSizes::meta_overlay_horizontal_padding(),
                inset + grid_height - 6.0 - f32::from(plate.height) + MessageSizes::meta_overlay_vertical_padding(),
                meta_width,
                meta_height,
            );
            return Some(Self {
                message: message.clone(),
                items,
                grid,
                is_own,
                reserves_avatar_space: reserves_avatar,
                top_padding: top,
                bottom_padding: bottom,
                bubble_radii: radii,
                tail_side: descriptor.tail_side,
                draws_tail: descriptor.draws_tail,
                grid_radii,
                media_tail_side,
                caption: None,
                meta,
                bubble_size: size(px(bubble_width), px(bubble_height)),
                grid_frame,
                caption_frame: Bounds::default(),
                meta_frame,
                row_height: (top + body + bottom).ceil(),
            });
        }

        let horizontal_padding = MessageSizes::horizontal_bubble_padding(viewport_width);
        let caption_width = (column_width - horizontal_padding * 2.0).max(1.0);
        let caption = Self::measure_caption(
            &message.text,
            caption_width,
            MessageSizes::text_font_size(),
            MessageSizes::text_line_spacing(),
            cx,
        );
        let caption_height = f32::from(caption.size.height);
        let caption_frame = Self::rect(
            body_min_x + horizontal_padding,
            inset + grid_height + 6.0,
            caption_width,
            caption_height,
        );
        let meta_row_y = inset + grid_height + 6.0 + caption_height + 6.0;
        let meta_frame = Self::rect(
            body_min_x + column_width - horizontal_padding - meta_width,
            meta_row_y,
            meta_width,
            meta_height,
        );
        let column_height = meta_row_y + meta_height + MessageSizes::bubble_inner_bottom_padding();
        let bubble_height = MessageSizes::min_message_row_height().max(column_height);
        let body = if reserves_avatar { MessageSizes::avatar_size().max(bubble_height) } else { bubble_height };
        Some(Self {
            message: message.clone(),
            items,
            grid,
            is_own,
            reserves_avatar_space: reserves_avatar,
            top_padding: top,
            bottom_padding: bottom,
            bubble_radii: radii,
            tail_side: descriptor.tail_side,
            draws_tail: descriptor.draws_tail,
            grid_radii,
            media_tail_side,
            caption: Some(caption),
            meta,
            bubble_size: size(px(bubble_width), px(bubble_height)),
            grid_frame,
            caption_frame,
            meta_frame,
            row_height: Self::pixel_ceil(top + body + bottom),
        })
    }

    pub fn measure_caption(
        text: &str,
        width: f32,
        font_size: f32,
        line_spacing: f32,
        cx: &App,
    ) -> Rc<RiseNativeMediaLayoutCaption> {
        let key = CaptionKey {
            text: text.to_owned(),
            width: width.to_bits(),
            font_size: font_size.to_bits(),
            line_spacing: line_spacing.to_bits(),
        };
        if let Some(cached) = CAPTION_CACHE.with(|cache| cache.borrow().get(&key).cloned()) {
            return cached;
        }
        let font = AsyncTextBubblePreparer::system_font();
        let text_system = cx.text_system().clone();
        let measured = AsyncTextBubblePreparer::measure_text(text, &font, px(font_size), width, line_spacing, &text_system);
        let line_height = font_size * NATURAL_LINE_HEIGHT;
        let lines: Vec<AsyncTextBubbleLinePlacement> = measured
            .lines
            .into_iter()
            .enumerate()
            .map(|(index, measured_line)| {
                let mut placement = measured_line.placement;
                let ascender = f32::from(placement.line.ascent);
                placement.baseline_origin = point(px(0.0), px(ascender + index as f32 * (line_height + line_spacing)));
                placement
            })
            .collect();
        let count = lines.len().max(1) as f32;
        let height = Self::pixel_ceil(count * line_height + (count - 1.0) * line_spacing);
        let caption = Rc::new(RiseNativeMediaLayoutCaption { lines, size: size(px(width), px(height)) });
        CAPTION_CACHE.with(|cache| {
            let mut cache = cache.borrow_mut();
            if cache.len() >= 512 {
                cache.clear();
            }
            cache.insert(key, caption.clone());
        });
        caption
    }

    /// rounds up to the pixel grid
    /// rows are priced without a window so float noise must not add a whole point
    pub fn pixel_ceil(value: f32) -> f32 {
        native_bubble_pixel_ceil(value)
    }

    fn rect(x: f32, y: f32, width: f32, height: f32) -> Bounds<Pixels> {
        bounds(point(px(x), px(y)), size(px(width), px(height)))
    }
}
