use gpui::{App, Task};
use rise_engine::desktop_runtime::rise_account_runtime::RiseAccountRuntime;

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::sai::Sai;
use crate::modules::chat::engine::core::rise_conversation_repository_facade::RiseConversationActionExecutor;
use crate::modules::chat::stores::message::message_interactions::message_interactions::MessageInteractionsStore;

impl MessageInteractionsStore {
    pub fn vote_poll(&self, message_id: &str, option_ids: Vec<String>, cx: &mut App) -> Task<bool> {
        if message_id.is_empty() {
            return Task::ready(false);
        }
        let account_user_id = SaiAccountCacheScope::active_user_id();
        let account_epoch = SaiAccountCacheScope::active_epoch();
        let message_id = message_id.to_owned();
        cx.spawn(async move |cx| {
            let outcome: anyhow::Result<bool> = async {
                let domain = RiseConversationActionExecutor::require_domain(cx).await?;
                if !SaiAccountCacheScope::is_current_account_epoch(account_user_id.as_deref(), account_epoch.as_deref())
                {
                    return Ok(false);
                }
                let local_id = message_id.clone();
                RiseAccountRuntime::current()
                    .spawn(async move { domain.messages.vote_poll(&local_id, &option_ids).await })
                    .await??;
                Ok(true)
            }
            .await;
            match outcome {
                Ok(counted) => counted,
                Err(error) => {
                    let line = format!("Rise vote_poll({message_id}) failed: {error}");
                    cx.update(|cx| Sai::log("Message", move || line, SaiLogKind::Network, cx));
                    false
                }
            }
        })
    }
}
