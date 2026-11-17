use std::rc::Rc;

use gpui::App;
use rise_engine::CancellationError;
use rise_i18n::tr;

use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::core::widgets::overlay::premium_upsell_store::PremiumUpsellStore;
use crate::modules::chat::stores::message::message_actions::message_actions::MessageActionErrorHandler;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;

impl MessageInteractionsStore {
    pub fn load_initial_pinned_messages(&self, chat_id: &str, cx: &mut App) {
        if chat_id.is_empty() {
            return;
        }
        let has_cache = Self::service(cx).update(cx, |service, cx| service.has_pinned_messages_cache(chat_id, cx));
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_pinned_messages_action(chat_id, None, false, Some(!has_cache), cx);
        });
    }

    pub fn load_more_pinned_messages(&self, chat_id: &str, cx: &mut App) {
        if chat_id.is_empty() {
            return;
        }
        let page = Self::service(cx).update(cx, |service, cx| service.pinned_messages_pagination(chat_id, cx));
        if !page.has_more {
            return;
        }
        let Some(relative_id) = page.relative_id else {
            return;
        };
        Self::actions(cx).update(cx, |actions, cx| {
            actions.get_pinned_messages_action(chat_id, Some(&relative_id), true, None, cx);
        });
    }

    #[allow(clippy::too_many_arguments)]
    pub fn pin_message(
        &self,
        id: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        for_everyone: Option<bool>,
        silent: Option<bool>,
        cx: &mut App,
    ) {
        Self::actions(cx).update(cx, |actions, cx| {
            actions.pin_message_action(
                id,
                chat_id,
                topic_id,
                thread_root_id,
                for_everyone,
                silent,
                Some(Self::pin_failure_reporter()),
                cx,
            );
        });
    }

    pub fn unpin_message(
        &self,
        id: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        cx: &mut App,
    ) {
        Self::actions(cx).update(cx, |actions, cx| {
            actions.unpin_message_action(id, chat_id, topic_id, thread_root_id, Some(Self::pin_failure_reporter()), cx);
        });
    }

    pub fn unpin_all_messages(&self, chat_id: &str, cx: &mut App) {
        Self::actions(cx).update(cx, |actions, cx| {
            actions.unpin_all_messages_action(chat_id, Some(Self::pin_failure_reporter()), cx);
        });
    }

    fn pin_failure_reporter() -> MessageActionErrorHandler {
        Rc::new(Self::report_pin_failure)
    }

    fn report_pin_failure(error: &anyhow::Error, cx: &mut App) {
        if error.is::<CancellationError>() {
            return;
        }
        if PremiumUpsellStore::shared(cx).update(cx, |store, cx| store.offer_covers(error, cx)) {
            return;
        }
        let text = Self::message_refusal_text(error).unwrap_or_else(|| tr("retry_later_error"));
        let options = NotifierSendOptions {
            r#type: NotifierType::Error,
            position: NotifierPosition::Bottom,
            ..NotifierSendOptions::default()
        };
        Notifier::send(text, options, cx);
    }

    pub fn delete_all_messages_from_user(&self, chat_id: &str, target_user_id: &str, cx: &mut App) {
        Self::actions(cx).update(cx, |actions, cx| {
            actions.delete_all_messages_from_user_action(
                chat_id,
                target_user_id,
                Some(Self::delete_failure_reporter(chat_id)),
                cx,
            );
        });
    }
}
