use gpui::prelude::*;
use gpui::{App, AppContext, Context, Keystroke, Window};
use rise_canon::RiseGuideSignal;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::ns_range::NSRange;
use rise_ui::keyboard_dismiss::ScreenFocus;
use rise_ui::swiftui::environment::env;
use rise_ui::swiftui::prelude::*;
use crate::modules::chat::components::chat::composer::chat_composer_inline_context::ChatComposerMentionEntityCodec;
use rise_bot_api::ChatAction;
use crate::modules::chat::components::chat::composer::chat_composer_types::{ChatBottomBarMetrics, ChatDraftCustomEmoji};
use crate::modules::chat::components::chat::composer::chat_draft_store::ChatDraftStore;
use crate::modules::chat::pages::chats::chat::chat_roles::role_mention_composer::RoleMentionComposer;
use crate::modules::chat::pages::chats::chat::chat_roles::topic_mention_composer::TopicMentionComposer;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;
use crate::modules::media_picker::api::media_picker_result::MediaPickerItem;
use rise_ui::swiftui::button_style::plain;
use rise_core::foundation::foundation_url::Url;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::modules::chat::components::chat::composer::chat_composer_types::{ChatComposerCaretEdit, ChatComposerCaretEditResult, ChatComposerCustomEmojiEditHydrator, ChatComposerCustomEmojiRun};
use crate::modules::chat::shared::utils::chat_composer_text_splitter::{ChatComposerTextSplitter, ChatComposerTextSplitterBudget};
use crate::modules::chat::stores::message::message_actions::message_types::{MessageCustomEmojiEntity, RemoteMediaURL};
use crate::modules::chat::stores::message::message_actions::message_wire_types::{CustomEmojiEntityWire, MessageEntitiesCodec};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{StickerAssetType, StickerAssetTypeInfer, StickerItem};
use crate::modules::stickers::stores::stickers::stickers_services::stickers_services::StickersServicesStore;
use crate::modules::media_picker::api::media_picker_result::MediaPickerItemKind;
use super::super::chat_screen::*;
use super::super::chat_screen_support::*;

impl ChatScreen {
    /// drafts belong to the send destination, also when composing from the aggregate feed
    pub(in super::super) fn composer_draft_scope_key(&self, cx: &mut App) -> String {
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.inline_composer_topic_id(cx);
        ChatDraftStore::scope_key(&chat_id, topic_id.as_deref(), self.resolved_thread_root_id())
    }

    pub(in super::super) fn persist_composer_draft(&mut self, text: &str, scope_key: Option<&str>, _window: &mut Window, cx: &mut Context<Self>) {
        self.persist_composer_draft_text(text, scope_key, cx);
    }

    pub(in super::super) fn persist_composer_draft_text(&mut self, text: &str, scope_key: Option<&str>, cx: &mut Context<Self>) {
        if self.is_context_preview(cx) || self.resolved_chat_id(cx).is_empty() {
            return;
        }
        if self.state.edit_target.is_some() {
            return;
        }
        let custom_emojis: Vec<StickerItem> = self.draft_custom_emojis.iter().map(|draft| draft.item.clone()).collect();
        let mentions = self.inline_user_mentions.clone();
        let link_preview = self.link_preview_draft.read(cx).preference();
        let scope = match scope_key {
            Some(scope_key) => scope_key.to_owned(),
            None => self.composer_draft_scope_key(cx),
        };
        draft_store(cx, |store, cx| store.set_draft(text, custom_emojis, mentions, Some(link_preview), &scope, cx));
    }

    pub(in super::super) fn handle_composer_draft_scope_change(
        &mut self,
        previous_key: &str,
        current_key: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.restored_draft_scope_key.as_deref() == Some(current_key) || self.resolved_chat_id(cx).is_empty() {
            return;
        }
        let previous_was_restored = self.restored_draft_scope_key.as_deref() == Some(previous_key);
        let previous_chat_id = ChatDraftStore::chat_id(previous_key);
        let carries_identity = previous_key != current_key
            && previous_was_restored
            && ChatServicesStore::shared(cx).read(cx).canonical_chat_id(&previous_chat_id) == ChatDraftStore::chat_id(current_key);
        let input_text = self.state.input_text.clone();
        if carries_identity {
            self.restored_draft_scope_key = Some(current_key.to_owned());
            draft_store(cx, |store, cx| store.clear(previous_key, cx));
            self.persist_composer_draft_text(&input_text, None, cx);
            return;
        }
        if previous_key != current_key && previous_was_restored {
            self.persist_composer_draft_text(&input_text, Some(previous_key), cx);
            draft_store(cx, |store, cx| store.publish_pending_previews(cx));
        }
        self.adopt_composer_draft(current_key, cx);
    }

