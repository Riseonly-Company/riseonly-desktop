use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use gpui::Context;
use rise_canon::rise_key::{
    bot as bot_key, comment as comment_key, events as events_key, file_storage_admin as file_storage_admin_key,
    message as message_key, payload as payload_key, stories as stories_key, upload as upload_key,
};
use rise_core::foundation::foundation_date::RiseClock;
use rise_core::foundation::foundation_file_url::is_local_absolute_path;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::core::lib::rise_local_key::RiseLocalKey;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::lib::sai::modules::sai_updater::sai_updater::SaiUpdaterWriteScope;
use crate::core::lib::sai::modules::sai_ws::sai_ws_optimistic_update::SaiWsOptimisticSuccessContext;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsSendError;
use crate::core::lib::sai::sai::Sai;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::bubble::message_sticker_view::{
    ChatAnimatedMediaPlaybackKind, ChatAnimatedMediaSourcePolicy,
};
use crate::modules::chat::components::tabs::chat_profile_tabs::chat_profile_source::ChatProfileMediaEntry;
use crate::modules::chat::shared::utils::voice::voice_message_disk_cache::VoiceMessageDiskCache;
use crate::modules::chat::shared::utils::voice::voice_message_metadata::VoiceMessageMetadata;
use crate::modules::chat::stores::chat_stores_hooks::chat_stores_share_hooks;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatType;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageDeliveryStatus, MessageMediaItem, MessageMediaKind,
    MessageMediaViewLimit, MessageNormalizedEntities, MessageSharedPost, MessageSharedPostAuthor, MessageStickerKind, MessageStoryReply,
    MessageStoryReplyAuthor, MessageThreadOrder, RemoteMediaURL,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{
    CreateMessageGatewayDTO, ForwardMessagesGatewayDTO, MessageBotReplyMarkupWireDTO, MessageBotWebAppDataWireDTO,
    MessagePayloadCodec, MessageWaveformCodec, MessageWireDTO, RiseStoredOutgoingMedia, RiseStoredOutgoingMessage,
};
use crate::modules::chat::stores::message::message_interactions::share_destination_hooks::ShareSheetDestination;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItem, MediaPickerItemKind};
use crate::modules::media_picker::editor::media_reedit_store::MediaReeditStore;
use crate::modules::media_picker::export::outgoing_media_compressor::OutgoingMediaCompressorResult;
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;
use crate::modules::share_extension::send_handoff::share_send_handoff_models::RiseStoredSharePayloadResult;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{
    GifItem, StickerAssetType, StickerItem,
};

const TEMP_FLAG_KEY: &str = "isTemp";
const MESSAGE_LOG_NAME: &str = "Message";
const REEDIT_LOG_NAME: &str = "Reedit";
const WEB_APP_SUBMISSION_DIGEST_DOMAIN: &str = "riseonly.webapp.submission.v1";
const WEB_APP_SUBMISSION_TEMP_PREFIX: &str = "webapp_";
const UPLOAD_PROGRESS_FINAL_THRESHOLD: f64 = 99.5;
const UPLOAD_PROGRESS_THROTTLE_SECONDS: f64 = 0.2;

struct MessagesChatState {
    messages: Vec<GetMessageMessage>,
}

enum CreatedMessageEnvelope<'a> {
    Gateway(&'a CreateMessageGatewayDTO),
    Dictionary(&'a Map<String, Value>),
}

enum MessageWireRuntime {}

thread_local! {
    static OUTGOING_WIRE_TEMP_BY_REQUEST: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

impl MessageWireRuntime {
    fn outgoing_wire_temp_by_request<R>(body: impl FnOnce(&mut HashMap<String, String>) -> R) -> R {
        OUTGOING_WIRE_TEMP_BY_REQUEST.with_borrow_mut(body)
    }
}

fn put(map: &mut Map<String, Value>, key: &str, value: impl Into<Value>) {
    map.insert(key.to_owned(), value.into());
}

fn string_field<'a>(map: &'a Map<String, Value>, key: &str) -> Option<&'a str> {
    map.get(key).and_then(Value::as_str)
}

fn bool_field(map: &Map<String, Value>, key: &str) -> Option<bool> {
    map.get(key).and_then(Value::as_bool)
}

fn number_f64(map: &Map<String, Value>, key: &str) -> Option<f64> {
    map.get(key).and_then(Value::as_f64)
}

fn number_i64(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| value.as_f64().map(|number| number as i64))
}

fn int_val(value: Option<&Value>) -> i64 {
    match value {
        Some(Value::Number(_)) => value.and_then(number_i64).unwrap_or(0),
        Some(Value::String(text)) => text.parse::<i64>().unwrap_or(0),
        _ => 0,
    }
}

fn string_array(value: Option<&Value>) -> Option<Vec<String>> {
    value?.as_array()?.iter().map(|item| item.as_str().map(str::to_owned)).collect()
}

fn dictionary_array(value: Option<&Value>) -> Option<Vec<Map<String, Value>>> {
    value?.as_array()?.iter().map(|item| item.as_object().cloned()).collect()
}

fn json_container(raw: &str) -> Option<Value> {
    serde_json::from_str::<Value>(raw).ok().filter(|value| value.is_object() || value.is_array())
}

fn json_dictionary(raw: &str) -> Option<Map<String, Value>> {
    match json_container(raw) {
        Some(Value::Object(dictionary)) => Some(dictionary),
        _ => None,
    }
}

fn slot_position(file_index: i64, count: usize) -> Option<usize> {
    usize::try_from(file_index).ok().filter(|position| *position < count)
}

