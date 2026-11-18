use gpui::px;
use rise_theme::ThemeStore;
use rise_chat::ChatType;
use rise_core::foundation::foundation_calendar::TimeZone;
use rise_core::foundation::foundation_date::Date;
use rise_core::foundation::ns_range::NSRange;
use rise_ui::message_bubble_geometry::MessageBubbleTailSide;
use rise_ui::rich_text::rich_text_markup::RichTextMarkup;

use crate::core::lib::canon_format::RiseFormat;
use crate::core::ui::formatted_text_ui::FormattedText;
use crate::modules::chat::components::chat::async_render::async_text_bubble_types::{
    AsyncTextBubbleDeliveryState, AsyncTextBubbleLayoutSpec, AsyncTextBubbleSeed,
};
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatInfo;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageDeliveryStatus, MessageNormalizedEntities,
};

pub enum RiseAsyncTextMessageEligibility {}

impl RiseAsyncTextMessageEligibility {
    /// defaults: allows_incoming_group_without_chrome false, current time zone no highlight ranges
    /// plain_text_eligibility none
    pub fn seed(
        message: &GetMessageMessage,
        chat: &ChatInfo,
        is_own: bool,
        allows_incoming_group_without_chrome: bool,
        time_zone_identifier: Option<String>,
        highlight_ranges: Vec<NSRange>,
        plain_text_eligibility: Option<bool>,
    ) -> Option<AsyncTextBubbleSeed> {
        if !(Self::supports_message_content(message)
            && plain_text_eligibility.unwrap_or_else(|| Self::supports_plain_text(&message.text)))
        {
            return None;
        }

        if chat.r#type == ChatType::Group && !is_own && !allows_incoming_group_without_chrome {
            return None;
        }

        Some(
            AsyncTextBubbleSeed::new(
                message.id.clone(),
                message.text.clone(),
                Date::from_time_interval_since_1970(message.created_at),
            )
            .time_zone_identifier(time_zone_identifier.unwrap_or_else(|| TimeZone::current().identifier()))
            .delivery_state(Self::delivery_state(message, chat, is_own))
            .view_count_text(if chat.r#type == ChatType::Channel {
                message.view_count.map(RiseFormat::compact_count)
            } else {
                None
            })
            .highlight_ranges(highlight_ranges),
        )
    }

    pub fn supports_message_content(message: &GetMessageMessage) -> bool {
        message.content_type == MessageContentType::Text
            && !message.text.is_empty()
            && message.reply_to.is_none()
            && message.story_reply.is_none()
            && message.forward_info.is_none()
            && message.via_bot_label_tag().is_none()
            && message.link_preview.is_none()
            && !message.has_reaction_row()
            && !message
                .reply_markup
                .as_ref()
                .is_some_and(|reply_markup| reply_markup.has_inline_keyboard())
            && message.edited_at.is_none()
            && !message.is_pinned
            && message.discussion_chat_id.as_ref().is_none_or(|id| id.is_empty())
            && message.discussion_message_id.as_ref().is_none_or(|id| id.is_empty())
            && message.shared_post.is_none()
            && message.contact.is_none()
            && message.location.is_none()
            && message.poll.is_none()
            && message.media_items.is_empty()
            && !Self::has_renderable_entities(&message.resolved_entities())
            && message.document_raw.is_none()
            && !message.is_disabled_by_moderation()
    }

    /// defaults: maximum_width is the bubble max width, viewport_width 390 is_own none
    pub fn layout_spec(maximum_width: f32, viewport_width: f32, is_own: Option<bool>) -> AsyncTextBubbleLayoutSpec {
        AsyncTextBubbleLayoutSpec::new(
            px(maximum_width),
            px(MessageSizes::bubble_minimum_width()),
            px(MessageSizes::min_message_row_height()),
            px(MessageSizes::horizontal_bubble_padding(viewport_width)),
            px(MessageSizes::bubble_inner_vertical_padding()),
            Some(px(MessageSizes::bubble_inner_top_padding())),
            Some(px(MessageSizes::bubble_inner_bottom_padding())),
            is_own.map_or(MessageBubbleTailSide::None, |is_own| {
                if is_own {
                    MessageBubbleTailSide::Trailing
                } else {
                    MessageBubbleTailSide::Leading
                }
            }),
            px(MessageSizes::bubble_tail_inset()),
            px(MessageSizes::text_font_size()),
            px(Self::time_font_size()),
            px(MessageSizes::text_line_spacing()),
            px(MessageSizes::text_meta_flow_spacing()),
            px(2.0),
            px(MessageSizes::meta_icon_font_size()),
            px(MessageSizes::meta_content_spacing()),
        )
    }

    /// clock and view count size of the fast path, equal to MainText at meta_font_size
    pub fn time_font_size() -> f32 {
        (MessageSizes::meta_font_size() + ThemeStore::text_delta_snapshot()).max(1.0)
    }

    /// asks the parsed entities and not the raw json
    /// the string heuristic said true for any non empty array, even kinds the parser drops
    pub fn has_renderable_entities(entities: &MessageNormalizedEntities) -> bool {
        !entities.custom_emojis.is_empty()
            || !entities.role_mentions.is_empty()
            || !entities.topic_mentions.is_empty()
            || !entities.text_links.is_empty()
    }

    pub fn supports_plain_text(text: &str) -> bool {
        let lowercased = text.to_lowercase();
        if lowercased.contains("http://") || lowercased.contains("https://") || lowercased.contains("www.") {
            return false;
        }
        if text.contains('.') && FormattedText::contains_link(text) {
            return false;
        }
        if RichTextMarkup::has_any_markup(text) {
            return false;
        }
        !text.contains('@') && !text.contains('#')
    }

    fn delivery_state(message: &GetMessageMessage, chat: &ChatInfo, is_own: bool) -> AsyncTextBubbleDeliveryState {
        if chat.r#type == ChatType::Channel {
            if !message.is_pending_outgoing() {
                return AsyncTextBubbleDeliveryState::None;
            }
            return if message.delivery_status == MessageDeliveryStatus::Failed {
                AsyncTextBubbleDeliveryState::Failed
            } else {
                AsyncTextBubbleDeliveryState::Sending
            };
        }
        if !is_own {
            return AsyncTextBubbleDeliveryState::None;
        }
        if message.delivery_status == MessageDeliveryStatus::Failed {
            return AsyncTextBubbleDeliveryState::Failed;
        }
        if message.is_pending_outgoing() {
            return AsyncTextBubbleDeliveryState::Sending;
        }
        match message.delivery_status {
            MessageDeliveryStatus::Sending => AsyncTextBubbleDeliveryState::Sending,
            MessageDeliveryStatus::Failed => AsyncTextBubbleDeliveryState::Failed,
            MessageDeliveryStatus::Sent => AsyncTextBubbleDeliveryState::Sent,
            MessageDeliveryStatus::Delivered => AsyncTextBubbleDeliveryState::Delivered,
            MessageDeliveryStatus::Read => AsyncTextBubbleDeliveryState::Read,
        }
    }
}