    pub(in super::super) fn adopt_composer_draft(&mut self, scope_key: &str, cx: &mut Context<Self>) {
        self.restored_draft_scope_key = Some(scope_key.to_owned());
        if self.state.edit_target.is_some() {
            return;
        }
        if self.channel_posting_locked(cx) {
            draft_store(cx, |store, cx| store.clear(scope_key, cx));
            return;
        }
        let draft = draft_store(cx, |store, cx| store.draft(scope_key, cx));
        let text = draft.as_ref().map(|draft| draft.text.clone()).unwrap_or_default();
        let link_preview = draft.as_ref().and_then(|draft| draft.link_preview.clone());
        self.link_preview_draft.update(cx, |preview, cx| preview.restore(link_preview, &text, cx));
        if text == self.state.input_text {
            return;
        }
        self.restored_draft_text = (!text.is_empty()).then(|| text.clone());
        self.draft_custom_emojis = draft
            .as_ref()
            .map(|draft| {
                draft
                    .custom_emojis
                    .iter()
                    .map(|item| ChatDraftCustomEmoji { placeholder: COMPOSER_CUSTOM_EMOJI_PLACEHOLDER.to_owned(), item: item.clone() })
                    .collect()
            })
            .unwrap_or_default();
        self.inline_user_mentions = draft.map(|draft| draft.mentions).unwrap_or_default();
        self.inline_user_mentions_anchor_text = text.clone();
        self.state.input_text = text;
        cx.notify();
    }

    pub(in super::super) fn insert_composer_emoji(&mut self, emoji: &str, _window: &mut Window, cx: &mut Context<Self>) {
        let caret = self.composer_caret();
        let edit = ChatComposerCaretEdit::replace(&self.state.input_text, caret, emoji, &mut self.draft_custom_emojis, None);
        self.apply_composer_caret_edit(edit, cx);
    }

    pub(in super::super) fn composer_field_reachable(&self, cx: &mut App) -> bool {
        if self.state.is_message_selection_active
            || self.is_message_request(cx)
            || self.chat_unavailable(cx)
            || self.chat_metadata_loading(cx)
        {
            return false;
        }
        let chat = self.resolved_chat(cx);
        !(self.viewer_is_outside(&chat, cx)
            || self.viewer_has_blocked_peer(&chat, cx)
            || self.peer_restricts_viewer(&chat, cx)
            || self.channel_posting_locked(cx)
            || self.topic_posting_locked(cx)
            || self.composer_text_locked(cx))
    }

    /// hands the keyboard to the field itself, so real key presses reach its text input handler
    pub(in super::super) fn focus_composer_field(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.is_on_screen
            || self.picker_visible
            || !self.pasted_media.is_empty()
            || self.capture_finger_down
            || self.show_replace_media_menu
            || self.show_bot_chat_menu
            || self.is_context_preview(cx)
            || ScreenFocus::text_field_is_focused(window, cx)
            || ScreenFocus::top_sheet(window, cx).is_some()
        {
            return false;
        }
        if self.state.is_message_search_active {
            self.message_search_focus_requests += 1;
        } else if self.composer_field_reachable(cx) {
            self.composer_focus_token = self.composer_focus_token.wrapping_add(1);
        } else {
            return false;
        }
        cx.notify();
        true
    }

