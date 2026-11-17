use std::collections::BTreeMap;

use rise_canon::rise_key::bot as bot_key;
use rise_canon::rise_key::chat as chat_key;
use rise_canon::rise_key::events as events_key;
use rise_canon::rise_key::message as message_key;
use rise_canon::rise_key::payload as payload_key;
use rise_canon::rise_proto::message as proto;
use rise_canon::rise_request::message::create_message::MediaItemsElement;
use rise_canon::rise_response::link_preview::GetLinkPreview;
use rise_canon::rise_view::message as view;
use rise_canon::{RiseEvent, RiseResponseJSON};
use rise_engine::{RiseJSONValue, RiseWirePushEnvelope};
use rise_events::name;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::message_wire_types::{
    CreateMessageGatewayDTO, FanoutRefusalDetailsWireDTO, ForwardMessageDestinationWireDTO,
    ForwardMessageResultWireDTO, ForwardMessagesGatewayDTO, GetLinkMessagesGatewayDTO, GetMediaMessagesGatewayDTO,
    GetMessagesAroundGatewayDTO, GetMessagesGatewayDTO, GetMessagesListGatewayDTO, LinkMessageWireDTO,
    MediaItemWireDTO, MessageLinkMetadataWireDTO, MessageMediaInfoWireDTO, MessagePayloadCodec,
    MessageSearchMatchWireDTO, MessageSearchRangeWireDTO, MessageSenderMoreWireDTO, MessageSenderWireDTO,
    MessageWireDTO, RiseStoredOutgoingMessage,
};
use crate::modules::chat::engine::messages::rise_message_read_receipt_wire_types::{
    RiseStoredReadReceipt, RiseStoredReadReceipts,
};
use crate::modules::chat::engine::messages::rise_message_wire_codec::RiseMessageWireCodec;

impl MessageWireDTO {
    pub fn from_canon(message: &view::MessageInfo) -> anyhow::Result<Self> {
        RiseMessageWireCodec::wire(Self::stored_object(message)?)
    }

