use gpui::App;
use rise_canon::RiseGuideAnchor;
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_core::foundation::foundation_url::Url;
use rise_i18n::tr;
use rise_ui::hold_context_menu_ui::hold_context_menu_types::HoldContextMenuAction;

use crate::core::config::app_environment::AppEnvironment;
use crate::core::lib::sai::core::utilities::get_sai_server_id::GetSaiServerId;
use crate::core::lib::sai::modules::sai_ws::sai_ws_response_cache::SaiWsResponseCache;
use crate::modules::chat::shared::media_save_target_hooks::{MediaSaveTargetKind, MessageMediaSaveClassifier};
use crate::core::push_notifications::riseonly_public_link::RiseonlyPublicLink;
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::chat_engine_hooks::chat_engine_hooks;
use crate::modules::chat::shared::chat_permission_resolution::ChatPermissionResolution;
use crate::modules::chat::shared::chat_shared_sparks_hooks::chat_shared_sparks_hooks;
use crate::modules::chat::stores::chat_guide_sandbox_hooks::chat_guide_sandbox_hooks;
use crate::modules::chat::stores::chats::chat_actions::chat_types::{ChatInfo, ChatPerm, ChatPermExt, ChatType};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::EffectivePermissionsDTOExt;
use crate::modules::chat::stores::message::message_actions::message_types::{
    GetMessageMessage, MessageContentType, MessageDeliveryStatus, MessageMediaItem,
};

pub enum ChatMessageAction {}

impl ChatMessageAction {
    pub const REPLY: &'static str = "reply";
    pub const COPY: &'static str = "copy";
    pub const COPY_LINK: &'static str = "copy.link";
    pub const FORWARD: &'static str = "forward";
    pub const PIN: &'static str = "pin";
    pub const UNPIN: &'static str = "unpin";
    pub const EDIT: &'static str = "edit";
    pub const DELETE: &'static str = "delete";
    pub const CANCEL_SENDING: &'static str = "cancel.sending";
    pub const RETRY_SENDING: &'static str = "retry.sending";
    pub const FAILED_SEND_OPTIONS: &'static str = "failed.send.options";
    pub const REPORT: &'static str = "report";
    pub const SAVE: &'static str = "save";
    pub const SAVE_TO_GALLERY: &'static str = "save.gallery";
    pub const SELECT: &'static str = "select";
    pub const SHOW_IN_CHAT: &'static str = "show.in.chat";
    pub const SHOW_IN_FAVOURITES: &'static str = "show.in.favourites";
    pub const ADD_TO_LOCAL_PLAYLIST: &'static str = "music.add_to_local_playlist";
    pub const SEND_SPARKS: &'static str = "sparks.send";
    pub const MENTION_VIA_BOT: &'static str = "message.via_bot.mention";
    const READ_RECEIPT_USER_PREFIX: &'static str = "message.read_receipt.user:";
    const JUMP_TO_MESSAGE_PREFIX: &'static str = "message.jump_to:";
    const REPLY_TO_MESSAGE_PREFIX: &'static str = "message.reply_to:";
    const BOT_INLINE_QUERY_PREFIX: &'static str = "bot.inline_query_current_chat:";

    pub const DELETE_EVERYONE: &'static str = "delete.everyone";
    pub const DELETE_ME: &'static str = "delete.me";
    pub const DELETE_MODERATE: &'static str = "delete.moderate";
    pub const DELETE_FROM_FAVOURITES: &'static str = "delete.from.favourites";
    pub const PIN_EVERYONE_NOTIFY: &'static str = "pin.everyone.notify";
    pub const PIN_EVERYONE_SILENT: &'static str = "pin.everyone.silent";
    pub const PIN_EVERYONE: &'static str = "pin.everyone";
    pub const PIN_ME: &'static str = "pin.me";

    pub fn read_receipt_user_action(user_id: &str) -> String {
        format!("{}{user_id}", Self::READ_RECEIPT_USER_PREFIX)
    }

    pub fn read_receipt_user_id(action: &str) -> Option<String> {
        Self::trimmed_suffix(action, Self::READ_RECEIPT_USER_PREFIX)
    }