    pub(in super::super) fn type_into_composer(&mut self, keystroke: &Keystroke, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.is_on_screen
            || !self.focus_handle.is_in_rendered_frame(window)
            || self.media_viewer_covers_feed
            || !self.pasted_media.is_empty()
            || self.capture_finger_down
            || self.show_replace_media_menu
            || self.show_bot_chat_menu
            || self.is_context_preview(cx)
            || ScreenFocus::text_field_is_focused(window, cx)
            || ScreenFocus::top_sheet(window, cx).is_some()
        {
            return false;
        }
        let searching = self.state.is_message_search_active;
        if !searching && !self.composer_field_reachable(cx) {
            return false;
        }
        let modifiers = keystroke.modifiers;
        let pastes = modifiers.secondary() && !modifiers.alt && keystroke.key == "v";
        if pastes && !searching && Self::handle_composer_paste(&cx.weak_entity(), window, cx) {
            self.composer_focus_token = self.composer_focus_token.wrapping_add(1);
            cx.notify();
            return true;
        }
        let typed = if pastes {
            cx.read_from_clipboard().and_then(|item| item.text())
        } else if modifiers.control || modifiers.platform || modifiers.function {
            None
        } else {
            keystroke
                .key_char
                .clone()
                .filter(|typed| !typed.chars().any(|ch| ch.is_control() || ('\u{F700}'..='\u{F8FF}').contains(&ch)))
        };
        let Some(typed) = typed.filter(|typed| !typed.is_empty()) else {
            return false;
        };
        if searching {
            let query = format!("{}{}", self.state.message_search_query, typed);
            self.state.message_search_query = query.clone();
            self.handle_message_search_query_change(&query, cx);
            self.message_search_focus_requests += 1;
        } else {
            self.insert_composer_emoji(&typed, window, cx);
            self.composer_focus_token = self.composer_focus_token.wrapping_add(1);
        }
        cx.notify();
        true
    }

    pub(in super::super) fn insert_composer_custom_emoji(&mut self, item: StickerItem, _window: &mut Window, cx: &mut Context<Self>) {
        let caret = self.composer_caret();
        let edit = ChatComposerCaretEdit::replace(
            &self.state.input_text,
            caret,
            COMPOSER_CUSTOM_EMOJI_PLACEHOLDER,
            &mut self.draft_custom_emojis,
            Some(ChatDraftCustomEmoji { placeholder: COMPOSER_CUSTOM_EMOJI_PLACEHOLDER.to_owned(), item }),
        );
        self.apply_composer_caret_edit(edit, cx);
    }

    pub(in super::super) fn composer_backspace(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let caret = self.composer_caret();
        let Some(edit) = ChatComposerCaretEdit::delete_backward(&self.state.input_text, caret, &mut self.draft_custom_emojis) else {
            return;
        };
        self.apply_composer_caret_edit(edit, cx);
    }

    /// last caret of the field, or nil when it was reported for other text
    /// then the panel writes at the end
    pub(in super::super) fn composer_caret(&self) -> Option<NSRange> {
        (self.composer_selection_text == self.state.input_text).then_some(self.composer_selection)
    }

    /// without focusing the field, focus would swap the emoji panel away
    pub(in super::super) fn apply_composer_caret_edit(&mut self, edit: ChatComposerCaretEditResult, cx: &mut Context<Self>) {
        self.composer_selection = edit.caret;
        self.composer_selection_text = edit.text.clone();
        self.state.input_text = edit.text;
        self.composer_requested_selection = Some(edit.caret);
        self.composer_selection_token = self.composer_selection_token.wrapping_add(1);
        cx.notify();
    }

    pub(in super::super) fn draft_custom_emoji_entities(&self, text: &str) -> Vec<CustomEmojiEntityWire> {
        self.composer_custom_emoji_runs(text)
            .into_iter()
            .map(|run| CustomEmojiEntityWire {
                offset: run.range.location as i64,
                length: run.range.length as i64,
                custom_emoji_id: run.item.id,
                file_url: RemoteMediaURL::optional_normalized(Some(run.item.file_url.as_str()))
                    .unwrap_or_else(|| run.item.file_url.clone()),
                asset_type: run.item.asset_type.wire().to_owned(),
                width: run.item.width,
                height: run.item.height,
                pack_id: run.item.pack_id,
            })
            .collect()
    }

    pub(in super::super) fn composer_custom_emoji_runs(&self, text: &str) -> Vec<ChatComposerCustomEmojiRun> {
        ChatComposerCaretEdit::custom_emoji_runs(text, &self.draft_custom_emojis)
    }

