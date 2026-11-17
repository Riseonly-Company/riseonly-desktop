use gpui::{App, Context};
use rise_i18n::tr;

use crate::modules::auth::stores::auth_interactions::auth_interactions::AuthInteractionsStore;
use crate::modules::user::stores::user::user_actions::user_actions::UserActionsStore;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

impl UserInteractionsStore {
    pub fn delete_my_account(&mut self, on_error: impl FnOnce(String, &mut App) + 'static, cx: &mut Context<Self>) {
        if self.is_deleting_account {
            return;
        }
        self.is_deleting_account = true;
        cx.notify();

        let on_success_store = cx.entity().downgrade();
        let on_error_store = on_success_store.clone();
        let on_success: Box<dyn FnOnce(&mut App)> = Box::new(move |cx: &mut App| {
            let _ = on_success_store.update(cx, |store, cx| {
                store.is_deleting_account = false;
                cx.notify();
            });
            AuthInteractionsStore::shared(cx).update(cx, |auth, cx| auth.logout(cx));
        });
        let on_failure: Box<dyn FnOnce(&anyhow::Error, &mut App)> = Box::new(move |_: &anyhow::Error, cx: &mut App| {
            let _ = on_error_store.update(cx, |store, cx| {
                store.is_deleting_account = false;
                cx.notify();
            });
            on_error(tr("settings_delete_account_error"), cx);
        });
        UserActionsStore::shared(cx).update(cx, |actions, cx| {
            actions.delete_my_account_action(Some(on_success), Some(on_failure), cx);
        });
    }
}