    pub fn stored_object(m: &view::MessageInfo) -> anyhow::Result<BTreeMap<String, RiseJSONValue>> {
        fn string(value: &Option<String>) -> RiseJSONValue {
            value.clone().map_or(RiseJSONValue::Null, RiseJSONValue::String)
        }
        fn integer(value: Option<i64>) -> RiseJSONValue {
            value.map_or(RiseJSONValue::Null, RiseJSONValue::Integer)
        }
        fn encoded<T: Serialize>(value: Option<&T>) -> anyhow::Result<RiseJSONValue> {
            match value {
                Some(value) => Ok(RiseJSONValue::encode(value)?),
                None => Ok(RiseJSONValue::Null),
            }
        }
        fn canon(value: &Option<RiseResponseJSON>) -> RiseJSONValue {
            value.as_ref().map_or(RiseJSONValue::Null, rise_json_value_from_canon)
        }
        let entries: [(&str, RiseJSONValue); 54] = [
            (bot_key::ID, RiseJSONValue::String(m.id.clone())),
            (bot_key::CHAT_ID, RiseJSONValue::String(m.chat_id.clone())),
            (bot_key::TOPIC_ID, string(&m.topic_id)),
            (bot_key::TEMP_ID, string(&m.temp_id)),
            (bot_key::SENDER_ID, RiseJSONValue::String(m.sender_id.clone())),
            (bot_key::SENDER_NAME, RiseJSONValue::String(m.sender_name.clone())),
            (bot_key::SENDER, encoded(m.sender.as_ref())?),
            (bot_key::CONTENT, RiseJSONValue::String(m.content.clone())),
            (bot_key::ORIGINAL_CONTENT, RiseJSONValue::String(m.original_content.clone())),
            (bot_key::CONTENT_TYPE, RiseJSONValue::String(m.content_type.clone())),
            (bot_key::CAPTION, RiseJSONValue::String(m.caption.clone())),
            (bot_key::ENTITIES, string(&m.entities)),
            (bot_key::CREATED_AT, RiseJSONValue::Integer(m.created_at)),
            (bot_key::TIMESTAMP, RiseJSONValue::Integer(m.timestamp)),
            (bot_key::EDIT_DATE, integer(m.edit_date)),
            (bot_key::IS_PINNED, RiseJSONValue::Bool(m.is_pinned)),
            (bot_key::IS_READ, RiseJSONValue::Bool(m.is_read)),
            (bot_key::IS_EDITED, RiseJSONValue::Bool(m.is_edited)),
            (bot_key::IS_FORWARDED, RiseJSONValue::Bool(m.is_forwarded)),
            (bot_key::IS_REPLY, RiseJSONValue::Bool(m.is_reply)),
            (bot_key::IS_MENTIONED_YOU, RiseJSONValue::Bool(m.is_mentioned_you)),
            (message_key::IS_REPLIED_YOU, RiseJSONValue::Bool(m.is_replied_you)),
            (bot_key::IS_SYSTEM_MESSAGE, RiseJSONValue::Bool(m.is_system_message)),
            (bot_key::SYSTEM_MESSAGE_TYPE, RiseJSONValue::String(m.system_message_type.clone())),
            (bot_key::SYSTEM_ACTOR_INFO, encoded(m.system_actor_info.as_ref())?),
            (bot_key::IS_BOT_MESSAGE, RiseJSONValue::Bool(m.is_bot_message)),
            (bot_key::VIA_BOT, string(&m.via_bot)),
            (bot_key::VIA_BOT_TAG, string(&m.via_bot_tag)),
            (bot_key::HAS_ATTACHMENTS, RiseJSONValue::Bool(m.has_attachments)),
            (bot_key::HAS_PROTECTED_CONTENT, RiseJSONValue::Bool(m.has_protected_content)),
            (bot_key::HAS_REACTIONS, RiseJSONValue::Bool(m.has_reactions)),
            (bot_key::REACTIONS_UNREADED, RiseJSONValue::Bool(m.reactions_unreaded)),
            (bot_key::REACTIONS, encoded(Some(&m.reactions))?),
            (bot_key::REACTED_BY, encoded(Some(&m.reacted_by))?),
            (
                bot_key::PLAYED_BY,
                RiseJSONValue::Array(m.played_by.iter().cloned().map(RiseJSONValue::String).collect()),
            ),
            (bot_key::MEDIA_GROUP_ID, RiseJSONValue::String(m.media_group_id.clone())),
            (bot_key::MEDIA_ITEMS, encoded(Some(&m.media_items))?),
            (bot_key::REPLY_TO, encoded(m.reply_to.as_ref())?),
            (bot_key::FORWARD_INFO, encoded(m.forward_info.as_ref())?),
            (bot_key::LINK_PREVIEW, encoded(m.link_preview.as_ref())?),
            (bot_key::VIEW_COUNT, RiseJSONValue::Integer(m.view_count)),
            (bot_key::MODERATION_STATUS, string(&m.moderation_status)),
            (bot_key::DISCUSSION_CHAT_ID, string(&m.discussion_chat_id)),
            (bot_key::DISCUSSION_MESSAGE_ID, string(&m.discussion_message_id)),
            (bot_key::COMMENTS_COUNT, integer(m.comments_count)),
            (bot_key::SPARKS_TOTAL, RiseJSONValue::Integer(m.sparks_total)),
            (bot_key::MY_SPARKS, RiseJSONValue::Integer(m.my_sparks)),
            (bot_key::METADATA, canon(&m.metadata)),
            (payload_key::SHARED_POST, canon(&m.shared_post)),
            (events_key::STORY_REPLY, canon(&m.story_reply)),
            (bot_key::DOCUMENT, canon(&m.document)),
            (bot_key::RECENT_COMMENTERS, canon(&m.recent_commenters)),
            (bot_key::REPLY_MARKUP, canon(&m.reply_markup)),
            (bot_key::WEB_APP_DATA, canon(&m.web_app_data)),
        ];
        Ok(entries.into_iter().map(|(key, value)| (key.to_owned(), value)).collect())
    }
}

