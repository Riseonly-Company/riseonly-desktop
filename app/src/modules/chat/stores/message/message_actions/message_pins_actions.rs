use gpui::App;
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

use super::message_actions::{
    MessageActionErrorHandler, MessageActionsStore, effective_user_id_for_wire, log_message_failure, on_account_runtime,
};
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::chat::engine::messages::rise_message_scope::RiseMessageScope;
use crate::modules::chat::stores::message::message_services::message_services::MessageServicesStore;

impl MessageActionsStore {
    pub fn get_pinned_messages_action(
        &self,
        chat_id: &str,
        relative_id: Option<&str>,
        _reload_if_cached: bool,
        _reflect_pending_in_ui: Option<bool>,
        cx: &mut App,
    ) {
        let Some(user_id) = effective_user_id_for_wire(cx) else {
            return;
        };
        if chat_id.is_empty() {
            return;
        }
        let chat_id = chat_id.to_owned();
        let relative_id = relative_id.map(str::to_owned);
        cx.spawn(async move |cx| match RiseConversationActionExecutor::require_domain(cx).await {
            Ok(domain) => {
                drop(RiseAccountRuntime::current().spawn(async move {
                    domain
                        .load_pinned_messages(
                            &user_id,
                            &chat_id,
                            relative_id.as_deref(),
                            Some(MessageServicesStore::PINNED_MESSAGES_LIMIT),
                        )
                        .await;
                }));
            }
            Err(error) => cx.update(|cx| {
                log_message_failure(format!("Rise get_pinned_messages({chat_id}) failed: {error}"), cx);
            }),
        })
        .detach();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn pin_message_action(
        &self,
        message_id: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        for_everyone: Option<bool>,
        silent: Option<bool>,
        on_error: Option<MessageActionErrorHandler>,
        cx: &mut App,
    ) {
        let scope =
            RiseMessageScope::new(chat_id.to_owned(), topic_id.map(str::to_owned), thread_root_id.map(str::to_owned));
        let message_id = message_id.to_owned();
        let chat_id = chat_id.to_owned();
        cx.spawn(async move |cx| {
            let outcome: anyhow::Result<()> = async {
                let domain = RiseConversationActionExecutor::require_domain(cx).await?;
                let (work_message_id, work_scope, work_chat_id) = (message_id.clone(), scope.clone(), chat_id.clone());
                on_account_runtime(async move {
                    domain.messages.set_pinned(true, &work_message_id, &work_scope, for_everyone, silent).await?;
                    domain.refresh_pinned_messages_presentation_from_cache(&work_chat_id).await;
                    anyhow::Ok(())
                })
                .await?
            }
            .await;
            if let Err(error) = outcome {
                cx.update(|cx| {
                    log_message_failure(
                        format!("Rise pin_message({}/{message_id}) failed: {error:#}", scope.storage_key()),
                        cx,
                    );
                    if let Some(on_error) = on_error {
                        on_error(&error, cx);
                    }
                });
            }
        })
        .detach();
    }

    pub fn unpin_message_action(
        &self,
        message_id: &str,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        on_error: Option<MessageActionErrorHandler>,
        cx: &mut App,
    ) {
        let scope =
            RiseMessageScope::new(chat_id.to_owned(), topic_id.map(str::to_owned), thread_root_id.map(str::to_owned));
        let message_id = message_id.to_owned();
        let chat_id = chat_id.to_owned();
        cx.spawn(async move |cx| {
            let outcome: anyhow::Result<()> = async {
                let domain = RiseConversationActionExecutor::require_domain(cx).await?;
                let (work_message_id, work_scope, work_chat_id) = (message_id.clone(), scope.clone(), chat_id.clone());
                on_account_runtime(async move {
                    domain.messages.set_pinned(false, &work_message_id, &work_scope, None, None).await?;
                    domain.refresh_pinned_messages_presentation_from_cache(&work_chat_id).await;
                    anyhow::Ok(())
                })
                .await?
            }
            .await;
            if let Err(error) = outcome {
                cx.update(|cx| {
                    log_message_failure(
                        format!("Rise unpin_message({}/{message_id}) failed: {error:#}", scope.storage_key()),
                        cx,
                    );
                    if let Some(on_error) = on_error {
                        on_error(&error, cx);
                    }
                });
            }
        })
        .detach();
    }

    pub fn unpin_all_messages_action(&self, chat_id: &str, on_error: Option<MessageActionErrorHandler>, cx: &mut App) {
        if chat_id.is_empty() {
            return;
        }
        let chat_id = chat_id.to_owned();
        cx.spawn(async move |cx| {
            let outcome: anyhow::Result<()> = async {
                let domain = RiseConversationActionExecutor::require_domain(cx).await?;
                let work_chat_id = chat_id.clone();
                on_account_runtime(async move {
                    domain.messages.unpin_all(&work_chat_id).await?;
                    domain.refresh_pinned_messages_presentation_from_cache(&work_chat_id).await;
                    anyhow::Ok(())
                })
                .await?
            }
            .await;
            if let Err(error) = outcome {
                cx.update(|cx| {
                    log_message_failure(format!("Rise unpin_all_messages({chat_id}) failed: {error}"), cx);
                    if let Some(on_error) = on_error {
                        on_error(&error, cx);
                    }
                });
            }
        })
        .detach();
    }
}
