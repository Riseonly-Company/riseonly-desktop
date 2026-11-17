use std::cell::Cell;
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gpui::{App, Context, size};
use rise_bot_api::ChatAction;
use rise_canon::{ChatMessageContentType, RiseLimits, RiseUploadContextFormats, RiseUploadFormat, rise_key, rise_upload};
use rise_core::foundation::foundation_date::{RiseClock, TimeInterval};
use rise_core::foundation::foundation_json_serialization::{JSONSerialization, JSONWritingOptions};
use rise_core::foundation::foundation_text::{characters, trimming_whitespaces_and_newlines};
use rise_core::foundation::foundation_url::Url;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::{FormatArg, tr, tr_args};
use rise_platform::paths::temporary_directory;
use rise_platform::uniform_type::UTType;
use serde_json::{Map, Value};

use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::canon_format::RiseFormat;
use crate::core::lib::canon_rules::RiseLimitsCanonRules;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::modules::sai_ws::sai_ws_chunked_file_upload::SaiWsChunkedFileUploadError;
use crate::core::lib::sai::modules::sai_ws::sai_ws_request_options::SaiWsSendError;
use crate::core::lib::sai::sai::Sai;
use crate::core::media::rise_media_kind_classifier::{RiseMediaKind, RiseMediaKindClassifier};
use crate::core::ui::lottie_sticker::sticker_premium_effect::StickerPremiumEffectRegistry;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::composer::chat_link_preview_draft::{
    ChatLinkPreviewSelection, ChatLinkPreviewURL,
};
use crate::modules::chat::engine::messages::rise_message_contracts::RiseMessageRepositoryError;
use crate::modules::chat::engine::messages::rise_message_repository::RiseMessageRepository;
use crate::modules::chat::shared::utils::chat_composer_text_splitter::{
    ChatComposerTextSplitter, ChatComposerTextSplitterBudget, ChatComposerTextSplitterPart,
};
use crate::modules::chat::shared::utils::voice::voice_message_metadata::VoiceMessageMetadata;
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatPermKey;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_actions::MessageActionCompletion;
use crate::modules::chat::stores::message::message_actions::message_poll_codec::{MessagePollCodec, MessagePollDraft};
use crate::modules::chat::stores::message::message_actions::message_types::{
    MessageContentType, MessageLocationCodec, MessageLocationInfo,
};
use crate::modules::chat::stores::message::message_actions::message_wire_types::RiseStoredOutgoingMessage;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_interactions::message_send_stickers_hooks::message_send_stickers_hooks;
use crate::modules::chat::stores::message::message_interactions::share_destination_hooks::ShareSheetDestination;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::media_picker::api::media_picker_result::{
    MediaPickerEditedVideo, MediaPickerItem, MediaPickerItemKind,
};
use crate::modules::media_picker::editor::media_reedit_store::MediaReeditStore;
use crate::modules::media_picker::export::rise_outgoing_media_file::RiseOutgoingMediaFile;
use crate::modules::media_picker::export::video_note_composition::{VideoNoteCaptureRecipe, VideoNoteComposition};
use crate::modules::music::shared::utils::music::music_track_file_upload::MusicTrackFileUpload;
use crate::modules::post::stores::post_actions::post_types::GetPostFeedResponse;
use crate::modules::share_extension::send_handoff::share_send_handoff_models::RiseStoredSharePayloadResult;
use crate::modules::share_extension::shared::rise_upload_context_rules::RiseLimitsUploadContextRulesExt;
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{GifItem, PackKind, StickerItem};
use crate::modules::story::stores::story::story_actions::story_wire_types::{StoryAuthorDTO, StoryDTO};

const WEB_APP_BUTTON_TEXT_MAX_CHARACTERS: usize = 64;
const STORY_CAPTION_PREVIEW_CHARACTERS: usize = 140;
const POST_SNIPPET_PREVIEW_CHARACTERS: usize = 280;
const IDEMPOTENCY_KEY_MAX_BYTES: usize = 128;
const IMMEDIATE_SHARE_RESPONSE_TIMEOUT_SECONDS: TimeInterval = 30.0;

type SharedCompletion = Rc<Cell<Option<MessageActionCompletion>>>;

fn owned(value: Option<&str>) -> Option<String> {
    value.map(str::to_owned)
}

fn prefix_characters(text: &str, count: usize) -> String {
    characters(text).take(count).collect()
}

fn timestamped_temp_id() -> String {
    format!("temp_{}_{}", RiseClock::now().ms(), rand::random_range(1000_i64..9999))
}

fn lowercased_uuid_temp_id() -> String {
    format!("temp_{}", Uuid::new_v4().uuid_string().to_lowercase())
}

fn file_path(url: &Url) -> PathBuf {
    url.to_file_path().unwrap_or_default()
}

fn fail_send(completion: Option<MessageActionCompletion>, error: anyhow::Error, cx: &mut App) {
    if let Some(completion) = completion {
        cx.defer(move |cx| completion(Err(error), cx));
    }
}

fn encoded_json(object: Map<String, Value>, sorted_keys: bool) -> Option<Vec<u8>> {
    JSONSerialization::data_with_json_object(
        &Value::Object(object),
        JSONWritingOptions { sorted_keys, ..JSONWritingOptions::default() },
    )
}

fn web_app_data_rule() -> RiseFieldRule {
    RiseFieldRule::client_rule("bot_mini_app_send_data")
}

fn web_app_init_data_rule() -> RiseFieldRule {
    RiseFieldRule::client_rule("bot_mini_app_init_data")
}

fn web_app_envelope_max_bytes() -> i64 {
    RiseLimits::service_value("max_web_app_envelope_bytes")
}

fn bottom_options(notifier_type: NotifierType) -> NotifierSendOptions {
    NotifierSendOptions {
        r#type: notifier_type,
        position: NotifierPosition::Bottom,
        ..NotifierSendOptions::default()
    }
}

impl MessageInteractionsStore {
    pub fn apply_shared_payload_success(
        &mut self,
        result: &RiseStoredSharePayloadResult,
        destination: &ShareSheetDestination,
        cx: &mut Context<Self>,
    ) {
        Self::service(cx).update(cx, |service, cx| service.apply_shared_payload_success(result, destination, cx));
    }

    pub fn media_send_failure_text(&self, error: &anyhow::Error) -> Option<String> {
        if let Some(upload) = error.downcast_ref::<SaiWsChunkedFileUploadError>() {
            if let SaiWsChunkedFileUploadError::FileTooLarge { maximum_bytes, .. } = upload {
                let maximum = RiseFormat::file_size(*maximum_bytes);
                return Some(tr_args("upload_error_file_too_large_max", &[FormatArg::Text(maximum.into())]));
            }
            return upload.error_description();
        }
        if let Some(code) = RiseMessageRepository::upload_error_code(error) {
            return Some(tr(code.i18n_key()));
        }
        if let Some(RiseMessageRepositoryError::RemoteFailure(code)) = error.downcast_ref::<RiseMessageRepositoryError>() {
            return CanonErrors::text(Some(code.as_str()), &HashMap::new());
        }
        Self::message_refusal_text(error)
    }

    fn send_replay(action: impl Fn(&mut Self, String, &mut Context<Self>) + 'static) -> Rc<dyn Fn(String, &mut App)> {
        Rc::new(move |real_id: String, cx: &mut App| {
            Self::shared(cx).update(cx, |store, cx| action(store, real_id, cx));
        })
    }