pub fn rise_json_value_from_canon(value: &RiseResponseJSON) -> RiseJSONValue {
    match value {
        Value::Null => RiseJSONValue::Null,
        Value::Bool(flag) => RiseJSONValue::Bool(*flag),
        Value::String(text) => RiseJSONValue::String(text.clone()),
        Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                RiseJSONValue::Integer(integer)
            } else if let Some(unsigned) = number.as_u64() {
                RiseJSONValue::UnsignedInteger(unsigned)
            } else {
                let double = number.as_f64().unwrap_or(0.0);
                if double.round() == double && double.abs() < 9_007_199_254_740_992.0 {
                    RiseJSONValue::Integer(double as i64)
                } else {
                    RiseJSONValue::Number(double)
                }
            }
        }
        Value::Array(items) => RiseJSONValue::Array(items.iter().map(rise_json_value_from_canon).collect()),
        Value::Object(fields) => RiseJSONValue::Object(
            fields.iter().map(|(key, field)| (key.clone(), rise_json_value_from_canon(field))).collect(),
        ),
    }
}

fn wire_messages(messages: &[view::MessageInfo]) -> anyhow::Result<Vec<MessageWireDTO>> {
    messages.iter().map(MessageWireDTO::from_canon).collect()
}

impl GetMessagesGatewayDTO {
    pub fn from_canon(page: &view::Page) -> anyhow::Result<Self> {
        Ok(Self {
            chat_id: page.chat_id.clone(),
            limit: page.limit,
            relative_id: page.relative_id.clone(),
            is_have_more: page.is_have_more,
            messages: Some(wire_messages(&page.messages)?),
            success: None,
            error: None,
        })
    }
}

impl GetMessagesAroundGatewayDTO {
    pub fn from_canon(page: &view::Around) -> anyhow::Result<Self> {
        Ok(Self {
            success: None,
            error: None,
            chat_id: page.chat_id.clone(),
            anchor_message_id: Some(page.anchor_message_id.clone()),
            anchor_relative_id: page.anchor_relative_id.clone(),
            before_limit: Some(page.before_limit),
            after_limit: Some(page.after_limit),
            older_relative_id: page.older_relative_id.clone(),
            has_older: Some(page.has_older),
            newer_relative_id: page.newer_relative_id.clone(),
            has_newer: Some(page.has_newer),
            messages: Some(wire_messages(&page.messages)?),
        })
    }
}

pub trait RiseCanonMessageList: DeserializeOwned + Send + Sync {
    fn messages(&self) -> &[view::MessageInfo];
    fn is_have_more(&self) -> Option<bool>;
    fn list_cursor(&self) -> Option<String>;
}

impl RiseCanonMessageList for view::Files {
    fn messages(&self) -> &[view::MessageInfo] {
        &self.messages
    }

    fn is_have_more(&self) -> Option<bool> {
        self.is_have_more
    }

    fn list_cursor(&self) -> Option<String> {
        None
    }
}

impl RiseCanonMessageList for view::Music {
    fn messages(&self) -> &[view::MessageInfo] {
        &self.messages
    }

    fn is_have_more(&self) -> Option<bool> {
        self.is_have_more
    }

    fn list_cursor(&self) -> Option<String> {
        None
    }
}

impl RiseCanonMessageList for view::Audio {
    fn messages(&self) -> &[view::MessageInfo] {
        &self.messages
    }

    fn is_have_more(&self) -> Option<bool> {
        self.is_have_more
    }

    fn list_cursor(&self) -> Option<String> {
        None
    }
}

impl RiseCanonMessageList for view::Gifs {
    fn messages(&self) -> &[view::MessageInfo] {
        &self.messages
    }

    fn is_have_more(&self) -> Option<bool> {
        self.is_have_more
    }

    fn list_cursor(&self) -> Option<String> {
        None
    }
}

impl RiseCanonMessageList for view::Favorites {
    fn messages(&self) -> &[view::MessageInfo] {
        &self.messages
    }

    fn is_have_more(&self) -> Option<bool> {
        self.is_have_more
    }

