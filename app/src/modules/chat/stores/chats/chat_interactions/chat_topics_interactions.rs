use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::LazyLock;
use std::time::Duration;

use gpui::{App, AsyncApp, Context, Global, Task, WeakEntity};
use rise_canon::rise_key::chat as key;
use rise_canon::rise_response::chat as response;
use rise_canon::rise_sync::{RETRY_BASE_MS, RETRY_CEILING_MS, RETRY_EXPONENT_MAXIMUM, RETRY_MAXIMUM_ATTEMPTS};
use rise_canon::{RiseGuideSignal, RiseRequestBody, RiseTimingCanon, rise_request, rise_wire_methods};
use rise_core::foundation::foundation_text::trimming_whitespaces_and_newlines;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;
use rise_i18n::tr;
use serde_json::{Map, Value};
use rise_core::foundation::foundation_uuid::Uuid;

use super::chat_interactions::ChatInteractionsStore;
use crate::core::guides::guide_signal_bus::GuideSignalBus;
use crate::core::lib::canon_timing::RiseTimingCanonExt;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::modules::sai_debouncer::sai_debouncer::SaiDebouncer;
use crate::core::lib::sai::modules::sai_updater::sai_updater::SaiUpdaterWriteScope;
use crate::core::lib::sai::sai::Sai;
use crate::core::rise_engine_bridge::rise_transport_status::RiseTransportStatus;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::auth::stores::auth_services::auth_services::AuthServicesStore;
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::chat::engine::forum::forum_voice_subscription_registry::ForumVoiceSubscriptionRegistry;
use crate::modules::chat::engine::forum::rise_chat_voice_presence::RiseChatVoicePresentation;
use crate::modules::chat::engine::messages::rise_message_unread_marks::RiseUnreadMarkKind;
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomainRegistry;
use crate::modules::chat::shared::chat_forum_ordering_policy::{ChatForumOrderingPolicy, ChatForumTopicPlacement};
use crate::modules::chat::shared::chat_notifications_debounce::ChatNotificationsDebounce;
use crate::modules::chat::stores::chat_guide_sandbox_hooks::chat_guide_sandbox_hooks;
use crate::modules::chat::stores::chat_stores_hooks::chat_stores_rtc_hooks;
use crate::modules::chat::stores::chats::chat_actions::chat_actions::ChatActionsStore;
use crate::modules::chat::stores::chats::chat_actions::chat_topics_actions::{ChatTopicCallback, ChatTopicSuccess};
use crate::modules::chat::stores::chats::chat_actions::chat_types::{
    ChatChannelKind, ChatInfo, ChatMuteScopes, ChatPerm, ChatTopic, VoiceChannelConfig,
};
use crate::modules::chat::stores::chats::chat_actions::chat_wire_types::RiseStoredChatTopic;
use crate::modules::chat::stores::chats::chat_services::chat_services::{
    ChatServicesStore, ChatServicesStoreTopicNotificationCommand,
};
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;

const TOPIC_NOTIFICATIONS_MAX_CACHE_DATA: i64 = 50;

#[derive(Default)]
struct DroppedTopicPreviewLedger {
    attempts: HashMap<String, usize>,
    scheduled: HashSet<String>,
}

impl Global for DroppedTopicPreviewLedger {}

#[derive(Default)]
struct GroupVoiceSeedTasks {
    tasks: HashMap<String, Task<()>>,
}

impl Global for GroupVoiceSeedTasks {}

static DROPPED_PREVIEW_RECHECK_DELAYS_MS: LazyLock<Vec<i64>> = LazyLock::new(|| {
    let ladder = RiseTimingCanon::FRESHNESS_LADDER_MS;
    let mut delays: Vec<i64> = ladder.iter().take(3).copied().collect();
    let Some(cadence) = delays.last().copied() else {
        return delays;
    };
    if ladder.len() <= 3 || cadence <= 0 {
        return delays;
    }
    while delays.iter().sum::<i64>() < ladder[3] {
        delays.push(cadence);
    }
    delays
});

fn effective_user_id(cx: &mut App) -> Option<String> {
    AuthServicesStore::shared(cx).read(cx).effective_user_id_for_wire()
}

fn guide_owns_forum(chat_id: &str, cx: &App) -> bool {
    (chat_guide_sandbox_hooks().owns_forum_chat_id)(chat_id, cx)
}

fn author_is_premium(cx: &mut App) -> bool {
    AuthServicesStore::shared(cx)
        .read(cx)
        .profile()
        .is_some_and(|profile| profile.more.is_premium)
}

fn report(on_result: &Option<Rc<dyn Fn(bool, &mut App)>>, success: bool, cx: &mut App) {
    if let Some(on_result) = on_result.clone() {
        cx.defer(move |cx| on_result(success, cx));
    }
}

fn create_topic_channel_kind(kind: ChatChannelKind) -> rise_request::chat::create_topic::ChannelKind {
    match kind {
        ChatChannelKind::Text => rise_request::chat::create_topic::ChannelKind::Text,
        ChatChannelKind::Voice => rise_request::chat::create_topic::ChannelKind::Voice,
        ChatChannelKind::Stage => rise_request::chat::create_topic::ChannelKind::Stage,
    }
}

fn update_topic_channel_kind(kind: ChatChannelKind) -> rise_request::chat::update_topic::ChannelKind {
    match kind {
        ChatChannelKind::Text => rise_request::chat::update_topic::ChannelKind::Text,
        ChatChannelKind::Voice => rise_request::chat::update_topic::ChannelKind::Voice,
        ChatChannelKind::Stage => rise_request::chat::update_topic::ChannelKind::Stage,
    }
}

fn topic_notifications_debounce_id(chat_id: &str, topic_id: &str) -> String {
    format!("{chat_id}:topic:{topic_id}")
}

