use std::rc::Rc;
use std::sync::Arc;

use gpui::App;

use super::message_actions::{MessageActionsStore, log_message_failure, on_account_runtime};
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::chat::engine::messages::rise_chat_calendar::{RiseChatCalendarPage, RiseChatCalendarWindow};
use crate::modules::chat::engine::messages::rise_message_scope::{RiseMessageScope, RiseMessageWindowConfiguration};
use crate::modules::chat::engine::rise_conversation_domain::RiseConversationDomain;

async fn chat_calendar_page(
    domain: &Arc<RiseConversationDomain>,
    scope: &Arc<RiseMessageScope>,
    window: &Arc<RiseChatCalendarWindow>,
    include_previews: bool,
    force_refresh: bool,
) -> Option<RiseChatCalendarPage> {
    let (domain, scope, window) = (domain.clone(), scope.clone(), window.clone());
    on_account_runtime(async move { domain.chat_calendar(&scope, &window, include_previews, force_refresh).await })
        .await
        .ok()
        .flatten()
}

impl MessageActionsStore {
    #[allow(clippy::too_many_arguments)]
    pub fn load_chat_calendar_month_action(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        window: RiseChatCalendarWindow,
        include_previews: bool,
        force_refresh: bool,
        revalidated: Option<Rc<dyn Fn(RiseChatCalendarPage, &mut App)>>,
        completion: Rc<dyn Fn(Option<RiseChatCalendarPage>, &mut App)>,
        cx: &mut App,
    ) {
        if chat_id.is_empty() {
            cx.defer(move |cx| completion(None, cx));
            return;
        }
        let scope = Arc::new(RiseMessageScope::new(
            chat_id.to_owned(),
            topic_id.map(str::to_owned),
            thread_root_id.map(str::to_owned),
        ));
        let window = Arc::new(window);
        cx.spawn(async move |cx| {
            let domain = match RiseConversationActionExecutor::require_domain(cx).await {
                Ok(domain) => domain,
                Err(error) => {
                    cx.update(|cx| {
                        log_message_failure(
                            format!("Rise get_chat_calendar({}) failed: {error}", scope.storage_key()),
                            cx,
                        );
                        completion(None, cx);
                    });
                    return;
                }
            };
            if !force_refresh && let Some(revalidated) = revalidated {
                let reading = {
                    let (domain, scope, window) = (domain.clone(), scope.clone(), window.clone());
                    on_account_runtime(async move { domain.cached_chat_calendar(&scope, &window).await })
                };
                if let Some(cached) = reading.await.ok().flatten() {
                    let delivered = cached.clone();
                    cx.update(|cx| completion(Some(delivered), cx));
                    if let Some(fresh) = chat_calendar_page(&domain, &scope, &window, include_previews, true).await
                        && fresh != cached
                    {
                        cx.update(|cx| revalidated(fresh, cx));
                    }
                    return;
                }
            }
            let page = chat_calendar_page(&domain, &scope, &window, include_previews, force_refresh).await;
            cx.update(|cx| completion(page, cx));
        })
        .detach();
    }

    pub fn jump_to_date_action(
        &self,
        chat_id: &str,
        topic_id: Option<&str>,
        thread_root_id: Option<&str>,
        day_start_ms: i64,
        completion: Rc<dyn Fn(bool, &mut App)>,
        cx: &mut App,
    ) {
        if chat_id.is_empty() {
            cx.defer(move |cx| completion(false, cx));
            return;
        }
        let scope =
            RiseMessageScope::new(chat_id.to_owned(), topic_id.map(str::to_owned), thread_root_id.map(str::to_owned));
        cx.spawn(async move |cx| {
            let is_installed = match RiseConversationActionExecutor::require_domain(cx).await {
                Ok(domain) => on_account_runtime(async move {
                    domain
                        .jump_to_date(&scope, day_start_ms, RiseMessageWindowConfiguration::presentation())
                        .await
                        .is_installed()
                })
                .await
                .unwrap_or(false),
                Err(error) => {
                    cx.update(|cx| {
                        log_message_failure(format!("Rise jump-to-date({}) failed: {error}", scope.storage_key()), cx);
                    });
                    false
                }
            };
            cx.update(|cx| completion(is_installed, cx));
        })
        .detach();
    }
}