    fn list_cursor(&self) -> Option<String> {
        None
    }
}

impl RiseCanonMessageList for view::Pinned {
    fn messages(&self) -> &[view::MessageInfo] {
        &self.messages
    }

    fn is_have_more(&self) -> Option<bool> {
        self.is_have_more
    }

    fn list_cursor(&self) -> Option<String> {
        self.relative_id.clone()
    }
}

impl GetMessagesListGatewayDTO {
    pub fn from_canon_list<List: RiseCanonMessageList>(list: &List) -> anyhow::Result<Self> {
        Ok(Self {
            success: None,
            error: None,
            messages: Some(wire_messages(list.messages())?),
            is_have_more: list.is_have_more(),
            relative_id: list.list_cursor(),
            matches: None,
            total_count: None,
            chats: None,
        })
    }

    pub fn from_canon(found: &view::Found) -> anyhow::Result<Self> {
        Ok(Self {
            success: None,
            error: found.error.clone(),
            messages: Some(wire_messages(&found.messages)?),
            is_have_more: found.is_have_more,
            relative_id: found.relative_id.clone(),
            matches: Some(found.matches.iter().map(MessageSearchMatchWireDTO::from_canon).collect()),
            total_count: found.total_count,
            chats: None,
        })
    }
}

impl MessageSearchMatchWireDTO {
    pub fn from_canon(found: &proto::ProtoMessageSearchMatch) -> Self {
        fn ranges(items: &[proto::ProtoTextRange]) -> Vec<MessageSearchRangeWireDTO> {
            items.iter().map(|range| MessageSearchRangeWireDTO { start: range.start, end: range.end }).collect()
        }
        Self {
            message_id: Some(found.message_id.clone()),
            ranges: Some(ranges(&found.ranges)),
            snippet: Some(found.snippet.clone()),
            snippet_ranges: Some(ranges(&found.snippet_ranges)),
        }
    }
}

impl MessageSenderWireDTO {
    pub fn from_canon(sender: &proto::ProtoSenderShort) -> Self {
        Self {
            id: Some(sender.id.clone()),
            name: Some(sender.name.clone()),
            tag: Some(sender.tag.clone()),
            role: Some(sender.role.clone()),
            user_chat_id: Some(sender.user_chat_id.clone()),
            is_premium: Some(sender.is_premium),
            is_official: Some(sender.is_official),
            premium_badge_emoji: sender.premium_badge_emoji.clone(),
            name_color: sender.name_color.clone(),
            more: sender.more.as_ref().map(|more| MessageSenderMoreWireDTO {
                logo: Some(more.logo.clone()),
                banner: Some(more.banner.clone()),
                description: Some(more.description.clone()),
                level: Some(more.level),
                is_online: Some(more.is_online),
                last_seen: Some(more.last_seen as f64),
            }),
            is_deleted: Some(sender.is_deleted),
        }
    }

    pub fn from_canon_info(sender: &proto::ProtoSenderInfo) -> Self {
        Self {
            id: Some(sender.id.clone()),
            name: Some(sender.name.clone()),
            tag: Some(sender.tag.clone()),
            role: Some(sender.role.clone()),
            user_chat_id: Some(sender.user_chat_id.clone()),
            is_premium: Some(sender.is_premium),
            is_official: Some(sender.is_official),
            premium_badge_emoji: sender.premium_badge_emoji.clone(),
            name_color: sender.name_color.clone(),
            more: sender.more.as_ref().map(|more| MessageSenderMoreWireDTO {
                logo: Some(more.logo.clone()),
                banner: Some(more.banner.clone()),
                description: Some(more.description.clone()),
                level: Some(more.level),
                is_online: None,
                last_seen: None,
            }),
            is_deleted: Some(sender.is_deleted),
        }
    }
}