fn recheck_dropped_topic_previews(chat_id: &str, cx: &mut App) {
    let services = ChatServicesStore::shared(cx);
    if !services.update(cx, |store, cx| store.forum_topic_previews_look_dropped(chat_id, cx)) {
        cx.default_global::<DroppedTopicPreviewLedger>().attempts.remove(chat_id);
        return;
    }
    let ledger = cx.default_global::<DroppedTopicPreviewLedger>();
    let attempt = ledger.attempts.get(chat_id).copied().unwrap_or(0);
    let Some(delay_ms) = DROPPED_PREVIEW_RECHECK_DELAYS_MS.get(attempt).copied() else {
        return;
    };
    if !ledger.scheduled.insert(chat_id.to_owned()) {
        return;
    }
    ledger.attempts.insert(chat_id.to_owned(), attempt + 1);
    let account = effective_user_id(cx);
    let chat_id = chat_id.to_owned();
    cx.spawn(async move |cx: &mut AsyncApp| {
        cx.background_executor().timer(Duration::from_millis(delay_ms as u64)).await;
        cx.update(|cx| {
            cx.default_global::<DroppedTopicPreviewLedger>().scheduled.remove(&chat_id);
            let services = ChatServicesStore::shared(cx);
            if effective_user_id(cx) != account
                || !services.update(cx, |store, cx| store.forum_topic_previews_look_dropped(&chat_id, cx))
            {
                return;
            }
            let on_loaded: Rc<dyn Fn(&mut App)> = {
                let chat_id = chat_id.clone();
                Rc::new(move |cx: &mut App| recheck_dropped_topic_previews(&chat_id, cx))
            };
            ChatActionsStore::shared(cx).update(cx, |actions, cx| {
                actions.get_forum_structure_action(&chat_id, true, Some(on_loaded), None, cx);
                actions.get_topics_action(&chat_id, false, cx);
            });
        });
    })
    .detach();
}

async fn seed_group_voice(chat_id: String, cx: &mut AsyncApp) {
    let mut attempt: u64 = 0;
    loop {
        let load = cx.update(|cx| {
            let registry = ForumVoiceSubscriptionRegistry::shared(cx);
            if !registry.read(cx).observes_group(&chat_id) {
                return None;
            }
            Some(chat_stores_rtc_hooks().map(|hooks| (hooks.group_voice_settings_load)(chat_id.clone(), cx)))
        });
        let Some(load) = load else {
            break;
        };
        let loaded = match load {
            Some(load) => load.await,
            None => false,
        };
        if loaded {
            break;
        }
        attempt += 1;
        if attempt >= RETRY_MAXIMUM_ATTEMPTS {
            break;
        }
        let exponent = (attempt - 1).min(RETRY_EXPONENT_MAXIMUM);
        let delay_ms = (RETRY_BASE_MS << exponent).min(RETRY_CEILING_MS);
        cx.background_executor().timer(Duration::from_millis(delay_ms)).await;
    }
    cx.update(|cx| {
        cx.default_global::<GroupVoiceSeedTasks>().tasks.remove(&chat_id);
    });
}

