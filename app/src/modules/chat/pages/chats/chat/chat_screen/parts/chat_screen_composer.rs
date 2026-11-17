use std::rc::Rc;
use gpui::prelude::*;
use gpui::{AnyElement, App, AppContext, Context, Pixels, SharedString, WeakEntity, Window, div, px};
use rise_canon::RiseGuideSignal;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_uuid::Uuid;
use rise_i18n::tr;
use rise_platform::haptics::selection_changed;
use rise_theme::AppTheme;
use rise_theme::theme::theme_store::ThemeStore;
use rise_ui::input_ui::input_actions::MoveUp;
use rise_ui::keyboard_dismiss::KeyboardDismissModifiers;
use rise_ui::liquid_glass_button::{LiquidGlassButton, LiquidGlassButtonShape};
use rise_ui::native_confirm_alert::{NativeConfirmAlert, NativeConfirmAlertAction};
use rise_ui::swiftui::binding::Binding;
use rise_ui::swiftui::measure::measure;
use rise_ui::swiftui::motion::spring;
use rise_ui::swiftui::prelude::*;
use rise_ui::swiftui::tooltip_label::TooltipLabelModifiers;
use rise_ui::swiftui::transition::Transition;
use rise_ui::{IconUi, MainText};
use crate::app::router::navigation_types::RootRoute;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::components::chat::bot_chat_menu_sheet::BotChatMenuPresentationContract;
use rise_bot_api::ChatAction;
use crate::modules::chat::components::chat::composer::chat_paste_media_sheet::{ChatPasteMediaLoader, ChatPasteMediaProvider};
use crate::modules::chat::components::chat::composer::chat_composer_input_surface::ChatComposerInputSurface;
use crate::modules::chat::components::chat::composer::chat_composer_paste::ChatComposerPaste;
use crate::modules::chat::components::chat::composer::chat_composer_types::{ChatBottomBarMetrics, ChatComposerEditTarget, ChatComposerReplyTarget, ChatDraftCustomEmoji, resign_chat_keyboard};
use crate::modules::chat::components::chat::message_list::MessageList;
use crate::modules::chat::components::chat::voice::chat_composer_capture_mode::ChatComposerCaptureMode;
use crate::modules::chat::components::chat::chat_title_row::ChatTitleRow;
use crate::modules::chat::shared::message_permissions::MessagePermissions;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatPermKey, ChatType};
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_actions::message_types::GetMessageMessage;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::media_picker::api::media_picker_configuration::MediaPickerConfiguration;
use crate::modules::media_picker::api::media_picker_result::MediaPickerItem;
use rise_errors::ErrorCode;
use rise_platform::haptics::{ImpactStyle, impact};
use rise_platform::host_os::HostOs;
use rise_platform::pasteboard::UIPasteboard;
use rise_ui::rich_text::rich_text_markup::RichEditorStyle;
use rise_ui::swiftui::button_style::plain;
use rise_ui::swiftui::content_mode::ContentMode;
use rise_ui::text_field_alert::TextFieldAlert;
use crate::core::ui::clever_image_ui::CleverImageUi;
use crate::core::ui::user_logo_ui::UserLogoUi;
use crate::modules::chat::components::chat::chat_composer_context_bar::{ChatComposerContextBar, ChatComposerContextBarKind};
use crate::modules::chat::components::chat::composer::chat_composer_text_view::ChatComposerTextView;
use crate::modules::chat::components::chat::composer::chat_composer_types::ChatComposerInputChromeModifiers;
use crate::modules::chat::components::chat::composer::chat_link_preview_panel::ChatLinkPreviewBar;
use crate::modules::chat::components::chat::composer::chat_slow_mode_countdown_button::ChatSlowModeCountdownButton;
use crate::modules::chat::components::chat::scroll::chat_messages_scroll_controller::ChatComposerInsetAnimation;
use crate::modules::chat::components::chat::voice::chat_composer_send_mic_button::ChatComposerSendMicButton;
use crate::modules::chat::components::chat::voice::chat_voice_recording_ui::{ChatVoiceDraftTrack, ChatVoiceHoldTrack, ChatVoiceLockedTrack, ChatVoicePauseButton, ChatVoiceSendButton};
use crate::modules::chat::shared::config::message_sizes::MessageSizes;
use crate::modules::chat::shared::utils::message_reply_preview::MessageReplyPreview;
use crate::modules::media_picker::editor::media_reedit_launcher::{MediaReeditLauncher, MediaReeditResult};
use crate::modules::media_picker::editor::media_reedit_store::MediaReeditStore;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::canon_field_rule::RiseFieldRule;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::sai::Sai;
use crate::modules::chat::stores::message::message_actions::message_types::{MessageMediaItem, MessageMediaKind, RemoteMediaURL};
use crate::modules::stickers::stores::stickers::stickers_actions::stickers_types::{GifItem, PackKind, StickerItem};
use gpui::AsyncApp;
use rise_canon::RiseGuideAnchor;
use rise_engine::CancellationError;
use rise_ui::guide_anchor_ui::guide_anchor_modifier::GuideAnchorModifierModifiers;
use rise_ui::rich_text::document::rise_document::RiseDocument;
use rise_ui::rich_text::document::rise_document_codec::RiseDocumentCodec;
use rise_ui::rich_text::document::rise_document_projection::RiseDocumentProjection;
use crate::core::media_viewer::media_viewer_image_cache::{MediaViewerBitmapLoader, MediaViewerImageMemoryCache};
use crate::core::ui::clever_image_ui::CleverImageCachePolicy;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::overlay::premium_upsell_store::PremiumUpsellStore;
use crate::modules::chat::components::chat::scroll::chat_messages_scroll_controller::{ChatComposerBottomPaddingPolicy, ChatPickerPanelHeightPolicy};
use crate::modules::chat::components::chat::voice::chat_composer_capture_mode::ChatComposerRecordingUIPhase;
use crate::modules::media_picker::api::media_picker_result::{MediaPickerItemKind, MediaPickerResult};
use crate::modules::media_picker::editor::editor_image_bake::EditorImageBake;
use super::super::chat_screen::*;
use super::super::chat_screen_support::*;
use rise_ui::liquid_chrome::LiquidChrome;

impl ChatScreen {
    /// how much of the composer row the leading circle buttons take
    /// suggestion panel and input draw one plate so the panel starts where the input starts
    pub(in super::super) fn composer_leading_controls_width(&self, cx: &mut App) -> f32 {
        if self.is_any_capture_active(cx) {
            return 0.0;
        }
        let mut width = ChatBottomBarMetrics::CIRCLE + ChatBottomBarMetrics::ROW_GAP;
        if self.resolved_chat(cx).is_active_bot_chat() {
            width += BotChatMenuPresentationContract::BUTTON_SIZE + ChatBottomBarMetrics::ROW_GAP;
        }
        width
    }

    /// text switched off for this member while media may still go
    /// the field locks and the typed draft waits unsent, an edit isnt a new send
    pub(in super::super) fn composer_text_locked(&self, cx: &mut App) -> bool {
        self.state.edit_target.is_none() && self.member_text_switched_off(cx)
    }

    pub(in super::super) fn member_text_switched_off(&self, cx: &mut App) -> bool {
        let chat_id = self.resolved_chat_id(cx);
        let post_topic_id = self.post_topic_id(cx);
        chat_services(cx, |store, cx| {
            store.member_send_switched_off_forum_topic_id(&chat_id, ChatPermKey::TEXT, post_topic_id.as_deref(), cx)
        })
    }

    /// chat name rides along so the picker editor can show where the media goes
    /// resolved only while the picker is open so a closed picker adds no store reads
    pub(in super::super) fn media_picker_configuration(&self, cx: &mut App) -> MediaPickerConfiguration {
        let mode = self.state.media_picker_mode;
        let mut configuration = if mode == ChatMediaPickerMode::VideoNote {
            MediaPickerConfiguration::chat_video_note()
        } else {
            MediaPickerConfiguration::chat_attachments()
        };
        if mode != ChatMediaPickerMode::None {
            let chat = self.resolved_chat(cx);
            configuration.destination_title = Some(ChatTitleRow::plain_title(&chat, cx));
            configuration.allows_poll = mode != ChatMediaPickerMode::VideoNote && self.accepts_polls(cx);
            configuration.allows_view_once =
                mode != ChatMediaPickerMode::VideoNote && chat.r#type == ChatType::Direct && !chat.is_bot_chat();
        }
        configuration
    }

    pub(in super::super) fn current_visible_messages(&self, cx: &mut App) -> Vec<GetMessageMessage> {
        let scope = self.message_scope(cx);
        let snapshot = message_services(cx, |store, cx| store.message_scope_presentation_snapshot(&scope, cx));
        self.visible_messages(snapshot.as_ref(), cx)
    }