impl RiseStoredReadReceipts {
    pub fn from_canon(readers: &view::Readers) -> Self {
        Self {
            message_id: Some(readers.message_id.clone()),
            users: Some(readers.users.iter().map(MessageSenderWireDTO::from_canon_info).collect()),
            is_have_more: Some(readers.is_have_more),
            receipts: Some(
                readers
                    .receipts
                    .iter()
                    .map(|receipt| RiseStoredReadReceipt {
                        user: receipt.user.as_ref().map(MessageSenderWireDTO::from_canon_info),
                        read_at: Some(receipt.read_at),
                    })
                    .collect(),
            ),
            total_count: Some(readers.total_count),
            summary_kind: Some(readers.summary_kind.clone()),
            representative_read_at: readers.representative_read_at,
        }
    }
}

impl MessageMediaInfoWireDTO {
    pub fn from_canon(media: &proto::ProtoMediaInfo) -> Self {
        Self {
            media_type: Some(media.media_type.clone()),
            file_id: media.file_id.clone(),
            file_url: media.file_url.clone(),
            thumbnail_url: media.thumbnail_url.clone(),
            duration: media.duration.map(|duration| duration as f64),
            width: media.width,
            height: media.height,
            file_name: media.file_name.clone(),
            file_size: media.file_size,
            mime_type: media.mime_type.clone(),
            bitrate: media.bitrate,
            fps: media.fps.map(f64::from),
            codec: media.codec.clone(),
            waveform: media.waveform.as_deref().map(Self::waveform_wire),
            sticker_id: media.sticker_id.clone(),
            pack_id: media.pack_id.clone(),
            sticker_type: media.sticker_type.clone(),
            associated_emojis: Some(media.associated_emojis.clone()),
            is_uploading: None,
            upload_progress: None,
            uploaded_bytes: None,
            total_bytes: None,
            is_compressing: None,
            compression_progress: None,
            is_cancelled: None,
            local_preview_url: None,
            local_slot_index: None,
            has_spoiler: (media.has_spoiler == Some(true)).then_some(true),
            view_mode: media.view_mode.clone(),
            view_seconds: media.view_seconds,
            view_expires_at: media.view_expires_at,
            view_state: media.view_state.clone(),
        }
    }

    fn waveform_wire(bytes: &[u8]) -> String {
        bytes.iter().map(|byte| format!("{:.4}", f32::from(*byte) / 255.0)).collect::<Vec<String>>().join(",")
    }
}

impl MediaItemWireDTO {
    pub fn from_canon(item: &proto::ProtoMediaItemResponse) -> Self {
        Self {
            id: Some(item.id.clone()),
            message_id: Some(item.message_id.clone()),
            chat_id: Some(item.chat_id.clone()),
            sender_id: Some(item.sender_id.clone()),
            sender_name: Some(item.sender_name.clone()),
            sender: item.sender.as_ref().map(MessageSenderWireDTO::from_canon),
            caption: Some(item.caption.clone()),
            content_type: Some(item.content_type.clone()),
            created_at: Some(item.created_at as f64),
            timestamp: Some(item.timestamp as f64),
            media_info: item.media_info.as_ref().map(MessageMediaInfoWireDTO::from_canon),
            media_index: Some(item.media_index),
            total_media_in_message: Some(item.total_media_in_message),
            has_protected_content: Some(item.has_protected_content),
            topic_id: item.topic_id.clone().filter(|topic_id| !topic_id.is_empty()),
        }
    }
}

impl GetMediaMessagesGatewayDTO {
    pub fn from_canon(page: &proto::ProtoGetMediaMessagesResponse) -> Self {
        Self {
            success: None,
            error: None,
            media_items: Some(page.media_items.iter().map(MediaItemWireDTO::from_canon).collect()),
            is_have_more: page.is_have_more,
            next_cursor: page.next_cursor.clone(),
        }
    }
}

impl LinkMessageWireDTO {
    pub fn from_canon(item: &proto::ProtoLinkMessageItem) -> Self {
        Self {
            created_at: Some(item.created_at as f64),
            link: Some(item.link.clone()),
            message_id: Some(item.message_id.clone()),
            link_metadata: item.link_metadata.as_ref().map(|metadata| MessageLinkMetadataWireDTO {
                title: metadata.title.clone(),
                description: metadata.description.clone(),
                image_url: metadata.image_url.clone(),
                link_preview_type: metadata.link_preview_type.clone(),
            }),
            sender: item.sender.as_ref().map(MessageSenderWireDTO::from_canon),
        }
    }
}

