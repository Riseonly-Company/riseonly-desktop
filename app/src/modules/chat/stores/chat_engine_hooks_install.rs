use gpui::App;

use crate::modules::chat::engine::chat_engine_hooks::{
    ChatEngineHooks, ChatEngineWarmCacheLimits, set_chat_engine_hooks,
};
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;
use crate::modules::chat::stores::message::message_services::message_services::{
    MessageHistoryWarmCacheLimits, MessageHistoryWarmCachePolicy, MessageServicesStore,
};

pub(crate) fn install_chat_engine_hooks() {
    set_chat_engine_hooks(ChatEngineHooks {
        get_chat_by_id_action: |chat_id, force, cx| {
            ChatActionsStore::shared(cx).update(cx, |store, cx| store.get_chat_by_id_action(chat_id, force, cx));
        },
        get_chat_settings_action: |chat_id, cx| {
            ChatActionsStore::shared(cx).update(cx, |store, cx| store.get_chat_settings_action(chat_id, cx));
        },
        get_forum_structure_action: |chat_id, force, on_loaded, on_failed, cx| {
            ChatActionsStore::shared(cx).update(cx, |store, cx| {
                store.get_forum_structure_action(chat_id, force, on_loaded, on_failed, cx);
            });
        },
        get_topics_action: |chat_id, force_refresh, cx| {
            ChatActionsStore::shared(cx).update(cx, |store, cx| store.get_topics_action(chat_id, force_refresh, cx));
        },
        load_forum_voice_roster_action: |chat_id, topic_id, cx| {
            ChatActionsStore::shared(cx).update(cx, |store, cx| store.load_forum_voice_roster_action(chat_id, topic_id, cx));
        },

        announce_removed_from_chat: |chat_id, stored, cx| {
            ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.announce_removed_from_chat(chat_id, stored.map(Into::into), cx);
            });
        },
        load_effective_permissions: |chat_id, topic_id, force, cx| {
            ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                store.load_effective_permissions(chat_id, topic_id, force, cx);
            });
        },

        adopt_known_chats_for_draft_direct_chats: |cx| {
            ChatServicesStore::shared(cx).update(cx, |store, cx| store.adopt_known_chats_for_draft_direct_chats(cx));
        },
        chat_info: |chat_id, cx| {
            let store = ChatServicesStore::shared(cx);
            let app: &App = cx;
            store.read(app).chat_info(chat_id, app)
        },
        known_direct_chat_id: |peer_user_id, cx| {
            let store = ChatServicesStore::shared(cx);
            let app: &App = cx;
            store.read(app).known_direct_chat_id(peer_user_id, app)
        },
        member: |scope, user_id, cx| {
            ChatServicesStore::shared(cx).update(cx, |store, cx| store.member(scope, user_id, cx))
        },
        apply_member_fields_local: |chat_id, user_id, fields, cx| {
            ChatServicesStore::shared(cx)
                .update(cx, |store, cx| store.apply_member_fields_local(chat_id, user_id, fields, cx));
        },
        refresh_loaded_member_lists: |chat_id, cx| {
            ChatServicesStore::shared(cx).update(cx, |store, cx| store.refresh_loaded_member_lists(chat_id, cx));
        },
        remove_member_local: |chat_id, user_id, reason, add_to_blocked, cx| {
            ChatServicesStore::shared(cx).update(cx, |store, cx| {
                store.remove_member_local(chat_id, user_id, reason, add_to_blocked, cx);
            });
        },
        effective_permissions: |chat_id, topic_id, cx| {
            ChatServicesStore::shared(cx).update(cx, |store, cx| store.effective_permissions(chat_id, topic_id, cx))
        },
        invalidate_effective_permissions: |chat_id, cx| {
            ChatServicesStore::shared(cx).update(cx, |store, cx| store.invalidate_effective_permissions(chat_id, cx));
        },
        invalidate_overwrite_caches: |chat_id, cx| {
            ChatServicesStore::shared(cx).update(cx, |store, cx| store.invalidate_overwrite_caches(chat_id, cx));
        },
        preserve_official_bot_after_history_clear: |chat_id, cx| {
            ChatServicesStore::shared(cx)
                .update(cx, |store, cx| store.preserve_official_bot_after_history_clear(chat_id, cx));
        },
        record_own_server_send: |chat_id, sent_at, cx| {
            ChatServicesStore::shared(cx).update(cx, |store, cx| store.record_own_server_send(chat_id, sent_at, cx));
        },
        settings_by_chat: |chat_id, cx| {
            let store = ChatServicesStore::shared(cx);
            let app: &App = cx;
            store.read(app).chat_settings(chat_id, app).cloned()
        },
        live_voice_change: ChatServicesStore::live_voice_change,
        apply_topic_deleted_event: |any, cx| {
            ChatServicesStore::shared(cx).update(cx, |store, cx| store.apply_topic_deleted_event(any.as_ref(), cx))
        },
        apply_topic_message_created: |chat_id, topic_id, last_message, updated_at, increment_unread, cx| {
            ChatServicesStore::shared(cx).update(cx, |store, cx| {
                store.apply_topic_message_created(chat_id, topic_id, last_message, updated_at, increment_unread, cx)
            })
        },
        apply_topic_message_deleted: |chat_id, topic_id, message_id, cx| {
            ChatServicesStore::shared(cx)
                .update(cx, |store, cx| store.apply_topic_message_deleted(chat_id, topic_id, message_id, cx))
        },
        apply_topic_message_edited: |chat_id, topic_id, message_id, last_message, cx| {
            ChatServicesStore::shared(cx).update(cx, |store, cx| {
                store.apply_topic_message_edited(chat_id, topic_id, message_id, last_message, cx);
            });
        },
        apply_topic_message_send_failed: |chat_id, local_id, failed_row, cx| {
            ChatServicesStore::shared(cx)
                .update(cx, |store, cx| store.apply_topic_message_send_failed(chat_id, local_id, failed_row, cx));
        },
        apply_topic_read_local: |chat_id, topic_id, cx| {
            ChatServicesStore::shared(cx).update(cx, |store, cx| store.apply_topic_read_local(chat_id, topic_id, cx));
        },
        apply_topic_receipt: |chat_id, server_ids, cx| {
            ChatServicesStore::shared(cx).update(cx, |store, cx| store.apply_topic_receipt(chat_id, server_ids, cx));
        },
        apply_topic_unread_marks_delta: |chat_id, topic_id, mentions, replies, reactions, cx| {
            ChatServicesStore::shared(cx).update(cx, |store, cx| {
                store.apply_topic_unread_marks_delta(chat_id, topic_id, mentions, replies, reactions, cx)
            })
        },
        forum_structure_cache_key: ChatServicesStore::forum_structure_cache_key,
        member_scope_key: ChatServicesStore::member_scope_key,
        members_cache_key: ChatServicesStore::members_cache_key,
        topics_cache_key: ChatServicesStore::topics_cache_key,
        is_joined_member_role: ChatServicesStore::is_joined_member_role,
        effective_perms_cache_key: ChatServicesStore::effective_perms_cache_key,

        apply_thread_total: |discussion_message_id, count, cx| {
            MessageServicesStore::shared(cx)
                .update(cx, |store, cx| store.apply_thread_total(discussion_message_id, count, cx));
        },
        clear_local_history: |chat_id, scopes, cx| {
            MessageServicesStore::shared(cx).update(cx, |store, cx| store.clear_local_history(chat_id, scopes, cx));
        },
        record_confirmed_message_deletions: |chat_id, message_ids, cx| {
            MessageServicesStore::shared(cx)
                .update(cx, |store, cx| store.record_confirmed_message_deletions(chat_id, message_ids, cx));
        },
        typing_peers_by_chat: |cx| MessageServicesStore::shared(cx).read(cx).typing_peers_by_chat.clone(),
        set_typing_peers_by_chat: |key, value, cx| {
            MessageServicesStore::shared(cx).update(cx, |store, cx| {
                match value {
                    Some(peers) => store.typing_peers_by_chat.insert(key.to_owned(), peers),
                    None => store.typing_peers_by_chat.remove(key),
                };
                cx.notify();
            });
        },
        remove_all_typing_peers_by_chat: |cx| {
            MessageServicesStore::shared(cx).update(cx, |store, cx| {
                store.typing_peers_by_chat.clear();
                cx.notify();
            });
        },
        chat_overlay_message_search_state: |query, chat_kind, cx| {
            MessageServicesStore::shared(cx)
                .update(cx, |store, cx| store.chat_overlay_message_search_state(query, chat_kind, cx))
        },
        message_scope: MessageServicesStore::message_scope,
        audio_tab_entries: |chat_id, cx| {
            MessageServicesStore::shared(cx).update(cx, |store, cx| store.audio_tab_entries(chat_id, cx))
        },
        fav_tab_entries: |chat_id, cx| {
            MessageServicesStore::shared(cx).update(cx, |store, cx| store.fav_tab_entries(chat_id, cx))
        },
        file_tab_entries: |chat_id, cx| {
            MessageServicesStore::shared(cx).update(cx, |store, cx| store.file_tab_entries(chat_id, cx))
        },
        gif_tab_entries: |chat_id, cx| {
            MessageServicesStore::shared(cx).update(cx, |store, cx| store.gif_tab_entries(chat_id, cx))
        },
        link_tab_entries: |chat_id, cx| {
            MessageServicesStore::shared(cx).update(cx, |store, cx| store.link_tab_entries(chat_id, cx))
        },
        media_tab_entries: |chat_id, cx| {
            MessageServicesStore::shared(cx).update(cx, |store, cx| store.media_tab_entries(chat_id, cx))
        },
        music_tab_entries: |chat_id, cx| {
            MessageServicesStore::shared(cx).update(cx, |store, cx| store.music_tab_entries(chat_id, cx))
        },
        message: |message_id, chat_id, cx| {
            MessageServicesStore::shared(cx).update(cx, |store, cx| store.message(message_id, chat_id, cx))
        },
        messages: |chat_id, cx| MessageServicesStore::shared(cx).update(cx, |store, cx| store.messages(chat_id, cx)),
        warm_cache_limits_production: || {
            let limits = MessageHistoryWarmCacheLimits::production();
            ChatEngineWarmCacheLimits {
                maximum_entry_count: limits.maximum_entry_count,
                maximum_estimated_cost: limits.maximum_estimated_cost,
                time_to_live: limits.time_to_live,
            }
        },
        estimated_resident_cost: MessageHistoryWarmCachePolicy::estimated_resident_cost,

        can_repark_draft_send: |message, chat_id, cx| {
            MessageInteractionsStore::shared(cx).read(cx).can_repark_draft_send(message, chat_id)
        },
        is_parked_draft_send: |local_id, chat_id, cx| {
            MessageInteractionsStore::shared(cx).read(cx).is_parked_draft_send(local_id, chat_id)
        },
    });
}