    pub(in super::super) fn send_document(
        &mut self,
        document: &RiseDocument,
        items: Vec<MediaPickerItem>,
        entities: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let projection = RiseDocumentProjection::project(document).trimmed_for_wire();
        let Some(encoded) = RiseDocumentCodec::encode_to_string(document) else {
            return;
        };
        if projection.content.is_empty() && items.is_empty() {
            return;
        }
        let reply_id = self.take_reply_target();
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.resolved_topic_id(cx);
        let post_topic_id = self.post_topic_id(cx);
        let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
        if items.is_empty() {
            message_interactions(cx, |store, cx| {
                let _ = store.send_text(
                    &chat_id,
                    &projection.content,
                    topic_id.as_deref(),
                    post_topic_id.as_deref(),
                    thread_root_id.as_deref(),
                    reply_id.as_deref(),
                    None,
                    None,
                    entities,
                    Some(encoded.as_str()),
                    None,
                    None,
                    cx,
                );
            });
        } else {
            message_interactions(cx, |store, cx| {
                store.send_media(
                    &chat_id,
                    items,
                    &projection.content,
                    topic_id.as_deref(),
                    post_topic_id.as_deref(),
                    thread_root_id.as_deref(),
                    reply_id.as_deref(),
                    entities,
                    Some(encoded.as_str()),
                    None,
                    Some(Self::media_send_completion()),
                    cx,
                );
            });
        }
        self.follow_outgoing_message(window, cx);
    }

    pub(in super::super) fn handle_media_picker_result(
        &mut self,
        result: MediaPickerResult,
        mode: ChatMediaPickerMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let chat_id = self.resolved_chat_id(cx);
        let topic_id = self.resolved_topic_id(cx);
        let post_topic_id = self.post_topic_id(cx);
        let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
        if let Some(poll) = result.poll.as_ref() {
            let reply_id = self.take_reply_target();
            message_interactions(cx, |store, cx| {
                let _ = store.send_poll(
                    &chat_id,
                    poll,
                    topic_id.as_deref(),
                    post_topic_id.as_deref(),
                    thread_root_id.as_deref(),
                    reply_id.as_deref(),
                    cx,
                );
            });
            self.follow_outgoing_message(window, cx);
            return;
        }
        if let Some(location) = result.location.as_ref() {
            let reply_id = self.take_reply_target();
            message_interactions(cx, |store, cx| {
                let _ = store.send_location(
                    &chat_id,
                    location,
                    topic_id.as_deref(),
                    post_topic_id.as_deref(),
                    thread_root_id.as_deref(),
                    reply_id.as_deref(),
                    cx,
                );
            });
            self.follow_outgoing_message(window, cx);
            return;
        }
        if let Some(document) = result.document.as_ref() {
            self.send_document(document, result.items.clone(), result.document_entities.as_deref(), window, cx);
            return;
        }
        if self.picker_replaces_edit_media {
            self.picker_replaces_edit_media = false;
            cx.notify();
            if result.was_cancelled || self.state.edit_target.is_none() {
                return;
            }
            let Some(item) = result.items.iter().find(|item| matches!(item.kind, MediaPickerItemKind::Image)) else {
                return;
            };
            self.begin_pending_reedit(item.clone(), cx);
            return;
        }
        if result.was_cancelled || result.items.is_empty() || chat_id.is_empty() {
            return;
        }
        if mode == ChatMediaPickerMode::VideoNote
            && !result.items.first().is_some_and(|item| matches!(item.kind, MediaPickerItemKind::Video))
        {
            return;
        }
        let as_video_note = mode == ChatMediaPickerMode::VideoNote;
        let switched_off = message_interactions(cx, |store, cx| {
            store.media_send_switched_off(&chat_id, &result.items, as_video_note, post_topic_id.as_deref(), cx)
        });
        if switched_off {
            return;
        }
        let caption = trimming_whitespaces_and_newlines(&self.state.input_text).to_owned();
        let entities = self.composer_entities(&caption, cx);
        let link_preview = self.link_preview_draft.update(cx, |draft, cx| {
            let selection = draft.selection(&caption, cx);
            draft.reset(cx);
            selection
        });
        self.state.input_text = String::new();
        self.draft_custom_emojis = Vec::new();
        let reply_id = self.take_reply_target();
        cx.notify();
        if as_video_note {
            let Some(item) = result.items.first() else {
                return;
            };
            message_interactions(cx, |store, cx| {
                let _ = store.send_video_note(
                    &chat_id,
                    item,
                    None,
                    &caption,
                    topic_id.as_deref(),
                    post_topic_id.as_deref(),
                    thread_root_id.as_deref(),
                    false,
                    reply_id.as_deref(),
                    entities.as_deref(),
                    None,
                    cx,
                );
            });
        } else if let [audio_item] = result.items.as_slice()
            && audio_item.audio_payload.is_some()
        {
            message_interactions(cx, |store, cx| {
                store.send_audio(
                    &chat_id,
                    audio_item,
                    &caption,
                    topic_id.as_deref(),
                    post_topic_id.as_deref(),
                    thread_root_id.as_deref(),
                    reply_id.as_deref(),
                    entities.as_deref(),
                    link_preview,
                    Some(Self::media_send_completion()),
                    cx,
                );
            });
        } else {
            message_interactions(cx, |store, cx| {
                store.send_media(
                    &chat_id,
                    result.items,
                    &caption,
                    topic_id.as_deref(),
                    post_topic_id.as_deref(),
                    thread_root_id.as_deref(),
                    reply_id.as_deref(),
                    entities.as_deref(),
                    None,
                    link_preview,
                    Some(Self::media_send_completion()),
                    cx,
                );
            });
        }
        self.follow_outgoing_message(window, cx);
    }

    /// replacement photo of a message being edited
    /// thumbnail shows at once from memory then disk, picker edits are drawn in off the main thread
    pub(in super::super) fn begin_pending_reedit(&mut self, item: MediaPickerItem, cx: &mut Context<Self>) {
        let source = item.url.absolute_string().to_string();
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let immediate = MediaViewerImageMemoryCache::memory_only(&source, account_user_id.as_deref());
        self.pending_reedit_thumb_image = immediate.clone();
        let item_id = item.id.clone();
        let deferred_edit_state = item
            .image_payload
            .as_ref()
            .filter(|payload| payload.is_processing_deferred)
            .and_then(|payload| payload.deferred_edit_state.clone());
        self.pending_reedit_item = Some(item);
        cx.spawn(async move |this, cx| {
            let is_current = |this: &WeakEntity<Self>, cx: &mut AsyncApp| {
                this.read_with(&*cx, |screen, _| screen.pending_reedit_item.as_ref().is_some_and(|item| item.id == item_id))
                    .unwrap_or(false)
            };
            if !is_current(&this, cx) {
                return;
            }
            let base_image = match immediate {
                Some(immediate) => Some(immediate),
                None => {
                    MediaViewerBitmapLoader::load_into_memory(&source, false, true, CleverImageCachePolicy::default(), &*cx).await
                }
            };
            let Some(base_image) = base_image else {
                return;
            };
            if !is_current(&this, cx) {
                return;
            }
            let preview = match deferred_edit_state.filter(|state| state.has_meaningful_edits()) {
                Some(state) => {
                    let input = ChatPendingReeditRenderInput { image: base_image, edit_state: state };
                    cx.background_spawn(async move { EditorImageBake::render_edited_image(&input.image, &input.edit_state) }).await
                }
                None => base_image,
            };
            if !is_current(&this, cx) {
                return;
            }
            let _ = this.update(cx, |screen, cx| {
                screen.pending_reedit_thumb_image = Some(preview);
                cx.notify();
            });
        })
        .detach();
    }

    /// the reply the composer holds goes with exactly one outgoing message whatever it is
    /// then the plate closes
    pub(in super::super) fn take_reply_target(&mut self) -> Option<String> {
        self.state.reply_target.take().map(|target| target.message_id)
    }