    /// the target rides inside the action id because the bubble only talks back through
    /// on_message_action
    /// and that names the tapped message
    pub fn jump_to_message_action(message_id: &str) -> String {
        format!("{}{message_id}", Self::JUMP_TO_MESSAGE_PREFIX)
    }

    pub fn jump_to_message_id(action: &str) -> Option<String> {
        Self::trimmed_suffix(action, Self::JUMP_TO_MESSAGE_PREFIX)
    }

    pub fn reply_to_message_action(message_id: &str) -> String {
        format!("{}{message_id}", Self::REPLY_TO_MESSAGE_PREFIX)
    }

    pub fn reply_to_message_id(action: &str) -> Option<String> {
        Self::trimmed_suffix(action, Self::REPLY_TO_MESSAGE_PREFIX)
    }

    pub fn bot_inline_query_action(query: &str) -> String {
        format!("{}{query}", Self::BOT_INLINE_QUERY_PREFIX)
    }

    pub fn bot_inline_query(action: &str) -> Option<String> {
        action.strip_prefix(Self::BOT_INLINE_QUERY_PREFIX).map(str::to_owned)
    }

    fn trimmed_suffix(action: &str, prefix: &str) -> Option<String> {
        let suffix = trimming_whitespaces_and_newlines(action.strip_prefix(prefix)?);
        (!suffix.is_empty()).then(|| suffix.to_owned())
    }
}

/// moderation needs a resolved grant
/// the chat row can_manage_chat also covers change info so it cant prove delete, ban or restrict
pub enum ChatModerationRights {}

impl ChatModerationRights {
    pub fn allows(chat: &ChatInfo, permission: i64, cx: &mut App) -> bool {
        if chat.r#type != ChatType::Group && chat.r#type != ChatType::Channel {
            return false;
        }
        if !is_joined_member_role(chat.role.as_deref()) {
            return false;
        }
        if ChatPerm::is_owner_role(chat.role.as_deref()) {
            return true;
        }
        let Some(mask) = Self::resolved_mask(&chat.id, None, cx) else {
            return false;
        };
        ChatPerm::has(mask, ChatPerm::VIEW_CHANNELS) && ChatPerm::has(mask, permission)
    }

    pub fn holds_unless_denied(chat: &ChatInfo, permission: i64, topic_id: Option<&str>, cx: &mut App) -> bool {
        if ChatPerm::is_owner_role(chat.role.as_deref()) {
            return true;
        }
        match Self::topic_or_chat_mask(chat, topic_id, cx) {
            Some(mask) => ChatPerm::has(mask, permission),
            None => true,
        }
    }

    /// a right a role hands to plain members like pinning
    /// row flag isnt patched when owner edits a role but the mask reloads on every role event
    /// so a loaded mask answers first, the row only until then
    pub fn holds(chat: &ChatInfo, permission: i64, topic_id: Option<&str>, row_flag: bool, cx: &mut App) -> bool {
        if ChatPerm::is_owner_role(chat.role.as_deref()) {
            return true;
        }
        match Self::topic_or_chat_mask(chat, topic_id, cx) {
            Some(mask) => ChatPerm::has(mask, permission),
            None => row_flag,
        }
    }

    fn topic_or_chat_mask(chat: &ChatInfo, topic_id: Option<&str>, cx: &mut App) -> Option<i64> {
        let topic = match topic_id {
            Some(topic_id) if !topic_id.is_empty() => Self::resolved_mask(&chat.id, Some(topic_id), cx),
            _ => None,
        };
        topic.or_else(|| Self::resolved_mask(&chat.id, None, cx))
    }

    /// read from memory only so a hold or render never waits on disk
    /// zero or no view channels is a real denial not a failed read
    fn resolved_mask(chat_id: &str, topic_id: Option<&str>, cx: &mut App) -> Option<i64> {
        if chat_id.is_empty() {
            return None;
        }
        let hooks = chat_engine_hooks()?;
        SaiWsResponseCache::observed_memory_only(&(hooks.effective_perms_cache_key)(chat_id, topic_id), cx)?;
        (hooks.effective_permissions)(chat_id, topic_id, cx).map(|permissions| permissions.mask())
    }
}

fn is_joined_member_role(role: Option<&str>) -> bool {
    chat_engine_hooks().is_some_and(|hooks| (hooks.is_joined_member_role)(role))
}

