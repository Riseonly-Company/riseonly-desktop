use gpui::App;

use crate::modules::auth::services::account_state_reset_hooks::{
    AccountStateResetChatHooks, set_account_state_reset_chat_hooks,
};
use crate::modules::chat::components::chat::backdrop::chat_wallpaper_store::ChatWallpaperStore;
use crate::modules::chat::components::chat::composer::chat_draft_store::ChatDraftStore;
use crate::modules::chat::components::chat::input_picker::sticker_hold_preview::StickerHoldPreviewPresenter;
use crate::modules::chat::components::chat::scroll::chat_scroll_position_store::ChatScrollPositionStore;
use crate::modules::chat::components::chat::video_note::video_note_player_store::VideoNotePlayerStore;
use crate::modules::chat::components::chat::voice::chat_voice_message_player::ChatVoiceMessagePlayer;
use crate::modules::chat::components::chat_search::chat_search_overlay_presenter::ChatSearchOverlayPresenter;
use crate::modules::chat::engine::core::rise_conversation_presentation::RiseConversationPresentation;
use crate::modules::chat::engine::messages::rise_message_read_receipt_presentation::RiseMessageReadReceiptPresentation;
use crate::modules::chat::shared::chat_message_jump_presenter::ChatMessageJumpPresenter;
use crate::modules::chat::shared::utils::voice::voice_message_disk_cache::VoiceMessageDiskCache;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::chat::widgets::bottomsheets::create_chat_sheet::create_chat_sheet_state::CreateChatSheetState;

pub fn install_account_state_reset_chat_hooks() {
    set_account_state_reset_chat_hooks(AccountStateResetChatHooks {
        activate_account_state: |cx: &mut App| {
            ChatScrollPositionStore::activate_account_state(cx);
            ChatDraftStore::shared(cx).update(cx, |store, cx| store.activate_account_state(cx));
            ChatWallpaperStore::shared(cx).update(cx, |store, cx| store.activate_account_state(cx));
        },
        fence_account_transition: |cx: &mut App| {
            RiseConversationPresentation::shared(cx).update(cx, |presentation, cx| presentation.deactivate(cx));
            RiseMessageReadReceiptPresentation::shared(cx).update(cx, |presentation, cx| presentation.deactivate(cx));
        },
        reset_account_state: |preserve_account_disk: bool, cx: &mut App| {
            ChatServicesStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
            MessageServicesStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
            ChatSearchOverlayPresenter::shared(cx).update(cx, |presenter, cx| presenter.reset_account_state(cx));
            CreateChatSheetState::shared(cx).update(cx, |state, cx| state.reset_account_state(cx));
            StickerHoldPreviewPresenter::reset_account_state(cx);
            MessageInteractionsStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
            ChatVoiceMessagePlayer::shared(cx).update(cx, |player, cx| player.reset_account_state(cx));
            VideoNotePlayerStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
            ChatMessageJumpPresenter::shared(cx).update(cx, |presenter, cx| presenter.reset_account_state(cx));
            ChatWallpaperStore::shared(cx).update(cx, |store, cx| store.reset_account_state(cx));
            ChatScrollPositionStore::reset_account_state(preserve_account_disk, cx);
            ChatDraftStore::shared(cx).update(cx, |store, cx| store.reset_account_state(preserve_account_disk, cx));
        },
        remove_persisted_account: |user_id: &str, cx: &mut App| {
            ChatScrollPositionStore::remove_persisted_account(user_id);
            ChatDraftStore::shared(cx).update(cx, |store, cx| store.remove_persisted_account(user_id, cx));
            VoiceMessageDiskCache::remove_persisted_account(user_id);
            ChatWallpaperStore::shared(cx).read(cx).remove_persisted_account(user_id);
        },
    });
}