    pub(in super::super) fn handle_media_send_completion(result: anyhow::Result<()>, cx: &mut App) {
        let Err(error) = result else {
            return;
        };
        if error.is::<CancellationError>() {
            return;
        }
        if PremiumUpsellStore::shared(cx).update(cx, |store, cx| store.offer_covers(&error, cx)) {
            tracing::warn!(target: "riseonly::X1801", %error, "media send failure covered by premium offer");
            return;
        }
        let content = MessageInteractionsStore::shared(cx).read(cx).media_send_failure_text(&error);
        tracing::warn!(target: "riseonly::X1428", %error, ?content, "media send failure toast");
        Notifier::send(
            tr("chat_media_send_failed"),
            NotifierSendOptions {
                content,
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    pub(in super::super) fn composer_bottom_padding(&self, safe_bottom_inset: f32, cx: &App) -> f32 {
        ChatComposerBottomPaddingPolicy::resolve(
            safe_bottom_inset,
            0.0,
            self.picker_visible,
            self.picker_panel_height(cx),
            false,
            ChatBottomBarMetrics::TOP_PADDING,
        )
    }

    /// mirrors the bottom bar branch chain in the body
    /// a state that replaces the composer and is missing here leaves the last rows unreachable
    pub(in super::super) fn composer_context_height(&self, cx: &App) -> f32 {
        let reply_height =
            if self.composer_context_bar_visible(cx) { ChatBottomBarMetrics::REPLY_EDIT_BAR_HEIGHT } else { 0.0 };
        let preview_height =
            if self.composer_link_preview_bar_visible(cx) { ChatBottomBarMetrics::REPLY_EDIT_BAR_HEIGHT } else { 0.0 };
        reply_height + preview_height
    }

    pub(in super::super) fn composer_link_preview_bar_visible(&self, cx: &App) -> bool {
        self.link_preview_draft.read(cx).is_visible()
            && !self.state.edit_target.as_ref().is_some_and(|edit| edit.is_media_caption)
    }

    pub(in super::super) fn composer_context_bar_visible(&self, cx: &App) -> bool {
        if self.state.edit_target.is_some() {
            return !self.composer_link_preview_bar_visible(cx);
        }
        self.state.reply_target.is_some()
    }

    pub(in super::super) fn composer_row_height(&self, peer_blocked: bool, cx: &mut App) -> f32 {
        if self.state.is_message_search_active || self.state.is_message_selection_active {
            return ChatBottomBarMetrics::CIRCLE;
        }
        if self.is_message_request(cx) {
            return 130.0;
        }
        if self.chat_unavailable(cx) || self.chat_metadata_loading(cx) {
            return 0.0;
        }
        if self.is_community_outsider(cx) {
            let hint = if self.shows_thread_members_only_hint(cx) {
                ChatComposerPlateMetrics::HINT_HEIGHT + ChatComposerPlateMetrics::HINT_SPACING
            } else {
                0.0
            };
            return ChatBottomBarMetrics::CIRCLE + hint;
        }
        if peer_blocked || self.channel_posting_locked(cx) {
            return ChatComposerPlateMetrics::row_height();
        }
        if self.is_any_capture_active(cx) {
            return ChatBottomBarMetrics::CIRCLE;
        }
        let input_height = if self.composer_text_locked(cx) { ChatBottomBarMetrics::CIRCLE } else { self.composer_input_height };
        input_height + self.composer_context_height(cx)
    }

    pub(in super::super) fn composer_total_height(&self, safe_bottom_inset: f32, reply_keyboard_panel_height: f32, peer_blocked: bool, cx: &mut App) -> f32 {
        if self.uses_context_preview_layout(cx) {
            return 8.0;
        }
        self.composer_row_height(peer_blocked, cx)
            + ChatBottomBarMetrics::TOP_PADDING
            + self.composer_bottom_padding(safe_bottom_inset, cx)
            + reply_keyboard_panel_height
    }

    pub(in super::super) fn picker_panel_height(&self, _cx: &App) -> f32 {
        ChatPickerPanelHeightPolicy::resolve(0.0, None)
    }

    pub(in super::super) fn toggle_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer_input_surface.toggled() == ChatComposerInputSurface::SystemKeyboard {
            let _ = selection_changed();
            self.close_picker(cx);
            self.composer_focus_token = self.composer_focus_token.wrapping_add(1);
            return;
        }
        self.open_picker(window, cx);
    }

    pub(in super::super) fn open_picker(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let _ = selection_changed();
        ChatComposerInsetAnimation::set_last_duration(ChatBottomBarMetrics::COMPOSER_ANIM_DURATION);
        self.composer_input_surface = ChatComposerInputSurface::Picker;
        self.picker_visible = true;
        self.composer_focus_token = self.composer_focus_token.wrapping_add(1);
        cx.notify();
    }

    pub(in super::super) fn close_picker(&mut self, cx: &mut Context<Self>) {
        if !self.picker_visible && self.composer_input_surface != ChatComposerInputSurface::Picker {
            return;
        }
        ChatComposerInsetAnimation::set_last_duration(ChatBottomBarMetrics::COMPOSER_ANIM_DURATION);
        self.composer_input_surface = ChatComposerInputSurface::SystemKeyboard;
        self.picker_visible = false;
        cx.notify();
    }

    pub(in super::super) fn reset_composer_input_surface(&mut self, cx: &mut Context<Self>) {
        self.composer_input_surface = ChatComposerInputSurface::SystemKeyboard;
        self.picker_visible = false;
        self.show_bot_chat_menu = false;
        self.bot_menu_starts_expanded = false;
        cx.notify();
    }

    pub(in super::super) fn handle_picker_send_sticker(&mut self, sticker: StickerItem, window: &mut Window, cx: &mut Context<Self>) {
        let chat_id = self.resolved_chat_id(cx);
        if chat_id.is_empty() {
            return;
        }
        let post_topic_id = self.post_topic_id(cx);
        let switched_off = chat_interactions(cx, |store, cx| {
            store.send_switched_off_forum_topic_id(&chat_id, ChatPermKey::STICKERS, post_topic_id.as_deref(), cx)
        });
        if switched_off {
            return;
        }
        let reply_id = self.take_reply_target();
        let topic_id = self.resolved_topic_id(cx);
        let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
        message_interactions(cx, |store, cx| {
            let _ = store.send_sticker(
                &chat_id,
                &sticker,
                "",
                topic_id.as_deref(),
                post_topic_id.as_deref(),
                thread_root_id.as_deref(),
                reply_id.as_deref(),
                cx,
            );
        });
        self.follow_outgoing_message(window, cx);
        cx.notify();
    }

    pub(in super::super) fn handle_picker_send_gif(&mut self, gif: GifItem, window: &mut Window, cx: &mut Context<Self>) {
        let chat_id = self.resolved_chat_id(cx);
        if chat_id.is_empty() {
            return;
        }
        let post_topic_id = self.post_topic_id(cx);
        let switched_off = chat_interactions(cx, |store, cx| {
            store.send_switched_off_forum_topic_id(&chat_id, ChatPermKey::GIF, post_topic_id.as_deref(), cx)
        });
        if switched_off {
            return;
        }
        let reply_id = self.take_reply_target();
        let topic_id = self.resolved_topic_id(cx);
        let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);
        message_interactions(cx, |store, cx| {
            let _ = store.send_gif(
                &chat_id,
                &gif,
                "",
                topic_id.as_deref(),
                post_topic_id.as_deref(),
                thread_root_id.as_deref(),
                reply_id.as_deref(),
                cx,
            );
        });
        self.follow_outgoing_message(window, cx);
        cx.notify();
    }

    pub(in super::super) fn handle_open_pack_settings(&mut self, kind: PackKind, window: &mut Window, cx: &mut Context<Self>) {
        self.close_picker(cx);
        resign_chat_keyboard(window, cx);
        self.push_route(RootRoute::StickerSettings { kind }, cx);
    }

    pub(in super::super) fn handle_send_button(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chat_id = self.resolved_chat_id(cx);
        let trimmed = trimming_whitespaces_and_newlines(&self.state.input_text).to_owned();
        let has_pending_media = self.pending_reedit_item.is_some();
        if trimmed.is_empty() && !(self.state.edit_target.is_some() && has_pending_media) {
            return;
        }
        let topic_id = self.resolved_topic_id(cx);
        let thread_root_id = self.resolved_thread_root_id().map(str::to_owned);

        if let Some(edit) = self.state.edit_target.clone() {
            if (edit.is_media_caption || has_pending_media)
                && message_interactions(cx, |store, cx| store.refuses_caption_edit(&trimmed, cx))
            {
                return;
            }
            let media = self.pending_reedit_item.take();
            let current_custom_emoji_runs = self.composer_custom_emoji_runs(&trimmed);
            let edit_entities = self.composer_entities(&trimmed, cx).unwrap_or_else(|| "[]".to_owned());
            let edited_custom_emojis = self.draft_custom_emojis.clone();
            let edit_draft_scope_key = self.composer_draft_scope_key(cx);
            let edit_link_preview_type = if edit.is_media_caption || media.is_some() {
                None
            } else {
                self.link_preview_draft.update(cx, |draft, cx| draft.edit_wire_type(&trimmed, cx))
            };
            self.link_preview_draft.update(cx, |draft, cx| draft.reset(cx));
            self.state.input_text = String::new();
            self.composer_input_height = ChatBottomBarMetrics::CIRCLE;
            self.state.edit_target = None;
            self.state.saved_draft = None;
            self.saved_draft_custom_emojis = None;
            draft_store(cx, |store, cx| store.clear(&edit_draft_scope_key, cx));
            self.pending_reedit_thumb_image = None;
            self.reedit_in_progress = false;
            self.draft_custom_emojis = Vec::new();
            self.inline_user_mentions = Vec::new();
            self.inline_invocation_exclusion_tag = None;
            self.dismiss_inline_composer(false, window, cx);
            message_interactions(cx, |store, cx| store.set_typing(&chat_id, false, ChatAction::Typing, cx));
            if chat_id.is_empty() {
                return;
            }
            if let Some(media) = media {
                message_interactions(cx, |store, cx| {
                    store.edit_message_media(
                        &edit.message_id,
                        media,
                        &trimmed,
                        Some(edit_entities.as_str()),
                        &chat_id,
                        topic_id.as_deref(),
                        thread_root_id.as_deref(),
                        cx,
                    );
                });
            } else if trimmed != edit.original_text
                || current_custom_emoji_runs != edit.original_custom_emoji_runs
                || edit_link_preview_type.is_some()
            {
                let this = cx.weak_entity();
                let failed_edit = edit.clone();
                let failed_text = trimmed.clone();
                let on_failure: Box<dyn FnOnce(&mut App)> = Box::new(move |cx: &mut App| {
                    Self::deliver(&this, cx, move |screen, cx| {
                        screen.restore_failed_edit(failed_edit, failed_text, edited_custom_emojis, edit_draft_scope_key, cx);
                    });
                });
                message_interactions(cx, |store, cx| {
                    store.edit_message(
                        &edit.message_id,
                        &trimmed,
                        Some(edit_entities.as_str()),
                        edit_link_preview_type.as_deref(),
                        &chat_id,
                        topic_id.as_deref(),
                        thread_root_id.as_deref(),
                        Some(on_failure),
                        cx,
                    );
                });
            }
            return;
        }

        let post_topic_id = self.post_topic_id(cx);
        if chat_interactions(cx, |store, cx| store.slow_mode_blocks_send(&chat_id, cx)) {
            return;
        }
        if chat_interactions(cx, |store, cx| {
            store.send_switched_off_forum_topic_id(&chat_id, ChatPermKey::TEXT, post_topic_id.as_deref(), cx)
        }) {
            return;
        }

        let entities = self.composer_entities(&trimmed, cx);
        let Some(parts) = message_interactions(cx, |store, cx| store.text_parts(&chat_id, &trimmed, entities.as_deref(), cx)) else {
            return;
        };
        let reply_id = self.state.reply_target.as_ref().map(|target| target.message_id.clone());
        let link_preview = self.link_preview_draft.update(cx, |draft, cx| {
            let selection = draft.selection(&trimmed, cx);
            draft.reset(cx);
            selection
        });
        ChatComposerInsetAnimation::announce_send();
        self.state.input_text = String::new();
        self.composer_input_height = ChatBottomBarMetrics::CIRCLE;
        self.state.reply_target = None;
        self.saved_draft_custom_emojis = None;
        self.draft_custom_emojis = Vec::new();
        self.inline_user_mentions = Vec::new();
        self.inline_invocation_exclusion_tag = None;
        self.dismiss_inline_composer(false, window, cx);
        message_interactions(cx, |store, cx| store.set_typing(&chat_id, false, ChatAction::Typing, cx));
        if chat_id.is_empty() {
            return;
        }
        message_interactions(cx, |store, cx| {
            store.send_text_parts(
                &parts,
                &chat_id,
                topic_id.as_deref(),
                post_topic_id.as_deref(),
                thread_root_id.as_deref(),
                reply_id.as_deref(),
                link_preview,
                cx,
            );
        });
        self.follow_outgoing_message(window, cx);
        GuideSignalBus::emit(RiseGuideSignal::MessageSent);
        cx.notify();
    }

    pub(in super::super) fn text_file_name_rule() -> RiseFieldRule {
        RiseFieldRule::client_rule("message_media_file_name")
    }

    pub(in super::super) fn begin_reply(&mut self, message: &GetMessageMessage, perms: MessagePermissions<'_>, _window: &mut Window, cx: &mut Context<Self>) {
        let copy_text = trimming_whitespaces_and_newlines(perms.copy_text());
        let thumbnail_source = MessageReplyPreview::thumbnail_source(&message.media_items);
        let snippet = if copy_text.is_empty() {
            MessageReplyPreview::label(&message.content_type, "", thumbnail_source.is_some())
        } else {
            copy_text.chars().take(120).collect()
        };
        ChatComposerInsetAnimation::set_last_duration(ChatBottomBarMetrics::COMPOSER_ANIM_DURATION);
        self.state.edit_target = None;
        self.state.saved_draft = None;
        self.state.reply_target = Some(ChatComposerReplyTarget {
            message_id: message.id.clone(),
            author_name: message.sender_name.clone(),
            snippet,
            media_thumbnail_source: thumbnail_source,
        });
        cx.notify();
    }

    pub(in super::super) fn begin_edit(&mut self, message: &GetMessageMessage, cx: &mut Context<Self>) {
        let single_image = match message.media_items.as_slice() {
            [item] if matches!(item.kind, MessageMediaKind::Image) => Some(item),
            _ => None,
        };
        let (original_text, drafts, runs) = self.editable_custom_emoji_state(message, cx);
        self.pending_reedit_item = None;
        self.pending_reedit_thumb_image = None;
        self.reedit_in_progress = false;
        self.picker_replaces_edit_media = false;
        ChatComposerInsetAnimation::set_last_duration(ChatBottomBarMetrics::COMPOSER_ANIM_DURATION);
        if self.state.edit_target.is_none() {
            self.state.saved_draft = Some(self.state.input_text.clone());
            self.saved_draft_custom_emojis = Some(self.draft_custom_emojis.clone());
        }
        self.state.reply_target = None;
        self.state.edit_target = Some(ChatComposerEditTarget {
            message_id: message.id.clone(),
            original_text: original_text.clone(),
            preview_text: message.text.clone(),
            original_custom_emoji_runs: runs,
            media_thumbnail_source: single_image.map(MessageMediaItem::display_source),
            media_full_source: single_image
                .map(|item| RemoteMediaURL::optional_normalized(Some(item.url.as_str())).unwrap_or_else(|| item.display_source())),
            can_reedit_media: single_image.is_some(),
            is_media_caption: !message.media_items.is_empty(),
        });
        if message.media_items.is_empty() {
            self.link_preview_draft
                .update(cx, |draft, cx| draft.seed_for_edit(&original_text, message.link_preview.as_ref(), cx));
        } else {
            self.link_preview_draft.update(cx, |draft, cx| draft.reset(cx));
        }
        self.state.input_text = original_text;
        self.draft_custom_emojis = drafts;
        self.close_picker(cx);
        self.composer_focus_token = self.composer_focus_token.wrapping_add(1);
        cx.notify();
    }

    pub(in super::super) fn media_reedit_finish(this: WeakEntity<Self>, message_id: String) -> impl FnOnce(Option<MediaReeditResult>, &mut App) + 'static {
        move |result, cx| {
            Self::deliver(&this, cx, move |screen, cx| {
                screen.reedit_in_progress = false;
                cx.notify();
                let Some(result) = result else {
                    return;
                };
                if screen.state.edit_target.as_ref().map(|edit| edit.message_id.as_str()) != Some(message_id.as_str()) {
                    return;
                }
                screen.begin_pending_reedit(result.item, cx);
                if !result.caption.is_empty() || !screen.state.input_text.is_empty() {
                    screen.state.input_text = result.caption;
                }
            });
        }
    }