    fn rejects_guide_chat_mutation(chat_id: &str, cx: &mut App) -> bool {
        Self::actions(cx).update(cx, |actions, cx| actions.reject_guide_chat_mutation(chat_id, cx))
    }

    fn resolved_reply(
        reply_to_id: Option<&str>,
        thread_root_id: Option<&str>,
        scope: &str,
        cx: &mut App,
    ) -> Option<String> {
        let message_id = reply_to_id.or(thread_root_id)?;
        Some(Self::service(cx).update(cx, |service, cx| service.server_message_id(message_id, scope, cx)))
    }

    fn bump_chat_list_with_last_message_in(chat_id: &str, cx: &mut App) {
        Self::service(cx).update(cx, |service, cx| service.bump_chat_list(chat_id, cx));
    }

    #[allow(clippy::too_many_arguments)]
    fn create_message(
        body: RiseStoredOutgoingMessage,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        local_files: Vec<RiseOutgoingMediaFile>,
        picker_items: Vec<MediaPickerItem>,
        voice_metadata_slots: Vec<VoiceMessageMetadata>,
        temp_id: &str,
        defer_until_connected: bool,
        response_timeout_seconds: Option<TimeInterval>,
        completion: Option<MessageActionCompletion>,
        cx: &mut App,
    ) {
        Self::actions(cx).update(cx, |actions, cx| {
            actions.create_message_action(
                body,
                chat_id,
                topic_id,
                thread_root_id,
                local_files,
                picker_items,
                voice_metadata_slots,
                temp_id,
                defer_until_connected,
                response_timeout_seconds,
                None,
                completion,
                cx,
            );
        });
    }