    pub(in super::super) fn composer_entities(&self, trimmed_text: &str, cx: &mut App) -> Option<String> {
        let chat_id = self.resolved_chat_id(cx);
        let role_entities = RoleMentionComposer::entities_json(trimmed_text, &chat_id, cx);
        let topic_id = self.inline_composer_topic_id(cx);
        let topic_entities =
            TopicMentionComposer::entities_json(trimmed_text, &chat_id, topic_id.as_deref(), role_entities, cx);
        let custom_emoji_entities =
            MessageEntitiesCodec::encode(&self.draft_custom_emoji_entities(trimmed_text), topic_entities.as_deref());
        let send_mentions = ChatComposerMentionEntityCodec::rebased_for_trimming(&self.inline_user_mentions, &self.state.input_text);
        ChatComposerMentionEntityCodec::encode(trimmed_text, &send_mentions, custom_emoji_entities.as_deref())
    }

    pub(in super::super) fn editable_custom_emoji_state(
        &self,
        message: &GetMessageMessage,
        cx: &mut App,
    ) -> (String, Vec<ChatDraftCustomEmoji>, Vec<ChatComposerCustomEmojiRun>) {
        let hydration = ChatComposerCustomEmojiEditHydrator::hydrate(&message.text, &message.resolved_entities().custom_emojis);
        let items: Vec<StickerItem> = hydration.entities.iter().map(|entity| Self::custom_emoji_item_for(entity, cx)).collect();
        let drafts = items
            .iter()
            .map(|item| ChatDraftCustomEmoji { placeholder: COMPOSER_CUSTOM_EMOJI_PLACEHOLDER.to_owned(), item: item.clone() })
            .collect();
        let runs = hydration
            .entities
            .iter()
            .zip(items)
            .map(|(entity, item)| ChatComposerCustomEmojiRun { range: NSRange::new(entity.offset.max(0) as usize, 1), item })
            .collect();
        (hydration.text, drafts, runs)
    }

    pub(in super::super) fn custom_emoji_item_for(entity: &MessageCustomEmojiEntity, cx: &mut App) -> StickerItem {
        let cached = StickersServicesStore::shared(cx).read(cx).custom_emoji_item(&entity.custom_emoji_id, cx);
        let raw_file_url = trimming_whitespaces_and_newlines(&entity.file_url);
        let file_url = RemoteMediaURL::optional_normalized(Some(raw_file_url)).unwrap_or_else(|| {
            if raw_file_url.is_empty() {
                cached.as_ref().map(|item| item.file_url.clone()).unwrap_or_default()
            } else {
                raw_file_url.to_owned()
            }
        });
        let raw_asset_type = trimming_whitespaces_and_newlines(&entity.asset_type);
        let asset_type_name = if raw_asset_type.is_empty() {
            cached.as_ref().map(|item| item.asset_type.wire().to_owned())
        } else {
            Some(raw_asset_type.to_owned())
        };
        let asset_type = <StickerAssetType as StickerAssetTypeInfer>::infer(asset_type_name.as_deref(), &file_url);
        StickerItem {
            id: entity.custom_emoji_id.clone(),
            pack_id: if entity.pack_id.is_empty() {
                cached.as_ref().map(|item| item.pack_id.clone()).unwrap_or_default()
            } else {
                entity.pack_id.clone()
            },
            file_url,
            thumbnail_url: cached.as_ref().and_then(|item| item.thumbnail_url.clone()),
            width: if entity.width > 0 { entity.width } else { cached.as_ref().map_or(0, |item| item.width) },
            height: if entity.height > 0 { entity.height } else { cached.as_ref().map_or(0, |item| item.height) },
            duration_ms: cached.as_ref().and_then(|item| item.duration_ms),
            position: cached.as_ref().map_or(0, |item| item.position),
            created_at_ms: cached.as_ref().map_or(0, |item| item.created_at_ms),
            associated_emojis: cached.as_ref().map(|item| item.associated_emojis.clone()).unwrap_or_default(),
            asset_type,
            mime_type: cached.as_ref().and_then(|item| item.mime_type.clone()),
            codec: cached.as_ref().and_then(|item| item.codec.clone()),
        }
    }

    pub(in super::super) fn set_composer_text(&mut self, text: String, selection: NSRange, cx: &mut Context<Self>) {
        self.composer_selection_text = text.clone();
        self.state.input_text = text;
        self.composer_requested_selection = Some(selection);
        self.composer_selection = selection;
        self.composer_selection_token = self.composer_selection_token.wrapping_add(1);
        self.composer_focus_token = self.composer_focus_token.wrapping_add(1);
        cx.notify();
    }