/// borrows the message and the chat since a rust clone is deep and the reaction strip asks
/// this per row
#[derive(Clone, Copy, Debug)]
pub struct MessagePermissions<'a> {
    pub message: &'a GetMessageMessage,
    pub chat: &'a ChatInfo,
    pub is_own: bool,
}

impl<'a> MessagePermissions<'a> {
    /// a system row belongs to nobody even if the viewer triggered it, same answer as the held pill
    pub fn new(message: &'a GetMessageMessage, chat: &'a ChatInfo, is_own: bool) -> Self {
        Self { message, chat, is_own: is_own && message.content_type != MessageContentType::System }
    }

    pub fn is_owner(&self) -> bool {
        ChatPerm::is_owner_role(self.chat.role.as_deref())
    }

    fn acts_as_author(&self, cx: &App) -> bool {
        AuthServicesStore::try_read(cx).is_some_and(AuthServicesStore::viewer_acts_as_author)
    }

    pub fn is_admin(&self) -> bool {
        ChatPerm::is_owner_or_legacy_admin_role(self.chat.role.as_deref())
    }

    fn guide_sandbox_owns_message(&self, cx: &App) -> bool {
        (chat_guide_sandbox_hooks().owns_message_id)(&self.message.id, cx)
    }

    fn has_stable_server_id(&self, cx: &App) -> bool {
        if self.guide_sandbox_owns_message(cx) {
            return true;
        }
        let server_id = self.message.sai_server_id();
        let server_id = trimming_whitespaces_and_newlines(&server_id);
        !self.message.is_temp
            && !self.message.is_pending_outgoing()
            && !server_id.is_empty()
            && !GetSaiServerId::is_temp_id(Some(server_id))
    }