impl GetLinkMessagesGatewayDTO {
    pub fn from_canon(page: &proto::ProtoGetLinkMessagesResponse) -> Self {
        Self {
            success: None,
            error: None,
            link_messages: Some(page.link_messages.iter().map(LinkMessageWireDTO::from_canon).collect()),
            is_have_more: page.is_have_more,
        }
    }
}

impl CreateMessageGatewayDTO {
    pub fn from_canon(created: &view::Created) -> anyhow::Result<Self> {
        Ok(Self {
            message_id: created.message_id.clone(),
            id: None,
            chat_id: created.chat_id.clone(),
            sender_id: created.sender_id.clone(),
            created_at: created.created_at.map(|value| value as f64),
            timestamp: created.timestamp.map(|value| value as f64),
            temp_id: created.temp_id.clone(),
            message: created.message.as_ref().map(MessageWireDTO::from_canon).transpose()?,
            messages: None,
            success: None,
            error: None,
        })
    }

    pub fn canon(payload: Option<&Value>) -> Option<CreateMessageGatewayDTO> {
        let created: view::Created = MessagePayloadCodec::decode(payload)?;
        CreateMessageGatewayDTO::from_canon(&created).ok()
    }
}

impl ForwardMessagesGatewayDTO {
    pub fn from_canon(forwarded: &view::Forwarded) -> Self {
        Self {
            success: None,
            error: None,
            forwarded_messages: Some(
                forwarded.forwarded_messages.iter().map(|(key, value)| (key.clone(), value.clone())).collect(),
            ),
            idempotency_key: forwarded.idempotency_key.clone(),
            all_succeeded: Some(forwarded.all_succeeded),
            results: Some(forwarded.results.iter().map(ForwardMessageResultWireDTO::from_canon).collect()),
        }
    }
}

impl ForwardMessageResultWireDTO {
    pub fn from_canon(result: &view::ForwardResult) -> Self {
        let details = result.error_details.as_ref().map(rise_json_value_from_canon);
        let retry_after = match details
            .as_ref()
            .and_then(RiseJSONValue::object_value)
            .and_then(|object| object.get(payload_key::RETRY_AFTER_S))
        {
            Some(RiseJSONValue::String(text)) => Some(text.clone()),
            Some(RiseJSONValue::Integer(number)) => Some(number.to_string()),
            _ => None,
        };
        Self {
            source_message_id: result.source_message_id.clone(),
            destination: result.destination.as_ref().map(|destination| ForwardMessageDestinationWireDTO {
                chat_id: destination.chat_id.clone(),
                topic_id: destination.topic_id.clone(),
            }),
            success: result.success,
            error: if result.error.is_empty() { None } else { Some(result.error.clone()) },
            error_code: result.error_code.clone(),
            error_details: result
                .error_details
                .as_ref()
                .map(|_| FanoutRefusalDetailsWireDTO { retry_after_s: retry_after }),
            retryable: result.retryable,
            message_id: result.message_id.clone(),
            message: result.message.as_ref().and_then(|message| MessageWireDTO::from_canon(message).ok()),
        }
    }
}