fn absolute_string(path: &Path) -> String {
    if path.as_os_str().is_empty() {
        return String::new();
    }
    Url::from_file_path(path)
        .map(|url| url.absolute_string().to_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

impl MessageServicesStore {
    pub fn apply_shared_payload_success(
        &mut self,
        result: &RiseStoredSharePayloadResult,
        destination: &ShareSheetDestination,
        cx: &mut Context<Self>,
    ) {
        let (true, Some(wire)) = (result.success, result.message.as_ref()) else {
            return;
        };

        let mut message = wire.to_message();
        message.chat_id = destination.chat_id.clone();
        message.topic_id = destination.topic_id.clone();

        let mut payload = Self::forward_message_payload(wire).unwrap_or_else(|| self.wire_to_dict_from(&message));
        put(&mut payload, bot_key::ID, message.id.clone());
        put(&mut payload, bot_key::CHAT_ID, destination.chat_id.clone());
        if let Some(topic_id) = &destination.topic_id {
            put(&mut payload, bot_key::TOPIC_ID, topic_id.clone());
        } else {
            payload.remove(message_key::TOPIC_ID);
        }

        let payload = Value::Object(payload);
        ChatServicesStore::shared(cx)
            .update(cx, |chats, cx| chats.message_created_success_handler(Some(&payload), cx));
        self.message_created_thread_handler(Some(&payload), cx);
    }

    pub fn apply_forward_messages_success(&mut self, dto: &ForwardMessagesGatewayDTO, cx: &mut Context<Self>) {
        let mut successful = dto
            .results
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(|result| result.success && result.message.is_some())
            .collect::<Vec<_>>();
        let order = |message: &Option<MessageWireDTO>| {
            message
                .as_ref()
                .map(|wire| MessageThreadOrder::normalize(wire.created_at.or(wire.timestamp).unwrap_or(0.0)))
                .unwrap_or(0.0)
        };
        successful.sort_by(|lhs_result, rhs_result| {
            order(&lhs_result.message)
                .total_cmp(&order(&rhs_result.message))
                .then_with(|| lhs_result.source_message_id.cmp(&rhs_result.source_message_id))
        });

        let mut donated_destinations = HashSet::<String>::new();
        for result in successful {
            let Some(wire) = result.message.as_ref() else {
                continue;
            };
            let mut message = wire.to_message();
            let Some(wire_destination) = result.destination.as_ref() else {
                continue;
            };
            let destination_chat_id = trimming_whitespaces_and_newlines(&wire_destination.chat_id).to_owned();
            if destination_chat_id.is_empty() {
                continue;
            }
            let destination_topic_id = wire_destination
                .topic_id
                .as_deref()
                .map(trimming_whitespaces_and_newlines)
                .filter(|topic_id| !topic_id.is_empty())
                .map(str::to_owned);
            message.chat_id = destination_chat_id.clone();
            message.topic_id = destination_topic_id;

            let mut payload = Self::forward_message_payload(wire).unwrap_or_else(|| self.wire_to_dict_from(&message));
            put(&mut payload, bot_key::ID, message.id.clone());
            put(&mut payload, bot_key::CHAT_ID, destination_chat_id.clone());
            if let Some(topic_id) = message.topic_id.as_ref().filter(|topic_id| !topic_id.is_empty()) {
                put(&mut payload, bot_key::TOPIC_ID, topic_id.clone());
            } else {
                payload.remove(message_key::TOPIC_ID);
            }

            let payload = Value::Object(payload);
            ChatServicesStore::shared(cx)
                .update(cx, |chats, cx| chats.message_created_success_handler(Some(&payload), cx));
            self.message_created_thread_handler(Some(&payload), cx);
            donated_destinations.insert(destination_chat_id);
        }
        if let Some(hooks) = chat_stores_share_hooks() {
            for chat_id in &donated_destinations {
                (hooks.donate_outgoing_interaction)(chat_id, cx);
            }
        }
    }

    fn forward_message_payload(wire: &MessageWireDTO) -> Option<Map<String, Value>> {
        match serde_json::to_value(wire).ok()? {
            Value::Object(payload) => Some(payload),
            _ => None,
        }
    }

    pub fn clear_outgoing_wire_temps_for_account_transition(&mut self) {
        MessageWireRuntime::outgoing_wire_temp_by_request(HashMap::clear);
    }

    pub fn web_app_submission_temp_id(&self, init_data: &str, data: &str, button_text: Option<&str>) -> String {
        let mut material = Vec::<u8>::new();
        Self::append_web_app_submission_digest_field(WEB_APP_SUBMISSION_DIGEST_DOMAIN.as_bytes(), &mut material);
        Self::append_web_app_submission_digest_field(init_data.as_bytes(), &mut material);
        Self::append_web_app_submission_digest_field(data.as_bytes(), &mut material);
        Self::append_web_app_submission_digest_field(button_text.unwrap_or("").as_bytes(), &mut material);
        let digest = Sha256::digest(&material);
        let hex = digest.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        format!("{WEB_APP_SUBMISSION_TEMP_PREFIX}{hex}")
    }

    fn append_web_app_submission_digest_field(field: &[u8], material: &mut Vec<u8>) {
        material.extend_from_slice(&(field.len() as u64).to_be_bytes());
        material.extend_from_slice(field);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_optimistic_message_data(
        &mut self,
        outbound: &Value,
        chat_id: &str,
        temp_id: &str,
        local_files: &[RiseOutgoingMediaFile],
        picker_items: &[MediaPickerItem],
        voice_metadata_slots: &[VoiceMessageMetadata],
        cx: &mut Context<Self>,
    ) -> Map<String, Value> {
        let body = outbound.as_object().cloned().unwrap_or_default();
        let auth = AuthServicesStore::shared(cx);
        let profile = auth.read(cx).profile().cloned();
        let my_user_id = auth.read(cx).message_author_user_id_for_outgoing();
        let now = RiseClock::now().time_interval_since_1970();

        let chats = ChatServicesStore::shared(cx);
        let channel_chat = chats.read(cx).chat_info(chat_id, cx).filter(|chat| chat.r#type == ChatType::Channel);
        let mut sender = Map::new();
        if let Some(channel) = &channel_chat {
            put(&mut sender, bot_key::ID, channel.id.clone());
            put(&mut sender, bot_key::NAME, channel.title.clone());
            let mut more = Map::new();
            put(&mut more, bot_key::LOGO, channel.logo_url.clone().unwrap_or_default());
            put(&mut sender, bot_key::MORE, more);
        } else if let Some(profile) = &profile {
            let sender_id = if my_user_id.is_empty() { profile.id.clone() } else { my_user_id.clone() };
            put(&mut sender, bot_key::ID, sender_id);
            put(&mut sender, bot_key::NAME, profile.name.clone());
            put(&mut sender, bot_key::TAG, profile.tag.clone());
            let mut more = Map::new();
            put(&mut more, bot_key::LOGO, profile.avatar_url.clone().unwrap_or_else(|| profile.more.logo.clone()));
            put(&mut sender, bot_key::MORE, more);
            put(&mut sender, bot_key::IS_PREMIUM, profile.more.is_premium);
        } else if !my_user_id.is_empty() {
            put(&mut sender, bot_key::ID, my_user_id.clone());
        }

        let mut media_placeholders = Vec::<Value>::new();
        if !local_files.is_empty() {
            let fallback_content_type = string_field(&body, bot_key::CONTENT_TYPE).unwrap_or("image");
            media_placeholders = local_files
                .iter()
                .enumerate()
                .map(|(index, local_file)| {
                    Value::Object(Self::optimistic_media_placeholder(
                        index,
                        local_file,
                        picker_items.get(index),
                        voice_metadata_slots.get(index),
                        fallback_content_type,
                    ))
                })
                .collect();
        } else if let Some(body_items) =
            dictionary_array(body.get(bot_key::MEDIA_ITEMS)).filter(|items| !items.is_empty())
        {
            media_placeholders = body_items
                .into_iter()
                .enumerate()
                .map(|(index, mut slot)| {
                    if string_field(&slot, bot_key::ID).is_none_or(str::is_empty) {
                        let slot_id = string_field(&slot, bot_key::STICKER_ID)
                            .map(str::to_owned)
                            .unwrap_or_else(|| format!("{temp_id}-{index}"));
                        put(&mut slot, bot_key::ID, slot_id);
                    }
                    if !slot.get(RiseLocalKey::LOCAL_SLOT_INDEX).is_some_and(Value::is_number) {
                        put(&mut slot, RiseLocalKey::LOCAL_SLOT_INDEX, index as i64);
                    }
                    Value::Object(slot)
                })
                .collect();
        }

        let has_attachments = !media_placeholders.is_empty();
        let body_value = |key: &str, fallback: &str| body.get(key).cloned().unwrap_or_else(|| Value::from(fallback));

        let mut dict = Map::new();
        put(&mut dict, bot_key::ID, temp_id);
        put(&mut dict, TEMP_FLAG_KEY, true);
        put(&mut dict, bot_key::CHAT_ID, chat_id);
        put(
            &mut dict,
            bot_key::SENDER_ID,
            channel_chat.as_ref().map(|chat| chat.id.clone()).unwrap_or_else(|| my_user_id.clone()),
        );
        put(
            &mut dict,
            bot_key::SENDER_NAME,
            channel_chat
                .as_ref()
                .map(|chat| chat.title.clone())
                .or_else(|| profile.as_ref().map(|profile| profile.name.clone()))
                .unwrap_or_default(),
        );
        put(&mut dict, bot_key::CONTENT, body_value(bot_key::CONTENT, ""));
        put(
            &mut dict,
            bot_key::ORIGINAL_CONTENT,
            body.get(bot_key::ORIGINAL_CONTENT).cloned().unwrap_or_else(|| body_value(bot_key::CONTENT, "")),
        );
        put(&mut dict, bot_key::CONTENT_TYPE, body_value(bot_key::CONTENT_TYPE, "text"));
        put(&mut dict, bot_key::CAPTION, body_value(bot_key::CAPTION, ""));
        put(&mut dict, bot_key::CREATED_AT, now);
        put(&mut dict, bot_key::TIMESTAMP, now);
        put(&mut dict, bot_key::IS_READ, false);
        put(&mut dict, bot_key::IS_PINNED, false);
        put(&mut dict, bot_key::IS_SYSTEM_MESSAGE, false);
        put(&mut dict, bot_key::HAS_ATTACHMENTS, has_attachments);
        put(&mut dict, bot_key::HAS_REACTIONS, false);
        put(&mut dict, bot_key::MEDIA_ITEMS, media_placeholders);
        put(&mut dict, bot_key::SENDER, sender);
        put(&mut dict, bot_key::REACTIONS, Vec::<Value>::new());

        if let Some(preview) = body.get(bot_key::LINK_PREVIEW) {
            put(&mut dict, bot_key::LINK_PREVIEW, preview.clone());
        }
        if let Some(entities) = string_field(&body, bot_key::ENTITIES).filter(|entities| !entities.is_empty()) {
            put(&mut dict, bot_key::ENTITIES, entities);
        }
        if let Some(markup) = string_field(&body, bot_key::REPLY_MARKUP).and_then(json_container) {
            put(&mut dict, bot_key::REPLY_MARKUP, markup);
        } else if let Some(markup) = body.get(bot_key::REPLY_MARKUP) {
            put(&mut dict, bot_key::REPLY_MARKUP, markup.clone());
        }
        if let Some(via_bot) = string_field(&body, bot_key::VIA_BOT).filter(|via_bot| !via_bot.is_empty()) {
            put(&mut dict, bot_key::VIA_BOT, via_bot);
            put(&mut dict, bot_key::IS_BOT_MESSAGE, true);
        } else if let Some(is_bot_message) = bool_field(&body, bot_key::IS_BOT_MESSAGE) {
            put(&mut dict, bot_key::IS_BOT_MESSAGE, is_bot_message);
        }

        if let Some(info) = string_field(&body, bot_key::SHARED_POST_INFO).and_then(json_dictionary) {
            put(&mut dict, payload_key::SHARED_POST, info);
        }

        if let Some(info) = string_field(&body, bot_key::STORY_REPLY_INFO).and_then(json_dictionary) {
            put(&mut dict, events_key::STORY_REPLY, info);
        }

        if let Some(topic_id) = string_field(&body, bot_key::TOPIC_ID).filter(|topic_id| !topic_id.is_empty()) {
            put(&mut dict, bot_key::TOPIC_ID, topic_id);
        }
        if let Some(mut value) = string_field(&body, bot_key::WEB_APP_DATA).and_then(json_dictionary) {
            value.remove(bot_key::INIT_DATA);
            put(&mut dict, bot_key::WEB_APP_DATA, value);
        }
        dict
    }

    #[allow(clippy::too_many_arguments)]
    pub fn append_optimistic_from_coordinator_row(
        &mut self,
        row: &Map<String, Value>,
        request_temp_id: &str,
        chat_id: &str,
        picker_items: &[MediaPickerItem],
        local_files: &[RiseOutgoingMediaFile],
        voice_metadata_slots: &[VoiceMessageMetadata],
        cx: &mut Context<Self>,
    ) {
        let Some(temp_id) = string_field(row, bot_key::ID).filter(|temp_id| !temp_id.is_empty()) else {
            return;
        };

        MessageWireRuntime::outgoing_wire_temp_by_request(|temps| {
            temps.insert(request_temp_id.to_owned(), temp_id.to_owned());
        });
        let optimistic = self.map_optimistic_temp(row, chat_id, picker_items, local_files, voice_metadata_slots);

        self.mutate_messages_list(
            chat_id,
            SaiUpdaterWriteScope::Cache,
            |messages| {
                if let Some(index) = messages.iter().position(|message| message.id == temp_id) {
                    messages[index] = optimistic.clone();
                } else {
                    messages.push(optimistic.clone());
                }
                *messages = MessageThreadOrder::chronological(std::mem::take(messages));
            },
            cx,
        );
    }

    pub fn outgoing_wire_temp_id(&self, request_temp_id: &str) -> Option<String> {
        MessageWireRuntime::outgoing_wire_temp_by_request(|temps| temps.get(request_temp_id).cloned())
    }

    pub fn clear_outgoing_wire_temp(&mut self, request_temp_id: &str) {
        MessageWireRuntime::outgoing_wire_temp_by_request(|temps| {
            temps.remove(request_temp_id);
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn patch_outgoing_media_upload_progress(
        &mut self,
        chat_id: &str,
        message_id: &str,
        file_index: i64,
        is_compressing: bool,
        compression_progress: f64,
        upload_progress: f64,
        uploaded_bytes: i64,
        total_bytes: Option<i64>,
        cx: &mut Context<Self>,
    ) {
        if self.should_throttle_progress(&format!("{message_id}#{file_index}"), is_compressing, upload_progress) {
            return;
        }

        let positive_total_bytes = total_bytes.filter(|total_bytes| *total_bytes > 0);
        self.direct_patch_message_row(
            chat_id,
            message_id,
            SaiUpdaterWriteScope::Cache,
            false,
            Some(&mut |message: &mut GetMessageMessage| {
                let Some(position) = slot_position(file_index, message.media_items.len()) else {
                    return;
                };
                let item = &mut message.media_items[position];
                item.is_compressing = is_compressing;
                item.compression_progress = compression_progress;
                item.is_uploading = !is_compressing;
                item.upload_progress = upload_progress;
                item.uploaded_bytes = uploaded_bytes;
                if let Some(total_bytes) = positive_total_bytes {
                    item.total_bytes = total_bytes;
                    item.file_size = Some(total_bytes);
                }
            }),
            |row: &mut Map<String, Value>| {
                let mut media = MessagePayloadCodec::array_of_dictionaries(row.get(bot_key::MEDIA_ITEMS));
                let Some(position) = slot_position(file_index, media.len()) else {
                    return;
                };

                let slot = &mut media[position];
                put(slot, RiseLocalKey::IS_COMPRESSING, is_compressing);
                put(slot, RiseLocalKey::COMPRESSION_PROGRESS, compression_progress);
                put(slot, RiseLocalKey::IS_UPLOADING, !is_compressing);
                put(slot, RiseLocalKey::UPLOAD_PROGRESS, upload_progress);
                put(slot, RiseLocalKey::UPLOADED_BYTES, uploaded_bytes);
                if let Some(total_bytes) = positive_total_bytes {
                    put(slot, file_storage_admin_key::TOTAL_BYTES, total_bytes);
                    put(slot, bot_key::FILE_SIZE, total_bytes);
                }
                put(row, bot_key::MEDIA_ITEMS, media.into_iter().map(Value::Object).collect::<Vec<_>>());
            },
            cx,
        );
    }

    pub fn should_throttle_progress(&mut self, key: &str, is_compressing: bool, upload_progress: f64) -> bool {
        let now = RiseClock::now().time_interval_since_1970();
        let is_final = upload_progress >= UPLOAD_PROGRESS_FINAL_THRESHOLD;
        let previous = self.upload_progress_throttle.get(key).copied();
        let state_changed = previous.is_none_or(|(_, compressing, _)| compressing != is_compressing);
        let restarted = previous.is_some_and(|(_, _, progress)| upload_progress < progress);
        if let Some((at, _, _)) = previous
            && !state_changed
            && !restarted
            && !is_final
            && now - at < UPLOAD_PROGRESS_THROTTLE_SECONDS
        {
            return true;
        }
        if is_final {
            self.upload_progress_throttle.remove(key);
        } else {
            self.upload_progress_throttle.insert(key.to_owned(), (now, is_compressing, upload_progress));
        }
        false
    }

    pub fn apply_outgoing_media_compression_result(
        &mut self,
        chat_id: &str,
        message_id: &str,
        file_index: i64,
        result: &OutgoingMediaCompressorResult,
        cx: &mut Context<Self>,
    ) {
        let local_preview = result.url.absolute_string().to_owned();
        let is_image = result.content_type.starts_with("image/");

        self.direct_patch_message_row(
            chat_id,
            message_id,
            SaiUpdaterWriteScope::Cache,
            false,
            Some(&mut |message: &mut GetMessageMessage| {
                let Some(position) = slot_position(file_index, message.media_items.len()) else {
                    return;
                };

                let item = &mut message.media_items[position];
                item.is_compressing = false;
                item.compression_progress = 100.0;
                item.is_uploading = true;
                item.upload_progress = 0.0;
                item.uploaded_bytes = 0;
                item.total_bytes = result.byte_count;
                item.file_size = Some(result.byte_count);
                item.mime_type = Some(result.content_type.clone());
                if let Some(width) = result.pixel_width.filter(|width| *width > 0) {
                    item.width = width as f32;
                }
                if let Some(height) = result.pixel_height.filter(|height| *height > 0) {
                    item.height = height as f32;
                }
                if is_image {
                    item.url = local_preview.clone();
                    item.thumbnail_url = Some(local_preview.clone());
                    item.local_preview_url = Some(local_preview.clone());
                }
            }),
            |row: &mut Map<String, Value>| {
                let mut media = MessagePayloadCodec::array_of_dictionaries(row.get(bot_key::MEDIA_ITEMS));
                let Some(position) = slot_position(file_index, media.len()) else {
                    return;
                };

                let slot = &mut media[position];
                put(slot, RiseLocalKey::IS_COMPRESSING, false);
                put(slot, RiseLocalKey::COMPRESSION_PROGRESS, 100);
                put(slot, RiseLocalKey::IS_UPLOADING, true);
                put(slot, RiseLocalKey::UPLOAD_PROGRESS, 0);
                put(slot, RiseLocalKey::UPLOADED_BYTES, 0);
                put(slot, file_storage_admin_key::TOTAL_BYTES, result.byte_count);
                put(slot, bot_key::FILE_SIZE, result.byte_count);
                put(slot, bot_key::MIME_TYPE, result.content_type.clone());
                if let Some(width) = result.pixel_width.filter(|width| *width > 0) {
                    put(slot, bot_key::WIDTH, width);
                }
                if let Some(height) = result.pixel_height.filter(|height| *height > 0) {
                    put(slot, bot_key::HEIGHT, height);
                }
                if is_image {
                    put(slot, bot_key::FILE_URL, local_preview.clone());
                    put(slot, bot_key::THUMBNAIL_URL, local_preview.clone());
                    put(slot, RiseLocalKey::LOCAL_PREVIEW_URL, local_preview.clone());
                }
                put(row, bot_key::MEDIA_ITEMS, media.into_iter().map(Value::Object).collect::<Vec<_>>());
            },
            cx,
        );
    }

    pub fn estimated_outgoing_file_bytes(
        &mut self,
        chat_id: &str,
        message_id: &str,
        file_index: i64,
        cx: &mut Context<Self>,
    ) -> i64 {
        let messages = self.messages(chat_id, cx);
        let Some(message) = messages.iter().find(|message| message.id == message_id) else {
            return 0;
        };
        let Some(position) = slot_position(file_index, message.media_items.len()) else {
            return 0;
        };
        let item = &message.media_items[position];
        if item.total_bytes > 0 {
            return item.total_bytes;
        }
        item.file_size.filter(|file_size| *file_size > 0).unwrap_or(0)
    }

    pub fn apply_uploaded_media_slot(
        &mut self,
        chat_id: &str,
        message_id: &str,
        file_index: i64,
        slot: &Map<String, Value>,
        local_file: &RiseOutgoingMediaFile,
        cx: &mut Context<Self>,
    ) {
        self.mutate_messages_list(
            chat_id,
            SaiUpdaterWriteScope::Cache,
            |messages| {
                let Some(message_position) = messages.iter().position(|message| message.id == message_id) else {
                    return;
                };
                let Some(position) = slot_position(file_index, messages[message_position].media_items.len()) else {
                    return;
                };
                let previous = messages[message_position].media_items[position].clone();
                let is_voice = messages[message_position].content_type == MessageContentType::Voice
                    || previous.kind == MessageMediaKind::Voice;
                let mut mapped = Self::media_item_from_optimistic_slot(slot, position, message_id, Some(local_file));
                mapped.is_uploading = false;
                mapped.is_compressing = false;
                mapped.upload_progress = 100.0;
                if previous.has_spoiler {
                    mapped.has_spoiler = true;
                }
                if mapped.view_limit.is_none() {
                    mapped.view_limit = previous.view_limit.clone();
                }
                if mapped.width <= 0.0 && previous.width > 0.0 {
                    mapped.width = previous.width;
                }
                if mapped.height <= 0.0 && previous.height > 0.0 {
                    mapped.height = previous.height;
                }
                if !is_voice {
                    if matches!(mapped.kind, MessageMediaKind::Video | MessageMediaKind::VideoNote)
                        && let Some(thumb) = previous.thumbnail_url.as_deref()
                        && Self::is_local_or_pending_media_url(thumb)
                        && !MessageMediaItem::is_local_video_file(thumb)
                    {
                        mapped.thumbnail_url = Some(thumb.to_owned());
                    }
                    if mapped.kind == MessageMediaKind::Document
                        && mapped.thumbnail_url.is_none()
                        && let Some(thumb) = previous.thumbnail_url.as_deref()
                        && MessageMediaItem::is_local_preview_url(thumb)
                        && MessageMediaItem::is_decodable_image_source(thumb)
                    {
                        mapped.thumbnail_url = Some(thumb.to_owned());
                    }
                }
                let local_url = absolute_string(&local_file.url);
                mapped.local_preview_url = previous
                    .resolved_local_preview_url()
                    .or_else(|| MessageMediaItem::is_local_preview_url(&local_url).then_some(local_url));
                messages[message_position].media_items[position] = mapped;
            },
            cx,
        );
    }

    pub fn extract_uploaded_media_slot(
        &self,
        done: &Map<String, Value>,
        local_file: &RiseOutgoingMediaFile,
        picker_item: Option<&MediaPickerItem>,
        voice_metadata: Option<&VoiceMessageMetadata>,
    ) -> Map<String, Value> {
        let url = string_field(done, bot_key::FILE_URL).or_else(|| string_field(done, bot_key::URL)).unwrap_or("");
        let thumbnail_url = RemoteMediaURL::optional_normalized(string_field(done, bot_key::THUMBNAIL_URL));
        let media_type = match local_file.media_type.as_deref().filter(|preferred| !preferred.is_empty()) {
            Some(preferred) => preferred.to_owned(),
            None if voice_metadata.is_some() => "voice".to_owned(),
            None => string_field(done, bot_key::MEDIA_TYPE)
                .map(str::to_owned)
                .unwrap_or_else(|| Self::guess_media_type(&local_file.content_type).to_owned()),
        };
        let local_byte_count = Self::local_file_byte_count(&local_file.url);
        let uploaded_byte_count = Self::positive_byte_count(done.get(bot_key::FILE_SIZE)).unwrap_or(local_byte_count);
        let file_name = if media_type == MessageContentType::Document.raw_value() {
            local_file.filename.clone()
        } else {
            string_field(done, bot_key::FILE_NAME).map(str::to_owned).unwrap_or_else(|| local_file.filename.clone())
        };
        let mut slot = Map::new();
        put(&mut slot, bot_key::MEDIA_TYPE, media_type);
        put(&mut slot, bot_key::FILE_URL, url);
        put(&mut slot, bot_key::FILE_NAME, file_name);
        put(
            &mut slot,
            bot_key::MIME_TYPE,
            done.get(bot_key::MIME_TYPE).cloned().unwrap_or_else(|| Value::from(local_file.content_type.clone())),
        );
        if let Some(file_id) = string_field(done, bot_key::FILE_ID).filter(|file_id| !file_id.is_empty()) {
            put(&mut slot, bot_key::FILE_ID, file_id);
        } else if let Some(key) = string_field(done, upload_key::FILE_KEY).filter(|key| !key.is_empty()) {
            let stem = Path::new(key).file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default();
            if !stem.is_empty() {
                put(&mut slot, bot_key::FILE_ID, stem);
            }
        }
        if let Some(thumbnail_url) = thumbnail_url {
            put(&mut slot, bot_key::THUMBNAIL_URL, thumbnail_url);
        }
        if let Some(local_preview_url) = string_field(done, RiseLocalKey::LOCAL_PREVIEW_URL)
            .filter(|local_preview_url| MessageMediaItem::is_local_preview_url(local_preview_url))
        {
            put(&mut slot, RiseLocalKey::LOCAL_PREVIEW_URL, local_preview_url);
        }
        if let Some(width) = done.get(bot_key::WIDTH) {
            put(&mut slot, bot_key::WIDTH, width.clone());
        }
        if let Some(height) = done.get(bot_key::HEIGHT) {
            put(&mut slot, bot_key::HEIGHT, height.clone());
        }
        if uploaded_byte_count > 0 {
            put(&mut slot, bot_key::FILE_SIZE, uploaded_byte_count);
        }
        if let Some(duration) = done.get(bot_key::DURATION) {
            put(&mut slot, bot_key::DURATION, duration.clone());
        }
        if let Some(waveform) = string_field(done, bot_key::WAVEFORM).filter(|waveform| !waveform.is_empty()) {
            put(&mut slot, bot_key::WAVEFORM, waveform);
        } else if let Some(wire) = voice_metadata
            .and_then(|voice_metadata| RiseStoredOutgoingMedia::waveform_wire_string(&voice_metadata.waveform))
        {
            put(&mut slot, bot_key::WAVEFORM, wire);
        }
        if !slot.contains_key(bot_key::DURATION)
            && let Some(voice_metadata) = voice_metadata
        {
            put(&mut slot, bot_key::DURATION, voice_metadata.duration_seconds);
        }
        if let Some(picker_item) = picker_item
            && let Some(audio) = &picker_item.audio_payload
        {
            put(&mut slot, bot_key::FILE_NAME, picker_item.audio_file_name().unwrap_or_default());
            put(&mut slot, bot_key::DURATION, audio.duration.round() as i64);
            put(&mut slot, RiseLocalKey::LOCAL_PREVIEW_URL, absolute_string(&local_file.url));
            if let Some(cover_url) = &audio.cover_url {
                put(&mut slot, bot_key::THUMBNAIL_URL, cover_url.absolute_string());
            }
        }
        put(&mut slot, RiseLocalKey::IS_UPLOADING, false);
        put(&mut slot, RiseLocalKey::IS_COMPRESSING, false);
        put(&mut slot, RiseLocalKey::UPLOAD_PROGRESS, 100);
        put(&mut slot, RiseLocalKey::COMPRESSION_PROGRESS, 100);
        slot
    }

    pub fn finalize_audio_uploaded_slots(
        &self,
        uploaded: &[Value],
        audio_index: i64,
        cover_index: Option<i64>,
    ) -> Vec<Value> {
        let Some(mut audio) = slot_position(audio_index, uploaded.len())
            .and_then(|position| MessagePayloadCodec::dictionary(uploaded.get(position)))
        else {
            return uploaded.to_vec();
        };

        if let Some(position) = cover_index.and_then(|cover_index| slot_position(cover_index, uploaded.len()))
            && let Some(cover) = MessagePayloadCodec::dictionary(uploaded.get(position))
            && let Some(cover_url) = string_field(&cover, bot_key::FILE_URL)
                .or_else(|| string_field(&cover, bot_key::URL))
                .filter(|cover_url| !cover_url.is_empty())
        {
            put(&mut audio, bot_key::THUMBNAIL_URL, cover_url);
        }
        vec![Value::Object(audio)]
    }

    pub fn extract_real_message_data(
        &self,
        any: Option<&Value>,
        context: Option<&SaiWsOptimisticSuccessContext>,
    ) -> Map<String, Value> {
        let temp_id = context.map(|context| context.temp_id.as_str());
        if let Some(dict) = MessagePayloadCodec::dictionary(any) {
            if let Some(nested) = MessagePayloadCodec::dictionary(dict.get(bot_key::MESSAGE)) {
                return Self::finalize_extracted_message_row(&nested, temp_id, &dict);
            }
            if dict.contains_key(bot_key::ID) || dict.contains_key(bot_key::MESSAGE_ID) {
                return Self::finalize_extracted_message_row(&dict, temp_id, &dict);
            }
        }
        if let Some(dto) = CreateMessageGatewayDTO::canon(any)
            && let Some(message) = dto.message.as_ref().or_else(|| dto.messages.as_ref().and_then(|list| list.first()))
        {
            let row = self.wire_to_dict(message);
            return Self::finalize_extracted_message_row(&row, temp_id, &Self::envelope_dict(&dto));
        }
        Map::new()
    }

    fn envelope_dict(dto: &CreateMessageGatewayDTO) -> Map<String, Value> {
        let mut envelope = Map::new();
        if let Some(timestamp) = dto.created_at.or(dto.timestamp).filter(|timestamp| *timestamp > 0.0) {
            put(&mut envelope, bot_key::CREATED_AT, timestamp);
            put(&mut envelope, bot_key::TIMESTAMP, timestamp);
        }
        envelope
    }

    fn finalize_extracted_message_row(
        row: &Map<String, Value>,
        temp_id: Option<&str>,
        envelope: &Map<String, Value>,
    ) -> Map<String, Value> {
        let mut out = if GetSaiServerId::is_temp_id(temp_id) {
            GetSaiServerId::apply_temp_id_preserving_server_id(row, temp_id)
        } else {
            row.clone()
        };
        if MessageThreadOrder::timestamp_seconds(&out).is_none()
            && let Some(timestamp) = MessageThreadOrder::timestamp_seconds(envelope)
        {
            put(&mut out, bot_key::CREATED_AT, timestamp);
            put(&mut out, bot_key::TIMESTAMP, timestamp);
        }
        put(&mut out, TEMP_FLAG_KEY, false);
        out
    }

    pub fn bump_chat_optimistically(&mut self, any: Option<&Value>, chat_id: &str, cx: &mut Context<Self>) {
        let Some(dict) = MessagePayloadCodec::dictionary(any) else {
            return;
        };

        let now = RiseClock::now().time_interval_since_1970();
        let preview = self.map_optimistic_temp(&dict, chat_id, &[], &[], &[]);
        let topic_id = preview.topic_id.clone().unwrap_or_default();

        let actions = ChatActionsStore::shared(cx);
        let is_listed = actions
            .read(cx)
            .get_chats_sai
            .data()
            .and_then(|data| data.chats)
            .is_some_and(|chats| chats.iter().any(|chat| chat.id == chat_id));
        let chats = ChatServicesStore::shared(cx);
        if is_listed {
            chats.update(cx, |chats, cx| chats.apply_optimistic_last_message(chat_id, preview, cx));
        }
        chats.update(cx, |chats, cx| {
            chats.apply_topic_message_created(chat_id, &topic_id, dict, now, false, cx);
        });
    }

    pub fn create_message_success_handler(
        &mut self,
        any: Option<&Value>,
        chat_id: &str,
        message_scope: &str,
        posted_topic_id: Option<&str>,
        temp_id: &str,
        cx: &mut Context<Self>,
    ) {
        let drop_temp_id = self.outgoing_wire_temp_id(temp_id).unwrap_or_else(|| temp_id.to_owned());
        self.clear_outgoing_wire_temp(temp_id);
        Self::log_create_message_success(any, message_scope, temp_id, cx);
        if let Some(completion) = self.create_message_completions.remove(temp_id) {
            cx.defer(move |cx| completion(Ok(()), cx));
        }

        self.comment_bump_target_by_temp_id.remove(temp_id);

        if let Some(mut real) = self.resolve_created_message(any, chat_id, &drop_temp_id) {
            if real.topic_id.is_none()
                && let Some(posted_topic_id) = posted_topic_id.filter(|topic_id| !topic_id.is_empty())
            {
                real.topic_id = Some(posted_topic_id.to_owned());
            }
            self.mutate_messages_list(
                message_scope,
                SaiUpdaterWriteScope::Cache,
                |messages| {
                    Self::apply_created_message_in_list(messages, &real, chat_id, &drop_temp_id);
                },
                cx,
            );
            let mut chat_preview = real.clone();
            let chat_preview_id = real.sai_server_id();
            if !chat_preview_id.is_empty() {
                chat_preview.id = chat_preview_id;
                chat_preview.server_id = None;
            }
            let topic_preview = Self::topic_preview_row(&real);
            let topic_id = real.topic_id.clone().unwrap_or_default();
            let updated_at = real.created_at;
            let chats = ChatServicesStore::shared(cx);
            chats.update(cx, |chats, cx| chats.apply_optimistic_last_message(chat_id, chat_preview, cx));
            chats.update(cx, |chats, cx| {
                chats.apply_topic_message_created(chat_id, &topic_id, topic_preview, updated_at, false, cx);
            });
            self.link_reedit_doc_if_pending(temp_id, &drop_temp_id, Some(&real), message_scope, cx);
        } else {
            self.reconcile_outgoing_messages_cache(message_scope, cx);
            self.link_reedit_doc_if_pending(temp_id, &drop_temp_id, None, message_scope, cx);
        }
    }

    fn topic_preview_row(message: &GetMessageMessage) -> Map<String, Value> {
        let server_id = message.sai_server_id();
        let mut row = Map::new();
        put(&mut row, bot_key::ID, if server_id.is_empty() { message.id.clone() } else { server_id });
        put(&mut row, bot_key::CHAT_ID, message.chat_id.clone());
        put(&mut row, bot_key::TOPIC_ID, message.topic_id.clone().unwrap_or_default());
        put(&mut row, bot_key::SENDER_ID, message.sender_id.clone());
        put(&mut row, bot_key::SENDER_NAME, message.sender_name.clone());
        put(&mut row, bot_key::CONTENT, message.text.clone());
        put(&mut row, bot_key::CONTENT_TYPE, message.content_type.raw_value());
        put(&mut row, bot_key::CREATED_AT, message.created_at);
        put(&mut row, bot_key::TIMESTAMP, message.created_at);
        put(&mut row, bot_key::IS_READ, message.is_read);
        put(&mut row, bot_key::IS_PINNED, message.is_pinned);
        row
    }

    fn link_reedit_doc_if_pending(
        &mut self,
        logical_temp_id: &str,
        row_temp_id: &str,
        real: Option<&GetMessageMessage>,
        chat_id: &str,
        cx: &mut Context<Self>,
    ) {
        let reedit = MediaReeditStore::shared(cx);
        let Some(edit_id) = reedit.update(cx, |reedit, _| reedit.consume_pending_temp(logical_temp_id)) else {
            return;
        };
        let mut identities = HashSet::<String>::from([row_temp_id.to_owned()]);
        if let Some(real) = real {
            identities.insert(real.id.clone());
            let server_id = real.sai_server_id();
            if !server_id.is_empty() {
                identities.insert(server_id);
            }
        }
        let resolved = self.server_message_id(row_temp_id, chat_id, cx);
        if !resolved.is_empty() {
            identities.insert(resolved);
        }
        reedit.update(cx, |reedit, _| {
            for id in identities.iter().filter(|id| !id.is_empty()) {
                reedit.link(id, &edit_id);
            }
        });
        Sai::log(
            REEDIT_LOG_NAME,
            move || format!("linked doc editId={edit_id} ids={:?}", identities.into_iter().collect::<Vec<_>>()),
            SaiLogKind::Debug,
            cx,
        );
    }

    fn resolve_created_message(
        &self,
        any: Option<&Value>,
        chat_id: &str,
        temp_id: &str,
    ) -> Option<GetMessageMessage> {
        if !self.is_create_message_wire_payload(any) {
            return None;
        }

        if let Some(dto) = CreateMessageGatewayDTO::canon(any)
            && let Some(wire) = dto.message.as_ref().or_else(|| dto.messages.as_ref().and_then(|list| list.first()))
        {
            return Some(Self::finalize_created_message(
                wire.to_message(),
                chat_id,
                temp_id,
                CreatedMessageEnvelope::Gateway(&dto),
            ));
        }

        if let Some(dict) = MessagePayloadCodec::dictionary(any)
            && let Some(nested) = MessagePayloadCodec::dictionary(dict.get(bot_key::MESSAGE))
            && let Some(wire) =
                MessagePayloadCodec::decode::<MessageWireDTO>(Some(&Value::Object(nested)))
        {
            return Some(Self::finalize_created_message(
                wire.to_message(),
                chat_id,
                temp_id,
                CreatedMessageEnvelope::Dictionary(&dict),
            ));
        }

        None
    }

    fn finalize_created_message(
        message: GetMessageMessage,
        chat_id: &str,
        temp_id: &str,
        envelope: CreatedMessageEnvelope<'_>,
    ) -> GetMessageMessage {
        let mut real = message;
        if real.chat_id.is_empty() {
            real.chat_id = chat_id.to_owned();
        }
        if GetSaiServerId::is_temp_id(Some(temp_id)) {
            let wire_id = real.sai_server_id();
            real.id = temp_id.to_owned();
            if !wire_id.is_empty() && !GetSaiServerId::is_temp_id(Some(&wire_id)) {
                real.server_id = Some(wire_id);
            }
        }
        if real.created_at <= 0.0 {
            match envelope {
                CreatedMessageEnvelope::Gateway(dto) => {
                    let raw_timestamp = dto.created_at.or(dto.timestamp).unwrap_or(0.0);
                    if raw_timestamp > 0.0 {
                        real.created_at = MessageThreadOrder::normalize(raw_timestamp);
                    }
                }
                CreatedMessageEnvelope::Dictionary(dict) => {
                    if let Some(timestamp) = MessageThreadOrder::timestamp_seconds(dict) {
                        real.created_at = timestamp;
                    }
                }
            }
        }
        real.is_temp = false;
        real.delivery_status = MessageDeliveryStatus::Sent;
        real
    }

    fn apply_created_message_in_list(
        messages: &mut Vec<GetMessageMessage>,
        real: &GetMessageMessage,
        chat_id: &str,
        temp_id: &str,
    ) {
        let mut state = MessagesChatState { messages: std::mem::take(messages) };
        Self::apply_created_message(&mut state, real, chat_id, temp_id);
        *messages = state.messages;
    }

    fn apply_created_message(
        state: &mut MessagesChatState,
        real: &GetMessageMessage,
        chat_id: &str,
        temp_id: &str,
    ) {
        let mut real_message = real.clone();
        if real_message.chat_id.is_empty() {
            real_message.chat_id = chat_id.to_owned();
        }
        real_message.is_temp = false;
        real_message.delivery_status = MessageDeliveryStatus::Sent;

        if let Some(index) = state.messages.iter().position(|message| message.id == temp_id) {
            Self::merge_real_message_fields(&real_message, &mut state.messages[index], temp_id);
        } else if let Some(server_id) = real_message.server_id.as_deref()
            && let Some(index) = state
                .messages
                .iter()
                .position(|message| message.server_id.as_deref() == Some(server_id) || message.id == server_id)
        {
            let kept_id = state.messages[index].id.clone();
            Self::merge_real_message_fields(&real_message, &mut state.messages[index], &kept_id);
        } else {
            let wire_id = real_message.sai_server_id();
            let is_known = state
                .messages
                .iter()
                .any(|message| message.matches_message_id(&wire_id) || message.id == real_message.id);
            if !is_known {
                if !wire_id.is_empty() {
                    state.messages.retain(|message| {
                        !(GetSaiServerId::is_temp_id(Some(&message.id))
                            && (message.server_id.as_deref() == Some(wire_id.as_str()) || message.id == temp_id))
                    });
                }
                if GetSaiServerId::is_temp_id(Some(temp_id)) {
                    let mut inserted = real_message.clone();
                    inserted.id = temp_id.to_owned();
                    if inserted.server_id.as_deref().is_none_or(str::is_empty) {
                        inserted.server_id = (!GetSaiServerId::is_temp_id(Some(&real_message.id)))
                            .then(|| real_message.id.clone());
                    }
                    state.messages.push(inserted);
                } else {
                    state.messages.push(real_message);
                }
            }
        }
        state.messages = MessageThreadOrder::chronological(std::mem::take(&mut state.messages));
    }

    pub fn merge_real_message_fields(real: &GetMessageMessage, kept: &mut GetMessageMessage, temp_id: &str) {
        kept.id = temp_id.to_owned();
        let wire_id = real.sai_server_id();
        if !wire_id.is_empty() && !GetSaiServerId::is_temp_id(Some(&wire_id)) {
            kept.server_id = Some(wire_id);
        }
        kept.is_temp = false;
        kept.delivery_status = MessageDeliveryStatus::Sent;
        kept.is_read = real.is_read;
        kept.created_at = if real.created_at > 0.0 { real.created_at } else { kept.created_at };
        kept.edited_at = real.edited_at;
        if !real.chat_id.is_empty() {
            kept.chat_id = real.chat_id.clone();
        }
        if let Some(original_text) = &real.original_text {
            kept.original_text = Some(original_text.clone());
        }
        if let Some(entities) = &real.entities {
            kept.entities = Some(entities.clone());
            kept.normalized_entities = real
                .normalized_entities
                .clone()
                .or_else(|| Some(MessageNormalizedEntities::parse(Some(entities.as_str()))));
        }
        if let Some(reply_to) = &real.reply_to {
            kept.reply_to = Some(reply_to.clone());
        }
        if let Some(forward_info) = &real.forward_info {
            kept.forward_info = Some(forward_info.clone());
        }
        if let Some(link_preview) = &real.link_preview {
            kept.link_preview = Some(link_preview.clone());
        }
        if let Some(media_group_id) = &real.media_group_id {
            kept.media_group_id = Some(media_group_id.clone());
        }
        if let Some(topic_id) = &real.topic_id {
            kept.topic_id = Some(topic_id.clone());
        }
        if let Some(reply_markup) = &real.reply_markup {
            kept.reply_markup = Some(reply_markup.clone());
        }
        if let Some(via_bot) = real.via_bot.as_ref().filter(|via_bot| !via_bot.is_empty()) {
            kept.via_bot = Some(via_bot.clone());
        }
        if let Some(via_bot_tag) = real.via_bot_tag.as_ref().filter(|via_bot_tag| !via_bot_tag.is_empty()) {
            kept.via_bot_tag = Some(via_bot_tag.clone());
        }
        if real.is_bot_message {
            kept.is_bot_message = true;
        }
        if let Some(web_app_data) = &real.web_app_data {
            kept.web_app_data = Some(web_app_data.clone());
        }
        if let Some(author_signature) = &real.author_signature {
            kept.author_signature = Some(author_signature.clone());
        }
        if let Some(discussion_chat_id) = &real.discussion_chat_id {
            kept.discussion_chat_id = Some(discussion_chat_id.clone());
        }
        if let Some(discussion_message_id) = &real.discussion_message_id {
            kept.discussion_message_id = Some(discussion_message_id.clone());
        }
        if let Some(comments_count) = real.comments_count {
            kept.comments_count = Some(comments_count);
        }
        if !real.recent_commenters.is_empty() {
            kept.recent_commenters = real.recent_commenters.clone();
        }
        if !real.media_items.is_empty() {
            kept.media_items = Self::merge_media_items_on_send_success(&kept.media_items, &real.media_items);
        }
        if real.content_type == MessageContentType::Voice || kept.content_type == MessageContentType::Voice {
            kept.content_type = MessageContentType::Voice;
            for item in &mut kept.media_items {
                if item.kind == MessageMediaKind::Audio {
                    item.kind = MessageMediaKind::Voice;
                }
            }
            let chat_id = if kept.chat_id.is_empty() { real.chat_id.clone() } else { kept.chat_id.clone() };
            for item in kept.media_items.iter_mut().filter(|item| item.kind == MessageMediaKind::Voice) {
                if item.resolved_local_preview_url().is_none()
                    && let Some(seeded) = VoiceMessageDiskCache::seeded_local_file(
                        trimming_whitespaces_and_newlines(&chat_id),
                        temp_id,
                    )
                {
                    item.local_preview_url = Some(seeded);
                }
            }
        }
        if !real.text.is_empty() || kept.text.is_empty() {
            kept.text = real.text.clone();
        }
        if kept.sender_id.is_empty() {
            kept.sender_id = real.sender_id.clone();
        }
        if kept.sender_name.is_empty() {
            kept.sender_name = real.sender_name.clone();
        }
        if kept.sender_logo.is_none() {
            kept.sender_logo = real.sender_logo.clone();
        }
        kept.reactions = real.reactions.clone();
        kept.is_pinned = real.is_pinned;
        kept.view_count = real.view_count;
    }

    fn log_create_message_success(any: Option<&Value>, chat_id: &str, temp_id: &str, cx: &mut Context<Self>) {
        let dictionary = MessagePayloadCodec::dictionary(any);
        let kind = match (any, &dictionary) {
            (None, _) => "nil",
            (_, Some(_)) => "dict",
            (Some(Value::Array(_)), None) => "array",
            (Some(_), None) => "value",
        };
        let message_count = dictionary.as_ref().map(|dictionary| {
            MessagePayloadCodec::array_of_dictionaries(dictionary.get(message_key::MESSAGES)).len()
        });
        let chat_id = chat_id.to_owned();
        let temp_id = temp_id.to_owned();
        Sai::log(
            MESSAGE_LOG_NAME,
            move || {
                let messages = message_count.map_or_else(|| "?".to_owned(), |count| count.to_string());
                format!("create_message({chat_id}) success payload={kind} messages={messages} temp={temp_id}")
            },
            SaiLogKind::Network,
            cx,
        );
    }

    pub fn create_message_error_handler(
        &mut self,
        error: anyhow::Error,
        chat_id: &str,
        temp_id: &str,
        cx: &mut Context<Self>,
    ) {
        let drop_temp_id = self.outgoing_wire_temp_id(temp_id).unwrap_or_else(|| temp_id.to_owned());
        let is_not_connected = matches!(error.downcast_ref::<SaiWsSendError>(), Some(SaiWsSendError::NotConnected));
        let description = error.to_string();
        if let Some(completion) = self.create_message_completions.remove(temp_id) {
            cx.defer(move |cx| completion(Err(error), cx));
        }
        self.clear_outgoing_wire_temp(temp_id);

        if let Some(bump_target) = self.comment_bump_target_by_temp_id.remove(temp_id) {
            let _ = self.apply_comments_bump_optimistic(&bump_target, -1, cx);
        }

        let log_chat_id = chat_id.to_owned();
        if is_not_connected {
            self.mutate_messages_list(
                chat_id,
                SaiUpdaterWriteScope::Cache,
                |messages| {
                    if let Some(index) =
                        messages.iter().position(|message| message.id == drop_temp_id || message.id == temp_id)
                    {
                        messages[index].delivery_status = MessageDeliveryStatus::Failed;
                        messages[index].is_temp = true;
                    }
                },
                cx,
            );
            Sai::log(
                MESSAGE_LOG_NAME,
                move || format!("create_message({log_chat_id}) offline - message kept as failed"),
                SaiLogKind::Network,
                cx,
            );
            return;
        }

        self.mutate_messages_list(
            chat_id,
            SaiUpdaterWriteScope::Cache,
            |messages| {
                messages.retain(|message| message.id != drop_temp_id && message.id != temp_id);
            },
            cx,
        );
        Sai::log(
            MESSAGE_LOG_NAME,
            move || format!("create_message({log_chat_id}) error: {description}"),
            SaiLogKind::Network,
            cx,
        );
    }

    pub fn build_create_message_body_for_sticker(
        &self,
        sticker: &StickerItem,
        caption: Option<&str>,
        reply_to: Option<&str>,
        chat_id: &str,
        topic_id: Option<&str>,
    ) -> RiseStoredOutgoingMessage {
        let sticker_type = match sticker.asset_type {
            StickerAssetType::StaticImage => MessageStickerKind::Static.raw_value(),
            StickerAssetType::Video => MessageStickerKind::Animated.raw_value(),
            StickerAssetType::Lottie => MessageStickerKind::Lottie.raw_value(),
        };
        let trimmed_caption = caption.map(trimming_whitespaces_and_newlines);
        let media_item = RiseStoredOutgoingMedia {
            file_url: Some(sticker.file_url.clone()),
            thumbnail_url: sticker.thumbnail_url.clone(),
            width: Some(sticker.width),
            height: Some(sticker.height),
            mime_type: sticker.mime_type.clone(),
            duration: sticker.duration_ms.map(|duration_ms| duration_ms / 1000),
            codec: sticker.codec.clone(),
            sticker_id: Some(sticker.id.clone()),
            sticker_type: Some(sticker_type.to_owned()),
            pack_id: Some(sticker.pack_id.clone()),
            associated_emojis: (!sticker.associated_emojis.is_empty()).then(|| sticker.associated_emojis.clone()),
            ..RiseStoredOutgoingMedia::new(MessageMediaKind::Sticker.raw_value().to_owned())
        };

        Self::media_message_body(
            MessageContentType::Sticker.raw_value(),
            media_item,
            trimmed_caption,
            reply_to,
            chat_id,
            topic_id,
        )
    }

    pub fn build_create_message_body_for_gif(
        &self,
        gif: &GifItem,
        caption: Option<&str>,
        reply_to: Option<&str>,
        chat_id: &str,
        topic_id: Option<&str>,
    ) -> RiseStoredOutgoingMessage {
        let trimmed_caption = caption.map(trimming_whitespaces_and_newlines);
        let media_item = RiseStoredOutgoingMedia {
            file_url: Some(gif.file_url.clone()),
            thumbnail_url: (!gif.preview_url.is_empty()).then(|| gif.preview_url.clone()),
            width: Some(gif.width),
            height: Some(gif.height),
            duration: (gif.duration_ms > 0).then_some(gif.duration_ms / 1000),
            ..RiseStoredOutgoingMedia::new(MessageContentType::Animation.raw_value().to_owned())
        };

        Self::media_message_body(
            MessageContentType::Animation.raw_value(),
            media_item,
            trimmed_caption,
            reply_to,
            chat_id,
            topic_id,
        )
    }

    fn media_message_body(
        content_type: &str,
        media_item: RiseStoredOutgoingMedia,
        trimmed_caption: Option<&str>,
        reply_to: Option<&str>,
        chat_id: &str,
        topic_id: Option<&str>,
    ) -> RiseStoredOutgoingMessage {
        RiseStoredOutgoingMessage {
            chat_id: Some(chat_id.to_owned()),
            topic_id: topic_id.map(str::to_owned),
            content: trimmed_caption.unwrap_or("").to_owned(),
            original_content: trimmed_caption.unwrap_or("").to_owned(),
            content_type: content_type.to_owned(),
            reply_to_id: reply_to.map(str::to_owned),
            caption: trimmed_caption.filter(|caption| !caption.is_empty()).map(str::to_owned),
            media_items: Some(vec![media_item]),
            ..RiseStoredOutgoingMessage::default()
        }
    }

    fn merge_media_items_on_send_success(
        existing: &[MessageMediaItem],
        fresh: &[MessageMediaItem],
    ) -> Vec<MessageMediaItem> {
        fresh
            .iter()
            .enumerate()
            .map(|(index, server_item)| {
                let mut merged = server_item.clone();
                let previous = existing.get(index);
                if merged.kind == MessageMediaKind::Audio
                    && previous.is_some_and(|previous| previous.kind == MessageMediaKind::Voice)
                {
                    merged.kind = MessageMediaKind::Voice;
                }
                let is_voice = merged.kind == MessageMediaKind::Voice;
                match previous {
                    Some(previous) if is_voice => {
                        merged.local_preview_url = previous.resolved_local_preview_url();
                    }
                    Some(previous) => {
                        let remote_url = trimming_whitespaces_and_newlines(&merged.url).to_owned();
                        let is_remote = remote_url.to_lowercase().starts_with("http");
                        let is_supported_animation =
                            matches!(merged.kind, MessageMediaKind::Animation | MessageMediaKind::Gif)
                                && ChatAnimatedMediaSourcePolicy::playback_kind(
                                    &remote_url,
                                    merged.mime_type.as_deref(),
                                    merged.codec.as_deref(),
                                ) != ChatAnimatedMediaPlaybackKind::UnsupportedVideo;
                        let replaces_local_preview =
                            is_remote && (merged.kind == MessageMediaKind::Image || is_supported_animation);
                        merged.local_preview_url =
                            if replaces_local_preview { None } else { previous.resolved_local_preview_url() };
                        if merged.width <= 0.0 && previous.width > 0.0 {
                            merged.width = previous.width;
                        }
                        if merged.height <= 0.0 && previous.height > 0.0 {
                            merged.height = previous.height;
                        }
                        if merged.kind == MessageMediaKind::Document {
                            if let Some(existing_name) = previous
                                .file_name
                                .as_deref()
                                .map(trimming_whitespaces_and_newlines)
                                .filter(|existing_name| !existing_name.is_empty())
                            {
                                merged.file_name = Some(existing_name.to_owned());
                            }
                            if merged.file_size.unwrap_or(0) <= 0 {
                                merged.file_size = previous.file_size;
                            }
                        }
                    }
                    None => {
                        merged.local_preview_url = None;
                    }
                }
                if let Some(normalized) = RemoteMediaURL::optional_normalized(Some(merged.url.as_str()))
                    .filter(|normalized| !Self::is_local_or_pending_media_url(normalized))
                {
                    merged.url = normalized;
                }
                merged.is_uploading = false;
                merged.is_compressing = false;
                merged.upload_progress = 100.0;
                merged
            })
            .collect()
    }

    fn is_local_or_pending_media_url(url: &str) -> bool {
        let trimmed = trimming_whitespaces_and_newlines(url).to_lowercase();
        trimmed.starts_with("file://") || is_local_absolute_path(&trimmed) || trimmed.starts_with("content://")
    }

    pub fn flatten_media(
        &self,
        messages: &[GetMessageMessage],
        kinds: &[MessageContentType],
    ) -> Vec<ChatProfileMediaEntry> {
        let mut out = Vec::new();
        for message in messages {
            if !kinds.contains(&message.content_type) {
                continue;
            }
            for (index, item) in message.media_items.iter().enumerate().filter(|(_, item)| item.view_limit.is_none()) {
                out.push(ChatProfileMediaEntry {
                    id: ChatProfileMediaEntry::entry_id(&message.id, index as i64),
                    message_id: message.id.clone(),
                    created_at: message.created_at,
                    item: item.clone(),
                    sender_name: message.sender_name.clone(),
                    sender_id: message.sender_id.clone(),
                    has_protected_content: message.has_protected_content,
                    sender_logo: message.sender_logo.clone(),
                    topic_id: message.topic_id.clone(),
                });
            }
        }
        out
    }

    fn optimistic_media_placeholder(
        index: usize,
        local_file: &RiseOutgoingMediaFile,
        picker_item: Option<&MediaPickerItem>,
        voice_metadata: Option<&VoiceMessageMetadata>,
        fallback_content_type: &str,
    ) -> Map<String, Value> {
        let kind = picker_item.map_or(MediaPickerItemKind::Image, |picker_item| picker_item.kind);
        let content_type = match local_file.media_type.as_deref().filter(|preferred| !preferred.is_empty()) {
            Some(preferred) => preferred.to_owned(),
            None if fallback_content_type == "voice" => "voice".to_owned(),
            None => match kind {
                MediaPickerItemKind::Image => "image".to_owned(),
                MediaPickerItemKind::Video => "video".to_owned(),
                MediaPickerItemKind::File => fallback_content_type.to_owned(),
            },
        };
        let local_url = match (local_file.url.as_os_str().is_empty(), picker_item) {
            (true, Some(picker_item)) => picker_item.url.absolute_string().to_owned(),
            _ => absolute_string(&local_file.url),
        };
        let mut slot = Map::new();
        put(&mut slot, bot_key::ID, format!("temp-media-{index}"));
        put(&mut slot, RiseLocalKey::LOCAL_SLOT_INDEX, index as i64);
        put(&mut slot, bot_key::MEDIA_TYPE, content_type.clone());
        put(&mut slot, bot_key::FILE_URL, local_url.clone());
        put(&mut slot, bot_key::THUMBNAIL_URL, local_url.clone());
        put(&mut slot, bot_key::FILE_NAME, local_file.filename.clone());
        put(&mut slot, bot_key::MIME_TYPE, local_file.content_type.clone());
        put(&mut slot, RiseLocalKey::IS_UPLOADING, true);
        put(&mut slot, RiseLocalKey::UPLOAD_PROGRESS, 0);
        if content_type == MessageContentType::Document.raw_value()
            && !MessageMediaItem::is_decodable_image_source(&local_url)
        {
            put(&mut slot, bot_key::THUMBNAIL_URL, "");
        }
        let byte_count = Self::local_file_byte_count(&local_file.url);
        if byte_count > 0 {
            put(&mut slot, bot_key::FILE_SIZE, byte_count);
            put(&mut slot, file_storage_admin_key::TOTAL_BYTES, byte_count);
        }
        if let Some(picker_item) =
            picker_item.filter(|item| !item.is_file && !matches!(item.kind, MediaPickerItemKind::File))
        {
            if picker_item.has_spoiler || picker_item.view_mode.is_some() {
                put(&mut slot, bot_key::HAS_SPOILER, true);
            }
            if let Some(view_mode) = &picker_item.view_mode {
                put(&mut slot, message_key::VIEW_MODE, view_mode.wire_mode());
                if let Some(seconds) = view_mode.seconds() {
                    put(&mut slot, message_key::VIEW_SECONDS, seconds);
                }
            }
        }
        if let Some(image) = picker_item.and_then(|picker_item| picker_item.image_payload.as_ref()) {
            if image.pixel_size.width > 0.0 {
                put(&mut slot, bot_key::WIDTH, image.pixel_size.width as i64);
            }
            if image.pixel_size.height > 0.0 {
                put(&mut slot, bot_key::HEIGHT, image.pixel_size.height as i64);
            }
            if let Some(preview) = &image.optimistic_preview_url {
                put(&mut slot, bot_key::THUMBNAIL_URL, preview.absolute_string());
                put(&mut slot, RiseLocalKey::LOCAL_PREVIEW_URL, preview.absolute_string());
            }
        }
        if let Some(video) = picker_item.and_then(|picker_item| picker_item.video_payload.as_ref()) {
            if video.pixel_size.width > 0.0 {
                put(&mut slot, bot_key::WIDTH, video.pixel_size.width as i64);
            }
            if video.pixel_size.height > 0.0 {
                put(&mut slot, bot_key::HEIGHT, video.pixel_size.height as i64);
            }
            if video.duration > 0.0 {
                put(&mut slot, bot_key::DURATION, video.duration.round() as i64);
            }
            if let Some(poster) = video.optimistic_poster_url.as_ref().or(video.poster_url.as_ref()) {
                put(&mut slot, bot_key::THUMBNAIL_URL, poster.absolute_string());
                if poster.is_file_url() {
                    put(&mut slot, RiseLocalKey::LOCAL_PREVIEW_URL, poster.absolute_string());
                }
            }
        }
        if let Some(picker_item) = picker_item
            && let Some(audio) = &picker_item.audio_payload
        {
            put(&mut slot, bot_key::FILE_NAME, picker_item.audio_file_name().unwrap_or_default());
            put(&mut slot, bot_key::DURATION, audio.duration.round() as i64);
            put(&mut slot, RiseLocalKey::LOCAL_PREVIEW_URL, local_url);
            if let Some(cover_url) = &audio.cover_url {
                put(&mut slot, bot_key::THUMBNAIL_URL, cover_url.absolute_string());
            } else {
                put(&mut slot, bot_key::THUMBNAIL_URL, "");
            }
        }
        slot.extend(Self::optimistic_voice_slot(voice_metadata));
        slot
    }

    fn optimistic_voice_slot(voice_metadata: Option<&VoiceMessageMetadata>) -> Map<String, Value> {
        let Some(voice_metadata) = voice_metadata else {
            return Map::new();
        };

        let mut slot = Map::new();
        put(&mut slot, bot_key::DURATION, voice_metadata.duration_seconds);
        if let Some(wire) = RiseStoredOutgoingMedia::waveform_wire_string(&voice_metadata.waveform) {
            put(&mut slot, bot_key::WAVEFORM, wire);
        }
        slot
    }

    pub fn map_optimistic_temp(
        &self,
        temp: &Map<String, Value>,
        chat_id: &str,
        _picker_items: &[MediaPickerItem],
        local_files: &[RiseOutgoingMediaFile],
        _voice_metadata_slots: &[VoiceMessageMetadata],
    ) -> GetMessageMessage {
        let message_id =
            string_field(temp, bot_key::ID).map(str::to_owned).unwrap_or_else(|| Uuid::new_v4().uuid_string());
        let mut message = GetMessageMessage::new(message_id.clone());
        message.chat_id = chat_id.to_owned();
        message.sender_id = string_field(temp, bot_key::SENDER_ID).unwrap_or("").to_owned();
        message.sender_name = string_field(temp, bot_key::SENDER_NAME).unwrap_or("").to_owned();
        message.text = string_field(temp, bot_key::CONTENT).unwrap_or("").to_owned();
        if message.text.is_empty()
            && let Some(caption) = string_field(temp, bot_key::CAPTION)
        {
            message.text = caption.to_owned();
        }
        message.original_text = string_field(temp, bot_key::ORIGINAL_CONTENT).map(str::to_owned);
        if let Some(entities) = string_field(temp, bot_key::ENTITIES).filter(|entities| !entities.is_empty()) {
            message.entities = Some(entities.to_owned());
        }
        message.normalized_entities = Some(MessageNormalizedEntities::parse(message.entities.as_deref()));
        if let Some(topic_id) = string_field(temp, bot_key::TOPIC_ID).filter(|topic_id| !topic_id.is_empty()) {
            message.topic_id = Some(topic_id.to_owned());
        }
        if let Some(via_bot) = string_field(temp, bot_key::VIA_BOT).filter(|via_bot| !via_bot.is_empty()) {
            message.via_bot = Some(via_bot.to_owned());
        }
        message.is_bot_message = bool_field(temp, bot_key::IS_BOT_MESSAGE).unwrap_or(false);
        if let Some(markup) = MessagePayloadCodec::decode::<MessageBotReplyMarkupWireDTO>(temp.get(bot_key::REPLY_MARKUP))
        {
            message.reply_markup = Some(markup.to_model());
        }
        message.content_type =
            MessageContentType::from_raw_value(string_field(temp, bot_key::CONTENT_TYPE).unwrap_or("text"));
        message.created_at =
            number_f64(temp, bot_key::CREATED_AT).unwrap_or_else(|| RiseClock::now().time_interval_since_1970());
        message.media_group_id = string_field(temp, bot_key::MEDIA_GROUP_ID).map(str::to_owned);
        message.is_temp = bool_field(temp, TEMP_FLAG_KEY).unwrap_or(true);
        message.delivery_status = MessageDeliveryStatus::Sending;
        if let Some(server_id) = string_field(temp, RiseLocalKey::SERVER_ID).filter(|server_id| !server_id.is_empty()) {
            message.server_id = Some(server_id.to_owned());
        }
        if let Some(shared) = temp
            .get(payload_key::SHARED_POST)
            .and_then(Value::as_object)
            .and_then(Self::shared_post_from_optimistic)
        {
            message.shared_post = Some(shared);
            message.content_type = MessageContentType::PostShort;
        }
        if let Some(story_dict) = temp.get(events_key::STORY_REPLY).and_then(Value::as_object) {
            message.story_reply = Self::story_reply_from_optimistic(story_dict);
        }
        if let Some(value) = MessagePayloadCodec::decode::<MessageBotWebAppDataWireDTO>(temp.get(bot_key::WEB_APP_DATA))
        {
            message.web_app_data = Some(value.to_model());
        }

        let slots = MessagePayloadCodec::array_of_dictionaries(temp.get(bot_key::MEDIA_ITEMS));
        if !slots.is_empty() {
            message.media_items = slots
                .iter()
                .enumerate()
                .map(|(index, slot)| {
                    Self::media_item_from_optimistic_slot(slot, index, &message_id, local_files.get(index))
                })
                .collect();
        }
        message
    }

    fn shared_post_from_optimistic(dict: &Map<String, Value>) -> Option<MessageSharedPost> {
        let post_id = int_val(dict.get(events_key::POST_ID));

        if post_id <= 0 {
            return None;
        }

        let empty = Map::new();
        let author_dict = dict.get(events_key::AUTHOR).and_then(Value::as_object).unwrap_or(&empty);
        let author = MessageSharedPostAuthor {
            id: string_field(author_dict, bot_key::ID).unwrap_or("").to_owned(),
            tag: string_field(author_dict, bot_key::TAG).unwrap_or("").to_owned(),
            name: string_field(author_dict, bot_key::NAME).unwrap_or("").to_owned(),
            logo: RemoteMediaURL::optional_normalized(string_field(author_dict, bot_key::LOGO)).unwrap_or_default(),
            is_premium: bool_field(author_dict, bot_key::IS_PREMIUM).unwrap_or(false),
            is_official: false,
        };
        let is_short = bool_field(dict, comment_key::IS_SHORT);
        Some(MessageSharedPost {
            post_id: post_id.to_string(),
            is_short: is_short.unwrap_or(false),
            is_riff: bool_field(dict, comment_key::IS_RIFF).or(is_short).unwrap_or(false),
            show_in_publications: bool_field(dict, comment_key::SHOW_IN_PUBLICATIONS).unwrap_or(true),
            title: string_field(dict, bot_key::TITLE).unwrap_or("").to_owned(),
            snippet: string_field(dict, message_key::SNIPPET).unwrap_or("").to_owned(),
            images: string_array(dict.get(events_key::IMAGES)).unwrap_or_default(),
            video: RemoteMediaURL::optional_normalized(string_field(dict, events_key::VIDEO)).unwrap_or_default(),
            video_thumbnail: RemoteMediaURL::optional_normalized(string_field(dict, comment_key::VIDEO_THUMBNAIL))
                .unwrap_or_default(),
            likes_count: int_val(dict.get(comment_key::LIKES_COUNT)),
            comments_count: int_val(dict.get(bot_key::COMMENTS_COUNT)),
            author,
        })
    }

    fn story_reply_from_optimistic(dict: &Map<String, Value>) -> Option<MessageStoryReply> {
        let story_id = int_val(dict.get(events_key::STORY_ID));
        if story_id <= 0 {
            return None;
        }

        let empty = Map::new();
        let author_dict = dict.get(events_key::AUTHOR).and_then(Value::as_object).unwrap_or(&empty);
        Some(MessageStoryReply {
            story_id,
            media_url: RemoteMediaURL::optional_normalized(string_field(dict, bot_key::MEDIA_URL)).unwrap_or_default(),
            is_video: bool_field(dict, payload_key::IS_VIDEO).unwrap_or(false),
            caption: string_field(dict, bot_key::CAPTION).unwrap_or("").to_owned(),
            created_at_ms: int_val(dict.get(events_key::CREATED_AT_MS)),
            expires_at_ms: int_val(dict.get(events_key::EXPIRES_AT_MS)),
            post_to_profile: bool_field(dict, stories_key::POST_TO_PROFILE),
            author: MessageStoryReplyAuthor {
                id: string_field(author_dict, bot_key::ID).unwrap_or("").to_owned(),
                tag: string_field(author_dict, bot_key::TAG).unwrap_or("").to_owned(),
                name: string_field(author_dict, bot_key::NAME).unwrap_or("").to_owned(),
                logo: RemoteMediaURL::optional_normalized(string_field(author_dict, bot_key::LOGO)).unwrap_or_default(),
                is_premium: bool_field(author_dict, bot_key::IS_PREMIUM).unwrap_or(false),
            },
            is_known_gone: false,
        })
    }

    fn media_item_from_optimistic_slot(
        slot: &Map<String, Value>,
        index: usize,
        message_id: &str,
        local_file: Option<&RiseOutgoingMediaFile>,
    ) -> MessageMediaItem {
        let media_type = string_field(slot, bot_key::MEDIA_TYPE).map(str::to_owned).unwrap_or_else(|| {
            Self::guess_media_type(local_file.map_or("", |local_file| local_file.content_type.as_str())).to_owned()
        });
        let kind = MessageMediaKind::from_raw(&media_type.to_lowercase()).unwrap_or(MessageMediaKind::Image);
        let normalized_file_url = RemoteMediaURL::optional_normalized(string_field(slot, bot_key::FILE_URL));
        let local_url = local_file.map(|local_file| absolute_string(&local_file.url));
        let file_url = normalized_file_url.clone().or_else(|| local_url.clone()).unwrap_or_default();
        let normalized_thumbnail = RemoteMediaURL::optional_normalized(string_field(slot, bot_key::THUMBNAIL_URL));
        let thumb = normalized_thumbnail.clone().unwrap_or_else(|| file_url.clone());
        let item_id =
            string_field(slot, bot_key::ID).map(str::to_owned).unwrap_or_else(|| format!("{message_id}-{index}"));
        let mut item = MessageMediaItem::new(item_id, kind, file_url);
        item.thumbnail_url = Some(thumb);
        item.width = number_f64(slot, bot_key::WIDTH).unwrap_or(0.0) as f32;
        item.height = number_f64(slot, bot_key::HEIGHT).unwrap_or(0.0) as f32;
        item.duration = number_f64(slot, bot_key::DURATION);
        item.file_name = string_field(slot, bot_key::FILE_NAME)
            .map(str::to_owned)
            .or_else(|| local_file.map(|local_file| local_file.filename.clone()));
        item.file_size = slot.get(bot_key::FILE_SIZE).and_then(number_i64);
        item.mime_type = string_field(slot, bot_key::MIME_TYPE)
            .map(str::to_owned)
            .or_else(|| local_file.map(|local_file| local_file.content_type.clone()));
        let has_remote_url = normalized_file_url.is_some_and(|url| url.to_lowercase().starts_with("http"));
        item.is_uploading = bool_field(slot, RiseLocalKey::IS_UPLOADING).unwrap_or(!has_remote_url);
        item.is_compressing = bool_field(slot, RiseLocalKey::IS_COMPRESSING).unwrap_or(false);
        item.upload_progress = number_f64(slot, RiseLocalKey::UPLOAD_PROGRESS).unwrap_or(0.0);
        item.compression_progress = number_f64(slot, RiseLocalKey::COMPRESSION_PROGRESS).unwrap_or(0.0);
        item.uploaded_bytes = Self::positive_byte_count(slot.get(RiseLocalKey::UPLOADED_BYTES)).unwrap_or(0);
        item.total_bytes = Self::positive_byte_count(slot.get(file_storage_admin_key::TOTAL_BYTES))
            .unwrap_or_else(|| item.file_size.unwrap_or(0).max(0));
        item.local_slot_index = slot
            .get(RiseLocalKey::LOCAL_SLOT_INDEX)
            .and_then(number_i64)
            .unwrap_or(index as i64)
            .max(0);
        item.is_cancelled = bool_field(slot, RiseLocalKey::IS_CANCELLED).unwrap_or(false);
        let is_visual = matches!(kind, MessageMediaKind::Image | MessageMediaKind::Video);
        item.has_spoiler = bool_field(slot, bot_key::HAS_SPOILER) == Some(true) && is_visual;
        if let Some(mode) = string_field(slot, message_key::VIEW_MODE).filter(|mode| !mode.is_empty()) {
            if is_visual {
                item.view_limit = Some(MessageMediaViewLimit {
                    mode: mode.to_owned(),
                    seconds: slot.get(message_key::VIEW_SECONDS).and_then(number_i64),
                    expires_at_ms: None,
                    state: None,
                });
            }
        }
        if let Some(raw) = string_field(slot, bot_key::WAVEFORM) {
            item.waveform = MessageWaveformCodec::parse_floats(Some(raw));
        }
        if kind == MessageMediaKind::Sticker {
            if let Some(sticker_type) = string_field(slot, bot_key::STICKER_TYPE) {
                item.sticker_kind = MessageStickerKind::from_raw(sticker_type);
            }
            if let Some(pack_id) = string_field(slot, bot_key::PACK_ID) {
                item.pack_id = Some(pack_id.to_owned());
            }
            if let Some(emojis) = string_array(slot.get(bot_key::ASSOCIATED_EMOJIS)) {
                item.sticker_emojis = emojis;
            }
            item.thumbnail_url = None;
        }
        if kind == MessageMediaKind::Document {
            item.thumbnail_url =
                normalized_thumbnail.filter(|thumbnail| MessageMediaItem::is_decodable_image_source(thumbnail));
        }
        if let Some(preview) = string_field(slot, RiseLocalKey::LOCAL_PREVIEW_URL).filter(|preview| {
            MessageMediaItem::is_local_preview_url(preview) && !MessageMediaItem::is_local_video_file(preview)
        }) {
            item.local_preview_url = Some(preview.to_owned());
        } else if let Some(local) = local_url.filter(|local| {
            MessageMediaItem::is_local_preview_url(local) && !MessageMediaItem::is_local_video_file(local)
        }) {
            item.local_preview_url = Some(local);
        }
        item
    }

    fn guess_media_type(mime: &str) -> &'static str {
        let mime = mime.to_lowercase();
        if mime.starts_with("video/") {
            return "video";
        }
        if mime.starts_with("image/") {
            return "image";
        }
        if mime.starts_with("audio/") {
            return "audio";
        }
        "document"
    }

    fn local_file_byte_count(path: &Path) -> i64 {
        fs::metadata(path).map_or(0, |metadata| i64::try_from(metadata.len()).unwrap_or(0))
    }

    fn positive_byte_count(value: Option<&Value>) -> Option<i64> {
        let bytes = match value? {
            Value::Number(_) => value.and_then(number_i64),
            Value::String(text) => text.parse::<i64>().ok(),
            _ => None,
        }?;
        (bytes > 0).then_some(bytes)
    }
}
