use gpui::{App, AsyncApp, Context};

use crate::core::lib::sai::core::logger::sai_log_kind::SaiLogKind;
use crate::core::lib::sai::core::utilities::sai_account_cache_scope::SaiAccountCacheScope;
use crate::core::lib::sai::sai::Sai;
use crate::modules::user::engine::core::rise_user_debug_trace::RiseUserDebugTrace;
use crate::modules::user::engine::rise_user_domain::RiseUserActionExecutor;

use super::user_actions::UserActionsStore;

impl UserActionsStore {
    pub fn delete_my_account_action(
        &mut self,
        on_success: Option<Box<dyn FnOnce(&mut App)>>,
        on_error: Option<Box<dyn FnOnce(&anyhow::Error, &mut App)>>,
        cx: &mut Context<Self>,
    ) {
        let account_id = SaiAccountCacheScope::active_user_id().unwrap_or_default();
        let domain = RiseUserActionExecutor::require_domain(cx);
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let result = match domain.await {
                Ok(domain) => domain.delete_my_account().await.map(|_| ()),
                Err(error) => Err(error),
            };
            cx.update(|cx| match result {
                Ok(()) => {
                    if let Some(on_success) = on_success {
                        on_success(cx);
                    }
                }
                Err(error) => {
                    let label = RiseUserDebugTrace::error_label(&error);
                    Sai::log(
                        "User",
                        move || format!("account deletion failed for {account_id}: {label}"),
                        SaiLogKind::Network,
                        cx,
                    );
                    if let Some(on_error) = on_error {
                        on_error(&error, cx);
                    }
                }
            });
        })
        .detach();
    }
}