    pub fn send_web_app_data(
        &mut self,
        chat_id: &str,
        data: &str,
        init_data: &str,
        button_text: Option<&str>,
        completion: Option<MessageActionCompletion>,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        if Self::rejects_guide_chat_mutation(chat_id, cx) {
            fail_send(completion, SaiWsSendError::GuideSandbox.into(), cx);
            return None;
        }
        let completion: SharedCompletion = Rc::new(Cell::new(completion));
        let replay = {
            let data = data.to_owned();
            let init_data = init_data.to_owned();
            let button_text = owned(button_text);
            let completion = Rc::clone(&completion);
            Self::send_replay(move |store, real_id, cx| {
                store.send_web_app_data(&real_id, &data, &init_data, button_text.as_deref(), completion.take(), cx);
            })
        };
        if self.defer_send_until_direct_chat_exists(chat_id, replay, cx) {
            return None;
        }

        let chat_id = self.resolved_send_chat_id(chat_id, cx)?;
        if data.is_empty()
            || !web_app_data_rule().is_satisfied(data)
            || init_data.is_empty()
            || !web_app_init_data_rule().is_satisfied(init_data)
        {
            return None;
        }

        let normalized_button_text = button_text
            .map(|text| prefix_characters(trimming_whitespaces_and_newlines(text), WEB_APP_BUTTON_TEXT_MAX_CHARACTERS));
        let mut web_app_object = Map::new();
        web_app_object.insert(rise_key::events::DATA.to_owned(), Value::from(data));
        web_app_object.insert(rise_key::bot::INIT_DATA.to_owned(), Value::from(init_data));
        if let Some(text) = normalized_button_text.as_deref()
            && !text.is_empty()
        {
            web_app_object.insert(rise_bot_api::reply_markup::BUTTON_TEXT.to_owned(), Value::from(text));
        }
        let encoded = encoded_json(web_app_object, true)?;
        if encoded.len() as i64 > web_app_envelope_max_bytes() {
            return None;
        }
        let web_app_data = String::from_utf8(encoded).ok()?;

        let content = normalized_button_text.clone().unwrap_or_default();
        let temp_id = Self::service(cx).read(cx).web_app_submission_temp_id(
            init_data,
            data,
            normalized_button_text.as_deref(),
        );
        let body = RiseStoredOutgoingMessage {
            chat_id: Some(chat_id.clone()),
            original_content: content.clone(),
            content,
            content_type: ChatMessageContentType::Text.raw_value().to_owned(),
            web_app_data: Some(web_app_data),
            temp_id: Some(temp_id.clone()),
            ..RiseStoredOutgoingMessage::default()
        };

        Self::create_message(
            body,
            &chat_id,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            &temp_id,
            true,
            None,
            completion.take(),
            cx,
        );
        Self::bump_chat_list_with_last_message_in(&chat_id, cx);
        Some(temp_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn send_text(
        &mut self,
        chat_id: &str,
        text: &str,
        topic_id: Option<&str>,
        post_topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        reply_to_id: Option<&str>,
        forward_from_message_id: Option<&str>,
        forward_from_chat_id: Option<&str>,
        entities: Option<&str>,
        document: Option<&str>,
        link_preview: Option<ChatLinkPreviewSelection>,
        completion: Option<MessageActionCompletion>,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        if Self::rejects_guide_chat_mutation(chat_id, cx) {
            fail_send(completion, SaiWsSendError::GuideSandbox.into(), cx);
            return None;
        }
        let completion: SharedCompletion = Rc::new(Cell::new(completion));
        let replay = {
            let text = text.to_owned();
            let topic_id = owned(topic_id);
            let post_topic_id = owned(post_topic_id);
            let thread_root_id = owned(thread_root_id);
            let reply_to_id = owned(reply_to_id);
            let forward_from_message_id = owned(forward_from_message_id);
            let forward_from_chat_id = owned(forward_from_chat_id);
            let entities = owned(entities);
            let document = owned(document);
            let link_preview = link_preview.clone();
            let completion = Rc::clone(&completion);
            Self::send_replay(move |store, real_id, cx| {
                store.send_text(
                    &real_id,
                    &text,
                    topic_id.as_deref(),
                    post_topic_id.as_deref(),
                    thread_root_id.as_deref(),
                    reply_to_id.as_deref(),
                    forward_from_message_id.as_deref(),
                    forward_from_chat_id.as_deref(),
                    entities.as_deref(),
                    document.as_deref(),
                    link_preview.clone(),
                    completion.take(),
                    cx,
                );
            })
        };
        if self.defer_send_until_direct_chat_exists(chat_id, replay, cx) {
            return None;
        }

        let chat_id = self.resolved_send_chat_id(chat_id, cx)?;

        let trimmed = trimming_whitespaces_and_newlines(text);
        if trimmed.is_empty() {
            return None;
        }

        let scope = MessageServicesStore::message_scope(&chat_id, topic_id, thread_root_id);
        let temp_id = timestamped_temp_id();
        let resolved_reply = Self::resolved_reply(reply_to_id, thread_root_id, &scope, cx);

        let (link_preview_type, link_preview_metadata) = match link_preview {
            Some(selection) => (Some(selection.wire_type), selection.metadata),
            None => (None, None),
        };
        let body = RiseStoredOutgoingMessage {
            chat_id: Some(chat_id.clone()),
            topic_id: owned(post_topic_id.or(topic_id)),
            content: trimmed.to_owned(),
            original_content: trimmed.to_owned(),
            content_type: ChatMessageContentType::Text.raw_value().to_owned(),
            reply_to_id: resolved_reply,
            forward_from_message_id: owned(forward_from_message_id),
            forward_from_chat_id: owned(forward_from_chat_id),
            entities: owned(entities),
            document: owned(document),
            link_preview_type,
            link_preview: link_preview_metadata,
            ..RiseStoredOutgoingMessage::default()
        };

        Self::create_message(
            body,
            &chat_id,
            topic_id,
            thread_root_id,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            &temp_id,
            true,
            None,
            completion.take(),
            cx,
        );

        Self::bump_chat_list_with_last_message_in(&chat_id, cx);
        Some(temp_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn send_location(
        &mut self,
        chat_id: &str,
        location: &MessageLocationInfo,
        topic_id: Option<&str>,
        post_topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        reply_to_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        if Self::rejects_guide_chat_mutation(chat_id, cx) {
            return None;
        }
        let replay = {
            let location = location.clone();
            let topic_id = owned(topic_id);
            let post_topic_id = owned(post_topic_id);
            let thread_root_id = owned(thread_root_id);
            let reply_to_id = owned(reply_to_id);
            Self::send_replay(move |store, real_id, cx| {
                store.send_location(
                    &real_id,
                    &location,
                    topic_id.as_deref(),
                    post_topic_id.as_deref(),
                    thread_root_id.as_deref(),
                    reply_to_id.as_deref(),
                    cx,
                );
            })
        };
        if self.defer_send_until_direct_chat_exists(chat_id, replay, cx) {
            return None;
        }

        let chat_id = self.resolved_send_chat_id(chat_id, cx)?;
        let content = MessageLocationCodec::encode(location)?;

        let scope = MessageServicesStore::message_scope(&chat_id, topic_id, thread_root_id);
        let temp_id = timestamped_temp_id();
        let resolved_reply = Self::resolved_reply(reply_to_id, thread_root_id, &scope, cx);

        let body = RiseStoredOutgoingMessage {
            chat_id: Some(chat_id.clone()),
            topic_id: owned(post_topic_id.or(topic_id)),
            original_content: content.clone(),
            content,
            content_type: ChatMessageContentType::Location.raw_value().to_owned(),
            reply_to_id: resolved_reply,
            ..RiseStoredOutgoingMessage::default()
        };

        Self::create_message(
            body,
            &chat_id,
            topic_id,
            thread_root_id,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            &temp_id,
            true,
            None,
            None,
            cx,
        );

        Self::bump_chat_list_with_last_message_in(&chat_id, cx);
        Some(temp_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn send_poll(
        &mut self,
        chat_id: &str,
        poll: &MessagePollDraft,
        topic_id: Option<&str>,
        post_topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        reply_to_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        if Self::rejects_guide_chat_mutation(chat_id, cx) {
            return None;
        }
        let replay = {
            let poll = poll.clone();
            let topic_id = owned(topic_id);
            let post_topic_id = owned(post_topic_id);
            let thread_root_id = owned(thread_root_id);
            let reply_to_id = owned(reply_to_id);
            Self::send_replay(move |store, real_id, cx| {
                store.send_poll(
                    &real_id,
                    &poll,
                    topic_id.as_deref(),
                    post_topic_id.as_deref(),
                    thread_root_id.as_deref(),
                    reply_to_id.as_deref(),
                    cx,
                );
            })
        };
        if self.defer_send_until_direct_chat_exists(chat_id, replay, cx) {
            return None;
        }

        let chat_id = self.resolved_send_chat_id(chat_id, cx)?;
        let content = MessagePollCodec::encode(poll)?;

        let scope = MessageServicesStore::message_scope(&chat_id, topic_id, thread_root_id);
        let temp_id = timestamped_temp_id();
        let resolved_reply = Self::resolved_reply(reply_to_id, thread_root_id, &scope, cx);

        let body = RiseStoredOutgoingMessage {
            chat_id: Some(chat_id.clone()),
            topic_id: owned(post_topic_id.or(topic_id)),
            original_content: content.clone(),
            content,
            content_type: ChatMessageContentType::Poll.raw_value().to_owned(),
            reply_to_id: resolved_reply,
            ..RiseStoredOutgoingMessage::default()
        };

        Self::create_message(
            body,
            &chat_id,
            topic_id,
            thread_root_id,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            &temp_id,
            true,
            None,
            None,
            cx,
        );

        Self::bump_chat_list_with_last_message_in(&chat_id, cx);
        Some(temp_id)
    }

    pub fn text_parts(
        &self,
        chat_id: &str,
        text: &str,
        entities: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<Vec<ChatComposerTextSplitterPart>> {
        let Some(parts) = ChatComposerTextSplitter::split(text, entities, &ChatComposerTextSplitterBudget::message())
        else {
            Notifier::send(tr("chat_text_split_too_long"), bottom_options(NotifierType::Error), cx);
            return None;
        };
        if parts.len() > 1
            && ChatServicesStore::shared(cx).update(cx, |store, cx| store.slow_mode_seconds(chat_id, cx)) > 0
        {
            Notifier::send(tr("chat_text_split_slow_mode"), bottom_options(NotifierType::Info), cx);
            return None;
        }
        Some(parts)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn send_text_parts(
        &mut self,
        parts: &[ChatComposerTextSplitterPart],
        chat_id: &str,
        topic_id: Option<&str>,
        post_topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        reply_to_id: Option<&str>,
        link_preview: Option<ChatLinkPreviewSelection>,
        cx: &mut Context<Self>,
    ) {
        if Self::rejects_guide_chat_mutation(chat_id, cx) {
            return;
        }
        if parts.len() > 1 {
            let replay = {
                let parts = parts.to_vec();
                let topic_id = owned(topic_id);
                let post_topic_id = owned(post_topic_id);
                let thread_root_id = owned(thread_root_id);
                let reply_to_id = owned(reply_to_id);
                let link_preview = link_preview.clone();
                Self::send_replay(move |store, real_id, cx| {
                    store.send_text_parts(
                        &parts,
                        &real_id,
                        topic_id.as_deref(),
                        post_topic_id.as_deref(),
                        thread_root_id.as_deref(),
                        reply_to_id.as_deref(),
                        link_preview.clone(),
                        cx,
                    );
                })
            };
            if self.defer_send_until_direct_chat_exists(chat_id, replay, cx) {
                return;
            }
        }
        let preview_index = if parts.len() == 1 {
            Some(0)
        } else {
            parts.iter().position(|part| ChatLinkPreviewURL::first(&part.text).is_some())
        };
        for (index, part) in parts.iter().enumerate() {
            self.send_text(
                chat_id,
                &part.text,
                topic_id,
                post_topic_id,
                thread_root_id,
                if index == 0 { reply_to_id } else { None },
                None,
                None,
                part.entities.as_deref(),
                None,
                if Some(index) == preview_index { link_preview.clone() } else { None },
                None,
                cx,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn send_text_as_file(
        &mut self,
        text: &str,
        file_name: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        post_topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        reply_to_id: Option<&str>,
        completion: Option<MessageActionCompletion>,
        cx: &mut Context<Self>,
    ) {
        if Self::rejects_guide_chat_mutation(chat_id, cx) {
            fail_send(completion, SaiWsSendError::GuideSandbox.into(), cx);
            return;
        }
        let path = temporary_directory().join(format!("ro-file-{}.txt", Uuid::new_v4().uuid_string()));
        let text = text.to_owned();
        let file_name = file_name.to_owned();
        let chat_id = chat_id.to_owned();
        let topic_id = owned(topic_id);
        let post_topic_id = owned(post_topic_id);
        let thread_root_id = owned(thread_root_id);
        let reply_to_id = owned(reply_to_id);
        cx.spawn(async move |this, cx| {
            let written_path = path.clone();
            let written = cx.background_executor().spawn(async move { std::fs::write(&written_path, text.as_bytes()) }).await;
            if let Err(error) = written {
                if let Some(completion) = completion {
                    cx.update(|cx| completion(Err(error.into()), cx));
                }
                return;
            }
            let Some(url) = Url::from_file_path(&path) else {
                if let Some(completion) = completion {
                    let error = std::io::Error::other("temporary file has no file url");
                    cx.update(|cx| completion(Err(error.into()), cx));
                }
                return;
            };
            let item = MediaPickerItem::new(
                Uuid::new_v4().uuid_string(),
                MediaPickerItemKind::File,
                url,
                file_name,
                None,
                None,
                None,
                None,
                true,
            );
            let _ = this.update(cx, |store, cx| {
                store.send_media(
                    &chat_id,
                    vec![item],
                    "",
                    topic_id.as_deref(),
                    post_topic_id.as_deref(),
                    thread_root_id.as_deref(),
                    reply_to_id.as_deref(),
                    None,
                    None,
                    None,
                    completion,
                    cx,
                );
            });
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn send_media(
        &mut self,
        chat_id: &str,
        items: Vec<MediaPickerItem>,
        caption: &str,
        topic_id: Option<&str>,
        post_topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        reply_to_id: Option<&str>,
        entities: Option<&str>,
        document: Option<&str>,
        link_preview: Option<ChatLinkPreviewSelection>,
        completion: Option<MessageActionCompletion>,
        cx: &mut Context<Self>,
    ) {
        if Self::rejects_guide_chat_mutation(chat_id, cx) {
            fail_send(completion, SaiWsSendError::GuideSandbox.into(), cx);
            return;
        }
        let completion: SharedCompletion = Rc::new(Cell::new(completion));
        let replay = {
            let items = items.clone();
            let caption = caption.to_owned();
            let topic_id = owned(topic_id);
            let post_topic_id = owned(post_topic_id);
            let thread_root_id = owned(thread_root_id);
            let reply_to_id = owned(reply_to_id);
            let entities = owned(entities);
            let document = owned(document);
            let link_preview = link_preview.clone();
            let completion = Rc::clone(&completion);
            Self::send_replay(move |store, real_id, cx| {
                store.send_media(
                    &real_id,
                    items.clone(),
                    &caption,
                    topic_id.as_deref(),
                    post_topic_id.as_deref(),
                    thread_root_id.as_deref(),
                    reply_to_id.as_deref(),
                    entities.as_deref(),
                    document.as_deref(),
                    link_preview.clone(),
                    completion.take(),
                    cx,
                );
            })
        };
        if self.defer_send_until_direct_chat_exists(chat_id, replay, cx) {
            return;
        }

        let Some(chat_id) = self.resolved_send_chat_id(chat_id, cx) else {
            return;
        };
        if items.is_empty() {
            return;
        }
        if items.len() > 1 && items.iter().any(|item| item.view_mode.is_some()) {
            let last = items.len() - 1;
            for (index, item) in items.into_iter().enumerate() {
                let first = index == 0;
                self.send_media(
                    &chat_id,
                    vec![item],
                    if first { caption } else { "" },
                    topic_id,
                    post_topic_id,
                    thread_root_id,
                    if first { reply_to_id } else { None },
                    if first { entities } else { None },
                    if first { document } else { None },
                    if first { link_preview.clone() } else { None },
                    if index == last { completion.take() } else { None },
                    cx,
                );
            }
            return;
        }
        if let Some(oversize) = Self::oversize_file_error(&items, cx) {
            tracing::warn!(target: "riseonly::X1428", ?oversize, "send media refused over the size limit");
            fail_send(completion.take(), oversize.into(), cx);
            return;
        }

        let scope = MessageServicesStore::message_scope(&chat_id, topic_id, thread_root_id);
        let local_files: Vec<RiseOutgoingMediaFile> = items
            .iter()
            .map(|item| {
                let is_document = matches!(item.kind, MediaPickerItemKind::File) || item.is_file;
                let media_type = if is_document {
                    Some(MessageContentType::Document.raw_value().to_owned())
                } else if Self::is_animated_image(item) {
                    Some(MessageContentType::Animation.raw_value().to_owned())
                } else {
                    None
                };
                RiseOutgoingMediaFile {
                    media_type,
                    ..RiseOutgoingMediaFile::new(
                        file_path(&item.url),
                        item.suggested_file_name.clone(),
                        Self::mime_type(item),
                    )
                }
            })
            .collect();
        let temp_id = timestamped_temp_id();
        let first_kind_content_type = Self::first_kind_content_type(&items);
        let media_group_id = (items.len() > 1).then(|| format!("mg_{}", RiseClock::now().ms()));
        let resolved_reply = Self::resolved_reply(reply_to_id, thread_root_id, &scope, cx);

        Self::persist_first_send_reedit_doc(&items, &temp_id, cx);

        let (link_preview_type, link_preview_metadata) = match link_preview {
            Some(selection) => (Some(selection.wire_type), selection.metadata),
            None => (None, None),
        };
        let body = RiseStoredOutgoingMessage {
            chat_id: Some(chat_id.clone()),
            topic_id: owned(post_topic_id.or(topic_id)),
            content: caption.to_owned(),
            original_content: caption.to_owned(),
            content_type: first_kind_content_type,
            reply_to_id: resolved_reply,
            media_group_id,
            entities: owned(entities),
            caption: (!caption.is_empty()).then(|| caption.to_owned()),
            document: owned(document),
            link_preview_type,
            link_preview: link_preview_metadata,
            ..RiseStoredOutgoingMessage::default()
        };

        self.begin_upload_typing(&chat_id, &temp_id, Self::upload_typing_action(&items), cx);
        let upload_completion: MessageActionCompletion = {
            let this = cx.entity().downgrade();
            let chat_id = chat_id.clone();
            let temp_id = temp_id.clone();
            let completion = completion.take();
            Box::new(move |result, app: &mut App| {
                app.defer(move |app| {
                    let _ = this.update(app, |store, cx| store.end_upload_typing(&chat_id, &temp_id, cx));
                });
                if let Some(completion) = completion {
                    completion(result, app);
                }
            })
        };
        Self::create_message(
            body,
            &chat_id,
            topic_id,
            thread_root_id,
            local_files,
            items,
            Vec::new(),
            &temp_id,
            true,
            None,
            Some(upload_completion),
            cx,
        );

        Self::bump_chat_list_with_last_message_in(&chat_id, cx);
    }

    fn upload_typing_action(items: &[MediaPickerItem]) -> ChatAction {
        if items.iter().any(|item| item.is_file || matches!(item.kind, MediaPickerItemKind::File)) {
            return ChatAction::UploadDocument;
        }
        if items.iter().any(|item| matches!(item.kind, MediaPickerItemKind::Video)) {
            return ChatAction::UploadVideo;
        }
        ChatAction::UploadPhoto
    }

    fn first_kind_content_type(items: &[MediaPickerItem]) -> String {
        let Some(first) = items.first() else {
            return MessageContentType::Image.raw_value().to_owned();
        };
        if first.is_file {
            return MessageContentType::Document.raw_value().to_owned();
        }
        if items.len() == 1 && Self::is_animated_image(first) {
            return MessageContentType::Animation.raw_value().to_owned();
        }
        match first.kind {
            MediaPickerItemKind::Image => MessageContentType::Image.raw_value().to_owned(),
            MediaPickerItemKind::Video => MessageContentType::Video.raw_value().to_owned(),
            MediaPickerItemKind::File => MessageContentType::Document.raw_value().to_owned(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn send_audio(
        &mut self,
        chat_id: &str,
        item: &MediaPickerItem,
        caption: &str,
        topic_id: Option<&str>,
        post_topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        reply_to_id: Option<&str>,
        entities: Option<&str>,
        link_preview: Option<ChatLinkPreviewSelection>,
        completion: Option<MessageActionCompletion>,
        cx: &mut Context<Self>,
    ) {
        let completion: SharedCompletion = Rc::new(Cell::new(completion));
        let replay = {
            let item = item.clone();
            let caption = caption.to_owned();
            let topic_id = owned(topic_id);
            let post_topic_id = owned(post_topic_id);
            let thread_root_id = owned(thread_root_id);
            let reply_to_id = owned(reply_to_id);
            let entities = owned(entities);
            let link_preview = link_preview.clone();
            let completion = Rc::clone(&completion);
            Self::send_replay(move |store, real_id, cx| {
                store.send_audio(
                    &real_id,
                    &item,
                    &caption,
                    topic_id.as_deref(),
                    post_topic_id.as_deref(),
                    thread_root_id.as_deref(),
                    reply_to_id.as_deref(),
                    entities.as_deref(),
                    link_preview.clone(),
                    completion.take(),
                    cx,
                );
            })
        };
        if self.defer_send_until_direct_chat_exists(chat_id, replay, cx) {
            return;
        }

        let Some(chat_id) = self.resolved_send_chat_id(chat_id, cx) else {
            return;
        };
        let Some(audio) = item.audio_payload.as_ref() else {
            return;
        };

        let scope = MessageServicesStore::message_scope(&chat_id, topic_id, thread_root_id);
        let chat_context = RiseUploadContextFormats::CHAT.context.to_owned();
        let mut local_files: Vec<RiseOutgoingMediaFile> = Vec::new();
        let mut picker_items: Vec<MediaPickerItem> = Vec::new();
        if let Some(cover_url) = audio.cover_url.as_ref() {
            let cover_path = file_path(cover_url);
            local_files.push(RiseOutgoingMediaFile {
                file_context: Some(chat_context.clone()),
                media_type: Some(MessageContentType::Image.raw_value().to_owned()),
                ..RiseOutgoingMediaFile::new(
                    cover_path.clone(),
                    cover_url.last_path_component(),
                    MusicTrackFileUpload::mime_type(&cover_path),
                )
            });
            picker_items.push(MediaPickerItem::new(
                Uuid::new_v4().uuid_string(),
                MediaPickerItemKind::Image,
                cover_url.clone(),
                cover_url.last_path_component(),
                None,
                None,
                None,
                None,
                false,
            ));
        }
        let audio_path = file_path(&item.url);
        local_files.push(RiseOutgoingMediaFile {
            file_context: Some(chat_context),
            media_type: Some(MessageContentType::Audio.raw_value().to_owned()),
            ..RiseOutgoingMediaFile::new(
                audio_path.clone(),
                item.audio_file_name().unwrap_or_else(|| item.suggested_file_name.clone()),
                MusicTrackFileUpload::audio_mime_type(&audio_path),
            )
        });
        picker_items.push(item.clone());

        let temp_id = timestamped_temp_id();
        let resolved_reply = Self::resolved_reply(reply_to_id, thread_root_id, &scope, cx);
        let (link_preview_type, link_preview_metadata) = match link_preview {
            Some(selection) => (Some(selection.wire_type), selection.metadata),
            None => (None, None),
        };
        let body = RiseStoredOutgoingMessage {
            chat_id: Some(chat_id.clone()),
            topic_id: owned(post_topic_id.or(topic_id)),
            content: caption.to_owned(),
            original_content: caption.to_owned(),
            content_type: MessageContentType::Audio.raw_value().to_owned(),
            reply_to_id: resolved_reply,
            entities: owned(entities),
            caption: (!caption.is_empty()).then(|| caption.to_owned()),
            link_preview_type,
            link_preview: link_preview_metadata,
            ..RiseStoredOutgoingMessage::default()
        };

        Self::create_message(
            body,
            &chat_id,
            topic_id,
            thread_root_id,
            local_files,
            picker_items,
            Vec::new(),
            &temp_id,
            true,
            None,
            completion.take(),
            cx,
        );
        Self::bump_chat_list_with_last_message_in(&chat_id, cx);
    }

    fn persist_first_send_reedit_doc(items: &[MediaPickerItem], temp_id: &str, cx: &mut App) {
        let [item] = items else {
            return;
        };
        let Some(img) = item.image_payload.as_ref() else {
            return;
        };
        if !img.was_edited {
            return;
        }
        let (Some(state_data), Some(original_url)) = (img.editor_state_data.as_ref(), img.original_image_url.as_ref())
        else {
            return;
        };
        let Some(original_path) = original_url.to_file_path() else {
            return;
        };
        let reedit = MediaReeditStore::shared(cx);
        let Some(edit_id) = reedit.update(cx, |store, _| {
            store.save_document(&original_path, state_data, MediaReeditStore::chat_ttl_seconds())
        }) else {
            return;
        };
        reedit.update(cx, |store, _| store.register_pending_temp(temp_id, &edit_id));
        let temp_id = temp_id.to_owned();
        Sai::log(
            "Reedit",
            move || format!("saved doc on send editId={edit_id} pending tempId={temp_id}"),
            SaiLogKind::Debug,
            cx,
        );
    }

    pub fn make_video_note_item(
        &self,
        recipe: &VideoNoteCaptureRecipe,
        poster_url: Option<Url>,
        duration: f64,
        caption: &str,
    ) -> Option<MediaPickerItem> {
        let source_url = recipe
            .segments
            .first()
            .and_then(|segment| Url::from_file_path(&segment.url))
            .or_else(|| Url::from_file_path(Path::new("/dev/null")))?;
        let mut video_payload =
            MediaPickerEditedVideo::new(source_url.clone(), 0.0, duration, duration, false, caption.to_owned());
        video_payload.pixel_size =
            size(VideoNoteComposition::CANVAS_SIZE.width as f32, VideoNoteComposition::CANVAS_SIZE.height as f32);
        video_payload.poster_url = poster_url;
        Some(MediaPickerItem::new(
            Uuid::new_v4().uuid_string(),
            MediaPickerItemKind::Video,
            source_url,
            format!("video-note-{}.mp4", Uuid::new_v4().uuid_string()),
            None,
            None,
            Some(video_payload),
            None,
            false,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub fn send_video_note(
        &mut self,
        chat_id: &str,
        item: &MediaPickerItem,
        recipe: Option<VideoNoteCaptureRecipe>,
        caption: &str,
        topic_id: Option<&str>,
        post_topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        publish_to_shorts_feed: bool,
        reply_to_id: Option<&str>,
        entities: Option<&str>,
        completion: Option<MessageActionCompletion>,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let completion: SharedCompletion = Rc::new(Cell::new(completion));
        let replay = {
            let item = item.clone();
            let recipe = recipe.clone();
            let caption = caption.to_owned();
            let topic_id = owned(topic_id);
            let post_topic_id = owned(post_topic_id);
            let thread_root_id = owned(thread_root_id);
            let reply_to_id = owned(reply_to_id);
            let entities = owned(entities);
            let completion = Rc::clone(&completion);
            Self::send_replay(move |store, real_id, cx| {
                store.send_video_note(
                    &real_id,
                    &item,
                    recipe.clone(),
                    &caption,
                    topic_id.as_deref(),
                    post_topic_id.as_deref(),
                    thread_root_id.as_deref(),
                    publish_to_shorts_feed,
                    reply_to_id.as_deref(),
                    entities.as_deref(),
                    completion.take(),
                    cx,
                );
            })
        };
        if self.defer_send_until_direct_chat_exists(chat_id, replay, cx) {
            return None;
        }

        let resolved_chat_id =
            self.resolved_send_chat_id(chat_id, cx).filter(|_| matches!(item.kind, MediaPickerItemKind::Video));
        let Some(chat_id) = resolved_chat_id else {
            if let Some(recipe) = recipe.as_ref() {
                VideoNoteComposition::discard_sources(recipe);
            }
            return None;
        };

        let scope = MessageServicesStore::message_scope(&chat_id, topic_id, thread_root_id);
        let content_type =
            if recipe.is_none() { Self::mime_type(item) } else { RiseUploadFormat::VIDEO_MP4.mime.to_owned() };
        let local_file = RiseOutgoingMediaFile {
            media_type: Some(MessageContentType::VideoNote.raw_value().to_owned()),
            video_note: recipe,
            ..RiseOutgoingMediaFile::new(file_path(&item.url), item.suggested_file_name.clone(), content_type)
        };
        let temp_id = timestamped_temp_id();
        let resolved_reply = Self::resolved_reply(reply_to_id, thread_root_id, &scope, cx);
        let trimmed_caption = trimming_whitespaces_and_newlines(caption);

        let body = RiseStoredOutgoingMessage {
            chat_id: Some(chat_id.clone()),
            topic_id: owned(post_topic_id.or(topic_id)),
            content: trimmed_caption.to_owned(),
            original_content: trimmed_caption.to_owned(),
            content_type: MessageContentType::VideoNote.raw_value().to_owned(),
            reply_to_id: resolved_reply,
            entities: owned(entities),
            caption: (!trimmed_caption.is_empty()).then(|| trimmed_caption.to_owned()),
            publish_to_shorts_feed: publish_to_shorts_feed.then_some(true),
            ..RiseStoredOutgoingMessage::default()
        };

        Self::create_message(
            body,
            &chat_id,
            topic_id,
            thread_root_id,
            vec![local_file],
            vec![item.clone()],
            Vec::new(),
            &temp_id,
            true,
            None,
            completion.take(),
            cx,
        );

        Self::bump_chat_list_with_last_message_in(&chat_id, cx);

        Some(temp_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn send_voice(
        &mut self,
        chat_id: &str,
        file_url: &Path,
        metadata: &VoiceMessageMetadata,
        topic_id: Option<&str>,
        post_topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        reply_to_id: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let replay = {
            let file_url = file_url.to_path_buf();
            let metadata = metadata.clone();
            let topic_id = owned(topic_id);
            let post_topic_id = owned(post_topic_id);
            let thread_root_id = owned(thread_root_id);
            let reply_to_id = owned(reply_to_id);
            Self::send_replay(move |store, real_id, cx| {
                store.send_voice(
                    &real_id,
                    &file_url,
                    &metadata,
                    topic_id.as_deref(),
                    post_topic_id.as_deref(),
                    thread_root_id.as_deref(),
                    reply_to_id.as_deref(),
                    cx,
                );
            })
        };
        if self.defer_send_until_direct_chat_exists(chat_id, replay, cx) {
            return;
        }

        let Some(chat_id) = self.resolved_send_chat_id(chat_id, cx) else {
            return;
        };

        let scope = MessageServicesStore::message_scope(&chat_id, topic_id, thread_root_id);
        let temp_id = timestamped_temp_id();
        let resolved_reply = Self::resolved_reply(reply_to_id, thread_root_id, &scope, cx);
        let local_file = RiseOutgoingMediaFile {
            media_type: Some(MessageContentType::Voice.raw_value().to_owned()),
            ..RiseOutgoingMediaFile::new(
                file_url.to_path_buf(),
                format!("voice-{}.m4a", Uuid::new_v4().uuid_string()),
                RiseUploadFormat::AUDIO_MP4.mime.to_owned(),
            )
        };

        let body = RiseStoredOutgoingMessage {
            chat_id: Some(chat_id.clone()),
            topic_id: owned(post_topic_id.or(topic_id)),
            content_type: MessageContentType::Voice.raw_value().to_owned(),
            reply_to_id: resolved_reply,
            ..RiseStoredOutgoingMessage::default()
        };

        Self::create_message(
            body,
            &chat_id,
            topic_id,
            thread_root_id,
            vec![local_file],
            Vec::new(),
            vec![metadata.clone()],
            &temp_id,
            true,
            None,
            None,
            cx,
        );

        Self::bump_chat_list_with_last_message_in(&chat_id, cx);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn share_post_to_chat(
        &mut self,
        chat_id: &str,
        post: &GetPostFeedResponse,
        topic_id: Option<&str>,
        idempotency_key: Option<String>,
        defer_until_connected: bool,
        completion: Option<MessageActionCompletion>,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let completion: SharedCompletion = Rc::new(Cell::new(completion));
        let replay = {
            let post = post.clone();
            let topic_id = owned(topic_id);
            let idempotency_key = idempotency_key.clone();
            let completion = Rc::clone(&completion);
            Self::send_replay(move |store, real_id, cx| {
                store.share_post_to_chat(
                    &real_id,
                    &post,
                    topic_id.as_deref(),
                    idempotency_key.clone(),
                    defer_until_connected,
                    completion.take(),
                    cx,
                );
            })
        };
        if self.defer_send_until_direct_chat_exists(chat_id, replay, cx) {
            return None;
        }

        let chat_id = self.resolved_send_chat_id(chat_id, cx)?;
        let post_id = post.rise_view_target_post_id().parse::<i64>().ok().filter(|post_id| *post_id > 0)?;

        let requested_key = idempotency_key.as_deref().map(trimming_whitespaces_and_newlines);
        let temp_id = match requested_key {
            Some(key) if !key.is_empty() && key.len() <= IDEMPOTENCY_KEY_MAX_BYTES => key.to_owned(),
            _ => lowercased_uuid_temp_id(),
        };
        let trimmed_title = trimming_whitespaces_and_newlines(&post.title);
        let preview_text =
            if trimmed_title.is_empty() { trimming_whitespaces_and_newlines(&post.content) } else { trimmed_title };

        let body = RiseStoredOutgoingMessage {
            chat_id: Some(chat_id.clone()),
            topic_id: owned(topic_id),
            content: preview_text.to_owned(),
            original_content: preview_text.to_owned(),
            content_type: ChatMessageContentType::PostShort.raw_value().to_owned(),
            post_id: Some(post_id),
            shared_post_info: Self::optimistic_shared_post_info(post, post_id),
            temp_id: Some(temp_id.clone()),
            ..RiseStoredOutgoingMessage::default()
        };

        Self::create_message(
            body,
            &chat_id,
            topic_id,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            &temp_id,
            defer_until_connected,
            (!defer_until_connected).then_some(IMMEDIATE_SHARE_RESPONSE_TIMEOUT_SECONDS),
            completion.take(),
            cx,
        );

        Self::bump_chat_list_with_last_message_in(&chat_id, cx);

        Some(temp_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn send_story_reply(
        &mut self,
        chat_id: &str,
        text: &str,
        story: &StoryDTO,
        author: Option<&StoryAuthorDTO>,
        author_id: &str,
        completion: Option<MessageActionCompletion>,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let completion: SharedCompletion = Rc::new(Cell::new(completion));
        let replay = {
            let text = text.to_owned();
            let story = story.clone();
            let author = author.cloned();
            let author_id = author_id.to_owned();
            let completion = Rc::clone(&completion);
            Self::send_replay(move |store, real_id, cx| {
                store.send_story_reply(&real_id, &text, &story, author.as_ref(), &author_id, completion.take(), cx);
            })
        };
        if self.defer_send_until_direct_chat_exists(chat_id, replay, cx) {
            return None;
        }

        let chat_id = self.resolved_send_chat_id(chat_id, cx)?;
        if story.id <= 0 {
            return None;
        }

        let trimmed = trimming_whitespaces_and_newlines(text);
        if trimmed.is_empty() {
            return None;
        }

        let temp_id = lowercased_uuid_temp_id();
        let body = RiseStoredOutgoingMessage {
            chat_id: Some(chat_id.clone()),
            content: trimmed.to_owned(),
            original_content: trimmed.to_owned(),
            content_type: ChatMessageContentType::Text.raw_value().to_owned(),
            story_id: Some(story.id),
            story_reply_info: Self::optimistic_story_reply_info(story, author, author_id),
            temp_id: Some(temp_id.clone()),
            ..RiseStoredOutgoingMessage::default()
        };

        Self::create_message(
            body,
            &chat_id,
            None,
            None,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            &temp_id,
            true,
            None,
            completion.take(),
            cx,
        );

        Self::bump_chat_list_with_last_message_in(&chat_id, cx);

        Some(temp_id)
    }

    fn optimistic_story_reply_info(story: &StoryDTO, author: Option<&StoryAuthorDTO>, author_id: &str) -> Option<String> {
        let mut author_object = Map::new();
        author_object.insert(rise_key::post::ID.to_owned(), Value::from(author_id));
        author_object.insert(rise_key::post::TAG.to_owned(), Value::from(author.map_or("", |author| author.tag.as_str())));
        author_object
            .insert(rise_key::post::NAME.to_owned(), Value::from(author.map_or("", |author| author.name.as_str())));
        author_object
            .insert(rise_key::post::LOGO.to_owned(), Value::from(author.map_or("", |author| author.logo.as_str())));
        author_object.insert(rise_key::post::IS_PREMIUM.to_owned(), Value::from(author.is_some_and(|author| author.is_premium)));

        let caption = prefix_characters(story.caption.as_deref().unwrap_or_default(), STORY_CAPTION_PREVIEW_CHARACTERS);
        let mut object = Map::new();
        object.insert(rise_key::events::STORY_ID.to_owned(), Value::from(story.id));
        object.insert(rise_key::events::MEDIA_URL.to_owned(), Value::from(story.file_id.as_str()));
        object.insert(
            rise_key::payload::IS_VIDEO.to_owned(),
            Value::from(story.media_type.to_lowercase() == MessageContentType::Video.raw_value()),
        );
        object.insert(rise_key::events::CAPTION.to_owned(), Value::from(caption));
        object.insert(rise_key::post::CREATED_AT_MS.to_owned(), Value::from(story.created_at_ms));
        object.insert(rise_key::post::EXPIRES_AT_MS.to_owned(), Value::from(story.expires_at_ms));
        object.insert(rise_key::stories::POST_TO_PROFILE.to_owned(), Value::from(story.post_to_profile));
        object.insert(rise_key::post::AUTHOR.to_owned(), Value::Object(author_object));

        String::from_utf8(encoded_json(object, false)?).ok()
    }

    fn optimistic_shared_post_info(post: &GetPostFeedResponse, post_id: i64) -> Option<String> {
        let mut author = Map::new();
        author.insert(
            rise_key::post::ID.to_owned(),
            Value::from(post.author.id.as_deref().unwrap_or(post.author_id.as_str())),
        );
        author.insert(rise_key::post::TAG.to_owned(), Value::from(post.author.tag.as_str()));
        author.insert(rise_key::post::NAME.to_owned(), Value::from(post.author.name.as_str()));
        author.insert(rise_key::post::LOGO.to_owned(), Value::from(post.author.more.logo.as_str()));
        author.insert(rise_key::post::IS_PREMIUM.to_owned(), Value::from(post.author.more.is_premium));

        let mut object = Map::new();
        object.insert(rise_key::post::POST_ID.to_owned(), Value::from(post_id));
        object.insert(rise_key::post::IS_SHORT.to_owned(), Value::from(post.is_short));
        object.insert(rise_key::post::TITLE.to_owned(), Value::from(post.title.as_str()));
        object.insert(
            rise_key::message::SNIPPET.to_owned(),
            Value::from(prefix_characters(&post.content, POST_SNIPPET_PREVIEW_CHARACTERS)),
        );
        object.insert(rise_key::post::IMAGES.to_owned(), Value::from(post.images.clone()));
        object.insert(rise_key::post::VIDEO.to_owned(), Value::from(post.video.as_deref().unwrap_or_default()));
        object.insert(
            rise_key::post::VIDEO_THUMBNAIL.to_owned(),
            Value::from(post.video_thumbnail.as_deref().unwrap_or_default()),
        );
        object.insert(rise_key::post::LIKES_COUNT.to_owned(), Value::from(post.likes_count));
        object.insert(rise_key::post::COMMENTS_COUNT.to_owned(), Value::from(post.comments_count));
        object.insert(rise_key::post::AUTHOR.to_owned(), Value::Object(author));

        String::from_utf8(encoded_json(object, false)?).ok()
    }

    #[allow(clippy::too_many_arguments)]
    pub fn send_sticker(
        &mut self,
        chat_id: &str,
        sticker: &StickerItem,
        caption: &str,
        topic_id: Option<&str>,
        post_topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        reply_to_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let replay = {
            let sticker = sticker.clone();
            let caption = caption.to_owned();
            let topic_id = owned(topic_id);
            let post_topic_id = owned(post_topic_id);
            let thread_root_id = owned(thread_root_id);
            let reply_to_id = owned(reply_to_id);
            Self::send_replay(move |store, real_id, cx| {
                store.send_sticker(
                    &real_id,
                    &sticker,
                    &caption,
                    topic_id.as_deref(),
                    post_topic_id.as_deref(),
                    thread_root_id.as_deref(),
                    reply_to_id.as_deref(),
                    cx,
                );
            })
        };
        if self.defer_send_until_direct_chat_exists(chat_id, replay, cx) {
            return None;
        }

        let chat_id = self.resolved_send_chat_id(chat_id, cx)?;
        if sticker.file_url.is_empty() {
            return None;
        }
        if ChatInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.send_switched_off(&chat_id, ChatPermKey::STICKERS, cx))
        {
            return None;
        }

        let scope = MessageServicesStore::message_scope(&chat_id, topic_id, thread_root_id);
        let temp_id = timestamped_temp_id();
        let resolved_reply = Self::resolved_reply(reply_to_id, thread_root_id, &scope, cx);

        let mut body = Self::service(cx).read(cx).build_create_message_body_for_sticker(
            sticker,
            Some(caption),
            resolved_reply.as_deref(),
            &chat_id,
            post_topic_id.or(topic_id),
        );
        body.topic_id = owned(post_topic_id.or(topic_id));

        StickerPremiumEffectRegistry::shared().arm_send(&sticker.file_url);
        Self::create_message(
            body,
            &chat_id,
            topic_id,
            thread_root_id,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            &temp_id,
            true,
            None,
            None,
            cx,
        );

        Self::bump_chat_list_with_last_message_in(&chat_id, cx);

        let user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        if !user_id.is_empty() {
            if let Some(stickers) = message_send_stickers_hooks() {
                (stickers.write_recent)(&user_id, &sticker.id, Some(PackKind::Sticker), cx);
            }
        }

        Some(temp_id)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn send_gif(
        &mut self,
        chat_id: &str,
        gif: &GifItem,
        caption: &str,
        topic_id: Option<&str>,
        post_topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        reply_to_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let replay = {
            let gif = gif.clone();
            let caption = caption.to_owned();
            let topic_id = owned(topic_id);
            let post_topic_id = owned(post_topic_id);
            let thread_root_id = owned(thread_root_id);
            let reply_to_id = owned(reply_to_id);
            Self::send_replay(move |store, real_id, cx| {
                store.send_gif(
                    &real_id,
                    &gif,
                    &caption,
                    topic_id.as_deref(),
                    post_topic_id.as_deref(),
                    thread_root_id.as_deref(),
                    reply_to_id.as_deref(),
                    cx,
                );
            })
        };
        if self.defer_send_until_direct_chat_exists(chat_id, replay, cx) {
            return None;
        }

        let chat_id = self.resolved_send_chat_id(chat_id, cx)?;
        if gif.file_url.is_empty() {
            return None;
        }
        if ChatInteractionsStore::shared(cx)
            .update(cx, |store, cx| store.send_switched_off(&chat_id, ChatPermKey::GIF, cx))
        {
            return None;
        }

        let scope = MessageServicesStore::message_scope(&chat_id, topic_id, thread_root_id);
        let temp_id = timestamped_temp_id();
        let resolved_reply = Self::resolved_reply(reply_to_id, thread_root_id, &scope, cx);

        let mut body = Self::service(cx).read(cx).build_create_message_body_for_gif(
            gif,
            Some(caption),
            resolved_reply.as_deref(),
            &chat_id,
            post_topic_id.or(topic_id),
        );
        body.topic_id = owned(post_topic_id.or(topic_id));

        Self::create_message(
            body,
            &chat_id,
            topic_id,
            thread_root_id,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            &temp_id,
            true,
            None,
            None,
            cx,
        );

        Self::bump_chat_list_with_last_message_in(&chat_id, cx);

        let user_id = AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire().unwrap_or_default();
        if !user_id.is_empty() {
            if let Some(stickers) = message_send_stickers_hooks() {
                (stickers.add_gif_favourite)(&user_id, gif, cx);
            }
        }

        Some(temp_id)
    }

    pub fn media_send_switched_off(
        &mut self,
        chat_id: &str,
        items: &[MediaPickerItem],
        as_video_note: bool,
        forum_topic_id: Option<&str>,
        cx: &mut Context<Self>,
    ) -> bool {
        let keys: BTreeSet<&'static str> = if as_video_note {
            BTreeSet::from([ChatPermKey::VIDEO_MESSAGES])
        } else {
            items.iter().flat_map(Self::send_permission_keys).collect()
        };
        keys.into_iter().any(|key| {
            ChatInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.send_switched_off_forum_topic_id(chat_id, key, forum_topic_id, cx))
        })
    }

    fn send_permission_keys(item: &MediaPickerItem) -> Vec<&'static str> {
        if item.audio_payload.is_some() {
            return vec![ChatPermKey::MUSIC];
        }
        if item.is_file {
            let mime_type = Self::mime_type(item);
            let kind = RiseMediaKindClassifier::kind(Some(mime_type.as_str()), Some(item.suggested_file_name.as_str()));
            return if matches!(kind, RiseMediaKind::Audio) {
                vec![ChatPermKey::FILES, ChatPermKey::MUSIC]
            } else {
                vec![ChatPermKey::FILES]
            };
        }
        if Self::is_animated_image(item) {
            return vec![ChatPermKey::GIF];
        }
        match item.kind {
            MediaPickerItemKind::Image => vec![ChatPermKey::PHOTO],
            MediaPickerItemKind::Video => vec![ChatPermKey::VIDEO],
            MediaPickerItemKind::File => vec![ChatPermKey::FILES],
        }
    }

    fn is_animated_image(item: &MediaPickerItem) -> bool {
        matches!(item.kind, MediaPickerItemKind::Image)
            && !item.is_file
            && Self::mime_type(item) == RiseUploadFormat::IMAGE_GIF.mime
    }

    fn oversize_file_error(items: &[MediaPickerItem], cx: &mut App) -> Option<SaiWsChunkedFileUploadError> {
        let premium = AuthServicesStore::shared(cx).read(cx).profile().is_some_and(|profile| profile.more.is_premium);
        for item in items {
            if !matches!(item.kind, MediaPickerItemKind::File) && !item.is_file {
                continue;
            }
            let Some(path) = item.url.to_file_path() else {
                continue;
            };
            let Ok(metadata) = std::fs::metadata(path) else {
                continue;
            };
            let size = i64::try_from(metadata.len()).unwrap_or(i64::MAX);
            let maximum_bytes = i64::try_from(rise_upload::PIPELINE_MAXIMUM_FILE_BYTES).unwrap_or(i64::MAX).min(
                RiseLimits::upload_maximum_bytes(
                    RiseUploadContextFormats::CHAT.context,
                    &Self::mime_type(item),
                    premium,
                ),
            );
            if size > maximum_bytes {
                return Some(SaiWsChunkedFileUploadError::FileTooLarge { byte_count: size, maximum_bytes });
            }
        }
        None
    }

    fn mime_type(item: &MediaPickerItem) -> String {
        if let Some(mime_type) = UTType::from_filename_extension(&item.url.path_extension())
            .and_then(|uniform_type| uniform_type.preferred_mime_type())
        {
            return mime_type;
        }
        match item.kind {
            MediaPickerItemKind::Image => RiseUploadFormat::IMAGE_JPEG.mime.to_owned(),
            MediaPickerItemKind::Video => RiseUploadFormat::VIDEO_MP4.mime.to_owned(),
            MediaPickerItemKind::File => RiseUploadFormat::APPLICATION_OCTET_STREAM.mime.to_owned(),
        }
    }
}
