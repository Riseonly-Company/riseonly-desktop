use gpui::App;
use rise_canon::{rise_request, rise_wire_methods};
use rise_engine::CancellationError;
use rise_errors::{ErrorCategory, ErrorCode};
use rise_i18n::tr;

use super::message_actions::{
    MessageActionHandler, MessageActionsStore, guide_sandbox_owns_chat, on_account_runtime, profile_user_id,
};
use super::message_wire_types::{AddReactionGatewayDTO, RemoveReactionGatewayDTO};
use crate::core::lib::canon_errors::CanonErrors;
use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::sai::Sai;
use crate::core::widgets::notifier::notifier_store::{Notifier, NotifierPosition, NotifierSendOptions, NotifierType};
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

fn log_reaction_failure(text: String, cx: &mut App) {
    Sai::log("Reactions", move || text, SaiLogKind::Network, cx);
}

fn server_message_id(message_id: &str, scope: &str, cx: &mut App) -> String {
    MessageServicesStore::shared(cx).update(cx, |store, cx| store.server_message_id(message_id, scope, cx))
}

fn finish_reaction_failure(
    error: &anyhow::Error,
    on_failed: Option<MessageActionHandler>,
    on_connection_lost: Option<MessageActionHandler>,
    cx: &mut App,
) {
    let is_cancellation = error.is::<CancellationError>();
    if let Some(on_connection_lost) = on_connection_lost
        && (is_cancellation || CanonErrors::is_connection_loss(error))
    {
        on_connection_lost(cx);
        return;
    }
    if !is_cancellation && let Some(on_failed) = on_failed {
        on_failed(cx);
    }
    MessageActionsStore::report_reaction_refusal(error, cx);
}