    pub(in super::super) fn refresh_composer_overflow(&mut self, text: &str, _window: &mut Window, cx: &mut Context<Self>) {
        let trimmed = trimming_whitespaces_and_newlines(text);
        let entities = if self.draft_custom_emojis.is_empty() && self.inline_user_mentions.is_empty() {
            None
        } else {
            self.composer_entities(trimmed, cx)
        };
        let overflows =
            ChatComposerTextSplitter::exceeds_one_message(trimmed, entities.as_deref(), &ChatComposerTextSplitterBudget::message());
        if overflows != self.composer_overflows_message {
            self.composer_overflows_message = overflows;
            cx.notify();
        }
    }

    pub(in super::super) fn open_send_text_file(&mut self, cx: &mut Context<Self>) {
        self.send_text_file_name = DEFAULT_TEXT_FILE_NAME.to_owned();
        self.show_send_text_file = true;
        cx.notify();
    }

    pub(in super::super) fn text_file_name(typed: &str) -> String {
        let mut name = trimming_whitespaces_and_newlines(typed).replace('/', "_").replace(':', "_");
        if name.is_empty() || name == "." || name == ".." {
            name = DEFAULT_TEXT_FILE_NAME.to_owned();
        }
        let is_txt = std::path::Path::new(&name).extension().is_some_and(|extension| extension.to_string_lossy().to_lowercase() == "txt");
        if !is_txt {
            name.push_str(".txt");
        }
        name
    }

    pub(in super::super) fn send_composer_text_as_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chat_id = self.resolved_chat_id(cx);
        if chat_id.is_empty() || self.state.edit_target.is_some() {
            return;
        }
        let trimmed = trimming_whitespaces_and_newlines(&self.state.input_text).to_owned();
        if trimmed.is_empty() {
            return;
        }
        if chat_interactions(cx, |store, cx| store.slow_mode_blocks_send(&chat_id, cx)) {
            return;
        }
        let mut body: Vec<u16> = trimmed.encode_utf16().collect();
        for run in self.composer_custom_emoji_runs(&trimmed).iter().rev() {
            let replacement: Vec<u16> =
                run.item.associated_emojis.first().map(String::as_str).unwrap_or_default().encode_utf16().collect();
            let end = run.range.location + run.range.length;
            if end <= body.len() {
                body.splice(run.range.location..end, replacement);
            }
        }
        let body = String::from_utf16_lossy(&body);
        let file_name = Self::text_file_name(&self.send_text_file_name);
        let Some(probe_url) = Url::from_file_path(&std::env::temp_dir().join(&file_name)) else {
            return;
        };
        let probe = MediaPickerItem {
            id: String::new(),
            kind: MediaPickerItemKind::File,
            url: probe_url,
            suggested_file_name: file_name.clone(),
            asset_identifier: None,
            image_payload: None,
            video_payload: None,
            audio_payload: None,
            is_file: true,
            has_spoiler: false,
            view_mode: None,
        };
        let post_topic_id = self.post_topic_id(cx);
        let switched_off = message_interactions(cx, |store, cx| {
            store.media_send_switched_off(&chat_id, &[probe], false, post_topic_id.as_deref(), cx)
        });
        if switched_off {
            return;
        }
        let reply_id = self.take_reply_target();
        self.link_preview_draft.update(cx, |draft, cx| draft.reset(cx));
        self.state.input_text = String::new();
        self.composer_input_height = ChatBottomBarMetrics::CIRCLE;
        self.saved_draft_custom_emojis = None;
        self.draft_custom_emojis = Vec::new();
        self.inline_user_mentions = Vec::new();
        self.inline_invocation_exclusion_tag = None;
        self.dismiss_inline_composer(false, window, cx);
        message_interactions(cx, |store, cx| store.set_typing(&chat_id, false, ChatAction::Typing, cx));
        let topic_id = self.resolved_topic_id(cx);
        let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
        message_interactions(cx, |store, cx| {
            store.send_text_as_file(
                &body,
                &file_name,
                &chat_id,
                topic_id.as_deref(),
                post_topic_id.as_deref(),
                thread_root_id.as_deref(),
                reply_id.as_deref(),
                Some(Self::media_send_completion()),
                cx,
            );
        });
        self.follow_outgoing_message(window, cx);
        GuideSignalBus::emit(RiseGuideSignal::MessageSent);
        cx.notify();
    }
}
