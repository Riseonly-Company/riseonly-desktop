use gpui::{App, Entity};
use rise_i18n::tr;

use crate::app::router::navigation_store::NavigationStore;
use crate::app::router::navigation_types::RiseRTCRoomTarget;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::chat::stores::chats::chat_actions::chat_types::ChatVoiceConnectGateDecision;
use crate::modules::chat::stores::chats::chat_interactions::chat_interactions::ChatInteractionsStore;
use crate::modules::rtc::stores::rtc::rtc_interactions::rtc_interactions::RTCInteractionsStore;
use crate::modules::rtc::system::rise_rtc_call_window_hooks::rise_rtc_call_window_hooks;

pub enum RiseRTCNavigationGate {}

impl RiseRTCNavigationGate {
    /// a direct call opens at once, a voice channel only after the chat connect permission allows it
    pub fn open(target: RiseRTCRoomTarget, navigation_store: &Entity<NavigationStore>, cx: &mut App) {
        let kind = target.safe_log_kind();
        Sai::log(
            "RTC",
            move || format!("room open requested target={kind}"),
            SaiLogKind::Network,
            cx,
        );
        if matches!(target, RiseRTCRoomTarget::DirectCall { .. }) {
            RTCInteractionsStore::shared(cx).update(cx, |interactions, cx| interactions.present_room(target, cx));
            return;
        }
        let (chat_id, topic_id) = match &target {
            RiseRTCRoomTarget::DirectCall { .. } => return,
            RiseRTCRoomTarget::Topic { chat_id, topic_id } => (chat_id.clone(), Some(topic_id.clone())),
            RiseRTCRoomTarget::Group { chat_id } => (chat_id.clone(), None),
        };
        Self::resolve(target, chat_id, topic_id, navigation_store, cx);
    }

    /// this account presence in the forum row single owner since a channel can be
    /// left four ways (button, failed join and screen close and server eviction)
    pub fn mark_self_in_voice_channel(target: &RiseRTCRoomTarget, joined: bool, cx: &mut App) {
        match target {
            RiseRTCRoomTarget::Topic { chat_id, topic_id } => {
                ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.mark_self_in_voice_channel(chat_id, topic_id, joined, cx);
                });
            }
            RiseRTCRoomTarget::Group { chat_id } => {
                ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
                    interactions.mark_self_in_voice_channel(chat_id, "", joined, cx);
                });
            }
            RiseRTCRoomTarget::DirectCall { .. } => {}
        }
    }

    fn resolve(
        target: RiseRTCRoomTarget,
        chat_id: String,
        topic_id: Option<String>,
        _navigation_store: &Entity<NavigationStore>,
        cx: &mut App,
    ) {
        ChatInteractionsStore::shared(cx).update(cx, |interactions, cx| {
            interactions.resolve_voice_connect_permission(
                &chat_id,
                topic_id.as_deref(),
                Box::new(move |decision: ChatVoiceConnectGateDecision, cx: &mut App| {
                    let kind = target.safe_log_kind();
                    let value = decision.safe_log_value();
                    Sai::log(
                        "RTC",
                        move || format!("room permission target={kind} decision={value}"),
                        SaiLogKind::Network,
                        cx,
                    );
                    match decision {
                        ChatVoiceConnectGateDecision::Allowed => {
                            RTCInteractionsStore::shared(cx)
                                .update(cx, |interactions, cx| interactions.present_room(target, cx));
                            if let Some(hooks) = rise_rtc_call_window_hooks() {
                                cx.defer(hooks.activate_call_window);
                            }
                        }
                        ChatVoiceConnectGateDecision::Denied => {
                            Self::notify_error(tr("rtc_connect_permission_denied"), cx);
                        }
                        ChatVoiceConnectGateDecision::RefreshRequired => {
                            Self::notify_error(tr("rtc_permission_check_failed"), cx);
                        }
                    }
                }),
                cx,
            );
        });
    }

    fn notify_error(title: String, cx: &mut App) {
        Notifier::send(
            title,
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }
}

pub trait RiseRTCRoomTargetSafeLogKind {
    fn safe_log_kind(&self) -> &'static str;
}

impl RiseRTCRoomTargetSafeLogKind for RiseRTCRoomTarget {
    fn safe_log_kind(&self) -> &'static str {
        match self {
            Self::Topic { .. } => "topic",
            Self::Group { .. } => "group",
            Self::DirectCall { .. } => "direct_call",
        }
    }
}

trait ChatVoiceConnectGateDecisionSafeLogValue {
    fn safe_log_value(&self) -> &'static str;
}

impl ChatVoiceConnectGateDecisionSafeLogValue for ChatVoiceConnectGateDecision {
    fn safe_log_value(&self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Denied => "denied",
            Self::RefreshRequired => "refresh_required",
        }
    }
}
