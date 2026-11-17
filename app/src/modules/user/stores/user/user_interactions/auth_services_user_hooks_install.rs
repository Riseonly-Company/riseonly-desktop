use gpui::App;

use crate::modules::auth::stores::auth_services::auth_services_user_hooks::AuthServicesUserHooks;
use crate::modules::user::stores::user::user_interactions::user_interactions::UserInteractionsStore;

pub(crate) fn install_auth_services_user_hooks() {
    AuthServicesUserHooks::install(AuthServicesUserHooks {
        prepare_contact_sync_after_registration: Box::new(|cx: &mut App| {
            UserInteractionsStore::shared(cx)
                .update(cx, |store, cx| store.prepare_contact_sync_after_registration(cx));
        }),
        refresh_my_profile: Box::new(|cx: &mut App| {
            UserInteractionsStore::shared(cx).update(cx, |store, cx| store.refresh_my_profile(cx));
        }),
    });
}