impl MessageActionsStore {
    #[allow(clippy::too_many_arguments)]
    pub fn add_reaction_action(
        &self,
        chat_id: &str,
        message_id: &str,
        reaction: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        on_failed: Option<MessageActionHandler>,
        on_connection_lost: Option<MessageActionHandler>,
        on_succeeded: Option<MessageActionHandler>,
        cx: &mut App,
    ) {
        if self.reject_guide_chat_mutation(chat_id, cx) || profile_user_id(cx).is_none() {
            if let Some(on_failed) = on_failed {
                cx.defer(on_failed);
            }
            return;
        }
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, thread_root_id);
        let body = rise_request::message::AddReaction {
            message_id: server_message_id(message_id, &scope, cx),
            reaction: reaction.to_owned(),
        };
        let message_id = message_id.to_owned();
        cx.spawn(async move |cx| {
            let result = RiseConversationActionExecutor::call::<_, AddReactionGatewayDTO>(
                rise_wire_methods::message::SERVICE,
                rise_wire_methods::message::ADD_REACTION,
                &body,
                cx,
            )
            .await;
            match result {
                Ok(_) => {
                    if let Some(on_succeeded) = on_succeeded {
                        cx.update(|cx| on_succeeded(cx));
                    }
                }
                Err(error) => {
                    cx.update(|cx| {
                        log_reaction_failure(format!("Rise add_reaction({scope}/{message_id}) failed: {error}"), cx);
                        finish_reaction_failure(&error, on_failed, on_connection_lost, cx);
                    });
                }
            }
        })
        .detach();
    }

    pub fn preview_reaction_action(
        &self,
        chat_id: &str,
        message_id: &str,
        reaction: &str,
        selected: bool,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        cx: &mut App,
    ) {
        if guide_sandbox_owns_chat(chat_id, cx) {
            return;
        }
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, thread_root_id);
        let identifier = server_message_id(message_id, &scope, cx);
        let message_id = message_id.to_owned();
        let reaction = reaction.to_owned();
        cx.spawn(async move |cx| {
            let outcome: anyhow::Result<()> = async {
                let domain = RiseConversationActionExecutor::require_domain(cx).await?;
                let (reaction, identifier) = (reaction.clone(), identifier.clone());
                on_account_runtime(async move { domain.messages.preview_reaction(&reaction, selected, &identifier).await })
                    .await?
            }
            .await;
            if let Err(error) = outcome {
                cx.update(|cx| {
                    log_reaction_failure(format!("Rise preview_reaction({scope}/{message_id}) failed: {error}"), cx);
                });
            }
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn queue_reaction_action(
        &self,
        chat_id: &str,
        message_id: &str,
        reaction: &str,
        selected: bool,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        on_unqueued: MessageActionHandler,
        cx: &mut App,
    ) {
        if guide_sandbox_owns_chat(chat_id, cx) {
            cx.defer(on_unqueued);
            return;
        }
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, thread_root_id);
        let identifier = server_message_id(message_id, &scope, cx);
        let message_id = message_id.to_owned();
        let reaction = reaction.to_owned();
        cx.spawn(async move |cx| {
            let outcome: anyhow::Result<()> = async {
                let domain = RiseConversationActionExecutor::require_domain(cx).await?;
                let (reaction, identifier) = (reaction.clone(), identifier.clone());
                on_account_runtime(async move { domain.messages.queue_reaction(&reaction, selected, &identifier).await })
                    .await?
            }
            .await;
            if let Err(error) = outcome {
                cx.update(|cx| {
                    log_reaction_failure(format!("Rise queue_reaction({scope}/{message_id}) failed: {error}"), cx);
                    on_unqueued(cx);
                });
            }
        })
        .detach();
    }

    fn report_reaction_refusal(error: &anyhow::Error, cx: &mut App) {
        if error.is::<CancellationError>() {
            return;
        }
        let Some(failure) = CanonErrors::failure(error) else {
            return;
        };
        let code = failure.code.as_deref();
        if !matches!(
            CanonErrors::category(code),
            ErrorCategory::Permission | ErrorCategory::RateLimit | ErrorCategory::Quota | ErrorCategory::Unavailable
        ) {
            return;
        }
        let code = code.unwrap_or_default();
        let text = if code == ErrorCode::ChatSendRestricted.wire() || code == ErrorCode::ChatMemberMuted.wire() {
            Some(tr("chat_error_reaction_restricted"))
        } else {
            CanonErrors::text_for(&failure)
        };
        let Some(text) = text else {
            return;
        };
        Notifier::send(
            text,
            NotifierSendOptions {
                r#type: NotifierType::Error,
                position: NotifierPosition::Bottom,
                ..NotifierSendOptions::default()
            },
            cx,
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub fn remove_reaction_action(
        &self,
        chat_id: &str,
        message_id: &str,
        reaction: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        on_failed: Option<MessageActionHandler>,
        on_connection_lost: Option<MessageActionHandler>,
        on_succeeded: Option<MessageActionHandler>,
        cx: &mut App,
    ) {
        if self.reject_guide_chat_mutation(chat_id, cx) || profile_user_id(cx).is_none() {
            if let Some(on_failed) = on_failed {
                cx.defer(on_failed);
            }
            return;
        }
        let scope = MessageServicesStore::message_scope(chat_id, topic_id, thread_root_id);
        let body = rise_request::message::RemoveReaction {
            message_id: server_message_id(message_id, &scope, cx),
            reaction: reaction.to_owned(),
        };
        let message_id = message_id.to_owned();
        cx.spawn(async move |cx| {
            let result = RiseConversationActionExecutor::call::<_, RemoveReactionGatewayDTO>(
                rise_wire_methods::message::SERVICE,
                rise_wire_methods::message::REMOVE_REACTION,
                &body,
                cx,
            )
            .await;
            match result {
                Ok(_) => {
                    if let Some(on_succeeded) = on_succeeded {
                        cx.update(|cx| on_succeeded(cx));
                    }
                }
                Err(error) => {
                    cx.update(|cx| {
                        log_reaction_failure(format!("Rise remove_reaction({scope}/{message_id}) failed: {error}"), cx);
                        finish_reaction_failure(&error, on_failed, on_connection_lost, cx);
                    });
                }
            }
        })
        .detach();
    }
}
