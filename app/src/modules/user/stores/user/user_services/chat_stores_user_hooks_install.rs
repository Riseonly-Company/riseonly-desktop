use std::collections::HashSet;

use gpui::App;

use crate::modules::chat::stores::chat_stores_hooks::{ChatStoresUserHooks, set_chat_stores_user_hooks};
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

pub fn install_chat_stores_user_hooks() {
    set_chat_stores_user_hooks(ChatStoresUserHooks {
        restore_contact_join_dialogs_from_inbox: |cx: &mut App| {
            UserServicesStore::shared(cx).update(cx, |services, cx| services.restore_contact_join_dialogs_from_inbox(cx));
        },
        note_peer_refuses_viewer_messages: |user_id: &str, cx: &mut App| {
            UserServicesStore::shared(cx)
                .update(cx, |services, cx| services.note_peer_refuses_viewer_messages(user_id, cx));
        },
        is_contact_join_virtual_chat_id: UserServicesStore::is_contact_join_virtual_chat_id,
        saved_contacts_list_cache_key: UserServicesStore::saved_contacts_list_cache_key,
        chat_mute_override_ids: |cx: &App| -> HashSet<String> {
            rise_ui::shared::try_read::<UserServicesStore>(cx)
                .map(|services| services.chat_mute_override_ids().clone())
                .unwrap_or_default()
        },
        mute_all_chats_enabled: |cx: &App| {
            rise_ui::shared::try_read::<UserServicesStore>(cx).is_some_and(|services| services.mute_all_chats_enabled())
        },
        add_chat_mute_exception: |chat_id: &str, cx: &mut App| {
            UserInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.add_chat_mute_exception(chat_id, cx));
        },
        remove_chat_mute_exception: |chat_id: &str, cx: &mut App| {
            UserInteractionsStore::shared(cx)
                .update(cx, |interactions, cx| interactions.remove_chat_mute_exception(chat_id, cx));
        },
        hydrate_visible_chat_participants_for_contact_folders: |cx: &mut App| {
            UserInteractionsStore::shared(cx)
                .update(cx, |interactions, cx| interactions.hydrate_visible_chat_participants_for_contact_folders(cx));
        },
        load_saved_contacts_for_chat_folders: |cx: &mut App| {
            UserInteractionsStore::shared(cx)
                .update(cx, |interactions, cx| interactions.load_saved_contacts_for_chat_folders(cx));
        },
    });
}
