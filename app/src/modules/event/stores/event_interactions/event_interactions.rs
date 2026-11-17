use std::collections::HashSet;
use std::rc::Rc;

use gpui::{App, Context, Entity};
use rise_events::name;
use serde_json::Value;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::modules::sai_ws::sai_ws_event_registry::{
    SaiWsEventHandlerRegistration, SaiWsEventRegistry,
};
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_canon_events::{RiseCanonEventsParity, RiseWsEvents};
use crate::modules::auth::stores::auth_interactions::auth_interactions::AuthInteractionsStore;
use crate::modules::bot::stores::bot::bot_services::bot_services::BotServicesStore;
use crate::modules::chat::stores::chats::chat_actions::chat_types::OverwriteScope;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::chat::stores::chats::chat_services::chat_services::ChatServicesStore;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;
use crate::modules::moderation::stores::moderation_interactions::moderation_interactions::ModerationInteractionsStore;
use crate::modules::moderation::stores::moderation_services::moderation_services::ModerationServicesStore;
use crate::modules::music::stores::music_services::music_services::MusicServicesStore;
use crate::modules::post::stores::post_services::post_services::PostServicesStore;
use crate::modules::presence::stores::presence::presence_services::presence_services::PresenceServicesStore;
use crate::modules::story::stores::story::story_services::story_services::StoryServicesStore;
use crate::modules::user::stores::subscription::subscription_services::subscription_services::SubscriptionServicesStore;
use crate::modules::user::stores::user::user_services::user_services::UserServicesStore;

pub struct EventInteractionsStore {
    did_bootstrap: bool,
}