pub fn create_message_from_stored(body: &RiseStoredOutgoingMessage) -> rise_canon::rise_request::message::CreateMessage {
    rise_canon::rise_request::message::CreateMessage {
        content: body.content.clone(),
        content_type: body.content_type.clone(),
        caption: body.caption.clone(),
        chat_id: body.chat_id.clone(),
        document: body.document.clone(),
        encryption_info: None,
        entities: body.entities.clone(),
        forward_from_chat_id: body.forward_from_chat_id.clone(),
        forward_from_message_id: body.forward_from_message_id.clone(),
        is_encrypted: None,
        link_preview_type: body.link_preview_type.clone(),
        media_group_id: body.media_group_id.clone(),
        media_items: body.media_items.as_ref().map(|items| {
            items
                .iter()
                .map(|item| MediaItemsElement {
                    associated_emojis: item.associated_emojis.clone(),
                    bitrate: item.bitrate,
                    codec: item.codec.clone(),
                    duration: item.duration,
                    file_id: item.file_id.clone(),
                    file_name: item.file_name.clone(),
                    file_size: item.file_size,
                    file_url: item.file_url.clone(),
                    fps: item.fps,
                    has_spoiler: (item.has_spoiler == Some(true)).then_some(true),
                    height: item.height,
                    media_type: Some(item.media_type.clone()),
                    mime_type: item.mime_type.clone(),
                    pack_id: item.pack_id.clone(),
                    sticker_id: item.sticker_id.clone(),
                    sticker_type: item.sticker_type.clone(),
                    thumbnail_url: item.thumbnail_url.clone(),
                    variants: None,
                    view_mode: item.view_mode.clone(),
                    view_seconds: item.view_seconds,
                    waveform: item.waveform.clone(),
                    width: item.width,
                })
                .collect()
        }),
        original_content: Some(body.original_content.clone()),
        post_id: body.post_id,
        publish_to_shorts_feed: body.publish_to_shorts_feed,
        reply_markup: body.reply_markup.clone(),
        reply_to_id: body.reply_to_id.clone(),
        story_id: body.story_id,
        temp_id: body.temp_id.clone(),
        topic_id: body.topic_id.clone(),
        user_chat_id: body.user_chat_id.clone(),
        web_app_data: body.web_app_data.clone(),
    }
}

fn put(object: &mut BTreeMap<String, RiseJSONValue>, key: &str, value: Option<RiseJSONValue>) {
    if let Some(value) = value {
        object.insert(key.to_owned(), value);
    }
}

fn text(value: Option<String>) -> Option<RiseJSONValue> {
    value.map(RiseJSONValue::String)
}

fn positive(value: i64) -> Option<RiseJSONValue> {
    if value > 0 { Some(RiseJSONValue::Integer(value)) } else { None }
}

fn strings(values: Vec<String>) -> Option<RiseJSONValue> {
    Some(RiseJSONValue::Array(values.into_iter().map(RiseJSONValue::String).collect()))
}

