use std::collections::HashMap;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{App, Context, Entity};

use crate::core::desktop_platform::rise_app_events::RiseAppEvents;
use crate::core::lib::sai::core::websocket::sai_web_socket_manager::SAI_WS_DID_CONNECT;
use crate::modules::chat::engine::chat_engine_hooks::{chat_engine_hooks, chat_engine_rtc_hooks};
use crate::modules::chat::engine::forum::rise_forum_voice_realtime::RiseForumVoiceRealtime;
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomainRegistry;

#[derive(Clone, Debug)]
struct Interest {
    topic_ids: Vec<String>,
    includes_group: bool,
}

/// forum voice subscriptions this client thinks it holds
/// ws subscriptions die with the socket so the intent is kept here and re armed after a reconnect
pub struct ForumVoiceSubscriptionRegistry {
    active: HashMap<String, Interest>,
    did_install_observer: bool,
}

impl ForumVoiceSubscriptionRegistry {
    pub fn shared(cx: &mut App) -> Entity<Self> {
        rise_ui::shared::shared(cx, |_| Self { active: HashMap::new(), did_install_observer: false })
    }

    pub fn retain(&mut self, chat_id: &str, topic_ids: Vec<String>, includes_group: bool, cx: &mut Context<Self>) {
        if chat_id.is_empty() || (topic_ids.is_empty() && !includes_group) {
            return;
        }
        self.active.insert(chat_id.to_owned(), Interest { topic_ids, includes_group });
        self.install_reconnect_observer_if_needed(cx);
        cx.notify();
    }

    pub fn release(&mut self, chat_id: &str, cx: &mut Context<Self>) {
        self.active.remove(chat_id);
        cx.notify();
    }

    pub fn observes_group(&self, chat_id: &str) -> bool {
        self.active.get(chat_id).is_some_and(|interest| interest.includes_group)
    }

    pub fn release_all(&mut self, cx: &mut Context<Self>) {
        self.active = HashMap::new();
        cx.notify();
    }

    fn install_reconnect_observer_if_needed(&mut self, cx: &mut Context<Self>) {
        if self.did_install_observer {
            return;
        }
        self.did_install_observer = true;
        let (did_connect, mut on_did_connect) = mpsc::unbounded::<()>();
        RiseAppEvents::default_center()
            .observe(SAI_WS_DID_CONNECT, move |_| {
                let _ = did_connect.unbounded_send(());
            })
            .detach();
        cx.spawn(async move |this, cx| {
            while on_did_connect.next().await.is_some() {
                if this.update(cx, |registry, cx| registry.rearm(cx)).is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    fn rearm(&mut self, cx: &mut Context<Self>) {
        if self.active.is_empty() {
            return;
        }
        let domains = RiseConversationDomainRegistry::shared(cx);
        let Some(domain) = domains.read(cx).active() else {
            return;
        };
        let snapshot = self.active.clone();
        cx.spawn(async move |_, cx| {
            for (chat_id, interest) in snapshot {
                RiseForumVoiceRealtime::set_subscribed(
                    true,
                    &chat_id,
                    &interest.topic_ids,
                    interest.includes_group,
                    &domain.engine,
                )
                .await;
                if !interest.topic_ids.is_empty() {
                    cx.update(|cx| {
                        if let Some(hooks) = chat_engine_hooks() {
                            (hooks.get_forum_structure_action)(&chat_id, true, None, None, cx);
                        }
                    });
                }
                if interest.includes_group {
                    let load = cx.update(|cx| chat_engine_rtc_hooks().map(|hooks| (hooks.group_voice_settings_load)(&chat_id, cx)));
                    if let Some(load) = load {
                        load.await;
                    }
                }
            }
        })
        .detach();
    }
}