impl EventInteractionsStore {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, Self::new)
    }

    fn new(_cx: &mut Context<Self>) -> Self {
        Self {
            did_bootstrap: false,
        }
    }

    pub fn bootstrap(&mut self, cx: &mut Context<Self>) {
        if self.did_bootstrap {
            return;
        }
        self.did_bootstrap = true;
        cx.notify();

        let registrations: Vec<SaiWsEventHandlerRegistration> = vec![
            Self::registration(name::POST_REPOSTED, |payload, cx| {
                PostServicesStore::shared(cx)
                    .update(cx, |store, cx| store.post_reposted_handler(payload, cx));
            }),
            Self::registration(name::POST_DELETED, |payload, cx| {
                PostServicesStore::shared(cx)
                    .update(cx, |store, cx| store.post_deleted_event_handler(payload, cx));
            }),
            Self::registration(name::POST_UPDATED, |payload, cx| {
                PostServicesStore::shared(cx)
                    .update(cx, |store, cx| store.post_updated_event_handler(payload, cx));
            }),
            Self::registration(name::POST_MEDIA_READY, |payload, cx| {
                PostServicesStore::shared(cx)
                    .update(cx, |store, cx| store.post_media_ready_handler(payload, cx));
            }),
            Self::registration(name::MESSAGE_CREATED, |payload, cx| {
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.message_created_success_handler(payload, cx));
                MessageServicesStore::shared(cx)
                    .update(cx, |store, cx| store.message_created_thread_handler(payload, cx));
            }),
            Self::registration(name::MESSAGE_EDITED, |payload, cx| {
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.message_edited_success_handler(payload, cx));
                MessageServicesStore::shared(cx)
                    .update(cx, |store, cx| store.message_edited_thread_handler(payload, cx));
            }),
            Self::registration(name::MESSAGE_DELETED, |payload, cx| {
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.message_deleted_success_handler(payload, cx));
                MessageServicesStore::shared(cx)
                    .update(cx, |store, cx| store.message_deleted_thread_handler(payload, cx));
            }),
            Self::registration(name::CHAT_CREATED, |payload, cx| {
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.chat_created_success_handler(payload, cx));
            }),
            Self::registration(name::CHAT_ARCHIVED, |payload, cx| {
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.chat_archived_event_handler(payload, cx));
            }),
            Self::registration(name::CHAT_UNARCHIVED, |payload, cx| {
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.chat_unarchived_event_handler(payload, cx));
            }),
            Self::registration(name::MESSAGE_REQUEST_CREATED, |payload, cx| {
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.message_request_created_handler(payload, cx));
            }),
            Self::registration(name::MESSAGE_REQUEST_ACCEPTED, |payload, cx| {
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.message_request_accepted_handler(payload, cx));
            }),
            Self::registration(name::CALLBACK_QUERY_ANSWERED, |payload, cx| {
                BotServicesStore::shared(cx)
                    .update(cx, |store, cx| store.callback_query_answered_handler(payload, cx));
            }),
            Self::registration(name::CHAT_READ, |payload, cx| {
                MessageServicesStore::shared(cx)
                    .update(cx, |store, cx| store.chat_read_receipt_handler(payload, cx));
            }),
            Self::registration(name::CHAT_DELETED, |payload, cx| {
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.chat_deleted_event_handler(payload, cx));
            }),
            Self::registration(name::CHAT_UPDATED, |payload, cx| {
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.chat_updated_event_handler(payload, cx));
            }),
            Self::registration(name::MEMBER_LEFT, |payload, cx| {
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.member_left_event_handler(payload, cx));
            }),
            Self::registration(name::MEMBER_JOINED, |payload, cx| {
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.member_joined_event_handler(payload, cx));
            }),
            Self::registration(name::JOIN_REQUEST_CREATED, |payload, cx| {
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.join_request_created_handler(payload, cx));
            }),
            Self::registration(name::JOIN_REQUEST_APPROVED, |payload, cx| {
                ChatServicesStore::shared(cx).update(cx, |store, cx| {
                    store.join_request_processed_handler(payload, name::JOIN_REQUEST_APPROVED, cx)
                });
            }),
            Self::registration(name::JOIN_REQUEST_REJECTED, |payload, cx| {
                ChatServicesStore::shared(cx).update(cx, |store, cx| {
                    store.join_request_processed_handler(payload, name::JOIN_REQUEST_REJECTED, cx)
                });
            }),
            Self::registration(name::MESSAGE_TYPING, |payload, cx| {
                MessageServicesStore::shared(cx)
                    .update(cx, |store, cx| store.typing_handler(payload, cx));
            }),
            Self::registration(name::USER_ONLINE_STATUS_CHANGED, |payload, cx| {
                PresenceServicesStore::shared(cx)
                    .update(cx, |store, cx| store.apply_online_status(payload, cx));
            }),
            Self::registration(name::USER_PROFILE_UPDATED, |payload, cx| {
                let peer_user_id = UserServicesStore::shared(cx)
                    .update(cx, |store, cx| store.profile_updated_event_handler(payload, cx));
                if let Some(peer_user_id) = peer_user_id {
                    ChatInteractionsStore::shared(cx)
                        .update(cx, |store, cx| store.refresh_peer_chat(&peer_user_id, cx));
                }
            }),
            Self::registration(name::SUBSCRIPTION_CHANGED, |payload, cx| {
                SubscriptionServicesStore::shared(cx)
                    .update(cx, |store, cx| store.subscription_changed_event_handler(payload, cx));
            }),
            Self::registration(name::CONTACT_JOINED, |payload, cx| {
                UserServicesStore::shared(cx)
                    .update(cx, |store, cx| store.contact_joined_event_handler(payload, cx));
            }),
            Self::registration(name::CONTACT_PHOTO_SUGGESTED, |payload, cx| {
                UserServicesStore::shared(cx).update(cx, |store, cx| {
                    store.contact_photo_suggested_event_handler(payload, cx)
                });
            }),
            Self::registration(name::IN_CHAT_UPDATE, |payload, cx| {
                PresenceServicesStore::shared(cx)
                    .update(cx, |store, cx| store.apply_in_chat_update(payload, cx));
            }),
            Self::registration(name::MEDIA_PLAYED, |payload, cx| {
                MessageServicesStore::shared(cx)
                    .update(cx, |store, cx| store.media_played_handler(payload, cx));
            }),
            Self::registration(name::REACTION_ADDED, |payload, cx| {
                MessageServicesStore::shared(cx)
                    .update(cx, |store, cx| store.reaction_added_handler(payload, cx));
            }),
            Self::registration(name::REACTION_REMOVED, |payload, cx| {
                MessageServicesStore::shared(cx)
                    .update(cx, |store, cx| store.reaction_removed_handler(payload, cx));
            }),
            Self::registration(name::COMMENTS_COUNT_UPDATED, |payload, cx| {
                MessageServicesStore::shared(cx)
                    .update(cx, |store, cx| store.comments_count_updated_handler(payload, cx));
            }),
            Self::registration(name::TOPIC_CREATED, |payload, cx| {
                ChatServicesStore::shared(cx).update(cx, |store, cx| {
                    store.apply_topic_event_upsert(payload, name::TOPIC_CREATED, cx)
                });
            }),
            Self::registration(name::TOPIC_UPDATED, |payload, cx| {
                ChatServicesStore::shared(cx).update(cx, |store, cx| {
                    store.apply_topic_event_upsert(payload, name::TOPIC_UPDATED, cx)
                });
            }),
            Self::registration(name::TOPIC_DELETED, |payload, cx| {
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.apply_topic_deleted_event(payload, cx));
            }),
            Self::registration(name::TOPIC_OVERWRITE_CHANGED, |payload, cx| {
                ChatServicesStore::shared(cx).update(cx, |store, cx| {
                    store.apply_permission_overwrite_changed(payload, OverwriteScope::Topic, cx)
                });
            }),
            Self::registration(name::ROLE_CREATED, |p, cx| {
                ChatServicesStore::shared(cx).update(cx, |store, cx| {
                    store.apply_role_created_or_updated(p, name::ROLE_CREATED, cx)
                });
            }),
            Self::registration(name::ROLE_UPDATED, |p, cx| {
                ChatServicesStore::shared(cx).update(cx, |store, cx| {
                    store.apply_role_created_or_updated(p, name::ROLE_UPDATED, cx)
                });
            }),
            Self::registration(name::ROLE_DELETED, |p, cx| {
                ChatServicesStore::shared(cx).update(cx, |store, cx| store.apply_role_deleted(p, cx));
            }),
            Self::registration(name::ROLES_REORDERED, |p, cx| {
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.apply_roles_reordered(p, cx));
            }),
            Self::registration(name::MEMBER_ROLES_CHANGED, |p, cx| {
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.apply_member_roles_changed(p, cx));
            }),
            Self::registration(name::SECTION_CREATED, |p, cx| {
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.apply_section_event(p, name::SECTION_CREATED, cx));
            }),
            Self::registration(name::SECTION_UPDATED, |p, cx| {
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.apply_section_event(p, name::SECTION_UPDATED, cx));
            }),
            Self::registration(name::SECTION_DELETED, |p, cx| {
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.apply_section_event(p, name::SECTION_DELETED, cx));
            }),
            Self::registration(name::SECTIONS_REORDERED, |p, cx| {
                ChatServicesStore::shared(cx).update(cx, |store, cx| {
                    store.apply_section_event(p, name::SECTIONS_REORDERED, cx)
                });
            }),
            Self::registration(name::SECTION_OVERWRITE_CHANGED, |payload, cx| {
                ChatServicesStore::shared(cx).update(cx, |store, cx| {
                    store.apply_permission_overwrite_changed(payload, OverwriteScope::Section, cx)
                });
            }),
            Self::registration(name::REPORT_RESOLVED, |p, cx| {
                ModerationInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.handle_report_resolved(p, cx));
            }),
            Self::registration(name::REPORT_REOPENED, |p, cx| {
                ModerationInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.handle_report_reopened(p, cx));
            }),
            Self::registration(name::USER_SANCTIONED, |_, cx| {
                ModerationServicesStore::shared(cx)
                    .update(cx, |store, cx| store.user_sanctioned_event_handler(cx));
                ModerationInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.handle_user_sanctioned(cx));
            }),
            Self::registration(name::USER_WARNED, |_, cx| {
                ModerationInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.handle_user_warned(cx));
            }),
            Self::registration(name::APPEAL_RESOLVED, |_, cx| {
                ModerationInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.handle_appeal_resolved(cx));
            }),
            Self::registration(name::ACCOUNT_BLOCKED, |_, cx| {
                AuthInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.handle_account_blocked(cx));
            }),
            Self::registration(name::ACCOUNT_UNBLOCKED, |_, cx| {
                AuthInteractionsStore::shared(cx)
                    .update(cx, |store, cx| store.reconcile_account_block(cx));
            }),
            Self::registration(name::CONTENT_DISABLED, |p, cx| {
                ModerationServicesStore::shared(cx)
                    .update(cx, |store, cx| store.content_disabled_event_handler(p, cx));
            }),
            Self::registration(name::CONTENT_RESTORED, |p, cx| {
                ModerationServicesStore::shared(cx)
                    .update(cx, |store, cx| store.content_restored_event_handler(p, cx));
            }),
            Self::registration(name::MODERATOR_GRANTED, |_, cx| {
                ModerationServicesStore::shared(cx)
                    .update(cx, |store, cx| store.moderator_granted_event_handler(cx));
            }),
            Self::registration(name::MODERATOR_PERMISSIONS_UPDATED, |_, cx| {
                ModerationServicesStore::shared(cx)
                    .update(cx, |store, cx| store.moderator_rights_changed_event_handler(cx));
            }),
            Self::registration(name::MODERATOR_REMOVED, |_, cx| {
                ModerationServicesStore::shared(cx)
                    .update(cx, |store, cx| store.moderator_removed_event_handler(cx));
            }),
            Self::playlist_collaboration(name::PLAYLIST_JOIN_REQUEST_CREATED),
            Self::playlist_collaboration(name::PLAYLIST_JOIN_APPROVED),
            Self::playlist_collaboration(name::PLAYLIST_JOIN_REJECTED),
            Self::playlist_collaboration(name::PLAYLIST_JOIN_REQUEST_REVIEWED),
            Self::playlist_collaboration(name::PLAYLIST_MEMBER_PERMISSIONS_UPDATED),
            Self::playlist_collaboration(name::PLAYLIST_ACCESS_REVOKED),
            Self::playlist_collaboration(name::PLAYLIST_ACCESS_SUSPENDED),
            Self::playlist_collaboration(name::PLAYLIST_ACCESS_RESTORED),
            Self::playlist_collaboration(name::PLAYLIST_CONTENT_CHANGED),
            Self::registration(name::LOCAL_TRACK_CREATE_COMPLETED, |payload, cx| {
                MusicServicesStore::shared(cx).update(cx, |store, cx| {
                    store.local_track_create_completed_event_handler(payload, cx)
                });
            }),
            Self::playlist_collaboration(name::PLAYLIST_LISTENING_PARTICIPANT_JOINED),
            Self::playlist_collaboration(name::PLAYLIST_LISTENING_PERMISSION_REQUESTED),
            Self::playlist_collaboration(name::PLAYLIST_LISTENING_SESSION_ENDED),
            Self::playlist_collaboration(name::PLAYLIST_LISTENING_PERMISSIONS_UPDATED),
            Self::playlist_collaboration(name::PLAYLIST_LISTENING_PARTICIPANT_LEFT),
            Self::playlist_realtime(name::PLAYLIST_PRESENCE_DELTA),
            Self::playlist_realtime(name::LISTENING_PARTICIPANT_JOINED),
            Self::playlist_realtime(name::LISTENING_SESSION_STATE),
            Self::playlist_realtime(name::LISTENING_PERMISSION_REQUESTED),
            Self::playlist_realtime(name::LISTENING_PERMISSIONS_UPDATED),
            Self::playlist_realtime(name::LISTENING_SESSION_ENDED),
            Self::playlist_realtime(name::LISTENING_PARTICIPANT_LEFT),
            Self::registration(name::STORY_CREATED, |p, cx| {
                StoryServicesStore::shared(cx)
                    .update(cx, |store, cx| store.story_created_ring_handler(p, cx));
            }),
            Self::registration(name::STORY_DELETED, |p, cx| {
                StoryServicesStore::shared(cx)
                    .update(cx, |store, cx| store.story_deleted_ring_handler(p, cx));
            }),
            Self::registration(name::STORY_VIEWED, |p, cx| {
                StoryServicesStore::shared(cx)
                    .update(cx, |store, cx| store.story_viewed_ring_handler(p, cx));
            }),
        ];

        let registered: HashSet<String> =
            registrations.iter().map(|registration| registration.r#type.clone()).collect();
        SaiWsEventRegistry::shared(cx).update(cx, |registry, cx| registry.register(registrations, cx));
        RiseWsEvents::report_registration_drift(&registered, cx);
        RiseCanonEventsParity::verify(cx);

        Sai::log("Event", || "registered chat event handlers", SaiLogKind::Websocket, cx);
    }

    fn registration(
        r#type: &'static str,
        handler: impl Fn(Option<&Value>, &mut App) + 'static,
    ) -> SaiWsEventHandlerRegistration {
        SaiWsEventHandlerRegistration {
            r#type: r#type.to_owned(),
            handler: Rc::new(handler),
        }
    }

    fn playlist_collaboration(event: &'static str) -> SaiWsEventHandlerRegistration {
        Self::registration(event, move |payload, cx| {
            MusicServicesStore::shared(cx).update(cx, |store, cx| {
                store.playlist_collaboration_event_handler(payload, event, cx)
            });
        })
    }

    fn playlist_realtime(event: &'static str) -> SaiWsEventHandlerRegistration {
        Self::registration(event, move |payload, cx| {
            MusicServicesStore::shared(cx)
                .update(cx, |store, cx| store.playlist_realtime_event_handler(payload, event, cx));
        })
    }
}