impl RiseMessageWireCodec {
    pub fn canon_created_object(push: &RiseWirePushEnvelope) -> Option<BTreeMap<String, RiseJSONValue>> {
        if push.r#type != name::MESSAGE_CREATED {
            return None;
        }
        let data = push.data.data(false).ok()?;
        let Ok(RiseEvent::MessageCreated(event)) = RiseEvent::decode(&push.r#type, &data) else {
            return None;
        };
        if event.message_id.is_empty() {
            return None;
        }
        let mut object = Self::push_object(push);
        put(&mut object, bot_key::ID, text(Some(event.message_id)));
        put(&mut object, message_key::CHAT_ID, text(Some(event.chat_id)));
        put(&mut object, message_key::TOPIC_ID, text(event.topic_id));
        put(&mut object, message_key::TEMP_ID, text(event.temp_id));
        put(&mut object, message_key::SENDER_ID, text(Some(event.sender_id)));
        put(&mut object, message_key::CONTENT, text(Some(event.content)));
        put(&mut object, message_key::CONTENT_TYPE, text(Some(event.content_type)));
        put(&mut object, message_key::CREATED_AT, positive(event.created_at));
        put(&mut object, message_key::TIMESTAMP, positive(event.timestamp));
        put(&mut object, message_key::IS_MENTIONED_YOU, event.is_mentioned_you.map(RiseJSONValue::Bool));
        put(&mut object, message_key::IS_SYSTEM_MESSAGE, event.is_system_message.map(RiseJSONValue::Bool));
        put(&mut object, message_key::SYSTEM_MESSAGE_TYPE, text(event.system_message_type));
        put(&mut object, message_key::IS_BOT_MESSAGE, Some(RiseJSONValue::Bool(event.is_bot_message)));
        put(&mut object, message_key::VIA_BOT, text(event.via_bot));
        put(&mut object, message_key::VIA_BOT_TAG, text(event.via_bot_tag));
        Some(object)
    }

    pub fn canon_root(
        push: &RiseWirePushEnvelope,
        root: &BTreeMap<String, RiseJSONValue>,
    ) -> BTreeMap<String, RiseJSONValue> {
        let Ok(data) = push.data.data(false) else {
            return root.clone();
        };
        let Ok(event) = RiseEvent::decode(&push.r#type, &data) else {
            return root.clone();
        };
        let mut object = root.clone();
        match event {
            RiseEvent::MessageEdited(edit) => {
                put(&mut object, message_key::MESSAGE_ID, text(Some(edit.message_id)));
                put(&mut object, message_key::CHAT_ID, text(Some(edit.chat_id)));
                put(&mut object, message_key::TOPIC_ID, text(edit.topic_id.filter(|topic| !topic.is_empty())));
                put(&mut object, message_key::SENDER_ID, text(Some(edit.sender_id)));
                put(&mut object, message_key::NEW_CONTENT, text(Some(edit.new_content)));
                put(&mut object, message_key::ENTITIES, text(edit.entities));
                put(&mut object, message_key::CAPTION, text(edit.caption));
                put(&mut object, events_key::EDIT_TIMESTAMP, positive(edit.edit_timestamp));
                put(&mut object, message_key::REPLY_MARKUP, text(edit.reply_markup));
                put(&mut object, message_key::VIA_BOT, text(edit.via_bot));
            }
            RiseEvent::ChatRead(read) => {
                put(&mut object, message_key::CHAT_ID, text(Some(read.chat_id)));
                put(&mut object, message_key::USER_ID, text(Some(read.user_id)));
                put(&mut object, chat_key::LAST_READ_MESSAGE_ID, text(Some(read.last_read_message_id)));
                put(&mut object, events_key::LAST_READ_CREATED_AT, read.last_read_created_at.map(RiseJSONValue::Integer));
                put(
                    &mut object,
                    events_key::LAST_READ_SAME_MILLISECOND_MESSAGE_IDS,
                    strings(read.last_read_same_millisecond_message_ids),
                );
                put(&mut object, message_key::READ_MESSAGE_IDS, strings(read.read_message_ids));
                put(&mut object, events_key::READ_TIMESTAMP, Some(RiseJSONValue::Integer(read.read_timestamp)));
                put(&mut object, message_key::TOPIC_ID, text(read.topic_id));
                put(&mut object, message_key::CLEARED_COUNT, Some(RiseJSONValue::Integer(read.cleared_count)));
                put(&mut object, message_key::UNREAD_COUNT, Some(RiseJSONValue::Integer(read.unread_count)));
            }
            RiseEvent::ReactionAdded(reaction) | RiseEvent::ReactionRemoved(reaction) => {
                put(&mut object, events_key::EVENT_TYPE, text(Some(reaction.event_type)));
                put(&mut object, message_key::MESSAGE_ID, text(Some(reaction.message_id)));
                put(&mut object, message_key::CHAT_ID, text(Some(reaction.chat_id)));
                put(&mut object, message_key::TOPIC_ID, text(reaction.topic_id));
                put(&mut object, message_key::TIMESTAMP, Some(RiseJSONValue::Integer(reaction.timestamp)));
                put(&mut object, message_key::REACTION, text(reaction.reaction));
                put(&mut object, events_key::REMOVER_ID, text(reaction.remover_id));
                put(&mut object, message_key::REACTION_ID, text(reaction.reaction_id));
                put(&mut object, message_key::ACTOR_ID, text(reaction.actor_id));
            }
            _ => {}
        }
        object
    }
}

impl MessageLinkMetadataWireDTO {
    pub fn from_canon(reply: &GetLinkPreview) -> Self {
        Self {
            title: reply.title.clone(),
            description: reply.description.clone(),
            image_url: reply.image_url.clone(),
            link_preview_type: None,
        }
    }
}