impl ChatInteractionsStore {
    pub fn load_forum_home(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty() || guide_owns_forum(chat_id, cx) {
            return;
        }

        let is_unknown = ChatServicesStore::shared(cx)
            .update(cx, |store, cx| store.chat_info(chat_id, cx))
            .is_none();
        if is_unknown {
            ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.get_chat_by_id_action(chat_id, false, cx));
        }
        cx.default_global::<DroppedTopicPreviewLedger>().attempts.remove(chat_id);
        let on_loaded: Rc<dyn Fn(&mut App)> = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |cx: &mut App| recheck_dropped_topic_previews(&chat_id, cx))
        };
        ChatActionsStore::shared(cx)
            .update(cx, |actions, cx| actions.get_forum_structure_action(chat_id, false, Some(on_loaded), None, cx));
        self.load_effective_permissions(chat_id, None, false, cx);
        self.load_chat_settings(chat_id, cx);
        self.load_topics(chat_id, cx);
    }

    pub fn load_topics(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty() {
            return;
        }

        ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.get_topics_action(chat_id, false, cx));
    }

    pub fn verify_opened_topic(&mut self, chat_id: &str, topic_id: &str, retry_on_reconnect: bool, cx: &mut Context<Self>) {
        if chat_id.is_empty()
            || topic_id.is_empty()
            || topic_id == ChatTopic::GENERAL_WALLPAPER_SCOPE_ID
            || guide_owns_forum(chat_id, cx)
        {
            return;
        }
        let Some(account) = effective_user_id(cx) else {
            return;
        };
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let watch_key = format!("{chat_id}|{topic_id}");
        if let Some(watch) = self.topic_verify_reconnect_watches.remove(&watch_key) {
            watch.stop();
        }

        let on_loaded: Rc<dyn Fn(&mut App)> = {
            let chat_id = chat_id.to_owned();
            let topic_id = topic_id.to_owned();
            Rc::new(move |cx: &mut App| {
                if effective_user_id(cx).as_deref() != Some(account.as_str()) {
                    return;
                }
                let services = ChatServicesStore::shared(cx);
                let Some(readable) = services.update(cx, |store, cx| store.in_memory_forum_readable_topic_ids(&chat_id, cx))
                else {
                    return;
                };
                if readable.contains(&topic_id) {
                    return;
                }
                let before = HashSet::from([topic_id.clone()]);
                services.update(cx, |store, cx| store.note_lost_forum_topics(&chat_id, before, HashSet::new(), cx));
            })
        };
        let on_failed: Rc<dyn Fn(&mut App)> = {
            let chat_id = chat_id.to_owned();
            let topic_id = topic_id.to_owned();
            Rc::new(move |cx: &mut App| {
                if !retry_on_reconnect
                    || !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
                {
                    return;
                }
                let chat_id = chat_id.clone();
                let topic_id = topic_id.clone();
                let watch_key = watch_key.clone();
                let account_user_id = account_user_id.clone();
                let account_epoch = account_epoch.clone();
                cx.defer(move |cx| {
                    ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.retry_opened_topic_verification(chat_id, topic_id, watch_key, account_user_id, account_epoch, cx);
                    });
                });
            })
        };
        ChatActionsStore::shared(cx)
            .update(cx, |actions, cx| actions.get_forum_structure_action(chat_id, true, Some(on_loaded), Some(on_failed), cx));
    }

    fn retry_opened_topic_verification(
        &mut self,
        chat_id: String,
        topic_id: String,
        watch_key: String,
        account_user_id: Option<String>,
        account_epoch: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if RiseTransportStatus::is_connected(cx) {
            self.verify_opened_topic(&chat_id, &topic_id, false, cx);
            return;
        }
        let handler_key = watch_key.clone();
        let watch = RiseTransportStatus::on_next_connect(
            move |cx: &mut App| {
                cx.defer(move |cx| {
                    ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                        if store.topic_verify_reconnect_watches.remove(&handler_key).is_none()
                            || !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
                        {
                            return;
                        }
                        store.verify_opened_topic(&chat_id, &topic_id, false, cx);
                    });
                });
            },
            cx,
        );
        self.topic_verify_reconnect_watches.insert(watch_key, watch);
        RiseTransportStatus::wake_if_offline(cx);
    }

    pub fn begin_observing_voice_channels(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty() || guide_owns_forum(chat_id, cx) {
            return;
        }

        let topics = ChatServicesStore::shared(cx).update(cx, |store, cx| store.in_memory_forum_topics(chat_id, cx));
        let voice_topic_ids: Vec<String> = topics
            .iter()
            .filter(|topic| topic.is_rtc_channel() && !topic.id.is_empty())
            .map(|topic| topic.id.clone())
            .collect();
        if voice_topic_ids.is_empty() {
            return;
        }

        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.set_forum_voice_subscription_action(chat_id, voice_topic_ids, true, false, cx);
            actions.adopt_forum_voice_state_action(chat_id, &topics, cx);
        });
    }

    pub fn begin_observing_group_voice(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty() {
            return;
        }

        ChatActionsStore::shared(cx)
            .update(cx, |actions, cx| actions.set_forum_voice_subscription_action(chat_id, Vec::new(), true, true, cx));
        let seed_chat_id = chat_id.to_owned();
        let seed = cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            seed_group_voice(seed_chat_id, cx).await;
        });
        cx.default_global::<GroupVoiceSeedTasks>().tasks.insert(chat_id.to_owned(), seed);
    }

    pub fn end_observing_group_voice(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty() {
            return;
        }

        let live_while_observing = RiseChatVoicePresentation::shared(cx).read(cx).is_live(chat_id);
        let actions = ChatActionsStore::shared(cx);
        actions.update(cx, |actions, cx| actions.set_forum_voice_subscription_action(chat_id, Vec::new(), false, false, cx));
        cx.default_global::<GroupVoiceSeedTasks>().tasks.remove(chat_id);
        actions.update(cx, |actions, cx| actions.forget_chat_voice_presence_action(chat_id, cx));
        if live_while_observing == Some(false) {
            actions.update(cx, |actions, cx| actions.get_chat_by_id_action(chat_id, true, cx));
        }
    }

    pub fn mark_self_in_voice_channel(&mut self, chat_id: &str, topic_id: &str, joined: bool, cx: &mut Context<Self>) {
        if chat_id.is_empty() {
            return;
        }

        ChatActionsStore::shared(cx)
            .update(cx, |actions, cx| actions.set_forum_voice_self_presence_action(chat_id, topic_id, joined, cx));
    }

    pub fn expand_voice_channel_preview(&mut self, chat_id: &str, topic_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty() || topic_id.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.expand_forum_voice_preview_action(chat_id, topic_id, cx));
    }

    pub fn collapse_voice_channel_preview(&mut self, chat_id: &str, topic_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty() || topic_id.is_empty() {
            return;
        }
        Self::actions(cx).update(cx, |actions, cx| actions.collapse_forum_voice_preview_action(chat_id, topic_id, cx));
    }

    pub fn end_observing_voice_channels(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        if chat_id.is_empty() || guide_owns_forum(chat_id, cx) {
            return;
        }

        let live_while_observing = RiseChatVoicePresentation::shared(cx).read(cx).is_live(chat_id);
        let actions = ChatActionsStore::shared(cx);
        actions.update(cx, |actions, cx| actions.forget_chat_voice_presence_action(chat_id, cx));
        if live_while_observing == Some(false) {
            actions.update(cx, |actions, cx| actions.get_chat_by_id_action(chat_id, true, cx));
        }
        actions.update(cx, |actions, cx| actions.set_forum_voice_subscription_action(chat_id, Vec::new(), false, false, cx));
    }

    pub fn mark_topic_read(&mut self, chat_id: &str, topic: &ChatTopic, cx: &mut Context<Self>) {
        let topic_id = if topic.is_general { "" } else { topic.id.as_str() };
        let had_mentions = topic.unread_mentions_count > 0;
        let had_replies = topic.unread_replies_count > 0;
        let had_reactions = topic.unread_reactions_count > 0;

        let services = ChatServicesStore::shared(cx);
        services.update(cx, |store, cx| {
            store.apply_topic_read_local(chat_id, topic_id, cx);
            tracing::warn!(
                target: "riseonly::X2254",
                chat = chat_id,
                topic = topic_id,
                unread = topic.unread_count,
                mentions = topic.unread_mentions_count,
                replies = topic.unread_replies_count,
                reactions = topic.unread_reactions_count,
                "mark topic read"
            );
            store.apply_topic_unread_marks_delta(
                chat_id,
                topic_id,
                -topic.unread_mentions_count,
                -topic.unread_replies_count,
                -topic.unread_reactions_count,
                cx,
            );
        });
        let messages = MessageInteractionsStore::shared(cx);
        messages.update(cx, |store, cx| {
            if let Some(task) = store.mark_chat_as_read(chat_id, Some(topic_id), false, cx) {
                task.detach();
            }
            if had_mentions {
                store.read_all_unread_marks(chat_id, Some(topic_id), RiseUnreadMarkKind::Mention, cx);
            }
            if had_replies {
                store.read_all_unread_marks(chat_id, Some(topic_id), RiseUnreadMarkKind::Reply, cx);
            }
            if had_reactions {
                store.read_all_unread_marks(chat_id, Some(topic_id), RiseUnreadMarkKind::Reaction, cx);
            }
        });
    }

    pub fn mark_topic_unread(&mut self, chat_id: &str, topic: &ChatTopic, cx: &mut Context<Self>) {
        let topic_id = if topic.is_general { "" } else { topic.id.as_str() };
        ChatServicesStore::shared(cx).update(cx, |store, cx| store.set_topic_forced_unread(chat_id, topic_id, true, cx));
    }

    pub fn topic_opened(&mut self, chat_id: &str, topic_id: Option<&str>, cx: &mut Context<Self>) {
        let Some(topic_id) = topic_id else {
            return;
        };
        ChatServicesStore::shared(cx).update(cx, |store, cx| store.set_topic_forced_unread(chat_id, topic_id, false, cx));
    }

    pub fn set_all_topics_view_hidden(&mut self, chat_id: &str, hidden: bool, cx: &mut Context<Self>) {
        let services = ChatServicesStore::shared(cx);
        if services.update(cx, |store, cx| store.all_topics_view_hidden(chat_id, cx)) == hidden {
            return;
        }
        let account_user_id = effective_user_id(cx);

        services.update(cx, |store, cx| store.set_all_topics_view_hidden_override(chat_id, Some(hidden), cx));
        let on_success = {
            let chat_id = chat_id.to_owned();
            let account_user_id = account_user_id.clone();
            move |cx: &mut App| {
                if effective_user_id(cx) != account_user_id {
                    return;
                }
                ChatServicesStore::shared(cx).update(cx, |store, cx| store.apply_all_topics_view_hidden(&chat_id, hidden, cx));
            }
        };
        let on_error = {
            let chat_id = chat_id.to_owned();
            move |cx: &mut App| {
                if effective_user_id(cx) != account_user_id {
                    return;
                }
                ChatServicesStore::shared(cx).update(cx, |store, cx| store.set_all_topics_view_hidden_override(&chat_id, None, cx));
                Notifier::send(
                    tr("chat_topics_hide_all_error"),
                    NotifierSendOptions {
                        r#type: NotifierType::Error,
                        position: NotifierPosition::Bottom,
                        ..NotifierSendOptions::default()
                    },
                    cx,
                );
            }
        };
        ChatActionsStore::shared(cx)
            .update(cx, |actions, cx| actions.set_hide_all_topics_action(chat_id, hidden, on_success, on_error, cx));
    }

    pub fn enable_topics(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.set_topics_enabled(chat_id, true, cx);
    }

    pub fn disable_topics(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.set_topics_enabled(chat_id, false, cx);
    }

    pub fn create_topic(
        &mut self,
        chat_id: &str,
        title: &str,
        icon: &str,
        section_id: Option<&str>,
        channel_kind: ChatChannelKind,
        voice_config: Option<VoiceChannelConfig>,
        on_created: Option<Rc<dyn Fn(ChatTopic, &mut App)>>,
        on_result: Option<Rc<dyn Fn(bool, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let Some(user_id) = effective_user_id(cx).filter(|user_id| !user_id.is_empty()) else {
            report(&on_result, false, cx);
            return;
        };
        if chat_id.is_empty() {
            report(&on_result, false, cx);
            return;
        }

        let normalized_title = trimming_whitespaces_and_newlines(title).to_owned();
        if normalized_title.is_empty() {
            report(&on_result, false, cx);
            return;
        }
        let services = ChatServicesStore::shared(cx);
        if !services.update(cx, |store, cx| store.can_manage_channel(chat_id, None, channel_kind, cx)) {
            report(&on_result, false, cx);
            return;
        }

        let resolved_icon = ChatTopic::selectable_icon(Some(icon)).unwrap_or_else(|| ChatTopic::DEFAULT_ICON.to_owned());
        let reporting_created: Rc<dyn Fn(ChatTopic, &mut App)> = Rc::new(move |topic: ChatTopic, cx: &mut App| {
            GuideSignalBus::emit(RiseGuideSignal::TopicCreated);
            if let Some(on_created) = &on_created {
                on_created(topic, cx);
            }
        });
        if channel_kind.is_rtc() {
            self.create_rtc_topic(
                chat_id,
                &normalized_title,
                &resolved_icon,
                section_id,
                channel_kind,
                voice_config,
                reporting_created,
                on_result,
                cx,
            );
            return;
        }
        let topic_id = format!("local-topic-{}", Uuid::new_v4().uuid_string());
        services.update(cx, |store, cx| {
            store.insert_optimistic_topic(chat_id, &topic_id, &normalized_title, Some(&resolved_icon), section_id, &user_id, cx);
        });
        let on_success: ChatTopicSuccess = {
            let chat_id = chat_id.to_owned();
            let topic_id = topic_id.clone();
            let on_result = on_result.clone();
            Rc::new(move |wire: RiseStoredChatTopic, cx: &mut App| {
                tracing::warn!(target: "riseonly::X1046", chat = %chat_id, local = %topic_id, topic = %wire.topic_id, "create topic success");
                ChatServicesStore::shared(cx).update(cx, |store, cx| {
                    store.remove_topic(&chat_id, &topic_id, SaiUpdaterWriteScope::All, cx);
                    store.insert_topic(&wire, &chat_id, cx);
                });
                reporting_created(wire.to_topic(), cx);
                report(&on_result, true, cx);
            })
        };
        let on_error: ChatTopicCallback = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |cx: &mut App| {
                tracing::warn!(target: "riseonly::X1046", chat = %chat_id, local = %topic_id, "create topic error, optimistic row removed");
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.remove_topic(&chat_id, &topic_id, SaiUpdaterWriteScope::All, cx));
                report(&on_result, false, cx);
            })
        };
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.create_topic_action(
                chat_id,
                &normalized_title,
                Some(&resolved_icon),
                section_id,
                Some(on_success),
                Some(on_error),
                cx,
            );
        });
    }

    pub fn create_section(
        &mut self,
        chat_id: &str,
        name: &str,
        name_entities: Option<&str>,
        on_result: Option<Rc<dyn Fn(bool, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let reporting: Rc<dyn Fn(bool, &mut App)> = Rc::new(move |created: bool, cx: &mut App| {
            if created {
                GuideSignalBus::emit(RiseGuideSignal::SectionCreated);
            }
            if let Some(on_result) = &on_result {
                on_result(created, cx);
            }
        });
        ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.create_section_action(chat_id, name, name_entities, Some(reporting), cx));
    }

    pub fn delete_section(&mut self, chat_id: &str, section_id: &str, cx: &mut Context<Self>) {
        ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.delete_section_action(chat_id, section_id, cx));
    }

    pub fn rename_section(
        &mut self,
        chat_id: &str,
        section_id: &str,
        name: &str,
        name_entities: Option<&str>,
        on_result: Option<Rc<dyn Fn(bool, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let normalized = trimming_whitespaces_and_newlines(name);
        let can_manage =
            ChatServicesStore::shared(cx).update(cx, |store, cx| store.can(chat_id, ChatPerm::MANAGE_SECTIONS, cx));
        if normalized.is_empty() || !can_manage {
            report(&on_result, false, cx);
            return;
        }

        ChatActionsStore::shared(cx)
            .update(cx, |actions, cx| actions.update_section_action(chat_id, section_id, normalized, name_entities, on_result, cx));
    }

    pub fn reorder_sections(&mut self, chat_id: &str, section_ids: &[String], cx: &mut Context<Self>) {
        let services = ChatServicesStore::shared(cx);
        if !services.update(cx, |store, cx| store.can(chat_id, ChatPerm::MANAGE_SECTIONS, cx)) {
            return;
        }

        let known: HashSet<String> = services
            .update(cx, |store, cx| store.sections(chat_id, cx))
            .into_iter()
            .map(|section| section.id)
            .collect();
        let mut seen = HashSet::new();
        let ordered: Vec<String> = section_ids
            .iter()
            .filter(|id| known.contains(*id) && seen.insert((*id).clone()))
            .cloned()
            .collect();
        if ordered.is_empty() || ordered.len() as i64 > ChatForumOrderingPolicy::maximum_reorder_ids() {
            return;
        }

        ChatActionsStore::shared(cx).update(cx, |actions, cx| actions.reorder_sections_action(chat_id, ordered, cx));
    }

    pub fn reorder_topics(&mut self, chat_id: &str, placements: &[ChatForumTopicPlacement], cx: &mut Context<Self>) {
        let services = ChatServicesStore::shared(cx);
        let current = services.update(cx, |store, cx| store.forum_manageable_topics(chat_id, cx));
        let mut by_id: HashMap<String, ChatTopic> = HashMap::with_capacity(current.len());
        for topic in current {
            by_id.entry(topic.id.clone()).or_insert(topic);
        }
        let changed = ChatForumOrderingPolicy::changed_placements(placements, &by_id);
        if changed.is_empty() {
            return;
        }
        let limit = ChatForumOrderingPolicy::maximum_reorder_ids();
        let changed_count = changed.len() as i64;
        if changed_count > limit {
            Sai::log(
                "Chat",
                move || {
                    format!(
                        "reorder_topics: {changed_count} moved rows exceed the {limit} ceiling, {} left unsent",
                        changed_count - limit
                    )
                },
                SaiLogKind::Network,
                cx,
            );
        }

        for placement in changed.iter().take(limit.max(0) as usize) {
            let Some(topic) = by_id.get(&placement.topic_id) else {
                continue;
            };
            if topic.is_general
                || !services.update(cx, |store, cx| store.can_manage_channel(chat_id, Some(&topic.id), topic.channel_kind, cx))
            {
                continue;
            }
            let section_id = if topic.section_id == placement.section_id {
                None
            } else {
                Some(placement.section_id.as_str())
            };
            self.update_topic_state(
                chat_id,
                &placement.topic_id,
                None,
                None,
                None,
                section_id,
                Some(placement.position),
                None,
                cx,
            );
        }
    }

    pub fn delete_topic(
        &mut self,
        chat_id: &str,
        topic_id: &str,
        on_result: Option<Rc<dyn Fn(bool, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let services = ChatServicesStore::shared(cx);
        let can_delete = services.update(cx, |store, cx| {
            store
                .topic_from_any_cache(chat_id, topic_id, cx)
                .is_some_and(|current| store.can_manage_channel(chat_id, Some(topic_id), current.channel_kind, cx))
        });
        if !can_delete {
            report(&on_result, false, cx);
            return;
        }
        let snapshot = services.update(cx, |store, cx| store.topic_raw_rows(chat_id, topic_id, cx));
        if snapshot.0.is_none() && snapshot.1.is_none() {
            report(&on_result, false, cx);
            return;
        }

        services.update(cx, |store, cx| store.remove_topic(chat_id, topic_id, SaiUpdaterWriteScope::Cache, cx));
        let on_success: ChatTopicCallback = {
            let chat_id = chat_id.to_owned();
            let topic_id = topic_id.to_owned();
            let on_result = on_result.clone();
            Rc::new(move |cx: &mut App| {
                ChatServicesStore::shared(cx).update(cx, |store, cx| {
                    store.remove_topic(&chat_id, &topic_id, SaiUpdaterWriteScope::All, cx);
                    store.note_own_deleted_topic(&chat_id, &topic_id, cx);
                });
                if let Some(domain) = RiseConversationDomainRegistry::shared(cx).read(cx).active() {
                    let chat_id = chat_id.clone();
                    let topic_id = topic_id.clone();
                    drop(RiseAccountRuntime::current().spawn(async move {
                        domain.revalidate_topic_scoped_preview(&chat_id, Some(topic_id.as_str()), true).await;
                    }));
                }
                report(&on_result, true, cx);
            })
        };
        let on_error: ChatTopicCallback = {
            let chat_id = chat_id.to_owned();
            let topic_id = topic_id.to_owned();
            Rc::new(move |cx: &mut App| {
                let snapshot = snapshot.clone();
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.restore_topic_rows(snapshot, &chat_id, &topic_id, SaiUpdaterWriteScope::All, cx));
                report(&on_result, false, cx);
            })
        };
        ChatActionsStore::shared(cx)
            .update(cx, |actions, cx| actions.delete_topic_action(chat_id, topic_id, Some(on_success), Some(on_error), cx));
    }

    pub fn set_topic_closed(&mut self, chat_id: &str, topic_id: &str, closed: bool, cx: &mut Context<Self>) {
        let can_manage = ChatServicesStore::shared(cx).update(cx, |store, cx| {
            store
                .topic_from_any_cache(chat_id, topic_id, cx)
                .is_some_and(|current| store.can_manage_channel(chat_id, Some(topic_id), current.channel_kind, cx))
        });
        if !can_manage {
            return;
        }
        self.update_topic_state(chat_id, topic_id, Some(closed), None, None, None, None, None, cx);
    }

    pub fn update_topic(
        &mut self,
        chat_id: &str,
        topic_id: &str,
        title: Option<&str>,
        icon: Option<&str>,
        section_id: Option<&str>,
        voice_config: Option<VoiceChannelConfig>,
        on_result: Option<Rc<dyn Fn(bool, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let services = ChatServicesStore::shared(cx);
        let Some(current) = services.update(cx, |store, cx| store.topic_from_any_cache(chat_id, topic_id, cx)) else {
            report(&on_result, false, cx);
            return;
        };
        if !services.update(cx, |store, cx| store.can_manage_channel(chat_id, Some(topic_id), current.channel_kind, cx)) {
            report(&on_result, false, cx);
            return;
        }
        if current.is_rtc_channel() {
            self.update_rtc_topic(
                chat_id,
                topic_id,
                title,
                icon,
                section_id,
                current.channel_kind,
                voice_config.or_else(|| current.voice_config.clone()),
                on_result,
                cx,
            );
            return;
        }
        self.update_topic_state(chat_id, topic_id, None, title, icon, section_id, None, on_result, cx);
    }

    fn create_rtc_topic(
        &mut self,
        chat_id: &str,
        title: &str,
        icon: &str,
        section_id: Option<&str>,
        channel_kind: ChatChannelKind,
        voice_config: Option<VoiceChannelConfig>,
        on_created: Rc<dyn Fn(ChatTopic, &mut App)>,
        on_result: Option<Rc<dyn Fn(bool, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let is_premium = author_is_premium(cx);
        let config = voice_config
            .or_else(|| VoiceChannelConfig::defaults(channel_kind))
            .and_then(|config| config.normalized(channel_kind).ok())
            .map(|config| config.capped_for_author(is_premium));
        let Some(config) = config else {
            report(&on_result, false, cx);
            return;
        };
        let mut body = rise_request::chat::CreateTopic::new(chat_id.to_owned(), title.to_owned());
        body.channel_kind = Some(create_topic_channel_kind(channel_kind));
        body.icon = Some(icon.to_owned());
        body.section_id = section_id.map(str::to_owned);
        body.voice_config = Some(rise_request::chat::create_topic::VoiceConfig {
            audio_bitrate_kbps: Some(config.audio_bitrate_kbps),
            default_muted: Some(config.default_muted),
            region: Some(config.region.clone()),
            screen_share_enabled: Some(config.screen_share_enabled),
            speaker_limit: Some(config.speaker_limit),
            user_limit: Some(config.user_limit),
            video_enabled: Some(config.video_enabled),
        });
        let chat_id = chat_id.to_owned();
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = RiseConversationActionExecutor::call::<_, response::CreateTopic>(
                rise_wire_methods::chat::SERVICE,
                rise_request::chat::CreateTopic::ACTION,
                &body,
                cx,
            )
            .await;
            cx.update(|cx| match result.ok().and_then(|response| response.topic) {
                Some(topic) => {
                    let wire = RiseStoredChatTopic::from_canon(&topic);
                    ChatServicesStore::shared(cx).update(cx, |store, cx| store.insert_topic(&wire, &chat_id, cx));
                    on_created(wire.to_topic(), cx);
                    report(&on_result, true, cx);
                }
                None => report(&on_result, false, cx),
            });
        })
        .detach();
    }

    fn update_rtc_topic(
        &mut self,
        chat_id: &str,
        topic_id: &str,
        title: Option<&str>,
        icon: Option<&str>,
        section_id: Option<&str>,
        channel_kind: ChatChannelKind,
        voice_config: Option<VoiceChannelConfig>,
        on_result: Option<Rc<dyn Fn(bool, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        if effective_user_id(cx).is_none_or(|user_id| user_id.is_empty()) {
            report(&on_result, false, cx);
            return;
        }
        let config = if channel_kind.is_rtc() {
            let normalized = voice_config
                .or_else(|| VoiceChannelConfig::defaults(channel_kind))
                .and_then(|config| config.normalized(channel_kind).ok());
            let Some(normalized) = normalized else {
                report(&on_result, false, cx);
                return;
            };
            Some(normalized.capped_for_author(author_is_premium(cx)))
        } else {
            None
        };
        let mut body = rise_request::chat::UpdateTopic::new(chat_id.to_owned(), topic_id.to_owned());
        body.channel_kind = Some(update_topic_channel_kind(channel_kind));
        body.title = title.map(str::to_owned);
        body.icon = icon.map(str::to_owned);
        body.section_id = section_id.map(str::to_owned);
        body.voice_config = config.map(|config| rise_request::chat::update_topic::VoiceConfig {
            audio_bitrate_kbps: Some(config.audio_bitrate_kbps),
            default_muted: Some(config.default_muted),
            region: Some(config.region.clone()),
            screen_share_enabled: Some(config.screen_share_enabled),
            speaker_limit: Some(config.speaker_limit),
            user_limit: Some(config.user_limit),
            video_enabled: Some(config.video_enabled),
        });
        let chat_id = chat_id.to_owned();
        cx.spawn(async move |_: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = RiseConversationActionExecutor::call::<_, response::UpdateTopic>(
                rise_wire_methods::chat::SERVICE,
                rise_request::chat::UpdateTopic::ACTION,
                &body,
                cx,
            )
            .await;
            cx.update(|cx| match result.ok().and_then(|response| response.topic) {
                Some(topic) => {
                    let wire = RiseStoredChatTopic::from_canon(&topic);
                    ChatServicesStore::shared(cx).update(cx, |store, cx| store.replace_topic(&wire, &chat_id, cx));
                    report(&on_result, true, cx);
                }
                None => report(&on_result, false, cx),
            });
        })
        .detach();
    }

    pub fn update_topic_notifications_debounced(
        &mut self,
        chat_id: &str,
        topic_id: &str,
        scopes: ChatMuteScopes,
        inherit: bool,
        on_result: Option<Rc<dyn Fn(bool, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let action_chat_id = chat_id.to_owned();
        let action_topic_id = topic_id.to_owned();
        SaiDebouncer::debounced_action_delay_milliseconds(
            topic_notifications_debounce_id(chat_id, topic_id),
            Box::new(move |cx: &mut App| {
                cx.defer(move |cx| {
                    ChatInteractionsStore::shared(cx).update(cx, |store, cx| {
                        store.update_topic_notifications(&action_chat_id, &action_topic_id, scopes, inherit, on_result, cx);
                    });
                });
            }),
            RiseTimingCanon::debounce("chat_topic_notifications").client_debounce_ms,
            ChatNotificationsDebounce::GROUP_KEY,
            TOPIC_NOTIFICATIONS_MAX_CACHE_DATA,
            None,
            cx,
        );
    }

    pub fn flush_topic_notifications(&mut self, chat_id: &str, topic_id: &str, cx: &mut Context<Self>) {
        SaiDebouncer::flush(
            topic_notifications_debounce_id(chat_id, topic_id),
            ChatNotificationsDebounce::GROUP_KEY,
            cx,
        );
    }

    fn set_topics_enabled(&mut self, chat_id: &str, enabled: bool, cx: &mut Context<Self>) {
        let services = ChatServicesStore::shared(cx);
        let (was_enabled_overridden, was_disabled_overridden) = services.update(cx, |store, cx| {
            let was_enabled_overridden = store.topics_enabled_override.contains(chat_id);
            let was_disabled_overridden = store.topics_disabled_override.contains(chat_id);
            if enabled {
                store.topics_enabled_override.insert(chat_id.to_owned());
                store.topics_disabled_override.remove(chat_id);
            } else {
                store.topics_enabled_override.remove(chat_id);
                store.topics_disabled_override.insert(chat_id.to_owned());
            }
            cx.notify();
            (was_enabled_overridden, was_disabled_overridden)
        });

        let on_success: ChatTopicCallback = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |cx: &mut App| {
                ChatServicesStore::shared(cx).update(cx, |store, cx| {
                    if !store.set_archived_topics_enabled_local(&chat_id, enabled, cx) {
                        if !store.current_chat_infos(cx).iter().any(|chat| chat.id == chat_id) {
                            return;
                        }
                        store.mutate_chat(&chat_id, |chat: &mut ChatInfo| chat.topics_enabled = enabled, cx);
                    }
                    store.topics_enabled_override.remove(&chat_id);
                    store.topics_disabled_override.remove(&chat_id);
                    cx.notify();
                });
            })
        };
        let on_error: ChatTopicCallback = {
            let chat_id = chat_id.to_owned();
            Rc::new(move |cx: &mut App| {
                ChatServicesStore::shared(cx).update(cx, |store, cx| {
                    if was_enabled_overridden {
                        store.topics_enabled_override.insert(chat_id.clone());
                    } else {
                        store.topics_enabled_override.remove(&chat_id);
                    }
                    if was_disabled_overridden {
                        store.topics_disabled_override.insert(chat_id.clone());
                    } else {
                        store.topics_disabled_override.remove(&chat_id);
                    }
                    cx.notify();
                });
            })
        };
        ChatActionsStore::shared(cx)
            .update(cx, |actions, cx| actions.set_topics_enabled_action(chat_id, enabled, Some(on_success), Some(on_error), cx));
    }

    fn update_topic_state(
        &mut self,
        chat_id: &str,
        topic_id: &str,
        is_closed: Option<bool>,
        title: Option<&str>,
        icon: Option<&str>,
        section_id: Option<&str>,
        position: Option<i64>,
        on_result: Option<Rc<dyn Fn(bool, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let normalized_icon = match icon {
            Some(icon) => match ChatTopic::selectable_icon(Some(icon)) {
                Some(selectable_icon) => Some(selectable_icon),
                None => {
                    report(&on_result, false, cx);
                    return;
                }
            },
            None => None,
        };

        let services = ChatServicesStore::shared(cx);
        let snapshot = services.update(cx, |store, cx| store.topic_raw_rows(chat_id, topic_id, cx));
        if snapshot.0.is_none() && snapshot.1.is_none() {
            report(&on_result, false, cx);
            return;
        }

        let mut patch: Map<String, Value> = Map::new();
        if let Some(is_closed) = is_closed {
            patch.insert(key::IS_CLOSED.to_owned(), Value::Bool(is_closed));
        }
        if let Some(title) = title {
            let normalized = trimming_whitespaces_and_newlines(title);
            if !normalized.is_empty() {
                patch.insert(key::TITLE.to_owned(), Value::String(normalized.to_owned()));
            }
        }
        if let Some(normalized_icon) = &normalized_icon {
            patch.insert(key::ICON.to_owned(), Value::String(normalized_icon.clone()));
        }
        if let Some(section_id) = section_id {
            patch.insert(key::SECTION_ID.to_owned(), Value::String(section_id.to_owned()));
        }
        if let Some(position) = position.filter(|position| *position >= 0) {
            patch.insert(key::POSITION.to_owned(), Value::from(position));
        }
        if !patch.is_empty() {
            services.update(cx, |store, cx| store.patch_topic(chat_id, topic_id, patch, cx));
        }

        let on_success: ChatTopicSuccess = {
            let chat_id = chat_id.to_owned();
            let on_result = on_result.clone();
            Rc::new(move |wire: RiseStoredChatTopic, cx: &mut App| {
                ChatServicesStore::shared(cx).update(cx, |store, cx| store.replace_topic(&wire, &chat_id, cx));
                report(&on_result, true, cx);
            })
        };
        let on_error: ChatTopicCallback = {
            let chat_id = chat_id.to_owned();
            let topic_id = topic_id.to_owned();
            Rc::new(move |cx: &mut App| {
                let snapshot = snapshot.clone();
                ChatServicesStore::shared(cx)
                    .update(cx, |store, cx| store.restore_topic_rows(snapshot, &chat_id, &topic_id, SaiUpdaterWriteScope::All, cx));
                report(&on_result, false, cx);
            })
        };
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.update_topic_action(
                chat_id,
                topic_id,
                is_closed,
                title,
                normalized_icon.as_deref(),
                section_id,
                position,
                Some(on_success),
                Some(on_error),
                cx,
            );
        });
    }

    fn update_topic_notifications(
        &mut self,
        chat_id: &str,
        topic_id: &str,
        scopes: ChatMuteScopes,
        inherit: bool,
        on_result: Option<Rc<dyn Fn(bool, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let command = ChatServicesStoreTopicNotificationCommand::new(
            chat_id.to_owned(),
            topic_id.to_owned(),
            scopes,
            inherit,
            on_result,
        );
        let enqueued = ChatServicesStore::shared(cx).update(cx, |store, cx| store.enqueue_topic_notification_command(command.clone(), cx));
        if !enqueued {
            Self::apply_topic_notification_patch(&command, cx);
            return;
        }

        self.dispatch_topic_notification_command(command, cx);
    }

    fn dispatch_topic_notification_command(&mut self, command: ChatServicesStoreTopicNotificationCommand, cx: &mut Context<Self>) {
        let chat_id = command.chat_id.clone();
        let topic_id = command.topic_id.clone();
        let snapshot = ChatServicesStore::shared(cx).update(cx, |store, cx| store.topic_raw_rows(&chat_id, &topic_id, cx));
        if snapshot.0.is_none() && snapshot.1.is_none() {
            self.finish_topic_notification_command(command, false, cx);
            return;
        }

        Self::apply_topic_notification_patch(&command, cx);

        let on_success: ChatTopicSuccess = {
            let command = command.clone();
            let chat_id = chat_id.clone();
            Rc::new(move |wire: RiseStoredChatTopic, cx: &mut App| {
                let services = ChatServicesStore::shared(cx);
                let (accepted, next) = services.update(cx, |store, cx| store.complete_topic_notification_command(&command, cx));
                if !accepted {
                    return;
                }

                services.update(cx, |store, cx| store.replace_topic(&wire, &chat_id, cx));
                Self::continue_topic_notification_commands(next, &command, true, cx);
            })
        };
        let on_error: ChatTopicCallback = {
            let command = command.clone();
            let chat_id = chat_id.clone();
            let topic_id = topic_id.clone();
            Rc::new(move |cx: &mut App| {
                let services = ChatServicesStore::shared(cx);
                let (accepted, next) = services.update(cx, |store, cx| store.complete_topic_notification_command(&command, cx));
                if !accepted {
                    return;
                }

                let snapshot = snapshot.clone();
                services.update(cx, |store, cx| store.restore_topic_rows(snapshot, &chat_id, &topic_id, SaiUpdaterWriteScope::All, cx));
                Self::continue_topic_notification_commands(next, &command, false, cx);
            })
        };
        ChatActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.update_topic_notifications_action(
                &chat_id,
                &topic_id,
                command.scopes,
                command.inherit,
                Some(on_success),
                Some(on_error),
                cx,
            );
        });
    }

    fn continue_topic_notification_commands(
        next: Option<ChatServicesStoreTopicNotificationCommand>,
        command: &ChatServicesStoreTopicNotificationCommand,
        success: bool,
        cx: &mut App,
    ) {
        match next {
            Some(next) => cx.defer(move |cx| {
                ChatInteractionsStore::shared(cx).update(cx, |store, cx| store.dispatch_topic_notification_command(next, cx));
            }),
            None => report(&command.on_result, success, cx),
        }
    }

    fn apply_topic_notification_patch(command: &ChatServicesStoreTopicNotificationCommand, cx: &mut App) {
        let mut fields: Map<String, Value> = Map::new();
        fields.insert(key::NOTIFICATION_MESSAGES.to_owned(), Value::Bool(command.scopes.messages));
        fields.insert(key::NOTIFICATION_MENTIONS.to_owned(), Value::Bool(command.scopes.mentions));
        fields.insert(key::NOTIFICATION_REPLIES.to_owned(), Value::Bool(command.scopes.replies));
        fields.insert(key::NOTIFICATION_REACTIONS.to_owned(), Value::Bool(command.scopes.reactions));
        fields.insert(key::NOTIFICATION_EVENTS.to_owned(), Value::Bool(command.scopes.events));
        fields.insert(key::NOTIFICATION_INHERITED.to_owned(), Value::Bool(command.inherit));
        ChatServicesStore::shared(cx).update(cx, |store, cx| store.patch_topic(&command.chat_id, &command.topic_id, fields, cx));
    }

    fn finish_topic_notification_command(
        &mut self,
        command: ChatServicesStoreTopicNotificationCommand,
        success: bool,
        cx: &mut Context<Self>,
    ) {
        let (accepted, next) =
            ChatServicesStore::shared(cx).update(cx, |store, cx| store.complete_topic_notification_command(&command, cx));
        if !accepted {
            return;
        }

        match next {
            Some(next) => self.dispatch_topic_notification_command(next, cx),
            None => report(&command.on_result, success, cx),
        }
    }
}
