use gpui::App;

use super::notify_interactions::NotifyInteractionsStore;
use crate::modules::auth::stores::auth_services::auth_services_notify_hooks::AuthServicesNotifyHooks;

pub fn install_auth_services_notify_hooks() {
    AuthServicesNotifyHooks::install(AuthServicesNotifyHooks {
        note_account_created: Box::new(|cx: &mut App| {
            NotifyInteractionsStore::shared(cx).update(cx, |store, cx| store.note_account_created(cx));
        }),
    });
}