    pub(in super::super) fn begin_media_reedit(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.state.edit_target.clone() else {
            return;
        };
        if !edit.can_reedit_media || self.reedit_in_progress {
            return;
        }
        self.reedit_in_progress = true;
        cx.notify();
        let message_id = edit.message_id.clone();
        let chat_id = self.resolved_chat_id(cx);
        let server_id = message_services(cx, |store, cx| store.server_message_id(&message_id, &chat_id, cx));
        let caption = self.state.input_text.clone();
        let theme_store = ThemeStore::shared(cx);
        let this = cx.weak_entity();

        let identities = [message_id.clone(), server_id.clone()];
        let document = MediaReeditStore::shared(cx).update(cx, |store, _| {
            let edit_id = store.edit_id_for_any(&identities)?;
            store.load_document(&edit_id, MediaReeditStore::chat_ttl_seconds())
        });
        if let Some(document) = document {
            let stroke_count = document.state.strokes.len();
            let text_count = document.state.texts.len();
            let edit_id = document.edit_id.clone();
            let log_id = edit_id.clone();
            Sai::log("Reedit", move || format!("restore editId={log_id} strokes={stroke_count} texts={text_count}"), SaiLogKind::Debug, cx);
            MediaReeditLauncher::present(
                document.original_image,
                Some(document.state),
                edit_id,
                caption,
                theme_store,
                Self::media_reedit_finish(this, message_id),
                cx,
            );
            return;
        }
        let log_ids = format!("[{message_id}, {server_id}]");
        Sai::log("Reedit", move || format!("no live doc for {log_ids}: flat fallback"), SaiLogKind::Debug, cx);

        let Some(source) = edit.media_full_source.or(edit.media_thumbnail_source) else {
            self.reedit_in_progress = false;
            return;
        };
        cx.spawn(async move |this, cx| {
            let image =
                MediaViewerBitmapLoader::load_into_memory(&source, true, true, CleverImageCachePolicy::default(), &*cx).await;
            let _ = this.update(cx, |screen, cx| {
                let still_editing =
                    screen.state.edit_target.as_ref().map(|edit| edit.message_id.as_str()) == Some(message_id.as_str());
                let (Some(image), true) = (image, still_editing) else {
                    screen.reedit_in_progress = false;
                    cx.notify();
                    return;
                };
                MediaReeditLauncher::present(
                    image,
                    None,
                    Uuid::new_v4().to_string(),
                    caption,
                    theme_store,
                    Self::media_reedit_finish(cx.weak_entity(), message_id),
                    cx,
                );
            });
        })
        .detach();
    }