    fn has_history_mutation_membership(&self) -> bool {
        match self.chat.r#type {
            ChatType::Direct | ChatType::Favourites => true,
            ChatType::Group | ChatType::Channel => {
                let role = self.chat.role.as_deref().map(trimming_whitespaces_and_newlines);
                match role {
                    Some(role) if !role.is_empty() => is_joined_member_role(Some(role)),
                    _ => self.chat.joined_at.is_some(),
                }
            }
        }
    }

    fn quotes_queued_own_message(&self) -> bool {
        self.message.is_pending_outgoing()
            && self.message.delivery_status != MessageDeliveryStatus::Failed
            && self.message.media_items.is_empty()
            && GetSaiServerId::is_temp_id(Some(self.message.id.as_str()))
    }

    pub fn is_private_like(&self) -> bool {
        self.chat.r#type == ChatType::Direct || self.chat.r#type == ChatType::Favourites
    }

    fn is_system(&self) -> bool {
        self.message.content_type == MessageContentType::System
    }

    fn is_call(&self) -> bool {
        self.message.content_type == MessageContentType::Call
    }

    /// a message hidden by moderation is read only for its author
    /// no reply, forward edit pin or reaction only copy delete and select
    fn is_disabled_by_moderation(&self) -> bool {
        self.message.is_disabled_by_moderation()
    }

    pub fn can_reply(&self, cx: &App) -> bool {
        if self.is_system() || self.is_call() {
            return false;
        }
        if !self.has_stable_server_id(cx) && !self.quotes_queued_own_message() {
            return false;
        }
        if (self.chat.r#type == ChatType::Group || self.chat.r#type == ChatType::Channel)
            && !self.has_history_mutation_membership()
        {
            return false;
        }
        if self.chat.r#type == ChatType::Channel {
            return self.is_admin();
        }
        true
    }

    /// a chat with forwarding off protects its content like telegram
    /// no forward, copy share or save
    pub fn protects_content(&self) -> bool {
        self.message.protects_content_locally() || !self.chat.forward_in_chat_enabled
    }

    pub fn can_copy(&self) -> bool {
        !self.is_system() && !self.is_call() && !self.protects_content() && !self.copy_text().is_empty()
    }

    pub fn can_forward(&self, cx: &App) -> bool {
        !self.is_system()
            && !self.is_call()
            && !self.is_disabled_by_moderation()
            && self.has_stable_server_id(cx)
            && !self.protects_content()
    }

    pub fn can_share(&self) -> bool {
        !self.is_system()
            && !self.is_call()
            && !self.is_disabled_by_moderation()
            && !self.protects_content()
            && self.has_history_mutation_membership()
    }

    /// a channel lays every post out as incoming so is_own is false even for your post
    /// like the server the author edits and so does a manage messages holder
    pub fn can_edit(&self, cx: &mut App) -> bool {
        if self.is_system()
            || self.is_call()
            || !self.has_stable_server_id(cx)
            || !self.is_editable_content()
            || self.message.has_view_limited_media()
        {
            return false;
        }
        if self.is_own || self.acts_as_author(cx) {
            return true;
        }
        if self.chat.r#type != ChatType::Channel || !self.has_history_mutation_membership() {
            return false;
        }
        AuthServicesStore::try_read(cx).is_some_and(|auth| auth.is_current_user(&self.message.sender_id))
            || ChatModerationRights::allows(self.chat, ChatPerm::MANAGE_MESSAGES, cx)
    }

    pub fn already_pinned(&self) -> bool {
        self.message.is_pinned
    }

    pub fn can_pin(&self, cx: &mut App) -> bool {
        if self.is_system() || self.is_call() || !self.has_stable_server_id(cx) {
            return false;
        }
        if self.is_private_like() {
            return true;
        }
        if !self.has_history_mutation_membership() {
            return false;
        }
        self.is_owner()
            || self.acts_as_author(cx)
            || ChatModerationRights::holds(
                self.chat,
                ChatPerm::PIN_MESSAGES,
                self.message.topic_id.as_deref(),
                self.chat.can_pin_messages,
                cx,
            )
    }

    pub fn can_delete_own(&self, cx: &App) -> bool {
        !self.is_system()
            && self.has_stable_server_id(cx)
            && self.has_history_mutation_membership()
            && (self.is_own || self.acts_as_author(cx))
    }

    pub fn can_delete_private_peer(&self, cx: &App) -> bool {
        self.has_stable_server_id(cx) && !self.is_own && self.is_private_like()
    }

    pub fn can_delete_for_me(&self, cx: &App) -> bool {
        self.has_stable_server_id(cx) && self.has_history_mutation_membership()
    }

    pub fn can_moderate_others(&self, cx: &mut App) -> bool {
        if !self.has_stable_server_id(cx) || !self.has_history_mutation_membership() || self.is_own {
            return false;
        }
        if self.chat.r#type != ChatType::Group && self.chat.r#type != ChatType::Channel {
            return false;
        }
        self.acts_as_author(cx) || ChatModerationRights::allows(self.chat, ChatPerm::MANAGE_MESSAGES, cx)
    }

    /// unsent row has no server id so normal actions are stripped
    /// cancel still works, its all local and clears the outbox entry
    pub fn can_cancel_sending(&self) -> bool {
        !self.is_system()
            && (self.message.is_pending_outgoing() || self.message.delivery_status == MessageDeliveryStatus::Failed)
    }

    /// retry shows only after the auto sender gave up, while queued it leaves by itself when
    /// connection returns
    /// unsent row of a draft private chat has no outbox entry so retry creates the chat first
    pub fn can_retry_sending(&self, cx: &mut App) -> bool {
        if self.is_system() || self.message.delivery_status != MessageDeliveryStatus::Failed {
            return false;
        }
        if self.chat.r#type == ChatType::Channel
            && !ChatPermissionResolution::can_post_to_channel(self.chat.role.as_deref())
        {
            return false;
        }
        if self.message.can_retry_send {
            return true;
        }
        let Some(hooks) = chat_engine_hooks() else {
            return false;
        };
        (hooks.is_parked_draft_send)(&self.message.id, &self.chat.id, cx)
            || (hooks.can_repark_draft_send)(self.message, &self.chat.id, cx)
    }

    pub fn can_delete(&self, cx: &mut App) -> bool {
        self.can_delete_own(cx) || self.can_delete_private_peer(cx) || self.can_moderate_others(cx)
    }

    /// reaction strip follows add reactions checked in the message topic
    /// so a member who lost it doesnt get a strip that does nothing
    pub fn can_add_reactions(&self, cx: &mut App) -> bool {
        if self.is_disabled_by_moderation() {
            return false;
        }
        if self.chat.r#type != ChatType::Group && self.chat.r#type != ChatType::Channel {
            return true;
        }
        ChatModerationRights::holds_unless_denied(
            self.chat,
            ChatPerm::ADD_REACTIONS,
            self.message.topic_id.as_deref(),
            cx,
        )
    }

    /// saved foreign message looks incoming but lives in your own chat so theres nobody to report to
    /// a channel post is sent as the channel so is_own never marks the admin who wrote it
    pub fn can_report(&self, cx: &mut App) -> bool {
        !self.is_system()
            && !self.is_call()
            && self.has_stable_server_id(cx)
            && !self.is_own
            && self.chat.r#type != ChatType::Favourites
            && !(self.chat.r#type == ChatType::Channel && self.runs_channel(cx))
    }

    fn runs_channel(&self, cx: &mut App) -> bool {
        self.is_admin() || ChatModerationRights::allows(self.chat, ChatPerm::MANAGE_MESSAGES, cx)
    }

    /// a system or call row can only be deleted so its selectable exactly when the viewer may
    /// delete it
    pub fn can_select(&self, cx: &App) -> bool {
        if !self.has_stable_server_id(cx) {
            return false;
        }
        if self.is_system() || self.is_call() {
            return self.can_delete_for_me(cx);
        }
        true
    }

    pub fn has_forwardable_content(&self) -> bool {
        !self.is_system() && !self.is_call()
    }

    fn savable_media_items(&self) -> &'a [MessageMediaItem] {
        if self.is_system()
            || self.is_call()
            || self.protects_content()
            || !self.has_history_mutation_membership()
        {
            return &[];
        }
        &self.message.media_items
    }

    pub fn gallery_savable_items(&self) -> Vec<&'a MessageMediaItem> {
        self.savable_media_items()
            .iter()
            .filter(|item| MessageMediaSaveClassifier::target(item) == Some(MediaSaveTargetKind::Gallery))
            .collect()
    }

    pub fn file_savable_items(&self) -> Vec<&'a MessageMediaItem> {
        self.savable_media_items()
            .iter()
            .filter(|item| MessageMediaSaveClassifier::target(item) == Some(MediaSaveTargetKind::Files))
            .collect()
    }

    pub fn can_save_to_gallery(&self) -> bool {
        !self.gallery_savable_items().is_empty()
    }

    pub fn can_save_to_files(&self) -> bool {
        !self.file_savable_items().is_empty()
    }

    pub fn can_add_mp3_to_local_playlist(&self) -> bool {
        !self.is_system()
            && !self.protects_content()
            && self.has_history_mutation_membership()
            && self.message.first_importable_mp3_item().is_some()
    }

    fn is_editable_content(&self) -> bool {
        matches!(self.message.content_type, MessageContentType::Text | MessageContentType::Image)
    }

    pub fn copy_text(&self) -> &'a str {
        &self.message.text
    }

    /// copy link like telegram, group or channel message and only for members
    pub fn message_link(&self, cx: &App) -> Option<Url> {
        if self.chat.r#type != ChatType::Group && self.chat.r#type != ChatType::Channel {
            return None;
        }
        if self.is_system()
            || self.is_call()
            || !self.has_stable_server_id(cx)
            || self.guide_sandbox_owns_message(cx)
        {
            return None;
        }
        let topic_id = self
            .message
            .topic_id
            .as_deref()
            .map(trimming_whitespaces_and_newlines)
            .filter(|topic| !topic.is_empty())
            .map(str::to_owned);
        RiseonlyPublicLink::ChatMessage {
            chat_id: self.chat.id.clone(),
            message_id: self.message.sai_server_id(),
            topic_id,
        }
        .web_url(AppEnvironment::current())
    }

    /// includes_composer_actions drops items that need an input field
    /// without a composer reply and edit lead nowhere
    pub fn context_menu_actions(
        &self,
        includes_selection: bool,
        includes_composer_actions: bool,
        cx: &mut App,
    ) -> Vec<HoldContextMenuAction> {
        if self.is_disabled_by_moderation() {
            let mut exhibit = Vec::new();
            if self.can_copy() {
                exhibit.push(
                    HoldContextMenuAction::new(ChatMessageAction::COPY, tr("ctx_copy")).system_image("doc.on.doc"),
                );
            }
            if let Some(delete_row) = self.delete_row(cx) {
                exhibit.push(delete_row);
            }
            if includes_selection && self.can_select(cx) {
                exhibit.push(self.select_row());
            }
            return exhibit;
        }
        if self.is_call() || self.is_system() {
            let mut items = Vec::new();
            if let Some(delete_row) = self.delete_row(cx) {
                items.push(delete_row);
            }
            if includes_selection && self.can_select(cx) {
                items.push(self.select_row());
            }
            return items;
        }
        if self.can_cancel_sending() {
            let mut pending = Vec::new();
            if self.can_reply(cx) && includes_composer_actions {
                pending.push(
                    HoldContextMenuAction::new(ChatMessageAction::REPLY, tr("ctx_reply"))
                        .system_image("arrowshape.turn.up.left"),
                );
            }
            if self.can_copy() {
                pending.push(
                    HoldContextMenuAction::new(ChatMessageAction::COPY, tr("ctx_copy")).system_image("doc.on.doc"),
                );
            }
            if self.can_retry_sending(cx) {
                pending.push(
                    HoldContextMenuAction::new(ChatMessageAction::RETRY_SENDING, tr("ctx_retry_sending"))
                        .system_image("arrow.clockwise"),
                );
            }
            let cancel_title = if self.message.delivery_status == MessageDeliveryStatus::Failed {
                tr("ctx_delete")
            } else {
                tr("ctx_cancel_sending")
            };
            pending.push(
                HoldContextMenuAction::new(ChatMessageAction::CANCEL_SENDING, cancel_title)
                    .system_image("trash")
                    .destructive(true),
            );
            return pending;
        }
        let mut items = Vec::new();

        if self.can_reply(cx) && includes_composer_actions {
            items.push(
                HoldContextMenuAction::new(ChatMessageAction::REPLY, tr("ctx_reply"))
                    .system_image("arrowshape.turn.up.left"),
            );
        }
        if self.can_copy() {
            items.push(HoldContextMenuAction::new(ChatMessageAction::COPY, tr("ctx_copy")).system_image("doc.on.doc"));
        }
        if self.message_link(cx).is_some() {
            items.push(HoldContextMenuAction::new(ChatMessageAction::COPY_LINK, tr("ctx_copy_link")).system_image("link"));
        }
        if chat_shared_sparks_hooks().is_some_and(|hooks| (hooks.offers_send)(self.message, self.chat, self.is_own, cx)) {
            items.push(
                HoldContextMenuAction::new(ChatMessageAction::SEND_SPARKS, tr("sparks_send_title"))
                    .system_image("sparkle"),
            );
        }
        if !self.is_system() {
            items.push(
                HoldContextMenuAction::new(ChatMessageAction::FORWARD, tr("ctx_forward"))
                    .system_image("arrowshape.turn.up.right")
                    .disabled(!self.can_forward(cx)),
            );
        }
        if self.can_save_to_gallery() {
            items.push(
                HoldContextMenuAction::new(ChatMessageAction::SAVE_TO_GALLERY, tr("ctx_save"))
                    .system_image("square.and.arrow.down"),
            );
        }
        if self.can_save_to_files() {
            items.push(HoldContextMenuAction::new(ChatMessageAction::SAVE, tr("ctx_save")).system_image("arrow.down.doc"));
        }
        if self.can_add_mp3_to_local_playlist() {
            items.push(
                HoldContextMenuAction::new(ChatMessageAction::ADD_TO_LOCAL_PLAYLIST, tr("ctx_add_mp3_to_local_playlist"))
                    .system_image("text.badge.plus")
                    .guide_anchor(RiseGuideAnchor::ChatMenuAddToLocalPlaylist),
            );
        }
        if self.can_pin(cx) {
            if self.already_pinned() {
                items.push(HoldContextMenuAction::new(ChatMessageAction::UNPIN, tr("ctx_unpin")).system_image("pin.slash"));
            } else {
                items.push(
                    HoldContextMenuAction::new(ChatMessageAction::PIN, tr("ctx_pin"))
                        .system_image("pin")
                        .submenu(self.pin_submenu()),
                );
            }
        }
        if self.can_edit(cx) && includes_composer_actions {
            items.push(
                HoldContextMenuAction::new(ChatMessageAction::EDIT, tr("ctx_edit"))
                    .system_image("pencil")
                    .guide_anchor(RiseGuideAnchor::ChatMenuEdit),
            );
        }
        if self.can_report(cx) {
            items.push(
                HoldContextMenuAction::new(ChatMessageAction::REPORT, tr("ctx_report"))
                    .system_image("exclamationmark.bubble")
                    .destructive(true),
            );
        }
        if let Some(delete_row) = self.delete_row(cx) {
            items.push(delete_row);
        }
        if includes_selection && self.can_select(cx) {
            items.push(self.select_row());
        }
        items
    }

    fn select_row(&self) -> HoldContextMenuAction {
        HoldContextMenuAction::new(ChatMessageAction::SELECT, tr("ctx_select"))
            .system_image("checkmark.circle")
            .section_break(true)
    }

    fn pin_submenu(&self) -> Vec<HoldContextMenuAction> {
        if self.is_private_like() {
            return vec![
                HoldContextMenuAction::new(ChatMessageAction::PIN_EVERYONE, tr("ctx_pin_for_everyone"))
                    .system_image("pin"),
                HoldContextMenuAction::new(ChatMessageAction::PIN_ME, tr("ctx_pin_for_me")).system_image("pin"),
            ];
        }
        vec![
            HoldContextMenuAction::new(ChatMessageAction::PIN_EVERYONE_NOTIFY, tr("ctx_pin_notify"))
                .system_image("bell"),
            HoldContextMenuAction::new(ChatMessageAction::PIN_EVERYONE_SILENT, tr("ctx_pin_silent"))
                .system_image("bell.slash"),
        ]
    }

    fn sender_is_moderatable(&self, cx: &mut App) -> bool {
        if self.chat.r#type == ChatType::Channel {
            return false;
        }
        let sender_id = &self.message.sender_id;
        let creator_id = self.chat.creator_info_decoded().map(|creator| creator.id).or_else(|| {
            self.chat.creator_info.as_deref().map(|info| trimming_whitespaces_and_newlines(info).to_owned())
        });
        if creator_id.is_some_and(|id| !id.is_empty() && id == *sender_id) {
            return false;
        }
        let Some(member) = chat_engine_hooks().and_then(|hooks| (hooks.member)(&self.chat.id, sender_id, cx)) else {
            return true;
        };
        if member.is_owner() {
            return false;
        }
        if member.is_admin() {
            return self.is_owner();
        }
        true
    }

    fn delete_submenu_row(id: &str, title_key: &str) -> HoldContextMenuAction {
        HoldContextMenuAction::new(id, tr(title_key)).system_image("trash").destructive(true)
    }

    pub fn delete_row(&self, cx: &mut App) -> Option<HoldContextMenuAction> {
        if !self.can_delete_for_me(cx) {
            return None;
        }
        let row = HoldContextMenuAction::new(ChatMessageAction::DELETE, tr("ctx_delete"))
            .system_image("trash")
            .destructive(true);
        if self.chat.r#type == ChatType::Favourites {
            return Some(row.submenu(vec![
                HoldContextMenuAction::new(ChatMessageAction::DELETE_FROM_FAVOURITES, tr("ctx_delete_from_favourites"))
                    .system_image("bookmark.slash")
                    .destructive(true),
            ]));
        }
        if self.can_moderate_others(cx) {
            let everyone_id = if self.is_system() || !self.sender_is_moderatable(cx) {
                ChatMessageAction::DELETE_EVERYONE
            } else {
                ChatMessageAction::DELETE_MODERATE
            };
            return Some(row.submenu(vec![
                Self::delete_submenu_row(everyone_id, "ctx_delete_for_everyone"),
                Self::delete_submenu_row(ChatMessageAction::DELETE_ME, "ctx_delete_for_me"),
            ]));
        }
        if self.can_delete_own(cx) || self.can_delete_private_peer(cx) {
            return Some(row.submenu(vec![
                Self::delete_submenu_row(ChatMessageAction::DELETE_EVERYONE, "ctx_delete_for_everyone"),
                Self::delete_submenu_row(ChatMessageAction::DELETE_ME, "ctx_delete_for_me"),
            ]));
        }
        Some(row.submenu(vec![Self::delete_submenu_row(ChatMessageAction::DELETE_ME, "ctx_delete_for_me")]))
    }
}