    pub(in super::super) fn cancel_reply(&mut self, cx: &mut Context<Self>) {
        ChatComposerInsetAnimation::set_last_duration(ChatBottomBarMetrics::COMPOSER_ANIM_DURATION);
        self.state.reply_target = None;
        cx.notify();
    }

    pub(in super::super) fn cancel_edit(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.pending_reedit_item = None;
        self.pending_reedit_thumb_image = None;
        self.reedit_in_progress = false;
        self.picker_replaces_edit_media = false;
        ChatComposerInsetAnimation::set_last_duration(ChatBottomBarMetrics::COMPOSER_ANIM_DURATION);
        self.state.edit_target = None;
        self.state.input_text = self.state.saved_draft.take().unwrap_or_default();
        self.draft_custom_emojis = self.saved_draft_custom_emojis.take().unwrap_or_default();
        let scope = self.composer_draft_scope_key(cx);
        let preference = draft_store(cx, |store, cx| store.draft(&scope, cx)).and_then(|draft| draft.link_preview);
        let text = self.state.input_text.clone();
        self.link_preview_draft.update(cx, |draft, cx| draft.restore(preference, &text, cx));
        cx.notify();
    }

    pub(in super::super) fn edit_last_own_message(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.state.input_text.is_empty()
            || self.state.edit_target.is_some()
            || self.state.is_message_search_active
            || self.is_context_preview(cx)
        {
            return false;
        }
        let chat = self.resolved_chat(cx);
        let messages = self.current_visible_messages(cx);
        let Some(message) = messages.iter().rev().find(|message| {
            let is_own = MessageList::is_own_bubble(message, &chat, cx);
            let authored = is_own || AuthServicesStore::try_read(cx).is_some_and(|auth| auth.is_current_user(&message.sender_id));
            authored && MessagePermissions::new(message, &chat, is_own).can_edit(cx)
        }) else {
            return false;
        };
        let message = message.clone();
        self.begin_edit(&message, cx);
        true
    }

    pub(in super::super) fn restore_failed_edit(
        &mut self,
        edit: ChatComposerEditTarget,
        text: String,
        custom_emojis: Vec<ChatDraftCustomEmoji>,
        draft_scope_key: String,
        cx: &mut Context<Self>,
    ) {
        if self.composer_draft_scope_key(cx) != draft_scope_key
            || self.state.edit_target.is_some()
            || self.state.reply_target.is_some()
            || !self.state.input_text.is_empty()
            || self.pending_reedit_item.is_some()
        {
            return;
        }
        ChatComposerInsetAnimation::set_last_duration(ChatBottomBarMetrics::COMPOSER_ANIM_DURATION);
        self.state.saved_draft = Some(String::new());
        self.saved_draft_custom_emojis = Some(Vec::new());
        self.state.edit_target = Some(edit);
        self.state.input_text = text;
        self.draft_custom_emojis = custom_emojis;
        cx.notify();
    }

    pub(in super::super) fn present_replace_media_menu(&mut self, cx: &mut Context<Self>) {
        self.show_replace_media_menu = true;
        cx.notify();
    }

    /// a paste holding images, videos or files opens the paste sheet instead of editing the text
    pub(in super::super) fn handle_composer_paste(this: &WeakEntity<Self>, window: &mut Window, cx: &mut App) -> bool {
        let Some(screen) = this.upgrade() else {
            return false;
        };
        if !ChatPasteMediaLoader::pasteboard_holds_media(UIPasteboard::general(), cx) {
            return false;
        }
        let Some(item) = cx.read_from_clipboard() else {
            return false;
        };
        let providers = ChatPasteMediaProvider::from_paste(ChatComposerPaste::of(&item, HostOs::current(), |path| path.is_file()));
        window.defer(cx, move |window, cx| {
            screen.update(cx, |screen, cx| screen.present_pasted_media(providers, window, cx));
        });
        true
    }

    pub(in super::super) fn present_pasted_media(&mut self, providers: Vec<ChatPasteMediaProvider>, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pasted_media.is_empty() || self.state.edit_target.is_some() {
            return;
        }
        cx.spawn_in(window, async move |this, cx| {
            let entries = ChatPasteMediaLoader::load(providers, cx).await;
            let rejected = this
                .update_in(cx, |screen, window, cx| {
                    if entries.is_empty() || !screen.pasted_media.is_empty() {
                        return Some(entries);
                    }
                    resign_chat_keyboard(window, cx);
                    screen.pasted_media_shown = entries.clone();
                    screen.pasted_media = entries;
                    cx.notify();
                    None
                })
                .unwrap_or(None);
            if let Some(entries) = rejected {
                ChatPasteMediaLoader::discard(&entries);
            }
        })
        .detach();
    }

    pub(in super::super) fn close_pasted_media(&mut self, cx: &mut Context<Self>) {
        let entries = std::mem::take(&mut self.pasted_media);
        cx.notify();
        ChatPasteMediaLoader::discard(&entries);
    }

    pub(in super::super) fn send_pasted_media(&mut self, as_file: bool, spoiler: bool, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.pasted_media.iter().map(|entry| entry.item(as_file, spoiler)).collect();
        self.pasted_media.clear();
        cx.notify();
        self.handle_media_picker_result(MediaPickerResult::new(items, false), ChatMediaPickerMode::Attachments, window, cx);
    }

    pub(in super::super) fn open_replace_media_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.picker_replaces_edit_media = true;
        resign_chat_keyboard(window, cx);
        self.state.media_picker_mode = ChatMediaPickerMode::Attachments;
        cx.notify();
    }

    pub(in super::super) fn chat_circle_chrome_button(
        id: &'static str,
        size: f32,
        is_enabled: bool,
        action: impl Fn(&mut Window, &mut App) + 'static,
        icon: impl IntoIterator<Item = impl IntoElement>,
    ) -> LiquidGlassButton {
        LiquidGlassButton::new(id, LiquidGlassButtonShape::Circle)
            .size(size)
            .disabled(!is_enabled)
            .on_press(action)
            .children(icon)
    }

    pub(in super::super) fn composer_text_locked_label(&mut self, theme: &AppTheme, height: f32, rtl: bool, cx: &mut Context<Self>) -> AnyElement {
        let color = theme.text.secondary.opacity(0.82);
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(
                plain("chat-composer-text-locked")
                    .on_press(|_, cx| {
                        chat_interactions(cx, |store, cx| store.member_send_refused(ErrorCode::ChatSendRestricted.wire(), cx));
                    })
                    .child(
                        div()
                            .w_full()
                            .h(px(height))
                            .flex()
                            .when(!rtl, |row| row.flex_row().pl(px(16.0)).pr(px(46.0)))
                            .when(rtl, |row| row.flex_row_reverse().pr(px(16.0)).pl(px(46.0)))
                            .items_center()
                            .gap(px(6.0))
                            .children(IconUi::sized("lock.fill", px(12.0), color, false))
                            .child(MainText::new(tr("chat_text_locked_placeholder")).px(15.0).color(color).number_of_lines(1)),
                    ),
            )
            .into_any_element()
    }

    pub(in super::super) fn composer_picker_toggle(&mut self, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        plain("chat-composer-picker-toggle")
            .on_press(Self::act(cx.weak_entity(), |screen, window, cx| screen.toggle_picker(window, cx)))
            .child(
                div()
                    .w(px(38.0))
                    .h(px(ChatBottomBarMetrics::CIRCLE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .children(IconUi::sized(self.composer_input_surface.toggle_icon_system_name(), px(21.0), theme.text.secondary, false)),
            )
            .guide_anchor(RiseGuideAnchor::ChatComposerStickers)
            .keyboard_dismiss_excluded()
            .into_any_element()
    }

    pub(in super::super) fn chrome_button_appearing<E: IntoElement>(id: &'static str, button: E, window: &mut Window, cx: &mut App) -> AnyElement {
        Self::appearing(
            id,
            Transition::scale(0.7).combined(Transition::opacity()),
            spring(0.32, 0.82),
            div().flex_none().child(button),
            window,
            cx,
        )
    }

    pub(in super::super) fn watch_replace_media_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let this = cx.weak_entity();
        Self::watch(
            &this,
            "chat-replace-media-menu",
            self.show_replace_media_menu,
            false,
            window,
            cx,
            |screen, _, is_presented, window, cx| {
                if !*is_presented {
                    return;
                }
                screen.show_replace_media_menu = false;
                let confirm_screen = cx.weak_entity();
                let another_screen = cx.weak_entity();
                let handle = window.window_handle();
                let on_another: NativeConfirmAlertAction = Box::new(move |cx: &mut App| {
                    Self::deliver_in_window(&another_screen, handle, cx, |screen, window, cx| {
                        screen.open_replace_media_picker(window, cx);
                    });
                });
                NativeConfirmAlert::present_delete(
                    &tr("reedit_replace_title"),
                    "",
                    Some(tr("reedit_open_editor")),
                    Some(tr("reedit_choose_another")),
                    false,
                    Some(on_another),
                    None,
                    move |cx: &mut App| {
                        Self::deliver(&confirm_screen, cx, |screen, cx| screen.begin_media_reedit(cx));
                    },
                    cx,
                );
            },
        );
    }

    pub(in super::super) fn send_text_file_alert(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.show_send_text_file {
            return None;
        }
        let entity = cx.entity();
        let name = Binding::entity(
            entity.clone(),
            |screen: &Self| screen.send_text_file_name.clone(),
            |screen: &mut Self, value: String, cx: &mut Context<Self>| {
                screen.send_text_file_name = value;
                cx.notify();
            },
        );
        Some(
            TextFieldAlert::new(
                "chat-send-text-file-alert",
                Self::flag_binding(entity, |screen| screen.show_send_text_file, |screen, value| screen.show_send_text_file = value),
                tr("chat_send_text_file_title"),
                tr("chat_send_text_file_name"),
                Self::text_file_name_rule().binding(name),
                tr("common_cancel"),
                tr("share_send"),
                Self::act(cx.weak_entity(), |screen, window, cx| screen.send_composer_text_as_file(window, cx)),
            )
            .into_any_element(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn composer_leading_controls(
        &mut self,
        theme: &AppTheme,
        capture_busy: bool,
        has_voice_draft: bool,
        input_height: f32,
        text_locked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let this = cx.weak_entity();
        let circle = ChatBottomBarMetrics::CIRCLE;
        let mut controls = Vec::new();
        if self.capture_mode == ChatComposerCaptureMode::VideoNote && self.is_any_capture_active(cx) {
            let flip = Self::chat_circle_chrome_button(
                "chat-capture-flip",
                circle,
                true,
                Self::act(this.clone(), |screen, _, cx| {
                    screen.video_note_recorder.update(cx, |recorder, cx| recorder.flip_camera(cx));
                }),
                IconUi::sized("arrow.triangle.2.circlepath.camera.fill", px(17.0), theme.text.primary, false),
            );
            controls.push(Self::chrome_button_appearing("chat-capture-flip-presence", flip, window, cx));
            if !self.video_note_recorder.read(cx).is_locked_or_finishing() {
                let pause = Self::chat_circle_chrome_button(
                    "chat-capture-pause",
                    circle,
                    true,
                    Self::act(this, |screen, _, cx| screen.handle_capture_pause_or_resume(cx)),
                    IconUi::sized(if self.is_capture_paused(cx) { "play.fill" } else { "pause.fill" }, px(15.0), theme.text.primary, false),
                );
                controls.push(Self::chrome_button_appearing("chat-capture-pause-presence", pause, window, cx));
            }
            return controls;
        }
        if has_voice_draft {
            let label = tr("ctx_delete");
            let discard = Self::chat_circle_chrome_button(
                "chat-voice-draft-discard",
                circle,
                true,
                Self::act(this, |screen, _, cx| screen.discard_voice_draft(cx)),
                IconUi::sized("trash", px(17.0), theme.semantic.error_100, false),
            )
            .accessibility_label(Some(label.clone()));
            let host = div().id("chat-voice-draft-discard-host").tooltip_label(label).child(discard);
            controls.push(Self::chrome_button_appearing("chat-voice-draft-discard-presence", host, window, cx));
            return controls;
        }
        if capture_busy {
            return controls;
        }
        if self.resolved_chat(cx).is_active_bot_chat() {
            let label = tr("bot_open_menu");
            let menu = LiquidGlassButton::new("chat-bot-menu", LiquidGlassButtonShape::Circle)
                .size(BotChatMenuPresentationContract::BUTTON_SIZE)
                .tint(Some(theme.primary._100))
                .accessibility_label(Some(label.clone()))
                .on_press(Self::act(this.clone(), |screen, window, cx| screen.toggle_bot_chat_menu(window, cx)))
                .children(IconUi::sized(if self.show_bot_chat_menu { "xmark" } else { "line.3.horizontal" }, px(15.0), theme.content_on(theme.primary._100), false));
            let host = div().id("chat-bot-menu-host").tooltip_label(label).child(menu);
            controls.push(Self::chrome_button_appearing("chat-bot-menu-presence", host, window, cx));
        }
        let editing_media = self.state.edit_target.as_ref().is_some_and(|edit| edit.can_reedit_media);
        let attachments_locked = !editing_media && self.member_attachments_locked(cx);
        let offers_text_file = self.composer_overflows_message
            && self.state.edit_target.is_none()
            && !attachments_locked
            && !text_locked
            && input_height >= circle * 2.0 + ChatBottomBarMetrics::ROW_GAP;
        let text_file = offers_text_file.then(|| {
            let label = tr("chat_send_text_file_button");
            let button = Self::chat_circle_chrome_button(
                "chat-send-text-file",
                circle,
                true,
                Self::act(this.clone(), |screen, _, cx| screen.open_send_text_file(cx)),
                IconUi::sized("doc.text", px(17.0), theme.text.primary, false),
            )
            .accessibility_label(Some(label.clone()));
            let host = div().id("chat-send-text-file-host").tooltip_label(label).child(button);
            Self::chrome_button_appearing("chat-send-text-file-presence", host, window, cx)
        });
        let attach_icon_color = theme.text.primary.opacity(if attachments_locked { 0.35 } else { 1.0 });
        let attach = Self::chat_circle_chrome_button(
            "chat-attach",
            circle,
            true,
            Self::act(this, move |screen, window, cx| {
                if editing_media {
                    screen.present_replace_media_menu(cx);
                } else if attachments_locked {
                    chat_interactions(cx, |store, cx| store.member_send_refused(ErrorCode::ChatSendRestricted.wire(), cx));
                } else {
                    resign_chat_keyboard(window, cx);
                    screen.state.media_picker_mode = ChatMediaPickerMode::Attachments;
                    cx.notify();
                }
            }),
            IconUi::sized(
                if editing_media { "arrow.triangle.2.circlepath" } else { "paperclip" },
                px(if editing_media { 16.0 } else { 18.0 }),
                attach_icon_color,
                false,
            ),
        )
        .guide_anchor(if editing_media { RiseGuideAnchor::ChatComposerReplaceMedia } else { RiseGuideAnchor::ChatComposerAttach });
        controls.push(
            div()
                .flex_none()
                .flex()
                .flex_col()
                .gap(px(ChatBottomBarMetrics::ROW_GAP))
                .children(text_file)
                .child(Self::chrome_button_appearing("chat-attach-presence", attach, window, cx))
                .into_any_element(),
        );
        controls
    }

    pub(in super::super) fn composer_input_stack(
        &mut self,
        theme: &AppTheme,
        placeholder_key: &str,
        input_height: f32,
        text_locked: bool,
        attached_to_inline_panel: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let this = cx.weak_entity();
        let circle = ChatBottomBarMetrics::CIRCLE;
        let inline_bot = self.inline_composer_bot_identity(cx);
        let inline_bot_leading_inset = if inline_bot.is_some() { 36.0 } else { 0.0 };
        let editor_width = (self.composer_input_width - inline_bot_leading_inset).max(1.0);
        let shows_context_bar = self.composer_context_bar_visible(cx);
        let context_bar = shows_context_bar.then(|| self.composer_context_bar(theme, cx)).flatten().map(|bar| {
            div().w_full().flex_none().pl(px(14.0)).pr(px(6.0)).h(px(ChatBottomBarMetrics::REPLY_EDIT_BAR_HEIGHT)).child(bar)
        });
        let link_preview = self.composer_link_preview_bar_visible(cx).then(|| {
            div().w_full().flex_none().pl(px(14.0)).pr(px(6.0)).h(px(ChatBottomBarMetrics::REPLY_EDIT_BAR_HEIGHT)).child(
                ChatLinkPreviewBar::new("chat-link-preview-bar", self.link_preview_draft.clone(), theme.clone()),
            )
        });

        let mut input = div().relative().w_full().flex_none().h(px(input_height));
        let rtl = LayoutDirection::current().is_right_to_left();
        if let Some(bot) = inline_bot {
            input = input.child(
                div().absolute().when(!rtl, |d| d.left(px(8.0))).when(rtl, |d| d.right(px(8.0))).top(px(7.0)).child(
                    UserLogoUi::new("chat-composer-inline-bot")
                        .source(SharedString::from(bot.avatar_url))
                        .name(SharedString::from(bot.name))
                        .user_id(SharedString::from(bot.bot_user_id))
                        .size(28.0)
                        .show_story_ring(false),
                ),
            );
        }
        if text_locked {
            input = input.child(self.composer_text_locked_label(theme, circle, rtl, cx));
        } else {
            if self.state.input_text.is_empty() {
                input = input.child(
                    div()
                        .absolute()
                        .when(!rtl, |d| d.left(px(16.0 + inline_bot_leading_inset)).right(px(circle)))
                        .when(rtl, |d| d.right(px(16.0 + inline_bot_leading_inset)).left(px(circle)))
                        .top(px(10.0))
                        .overflow_hidden()
                        .child(
                            MainText::new(placeholder_key.to_owned())
                                .px(15.0)
                                .color(theme.text.secondary.opacity(0.82))
                                .line_limit(1),
                        ),
                );
            }
            let entity = cx.entity();
            let text = Binding::entity(
                entity.clone(),
                |screen: &Self| screen.state.input_text.clone(),
                |screen: &mut Self, value: String, cx: &mut Context<Self>| {
                    screen.state.input_text = value;
                    cx.notify();
                },
            );
            let measured_height = Binding::entity(
                entity,
                |screen: &Self| screen.composer_input_height,
                |screen: &mut Self, value: f32, cx: &mut Context<Self>| {
                    screen.composer_input_height = value;
                    cx.notify();
                },
            );
            let selection_screen = this.clone();
            let runs_screen = this.clone();
            let paste_screen = this.clone();
            let field = ChatComposerTextView::new(
                "chat-composer-text",
                text,
                editor_width,
                circle,
                ChatBottomBarMetrics::INPUT_MAX_HEIGHT,
                theme.text.primary,
                RichEditorStyle::make(
                    MessageSizes::composer_text_font_size(),
                    theme.text.primary,
                    theme.text.secondary.opacity(0.7),
                    theme.text.primary,
                    theme.text.primary.opacity(0.12),
                ),
                measured_height,
            )
            .custom_emoji_runs(self.composer_custom_emoji_runs(&self.state.input_text))
            .focus_token(self.composer_focus_token)
            .requested_selection(self.composer_requested_selection)
            .selection_token(self.composer_selection_token)
            .on_selection_change(move |selection, cx| {
                let _ = selection_screen.update(cx, |screen, cx| {
                    let changed = screen.composer_selection != selection;
                    screen.composer_selection = selection;
                    screen.composer_selection_text = screen.state.input_text.clone();
                    if changed {
                        cx.notify();
                    }
                });
            })
            .on_custom_emoji_runs_change(move |runs, cx| {
                let _ = runs_screen.update(cx, |screen, cx| {
                    let unchanged = runs.len() == screen.draft_custom_emojis.len()
                        && runs.iter().zip(&screen.draft_custom_emojis).all(|(run, draft)| run.item == draft.item);
                    if unchanged {
                        return;
                    }
                    screen.draft_custom_emojis = runs
                        .into_iter()
                        .map(|run| ChatDraftCustomEmoji { placeholder: COMPOSER_CUSTOM_EMOJI_PLACEHOLDER.to_owned(), item: run.item })
                        .collect();
                    cx.notify();
                });
            })
            .on_paste(move |window, cx| Self::handle_composer_paste(&paste_screen, window, cx))
            .on_send(Self::act(this.clone(), |screen, window, cx| screen.handle_send_button(window, cx)))
            .on_cancel(Self::act(this.clone(), |screen, window, cx| {
                let _ = screen.handle_escape(window, cx);
            }));
            input = input.child(
                div()
                    .absolute()
                    .top_0()
                    .when(!rtl, |d| d.left(px(inline_bot_leading_inset)))
                    .when(rtl, |d| d.right(px(inline_bot_leading_inset)))
                    .w(px(editor_width))
                    .h(px(self.composer_input_height))
                    .capture_action({
                        let this = this.clone();
                        move |_: &MoveUp, window: &mut Window, cx: &mut App| {
                            let handled = this.update(cx, |screen, cx| screen.edit_last_own_message(window, cx)).unwrap_or(false);
                            if handled {
                                cx.stop_propagation();
                            }
                        }
                    })
                    .child(field),
            );
        }
        let toggle = self.composer_picker_toggle(theme, cx);
        input = input.child(if rtl {
            div().absolute().left_0().bottom_0().pl(px(4.0)).child(toggle)
        } else {
            div().absolute().right_0().bottom_0().pr(px(4.0)).child(toggle)
        });
        let opaque_fill = LiquidChrome::surface_fill(&theme, 0);

        div()
            .w_full()
            .relative()
            .flex()
            .flex_col()
            .child(measure("chat-composer-input-geometry", move |bounds, _, cx| {
                let width = f32::from(bounds.size.width);
                let _ = this.update(cx, |screen, cx| {
                    if (screen.composer_input_width - width).abs() > 0.5 {
                        screen.composer_input_width = width;
                        cx.notify();
                    }
                });
            })
            .absolute()
            .size_full())
            .children((!attached_to_inline_panel).then(|| {
                div().absolute().inset_0().rounded(px(ChatBottomBarMetrics::INPUT_CORNER)).bg(opaque_fill)
            }))
            .children(context_bar)
            .children(link_preview)
            .child(input)
            .chat_composer_input_chrome(attached_to_inline_panel)
    }

    #[allow(clippy::too_many_arguments)]
    pub(in super::super) fn bottom_bar(
        &mut self,
        theme: &AppTheme,
        placeholder_key: &str,
        safe_bottom_inset: f32,
        peer_blocked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let this = cx.weak_entity();
        let bottom_pad = self.composer_bottom_padding(safe_bottom_inset, cx);
        let text_locked = self.composer_text_locked(cx);
        let can_send = (!text_locked && !is_blank(&self.state.input_text))
            || (self.state.edit_target.is_some() && self.pending_reedit_item.is_some());
        let circle = ChatBottomBarMetrics::CIRCLE;
        let input_height = if text_locked { circle } else { self.composer_input_height };
        let ui_phase = self.recording_ui_phase(cx);
        let capture_busy = self.is_any_capture_active(cx);
        let voice_draft = if capture_busy { None } else { self.active_voice_draft(cx) };
        let input_visible = ui_phase == ChatComposerRecordingUIPhase::Idle && voice_draft.is_none();
        let inline_attached = self.inline_composer_panel_visible(peer_blocked, cx) && !self.inline_composer_panel_detached();

        self.watch_replace_media_menu(window, cx);
        let leading = self.composer_leading_controls(theme, capture_busy, voice_draft.is_some(), input_height, text_locked, window, cx);
        let input = self.composer_input_stack(theme, placeholder_key, input_height, text_locked, inline_attached, cx);
        let context_height = self.composer_context_height(cx);

        let mut stack = div()
            .relative()
            .w_full()
            .h(px(if input_visible { input_height + context_height } else { circle }))
            .child(div().size_full().opacity(if input_visible { 1.0 } else { 0.0 }).child(input));
        if ui_phase == ChatComposerRecordingUIPhase::Holding {
            stack = stack.child(div().absolute().inset_0().occlude().child(ChatVoiceHoldTrack::new(
                "chat-voice-hold-track",
                theme.clone(),
                circle,
                self.active_capture_elapsed_ms(cx),
                self.active_capture_clock_reader(),
                self.active_capture_shows_cancel_hint(cx),
                self.active_capture_drag_translation(cx),
            )));
        }
        if ui_phase == ChatComposerRecordingUIPhase::Locked {
            stack = stack.child(div().absolute().inset_0().occlude().child(ChatVoiceLockedTrack::new(
                "chat-voice-locked-track",
                theme.clone(),
                circle,
                self.active_capture_elapsed_ms(cx),
                self.active_capture_clock_reader(),
                self.is_capture_paused(cx),
                Self::act(this.clone(), |screen, _, cx| screen.handle_locked_capture_cancel(cx)),
            )));
        }
        if let Some(draft) = voice_draft.as_ref() {
            stack = stack.child(div().absolute().inset_0().occlude().child(ChatVoiceDraftTrack::new(
                "chat-voice-draft-track",
                theme.clone(),
                circle,
                draft.clone(),
                self.voice_draft_playback_id(cx),
                Self::act(this.clone(), |screen, _, cx| screen.toggle_voice_draft_playback(cx)),
            )));
        }
        let center = div().flex_1().min_w_0().child(stack).guide_anchor(RiseGuideAnchor::ChatComposerInput);

        let chat_id = self.resolved_chat_id(cx);
        let slow_mode_until = if self.state.edit_target.is_none() {
            ChatServicesStore::shared(cx).read(cx).slow_mode_until.get(&chat_id).cloned()
        } else {
            None
        };
        let trailing: AnyElement = if self.shows_locked_capture_controls(cx) {
            div()
                .flex()
                .flex_row()
                .gap(px(8.0))
                .child(ChatVoicePauseButton::new(
                    "chat-voice-pause",
                    circle,
                    self.is_capture_paused(cx),
                    theme.clone(),
                    Self::act(this.clone(), |screen, _, cx| screen.handle_capture_pause_or_resume(cx)),
                ))
                .child(ChatVoiceSendButton::new(
                    "chat-voice-send",
                    circle,
                    theme.clone(),
                    Self::act(this.clone(), |screen, window, cx| screen.handle_locked_capture_send(window, cx)),
                ))
                .into_any_element()
        } else if let Some(until) = slow_mode_until {
            let tap_chat_id = chat_id.clone();
            let expire_chat_id = chat_id.clone();
            ChatSlowModeCountdownButton {
                id: "chat-slow-mode-countdown".into(),
                size: circle,
                until,
                theme: theme.clone(),
                on_tap: Rc::new(move |_: &mut Window, cx: &mut App| {
                    let _ = chat_interactions(cx, |store, cx| store.slow_mode_blocks_send(&tap_chat_id, cx));
                }),
                on_expire: Rc::new(move |_: &mut Window, cx: &mut App| {
                    chat_interactions(cx, |store, cx| store.slow_mode_countdown_ended(&expire_chat_id, cx));
                }),
            }
            .guide_anchor(RiseGuideAnchor::ChatComposerSend)
            .into_any_element()
        } else if voice_draft.is_some() {
            ChatVoiceSendButton::new(
                "chat-voice-draft-send",
                circle,
                theme.clone(),
                Self::act(this.clone(), |screen, window, cx| screen.send_voice_draft(window, cx)),
            )
            .into_any_element()
        } else {
            let drag_screen = this.clone();
            let end_screen = this.clone();
            let records_captioned_note = can_send && self.state.edit_target.is_none();
            ChatComposerSendMicButton {
                id: "chat-composer-send-mic".into(),
                size: circle,
                has_text: can_send,
                is_editing: self.state.edit_target.is_some(),
                capture_mode: self.capture_mode,
                is_recording: self.is_capture_recording(cx),
                theme: theme.clone(),
                on_send: Rc::new(Self::act(this.clone(), |screen, window, cx| screen.handle_send_button(window, cx))),
                on_toggle_capture_mode: Rc::new(Self::act(this.clone(), move |screen, _, cx| {
                    if capture_busy {
                        return;
                    }
                    screen.capture_mode = if screen.capture_mode == ChatComposerCaptureMode::Voice {
                        ChatComposerCaptureMode::VideoNote
                    } else {
                        ChatComposerCaptureMode::Voice
                    };
                    let _ = impact(ImpactStyle::Light);
                    cx.notify();
                })),
                on_capture_press_began: Rc::new(Self::act(this.clone(), |screen, _, cx| screen.handle_capture_press_began(cx))),
                on_capture_press_ended: Rc::new(Self::act(this.clone(), |screen, _, cx| screen.handle_capture_press_ended(cx))),
                on_hold_began: Rc::new(Self::act(this.clone(), move |screen, window, cx| {
                    if records_captioned_note && !screen.is_any_capture_active(cx) {
                        screen.capture_mode = ChatComposerCaptureMode::VideoNote;
                    }
                    screen.handle_capture_hold_began(window, cx);
                })),
                on_hold_drag: Rc::new(move |translation: gpui::Size<Pixels>, _: &mut Window, cx: &mut App| {
                    let _ = drag_screen.update(cx, |screen, cx| screen.handle_capture_hold_drag(translation, cx));
                }),
                on_hold_ended: Rc::new(move |translation: gpui::Size<Pixels>, cancelled: bool, window: &mut Window, cx: &mut App| {
                    let _ = end_screen.update(cx, |screen, cx| {
                        screen.handle_capture_hold_ended(translation, cancelled, window, cx);
                    });
                }),
            }
            .hosted(window, cx)
            .guide_anchor(RiseGuideAnchor::ChatComposerSend)
            .into_any_element()
        };

        let alert = self.send_text_file_alert(cx);
        let is_rtl = LayoutDirection::current().is_right_to_left();
        div()
            .w_full()
            .flex_none()
            .flex()
            .when(!is_rtl, |row| row.flex_row())
            .when(is_rtl, |row| row.flex_row_reverse())
            .items_end()
            .gap(px(ChatBottomBarMetrics::ROW_GAP))
            .px(px(ChatBottomBarMetrics::H_PADDING))
            .pt(px(if inline_attached { 0.0 } else { ChatBottomBarMetrics::TOP_PADDING }))
            .pb(px(bottom_pad))
            .children(leading)
            .child(center)
            .child(trailing)
            .children(alert)
            .keyboard_dismiss_excluded()
            .into_any_element()
    }

    pub(in super::super) fn composer_context_bar(&mut self, theme: &AppTheme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let this = cx.weak_entity();
        if let Some(edit) = self.state.edit_target.clone() {
            let thumbnail = edit.can_reedit_media.then(|| self.reedit_thumbnail(edit.media_thumbnail_source.as_deref(), theme, cx));
            let subtitle = if edit.can_reedit_media && edit.original_text.is_empty() {
                tr("composer_photo")
            } else {
                edit.preview_text.clone()
            };
            return Some(
                div()
                    .size_full()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.0))
                    .children(thumbnail)
                    .child(ChatComposerContextBar::new(
                        "chat-composer-edit-bar",
                        ChatComposerContextBarKind::Edit,
                        tr("composer_editing"),
                        subtitle,
                        theme.clone(),
                        Self::act(this, |screen, window, cx| screen.cancel_edit(window, cx)),
                    ))
                    .into_any_element(),
            );
        }
        let reply = self.state.reply_target.clone()?;
        Some(
            ChatComposerContextBar::new(
                "chat-composer-reply-bar",
                ChatComposerContextBarKind::Reply,
                reply.author_name,
                reply.snippet,
                theme.clone(),
                Self::act(this, |screen, _, cx| screen.cancel_reply(cx)),
            )
            .media_thumbnail_source(reply.media_thumbnail_source)
            .into_any_element(),
        )
    }

    pub(in super::super) fn reedit_thumbnail(&mut self, source: Option<&str>, theme: &AppTheme, cx: &mut Context<Self>) -> AnyElement {
        let base: AnyElement = if let Some(preview) = self.pending_reedit_thumb_image.clone() {
            div()
                .size(px(38.0))
                .max_h(px(38.0))
                .rounded(px(8.0))
                .overflow_hidden()
                .child(
                    gpui::img(gpui::ImageSource::Render(preview))
                        .size(px(38.0))
                        .max_w(px(38.0))
                        .max_h(px(38.0))
                        .rounded(px(8.0))
                        .object_fit(gpui::ObjectFit::Cover),
                )
                .into_any_element()
        } else if let Some(source) = source.filter(|source| !source.is_empty()) {
            div()
                .size(px(38.0))
                .rounded(px(8.0))
                .overflow_hidden()
                .child(
                    CleverImageUi::new(SharedString::from(source.to_owned()))
                        .id("chat-reedit-thumbnail-image")
                        .content_mode(ContentMode::Fill)
                        .corner_radius(8.0)
                        .persist_to_disk(true)
                        .maximum_decode_pixel_dimension(114_i64),
                )
                .into_any_element()
        } else {
            div().size(px(38.0)).rounded(px(8.0)).bg(theme.text.secondary.opacity(0.18)).into_any_element()
        };
        let in_progress = self.reedit_in_progress;
        div()
            .id("chat-reedit-thumbnail")
            .relative()
            .flex_none()
            .size(px(38.0))
            .flex()
            .items_center()
            .justify_center()
            .opacity(if in_progress { 0.55 } else { 1.0 })
            .cursor_pointer()
            .occlude()
            .on_click(cx.listener(move |screen, _, _, cx| {
                if screen.reedit_in_progress {
                    return;
                }
                let _ = impact(ImpactStyle::Light);
                screen.present_replace_media_menu(cx);
            }))
            .child(div().absolute().inset_0().child(base))
            .child(div().absolute().size(px(24.0)).rounded_full().bg(gpui::black().opacity(0.38)))
            .children(IconUi::sized("pencil", px(13.0), gpui::white(), false))
            .into_any_element()
    }
}
